//! Read-only connectivity checks. Never send messages or return provider response bodies.
use crate::*;
fn step(name: &str, status: &str, detail: impl ToString) -> Value {
    json!({"name":name,"status":status,"detail":detail.to_string()})
}
fn response(steps: Vec<Value>) -> Json<Value> {
    Json(json!({"ok":!steps.iter().any(|s|s["status"]=="failed"),"steps":steps,"sent":false}))
}

fn endpoint(cfg: &Value, parts: &[&str]) -> Result<url::Url> {
    let mut url = url::Url::parse(cfg["url"].as_str().unwrap_or(""))?;
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut()
        .map_err(|_| anyhow!("Invalid server URL"))?
        .pop_if_empty()
        .extend(parts);
    Ok(url)
}
async fn read_json(request: reqwest::RequestBuilder) -> Result<Value> {
    let mut response = request.send().await.map_err(|_| {
        anyhow!("Read-only request failed; check connectivity, TLS and proxy settings")
    })?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "Read-only API returned HTTP {}; check credentials and destination access",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Could not read API response"))?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err(anyhow!("Diagnostic response is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| anyhow!("Read-only API returned invalid JSON"))
}
async fn other_channel(s: &AppState, c: &Channel) -> Json<Value> {
    let mut steps = vec![];
    let cfg = &c.config;
    if let Err(e) = network::validate(cfg).await {
        return response(vec![step("configuration", "failed", e)]);
    }
    if let Err(e) = valid_config(&c.kind, cfg, s.allow_http) {
        steps.push(step("configuration", "failed", e));
        return response(steps);
    }
    steps.push(step(
        "configuration",
        "passed",
        "Required channel settings are valid.",
    ));
    if c.kind == "smtp" {
        match providers::diagnose_smtp(cfg).await {
            Ok(()) => {
                steps.push(step(
                    "transport",
                    "passed",
                    "SMTP server accepted the NOOP connection check; no message was submitted.",
                ));
                steps.push(step(
                    "tls",
                    if cfg["tls"] == "none" {
                        "skipped"
                    } else {
                        "passed"
                    },
                    if cfg["tls"] == "none" {
                        "TLS is disabled for this trusted connection."
                    } else {
                        "Configured SMTP TLS was verified."
                    },
                ));
                steps.push(step(
                    "credentials",
                    if cfg["username"].as_str().unwrap_or("").is_empty() {
                        "skipped"
                    } else {
                        "passed"
                    },
                    if cfg["username"].as_str().unwrap_or("").is_empty() {
                        "No SMTP authentication is configured."
                    } else {
                        "SMTP authentication succeeded."
                    },
                ));
            }
            Err(e) => {
                steps.push(step("transport", "failed", e));
                steps.push(step("credentials","skipped","Authentication was not independently verified; resolve the connection/TLS/authentication failure first."));
            }
        }
        steps.push(step(
            "destination",
            "skipped",
            "Use Test to verify sender/recipient acceptance and actual delivery.",
        ));
        return response(steps);
    }
    let client = match proxy::client(&s.client, cfg) {
        Ok(client) => client,
        Err(e) => {
            steps.push(step("transport", "failed", e));
            return response(steps);
        }
    };
    let raw = match c.kind.as_str() {
        "pushover" => "https://api.pushover.net",
        _ => cfg["url"].as_str().unwrap_or(""),
    };
    let mut origin = match url::Url::parse(raw) {
        Ok(url) => url,
        Err(_) => {
            steps.push(step("transport", "failed", "Invalid server URL"));
            return response(steps);
        }
    };
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    match client.get(origin.clone()).send().await {
        Ok(reply) if reply.status() != reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED => {
            steps.push(step("transport","passed",if proxy::mode(cfg)=="http"{"Server is reachable through the configured HTTP proxy; no direct fallback was used."}else{"Server is reachable."}));
            steps.push(step(
                "tls",
                if origin.scheme() == "https" {
                    "passed"
                } else {
                    "skipped"
                },
                if origin.scheme() == "https" {
                    "HTTPS certificate and hostname verified."
                } else {
                    "HTTP is explicitly enabled; no TLS check applies."
                },
            ));
        }
        _ => {
            steps.push(step("transport","failed","Server is unreachable; check DNS, TCP, TLS and proxy credentials. No direct fallback was attempted."));
            return response(steps);
        }
    }
    let checked:Result<()>=async{
  match c.kind.as_str(){
   "mattermost" if mattermost::is_bot(cfg)=>{
    let token=cfg["bot_token"].as_str().unwrap();let me=read_json(client.get(mattermost::api_url(cfg,&["users","me"])?).bearer_auth(token)).await?;
    if me["id"].as_str().is_none(){return Err(anyhow!("Mattermost did not confirm the authenticated account"));}
    steps.push(step("credentials","passed","Mattermost accepted the bot or personal token."));
    let target=if cfg["target"]=="direct"{let raw=cfg["recipient"].as_str().unwrap().trim();if !raw.starts_with('@')&&raw.len()==26{mattermost::api_url(cfg,&["users",raw])?}else{mattermost::api_url(cfg,&["users","username",raw.trim_start_matches('@')])?}}else{mattermost::api_url(cfg,&["channels",cfg["channel_id"].as_str().unwrap()])?};
    let destination=read_json(client.get(target).bearer_auth(token)).await?;if destination["id"].as_str().is_none(){return Err(anyhow!("Mattermost did not confirm the destination"));}
    steps.push(step("destination","passed",if cfg["target"]=="direct"{"Recipient exists. No direct-message channel was created; use Test to verify sending."}else{"The channel is accessible; use Test to verify posting."}));
   },
   "matrix"=>{
    let token=cfg["access_token"].as_str().unwrap();let me=read_json(client.get(endpoint(cfg,&["_matrix","client","v3","account","whoami"])?).bearer_auth(token)).await?;
    if me["user_id"].as_str().is_none(){return Err(anyhow!("Matrix did not confirm the authenticated user"));}steps.push(step("credentials","passed","Matrix accepted the access token."));
    let rooms=read_json(client.get(endpoint(cfg,&["_matrix","client","v3","joined_rooms"])?).bearer_auth(token)).await?;
    if !rooms["joined_rooms"].as_array().is_some_and(|rooms|rooms.contains(&cfg["room_id"])){return Err(anyhow!("The Matrix account has not joined this room"));}
    let encryption=client.get(endpoint(cfg,&["_matrix","client","v3","rooms",cfg["room_id"].as_str().unwrap(),"state","m.room.encryption"])?).bearer_auth(token).send().await.map_err(|_|anyhow!("Could not check room encryption"))?;
    if encryption.status().is_success(){return Err(anyhow!("Encrypted Matrix rooms are not supported"));}
    if encryption.status()!=reqwest::StatusCode::NOT_FOUND{return Err(anyhow!("Matrix did not confirm an unencrypted room"));}
    steps.push(step("destination","passed","Account is joined to this unencrypted room; use Test to verify posting."));
   },
   "discord"=>{let data=read_json(client.get(cfg["url"].as_str().unwrap())).await?;if data["id"].as_str().is_none(){return Err(anyhow!("Discord did not confirm the webhook"));}steps.push(step("credentials","passed","Discord accepted the webhook token."));steps.push(step("destination","passed","Webhook metadata is accessible. Use Test to verify sending."));},
   "pushover"=>{let data=read_json(client.post("https://api.pushover.net/1/users/validate.json").form(&[("token",cfg["token"].as_str().unwrap()),("user",cfg["user"].as_str().unwrap())])).await?;if data["status"]!=1{return Err(anyhow!("Pushover did not validate the application/user keys"));}steps.push(step("credentials","passed","Pushover validated the application and user keys."));steps.push(step("destination","passed","User/group has an active destination; no notification was sent."));},
   "ntfy"|"gotify"=>{
    let health=if c.kind=="ntfy"{endpoint(cfg,&["v1","health"])?}else{endpoint(cfg,&["health"])?};
    match client.get(health).send().await{Ok(reply) if reply.status().is_success()=>{let data=read_json_response(reply).await?;if data["healthy"]==false||data["health"]=="red"{return Err(anyhow!("Notification service reports an unhealthy state"));}steps.push(step("service","passed","Service health endpoint responded successfully."));},Ok(reply) if reply.status()==reqwest::StatusCode::NOT_FOUND=>steps.push(step("service","skipped","This server does not expose the expected health endpoint.")),_=>return Err(anyhow!("Could not read the service health endpoint"))}
    steps.push(step("credentials","skipped","The sending token/topic permissions cannot be confirmed with this read-only check. Use Test."));steps.push(step("destination","skipped","Use Test to verify the configured destination without granting extra read permissions."));
   },
   _=>{steps.push(step("credentials","skipped","This integration has no safe credential-validation request; use Test."));steps.push(step("destination","skipped","Use Test to verify webhook acceptance and sending permissions."));}
  }Ok(())
 }.await;
    if let Err(e) = checked {
        let name = if steps
            .iter()
            .any(|s| s["name"] == "credentials" && s["status"] == "passed")
        {
            "destination"
        } else {
            "credentials"
        };
        steps.push(step(name, "failed", e));
    }
    response(steps)
}
async fn read_json_response(mut response: reqwest::Response) -> Result<Value> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Could not read API response"))?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err(anyhow!("Diagnostic response is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| anyhow!("Service did not return a valid health response"))
}
pub async fn channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let actor=auth::authenticated(&h,&s).await?;
    access::channel(&s.db,&actor,id,"edit").await?;
    let c = sqlx::query_as::<_, Channel>("SELECT id,name,kind,config,enabled,manual_paused,manual_pause_until,created_at,secrets_encrypted FROM channels WHERE id=$1")
        .bind(id)
        .fetch_optional(&s.db)
        .await
        .map_err(db_error)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?;
    Ok(check(&s, &c).await)
}
#[derive(Deserialize)]
pub struct Draft {
    id: Option<Uuid>,
    kind: String,
    config: Value,
}
pub async fn draft(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(mut draft): Json<Draft>,
) -> ApiResult<Value> {
    let principal = auth::authenticated(&h, &s).await?;
    require_template_editor(&principal.role, &draft.config, None)?;
    if let Some(id) = draft.id {
        access::channel(&s.db,&principal,id,"edit").await?;
        let old: Option<(Value, String, bool)> =
            sqlx::query_as("SELECT config,kind,secrets_encrypted FROM channels WHERE id=$1")
                .bind(id)
                .fetch_optional(&s.db)
                .await
                .map_err(db_error)?;
        let old = old.ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?;
        let plain = secrets::open_config(id, &old.1, &old.0, old.2).map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to decrypt channel credentials",
            )
        })?;
        require_template_editor(&principal.role, &draft.config, Some(&plain))?;
        restore_masked(&mut draft.config, &plain, &draft.kind, &old.1)
            .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    }
    let c = Channel {
        id: draft.id.unwrap_or_else(Uuid::new_v4),
        name: String::new(),
        kind: draft.kind,
        config: draft.config,
        enabled: true,
        manual_paused: false,
        manual_pause_until: None,
        created_at: Utc::now(),
    };
    Ok(check(&s, &c).await)
}
async fn bot_call(
    client: &reqwest::Client,
    cfg: &Value,
    method: &str,
    payload: Value,
) -> Result<Value> {
    let token = cfg["bot_token"].as_str().unwrap_or("");
    if token.is_empty()
        || !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":_-".contains(c))
    {
        return Err(anyhow!("Invalid bot token"));
    }
    let result = client
        .post(format!("https://api.telegram.org/bot{token}/{method}"))
        .json(&payload)
        .send()
        .await
        .map_err(|_| anyhow!("Telegram request failed; check connectivity and proxy settings"))?;
    let status = result.status();
    // Avoid retaining or reflecting provider errors containing tokens or identifiers.
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(anyhow!("Telegram rate limit; wait before retrying"));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Telegram rejected the request (HTTP {})",
            status.as_u16()
        ));
    }
    let data: Value = result
        .json()
        .await
        .map_err(|_| anyhow!("Invalid Telegram response"))?;
    if data["ok"] != true {
        return Err(anyhow!("Telegram rejected the request"));
    }
    Ok(data["result"].clone())
}
async fn check(s: &AppState, c: &Channel) -> Json<Value> {
    match tokio::time::timeout(Duration::from_secs(45),check_inner(s,c)).await{Ok(value)=>value,Err(_)=>response(vec![step("transport","failed","Diagnostic timed out. Check connectivity, TLS and proxy settings; no notification was sent.")])}
}
async fn check_inner(s: &AppState, c: &Channel) -> Json<Value> {
    if c.kind != "telegram" {
        return other_channel(s, c).await;
    }
    let mut steps = vec![];
    if let Err(e) = valid_config(&c.kind, &c.config, s.allow_http) {
        steps.push(step("configuration", "failed", e));
        return response(steps);
    }
    if let Err(e) = network::validate(&c.config).await {
        return response(vec![step("configuration", "failed", e)]);
    }
    steps.push(step(
        "configuration",
        "passed",
        "Required channel settings are valid.",
    ));
    if matches!(proxy::mode(&c.config), "mtproto" | "webproxy") {
        match telegram::transport(&c.config, "", Uuid::new_v4(), true, false).await {
            Ok(data) => {
                if let Some(items) = data["steps"].as_array() {
                    steps.extend(items.iter().cloned());
                }
            }
            Err(e) => steps.push(step("transport", "failed", e)),
        }
        return response(steps);
    }
    let client = match proxy::client(&s.client, &c.config) {
        Ok(client) => client,
        Err(e) => {
            steps.push(step("transport", "failed", e));
            return response(steps);
        }
    };
    match client.get("https://api.telegram.org").send().await {
        Ok(_) => steps.push(step(
            "transport",
            "passed",
            if proxy::mode(&c.config) == "http" {
                "Telegram is reachable through the configured HTTP proxy."
            } else {
                "Telegram HTTPS is reachable."
            },
        )),
        Err(_) => {
            steps.push(step("transport","failed","Telegram HTTPS is unreachable; check DNS, TLS, firewall and proxy credentials. No direct fallback was attempted."));
            return response(steps);
        }
    }
    let bot = match bot_call(&client, &c.config, "getMe", json!({})).await {
        Ok(bot) if bot["is_bot"] == true => {
            steps.push(step(
                "credentials",
                "passed",
                "Telegram accepted the bot token.",
            ));
            bot
        }
        _ => {
            steps.push(step(
                "credentials",
                "failed",
                "Bot authentication failed; check the token and Telegram rate limits.",
            ));
            return response(steps);
        }
    };
    let chat = match bot_call(
        &client,
        &c.config,
        "getChat",
        json!({"chat_id":c.config["chat_id"]}),
    )
    .await
    {
        Ok(chat) => {
            steps.push(step(
                "destination",
                "passed",
                "The bot can access the chat.",
            ));
            chat
        }
        Err(e) => {
            steps.push(step("destination", "failed", e));
            return response(steps);
        }
    };
    if let Ok(member) = bot_call(
        &client,
        &c.config,
        "getChatMember",
        json!({"chat_id":c.config["chat_id"],"user_id":bot["id"]}),
    )
    .await
    {
        let status = member["status"].as_str().unwrap_or("");
        let denied = matches!(status, "left" | "kicked")
            || (status == "restricted"
                && (member["is_member"] != true || member["can_send_messages"] != true))
            || (chat["type"] == "channel"
                && status != "creator"
                && (status != "administrator" || member["can_post_messages"] != true));
        if denied {
            if let Some(destination) = steps.last_mut() {
                *destination = step(
                    "destination",
                    "failed",
                    "The bot cannot send to this chat; check membership and posting permissions.",
                );
            }
        }
    }
    response(steps)
}
