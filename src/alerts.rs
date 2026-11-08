use crate::*;

#[derive(Serialize, FromRow)]
pub struct Policy {
    id: Uuid,
    owner_user_id: Option<Uuid>,
    final_only: bool,
    name: String,
    enabled: bool,
    watch_channel_ids: Vec<Uuid>,
    watch_route_ids: Vec<Uuid>,
    destination_ids: Vec<Uuid>,
    failure_threshold: i32,
    min_attempts: i32,
    window_minutes: i32,
    cooldown_minutes: i32,
    title: String,
    body: String,
    last_triggered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}
#[derive(Deserialize)]
pub struct PolicyInput {
    #[serde(default)]
    final_only: bool,
    name: String,
    enabled: bool,
    #[serde(default)]
    watch_channel_ids: Vec<Uuid>,
    #[serde(default)]
    watch_route_ids: Vec<Uuid>,
    destination_ids: Vec<Uuid>,
    failure_threshold: i32,
    min_attempts: i32,
    window_minutes: i32,
    cooldown_minutes: i32,
    title: String,
    body: String,
}
async fn validate(
    s: &AppState,
    input: &PolicyInput,
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || input.title.trim().is_empty()
        || input.title.len() > 256
        || input.body.trim().is_empty()
        || input.body.len() > 5000
        || !(1..=100).contains(&input.failure_threshold)
        || !(1..=5).contains(&input.min_attempts)
        || !(1..=1440).contains(&input.window_minutes)
        || !(1..=1440).contains(&input.cooldown_minutes)
        || (input.enabled && input.destination_ids.is_empty())
        || input.destination_ids.len() > 100
        || input.watch_channel_ids.len() > 100
        || input.watch_route_ids.len() > 100
    {
        return Err(err(StatusCode::BAD_REQUEST,"Provide a name, alert text, at least one recipient, and valid threshold/window/cooldown values"));
    }
    for ids in [&input.destination_ids, &input.watch_channel_ids] {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels WHERE id=ANY($1)")
            .bind(ids)
            .fetch_one(&s.db)
            .await
            .map_err(db_error)?;
        if count.0 != ids.len() as i64 {
            return Err(err(StatusCode::BAD_REQUEST, "Unknown or duplicate channel"));
        }
    }
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM routes WHERE id=ANY($1)")
        .bind(&input.watch_route_ids)
        .fetch_one(&s.db)
        .await
        .map_err(db_error)?;
    if count.0 != input.watch_route_ids.len() as i64 {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown or duplicate route"));
    }
    Ok(())
}
pub async fn list(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Vec<Policy>> {
    auth::require_admin(&h, &s).await?;
    Ok(Json(
        sqlx::query_as::<_, Policy>(
            "SELECT * FROM alert_policies WHERE owner_user_id IS NULL ORDER BY created_at",
        )
        .fetch_all(&s.db)
        .await
        .map_err(db_error)?,
    ))
}
pub async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PolicyInput>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    validate(&s, &input).await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO alert_policies(id,name,enabled,watch_channel_ids,watch_route_ids,destination_ids,failure_threshold,min_attempts,window_minutes,cooldown_minutes,title,body,final_only) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
        .bind(id).bind(input.name.trim()).bind(input.enabled).bind(input.watch_channel_ids).bind(input.watch_route_ids).bind(input.destination_ids)
        .bind(input.failure_threshold).bind(input.min_attempts).bind(input.window_minutes).bind(input.cooldown_minutes).bind(input.title).bind(input.body).bind(input.final_only)
        .execute(&s.db).await.map_err(db_error)?;
    Ok(Json(json!({"id":id})))
}
pub async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<PolicyInput>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    validate(&s, &input).await?;
    let result=sqlx::query("UPDATE alert_policies SET name=$2,enabled=$3,watch_channel_ids=$4,watch_route_ids=$5,destination_ids=$6,failure_threshold=$7,min_attempts=$8,window_minutes=$9,cooldown_minutes=$10,title=$11,body=$12,final_only=$13 WHERE id=$1 AND owner_user_id IS NULL")
        .bind(id).bind(input.name.trim()).bind(input.enabled).bind(input.watch_channel_ids).bind(input.watch_route_ids).bind(input.destination_ids)
        .bind(input.failure_threshold).bind(input.min_attempts).bind(input.window_minutes).bind(input.cooldown_minutes).bind(input.title).bind(input.body).bind(input.final_only)
        .execute(&s.db).await.map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(err(StatusCode::NOT_FOUND, "Alert rule not found"));
    }
    Ok(Json(json!({"ok":true})))
}
pub async fn delete(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    sqlx::query("DELETE FROM alert_policies WHERE id=$1 AND owner_user_id IS NULL")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn test(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    let policy = sqlx::query_as::<_, Policy>(
        "SELECT * FROM alert_policies WHERE id=$1 AND owner_user_id IS NULL",
    )
    .bind(id)
    .fetch_optional(&s.db)
    .await
    .map_err(db_error)?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "Alert rule not found"))?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let channels: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM channels WHERE id=ANY($1) AND enabled=true")
            .bind(&policy.destination_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_error)?;
    if channels.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "No enabled alert recipients"));
    }
    for (cid,) in &channels {
        sqlx::query("INSERT INTO alert_deliveries(id,policy_id,channel_id,title,message) VALUES($1,$2,$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(id).bind(cid).bind("Relayix: failure alert test").bind(format!("Test of alert rule '{}'. This is a test notification.",policy.name)).execute(&mut *tx).await.map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"queued":channels.len()})))
}

#[derive(Serialize, FromRow)]
pub struct AlertDelivery {
    id: Uuid,
    policy_name: Option<String>,
    channel_name: String,
    status: String,
    attempts: i32,
    error: Option<String>,
    title: String,
    created_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
}
pub async fn history(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(sort): Query<table::Sort>,
) -> ApiResult<Vec<AlertDelivery>> {
    auth::require_admin(&h, &s).await?;
    let order = table::order("alert_history", &sort)?;
    let mut sql=sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT a.id,p.name AS policy_name,c.name AS channel_name,a.status,a.attempts,a.error,a.title,a.created_at,a.sent_at FROM alert_deliveries a LEFT JOIN alert_policies p ON p.id=a.policy_id JOIN channels c ON c.id=a.channel_id WHERE a.owner_user_id IS NULL");
    sql.push(order).push(" LIMIT 100");
    Ok(Json(
        sql.build_query_as::<AlertDelivery>()
            .fetch_all(&s.db)
            .await
            .map_err(db_error)?,
    ))
}
pub async fn retry(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    let result=sqlx::query("UPDATE alert_deliveries SET status='queued',attempts=0,error=NULL,error_category=NULL,locked_at=NULL,next_attempt_at=NOW() WHERE id=$1 AND status='failed' AND owner_user_id IS NULL").bind(id).execute(&s.db).await.map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(err(
            StatusCode::CONFLICT,
            "Only failed alerts can be retried",
        ));
    }
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub struct PersonalInput {
    enabled: bool,
    #[serde(default)]
    destination_ids: Vec<Uuid>,
    #[serde(default)]
    watch_route_ids: Vec<Uuid>,
    trigger_mode: String,
    failure_threshold: i32,
    window_minutes: i32,
    cooldown_minutes: i32,
}
async fn account(
    h: &HeaderMap,
    s: &AppState,
) -> std::result::Result<Uuid, (StatusCode, Json<Value>)> {
    auth::authenticated(h, s).await?.id.ok_or_else(|| {
        err(
            StatusCode::FORBIDDEN,
            "Sign in with a personal account to manage delivery alerts",
        )
    })
}
pub async fn personal(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Value> {
    let owner = account(&h, &s).await?;
    let policy: Option<Policy> =
        sqlx::query_as("SELECT * FROM alert_policies WHERE owner_user_id=$1")
            .bind(owner)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    Ok(Json(json!({"policy":policy})))
}
pub async fn save_personal(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PersonalInput>,
) -> ApiResult<Value> {
    let owner = account(&h, &s).await?;
    if !matches!(
        input.trigger_mode.as_str(),
        "final" | "first_error" | "threshold"
    ) {
        return Err(err(StatusCode::BAD_REQUEST, "Unknown alert trigger"));
    }
    let mut policy=PolicyInput{final_only:input.trigger_mode=="final",name:"Personal delivery alerts".into(),enabled:input.enabled,watch_channel_ids:vec![],watch_route_ids:input.watch_route_ids,destination_ids:input.destination_ids,failure_threshold:if input.trigger_mode=="threshold"{input.failure_threshold}else{1},min_attempts:1,window_minutes:input.window_minutes,cooldown_minutes:input.cooldown_minutes,title:"Relayix: delivery failure".into(),body:"{failures} delivery failure(s). Channel: {channel}. Route: {route}. Message: {title}. Error: {error}".into()};
    if !policy.enabled {
        policy.destination_ids =
            sqlx::query_scalar("SELECT id FROM channels WHERE id=ANY($1) ORDER BY id")
                .bind(&policy.destination_ids)
                .fetch_all(&s.db)
                .await
                .map_err(db_error)?;
        policy.watch_route_ids =
            sqlx::query_scalar("SELECT id FROM routes WHERE id=ANY($1) ORDER BY id")
                .bind(&policy.watch_route_ids)
                .fetch_all(&s.db)
                .await
                .map_err(db_error)?;
    }
    validate(&s, &policy).await?;
    if policy.enabled {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE id=ANY($1) AND enabled")
                .bind(&policy.destination_ids)
                .fetch_one(&s.db)
                .await
                .map_err(db_error)?;
        if count != policy.destination_ids.len() as i64 {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Choose enabled alert recipients",
            ));
        }
    }
    let result:Policy=sqlx::query_as("INSERT INTO alert_policies(id,owner_user_id,name,enabled,watch_route_ids,destination_ids,failure_threshold,min_attempts,window_minutes,cooldown_minutes,title,body,final_only) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) ON CONFLICT(owner_user_id) WHERE owner_user_id IS NOT NULL DO UPDATE SET enabled=EXCLUDED.enabled,watch_route_ids=EXCLUDED.watch_route_ids,destination_ids=EXCLUDED.destination_ids,failure_threshold=EXCLUDED.failure_threshold,window_minutes=EXCLUDED.window_minutes,cooldown_minutes=EXCLUDED.cooldown_minutes,final_only=EXCLUDED.final_only RETURNING *")
 .bind(Uuid::new_v4()).bind(owner).bind(policy.name).bind(policy.enabled).bind(policy.watch_route_ids).bind(policy.destination_ids).bind(policy.failure_threshold).bind(policy.min_attempts).bind(policy.window_minutes).bind(policy.cooldown_minutes).bind(policy.title).bind(policy.body).bind(policy.final_only).fetch_one(&s.db).await.map_err(db_error)?;
    Ok(Json(json!({"policy":result})))
}
pub async fn test_personal(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Value> {
    let owner = account(&h, &s).await?;
    let policy: Policy =
        sqlx::query_as("SELECT * FROM alert_policies WHERE owner_user_id=$1 AND enabled")
            .bind(owner)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                err(
                    StatusCode::BAD_REQUEST,
                    "Save and enable your delivery alert subscription first",
                )
            })?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let channels: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM channels WHERE id=ANY($1) AND enabled")
            .bind(policy.destination_ids)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_error)?;
    if channels.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "No enabled alert recipients"));
    }
    for id in &channels {
        sqlx::query("INSERT INTO alert_deliveries(id,policy_id,channel_id,title,message,owner_user_id) VALUES($1,$2,$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(policy.id).bind(id).bind("Relayix: personal delivery alert test").bind("Your personal delivery alert channel is working. This is a test notification.").bind(owner).execute(&mut *tx).await.map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"queued":channels.len()})))
}
pub async fn personal_history(
    State(s): State<AppState>,
    h: HeaderMap,
) -> ApiResult<Vec<AlertDelivery>> {
    let owner = account(&h, &s).await?;
    Ok(Json(sqlx::query_as("SELECT a.id,p.name AS policy_name,c.name AS channel_name,a.status,a.attempts,a.error,a.title,a.created_at,a.sent_at FROM alert_deliveries a LEFT JOIN alert_policies p ON p.id=a.policy_id JOIN channels c ON c.id=a.channel_id WHERE a.owner_user_id=$1 ORDER BY a.created_at DESC,a.id DESC LIMIT 100").bind(owner).fetch_all(&s.db).await.map_err(db_error)?))
}

pub async fn retry_personal(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let owner = account(&h, &s).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM alert_deliveries WHERE id=$1 AND owner_user_id=$2)",
    )
    .bind(id)
    .bind(owner)
    .fetch_one(&s.db)
    .await
    .map_err(db_error)?;
    if !exists {
        return Err(err(StatusCode::NOT_FOUND, "Alert delivery not found"));
    }
    let result=sqlx::query("UPDATE alert_deliveries a SET status='queued',attempts=0,error=NULL,error_category=NULL,locked_at=NULL,next_attempt_at=NOW() WHERE id=$1 AND owner_user_id=$2 AND status='failed' AND EXISTS(SELECT 1 FROM alert_policies p WHERE p.id=a.policy_id AND p.enabled)").bind(id).bind(owner).execute(&s.db).await.map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(err(
            StatusCode::CONFLICT,
            "Only failed alerts with an enabled subscription can be retried",
        ));
    }
    Ok(Json(json!({"ok":true})))
}

