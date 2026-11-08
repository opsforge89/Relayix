//! One client identity for request quotas and failed-login windows.
use crate::*;
use axum::{
    extract::{ConnectInfo, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);
impl ClientIp {
    pub fn key(self) -> IpAddr {
        match canonical(self.0) {
            IpAddr::V6(address) => {
                IpAddr::V6(Ipv6Addr::from(u128::from(address) & (u128::MAX << 64)))
            }
            address => address,
        }
    }
}
fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(address),
        _ => address,
    }
}
#[derive(Clone, Default)]
pub struct TrustedProxies {
    networks: Vec<(IpAddr, u32)>,
}
impl TrustedProxies {
    pub fn parse(raw: &str) -> Result<Self> {
        let mut networks = Vec::new();
        for entry in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (address, prefix) = entry
                .split_once('/')
                .map_or((entry, None), |(a, p)| (a, Some(p)));
            let address: IpAddr = address
                .parse()
                .map_err(|_| anyhow!("TRUSTED_PROXIES requires IP addresses or CIDR networks"))?;
            let width = if address.is_ipv4() { 32 } else { 128 };
            let prefix = prefix
                .map(str::parse::<u32>)
                .transpose()
                .map_err(|_| anyhow!("Invalid TRUSTED_PROXIES prefix"))?
                .unwrap_or(width);
            if prefix > width {
                return Err(anyhow!("Invalid TRUSTED_PROXIES prefix"));
            }
            let (address, prefix) = match address {
                IpAddr::V6(v) if v.to_ipv4_mapped().is_some() => {
                    if prefix < 96 {
                        return Err(anyhow!(
                            "Mapped IPv4 TRUSTED_PROXIES prefixes must be at least /96"
                        ));
                    }
                    (canonical(address), prefix - 96)
                }
                _ => (address, prefix),
            };
            if prefix == 0 {
                return Err(anyhow!("TRUSTED_PROXIES must not trust all addresses (/0)"));
            }
            networks.push((address, prefix));
        }
        Ok(Self { networks })
    }
    pub fn is_empty(&self) -> bool {
        self.networks.is_empty()
    }
    fn contains(&self, address: IpAddr) -> bool {
        let address = canonical(address);
        self.networks
            .iter()
            .any(|(base, prefix)| match (address, *base) {
                (IpAddr::V4(a), IpAddr::V4(b)) => {
                    u32::from(a).checked_shr(32 - prefix).unwrap_or(0)
                        == u32::from(b).checked_shr(32 - prefix).unwrap_or(0)
                }
                (IpAddr::V6(a), IpAddr::V6(b)) => {
                    u128::from(a).checked_shr(128 - prefix).unwrap_or(0)
                        == u128::from(b).checked_shr(128 - prefix).unwrap_or(0)
                }
                _ => false,
            })
    }
    pub fn client(&self, peer: IpAddr, headers: &HeaderMap) -> Result<ClientIp> {
        let mut client = canonical(peer);
        if !self.contains(client) {
            return Ok(ClientIp(client));
        }
        let mut chain = Vec::new();
        let mut size = 0;
        for value in headers.get_all("x-forwarded-for") {
            size += value.as_bytes().len();
            if size > 4096 {
                return Err(anyhow!("X-Forwarded-For is too large"));
            }
            let value = value
                .to_str()
                .map_err(|_| anyhow!("Invalid X-Forwarded-For"))?;
            for hop in value.split(',') {
                if chain.len() >= 32 {
                    return Err(anyhow!("Too many X-Forwarded-For hops"));
                }
                chain.push(canonical(hop.trim().parse::<IpAddr>().map_err(|_| {
                    anyhow!("X-Forwarded-For must contain IP addresses")
                })?));
            }
        }
        for address in chain.into_iter().rev() {
            if !self.contains(client) {
                break;
            }
            client = address;
        }
        Ok(ClientIp(client))
    }
}
pub async fn resolve(
    State(proxies): State<Arc<TrustedProxies>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(peer) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip())
    else {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Connection address unavailable",
        )
        .into_response();
    };
    match proxies.client(peer, request.headers()) {
        Ok(client) => {
            tracing::debug!(peer_ip=%peer,client_ip=%client.0,"request client address");
            request.extensions_mut().insert(client);
            next.run(request).await
        }
        Err(error) => err(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", value.parse().unwrap());
        h
    }
    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }
    #[test]
    fn forwarded_headers_require_a_trusted_peer() {
        let p = TrustedProxies::parse("127.0.0.1,10.0.0.0/8,2001:db8:ffff::1/128").unwrap();
        assert_eq!(
            p.client(ip("192.0.2.1"), &headers("203.0.113.1"))
                .unwrap()
                .0,
            ip("192.0.2.1")
        );
        assert_eq!(
            p.client(ip("192.0.2.1"), &headers("garbage")).unwrap().0,
            ip("192.0.2.1")
        );
        assert_eq!(
            TrustedProxies::default()
                .client(ip("127.0.0.1"), &headers("203.0.113.1"))
                .unwrap()
                .0,
            ip("127.0.0.1")
        );
        assert_eq!(
            p.client(ip("127.0.0.1"), &HeaderMap::new()).unwrap().0,
            ip("127.0.0.1")
        );
    }
    #[test]
    fn chain_stops_at_the_first_untrusted_hop_from_the_right() {
        let p = TrustedProxies::parse("127.0.0.1,10.0.0.0/8").unwrap();
        assert_eq!(
            p.client(ip("127.0.0.1"), &headers("203.0.113.9,192.0.2.1,10.1.2.3"))
                .unwrap()
                .0,
            ip("192.0.2.1")
        );
        let mut h = headers("203.0.113.9,192.0.2.1");
        h.append("x-forwarded-for", "10.1.2.3".parse().unwrap());
        assert_eq!(p.client(ip("127.0.0.1"), &h).unwrap().0, ip("192.0.2.1"));
        assert_eq!(
            p.client(ip("::ffff:127.0.0.1"), &headers("::ffff:192.0.2.1"))
                .unwrap()
                .0,
            ip("192.0.2.1")
        );
    }
    #[test]
    fn explicit_ipv6_and_mapped_proxy_networks_are_trusted() {
        let p = TrustedProxies::parse("2001:db8:ffff::/64,::ffff:127.0.0.1/128").unwrap();
        assert_eq!(
            p.client(ip("2001:db8:ffff::2"), &headers("2001:db8:1:2::1234"))
                .unwrap()
                .0,
            ip("2001:db8:1:2::1234")
        );
        assert_eq!(
            p.client(ip("127.0.0.1"), &headers("192.0.2.1")).unwrap().0,
            ip("192.0.2.1")
        );
        assert_eq!(
            p.client(ip("8.8.8.8"), &headers("192.0.2.1")).unwrap().0,
            ip("8.8.8.8")
        );
    }
    #[test]
    fn invalid_trusted_chains_and_configurations_fail_closed() {
        let p = TrustedProxies::parse("127.0.0.1").unwrap();
        for value in [
            "",
            "unknown",
            "192.0.2.1:8080",
            "192.0.2.1,,10.0.0.1",
            "example.org",
        ] {
            assert!(p.client(ip("127.0.0.1"), &headers(value)).is_err());
        }
        assert!(p
            .client(ip("127.0.0.1"), &headers(&vec!["192.0.2.1"; 33].join(",")))
            .is_err());
        assert!(p
            .client(ip("127.0.0.1"), &headers(&"1".repeat(4097)))
            .is_err());
        for raw in [
            "localhost",
            "0.0.0.0/0",
            "192.0.2.1/0",
            "::/0",
            "::ffff:0.0.0.0/96",
            "127.0.0.1/33",
            "::1/129",
            "::ffff:127.0.0.1/64",
        ] {
            assert!(TrustedProxies::parse(raw).is_err());
        }
    }
    #[test]
    fn ipv6_subnets_share_keys_without_trusting_the_entire_subnet() {
        let p = TrustedProxies::parse("2001:db8:ffff::1").unwrap();
        assert_eq!(
            p.client(ip("2001:db8:ffff::2"), &headers("192.0.2.1"))
                .unwrap()
                .0,
            ip("2001:db8:ffff::2")
        );
        assert_eq!(
            ClientIp(ip("2001:db8:1:2::1")).key(),
            ClientIp(ip("2001:db8:1:2:ffff:ffff:ffff:ffff")).key()
        );
        assert_ne!(
            ClientIp(ip("2001:db8:1:2::1")).key(),
            ClientIp(ip("2001:db8:1:3::1")).key()
        );
        assert_eq!(ClientIp(ip("::ffff:192.0.2.1")).key(), ip("192.0.2.1"));
    }
}
