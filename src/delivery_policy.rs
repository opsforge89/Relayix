//! Route-owned quiet hours and durable grouping windows. Urgent events bypass grouping and quiet hours.
use crate::*;
use chrono::Timelike;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub ttl_seconds: i64,
    pub quiet_enabled: bool,
    pub timezone: String,
    pub quiet_start: String,
    pub quiet_end: String,
    pub urgent_priority: i32,
    pub dedup_seconds: i64,
    pub digest_seconds: i64,
    pub digest_below_priority: i32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl_seconds: 0,
            quiet_enabled: false,
            timezone: "UTC".into(),
            quiet_start: "22:00".into(),
            quiet_end: "08:00".into(),
            urgent_priority: 4,
            dedup_seconds: 0,
            digest_seconds: 0,
            digest_below_priority: 3,
        }
    }
}
impl Settings {
    pub fn parse(value: &Value) -> Result<Self> {
        let settings: Self = serde_json::from_value(value.clone())?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn validate(&self) -> Result<()> {
        if !(0..=2592000).contains(&self.ttl_seconds) {return Err(anyhow!("TTL must be 0–2592000 seconds"));}
        self.timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| anyhow!("Use an IANA timezone such as Europe/Moscow"))?;
        let start = minute(&self.quiet_start)?;
        let end = minute(&self.quiet_end)?;
        if self.quiet_enabled && start == end {
            return Err(anyhow!(
                "Quiet hours must have different start and end times"
            ));
        }
        if !(1..=5).contains(&self.urgent_priority)
            || !(1..=5).contains(&self.digest_below_priority)
            || !(0..=300).contains(&self.dedup_seconds)
            || (self.dedup_seconds > 0 && self.dedup_seconds < 5)
            || !(0..=86400).contains(&self.digest_seconds)
            || (self.digest_seconds > 0 && self.digest_seconds < 60)
        {
            return Err(anyhow!("Use priorities 1–5, deduplication 5–300 seconds or 0, and digest 60–86400 seconds or 0"));
        }
        Ok(())
    }
    fn quiet(&self, at: DateTime<Utc>, priority: i32) -> bool {
        if !self.quiet_enabled || priority >= self.urgent_priority {
            return false;
        }
        let Ok(zone) = self.timezone.parse::<chrono_tz::Tz>() else {
            return false;
        };
        let time = at.with_timezone(&zone);
        let now = time.hour() * 60 + time.minute();
        let start = minute(&self.quiet_start).unwrap_or(0);
        let end = minute(&self.quiet_end).unwrap_or(0);
        if start < end {
            now >= start && now < end
        } else {
            now >= start || now < end
        }
    }
    pub fn allowed_at(&self, at: DateTime<Utc>, priority: i32) -> DateTime<Utc> {
        if !self.quiet(at, priority) {
            return at;
        }
        // Walk real UTC minutes: ambiguous and missing local clock times remain DST-safe.
        let mut next = at + chrono::Duration::seconds(60 - at.second() as i64)
            - chrono::Duration::nanoseconds(at.nanosecond() as i64);
        for _ in 0..(48 * 60) {
            if !self.quiet(next, priority) {
                return next;
            }
            next += chrono::Duration::minutes(1);
        }
        next
    }
    pub fn grouping(&self, priority: i32) -> Option<(&'static str, i64)> {
        if priority >= self.urgent_priority {
            return None;
        }
        if self.digest_seconds > 0 && priority < self.digest_below_priority {
            return Some(("digest", self.digest_seconds));
        }
        if self.dedup_seconds > 0 {
            return Some(("dedup", self.dedup_seconds));
        }
        None
    }
}
fn minute(value: &str) -> Result<u32> {
    let time = chrono::NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| anyhow!("Use HH:MM for quiet hours"))?;
    Ok(time.hour() * 60 + time.minute())
}
#[derive(Default, Serialize)]
pub struct Schedule {
    #[serde(skip_serializing_if="Option::is_none")]
    ttl: Option<i64>,
    delay: Option<i64>,
    deliver_at: Option<DateTime<Utc>>,
}
impl Schedule {
    pub fn parse(headers: &HeaderMap) -> Result<Self> {
        fn value<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>> {
            if headers.get_all(name).iter().count() > 1 {
                return Err(anyhow!("Scheduling headers must occur only once"));
            }
            headers
                .get(name)
                .map(|v| {
                    v.to_str()
                        .map(str::trim)
                        .map_err(|_| anyhow!("Invalid scheduling header"))
                })
                .transpose()
        }
        let ttl=value(headers,"x-relay-ttl")?.map(|raw| {
            if raw.is_empty() || !raw.bytes().all(|c|c.is_ascii_digit()) {return Err(anyhow!("X-Relay-TTL must be 1–2592000 whole seconds"));}
            raw.parse::<i64>().ok().filter(|v|(1..=2592000).contains(v)).ok_or_else(||anyhow!("X-Relay-TTL must be 1–2592000 whole seconds"))
        }).transpose()?;
        let delay = value(headers, "x-relay-delay")?;
        let deliver_at = value(headers, "x-relay-deliver-at")?;
        if delay.is_some() && deliver_at.is_some() {
            return Err(anyhow!("Use X-Relay-Delay or X-Relay-Deliver-At, not both"));
        }
        let delay = delay
            .map(|v| {
                if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(anyhow!("X-Relay-Delay must be 0–2592000 whole seconds"));
                }
                v.parse::<i64>()
                    .ok()
                    .filter(|s| (0..=2592000).contains(s))
                    .ok_or_else(|| anyhow!("X-Relay-Delay must be 0–2592000 whole seconds"))
            })
            .transpose()?;
        let deliver_at = deliver_at
            .map(|v| {
                DateTime::parse_from_rfc3339(v)
                    .map(|at| at.with_timezone(&Utc))
                    .map_err(|_| anyhow!("X-Relay-Deliver-At must be an RFC 3339 timestamp"))
            })
            .transpose()?;
        Ok(Self { delay, deliver_at, ttl })
    }
    pub fn explicit(&self) -> bool {
        self.delay.is_some() || self.deliver_at.is_some() || self.ttl.is_some()
    }
    pub fn expiry(&self,route_ttl:i64,now:DateTime<Utc>)->Option<DateTime<Utc>> {
        let ttl=match (self.ttl,route_ttl) { (Some(value),route) if route>0=>value.min(route),(Some(value),_)=>value,(None,route)=>route };
        (ttl>0).then(||now+chrono::Duration::seconds(ttl))
    }
    pub fn requested_time(&self, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        let at = self
            .deliver_at
            .unwrap_or_else(|| now + chrono::Duration::seconds(self.delay.unwrap_or(0)));
        if at > now + chrono::Duration::days(30) {
            return Err(anyhow!("Schedule notifications at most 30 days ahead"));
        }
        Ok(at.max(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
    #[test]
    fn quiet_hours_wrap_midnight_and_preserve_urgent_delivery() {
        let settings = Settings {
            quiet_enabled: true,
            timezone: "Europe/Moscow".into(),
            ..Settings::default()
        };
        let now = at("2026-11-09T20:00:00Z");
        assert_eq!(settings.allowed_at(now, 3), at("2026-11-10T05:00:00Z"));
        assert_eq!(settings.allowed_at(now, 4), now);
        assert_eq!(
            settings.allowed_at(at("2026-11-10T05:00:00Z"), 1),
            at("2026-11-10T05:00:00Z")
        );
    }
    #[test]
    fn quiet_hours_handle_daylight_saving_gaps_and_repeated_hours() {
        let settings = Settings {
            quiet_enabled: true,
            timezone: "America/New_York".into(),
            quiet_start: "01:00".into(),
            quiet_end: "03:00".into(),
            ..Settings::default()
        };
        assert_eq!(
            settings.allowed_at(at("2026-03-08T06:30:00Z"), 1),
            at("2026-03-08T07:00:00Z")
        );
        assert_eq!(
            settings.allowed_at(at("2026-11-01T05:30:00Z"), 1),
            at("2026-11-01T08:00:00Z")
        );
    }
    #[test]
    fn ttl_is_bounded_and_cannot_extend_the_route_limit() {
        let now=at("2026-11-11T12:00:00Z");let mut headers=HeaderMap::new();
        assert!(Schedule::default().expiry(0,now).is_none());
        assert_eq!(Schedule::default().expiry(60,now),Some(now+chrono::Duration::seconds(60)));
        for value in ["0","-1","2592001","1.5","5m",""] {headers.insert("x-relay-ttl",value.parse().unwrap());assert!(Schedule::parse(&headers).is_err());}
        headers.insert("x-relay-ttl","120".parse().unwrap());
        let schedule=Schedule::parse(&headers).unwrap();
        assert_eq!(schedule.expiry(60,now),Some(now+chrono::Duration::seconds(60)));
        assert_eq!(schedule.expiry(0,now),Some(now+chrono::Duration::seconds(120)));
        headers.append("x-relay-ttl","60".parse().unwrap());assert!(Schedule::parse(&headers).is_err());
        assert_eq!(serde_json::to_value(Schedule::default()).unwrap(),json!({"delay":null,"deliver_at":null}));
    }
    #[test]
    fn explicit_schedules_are_bounded_and_unambiguous() {
        let now = at("2026-11-09T12:00:00Z");
        let mut headers = HeaderMap::new();
        headers.insert("x-relay-delay", "60".parse().unwrap());
        assert_eq!(
            Schedule::parse(&headers)
                .unwrap()
                .requested_time(now)
                .unwrap(),
            now + chrono::Duration::minutes(1)
        );
        for value in ["-1", "2592001", "1.5", "5m", ""] {
            headers.insert("x-relay-delay", value.parse().unwrap());
            assert!(Schedule::parse(&headers).is_err());
        }
        headers.insert("x-relay-delay", "1".parse().unwrap());
        headers.insert(
            "x-relay-deliver-at",
            "2026-11-09T13:00:00Z".parse().unwrap(),
        );
        assert!(Schedule::parse(&headers).is_err());
        headers.remove("x-relay-delay");
        headers.insert("x-relay-deliver-at", "invalid".parse().unwrap());
        assert!(Schedule::parse(&headers).is_err());
        headers.insert(
            "x-relay-deliver-at",
            "2026-12-10T12:00:00Z".parse().unwrap(),
        );
        assert!(Schedule::parse(&headers)
            .unwrap()
            .requested_time(now)
            .is_err());
        headers.insert(
            "x-relay-deliver-at",
            "2026-11-08T12:00:00Z".parse().unwrap(),
        );
        assert_eq!(
            Schedule::parse(&headers)
                .unwrap()
                .requested_time(now)
                .unwrap(),
            now
        );
        headers.append(
            "x-relay-deliver-at",
            "2026-11-09T13:00:00Z".parse().unwrap(),
        );
        assert!(Schedule::parse(&headers).is_err());
        headers.clear();
        assert_eq!(
            Schedule::parse(&headers)
                .unwrap()
                .requested_time(now)
                .unwrap(),
            now
        );
    }
}