fn render(
    pattern: &str,
    channel: &str,
    route: &str,
    error: &str,
    title: &str,
    count: usize,
) -> String {
    // Single pass: provider text cannot inject additional placeholder expansions.
    let replacements = [
        ("channel", channel.to_owned()),
        ("route", route.to_owned()),
        ("error", error.to_owned()),
        ("title", title.to_owned()),
        ("failures", count.to_string()),
    ];
    let mut output = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find('}') {
            let key = &rest[1..end];
            if let Some((_, value)) = replacements.iter().find(|(name, _)| *name == key) {
                output.push_str(value);
            } else {
                output.push_str(&rest[..=end]);
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    output.push_str(rest);
    output
}

async fn schedule(s: &AppState) -> Result<()> {
    let policies:Vec<(Uuid,)>=sqlx::query_as("SELECT id FROM alert_policies p WHERE enabled=true AND (owner_user_id IS NULL OR EXISTS(SELECT 1 FROM users u WHERE u.id=p.owner_user_id AND u.enabled))").fetch_all(&s.db).await?;
    for (pid,) in policies {
        let mut tx = s.db.begin().await?;
        let policy=sqlx::query_as::<_,Policy>("SELECT * FROM alert_policies p WHERE id=$1 AND enabled=true AND (owner_user_id IS NULL OR EXISTS(SELECT 1 FROM users u WHERE u.id=p.owner_user_id AND u.enabled)) AND (last_triggered_at IS NULL OR last_triggered_at<=NOW()-cooldown_minutes*INTERVAL '1 minute') FOR UPDATE SKIP LOCKED")
            .bind(pid).fetch_optional(&mut *tx).await?;
        let Some(policy) = policy else {
            continue;
        };
        let failures:Vec<(Uuid,String,String,String,String,i32)>=sqlx::query_as("SELECT d.id,c.name,r.name,d.error,e.title,d.retry_generation FROM deliveries d JOIN channels c ON c.id=d.channel_id JOIN events e ON e.id=d.event_id JOIN routes r ON r.id=e.route_id WHERE d.status IN ('queued','failed') AND d.error IS NOT NULL AND d.cycle_attempts>=$1 AND (NOT $6 OR d.status='failed') AND ($7::timestamptz IS NULL OR d.last_error_at>=$7) AND d.last_error_at>=NOW()-$2::int*INTERVAL '1 minute' AND (CARDINALITY($3::uuid[])=0 OR d.channel_id=ANY($3)) AND (CARDINALITY($4::uuid[])=0 OR e.route_id=ANY($4)) AND NOT EXISTS(SELECT 1 FROM alert_incidents i WHERE i.policy_id=$5 AND i.delivery_id=d.id AND i.retry_generation=d.retry_generation) ORDER BY d.last_error_at DESC LIMIT 1000")
            .bind(policy.min_attempts).bind(policy.window_minutes).bind(&policy.watch_channel_ids).bind(&policy.watch_route_ids).bind(pid).bind(policy.final_only).bind(policy.owner_user_id.map(|_|policy.created_at)).fetch_all(&mut *tx).await?;
        if failures.len() < policy.failure_threshold as usize {
            continue;
        }
        let destinations: Vec<(Uuid,)> =
            sqlx::query_as("SELECT id FROM channels WHERE id=ANY($1) AND enabled=true")
                .bind(&policy.destination_ids)
                .fetch_all(&mut *tx)
                .await?;
        if destinations.is_empty() {
            continue;
        }
        let (source, channel, route, error, event_title, _) = &failures[0];
        let title = render(
            &policy.title,
            channel,
            route,
            error,
            event_title,
            failures.len(),
        )
        .chars()
        .take(256)
        .collect::<String>();
        let message = render(
            &policy.body,
            channel,
            route,
            error,
            event_title,
            failures.len(),
        )
        .chars()
        .take(8000)
        .collect::<String>();
        for (id, _, _, _, _, generation) in &failures {
            sqlx::query("INSERT INTO alert_incidents(policy_id,delivery_id,retry_generation) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(pid).bind(id).bind(generation).execute(&mut *tx).await?;
        }
        for (cid,) in destinations {
            sqlx::query("INSERT INTO alert_deliveries(id,policy_id,source_delivery_id,channel_id,title,message,owner_user_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(Uuid::new_v4()).bind(pid).bind(source).bind(cid).bind(&title).bind(&message).bind(policy.owner_user_id).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE alert_policies SET last_triggered_at=NOW() WHERE id=$1")
            .bind(pid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn send_next(s: &AppState) -> Result<lease::Claim<()>> {
    let mut tx = s.db.begin().await?;
    let channel = match lease::claim_channel(
        &mut tx,
        lease::Queue::Alert,
        s.channel_concurrency,
        s.proxy_concurrency,
    )
    .await?
    {
        lease::Claim::Ready(channel) => channel,
        lease::Claim::Idle => return Ok(lease::Claim::Idle),
        lease::Claim::Contended => return Ok(lease::Claim::Contended),
    };
    let row:Option<(Uuid,Uuid,String,String,i32)>=sqlx::query_as("SELECT id,channel_id,title,message,attempts+1 FROM alert_deliveries WHERE channel_id=$1 AND ((status='queued' AND next_attempt_at<=NOW()) OR (status='processing' AND locked_at<NOW()-INTERVAL '2 minutes')) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1").bind(channel).fetch_optional(&mut *tx).await?;
    let Some((id, cid, title, message, number)) = row else {
        return Ok(lease::Claim::Contended);
    };
    sqlx::query("UPDATE alert_deliveries SET status='processing',locked_at=NOW(),attempts=attempts+1 WHERE id=$1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    enum Outcome {
        Sent,
        Disabled,
    }
    let work = async {
        let allowed:bool=sqlx::query_scalar("SELECT a.owner_user_id IS NULL OR EXISTS(SELECT 1 FROM users u JOIN alert_policies p ON p.owner_user_id=u.id WHERE u.id=a.owner_user_id AND u.enabled AND p.enabled AND p.id=a.policy_id) FROM alert_deliveries a WHERE a.id=$1").bind(id).fetch_one(&s.db).await?;
        if !allowed {
            return Ok(Outcome::Disabled);
        }
        let channel = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id=$1")
            .bind(cid)
            .fetch_optional(&s.db)
            .await?;
        match channel {
            Some(c) if c.enabled => send(&s.client, &c, &title, &message, 4, s.allow_http, id)
                .await
                .map(|_| Outcome::Sent),
            _ => Err(anyhow!("Alert recipient removed or disabled")),
        }
    };
    let outcome = lease::guard(&s.db, lease::Queue::Alert, id, number, work).await;
    let mut tx = s.db.begin().await?;
    channel_health::lock(&mut tx, cid).await?;
    let owned: Option<Uuid> = sqlx::query_scalar("SELECT id FROM alert_deliveries WHERE id=$1 AND attempts=$2 AND status='processing' FOR UPDATE")
        .bind(id).bind(number).fetch_optional(&mut *tx).await?;
    if owned.is_none() { return Ok(lease::Claim::Ready(())); }
    match outcome {
        Ok(Outcome::Sent) => {
            channel_health::record(&mut tx, cid, &Ok(()), number, s.retry_seconds).await?;
            sqlx::query("UPDATE alert_deliveries SET status='sent',sent_at=NOW(),error=NULL,error_category=NULL,locked_at=NULL WHERE id=$1").bind(id).execute(&mut *tx).await?;
        }
        Ok(Outcome::Disabled) => {
            sqlx::query("UPDATE alert_deliveries SET status='failed',locked_at=NULL,error='Account or subscription disabled',error_category='configuration' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        }
        Err(error) => {
            let plan = retry_policy::plan(&error, number, s.retry_seconds);
            channel_health::record(&mut tx, cid, &Err(error), number, s.retry_seconds).await?;
            sqlx::query("UPDATE alert_deliveries SET status=CASE WHEN attempts>=5 OR $2 THEN 'failed' ELSE 'queued' END,error=(SELECT last_error FROM channel_health WHERE channel_id=$3),error_category=$4,locked_at=NULL,next_attempt_at=NOW()+$5::int*INTERVAL '1 second' WHERE id=$1")
                .bind(id).bind(plan.permanent).bind(cid).bind(plan.category).bind(plan.delay).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(lease::Claim::Ready(()))
}

pub async fn worker(s: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut last_schedule = tokio::time::Instant::now() - Duration::from_secs(10);
    loop {
        if *stop.borrow() {
            break;
        }
        if last_schedule.elapsed() >= Duration::from_secs(5) {
            if let Err(error) = schedule(&s).await {
                tracing::error!(%error,"failure alert scheduling");
            }
            last_schedule = tokio::time::Instant::now();
        }
        match send_next(&s).await {
            Ok(lease::Claim::Ready(())) => {}
            Ok(lease::Claim::Contended) => {
                runtime::pause(&mut stop, Duration::from_millis(20)).await
            }
            Ok(lease::Claim::Idle) => runtime::pause(&mut stop, Duration::from_millis(800)).await,
            Err(error) => {
                tracing::error!(%error,"failure alert worker");
                runtime::pause(&mut stop, Duration::from_secs(3)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_placeholders_are_not_recursively_expanded() {
        assert_eq!(
            render(
                "{failures}: {error}; {unknown}",
                "channel",
                "route",
                "{channel}",
                "title",
                2
            ),
            "2: {channel}; {unknown}"
        );
        assert_eq!(
            render("unfinished {error", "c", "r", "e", "t", 1),
            "unfinished {error"
        );
    }
}
