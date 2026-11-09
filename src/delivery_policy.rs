//! Route-owned quiet hours and durable grouping windows. Urgent events bypass grouping and quiet hours.
use crate::*;
use chrono::Timelike;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
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
        Self { quiet_enabled:false, timezone:"UTC".into(), quiet_start:"22:00".into(), quiet_end:"08:00".into(), urgent_priority:4, dedup_seconds:0, digest_seconds:0, digest_below_priority:3 }
    }
}
impl Settings {
    pub fn parse(value: &Value) -> Result<Self> {
        let settings: Self = serde_json::from_value(value.clone())?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn validate(&self) -> Result<()> {
        self.timezone.parse::<chrono_tz::Tz>().map_err(|_|anyhow!("Use an IANA timezone such as Europe/Moscow"))?;
        let start = minute(&self.quiet_start)?;
        let end = minute(&self.quiet_end)?;
        if self.quiet_enabled && start==end { return Err(anyhow!("Quiet hours must have different start and end times")); }
        if !(1..=5).contains(&self.urgent_priority) || !(1..=5).contains(&self.digest_below_priority)
            || !(0..=300).contains(&self.dedup_seconds) || (self.dedup_seconds>0 && self.dedup_seconds<5)
            || !(0..=86400).contains(&self.digest_seconds) || (self.digest_seconds>0 && self.digest_seconds<60) {
            return Err(anyhow!("Use priorities 1–5, deduplication 5–300 seconds or 0, and digest 60–86400 seconds or 0"));
        }
        Ok(())
    }
    fn quiet(&self, at: DateTime<Utc>, priority: i32) -> bool {
        if !self.quiet_enabled || priority>=self.urgent_priority { return false; }
        let Ok(zone) = self.timezone.parse::<chrono_tz::Tz>() else { return false; };
        let time = at.with_timezone(&zone);
        let now = time.hour()*60+time.minute();
        let start = minute(&self.quiet_start).unwrap_or(0);
        let end = minute(&self.quiet_end).unwrap_or(0);
        if start<end { now>=start && now<end } else { now>=start || now<end }
    }
    pub fn allowed_at(&self, at: DateTime<Utc>, priority: i32) -> DateTime<Utc> {
        if !self.quiet(at,priority) { return at; }
        // Walk real UTC minutes: ambiguous and missing local clock times remain DST-safe.
        let mut next = at + chrono::Duration::seconds(60-at.second() as i64)
            - chrono::Duration::nanoseconds(at.nanosecond() as i64);
        for _ in 0..(48*60) {
            if !self.quiet(next,priority) { return next; }
            next += chrono::Duration::minutes(1);
        }
        next
    }
    pub fn grouping(&self, priority: i32) -> Option<(&'static str,i64)> {
        if priority>=self.urgent_priority { return None; }
        if self.digest_seconds>0 && priority<self.digest_below_priority { return Some(("digest",self.digest_seconds)); }
        if self.dedup_seconds>0 { return Some(("dedup",self.dedup_seconds)); }
        None
    }
}
fn minute(value: &str) -> Result<u32> {
    let time = chrono::NaiveTime::parse_from_str(value,"%H:%M").map_err(|_|anyhow!("Use HH:MM for quiet hours"))?;
    Ok(time.hour()*60+time.minute())
}
pub fn requested_time(payload: &Value, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    if payload.get("deliver_at").is_some() && payload.get("delay").is_some() { return Err(anyhow!("Use deliver_at or delay, not both")); }
    let at = if let Some(value)=payload.get("deliver_at") {
        DateTime::parse_from_rfc3339(value.as_str().ok_or_else(||anyhow!("deliver_at must be an RFC 3339 timestamp"))?)?.with_timezone(&Utc)
    } else if let Some(value)=payload.get("delay") {
        let seconds = value.as_i64().filter(|s|(0..=2592000).contains(s)).ok_or_else(||anyhow!("delay must be 0–2592000 whole seconds"))?;
        now + chrono::Duration::seconds(seconds)
    } else { now };
    if at>now+chrono::Duration::days(30) { return Err(anyhow!("Schedule notifications at most 30 days ahead")); }
    Ok(at.max(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value:&str)->DateTime<Utc>{DateTime::parse_from_rfc3339(value).unwrap().with_timezone(&Utc)}
    #[test]
    fn quiet_hours_wrap_midnight_and_preserve_urgent_delivery() {
        let settings=Settings{quiet_enabled:true,timezone:"Europe/Moscow".into(),..Settings::default()};
        let now=at("2026-11-09T20:00:00Z");
        assert_eq!(settings.allowed_at(now,3),at("2026-11-10T05:00:00Z"));
        assert_eq!(settings.allowed_at(now,4),now);
        assert_eq!(settings.allowed_at(at("2026-11-10T05:00:00Z"),1),at("2026-11-10T05:00:00Z"));
    }
    #[test]
    fn quiet_hours_handle_daylight_saving_gaps_and_repeated_hours() {
        let settings=Settings{quiet_enabled:true,timezone:"America/New_York".into(),quiet_start:"01:00".into(),quiet_end:"03:00".into(),..Settings::default()};
        assert_eq!(settings.allowed_at(at("2026-03-08T06:30:00Z"),1),at("2026-03-08T07:00:00Z"));
        assert_eq!(settings.allowed_at(at("2026-11-01T05:30:00Z"),1),at("2026-11-01T08:00:00Z"));
    }
    #[test]
    fn explicit_schedules_are_bounded_and_unambiguous() {
        let now=at("2026-11-09T12:00:00Z");
        assert_eq!(requested_time(&json!({"delay":60}),now).unwrap(),now+chrono::Duration::minutes(1));
        for payload in [json!({"delay":-1}),json!({"delay":2592001}),json!({"delay":1.5}),json!({"delay":1,"deliver_at":"2026-11-09T13:00:00Z"}),json!({"deliver_at":"invalid"})] {
            assert!(requested_time(&payload,now).is_err());
        }
    }
}
