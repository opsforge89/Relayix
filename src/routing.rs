//! Shared webhook routing and diagnostics, including authenticated UI tests.
use crate::{auth, db_error, err, hash, message, ApiResult, AppState, Channel};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(FromRow)]
struct Endpoint {
    id: Uuid,
    name: String,
    enabled: bool,
    min_priority: i32,
    required_tag: Option<String>,
    token_hash: String,
    template_id: Option<Uuid>,
    payload_rules: Value,
    rate_limit_per_minute: i32,
    delivery_settings: Value,
}

struct Event {
    title: String,
    message: String,
    priority: i32,
    tags: Vec<String>,
}

fn parse_event(payload: &Value) -> Result<Event, &'static str> {
    if !payload.is_object() {
        return Err("Webhook payload must be a JSON object");
    }
    let title = crate::message::event_title(payload).to_owned();
    let message = crate::message::event_message(payload).to_owned();
    let priority = match payload.get("priority") {
        None | Some(Value::Null) => 3,
        Some(value) => value
            .as_i64()
            .ok_or("priority must be an integer")?
            .clamp(1, 5) as i32,
    };
    let tags = match payload.get("tags") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|tag| tag.trim().to_owned())
                    .ok_or("tags must be an array of strings")
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("tags must be an array of strings or omitted"),
    };
    if title.len() > 256 || message.len() > 10000 {
        return Err("Title or message is too long");
    }
    if tags.len() > 32 || tags.iter().any(|tag| tag.is_empty() || tag.len() > 64) {
        return Err("Use at most 32 nonempty tags, each at most 64 bytes");
    }
    Ok(Event {
        title,
        message,
        priority,
        tags,
    })
}

#[derive(FromRow)]
struct Destination {
    id: Uuid,
    name: String,
    kind: String,
    enabled: bool,
}

#[derive(Serialize)]
struct Recipient {
    id: Uuid,
    name: String,
    kind: String,
    enabled: bool,
    queued: bool,
    reason: &'static str,
}

#[derive(Serialize)]
struct Report {
    route_id: Uuid,
    route_name: String,
    matched: bool,
    reason: &'static str,
    route_enabled: bool,
    priority: i32,
    min_priority: i32,
    priority_match: bool,
    route_tag: Option<String>,
    tag_filter_applied: bool,
    destinations: usize,
    active_destinations: usize,
    recipients: Vec<Recipient>,
}

fn report(endpoint: &Endpoint, event: &Event, destinations: Vec<Destination>) -> Report {
    let route_tag = endpoint
        .required_tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty());
    let priority_match = event.priority >= endpoint.min_priority;
    // The legacy required_tag column is a UI label, never a delivery condition.
    let matched = endpoint.enabled && priority_match;
    let total = destinations.len();
    let active = destinations
        .iter()
        .filter(|destination| destination.enabled)
        .count();
    let reason = if !endpoint.enabled {
        "route_disabled"
    } else if !priority_match {
        "priority_below_minimum"
    } else if total == 0 {
        "no_destinations"
    } else if active == 0 {
        "no_active_destinations"
    } else {
        "queued"
    };
    let recipients = destinations
        .into_iter()
        .map(|destination| {
            let queued = matched && destination.enabled;
            let recipient_reason = if !destination.enabled {
                "destination_disabled"
            } else if queued {
                "queued"
            } else {
                reason
            };
            Recipient {
                id: destination.id,
                name: destination.name,
                kind: destination.kind,
                enabled: destination.enabled,
                queued,
                reason: recipient_reason,
            }
        })
        .collect();
    Report {
        route_id: endpoint.id,
        route_name: endpoint.name.clone(),
        matched,
        reason,
        route_enabled: endpoint.enabled,
        priority: event.priority,
        min_priority: endpoint.min_priority,
        priority_match,
        route_tag: route_tag.map(str::to_owned),
        tag_filter_applied: false,
        destinations: total,
        active_destinations: active,
        recipients,
    }
}

