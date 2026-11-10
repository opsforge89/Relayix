//! Incoming history, delivery attempts and bounded fallback dispatch.
use crate::*;

#[derive(Deserialize)]
pub struct EventQuery {
    route_id: Option<Uuid>,
    reason: Option<String>,
    search: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
    #[serde(flatten)]
    sort: table::Sort,
}
pub async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<EventQuery>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    if q.search.as_ref().is_some_and(|v| v.len() > 250)
        || q.reason.as_ref().is_some_and(|v| v.len() > 64)
    {
        return Err(err(StatusCode::BAD_REQUEST, "History filter is too long"));
    }
    let order = table::order("incoming", &q.sort)?;
    let page = q.page.unwrap_or(1).clamp(1, 1000000);
    let size = q.page_size.unwrap_or(25).clamp(1, 100);
    let search = q.search.filter(|v| !v.is_empty()).map(|v| format!("%{v}%"));
    let condition=" FROM events e JOIN routes r ON r.id=e.route_id WHERE ($1::uuid IS NULL OR e.route_id=$1) AND ($2::text IS NULL OR COALESCE(e.routing->>'reason','legacy')=$2) AND ($3::text IS NULL OR e.title ILIKE $3 OR e.message ILIKE $3 OR r.name ILIKE $3)";
    let mut count = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*)");
    count.push(condition);
    let (total,): (i64,) = count
        .build_query_as()
        .bind(q.route_id)
        .bind(q.reason.as_deref().filter(|v| !v.is_empty()))
        .bind(&search)
        .fetch_one(&s.db)
        .await
        .map_err(db_error)?;
    let mut rows=sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT jsonb_build_object('id',e.id,'route_id',e.route_id,'route_name',r.name,'route_icon',r.icon,'title',e.title,'message',e.message,'priority',e.priority,'tags',e.tags,'routing',e.routing,'_source',COALESCE(e.rendered_payload,e.payload),'created_at',e.created_at,'attachment_count',(SELECT COUNT(*) FROM event_attachments a WHERE a.event_id=e.id),'deliveries',(SELECT COUNT(*) FROM deliveries d WHERE d.event_id=e.id),'sent',(SELECT COUNT(*) FROM deliveries d WHERE d.event_id=e.id AND d.status='sent'))");
    rows.push(condition).push(order).push(" LIMIT $4 OFFSET $5");
    let items: Vec<(Value,)> = rows
        .build_query_as()
        .bind(q.route_id)
        .bind(q.reason.as_deref().filter(|v| !v.is_empty()))
        .bind(search)
        .bind(size)
        .bind((page - 1) * size)
        .fetch_all(&s.db)
        .await
        .map_err(db_error)?;
    let items: Vec<Value> = items
        .into_iter()
        .map(|(mut item,)| {
            let payload = item.as_object_mut().unwrap().remove("_source").unwrap();
            let title = message::event_title(&payload);
            item["has_title"] = json!(!title.is_empty());
            item
        })
        .collect();
    Ok(Json(
        json!({"items":items,"total":total,"page":page,"page_size":size}),
    ))
}
pub async fn event_details(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let row:Option<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('id',e.id,'route_id',e.route_id,'route_name',r.name,'route_icon',r.icon,'title',e.title,'message',e.message,'priority',e.priority,'tags',e.tags,'routing',e.routing,'created_at',e.created_at,'original_payload',e.payload,'rendered_payload',e.rendered_payload,'template',e.template_snapshot,'template_result',e.template_result,'has_title',COALESCE(e.rendered_payload,e.payload)->>'title' IS NOT NULL AND BTRIM(COALESCE(e.rendered_payload,e.payload)->>'title')<>'','recipients',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',d.id,'channel_id',c.id,'channel_name',c.name,'channel_kind',c.kind,'status',d.status,'attempts',d.attempts,'is_fallback',d.is_fallback,'error',d.error,'created_at',d.created_at,'sent_at',d.sent_at) ORDER BY LOWER(c.name),c.id) FROM deliveries d JOIN channels c ON c.id=d.channel_id WHERE d.event_id=e.id),'[]'::jsonb)) FROM events e JOIN routes r ON r.id=e.route_id WHERE e.id=$1").bind(id).fetch_optional(&s.db).await.map_err(db_error)?;
    let mut value = row
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Event not found"))?
        .0;
    let source = if value["rendered_payload"].is_object() {
        &value["rendered_payload"]
    } else {
        &value["original_payload"]
    };
    let has_title = !message::event_title(source).is_empty();
    value["has_title"] = json!(has_title);
    let group:Option<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('id',g.id,'kind',g.kind,'occurrences',g.occurrences,'leader_event_id',g.leader_event_id,'closes_at',g.closes_at,'members',(SELECT jsonb_agg(jsonb_build_object('id',m.id,'title',m.title,'message',m.message) ORDER BY m.created_at,m.id) FROM (SELECT id,title,message,created_at FROM events WHERE group_id=g.id ORDER BY created_at,id LIMIT 50) m)) FROM event_groups g JOIN events e ON e.group_id=g.id WHERE e.id=$1")
        .bind(id).fetch_optional(&s.db).await.map_err(db_error)?;
    value["delivery_group"] = group.map(|g| g.0).unwrap_or(Value::Null);
    value["attachments"] = json!(attachments::metadata(&s.db, id).await.map_err(db_error)?);
    Ok(Json(value))
}
#[derive(FromRow)]
struct AttemptSummary {
    attempts: i32,
    status: String,
    is_fallback: bool,
    error: Option<String>,
    next_attempt_at: DateTime<Utc>,
    channel_id: Uuid,
    error_category: Option<String>,
}
pub async fn attempts(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let delivery: Option<AttemptSummary> =
        sqlx::query_as("SELECT attempts,status,is_fallback,error,next_attempt_at,channel_id,error_category FROM deliveries WHERE id=$1")
            .bind(id)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    let AttemptSummary {
        attempts: total,
        status,
        is_fallback,
        error,
        next_attempt_at,
        channel_id,
        error_category,
    } = delivery.ok_or_else(|| err(StatusCode::NOT_FOUND, "Delivery not found"))?;
    let items:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('number',number,'started_at',started_at,'finished_at',finished_at,'status',status,'error',error,'error_category',error_category) FROM delivery_attempts WHERE delivery_id=$1 ORDER BY number DESC LIMIT 100").bind(id).fetch_all(&s.db).await.map_err(db_error)?;
    let health = channel_health::status(&s.db, channel_id)
        .await
        .map_err(db_error)?;
    Ok(Json(
        json!({"next_attempt_at":if status=="queued" {Some(next_attempt_at)} else {None},"error":error,"error_category":error_category,"channel_health":health,"delivery_id":id,"attempts":total,"status":status,"is_fallback":is_fallback,"items":items.into_iter().map(|r|r.0).collect::<Vec<_>>(),"history_limit":100,"can_retry":status=="failed"||(status=="queued"&&error.is_some())}),
    ))
}

async fn claim(s: &AppState) -> Result<lease::Claim<(Uuid, i32, Uuid, i32, Value)>> {
    let mut tx = s.db.begin().await?;
    let channel = match lease::claim_channel(
        &mut tx,
        lease::Queue::Delivery,
        s.channel_concurrency,
        s.proxy_concurrency,
    )
    .await?
    {
        lease::Claim::Ready(channel) => channel,
        lease::Claim::Idle => return Ok(lease::Claim::Idle),
        lease::Claim::Contended => return Ok(lease::Claim::Contended),
    };
    let row:Option<(Uuid,i32,Uuid,i32,Value)>=sqlx::query_as(
  "SELECT d.id,e.priority,d.channel_id,d.attempts+1 AS number,COALESCE(e.rendered_payload,e.payload) || jsonb_build_object('__relayix_group',e.group_id,'__relayix_policy',r.delivery_settings,'__relayix_route_id',e.route_id,'__relayix_group_key',(SELECT group_key FROM event_groups WHERE id=e.group_id)) FROM deliveries d JOIN events e ON e.id=d.event_id JOIN routes r ON r.id=e.route_id WHERE d.channel_id=$1 AND ((d.status='queued' AND d.next_attempt_at<=NOW() AND d.not_before<=NOW()) OR (d.status='processing' AND d.locked_at<NOW()-INTERVAL '2 minutes')) ORDER BY d.created_at,d.id FOR UPDATE OF d SKIP LOCKED LIMIT 1").bind(channel).fetch_optional(&mut *tx).await?;
    if let Some((id, priority, _, _, payload)) = &row {
        if !event_groups::try_claim(&mut tx, payload).await? {
            return Ok(lease::Claim::Contended);
        }
        let policy = delivery_policy::Settings::parse(&payload["__relayix_policy"])?;
        let now = Utc::now();
        let allowed = policy.allowed_at(now, *priority);
        if allowed > now {
            sqlx::query("UPDATE delivery_attempts SET status='interrupted',finished_at=NOW(),error='Worker lease expired; deferred by quiet hours' WHERE delivery_id=$1 AND status='processing'").bind(id).execute(&mut *tx).await?;
            sqlx::query("UPDATE deliveries SET status='queued',locked_at=NULL,next_attempt_at=GREATEST(next_attempt_at,$2) WHERE id=$1").bind(id).bind(allowed).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(lease::Claim::Contended);
        }
    }
    if let Some((id, _, _, number, _)) = &row {
        sqlx::query("UPDATE delivery_attempts SET status='interrupted',finished_at=NOW(),error='Worker lease expired; outcome unknown' WHERE delivery_id=$1 AND status='processing'").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE deliveries SET status='processing',locked_at=NOW(),attempts=attempts+1,cycle_attempts=cycle_attempts+1 WHERE id=$1").bind(id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO delivery_attempts(delivery_id,number) VALUES($1,$2)")
            .bind(id)
            .bind(number)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(match row {
        Some(row) => lease::Claim::Ready(row),
        None => lease::Claim::Contended,
    })
}
async fn finish(
    s: &AppState,
    id: Uuid,
    channel: Uuid,
    number: i32,
    result: Result<()>,
) -> Result<()> {
    let mut tx = s.db.begin().await?;
    channel_health::lock(&mut tx, channel).await?;
    let row:Option<(Uuid,bool,i32)>=sqlx::query_as("SELECT event_id,is_fallback,cycle_attempts FROM deliveries WHERE id=$1 AND attempts=$2 AND status='processing' FOR UPDATE").bind(id).bind(number).fetch_optional(&mut *tx).await?;
    let Some((event_id, is_fallback, cycle_attempts)) = row else {
        return Ok(());
    };
    let plan = result
        .as_ref()
        .err()
        .map(|e| retry_policy::plan(e, cycle_attempts, s.retry_seconds));
    channel_health::record(&mut tx, channel, &result, cycle_attempts, s.retry_seconds).await?;
    let error = result
        .err()
        .map(|e| e.to_string().chars().take(512).collect::<String>());
    let status = if error.is_none() {
        "sent"
    } else if cycle_attempts >= 5 || plan.as_ref().is_some_and(|p| p.permanent) {
        "failed"
    } else {
        "queued"
    };
    sqlx::query("UPDATE delivery_attempts SET status=$3,error=$4,error_category=$5,finished_at=NOW() WHERE delivery_id=$1 AND number=$2").bind(id).bind(number).bind(if error.is_none(){"sent"}else{"failed"}).bind(&error).bind(plan.as_ref().map(|p| p.category)).execute(&mut *tx).await?;
    sqlx::query("UPDATE deliveries SET status=$2,error=$3,locked_at=NULL,sent_at=CASE WHEN $2='sent' THEN NOW() ELSE NULL END,last_error_at=CASE WHEN $3::text IS NULL THEN NULL ELSE NOW() END,next_attempt_at=NOW()+$4::int*INTERVAL '1 second',error_category=$5 WHERE id=$1").bind(id).bind(status).bind(error).bind(plan.as_ref().map(|p| p.delay).unwrap_or(0)).bind(plan.as_ref().map(|p| p.category)).execute(&mut *tx).await?;
    if status == "failed" && !is_fallback {
        let targets:Vec<(Uuid,)>=sqlx::query_as("SELECT f.channel_id FROM event_fallback_channels f JOIN channels c ON c.id=f.channel_id WHERE f.event_id=$1 AND c.enabled ORDER BY f.channel_id").bind(event_id).fetch_all(&mut *tx).await?;
        for (cid,) in targets {
            let result=sqlx::query("INSERT INTO deliveries(id,event_id,channel_id,is_fallback) VALUES($1,$2,$3,TRUE) ON CONFLICT(event_id,channel_id) DO NOTHING").bind(Uuid::new_v4()).bind(event_id).bind(cid).execute(&mut *tx).await?;
            if result.rows_affected() > 0 {
                tracing::info!(%event_id,channel_id=%cid,failed_delivery_id=%id,"fallback delivery queued");
            }
        }
    }
    tx.commit().await?;
    Ok(())
}
async fn deliver_owned(
    s: &AppState,
    id: Uuid,
    cid: Uuid,
    number: i32,
    priority: i32,
    payload: &Value,
) -> Result<()> {
    let work = async {
        let payload = event_groups::presentation(&s.db, payload.clone()).await?;
        let channel = sqlx::query_as::<_, Channel>("SELECT id,name,kind,config,enabled,manual_paused,manual_pause_until,created_at FROM channels WHERE id=$1")
            .bind(cid)
            .fetch_optional(&s.db)
            .await;
        match channel {
            Ok(Some(c)) if c.enabled => {
                let files = attachments::for_delivery(&s.db, id).await?;
                attachments::deliver(s, &c, &payload, priority, id, &files).await
            }
            Ok(Some(_)) => Err(retry_policy::permanent("Channel disabled")),
            Ok(None) => Err(retry_policy::permanent("Channel removed")),
            Err(_) => Err(anyhow!("Channel lookup failed")),
        }
    };
    lease::guard(&s.db, lease::Queue::Delivery, id, number, work).await
}

pub async fn worker(s: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *stop.borrow() {
            break;
        }
        match claim(&s).await {
            Ok(lease::Claim::Ready((id, priority, cid, number, payload))) => {
                let result = deliver_owned(&s, id, cid, number, priority, &payload).await;
                if let Err(e) = finish(&s, id, cid, number, result).await {
                    tracing::error!(%e,%id,"worker result update failed");
                    runtime::pause(&mut stop, Duration::from_secs(3)).await;
                }
            }
            Ok(lease::Claim::Contended) => {
                runtime::pause(&mut stop, Duration::from_millis(20)).await
            }
            Ok(lease::Claim::Idle) => runtime::pause(&mut stop, Duration::from_millis(800)).await,
            Err(e) => {
                tracing::warn!(%e,"worker poll error");
                runtime::pause(&mut stop, Duration::from_secs(3)).await;
            }
        }
    }
}
