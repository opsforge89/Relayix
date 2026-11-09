//! Channel configuration and delivery adapters.
use crate::*;

#[derive(Serialize, FromRow)]
pub(crate) struct Channel {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) config: Value,
    pub(crate) enabled: bool,
    pub(crate) manual_paused: bool,
    pub(crate) manual_pause_until: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
}
#[derive(Deserialize)]
pub(crate) struct ChannelInput {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) config: Value,
    pub(crate) enabled: Option<bool>,
}
pub(crate) fn valid_channel_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(anyhow!(
            "Channel name must contain 1–100 characters and no control characters"
        ));
    }
    Ok(())
}
pub(crate) fn has_body_template(config: &Value) -> bool {
    config.get("body_template").is_some_and(|v| !v.is_null())
}
pub(crate) fn require_template_editor(
    role: &str,
    config: &Value,
    old: Option<&Value>,
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    if role != "admin" && (has_body_template(config) || old.is_some_and(has_body_template)) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Administrator required to manage channels with a webhook body template",
        ));
    }
    Ok(())
}
pub(crate) fn mask_for_role(config: &mut Value, role: &str) {
    mask(config);
    if role != "admin" && has_body_template(config) {
        config["body_template"] = json!("********");
    }
}

pub(crate) fn valid_config(kind: &str, cfg: &Value, allow_http: bool) -> Result<()> {
    if !cfg.is_object() {
        return Err(anyhow!("Channel config must be an object"));
    }
    let typed = channel_config::Config::parse(kind, cfg)?;
    if let Some(raw) = typed.url() {
        let url = url::Url::parse(raw).map_err(|_| anyhow!("Invalid destination URL"))?;
        if url.scheme() != "https" && !(allow_http && url.scheme() == "http") {
            return Err(anyhow!(
                "HTTPS required (or explicitly enable ALLOW_INSECURE_HTTP)"
            ));
        }
        if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err(anyhow!("Invalid URL"));
        }
    }
    proxy::validate(kind, cfg)?;
    if kind == "mattermost" {
        mattermost::validate(cfg)?;
    }
    if kind == "webhook" {
        if let Some(template) = cfg.get("body_template").filter(|v| !v.is_null()) {
            webhook_body::validate(template)?;
        }
    }
    if kind == "telegram" {
        telegram_thread(cfg)?;
    }
    if kind == "smtp" {
        providers::valid_smtp(cfg, allow_http)?;
    }
    if kind == "matrix" {
        let room = cfg["room_id"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing room ID"))?;
        if !room.starts_with('!') || !room.contains(':') || room.chars().any(char::is_whitespace) {
            return Err(anyhow!(
                "Matrix room ID must look like !room:example.org (room aliases are not supported)"
            ));
        }
        let url = url::Url::parse(
            typed
                .url()
                .ok_or_else(|| anyhow!("Missing homeserver URL"))?,
        )?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(anyhow!(
                "Matrix homeserver URL must not include a query or fragment"
            ));
        }
    }
    Ok(())
}
pub(crate) async fn channels(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Value> {
    let principal = auth::authenticated(&h, &s).await?;
    let mut result =
        sqlx::query_as::<_, Channel>("SELECT id,name,kind,config,enabled,manual_paused,manual_pause_until,created_at FROM channels ORDER BY created_at DESC")
            .fetch_all(&s.db)
            .await
            .map_err(db_error)?;
    for c in &mut result {
        mask_for_role(&mut c.config, &principal.role);
    }
    let health: Vec<(Uuid, Value)> = sqlx::query_as("SELECT channel_id,jsonb_build_object('consecutive_failures',consecutive_failures,'last_error',last_error,'paused_until',paused_until,'state',CASE WHEN paused_until>NOW() THEN 'paused' WHEN paused_until IS NOT NULL THEN 'probe' ELSE 'healthy' END) FROM channel_health")
        .fetch_all(&s.db).await.map_err(db_error)?;
    let health = health
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    Ok(Json(json!(result
        .into_iter()
        .map(|c| {
            let state = health
                .get(&c.id)
                .cloned()
                .unwrap_or_else(|| json!({"state":"healthy","consecutive_failures":0}));
            let mut c = json!(c);
            c["health"] = state;
            c
        })
        .collect::<Vec<_>>())))
}
pub(crate) fn mask(config: &mut Value) {
    for &key in providers::SECRET_FIELDS {
        if config.get(key).is_some() {
            config[key] = json!("********");
        }
    }
}
pub(crate) fn restore_masked(
    config: &mut Value,
    old: &Value,
    kind: &str,
    old_kind: &str,
) -> Result<()> {
    // Compare before restoration; a masked destination means "keep the stored value".
    let destination_changed = kind != old_kind
        || config
            .as_object()
            .into_iter()
            .chain(old.as_object())
            .flat_map(|fields| fields.keys())
            .filter(|key| {
                matches!(
                    key.as_str(),
                    "url" | "host" | "port" | "tls" | "webproxy_address"
                ) || key.starts_with("proxy_")
            })
            .any(|key| {
                let value = config.get(key);
                let stored = old.get(key);
                let unchanged_mask = providers::SECRET_FIELDS.contains(&key.as_str())
                    && value.and_then(Value::as_str) == Some("********")
                    && stored.is_some();
                !unchanged_mask && value != stored
            });
    let has_masked = providers::SECRET_FIELDS
        .iter()
        .copied()
        .chain(std::iter::once("body_template"))
        .any(|key| config.get(key).and_then(Value::as_str) == Some("********"));
    if destination_changed && has_masked {
        return Err(anyhow!(
            "Re-enter all masked secrets when changing the channel type, destination or proxy settings"
        ));
    }
    for key in providers::SECRET_FIELDS
        .iter()
        .copied()
        .chain(std::iter::once("body_template"))
    {
        if config.get(key).and_then(Value::as_str) == Some("********") {
            if let Some(value) = old.get(key) {
                config[key] = value.clone();
            }
        }
    }
    Ok(())
}
pub(crate) async fn create_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ChannelInput>,
) -> ApiResult<Value> {
    let principal = auth::authenticated(&h, &s).await?;
    valid_channel_name(&input.name).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    require_template_editor(&principal.role, &input.config, None)?;
    valid_config(&input.kind, &input.config, s.allow_http)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    network::validate(&input.config)
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO channels (id,name,kind,config,enabled) VALUES ($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(input.name.trim())
        .bind(input.kind)
        .bind(input.config)
        .bind(input.enabled.unwrap_or(true))
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"id":id})))
}
pub(crate) async fn update_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut input): Json<ChannelInput>,
) -> ApiResult<Value> {
    let principal = auth::authenticated(&h, &s).await?;
    valid_channel_name(&input.name).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let old: Option<(Value, String)> =
        sqlx::query_as("SELECT config,kind FROM channels WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
    let (old, old_kind) = old.ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?;
    require_template_editor(&principal.role, &input.config, Some(&old))?;
    restore_masked(&mut input.config, &old, &input.kind, &old_kind)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    valid_config(&input.kind, &input.config, s.allow_http)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    network::validate(&input.config)
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    sqlx::query("UPDATE channels SET name=$2,kind=$3,config=$4,enabled=$5 WHERE id=$1")
        .bind(id)
        .bind(input.name.trim())
        .bind(input.kind)
        .bind(input.config)
        .bind(input.enabled.unwrap_or(true))
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    // A validated save is an explicit recovery action for this channel and both queues.
    sqlx::query("DELETE FROM channel_health WHERE channel_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    for query in [
        "UPDATE deliveries SET next_attempt_at=GREATEST(NOW(),not_before) WHERE channel_id=$1 AND status='queued' AND error IS NOT NULL",
        "UPDATE alert_deliveries SET next_attempt_at=GREATEST(NOW(),not_before) WHERE channel_id=$1 AND status='queued' AND error IS NOT NULL",
    ] {
        sqlx::query(query)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct PauseInput {
    #[serde(default)]
    until: Option<DateTime<Utc>>,
}
pub async fn pause_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<PauseInput>,
) -> ApiResult<Value> {
    set_manual_pause(&s, &h, id, true, input.until, false).await
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResumeInput {
    backlog: Backlog,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Backlog {
    #[default]
    Send,
    Cancel,
}
pub async fn resume_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    body: axum::body::Bytes,
) -> ApiResult<Value> {
    let input: ResumeInput = if body.is_empty() {
        ResumeInput::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| err(StatusCode::BAD_REQUEST, "Use backlog: send or cancel"))?
    };
    set_manual_pause(
        &s,
        &h,
        id,
        false,
        None,
        matches!(input.backlog, Backlog::Cancel),
    )
    .await
}
pub async fn channel_backlog(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=$1)")
        .bind(id).fetch_one(&s.db).await.map_err(db_error)?;
    if !exists {
        return Err(err(StatusCode::NOT_FOUND, "Channel not found"));
    }
    let (messages, deliveries, alerts): (i64, i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(COALESCE(g.occurrences,1)),0)::bigint,COUNT(*)::bigint,(SELECT COUNT(*) FROM alert_deliveries WHERE channel_id=$1 AND status='queued') FROM deliveries d JOIN events e ON e.id=d.event_id LEFT JOIN event_groups g ON g.id=e.group_id WHERE d.channel_id=$1 AND d.status='queued'")
        .bind(id).fetch_one(&s.db).await.map_err(db_error)?;
    Ok(Json(json!({"messages":messages+alerts,"notification_messages":messages,"alert_messages":alerts,"deliveries":deliveries+alerts})))
}
async fn set_manual_pause(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    paused: bool,
    until: Option<DateTime<Utc>>,
    cancel_backlog: bool,
) -> ApiResult<Value> {
    let actor = auth::authenticated(h, s).await?;
    if cancel_backlog && actor.role != "admin" {
        return Err(err(StatusCode::FORBIDDEN, "Administrator required to cancel queued deliveries"));
    }
    if until
        .is_some_and(|date| date <= Utc::now() || date > Utc::now() + chrono::Duration::days(365))
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Pause expiry must be in the next 365 days",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let config: Option<(Value,)> =
        sqlx::query_as("SELECT config FROM channels WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
    let config = config
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?
        .0;
    require_template_editor(&actor.role, &config, Some(&config))?;
    sqlx::query("UPDATE channels SET manual_paused=$2,manual_pause_until=$3 WHERE id=$1")
        .bind(id)
        .bind(paused)
        .bind(until)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let mut cancelled = 0;
    let mut cancelled_messages = 0;
    if cancel_backlog {
        let (deliveries, messages): (i64, i64) = sqlx::query_as("WITH cancelled AS (UPDATE deliveries SET status='cancelled',locked_at=NULL,error='Cancelled by user on channel resume',error_category='cancelled' WHERE channel_id=$1 AND status='queued' RETURNING event_id) SELECT COUNT(*)::bigint,COALESCE(SUM(COALESCE(g.occurrences,1)),0)::bigint FROM cancelled d JOIN events e ON e.id=d.event_id LEFT JOIN event_groups g ON g.id=e.group_id")
            .bind(id).fetch_one(&mut *tx).await.map_err(db_error)?;
        let alerts = sqlx::query("UPDATE alert_deliveries SET status='cancelled',locked_at=NULL,error='Cancelled by user on channel resume',error_category='cancelled' WHERE channel_id=$1 AND status='queued'")
            .bind(id).execute(&mut *tx).await.map_err(db_error)?.rows_affected() as i64;
        cancelled = deliveries + alerts;
        cancelled_messages = messages + alerts;
    }
    if !paused {
        sqlx::query("DELETE FROM channel_health WHERE channel_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        for query in [
            "UPDATE deliveries SET next_attempt_at=GREATEST(NOW(),not_before) WHERE channel_id=$1 AND status='queued'",
            "UPDATE alert_deliveries SET next_attempt_at=GREATEST(NOW(),not_before) WHERE channel_id=$1 AND status='queued'",
        ] {
            sqlx::query(query)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"ok":true,"paused":paused,"until":until,"cancelled":cancelled,"cancelled_messages":cancelled_messages}),
    ))
}
pub(crate) async fn delete_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let principal = auth::authenticated(&h, &s).await?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let config: Option<(Value,)> =
        sqlx::query_as("SELECT config FROM channels WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
    if let Some((config,)) = config {
        require_template_editor(&principal.role, &config, None)?;
    }
    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}

