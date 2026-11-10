//! Successful control changes and their actors, committed with the affected records.
use crate::*;
pub async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &auth::Principal,
    action: &'static str,
    target_type: &'static str,
    target: Uuid,
    affected: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO audit_log(actor_id,actor,auth_method,action,target_type,target_id,affected) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(actor.id).bind(&actor.username).bind(actor.auth_method).bind(action).bind(target_type).bind(target).bind(affected).execute(&mut **tx).await?;
    Ok(())
}
pub async fn system(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    action: &'static str,
    affected: i64,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO audit_log(actor,auth_method,action,target_type,affected) VALUES('system','local',$1,'system',$2)").bind(action).bind(affected).execute(&mut **tx).await?;
    Ok(())
}
#[derive(Deserialize)]
pub struct Filter {
    page: Option<i64>,
    target_id: Option<Uuid>,
}
pub async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(filter): Query<Filter>,
) -> ApiResult<Value> {
    auth::require_admin(&h, &s).await?;
    let page = filter.page.unwrap_or(1).clamp(1, 1000000);
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE $1::uuid IS NULL OR target_id=$1")
            .bind(filter.target_id)
            .fetch_one(&s.db)
            .await
            .map_err(db_error)?;
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('id',id,'actor_id',actor_id,'actor',actor,'auth_method',auth_method,'action',action,'target_type',target_type,'target_id',target_id,'affected',affected,'created_at',created_at) FROM audit_log WHERE $1::uuid IS NULL OR target_id=$1 ORDER BY created_at DESC,id DESC LIMIT 50 OFFSET $2")
        .bind(filter.target_id).bind((page-1)*50).fetch_all(&s.db).await.map_err(db_error)?;
    Ok(Json(
        json!({"items":rows.into_iter().map(|r|r.0).collect::<Vec<_>>(),"total":total,"page":page,"page_size":50}),
    ))
}
