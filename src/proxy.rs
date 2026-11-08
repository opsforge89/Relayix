use crate::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub fn mode(cfg: &Value) -> &str {
    cfg["proxy_type"].as_str().unwrap_or("none")
}
fn host(cfg: &Value) -> Result<&str> {
    let raw = cfg["proxy_host"].as_str().unwrap_or("");
    if raw.is_empty()
        || raw.len() > 253
        || raw
            .chars()
            .any(|c| c.is_whitespace() || "/@?#\\".contains(c))
        || url::Host::parse(raw).is_err()
    {
        return Err(anyhow!(
            "Proxy host must be an IP address or DNS name, without a scheme or path"
        ));
    }
    Ok(raw)
}
pub fn port(cfg: &Value) -> Result<u16> {
    cfg["proxy_port"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .or_else(|| cfg["proxy_port"].as_str().and_then(|s| s.parse().ok()))
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow!("Proxy port must be 1–65535"))
}
pub fn validate(kind: &str, cfg: &Value) -> Result<()> {
    match mode(cfg) {
        "none" => return Ok(()),
        "http" => {}
        "mtproto" | "webproxy" if kind == "telegram" => return crate::telegram::validate(cfg),
        _ => return Err(anyhow!("Unsupported proxy type for this channel")),
    }
    host(cfg)?;
    port(cfg)?;
    for key in ["proxy_username", "proxy_password"] {
        if let Some(v) = cfg.get(key) {
            if !v.is_string()
                || v.as_str().unwrap().len() > 512
                || v.as_str().unwrap().chars().any(char::is_control)
            {
                return Err(anyhow!("Invalid proxy credentials"));
            }
        }
    }
    let user = cfg["proxy_username"].as_str().unwrap_or("");
    let pass = cfg["proxy_password"].as_str().unwrap_or("");
    if user.is_empty() != pass.is_empty() {
        return Err(anyhow!(
            "Provide both proxy login and password, or leave both empty"
        ));
    }
    Ok(())
}
pub fn client(default: &reqwest::Client, cfg: &Value) -> Result<reqwest::Client> {
    if mode(cfg) != "http" {
        return Ok(default.clone());
    }
    let mut endpoint = url::Url::parse("http://localhost").unwrap();
    endpoint
        .set_host(Some(host(cfg)?))
        .map_err(|_| anyhow!("Invalid proxy host"))?;
    endpoint
        .set_port(Some(port(cfg)?))
        .map_err(|_| anyhow!("Invalid proxy port"))?;
    let mut proxy = reqwest::Proxy::all(endpoint).map_err(|_| anyhow!("Invalid HTTP proxy"))?;
    if let Some(user) = cfg["proxy_username"].as_str().filter(|s| !s.is_empty()) {
        proxy = proxy.basic_auth(user, cfg["proxy_password"].as_str().unwrap_or(""));
    }
    reqwest::Client::builder()
        .dns_resolver(Arc::new(network::policy()?))
        .no_proxy()
        .proxy(proxy)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| anyhow!("Could not configure channel proxy"))
}
pub struct SmtpTunnel {
    pub port: u16,
    task: JoinHandle<()>,
}
impl Drop for SmtpTunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn smtp_tunnel(cfg: &Value, target: &str, target_port: u16) -> Result<SmtpTunnel> {
    let proxy_port = port(cfg)?;
    let proxy_host = network::policy()?.resolve(host(cfg)?, proxy_port).await?[0]
        .ip()
        .to_string();
    let user = cfg["proxy_username"].as_str().unwrap_or("");
    let pass = cfg["proxy_password"].as_str().unwrap_or("");
    let auth = if user.is_empty() {
        String::new()
    } else {
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            STANDARD.encode(format!("{user}:{pass}"))
        )
    };
    let target = if target.contains(':') && !target.starts_with('[') {
        format!("[{target}]:{target_port}")
    } else {
        format!("{target}:{target_port}")
    };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| anyhow!("Could not create SMTP proxy tunnel"))?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        let work = async {
            let (mut local, _) = listener.accept().await?;
            let mut remote =
                TcpStream::connect((proxy_host.trim_matches(['[', ']']), proxy_port)).await?;
            remote
                .write_all(
                    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}\r\n").as_bytes(),
                )
                .await?;
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                if headers.len() >= 16384 {
                    return Err(anyhow!("Proxy headers too large"));
                }
                headers.push(remote.read_u8().await?);
            }
            let status = std::str::from_utf8(&headers)?
                .lines()
                .next()
                .and_then(|s| s.split_whitespace().nth(1))
                .unwrap_or("");
            if status != "200" {
                return Err(anyhow!("HTTP CONNECT refused"));
            }
            tokio::io::copy_bidirectional(&mut local, &mut remote).await?;
            Ok::<(), anyhow::Error>(())
        };
        let _ = tokio::time::timeout(Duration::from_secs(35), work).await;
    });
    Ok(SmtpTunnel { port, task })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_validation_and_masking() {
        let cfg = json!({"proxy_type":"http","proxy_host":"proxy.example.org","proxy_port":"3128","proxy_username":key(),"proxy_password":key()});
        assert!(validate("webhook", &cfg).is_ok());
        for (field, value) in [
            ("proxy_host", json!("http://proxy/a")),
            ("proxy_port", json!(0)),
            ("proxy_password", json!("")),
            ("proxy_username", json!("bad\r\nheader")),
            ("proxy_type", json!("mtproto")),
        ] {
            let mut bad = cfg.clone();
            bad[field] = value;
            assert!(validate("webhook", &bad).is_err());
        }
        let mut masked = cfg.clone();
        mask(&mut masked);
        assert_eq!(masked["proxy_password"], "********");
        assert_eq!(masked["proxy_username"], "********");
        restore_masked(&mut masked, &cfg);
        assert_eq!(masked, cfg);
    }
    #[tokio::test]
    async fn https_proxy_uses_authenticated_connect_without_fallback() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let user = key();
        let password = key();
        let encoded = STANDARD.encode(format!("{user}:{password}")).to_lowercase();
        let check = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
            }
            let request = String::from_utf8(bytes).unwrap().to_lowercase();
            assert!(request.starts_with("connect blocked.invalid:443 "));
            assert!(request.contains(&format!("proxy-authorization: basic {}", encoded)));
            socket
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let cfg = json!({"proxy_type":"http","proxy_host":"127.0.0.1","proxy_port":address.port(),"proxy_username":user,"proxy_password":password});
        let result = client(&reqwest::Client::new(), &cfg)
            .unwrap()
            .get("https://blocked.invalid/")
            .send()
            .await;
        assert!(result.is_err());
        check.await.unwrap();
    }
}
