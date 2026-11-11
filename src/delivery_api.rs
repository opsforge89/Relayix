//! Delivery history, filters and manual retries.
use crate::*;

#[derive(Serialize, FromRow)]
pub(crate) struct Delivery {
    pub(crate) id: Uuid,
    pub(crate) event_id: Uuid,
    pub(crate) route_id: Uuid,
    pub(crate) channel_id: Uuid,
    pub(crate) channel_name: String,
    pub(crate) channel_kind: String,
    pub(crate) route_name: String,
    pub(crate) route_icon: String,
    pub(crate) title: String,
    pub(crate) tags: Value,
    pub(crate) route_tag: Option<String>,
    pub(crate) status: String,
    pub(crate) attempts: i32,
    pub(crate) is_fallback: bool,
    pub(crate) error: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) sent_at: Option<DateTime<Utc>>,
    pub(crate) message: String,
    pub(crate) has_title: bool,
    #[serde(skip)]
    pub(crate) payload: Value,
    #[serde(skip)]
    pub(crate) event_created_at: DateTime<Utc>,
}
pub(crate) const DELIVERY_SELECT:&str="SELECT d.id,d.event_id,e.route_id,d.channel_id,c.name AS channel_name,c.kind AS channel_kind,r.name AS route_name,r.icon AS route_icon,e.title,e.tags,NULLIF(BTRIM(r.required_tag),'') AS route_tag,d.status,d.attempts,d.is_fallback,d.error,d.created_at,d.sent_at,e.message,FALSE AS has_title,COALESCE(e.rendered_payload,e.payload) AS payload,e.created_at AS event_created_at";
#[derive(Deserialize, Default)]
pub(crate) struct DeliveryQuery {
    pub(crate) status: Option<String>,
    pub(crate) channel_id: Option<Uuid>,
    pub(crate) route_id: Option<Uuid>,
    pub(crate) event_id: Option<Uuid>,
    pub(crate) from: Option<DateTime<Utc>>,
    pub(crate) to: Option<DateTime<Utc>>,
    pub(crate) group_events: Option<bool>,
    pub(crate) search: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) period: Option<String>,
    pub(crate) page: Option<i64>,
    pub(crate) page_size: Option<i64>,
    #[serde(flatten)]
    pub(crate) sort: table::Sort,
}
pub(crate) const DELIVERY_CONDITION:&str=" FROM deliveries d JOIN events e ON e.id=d.event_id JOIN channels c ON c.id=d.channel_id JOIN routes r ON r.id=e.route_id WHERE ($1::text IS NULL OR d.status=$1 OR ($1='retrying' AND d.status='queued' AND d.error IS NOT NULL)) AND ($2::uuid IS NULL OR d.channel_id=$2) AND ($3::uuid IS NULL OR e.route_id=$3) AND ($4::timestamptz IS NULL OR d.created_at>=$4) AND ($5::timestamptz IS NULL OR d.created_at<=$5) AND ($6::text IS NULL OR e.title ILIKE $6 OR e.message ILIKE $6 OR c.name ILIKE $6 OR r.name ILIKE $6 OR COALESCE(d.error,'') ILIKE $6) AND ($7::text IS NULL OR e.tags @> jsonb_build_array($7::text) OR NULLIF(BTRIM(r.required_tag),'')=$7::text) AND ($8::uuid IS NULL OR e.id=$8)";
pub(crate) fn normalize_delivery_filter(
    filter: &mut DeliveryQuery,
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    if filter
        .status
        .as_deref()
        .map(|v| {
            !matches!(
                v,
                "sent" | "failed" | "queued" | "processing" | "retrying" | "cancelled" | "expired"
            )
        })
        .unwrap_or(false)
    {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid delivery status"));
    }
    if filter
        .from
        .zip(filter.to)
        .map(|(from, to)| from > to)
        .unwrap_or(false)
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Start date must precede end date",
        ));
    }
    if let Some(period) = filter.period.as_deref().filter(|p| !p.is_empty()) {
        let duration = monitoring_period(period)
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Invalid monitoring period"))?;
        let to = Utc::now();
        let from = to - duration;
        filter.from = Some(filter.from.map(|old| old.max(from)).unwrap_or(from));
        filter.to = Some(filter.to.map(|old| old.min(to)).unwrap_or(to));
    }
    if filter
        .search
        .as_ref()
        .map(|v| v.len() > 250)
        .unwrap_or(false)
    {
        return Err(err(StatusCode::BAD_REQUEST, "Search is too long"));
    }
    if filter.tag.as_ref().map(|v| v.len() > 64).unwrap_or(false) {
        return Err(err(StatusCode::BAD_REQUEST, "Tag is too long"));
    }
    Ok(())
}
pub(crate) async fn deliveries(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(mut filter): Query<DeliveryQuery>,
) -> ApiResult<Value> {
    let actor=auth::authenticated(&h,&s).await?;
    normalize_delivery_filter(&mut filter)?;
    let page = filter.page.unwrap_or(1).clamp(1, 1000000);
    let page_size = filter.page_size.unwrap_or(50).clamp(1, 100);
    let search = filter
        .search
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(|v| format!("%{v}%"));
    let condition=format!("{DELIVERY_CONDITION} AND {}",access::route_scope(&actor,"e.route_id"));
    if filter.group_events.unwrap_or(false) {
        return delivery_groups::list(
            &s.db,
            &filter,
            &condition.replace("d.created_at", "e.created_at").replace(
                "d.status=$1",
                "(d.status=$1 AND ($1<>'queued' OR d.error IS NULL))",
            ),
            &search,
            page,
            page_size,
        )
        .await;
    }
    let order = table::order("deliveries", &filter.sort)?;
    let mut count_sql = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*)");
    count_sql.push(&condition);
    let (total,): (i64,) = count_sql
        .build_query_as::<(i64,)>()
        .bind(&filter.status)
        .bind(filter.channel_id)
        .bind(filter.route_id)
        .bind(filter.from)
        .bind(filter.to)
        .bind(&search)
        .bind(filter.tag.as_deref().filter(|t| !t.is_empty()))
        .bind(filter.event_id)
        .fetch_one(&s.db)
        .await
        .map_err(db_error)?;
    let mut rows_sql = sqlx::QueryBuilder::<sqlx::Postgres>::new(DELIVERY_SELECT);
    rows_sql
        .push(&condition)
        .push(order)
        .push(" LIMIT $9 OFFSET $10");
    let mut items = rows_sql
        .build_query_as::<Delivery>()
        .bind(&filter.status)
        .bind(filter.channel_id)
        .bind(filter.route_id)
        .bind(filter.from)
        .bind(filter.to)
        .bind(&search)
        .bind(filter.tag.as_deref().filter(|t| !t.is_empty()))
        .bind(filter.event_id)
        .bind(page_size)
        .bind((page - 1) * page_size)
        .fetch_all(&s.db)
        .await
        .map_err(db_error)?;
    for row in &mut items {
        let title = message::event_title(&row.payload);
        row.has_title = !title.is_empty();
    }
    Ok(Json(
        json!({"items":items,"total":total,"page":page,"page_size":page_size}),
    ))
}
pub(crate) async fn retry(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let actor=auth::authenticated(&h,&s).await?;
    access::delivery(&s.db,&actor,id,true).await?;
    let result=sqlx::query("UPDATE deliveries SET status='queued',next_attempt_at=GREATEST(NOW(),not_before),locked_at=NULL,error=NULL,error_category=NULL,cycle_attempts=0,last_error_at=NULL,retry_generation=retry_generation+1 WHERE id=$1 AND (expires_at IS NULL OR expires_at>NOW()) AND (status='failed' OR (status='queued' AND error IS NOT NULL))").bind(id).execute(&s.db).await.map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(err(
            StatusCode::CONFLICT,
            "Only failed or retrying deliveries can be retried",
        ));
    }
    Ok(Json(json!({"ok":true})))
}