pub(crate) fn telegram_thread(cfg: &Value) -> Result<Option<i64>> {
    let Some(value) = cfg
        .get("message_thread_id")
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
    else {
        return Ok(None);
    };
    let id = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
        .filter(|id| *id > 0 && *id <= i32::MAX as i64)
        .ok_or_else(|| anyhow!("Telegram topic ID must be a positive integer"))?;
    Ok(Some(id))
}
pub(crate) fn telegram_payload(cfg: &Value, text: &str) -> Result<Value> {
    let mut payload = json!({"chat_id":cfg["chat_id"],"text":text});
    payload["parse_mode"] = json!("HTML");
    if let Some(id) = telegram_thread(cfg)? {
        payload["message_thread_id"] = json!(id);
    }
    Ok(payload)
}
pub(crate) async fn test_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let c = sqlx::query_as::<_, Channel>("SELECT id,name,kind,config,enabled,manual_paused,manual_pause_until,created_at FROM channels WHERE id=$1")
        .bind(id)
        .fetch_optional(&s.db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?;
    send(
        &s.client,
        &c,
        "Relayix test",
        "Test notification from Relayix",
        3,
        s.allow_http,
        Uuid::new_v4(),
    )
    .await
    .map_err(|e| err(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!({"ok":true})))
}

