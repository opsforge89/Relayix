use crate::network;
use anyhow::{anyhow, Result};
use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

pub const SECRET_FIELDS: &[&str] = &[
    "bot_token",
    "token",
    "access_token",
    "password",
    "webhook_secret",
    "url",
    "user",
    "proxy_username",
    "proxy_password",
    "proxy_secret",
    "api_hash",
    "chat_access_hash",
];

pub fn smtp_port(cfg: &Value) -> Result<u16> {
    let port = match cfg.get("port") {
        None => {
            if cfg["tls"].as_str() == Some("tls") {
                465
            } else {
                587
            }
        }
        Some(Value::String(s)) => s.parse::<u16>().map_err(|_| anyhow!("Invalid SMTP port"))?,
        Some(v) => v
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| anyhow!("Invalid SMTP port"))?,
    };
    if port == 0 {
        return Err(anyhow!("Invalid SMTP port"));
    }
    Ok(port)
}

pub fn smtp_sender(cfg: &Value) -> Result<Mailbox> {
    let mut sender = cfg["from"]
        .as_str()
        .unwrap_or("")
        .parse::<Mailbox>()
        .map_err(|_| anyhow!("Invalid SMTP from address"))?;
    if let Some(name) = cfg.get("from_name") {
        let name = name
            .as_str()
            .ok_or_else(|| anyhow!("SMTP sender name must be text"))?;
        if name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(anyhow!(
                "SMTP sender name must be at most 100 characters, without control characters"
            ));
        }
        // A separate name takes precedence; clearing it preserves the legacy From mailbox format.
        if !name.trim().is_empty() {
            sender = Mailbox::new(Some(name.trim().to_owned()), sender.email);
        }
    }
    Ok(sender)
}

pub fn valid_smtp(cfg: &Value, allow_insecure: bool) -> Result<()> {
    let host = cfg["host"].as_str().unwrap_or("");
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || "/@?#".contains(c))
        || url::Host::parse(host).is_err()
    {
        return Err(anyhow!(
            "SMTP host must be a hostname or IP address, without a URL scheme"
        ));
    }
    smtp_port(cfg)?;
    match cfg["tls"].as_str().unwrap_or("starttls") {
        "starttls" | "tls" => {}
        "none" if allow_insecure => {}
        "none" => {
            return Err(anyhow!(
                "Unencrypted SMTP requires ALLOW_INSECURE_HTTP=true"
            ))
        }
        _ => return Err(anyhow!("SMTP TLS mode must be starttls, tls or none")),
    }
    for field in ["from", "to"] {
        cfg[field]
            .as_str()
            .unwrap_or("")
            .parse::<Mailbox>()
            .map_err(|_| anyhow!("Invalid SMTP {field} address (use one mailbox per field)"))?;
    }
    smtp_sender(cfg)?;
    let username = cfg["username"].as_str().unwrap_or("");
    let password = cfg["password"].as_str().unwrap_or("");
    if username.is_empty() != password.is_empty() {
        return Err(anyhow!(
            "Provide both SMTP username and password, or leave both empty"
        ));
    }
    Ok(())
}

pub async fn send_smtp(cfg: &Value, title: &str, body: &str) -> Result<()> {
    let message = Message::builder()
        .from(smtp_sender(cfg)?)
        .to(cfg["to"].as_str().unwrap().parse()?)
        .subject(title)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_owned())?;
    let (transport, _tunnel) = smtp_transport(cfg).await?;
    // Bound the entire conversation, including connection establishment and TLS negotiation.
    tokio::time::timeout(Duration::from_secs(30), transport.send(message))
        .await
        .map_err(|_| anyhow!("SMTP delivery timed out"))?
        .map_err(|error| {
            let authentication = error.status().is_some_and(|code| matches!(code.to_string().as_str(), "534" | "535"));
            crate::retry_policy::transport(if authentication {"authentication"} else if error.is_permanent() {"destination"} else {"temporary"}, error.is_permanent(), None)
        })?;
    Ok(())
}

