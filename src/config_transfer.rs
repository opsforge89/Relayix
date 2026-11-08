//! Credential-free, append-only configuration transfer. Imports are disabled drafts.
use crate::*;
use std::collections::{HashMap, HashSet};

const SAFE_FIELDS: &[&str] = &[
    "chat_id",
    "message_thread_id",
    "topic",
    "room_id",
    "host",
    "port",
    "tls",
    "from",
    "to",
    "from_name",
    "proxy_type",
    "proxy_host",
    "proxy_port",
    "api_id",
    "mode",
    "target",
    "recipient",
    "channel_id",
];
const MAX_ITEMS: usize = 500;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    schema_version: u32,
    channels: Vec<PortableChannel>,
    routes: Vec<PortableRoute>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableChannel {
    key: Uuid,
    name: String,
    kind: String,
    config: Value,
    #[serde(default)]
    omitted_fields: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRoute {
    key: Uuid,
    name: String,
    min_priority: i32,
    rate_limit_per_minute: i32,
    required_tag: Option<String>,
    icon: String,
    payload_rules: Vec<payload_rules::Rule>,
    channel_keys: Vec<Uuid>,
    fallback_channel_keys: Vec<Uuid>,
}
fn safe_config(config: &Value) -> Value {
    let mut safe = serde_json::Map::new();
    for &field in SAFE_FIELDS {
        if let Some(value) = config
            .get(field)
            .filter(|v| v.is_string() || v.is_number() || v.is_boolean())
        {
            // Host fields must not smuggle credential-bearing URLs into a safe export.
            if matches!(field, "host" | "proxy_host")
                && value.as_str().is_none_or(|s| {
                    s.is_empty()
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
                })
            {
                continue;
            }
            safe.insert(field.to_owned(), value.clone());
        }
    }
    Value::Object(safe)
}
fn kind_valid(kind: &str) -> bool {
    // Deserializing the central adapter enum validates kinds without duplicating its registry.
    channel_config::Config::parse(kind, &json!({"bot_token":"placeholder","chat_id":"1","url":"https://example.org","topic":"placeholder","token":"placeholder","user":"placeholder","access_token":"placeholder","room_id":"!room:example.org","host":"example.org","from":"placeholder","to":"placeholder"})).is_ok()
}
fn validate(bundle: &Bundle) -> Result<()> {
    if bundle.schema_version != 1
        || bundle.channels.len() > MAX_ITEMS
        || bundle.routes.len() > MAX_ITEMS
        || serde_json::to_vec(bundle)?.len() > MAX_BYTES
    {
        return Err(anyhow!("Unsupported configuration version or bundle size (maximum 500 channels/routes and 2 MiB)"));
    }
    let mut channels = HashSet::new();
    for channel in &bundle.channels {
        if !channels.insert(channel.key) || !kind_valid(&channel.kind) {
            return Err(anyhow!(
                "Channel {} ({:?}): invalid kind or duplicate key",
                channel.key,
                channel.name.chars().take(80).collect::<String>()
            ));
        }
        valid_channel_name(&channel.name).map_err(|e| {
            anyhow!(
                "Channel {} ({:?}): {e}",
                channel.key,
                channel.name.chars().take(80).collect::<String>()
            )
        })?;
        if !channel.config.is_object()
            || safe_config(&channel.config) != channel.config
            || serde_json::to_vec(&channel.config)?.len() > 16384
        {
            return Err(anyhow!(
                "Channel {} ({:?}): config must contain only credential-free portable fields",
                channel.key,
                channel.name.chars().take(80).collect::<String>()
            ));
        }
    }
    let mut routes = HashSet::new();
    for route in &bundle.routes {
        if !routes.insert(route.key)
            || !(1..=5).contains(&route.min_priority)
            || route
                .channel_keys
                .iter()
                .chain(&route.fallback_channel_keys)
                .any(|id| !channels.contains(id))
        {
            return Err(anyhow!(
                "Route {} ({:?}): duplicate key, invalid priority or missing channel reference",
                route.key,
                route.name.chars().take(80).collect::<String>()
            ));
        }
        route_api::valid_route(&route_api::RouteInput {
            name: route.name.clone(),
            slug: None,
            enabled: Some(false),
            min_priority: Some(route.min_priority),
            rate_limit_per_minute: Some(route.rate_limit_per_minute),
            required_tag: route.required_tag.clone(),
            icon: Some(route.icon.clone()),
            template_id: None,
            payload_rules: route.payload_rules.clone(),
            channel_ids: route.channel_keys.clone(),
            fallback_channel_ids: route.fallback_channel_keys.clone(),
        })
        .map_err(|e| {
            anyhow!(
                "Route {} ({:?}): {e}",
                route.key,
                route.name.chars().take(80).collect::<String>()
            )
        })?;
    }
    Ok(())
}
pub async fn export(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Bundle> {
    auth::require_admin(&h, &s).await?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let channels: Vec<Channel> = sqlx::query_as("SELECT * FROM channels ORDER BY id LIMIT 501")
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    let routes: Vec<Route> = sqlx::query_as("SELECT id,name,slug,enabled,min_priority,required_tag,icon,message_template_id AS template_id,payload_rules,rate_limit_per_minute,created_at FROM routes ORDER BY id LIMIT 501").fetch_all(&mut *tx).await.map_err(db_error)?;
    let mut bundle = Bundle {
        schema_version: 1,
        channels: Vec::new(),
        routes: Vec::new(),
    };
    for channel in channels {
        let config = safe_config(&channel.config);
        let omitted_fields = channel
            .config
            .as_object()
            .map(|v| {
                v.keys()
                    .filter(|k| config.get(*k).is_none())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        bundle.channels.push(PortableChannel {
            key: channel.id,
            name: channel.name,
            kind: channel.kind,
            config,
            omitted_fields,
        });
    }
    for route in routes {
        let primary: Vec<Uuid> = sqlx::query_scalar(
            "SELECT channel_id FROM route_channels WHERE route_id=$1 ORDER BY channel_id",
        )
        .bind(route.id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        let fallback: Vec<Uuid> = sqlx::query_scalar(
            "SELECT channel_id FROM route_fallback_channels WHERE route_id=$1 ORDER BY channel_id",
        )
        .bind(route.id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
        bundle.routes.push(PortableRoute {
            key: route.id,
            name: route.name,
            min_priority: route.min_priority,
            rate_limit_per_minute: route.rate_limit_per_minute,
            required_tag: route.required_tag,
            icon: route.icon,
            payload_rules: serde_json::from_value(route.payload_rules).map_err(|_| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid stored payload rules",
                )
            })?,
            channel_keys: primary,
            fallback_channel_keys: fallback,
        });
    }
    validate(&bundle).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(bundle))
}
pub async fn import(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(bundle): Json<Bundle>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    validate(&bundle).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let mut ids = HashMap::new();
    for channel in &bundle.channels {
        let id = Uuid::new_v4();
        ids.insert(channel.key, id);
        sqlx::query("INSERT INTO channels(id,name,kind,config,enabled) VALUES($1,$2,$3,$4,false)")
            .bind(id)
            .bind(channel.name.trim())
            .bind(&channel.kind)
            .bind(&channel.config)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    for route in &bundle.routes {
        let id = Uuid::new_v4();
        let slug = route_api::generated_slug(&route.name, id);
        // No original endpoint/token is imported or exported, and no active route is changed.
        sqlx::query("INSERT INTO routes(id,name,slug,token_hash,enabled,min_priority,required_tag,icon,payload_rules,rate_limit_per_minute) VALUES($1,$2,$3,$4,false,$5,$6,$7,$8,$9)")
            .bind(id).bind(route.name.trim()).bind(slug).bind(hash(&key())).bind(route.min_priority).bind(&route.required_tag).bind(&route.icon).bind(json!(route.payload_rules)).bind(route.rate_limit_per_minute).execute(&mut *tx).await.map_err(db_error)?;
        for source in &route.channel_keys {
            sqlx::query("INSERT INTO route_channels(route_id,channel_id) VALUES($1,$2)")
                .bind(id)
                .bind(ids[source])
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
        for source in &route.fallback_channel_keys {
            sqlx::query("INSERT INTO route_fallback_channels(route_id,channel_id) VALUES($1,$2)")
                .bind(id)
                .bind(ids[source])
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"channels":bundle.channels.len(),"routes":bundle.routes.len(),"disabled":true,"message":"Imported disabled drafts. Configure credentials and URLs, review routes, then rotate new route keys before enabling."}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_legacy_names_report_the_record_and_share_unicode_validation() {
        let id = Uuid::new_v4();
        let channel = PortableChannel {
            key: id,
            name: " ".into(),
            kind: "webhook".into(),
            config: json!({}),
            omitted_fields: Vec::new(),
        };
        let mut bundle = Bundle {
            schema_version: 1,
            channels: vec![channel],
            routes: Vec::new(),
        };
        let error = validate(&bundle).unwrap_err().to_string();
        assert!(error.contains(&id.to_string()) && error.contains("Channel name"));
        bundle.channels[0].name = "😀".repeat(100);
        assert!(validate(&bundle).is_ok());
        bundle.channels[0].name = "x".repeat(101);
        assert!(validate(&bundle)
            .unwrap_err()
            .to_string()
            .contains(&id.to_string()));
    }
    #[test]
    fn portable_configs_omit_credentials_urls_and_custom_payloads() {
        let raw = json!({"bot_token":"private","url":"https://example.org/private-token","body_template":{"token":"private"},"proxy_secret":"private","chat_id":"42","host":"user:private@example.org"});
        assert_eq!(safe_config(&raw), json!({"chat_id":"42"}));
        assert!(kind_valid("telegram"));
        assert!(!kind_valid("unsupported"));
        let channel = PortableChannel {
            key: Uuid::new_v4(),
            name: "Draft".into(),
            kind: "webhook".into(),
            config: json!({"url":"https://example.org/private-token"}),
            omitted_fields: Vec::new(),
        };
        assert!(validate(&Bundle {
            schema_version: 1,
            channels: vec![channel],
            routes: Vec::new()
        })
        .is_err());
    }
}
