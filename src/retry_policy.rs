//! Safe transport failures and retry scheduling shared by notification queues.
use crate::*;
use std::fmt;

const MAX_PROVIDER_DELAY: i32 = 86_400;

#[derive(Debug)]
pub struct Failure {
    pub category: &'static str,
    pub permanent: bool,
    pub retry_after: Option<i32>,
    pub status: Option<u16>,
    message: &'static str,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(status) = self.status {
            write!(f, " (HTTP {status})")?;
        }
        Ok(())
    }
}
impl std::error::Error for Failure {}

pub fn permanent(message: &'static str) -> anyhow::Error {
    Failure {
        category: "configuration",
        permanent: true,
        retry_after: None,
        status: None,
        message,
    }
    .into()
}
pub fn transport(category: &'static str, permanent: bool, delay: Option<i32>) -> anyhow::Error {
    Failure {
        category,
        permanent,
        retry_after: delay.map(|value| value.clamp(0, MAX_PROVIDER_DELAY)),
        status: None,
        message: match category {
            "authentication" => "Credentials rejected; update channel credentials",
            "destination" => "Destination rejected; check recipient and permissions",
            "rate_limit" => "Provider rate limit; waiting before retry",
            _ => "Temporary transport failure; check connectivity and server availability",
        },
    }
    .into()
}
fn seconds(value: u64) -> i32 {
    value.min(MAX_PROVIDER_DELAY as u64) as i32
}
fn retry_after(value: &str, now: DateTime<Utc>) -> Option<i32> {
    if let Ok(value) = value.trim().parse::<u64>() {
        return Some(seconds(value));
    }
    DateTime::parse_from_rfc2822(value.trim()).ok().map(|date| {
        ((date.with_timezone(&Utc) - now)
            .num_milliseconds()
            .max(0)
            .saturating_add(999)
            / 1000)
            .min(MAX_PROVIDER_DELAY as i64) as i32
    })
}
fn http_failure(status: u16, delay: Option<i32>) -> anyhow::Error {
    let (category, permanent, message) = match status {
        401 | 403 => (
            "authentication",
            true,
            "Credentials or permissions rejected; update channel settings",
        ),
        429 => (
            "rate_limit",
            false,
            "Provider rate limit; waiting before retry",
        ),
        408 | 409 | 425 | 500..=599 => ("temporary", false, "Temporary remote server failure"),
        _ => (
            "destination",
            true,
            "Remote request rejected; check destination and message settings",
        ),
    };
    Failure {
        category,
        permanent,
        retry_after: delay.map(|value| value.clamp(0, MAX_PROVIDER_DELAY)),
        status: Some(status),
        message,
    }
    .into()
}
/// Inspect only safe status/rate-limit fields. Never persist remote bodies or credential URLs.
pub async fn response(mut response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let mut delay = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| retry_after(v, Utc::now()));
    if status.as_u16() == 429 {
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            if body.len() + chunk.len() > 16384 {
                break;
            }
            body.extend_from_slice(&chunk);
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&body) {
            let supplied = value["parameters"]["retry_after"]
                .as_u64()
                .map(seconds)
                .or_else(|| {
                    value["retry_after_ms"]
                        .as_u64()
                        .map(|v| seconds(v.saturating_add(999) / 1000))
                });
            delay = delay.into_iter().chain(supplied).max();
        }
    }
    Err(http_failure(status.as_u16(), delay))
}