async fn enqueue(
    s: &AppState,
    endpoint: Endpoint,
    payload: Value,
    test: bool,
    preview_role: Option<&str>,
    idempotency: Option<String>,
) -> ApiResult<Value> {
    let mut event = parse_event(&payload).map_err(|message| {
        tracing::warn!(route_id=%endpoint.id, reason="invalid_payload", error=message, test, "webhook payload rejected");
        err(StatusCode::BAD_REQUEST, message)
    })?;
    let now = chrono::Utc::now();
    let requested = crate::delivery_policy::requested_time(&payload, now)
        .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let policy =
        crate::delivery_policy::Settings::parse(&endpoint.delivery_settings).map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid route delivery settings",
            )
        })?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let payload_hash = hash(&serde_json::to_string(&payload).unwrap());
    if let Some(ref key_hash) = idempotency {
        // Serialize requests with the same route/key, including concurrent retries.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("{}:{key_hash}", endpoint.id))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let existing: Option<(Uuid, Option<String>, Value)> = sqlx::query_as(
            "SELECT id,payload_hash,routing FROM events WHERE route_id=$1 AND idempotency_hash=$2",
        )
        .bind(endpoint.id)
        .bind(key_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
        if let Some((id, previous_hash, routing)) = existing {
            if previous_hash.as_deref() != Some(&payload_hash) {
                return Err(err(
                    StatusCode::CONFLICT,
                    "Idempotency-Key was already used with a different payload",
                ));
            }
            let queued = routing["recipients"]
                .as_array()
                .map(|items| items.iter().filter(|r| r["queued"] == true).count())
                .unwrap_or(0);
            return Ok(Json(
                json!({"event_id":id,"queued":queued,"routing":routing,"duplicate":true}),
            ));
        }
    }
    let rules: Vec<crate::payload_rules::Rule> =
        serde_json::from_value(endpoint.payload_rules.clone()).map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid route validation configuration",
            )
        })?;
    let fields = crate::payload_rules::check(&rules, &payload);
    if !fields.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Invalid payload","fields":fields})),
        ));
    }
    let mut effective = payload.clone();
    let mut snapshot: Option<Value> = None;
    let mut template_result: Option<Value> = None;
    if let Some(id) = endpoint.template_id {
        if let Some(template) = sqlx::query_as::<_, crate::templates::Template>(
            "SELECT * FROM message_templates WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        {
            let (rendered, result) = crate::templates::render(&template, &payload)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            if crate::message::event_title(&rendered).trim().is_empty()
                && crate::message::event_message(&rendered).trim().is_empty()
            {
                return Err(err(StatusCode::BAD_REQUEST,"Template could not produce a message; provide its required fields or an original title/message"));
            }
            event = parse_event(&rendered).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            effective = rendered;
            snapshot = Some(serde_json::to_value(template).unwrap());
            template_result = Some(result);
        }
    }
    let destinations = sqlx::query_as::<_, Destination>(
        "SELECT c.id,c.name,c.kind,c.enabled FROM channels c JOIN route_channels rc ON rc.channel_id=c.id WHERE rc.route_id=$1 ORDER BY c.created_at,c.id")
        .bind(endpoint.id).fetch_all(&mut *tx).await.map_err(db_error)?;
    let routing = report(&endpoint, &event, destinations);
    let fallback:Vec<(Uuid,String,bool)>=sqlx::query_as("SELECT c.id,c.name,c.enabled FROM channels c JOIN route_fallback_channels f ON f.channel_id=c.id WHERE f.route_id=$1 ORDER BY c.id")
        .bind(endpoint.id).fetch_all(&mut *tx).await.map_err(db_error)?;
    let mut routing = serde_json::to_value(routing).unwrap();
    routing["fallback_recipients"] = json!(fallback
        .iter()
        .map(|(id, name, enabled)| json!({"id":id,"name":name,"enabled":enabled}))
        .collect::<Vec<_>>());
    if let Some(result) = &template_result {
        routing["template"] = result.clone();
    }
    let mut scheduled_at = policy.allowed_at(requested, event.priority);
    let grouping = if routing["matched"] == true {
        policy.grouping(event.priority)
    } else {
        None
    };
    if let Some((_, seconds)) = grouping {
        let closes = chrono::DateTime::from_timestamp(
            (now.timestamp().div_euclid(seconds) + 1) * seconds,
            0,
        )
        .unwrap();
        scheduled_at = policy.allowed_at(scheduled_at.max(closes), event.priority);
    }
    routing["delivery_policy"] = json!({"not_before":scheduled_at,"quiet_hours":policy.quiet_enabled,"timezone":policy.timezone,"grouping":grouping.map(|v|v.0),"window_seconds":grouping.map(|v|v.1)});
    if let Some(role) = preview_role {
        let would_queue = routing["recipients"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["queued"] == true)
            .count();
        for recipient in routing["recipients"].as_array_mut().unwrap() {
            recipient["would_queue"] = recipient["queued"].clone();
            recipient["queued"] = json!(false);
            if recipient["would_queue"] == true {
                recipient["reason"] = json!("would_queue");
            }
        }
        if would_queue > 0 {
            routing["reason"] = json!("would_queue");
        }
        let channels: Vec<Channel> = sqlx::query_as("SELECT c.* FROM channels c WHERE EXISTS(SELECT 1 FROM route_channels r WHERE r.route_id=$1 AND r.channel_id=c.id) OR EXISTS(SELECT 1 FROM route_fallback_channels f WHERE f.route_id=$1 AND f.channel_id=c.id) ORDER BY LOWER(c.name),c.id")
            .bind(endpoint.id).fetch_all(&mut *tx).await.map_err(db_error)?;
        let presentations = channels.into_iter().map(|c| {
            let presentation = message::preview(&c.kind, &c.config, &event.title, &event.message, event.priority, role=="admin")
                .unwrap_or_else(|_| json!({"error":"Unable to render this channel; check its template settings"}));
            json!({"id":c.id,"name":c.name,"kind":c.kind,"enabled":c.enabled,"paused":c.manual_paused && c.manual_pause_until.is_none_or(|at|at>chrono::Utc::now()),"is_fallback":fallback.iter().any(|r|r.0==c.id),"presentation":presentation})
        }).collect::<Vec<_>>();
        return Ok(Json(
            json!({"preview":true,"queued":0,"would_queue":would_queue,"routing":routing,"channel_previews":presentations,"rendered":{"title":event.title,"message":event.message}}),
        ));
    }
    if !endpoint.enabled {
        tracing::info!(route_id=%endpoint.id, reason=%routing["reason"], test, "webhook routing rejected");
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Route disabled","queued":0,"routing":routing})),
        ));
    }
    let group_key=grouping.map(|(kind,seconds)|{
        let content=if kind=="dedup"{json!({"title":event.title,"message":event.message,"priority":event.priority,"tags":event.tags})}else{json!("digest")};
        crate::hash(&json!({"kind":kind,"bucket":now.timestamp().div_euclid(seconds),"content":content,"recipients":routing["recipients"],"fallback":routing["fallback_recipients"],"policy":endpoint.delivery_settings,"deliver_at":payload.get("deliver_at"),"delay":payload.get("delay")}).to_string())
    });
    let existing_group = if let Some(key) = &group_key {
        crate::event_groups::find(&mut tx, endpoint.id, key)
            .await
            .map_err(|_| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unable to prepare delivery group",
                )
            })?
    } else {
        None
    };
    if existing_group.is_none() && endpoint.rate_limit_per_minute > 0 {
        let admitted: Option<Uuid> = sqlx::query_scalar("INSERT INTO route_rate_windows(route_id) VALUES($1) ON CONFLICT(route_id) DO UPDATE SET window_started_at=CASE WHEN route_rate_windows.window_started_at<=NOW()-INTERVAL '1 minute' THEN NOW() ELSE route_rate_windows.window_started_at END,requests=CASE WHEN route_rate_windows.window_started_at<=NOW()-INTERVAL '1 minute' THEN 1 ELSE route_rate_windows.requests+1 END WHERE route_rate_windows.window_started_at<=NOW()-INTERVAL '1 minute' OR route_rate_windows.requests<$2 RETURNING route_id")
            .bind(endpoint.id).bind(endpoint.rate_limit_per_minute).fetch_optional(&mut *tx).await.map_err(db_error)?;
        if admitted.is_none() {
            return Err(err(
                StatusCode::TOO_MANY_REQUESTS,
                "Route rate exceeded; retry in 60 seconds",
            ));
        }
    }
    if let Some(group) = &existing_group {
        routing["reason"] = json!("coalesced");
        routing["group_leader_id"] = json!(group.leader);
        for recipient in routing["recipients"].as_array_mut().unwrap() {
            recipient["queued"] = json!(false);
            recipient["reason"] = json!("coalesced");
        }
    }
    let event_id = Uuid::new_v4();
    sqlx::query("INSERT INTO events(id,route_id,title,message,priority,tags,payload,routing,idempotency_hash,payload_hash,rendered_payload,template_snapshot,template_result) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
        .bind(event_id).bind(endpoint.id).bind(&event.title).bind(&event.message).bind(event.priority)
        .bind(json!(event.tags)).bind(payload).bind(&routing).bind(idempotency).bind(payload_hash).bind(if snapshot.is_some(){Some(effective)}else{None}).bind(snapshot).bind(template_result).execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("UPDATE events SET deliver_at=$2 WHERE id=$1")
        .bind(event_id)
        .bind(scheduled_at)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    if let Some(group) = existing_group {
        sqlx::query("UPDATE events SET group_id=$2 WHERE id=$1")
            .bind(event_id)
            .bind(group.id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let count: i32 = sqlx::query_scalar(
            "UPDATE event_groups SET occurrences=occurrences+1 WHERE id=$1 RETURNING occurrences",
        )
        .bind(group.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        sqlx::query("UPDATE deliveries SET not_before=GREATEST(not_before,$2),next_attempt_at=GREATEST(next_attempt_at,$2) WHERE event_id=$1 AND status='queued'")
            .bind(group.leader).bind(scheduled_at).execute(&mut *tx).await.map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Ok(Json(
            json!({"event_id":event_id,"queued":0,"grouped":true,"group_id":group.id,"group_leader_id":group.leader,"occurrences":count,"routing":routing}),
        ));
    }
    if let (Some((kind, seconds)), Some(key)) = (grouping, group_key) {
        let group = Uuid::new_v4();
        let closes = chrono::DateTime::from_timestamp(
            (now.timestamp().div_euclid(seconds) + 1) * seconds,
            0,
        )
        .unwrap();
        sqlx::query("INSERT INTO event_groups(id,route_id,leader_event_id,group_key,kind,closes_at) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(group).bind(endpoint.id).bind(event_id).bind(key).bind(kind).bind(closes).execute(&mut *tx).await.map_err(db_error)?;
        sqlx::query("UPDATE events SET group_id=$2 WHERE id=$1")
            .bind(event_id)
            .bind(group)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    let mut queued = 0;
    for recipient in routing["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|recipient| recipient["queued"] == true)
    {
        sqlx::query("INSERT INTO deliveries(id,event_id,channel_id,not_before,next_attempt_at) VALUES($1,$2,$3,$4,$4)")
            .bind(Uuid::new_v4())
            .bind(event_id)
            .bind(Uuid::parse_str(recipient["id"].as_str().unwrap()).unwrap())
            .bind(scheduled_at)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        queued += 1;
    }
    for (id, _, _) in fallback {
        sqlx::query("INSERT INTO event_fallback_channels(event_id,channel_id) VALUES($1,$2)")
            .bind(event_id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    tracing::info!(%event_id, route_id=%endpoint.id, reason=%routing["reason"], matched=%routing["matched"],
        priority=event.priority, min_priority=endpoint.min_priority, tag_filter_applied=false,
        destinations=%routing["destinations"], active_destinations=%routing["active_destinations"], queued, test,
        "webhook routing evaluated");
    for recipient in routing["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|recipient| recipient["queued"] != true)
    {
        tracing::info!(%event_id, route_id=%endpoint.id, channel_id=%recipient["id"],
            reason=%recipient["reason"], test, "webhook destination excluded");
    }
    Ok(Json(
        json!({"event_id":event_id,"queued":queued,"routing":routing}),
    ))
}

pub async fn ingest(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut response = ingest_inner(State(s), Path(slug), headers, Json(payload))
        .await
        .into_response();
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert("retry-after", axum::http::HeaderValue::from_static("60"));
    }
    response
}

async fn ingest_inner(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> ApiResult<Value> {
    let endpoint = sqlx::query_as::<_, Endpoint>(
        "SELECT id,name,enabled,min_priority,required_tag,token_hash,message_template_id AS template_id,payload_rules,rate_limit_per_minute,delivery_settings FROM routes WHERE slug=$1")
        .bind(slug).fetch_optional(&s.db).await.map_err(db_error)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Unknown route"))?;
    let supplied = headers
        .get("x-relay-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if supplied.is_empty()
        || !bool::from(
            hash(supplied)
                .as_bytes()
                .ct_eq(endpoint.token_hash.as_bytes()),
        )
    {
        return Err(err(StatusCode::UNAUTHORIZED, "Invalid route key"));
    }
    let idempotency = match headers.get("idempotency-key") {
        None => None,
        Some(value) => {
            let value = value
                .to_str()
                .map_err(|_| err(StatusCode::BAD_REQUEST, "Invalid Idempotency-Key"))?;
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "Idempotency-Key must contain 1–256 characters",
                ));
            }
            Some(hash(value))
        }
    };
    enqueue(&s, endpoint, payload, false, None, idempotency).await
}

pub async fn test_route(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> ApiResult<Value> {
    auth::authenticated(&headers, &s).await?;
    let endpoint = sqlx::query_as::<_, Endpoint>(
        "SELECT id,name,enabled,min_priority,required_tag,token_hash,message_template_id AS template_id,payload_rules,rate_limit_per_minute,delivery_settings FROM routes WHERE id=$1")
        .bind(id).fetch_optional(&s.db).await.map_err(db_error)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Route not found"))?;
    enqueue(&s, endpoint, payload, true, None, None).await
}

pub async fn preview_route(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> ApiResult<Value> {
    let principal = auth::authenticated(&headers, &s).await?;
    let endpoint=sqlx::query_as::<_,Endpoint>("SELECT id,name,enabled,min_priority,required_tag,token_hash,message_template_id AS template_id,payload_rules,rate_limit_per_minute,delivery_settings FROM routes WHERE id=$1")
        .bind(id).fetch_optional(&s.db).await.map_err(db_error)?.ok_or_else(||err(StatusCode::NOT_FOUND,"Route not found"))?;
    enqueue(&s, endpoint, payload, true, Some(&principal.role), None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoint(tag: Option<&str>, minimum: i32) -> Endpoint {
        Endpoint {
            id: Uuid::new_v4(),
            name: "Test route".to_owned(),
            enabled: true,
            min_priority: minimum,
            required_tag: tag.map(str::to_owned),
            token_hash: String::new(),
            template_id: None,
            payload_rules: json!([]),
            rate_limit_per_minute: 0,
            delivery_settings: json!({}),
        }
    }
    #[test]
    fn optional_fields_and_configured_filters() {
        for payload in [
            json!({"title":"Test"}),
            json!({"priority":null,"tags":null}),
            json!({"tags":[]}),
        ] {
            let event = parse_event(&payload).unwrap();
            assert_eq!(event.priority, 3);
            assert!(event.tags.is_empty());
            assert!(report(&endpoint(None, 1), &event, vec![]).matched);
            assert!(report(&endpoint(Some("  "), 1), &event, vec![]).matched);
            assert!(report(&endpoint(Some("Deploy"), 1), &event, vec![]).matched);
            assert!(!report(&endpoint(Some("Deploy"), 1), &event, vec![]).tag_filter_applied);
            assert_eq!(
                report(&endpoint(None, 4), &event, vec![]).reason,
                "priority_below_minimum"
            );
        }
        assert!(parse_event(&json!({"tags":"prod"})).is_err());
        assert!(parse_event(&json!({"tags":[1]})).is_err());
        assert!(parse_event(&json!({"priority":"high"})).is_err());
        assert!(
            report(
                &endpoint(Some("prod"), 3),
                &parse_event(&json!({"tags":[" prod "]})).unwrap(),
                vec![]
            )
            .matched
        );
    }
}
