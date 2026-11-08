//! Event pagination is performed before loading all of each event's recipients.
use crate::*;

pub async fn list(
    db: &sqlx::PgPool,
    filter: &DeliveryQuery,
    condition: &str,
    search: &Option<String>,
    page: i64,
    page_size: i64,
) -> ApiResult<Value> {
    let order = table::order("delivery_events", &filter.sort)?;
    let mut tx = db.begin().await.map_err(db_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    macro_rules! filtered {
        ($query:expr) => {
            $query
                .bind(&filter.status)
                .bind(filter.channel_id)
                .bind(filter.route_id)
                .bind(filter.from)
                .bind(filter.to)
                .bind(search)
                .bind(filter.tag.as_deref().filter(|t| !t.is_empty()))
                .bind(filter.event_id)
        };
    }
    let mut count = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(DISTINCT e.id)");
    count.push(condition);
    let (total,): (i64,) = filtered!(count.build_query_as())
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
    let mut events = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT e.id");
    events
        .push(condition)
        .push(" GROUP BY e.id,r.name,r.required_tag")
        .push(order)
        .push(" LIMIT $9 OFFSET $10");
    let ids: Vec<(Uuid,)> = filtered!(events.build_query_as())
        .bind(page_size)
        .bind((page - 1) * page_size)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    let ids = ids.into_iter().map(|(id,)| id).collect::<Vec<_>>();
    let mut matching = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT d.id");
    matching.push(condition).push(" AND e.id=ANY($9)");
    let matched: Vec<(Uuid,)> = filtered!(matching.build_query_as())
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    let matched = matched
        .into_iter()
        .map(|(id,)| id)
        .collect::<std::collections::HashSet<_>>();
    let mut recipients = sqlx::QueryBuilder::<sqlx::Postgres>::new(DELIVERY_SELECT);
    recipients.push(" FROM deliveries d JOIN events e ON e.id=d.event_id JOIN channels c ON c.id=d.channel_id JOIN routes r ON r.id=e.route_id WHERE e.id=ANY($1) ORDER BY LOWER(c.name),c.id,d.id");
    let rows = recipients
        .build_query_as::<Delivery>()
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    let mut groups = std::collections::HashMap::<Uuid, Vec<Delivery>>::new();
    for mut row in rows {
        row.has_title = !message::event_title(&row.payload).is_empty();
        groups.entry(row.event_id).or_default().push(row);
    }
    let items=ids.into_iter().filter_map(|id|{
  let rows=groups.remove(&id)?;let first=rows.first()?;
  let mut event=json!({"id":id,"event_id":id,"route_id":first.route_id,"route_name":first.route_name,"route_icon":first.route_icon,"route_tag":first.route_tag,"created_at":first.event_created_at,"title":first.title,"has_title":first.has_title,"message":first.message,"tags":first.tags});
  event["recipients"]=Value::Array(rows.into_iter().map(|row|json!({"id":row.id,"channel_id":row.channel_id,"channel_name":row.channel_name,"channel_kind":row.channel_kind,"status":row.status,"attempts":row.attempts,"is_fallback":row.is_fallback,"error":row.error,"created_at":row.created_at,"sent_at":row.sent_at,"matches_filter":matched.contains(&row.id)})).collect());
  Some(event)
 }).collect::<Vec<_>>();
    Ok(Json(
        json!({"items":items,"total":total,"page":page,"page_size":page_size,"grouped":true}),
    ))
}
