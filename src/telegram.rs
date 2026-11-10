use crate::*;
use std::process::Stdio;
use tokio::{io::AsyncWriteExt, process::Command};
fn with_application_defaults(cfg: &Value, defaults: &Value) -> Value {
    let mut effective = cfg.clone();
    for key in ["api_id", "api_hash"] {
        if effective[key].is_null() || effective[key].as_str() == Some("") {
            effective[key] = defaults[key].clone();
        }
    }
    effective
}
fn application_config(cfg: &Value) -> Value {
    with_application_defaults(
        cfg,
        &json!({"api_id":std::env::var("TELEGRAM_API_ID").unwrap_or_default(),"api_hash":std::env::var("TELEGRAM_API_HASH").unwrap_or_default()}),
    )
}
pub fn validate(cfg: &Value) -> Result<()> {
    let effective = application_config(cfg);
    let cfg = &effective;
    let mode = crate::proxy::mode(cfg);
    let api_id = cfg["api_id"]
        .as_u64()
        .or_else(|| cfg["api_id"].as_str().and_then(|s| s.parse().ok()))
        .filter(|id| *id > 0 && *id <= i32::MAX as u64);
    if api_id.is_none() {
        return Err(anyhow!("Telegram MTProto/WEB is not configured: set TELEGRAM_API_ID and TELEGRAM_API_HASH on the server using your application credentials from my.telegram.org"));
    }
    let api_hash = cfg["api_hash"].as_str().unwrap_or("");
    if api_hash.len() != 32 || !api_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "Telegram API hash must contain 32 hexadecimal characters"
        ));
    }
    let secret = cfg["proxy_secret"].as_str().unwrap_or("");
    let plain = (secret.len() == 32 || secret.len() == 34 && secret.starts_with("dd"))
        && secret.bytes().all(|b| b.is_ascii_hexdigit());
    let ee = if mode == "mtproto"
        && secret.starts_with("ee")
        && secret.len() > 34
        && secret.len().is_multiple_of(2)
    {
        hex::decode(secret)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes[17..].to_vec()).ok())
            .map(|domain| {
                domain.len() <= 253
                    && domain.contains('.')
                    && domain.split('.').all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && label.as_bytes()[0].is_ascii_alphanumeric()
                            && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                            && label
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                    && domain
                        .rsplit('.')
                        .next()
                        .map(|label| {
                            label.len() >= 2 && label.bytes().all(|b| b.is_ascii_alphabetic())
                        })
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    } else {
        false
    };
    if !plain && !ee {
        return Err(anyhow!("Use a plain/dd MTProxy secret, or an ee secret with a valid SNI domain for MTProto. WEB-proxy supports plain/dd only"));
    }
    if mode == "mtproto" {
        let host = cfg["proxy_host"].as_str().unwrap_or("");
        if host.is_empty()
            || host
                .chars()
                .any(|c| c.is_whitespace() || "/@?#\\".contains(c))
            || url::Host::parse(host).is_err()
        {
            return Err(anyhow!("Invalid MTProxy host"));
        }
        crate::proxy::port(cfg)?;
    } else if mode == "webproxy" {
        let address = cfg["webproxy_address"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("https://");
        let url = url::Url::parse(&format!("https://{address}"))
            .map_err(|_| anyhow!("Invalid Telegram WEB-proxy address"))?;
        let host = url.host_str().unwrap_or("");
        if !host.contains('.')
            || host.parse::<std::net::IpAddr>().is_ok()
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(anyhow!(
                "WEB-proxy requires a DNS name and optional base path, on HTTPS port 443"
            ));
        }
        if !matches!(url.path(), "" | "/") {
            return Err(anyhow!(
                "This WEB-proxy connector supports a root hostname; base paths are not supported"
            ));
        }
    }
    if let Some(hash) = cfg["chat_access_hash"].as_str().filter(|s| !s.is_empty()) {
        if hash.parse::<i64>().is_err() {
            return Err(anyhow!("Chat access hash must be a signed integer"));
        }
    }
    Ok(())
}
pub async fn transport(
    cfg: &Value,
    text: &str,
    delivery_id: Uuid,
    diagnose: bool,
    html: bool,
) -> Result<Value> {
    transport_files(cfg, text, delivery_id, diagnose, html, &[]).await
}
async fn transport_files(
    cfg: &Value,
    text: &str,
    delivery_id: Uuid,
    diagnose: bool,
    html: bool,
    files: &[attachments::File],
) -> Result<Value> {
    validate(cfg)?;
    static TRANSPORT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let _permit = TRANSPORT_SLOTS.acquire().await?;
    tokio::time::timeout(
        Duration::from_secs(60),
        transport_inner(cfg, text, delivery_id, diagnose, html, files),
    )
    .await
    .map_err(|_| anyhow!("Delivery attempt timed out"))?
}

