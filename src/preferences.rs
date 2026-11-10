use crate::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct DeliveryFilters {
    pub status: String,
    pub channel_id: String,
    pub route_id: String,
    #[serde(default)]
    pub event_id: String,
    pub from: String,
    pub to: String,
    pub search: String,
    pub tag: String,
    pub period: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TableSettings {
    pub hidden_columns: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub column_order: Vec<String>,
    pub sort_by: String,
    pub sort_dir: String,
}
impl Default for TableSettings {
    fn default() -> Self {
        Self {
            hidden_columns: vec![],
            column_order: vec![],
            sort_by: "created_at".into(),
            sort_dir: "desc".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub tables: std::collections::BTreeMap<String, TableSettings>,
    pub theme: String,
    pub refresh_interval_seconds: u32,
    pub delivery_filters: DeliveryFilters,
    pub route_tag_filter: String,
    pub page_size: u32,
    pub list_search: String,
    pub dashboard_period: String,
    pub api_token_tag_filter: String,
    pub notification_timeout_seconds: u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            tables: Default::default(),
            theme: "light".into(),
            refresh_interval_seconds: 15,
            delivery_filters: DeliveryFilters::default(),
            route_tag_filter: String::new(),
            page_size: 50,
            list_search: String::new(),
            dashboard_period: "30d".into(),
            api_token_tag_filter: String::new(),
            notification_timeout_seconds: 8,
        }
    }
}
impl Preferences {
    pub fn from_value(value: Value) -> Self {
        let mut prefs: Self = serde_json::from_value(value).unwrap_or_default();
        prefs.normalize_tables();
        prefs
    }
    fn normalize_tables(&mut self) {
        for id in ["deliveries", "dashboard_deliveries"] {
            if let Some(settings) = self.tables.get_mut(id) {
                let legacy = settings
                    .hidden_columns
                    .iter()
                    .any(|key| matches!(key.as_str(), "channel" | "status" | "attempts"));
                if settings.sort_by == "channel" {
                    settings.sort_by = "recipients".into();
                }
                if matches!(settings.sort_by.as_str(), "status" | "attempts") {
                    settings.sort_by = "created_at".into();
                    settings.sort_dir = "desc".into();
                }
                settings.hidden_columns = settings
                    .hidden_columns
                    .iter()
                    .filter_map(|key| match key.as_str() {
                        "channel" => Some("recipients".into()),
                        "status" | "attempts" => None,
                        _ => Some(key.clone()),
                    })
                    .collect();
                if legacy
                    && settings.hidden_columns.len() == table::columns(id).unwrap().len()
                    && settings
                        .hidden_columns
                        .iter()
                        .all(|key| table::columns(id).unwrap().contains(&key.as_str()))
                {
                    settings.hidden_columns.clear();
                }
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.tables.len() > 6 {
            return Err(anyhow!("Unknown table settings"));
        }
        for (id, settings) in &self.tables {
            let columns = table::columns(id).ok_or_else(|| anyhow!("Unknown table settings"))?;
            if !matches!(settings.sort_dir.as_str(), "asc" | "desc")
                || !columns.contains(&settings.sort_by.as_str())
                || matches!(settings.sort_by.as_str(), "actions" | "details")
            {
                return Err(anyhow!("Invalid table sort"));
            }
            if settings.column_order.len() > columns.len()
                || settings
                    .column_order
                    .iter()
                    .any(|key| !columns.contains(&key.as_str()))
                || settings
                    .column_order
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != settings.column_order.len()
            {
                return Err(anyhow!("Invalid column order"));
            }
            if settings.hidden_columns.len() >= columns.len()
                || settings
                    .hidden_columns
                    .iter()
                    .any(|key| !columns.contains(&key.as_str()))
                || settings
                    .hidden_columns
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != settings.hidden_columns.len()
            {
                return Err(anyhow!(
                    "Invalid hidden columns; keep at least one visible column"
                ));
            }
        }
        if !matches!(self.theme.as_str(), "light" | "dark") {
            return Err(anyhow!("Theme must be light or dark"));
        }
        if self.refresh_interval_seconds != 0
            && !(5..=3600).contains(&self.refresh_interval_seconds)
        {
            return Err(anyhow!(
                "Refresh interval must be 5–3600 seconds, or 0 to disable automatic refresh"
            ));
        }
        if ![10, 25, 50, 100].contains(&self.page_size) {
            return Err(anyhow!("Page size must be 10, 25, 50 or 100"));
        }
        if monitoring_period(&self.dashboard_period).is_none() {
            return Err(anyhow!("Invalid dashboard period"));
        }
        if self.notification_timeout_seconds > 300 {
            return Err(anyhow!(
                "Notification timeout must be 0–300 seconds; 0 disables automatic dismissal"
            ));
        }
        let f = &self.delivery_filters;
        if !f.period.is_empty() && monitoring_period(&f.period).is_none() {
            return Err(anyhow!("Invalid saved monitoring period"));
        }
        if !matches!(
            f.status.as_str(),
            "" | "sent" | "failed" | "queued" | "processing" | "retrying" | "cancelled" | "expired"
        ) {
            return Err(anyhow!("Invalid saved delivery status"));
        }
        for id in [&f.channel_id, &f.route_id, &f.event_id] {
            if !id.is_empty() && Uuid::parse_str(id).is_err() {
                return Err(anyhow!("Invalid saved channel or route"));
            }
        }
        for value in [&f.from, &f.to] {
            if !value.is_empty()
                && chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").is_err()
                && chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").is_err()
            {
                return Err(anyhow!("Invalid saved date"));
            }
        }
        if self.list_search.len() > 250
            || f.search.len() > 250
            || f.tag.len() > 64
            || self.route_tag_filter.len() > 64
            || self.api_token_tag_filter.len() > 64
        {
            return Err(anyhow!("Saved search or tag is too long"));
        }
        Ok(())
    }
}
pub async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    payload: std::result::Result<Json<Preferences>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Preferences> {
    let user = auth::authenticated(&h, &s).await?;
    let Json(mut input) = payload.map_err(|_| {
        err(
            StatusCode::BAD_REQUEST,
            "Invalid preferences: check field names, types and nonnegative timeouts",
        )
    })?;
    input.normalize_tables();
    input
        .validate()
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let value = serde_json::to_value(&input).unwrap();
    if let Some(id) = user.id {
        sqlx::query("UPDATE users SET preferences=$2 WHERE id=$1")
            .bind(id)
            .bind(value)
            .execute(&s.db)
            .await
            .map_err(db_error)?;
    } else {
        sqlx::query("UPDATE api_preferences SET preferences=$1 WHERE id=true")
            .bind(value)
            .execute(&s.db)
            .await
            .map_err(db_error)?;
    }
    Ok(Json(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_order_is_validated_and_round_trips() {
        let mut prefs = Preferences::from_value(
            json!({"tables":{"incoming":{"column_order":["title","created_at"],"hidden_columns":["routing"]}}}),
        );
        assert!(prefs.validate().is_ok());
        assert_eq!(
            Preferences::from_value(serde_json::to_value(&prefs).unwrap()),
            prefs
        );
        for invalid in [vec!["title", "title"], vec!["not_a_column"]] {
            prefs.tables.get_mut("incoming").unwrap().column_order =
                invalid.into_iter().map(String::from).collect();
            assert!(prefs.validate().is_err());
        }
        let legacy = Preferences::from_value(
            json!({"tables":{"incoming":{"sort_by":"title","sort_dir":"asc"}}}),
        );
        assert!(legacy.validate().is_ok());
        assert!(legacy.tables["incoming"].column_order.is_empty());
    }
    #[test]
    fn defaults_and_preference_limits() {
        let mut prefs = Preferences::from_value(json!({}));
        assert_eq!(prefs, Preferences::default());
        assert!(prefs.validate().is_ok());
        for interval in [0, 5, 15, 3600] {
            prefs.refresh_interval_seconds = interval;
            assert!(prefs.validate().is_ok());
        }
        for interval in [1, 4, 3601] {
            prefs.refresh_interval_seconds = interval;
            assert!(prefs.validate().is_err());
        }
        prefs = Preferences::default();
        prefs.delivery_filters.tag = "deploy".into();
        prefs.theme = "dark".into();
        assert_eq!(
            Preferences::from_value(serde_json::to_value(&prefs).unwrap()),
            prefs
        );
        prefs.page_size = 999;
        assert!(prefs.validate().is_err());
        prefs = Preferences::default();
        assert_eq!(prefs.notification_timeout_seconds, 8);
        for seconds in [0, 1, 8, 300] {
            prefs.notification_timeout_seconds = seconds;
            assert!(prefs.validate().is_ok());
        }
        prefs.notification_timeout_seconds = 301;
        assert!(prefs.validate().is_err());
    }
}