pub(crate) async fn send(
    client: &reqwest::Client,
    c: &Channel,
    title: &str,
    message: &str,
    priority: i32,
    allow_http: bool,
    delivery_id: Uuid,
) -> Result<()> {
    if c.kind == "telegram" && matches!(proxy::mode(&c.config), "mtproto" | "webproxy") {
        valid_config(&c.kind, &c.config, allow_http).map_err(|_| {
            retry_policy::permanent("Invalid channel configuration; update channel settings")
        })?;
        let rendered = message::render(&c.kind, &c.config, title, message);
        // Telegram owns its deadline: capacity waits must not consume a transport attempt.
        return telegram::send(&c.config, &rendered.text, delivery_id, rendered.html).await;
    }
    tokio::time::timeout(
        Duration::from_secs(60),
        send_inner(client, c, title, message, priority, allow_http, delivery_id),
    )
    .await
    .map_err(|_| anyhow!("Delivery attempt timed out"))?
}

async fn send_inner(
    client: &reqwest::Client,
    c: &Channel,
    title: &str,
    message: &str,
    priority: i32,
    allow_http: bool,
    delivery_id: Uuid,
) -> Result<()> {
    let cfg = &c.config;
    valid_config(&c.kind, cfg, allow_http).map_err(|_| {
        retry_policy::permanent("Invalid channel configuration; update channel settings")
    })?;
    network::validate(cfg).await?;
    let rendered = message::render(&c.kind, cfg, title, message);
    let text = rendered.text;
    let client = proxy::client(client, cfg)?;
    if c.kind == "mattermost" && mattermost::is_bot(cfg) {
        return mattermost::send(&client, cfg, &text).await;
    }
    if c.kind == "smtp" {
        return providers::send_smtp(cfg, title, message).await;
    }
    if c.kind == "matrix" {
        return providers::send_matrix(&client, cfg, &text, delivery_id).await;
    }
    use channel_config::Config;
    let request = match channel_config::Config::parse(&c.kind, cfg)? {
        Config::Telegram { bot_token, .. } => {
            if !bot_token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'))
            {
                return Err(anyhow!("Invalid bot token"));
            }
            client
                .post(format!(
                    "https://api.telegram.org/bot{bot_token}/sendMessage"
                ))
                .json(&telegram_payload(cfg, &text)?)
        }
        Config::Ntfy { url, topic } => {
            if !topic
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
            {
                return Err(anyhow!("Invalid topic"));
            }
            client
                .post(format!("{}/{topic}", url.trim_end_matches('/')))
                .header("Title", rendered.title.as_str())
                .header("Priority", priority.to_string())
                .body(rendered.body)
        }
        Config::Gotify { url, token } => client
            .post(format!("{}/message", url.trim_end_matches('/')))
            .query(&[("token", token)])
            .json(&json!({"title":title,"message":message,"priority":priority})),
        Config::Pushover { token, user } => client
            .post("https://api.pushover.net/1/messages.json")
            .form(&[
                ("token", token.as_str()),
                ("user", user.as_str()),
                ("title", rendered.title.as_str()),
                ("message", rendered.body.as_str()),
            ]),
        Config::Discord { url } => client.post(url).json(&json!({"content":text})),
        Config::Webhook { url } => client.post(url).json(
            &webhook_body::render(cfg.get("body_template"), title, message, priority)
                .map_err(|_| retry_policy::transport("message", true, None))?,
        ),
        Config::Mattermost { url }
        | Config::Slack { url }
        | Config::Teams { url }
        | Config::GoogleChat { url }
        | Config::RocketChat { url } => client.post(url).json(&json!({"text":text})),
        Config::Smtp { .. } | Config::Matrix { .. } => {
            return Err(anyhow!("Adapter dispatch error"))
        }
    };
    let response = request.send().await.map_err(|error| {
        if error.is_builder() {
            retry_policy::transport("message", true, None)
        } else {
            anyhow!(error.without_url())
        }
    })?;
    retry_policy::response(response).await?;
    Ok(())
}

