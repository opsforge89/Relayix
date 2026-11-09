//! Expiring, account-owned retry previews; confirmation is atomic and idempotent.
use crate::*;
fn owner(principal: &auth::Principal) -> String {
    principal
        .id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "administrator-api-token".into())
}
#[derive(Deserialize)]
pub struct Input {
    #[serde(default)]
    filters: DeliveryQuery,
}
pub async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<Input>,
) -> ApiResult<Value> {
    let user = auth::authenticated(&h, &s).await?;
    let mut filter = input.filters;
    if filter
        .status
        .as_deref()
        .is_some_and(|v| !v.is_empty() && v != "failed")
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Batch retry only accepts final failures; select Failed or All statuses",
        ));
    }
    filter.status = Some("failed".into());
    normalize_delivery_filter(&mut filter)?;
    let search = filter
        .search
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|s| format!("%{s}%"));
    let condition = if filter.group_events.unwrap_or(false) {
        DELIVERY_CONDITION.replace("d.created_at", "e.created_at")
    } else {
        DELIVERY_CONDITION.to_owned()
    };
    macro_rules! bind {
        ($q:expr) => {
            $q.bind(&filter.status)
                .bind(filter.channel_id)
                .bind(filter.route_id)
                .bind(filter.from)
                .bind(filter.to)
                .bind(&search)
                .bind(filter.tag.as_deref().filter(|s| !s.is_empty()))
                .bind(filter.event_id)
        };
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let mut count = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*)");
    count.push(&condition);
    let (total,): (i64,) = bind!(count.build_query_as())
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let mut query=sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT jsonb_build_object('id',d.id,'title',e.title,'channel_name',c.name,'route_name',r.name,'created_at',d.created_at)");
    query
        .push(&condition)
        .push(" ORDER BY d.created_at DESC,d.id DESC LIMIT 500");
    let rows: Vec<(Value,)> = bind!(query.build_query_as())
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    let items: Vec<Value> = rows.into_iter().map(|r| r.0).collect();
    let ids: Vec<Uuid> = items
        .iter()
        .map(|item| Uuid::parse_str(item["id"].as_str().unwrap()).unwrap())
        .collect();
    let id = Uuid::new_v4();
    sqlx::query("DELETE FROM delivery_retry_batches WHERE expires_at<NOW()-INTERVAL '1 day'")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let (expires_at,):(DateTime<Utc>,)=sqlx::query_as("INSERT INTO delivery_retry_batches(id,owner_key,delivery_ids) VALUES($1,$2,$3) RETURNING expires_at").bind(id).bind(owner(&user)).bind(&ids).fetch_one(&mut *tx).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"batch_id":id,"total":total,"count":ids.len(),"limit":500,"expires_at":expires_at,"items":items}),
    ))
}
pub async fn execute(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let user = auth::authenticated(&h, &s).await?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let row:Option<(Vec<Uuid>,Option<i32>,bool)>=sqlx::query_as("SELECT delivery_ids,queued,expires_at>NOW() FROM delivery_retry_batches WHERE id=$1 AND owner_key=$2 FOR UPDATE").bind(id).bind(owner(&user)).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let (ids, queued, valid) =
        row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Retry preview not found"))?;
    if let Some(queued) = queued {
        return Ok(Json(
            json!({"queued":queued,"skipped":ids.len()-queued as usize,"duplicate":true}),
        ));
    }
    if !valid {
        return Err(err(
            StatusCode::CONFLICT,
            "Retry preview expired; preview the current failures again",
        ));
    }
    let result=sqlx::query("UPDATE deliveries SET status='queued',next_attempt_at=GREATEST(NOW(),not_before),locked_at=NULL,error=NULL,error_category=NULL,cycle_attempts=0,last_error_at=NULL,retry_generation=retry_generation+1 WHERE id=ANY($1) AND status='failed'").bind(&ids).execute(&mut *tx).await.map_err(db_error)?;
    let queued = result.rows_affected() as i32;
    sqlx::query("UPDATE delivery_retry_batches SET queued=$2 WHERE id=$1")
        .bind(id)
        .bind(queued)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"queued":queued,"skipped":ids.len()-queued as usize,"duplicate":false}),
    ))
}