async fn smtp_transport(
    cfg: &Value,
) -> Result<(
    AsyncSmtpTransport<Tokio1Executor>,
    Option<crate::proxy::SmtpTunnel>,
)> {
    let host = cfg["host"].as_str().unwrap();
    let policy = network::policy()?;
    let address = policy.resolve(host, smtp_port(cfg)?).await?[0];
    let tunnel = if crate::proxy::mode(cfg) == "http" {
        Some(crate::proxy::smtp_tunnel(cfg, &address.ip().to_string(), smtp_port(cfg)?).await?)
    } else {
        None
    };
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    let tls = TlsParameters::new(host.to_owned()).map_err(|_| anyhow!("Invalid SMTP TLS host"))?;
    let destination = if tunnel.is_some() {
        "127.0.0.1".to_owned()
    } else {
        address.ip().to_string()
    };
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(destination).tls(
        match cfg["tls"].as_str().unwrap_or("starttls") {
            "tls" => Tls::Wrapper(tls),
            "starttls" => Tls::Required(tls),
            "none" => Tls::None,
            _ => return Err(anyhow!("Invalid SMTP TLS mode")),
        },
    );
    let mut builder = builder
        .port(tunnel.as_ref().map(|t| t.port).unwrap_or(smtp_port(cfg)?))
        .timeout(Some(Duration::from_secs(12)));
    if let Some(username) = cfg["username"].as_str().filter(|s| !s.is_empty()) {
        builder = builder.credentials(Credentials::new(
            username.to_owned(),
            cfg["password"].as_str().unwrap().to_owned(),
        ));
    }
    Ok((builder.build(), tunnel))
}
pub async fn diagnose_smtp(cfg: &Value) -> Result<()> {
    let (transport, _tunnel) = smtp_transport(cfg).await?;
    let connected=tokio::time::timeout(Duration::from_secs(25),transport.test_connection()).await.map_err(|_|anyhow!("SMTP diagnostic timed out"))?.map_err(|_|anyhow!("SMTP connection, TLS or authentication failed; check host, port, TLS mode, proxy and credentials"))?;
    if !connected {
        return Err(anyhow!("SMTP server rejected the connection check"));
    }
    Ok(())
}

fn matrix_url(cfg: &Value, parts: &[&str]) -> Result<url::Url> {
    let mut url = url::Url::parse(cfg["url"].as_str().unwrap())?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("Invalid Matrix homeserver URL"))?
        .pop_if_empty()
        .extend([
            "_matrix",
            "client",
            "v3",
            "rooms",
            cfg["room_id"].as_str().unwrap(),
        ])
        .extend(parts);
    Ok(url)
}

pub async fn send_matrix(
    client: &reqwest::Client,
    cfg: &Value,
    text: &str,
    delivery_id: Uuid,
) -> Result<()> {
    let token = cfg["access_token"].as_str().unwrap();
    // Never post plaintext into an encrypted room. This adapter does not implement E2EE.
    let encryption = client
        .get(matrix_url(cfg, &["state", "m.room.encryption"])?)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| anyhow!(e.without_url()))?;
    if encryption.status().is_success() {
        return Err(crate::retry_policy::permanent(
            "Encrypted Matrix rooms are not supported; use a room with encryption disabled"
        ));
    }
    if encryption.status() != reqwest::StatusCode::NOT_FOUND {
        crate::retry_policy::response(encryption).await?;
    }
    let response = client
        .put(matrix_url(
            cfg,
            &["send", "m.room.message", &delivery_id.to_string()],
        )?)
        .bearer_auth(token)
        .json(&json!({"msgtype":"m.text", "body":text}))
        .send()
        .await
        .map_err(|e| anyhow!(e.without_url()))?;
    crate::retry_policy::response(response).await?;
    Ok(())
}