pub struct Plan {
    pub permanent: bool,
    pub delay: i32,
    pub category: &'static str,
    pub signature: String,
    pub cooldown: i32,
    pub affects_channel: bool,
}
pub fn plan(error: &anyhow::Error, attempt: i32, base: i32) -> Plan {
    let failure = error.downcast_ref::<Failure>();
    let backoff = base
        .max(1)
        .saturating_mul(2_i32.pow(attempt.clamp(0, 8) as u32));
    let provider_delay = failure
        .and_then(|f| f.retry_after)
        .map(|value| value.clamp(0, MAX_PROVIDER_DELAY));
    let delay = backoff.max(provider_delay.unwrap_or(0));
    let affects_channel = failure.is_none_or(|f| match f.status {
        Some(401 | 403 | 408 | 429 | 500..=599) => true,
        Some(_) => false,
        None => f.category != "destination",
    });
    let category = failure.map(|f| f.category).unwrap_or("temporary");
    Plan {
        permanent: failure.is_some_and(|f| f.permanent),
        delay,
        category,
        signature: format!(
            "{category}:{}",
            error.to_string().chars().take(256).collect::<String>()
        ),
        affects_channel,
        cooldown: if affects_channel && (category == "rate_limit" || provider_delay.is_some()) {
            delay
        } else {
            0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_limits_never_retry_before_the_provider_deadline() {
        let now = DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(retry_after("120", now), Some(120));
        assert_eq!(retry_after("Sun, 08 Nov 2026 12:02:00 GMT", now), Some(120));
        assert_eq!(retry_after("Sun, 08 Nov 2026 11:59:00 GMT", now), Some(0));
        assert_eq!(retry_after("invalid", now), None);
        let p = plan(&http_failure(429, Some(120)), 1, 5);
        assert_eq!(p.delay, 120);
        assert_eq!(p.cooldown, 120);
        assert!(!p.permanent);
        assert!(plan(&http_failure(503, Some(2)), 2, 5).delay >= 20);
    }
    #[test]
    fn provider_delays_are_bounded_for_every_transport_and_date_format() {
        let now = DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(retry_after("1000000", now), Some(MAX_PROVIDER_DELAY));
        assert_eq!(
            retry_after("18446744073709551615", now),
            Some(MAX_PROVIDER_DELAY)
        );
        assert_eq!(
            retry_after("Tue, 10 Nov 2026 12:00:00 GMT", now),
            Some(MAX_PROVIDER_DELAY)
        );
        for error in [
            http_failure(429, Some(i32::MAX)),
            transport("rate_limit", false, Some(i32::MAX)),
        ] {
            let plan = plan(&error, 1, 5);
            assert_eq!(plan.delay, MAX_PROVIDER_DELAY);
            assert_eq!(plan.cooldown, MAX_PROVIDER_DELAY);
        }
    }
    #[test]
    fn message_failures_do_not_pause_channels_even_with_retry_headers() {
        for status in [400, 404, 409, 413, 422, 425] {
            let plan = plan(&http_failure(status, Some(3600)), 3, 5);
            assert!(!plan.affects_channel);
            assert_eq!(plan.cooldown, 0);
        }
        for status in [401, 403, 408, 429, 500, 502, 503, 504] {
            assert!(plan(&http_failure(status, None), 3, 5).affects_channel);
        }
        assert!(plan(&anyhow!("Connection failed"), 3, 5).affects_channel);
        assert!(plan(&transport("authentication", true, None), 3, 5).affects_channel);
        assert!(!plan(&transport("destination", true, None), 3, 5).affects_channel);
    }
    #[tokio::test]
    async fn response_extracts_safe_delays_without_persisting_remote_bodies() {
        for (body, expected) in [
            (
                r#"{"parameters":{"retry_after":120},"secret":"do-not-store"}"#,
                120,
            ),
            (r#"{"retry_after_ms":120001}"#, 121),
            ("invalid JSON", 20),
            (
                r#"{"retry_after_ms":18446744073709551615}"#,
                MAX_PROVIDER_DELAY,
            ),
            (
                r#"{"parameters":{"retry_after":1000000}}"#,
                MAX_PROVIDER_DELAY,
            ),
        ] {
            let response = reqwest::Response::from(
                axum::http::Response::builder()
                    .status(429)
                    .header("retry-after", "20")
                    .body(body)
                    .unwrap(),
            );
            let error = super::response(response).await.unwrap_err();
            assert_eq!(plan(&error, 1, 1).delay, expected);
            assert!(!error.to_string().contains("do-not-store"));
        }
        let response = reqwest::Response::from(
            axum::http::Response::builder()
                .status(403)
                .body("private token echoed by server")
                .unwrap(),
        );
        let error = super::response(response).await.unwrap_err();
        assert!(plan(&error, 1, 1).permanent);
        assert!(!error.to_string().contains("private token"));
    }
    #[test]
    fn permanent_errors_stop_while_transient_errors_keep_the_attempt_budget() {
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(plan(&http_failure(status, None), 1, 5).permanent);
        }
        for status in [408, 409, 425, 429, 500, 502, 503, 504] {
            assert!(!plan(&http_failure(status, None), 1, 5).permanent);
        }
        assert!(!plan(&anyhow!("Delivery attempt timed out"), 1, 5).permanent);
    }
}