async fn transport_inner(
    cfg: &Value,
    text: &str,
    delivery_id: Uuid,
    diagnose: bool,
    html: bool,
    files: &[attachments::File],
) -> Result<Value> {
    network::validate(cfg).await?;
    let mut pinned = cfg.clone();
    if proxy::mode(cfg) == "webproxy" {
        let raw = cfg["webproxy_address"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("https://");
        let url = url::Url::parse(&format!("https://{raw}"))?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("Missing WEB proxy host"))?;
        let addresses = network::policy()?.resolve(host, 443).await?;
        pinned["outbound_proxy_host"] = json!(host);
        pinned["outbound_proxy_ip"] = json!(addresses[0].ip().to_string());
    } else if proxy::mode(cfg) != "none" {
        pinned["proxy_host"] = json!(network::policy()?
            .resolve(cfg["proxy_host"].as_str().unwrap_or(""), proxy::port(cfg)?)
            .await?[0]
            .ip()
            .to_string());
    }
    let cfg = &pinned;
    let effective = application_config(cfg);
    let cfg = &effective;
    let mut child = Command::new("python3")
        .env_remove("SECRETS_KEY")
        .env_remove("ADMIN_TOKEN")
        .env_remove("ADMIN_PASSWORD")
        .env_remove("POSTGRES_PASSWORD")
        .env_remove("DATABASE_URL")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .arg("/app/transport/telegram_send.py")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| anyhow!("Telegram transport could not start"))?;
    use base64::{engine::general_purpose::STANDARD, Engine};
    let uploaded=files.iter().map(|file|json!({"name":file.name,"content_type":file.content_type,"data":STANDARD.encode(&file.data)})).collect::<Vec<_>>();
    let payload = serde_json::to_vec(
        &json!({"config":cfg,"text":text,"delivery_id":delivery_id,"action":if diagnose{"diagnose"}else{"send"},"parse_mode":if html{Some("HTML")}else{None},"files":uploaded}),
    )?;
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(&payload)
        .await
        .map_err(|_| anyhow!("Telegram transport input failed"))?;
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(50), child.wait_with_output())
        .await
        .map_err(|_| anyhow!("Telegram proxy timed out; check proxy reachability and credentials"))?
        .map_err(|_| anyhow!("Telegram transport failed"))?;
    let data: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    if output.status.success() && data["ok"] == true {
        return Ok(data);
    }
    let message=match data["code"].as_str(){Some("destination")=>"Telegram destination could not be resolved; check the Chat ID or @username and bot membership/permissions",Some("credentials")=>"Telegram rejected the bot token or application credentials",Some("flood")=>"Telegram rate limit; delivery will retry",Some("proxy")=>"Telegram proxy connection failed; check address, secret and network",_=>"Telegram MTProto/WEB delivery failed; check bot permissions and destination"};
    match data["code"].as_str() {
        Some("credentials") => Err(retry_policy::transport("authentication", true, None)),
        Some("destination") => Err(retry_policy::transport("destination", true, None)),
        Some("flood") => Err(retry_policy::transport(
            "rate_limit",
            false,
            data["retry_after"]
                .as_u64()
                .map(|v| v.min(i32::MAX as u64) as i32),
        )),
        _ => Err(anyhow!(message)),
    }
}

pub async fn send(cfg: &Value, text: &str, delivery_id: Uuid, html: bool) -> Result<()> {
    transport(cfg, text, delivery_id, false, html)
        .await
        .map(|_| ())
}

pub async fn send_files(
    cfg: &Value,
    text: &str,
    delivery_id: Uuid,
    html: bool,
    files: &[attachments::File],
) -> Result<()> {
    transport_files(cfg, text, delivery_id, false, html, files)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_defaults_fill_empty_credentials_without_overwriting_channel_overrides() {
        let defaults = json!({"api_id":"12345","api_hash":Uuid::new_v4().simple().to_string()});
        let cfg = json!({"api_id":"","api_hash":"","bot_token":"private","proxy_type":"mtproto"});
        let resolved = with_application_defaults(&cfg, &defaults);
        assert_eq!(resolved["api_id"], defaults["api_id"]);
        assert_eq!(resolved["api_hash"], defaults["api_hash"]);
        assert_eq!(resolved["bot_token"], "private");
        assert_eq!(cfg["api_id"], "");
        let overridden = json!({"api_id":54321,"api_hash":Uuid::new_v4().simple().to_string()});
        assert_eq!(
            with_application_defaults(&overridden, &defaults),
            overridden
        );
    }
}
