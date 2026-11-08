//! Request boundaries, maintenance and process lifecycle.
use crate::*;
use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::Instant,
};

pub fn setting(name: &str, default: u32, min: u32, max: u32) -> Result<u32> {
    let value = env::var(name)
        .map(|v| v.parse::<u32>())
        .unwrap_or(Ok(default))
        .map_err(|_| anyhow!("{name} must be between {min} and {max}"))?;
    if !(min..=max).contains(&value) {
        return Err(anyhow!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name,value) in [
        ("x-content-type-options","nosniff"), ("x-frame-options","DENY"),
        ("referrer-policy","no-referrer"),
        ("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"),
    ] {response.headers_mut().insert(axum::http::HeaderName::from_static(name),axum::http::HeaderValue::from_static(value));}
    response
}
#[derive(Default)]
struct Limiter {
    entries: HashMap<(IpAddr, bool), (Instant, u32)>,
}
impl Limiter {
    fn allow(&mut self, ip: IpAddr, login: bool, now: Instant) -> bool {
        let ip = client_ip::ClientIp(ip).key();
        // Fixed windows bound both request work and limiter memory. Never evict active quotas.
        if self.entries.len() >= 10000 {
            self.entries
                .retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        }
        if self.entries.len() >= 10000 && !self.entries.contains_key(&(ip, login)) {
            return false;
        }
        let (start, count) = self.entries.entry((ip, login)).or_insert((now, 0));
        if now.duration_since(*start) >= Duration::from_secs(60) {
            *start = now;
            *count = 0;
        }
        let limit = if login { 60 } else { 600 };
        if *count >= limit {
            return false;
        }
        *count += 1;
        true
    }
}
pub async fn rate_limit(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path == "/api/auth/login" || path.starts_with("/h/") {
        static LIMITS: OnceLock<Mutex<Limiter>> = OnceLock::new();
        let Some(client) = request.extensions().get::<client_ip::ClientIp>() else {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Client address unavailable",
            )
            .into_response();
        };
        let allowed = LIMITS
            .get_or_init(|| Mutex::new(Limiter::default()))
            .lock()
            .map(|mut l| l.allow(client.key(), path == "/api/auth/login", Instant::now()))
            .unwrap_or(false);
        if !allowed {
            let mut response = err(
                StatusCode::TOO_MANY_REQUESTS,
                "Request rate exceeded; retry in 60 seconds",
            )
            .into_response();
            response
                .headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from_static("60"));
            return response;
        }
    }
    next.run(request).await
}
pub async fn pause(stop: &mut tokio::sync::watch::Receiver<bool>, duration: Duration) {
    tokio::select! {_=tokio::time::sleep(duration)=>{},_=stop.changed()=>{}}
}
pub async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("Shutdown requested; draining requests and delivery workers");
}
pub async fn maintenance(db: PgPool, days: u32, mut stop: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *stop.borrow() {
            break;
        }
        let result:Result<()>=async {
            sqlx::query("DELETE FROM delivery_retry_batches WHERE expires_at<NOW()").execute(&db).await?;
            sqlx::query("DELETE FROM login_attempts WHERE window_started_at<NOW()-INTERVAL '15 minutes'").execute(&db).await?;
            sqlx::query("DELETE FROM sessions WHERE expires_at<NOW()").execute(&db).await?;
            if days>0 {
                // Bound each sweep and preserve events that still have pending work.
                sqlx::query("DELETE FROM events WHERE id IN (SELECT e.id FROM events e WHERE e.created_at<NOW()-($1::int*INTERVAL '1 day') AND NOT EXISTS(SELECT 1 FROM deliveries d WHERE d.event_id=e.id AND d.status IN ('queued','processing')) ORDER BY e.created_at LIMIT 1000)").bind(days as i32).execute(&db).await?;
                sqlx::query("DELETE FROM alert_deliveries WHERE id IN (SELECT id FROM alert_deliveries WHERE created_at<NOW()-($1::int*INTERVAL '1 day') AND status IN ('sent','failed') ORDER BY created_at LIMIT 1000)").bind(days as i32).execute(&db).await?;
            }
            Ok(())
        }.await;
        if let Err(error) = result {
            tracing::warn!(%error,"Maintenance failed");
        }
        pause(&mut stop, Duration::from_secs(60)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotating_ipv6_addresses_do_not_fill_the_limiter() {
        let mut limiter = Limiter::default();
        let now = Instant::now();
        for suffix in 1..=20000u128 {
            let address = IpAddr::V6(std::net::Ipv6Addr::from(
                (0x20010db800010002u128 << 64) | suffix,
            ));
            assert_eq!(limiter.allow(address, true, now), suffix <= 60);
        }
        assert_eq!(limiter.entries.len(), 1);
        assert!(limiter.allow("2001:db8:1:3::1".parse().unwrap(), true, now));
        assert!(limiter.allow("192.0.2.1".parse().unwrap(), true, now));
        for _ in 1..60 {
            assert!(limiter.allow("::ffff:192.0.2.1".parse().unwrap(), true, now));
        }
        assert!(!limiter.allow("192.0.2.1".parse().unwrap(), true, now));
    }
    #[test]
    fn request_limits_isolate_clients_and_expire() {
        let mut limiter = Limiter::default();
        let now = Instant::now();
        let a = "192.0.2.1".parse().unwrap();
        let b = "192.0.2.2".parse().unwrap();
        for _ in 0..60 {
            assert!(limiter.allow(a, true, now));
        }
        assert!(!limiter.allow(a, true, now));
        assert!(limiter.allow(b, true, now));
        assert!(limiter.allow(a, false, now));
        assert!(limiter.allow(a, true, now + Duration::from_secs(60)));
        for _ in 1..600 {
            assert!(limiter.allow(a, false, now));
        }
        assert!(!limiter.allow(a, false, now));
    }
}
