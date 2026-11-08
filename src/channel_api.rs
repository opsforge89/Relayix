//! Channel configuration and delivery adapters.
use crate::*;

#[derive(Serialize, FromRow)]
pub(crate) struct Channel {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) config: Value,
    pub(crate) enabled: bool,
    pub(crate) created_at: DateTime<Utc>,
}
#[derive(Deserialize)]
pub(crate) struct ChannelInput {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) config: Value,
    pub(crate) enabled: Option<bool>,
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
pub(crate) async fn channels(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Vec<Channel>> {
    auth::authenticated(&h, &s).await?;
    let mut result =
        sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY created_at DESC")
            .fetch_all(&s.db)
            .await
            .map_err(db_error)?;
    for c in &mut result {
        mask(&mut c.config);
    }
    Ok(Json(result))
}
pub(crate) fn mask(config: &mut Value) {
    for &key in providers::SECRET_FIELDS {
        if config.get(key).is_some() {
            config[key] = json!("********");
        }
    }
}
pub(crate) fn restore_masked(config: &mut Value, old: &Value) {
    for &key in providers::SECRET_FIELDS {
        if config.get(key).and_then(Value::as_str) == Some("********") {
            if let Some(value) = old.get(key) {
                config[key] = value.clone();
            }
        }
    }
}
pub(crate) async fn create_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ChannelInput>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    valid_config(&input.kind, &input.config, s.allow_http)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    network::validate(&input.config)
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO channels (id,name,kind,config,enabled) VALUES ($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(input.name)
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
    auth::authenticated(&h, &s).await?;
    let old: Option<(Value,)> = sqlx::query_as("SELECT config FROM channels WHERE id=$1")
        .bind(id)
        .fetch_optional(&s.db)
        .await
        .map_err(db_error)?;
    let old = old
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?
        .0;
    restore_masked(&mut input.config, &old);
    valid_config(&input.kind, &input.config, s.allow_http)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    network::validate(&input.config)
        .await
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    sqlx::query("UPDATE channels SET name=$2,kind=$3,config=$4,enabled=$5 WHERE id=$1")
        .bind(id)
        .bind(input.name)
        .bind(input.kind)
        .bind(input.config)
        .bind(input.enabled.unwrap_or(true))
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn delete_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    sqlx::query("DELETE FROM channels WHERE id=$1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
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
    let c = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id=$1")
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
        valid_config(&c.kind, &c.config, allow_http)?;
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
    valid_config(&c.kind, cfg, allow_http)?;
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
                .header("Title", title)
                .header("Priority", priority.to_string())
                .body(message.to_owned())
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
                ("title", title),
                ("message", message),
            ]),
        Config::Discord { url } => client.post(url).json(&json!({"content":text})),
        Config::Webhook { url } => client.post(url).json(&webhook_body::render(cfg.get("body_template"), title, message, priority)?),
        Config::Mattermost { url }
        | Config::Slack { url }
        | Config::Teams { url }
        | Config::GoogleChat { url }
        | Config::RocketChat { url } => client.post(url).json(&json!({"text":text})),
        Config::Smtp { .. } | Config::Matrix { .. } => {
            return Err(anyhow!("Adapter dispatch error"))
        }
    };
    let response = request.send().await.map_err(|e| anyhow!(e.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("Remote server returned {status}"));
    }
    Ok(())
}
