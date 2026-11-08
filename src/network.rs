//! Outbound address policy. Apply at connection-time, including DNS answers.
use crate::*;
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

#[derive(Debug, Clone, Default)]
pub struct Policy {
    networks: Vec<(IpAddr, u8)>,
}
impl Policy {
    pub fn parse(raw: &str) -> Result<Self> {
        let mut networks = Vec::new();
        for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (address, bits) = entry
                .split_once('/')
                .map_or((entry, None), |(a, b)| (a, Some(b)));
            let address = IpAddr::from_str(address).map_err(|_| {
                anyhow!("OUTBOUND_ALLOWLIST must contain IP addresses or CIDR networks")
            })?;
            let max = if address.is_ipv4() { 32 } else { 128 };
            let bits = bits
                .map(str::parse::<u8>)
                .transpose()
                .map_err(|_| anyhow!("Invalid OUTBOUND_ALLOWLIST prefix"))?
                .unwrap_or(max);
            if bits > max {
                return Err(anyhow!("Invalid OUTBOUND_ALLOWLIST prefix"));
            }
            networks.push((address, bits));
        }
        Ok(Self { networks })
    }
    pub fn allows(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            _ => ip,
        };
        public(ip)
            || self.networks.iter().any(|(base, bits)| match (ip, *base) {
                (IpAddr::V4(a), IpAddr::V4(b)) => {
                    u32::from(a).checked_shr(32 - u32::from(*bits)).unwrap_or(0)
                        == u32::from(b).checked_shr(32 - u32::from(*bits)).unwrap_or(0)
                }
                (IpAddr::V6(a), IpAddr::V6(b)) => {
                    u128::from(a)
                        .checked_shr(128 - u32::from(*bits))
                        .unwrap_or(0)
                        == u128::from(b)
                            .checked_shr(128 - u32::from(*bits))
                            .unwrap_or(0)
                }
                _ => false,
            })
    }
    pub async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
        let host = host.trim_matches(['[', ']']);
        let addresses: Vec<_> = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| anyhow!("Destination DNS lookup timed out"))?
        .map_err(|_| anyhow!("Destination DNS lookup failed"))?
        .collect();
        if addresses.is_empty() || addresses.iter().any(|a| !self.allows(a.ip())) {
            return Err(anyhow!("Destination address blocked by outbound policy"));
        }
        Ok(addresses)
    }
    pub async fn validate(&self, cfg: &Value) -> Result<()> {
        if let Some(raw) = cfg["url"].as_str() {
            let url = url::Url::parse(raw)?;
            self.resolve(
                url.host_str()
                    .ok_or_else(|| anyhow!("Missing destination host"))?,
                url.port_or_known_default().unwrap_or(443),
            )
            .await?;
        }
        if let Some(host) = cfg["host"].as_str() {
            self.resolve(host, providers::smtp_port(cfg)?).await?;
        }
        if proxy::mode(cfg) == "webproxy" {
            let raw = cfg["webproxy_address"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches("https://");
            let url = url::Url::parse(&format!("https://{raw}"))?;
            self.resolve(
                url.host_str()
                    .ok_or_else(|| anyhow!("Missing WEB proxy host"))?,
                443,
            )
            .await?;
        } else if proxy::mode(cfg) != "none" {
            self.resolve(cfg["proxy_host"].as_str().unwrap_or(""), proxy::port(cfg)?)
                .await?;
        }
        Ok(())
    }
}
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99) || (b == 2 && c == 0)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            // Only global unicast, excluding documentation and transition mechanisms.
            (s[0] & 0xe000) == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}
impl reqwest::dns::Resolve for Policy {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let policy = self.clone();
        Box::pin(async move {
            let addresses = policy
                .resolve(name.as_str(), 0)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
pub fn policy() -> Result<Policy> {
    Policy::parse(&env::var("OUTBOUND_ALLOWLIST").unwrap_or_default())
}
pub async fn validate(cfg: &Value) -> Result<()> {
    policy()?.validate(cfg).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_and_special_addresses_are_blocked() {
        let p = Policy::default();
        for ip in [
            "127.0.0.1",
            "10.2.3.4",
            "169.254.169.254",
            "172.31.0.1",
            "192.168.0.1",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2002:7f00:1::",
            "2001:db8::1",
        ] {
            assert!(!p.allows(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(p.allows(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn allowlist_is_explicit_and_bounded() {
        let p = Policy::parse("10.2.0.0/16,::1/128").unwrap();
        assert!(p.allows("10.2.1.1".parse().unwrap()));
        assert!(p.allows("::1".parse().unwrap()));
        assert!(!p.allows("10.3.1.1".parse().unwrap()));
        assert!(!p.allows("::2".parse().unwrap()));
        for raw in ["localhost", "10.0.0.1/33", "::1/129"] {
            assert!(Policy::parse(raw).is_err());
        }
    }
    #[tokio::test]
    async fn resolved_private_destinations_are_blocked() {
        assert!(Policy::default().resolve("localhost", 80).await.is_err());
        assert!(Policy::default().resolve("[::1]", 80).await.is_err());
        assert!(Policy::parse("127.0.0.1,::1")
            .unwrap()
            .resolve("localhost", 80)
            .await
            .is_ok());
    }
}