#[cfg(test)]
mod access_tests {
    use super::*;
    #[test]
    fn body_templates_are_admin_only_and_cannot_be_retargeted_by_members() {
        let stored = json!({"url":"https://example.org/private", "body_template":{"api_key":"private","text":"{{message}}"}});
        let mut member = stored.clone();
        mask_for_role(&mut member, "member");
        assert_eq!(member["body_template"], "********");
        assert_eq!(member["url"], "********");
        let mut admin = stored.clone();
        mask_for_role(&mut admin, "admin");
        assert_eq!(admin["body_template"], stored["body_template"]);
        assert_eq!(admin["url"], "********");
        assert!(require_template_editor(
            "member",
            &json!({"url":"https://other.example"}),
            Some(&stored)
        )
        .is_err());
        assert!(require_template_editor("member", &stored, None).is_err());
        assert!(require_template_editor("admin", &member, Some(&stored)).is_ok());
        restore_masked(&mut member, &stored, "webhook", "webhook").unwrap();
        assert_eq!(member, stored);
        assert!(
            require_template_editor("member", &json!({"url":"https://example.org"}), None).is_ok()
        );
    }
    #[test]
    fn retargeting_requires_secrets_before_any_restoration() {
        for (kind, stored, field, replacement) in [
            (
                "gotify",
                json!({"url":"https://gotify.example.org", "token":"private"}),
                "url",
                json!("https://other.example.org"),
            ),
            (
                "matrix",
                json!({"url":"https://matrix.example.org", "access_token":"private"}),
                "url",
                json!("https://other.example.org"),
            ),
            (
                "smtp",
                json!({"host":"smtp.example.org", "password":"private"}),
                "host",
                json!("other.example.org"),
            ),
        ] {
            let mut edited = stored.clone();
            mask(&mut edited);
            edited[field] = replacement;
            let before = edited.clone();
            assert!(restore_masked(&mut edited, &stored, kind, kind).is_err());
            assert_eq!(edited, before);
        }
        let stored = json!({
            "url":"https://gotify.example.org", "token":"private",
            "host":"smtp.example.org", "port":587, "tls":"starttls",
            "proxy_type":"http", "proxy_host":"proxy.example.org", "proxy_port":3128,
            "proxy_username":"user", "proxy_password":"password", "proxy_secret":"secret"
        });
        for (field, value) in [
            ("port", json!(465)),
            ("tls", json!("none")),
            ("proxy_type", json!("none")),
            ("proxy_host", json!("other.example.org")),
            ("proxy_port", json!(8080)),
            ("proxy_username", json!("other")),
            ("proxy_password", json!("other")),
            ("proxy_secret", json!("other")),
            ("proxy_future_setting", json!("other")),
            ("webproxy_address", json!("other.example.org")),
        ] {
            let mut edited = stored.clone();
            mask(&mut edited);
            edited[field] = value;
            assert!(restore_masked(&mut edited, &stored, "gotify", "gotify").is_err());
        }
        for field in ["host", "port", "proxy_type", "proxy_host", "proxy_port"] {
            let mut edited = stored.clone();
            mask(&mut edited);
            edited.as_object_mut().unwrap().remove(field);
            assert!(restore_masked(&mut edited, &stored, "gotify", "gotify").is_err());
        }
        let mut edited = stored.clone();
        mask(&mut edited);
        assert!(restore_masked(&mut edited, &stored, "webhook", "gotify").is_err());
    }
    #[test]
    fn unchanged_destinations_preserve_masks_and_explicit_secrets_allow_retargeting() {
        let stored = json!({
            "url":"https://gotify.example.org", "token":"private",
            "proxy_type":"http", "proxy_host":"proxy.example.org", "proxy_port":3128,
            "proxy_username":"user", "proxy_password":"password"
        });
        let mut edited = stored.clone();
        mask(&mut edited);
        edited["priority"] = json!(4);
        restore_masked(&mut edited, &stored, "gotify", "gotify").unwrap();
        assert_eq!(edited["token"], stored["token"]);
        assert_eq!(edited["url"], stored["url"]);
        assert_eq!(edited["proxy_password"], stored["proxy_password"]);
        assert_eq!(edited["priority"], 4);

        let mut explicit = stored.clone();
        explicit["url"] = json!("https://other.example.org");
        explicit["token"] = json!("replacement");
        restore_masked(&mut explicit, &stored, "gotify", "gotify").unwrap();
        assert_eq!(explicit["token"], "replacement");
        assert_eq!(explicit["url"], "https://other.example.org");
    }
    #[test]
    fn channel_names_are_validated_as_unicode_on_all_write_paths() {
        for name in ["", "  ", "line\nbreak", &"a".repeat(101)] {
            assert!(valid_channel_name(name).is_err());
        }
        assert!(valid_channel_name(&"😀".repeat(100)).is_ok());
        assert!(valid_channel_name("  Notifications  ").is_ok());
    }
}
