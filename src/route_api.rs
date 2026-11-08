//! Route management and endpoint keys.
use crate::*;

#[derive(Serialize, FromRow)]
pub(crate) struct Route {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) enabled: bool,
    pub(crate) min_priority: i32,
    pub(crate) rate_limit_per_minute: i32,
    pub(crate) required_tag: Option<String>,
    pub(crate) icon: String,
    pub(crate) template_id: Option<Uuid>,
    pub(crate) payload_rules: Value,
    pub(crate) created_at: DateTime<Utc>,
}
#[derive(Serialize)]
pub(crate) struct RouteResponse {
    #[serde(flatten)]
    pub(crate) route: Route,
    pub(crate) channels: Vec<Uuid>,
    pub(crate) fallback_channel_ids: Vec<Uuid>,
}
#[derive(Deserialize)]
pub(crate) struct RouteInput {
    #[serde(default)]
    pub(crate) payload_rules: Vec<payload_rules::Rule>,
    #[serde(default)]
    pub(crate) icon: Option<String>,
    #[serde(default)]
    pub(crate) template_id: Option<Uuid>,
    #[serde(default)]
    pub(crate) rate_limit_per_minute: Option<i32>,
    pub(crate) name: String,
    pub(crate) slug: Option<String>,
    pub(crate) enabled: Option<bool>,
    pub(crate) min_priority: Option<i32>,
    pub(crate) required_tag: Option<String>,
    pub(crate) channel_ids: Vec<Uuid>,
    #[serde(default)]
    pub(crate) fallback_channel_ids: Vec<Uuid>,
}
pub(crate) fn slug_valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub(crate) fn generated_slug(name: &str, id: Uuid) -> String {
    let mut prefix = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            prefix.push(ch.to_ascii_lowercase())
        } else if !prefix.is_empty() && !prefix.ends_with('-') {
            prefix.push('-')
        }
        if prefix.len() >= 40 {
            break;
        }
    }
    let prefix = prefix.trim_matches('-');
    format!(
        "{}-{}",
        if prefix.is_empty() { "route" } else { prefix },
        id.simple()
    )
}
pub(crate) fn valid_route(input: &RouteInput) -> Result<()> {
    payload_rules::validate_rules(&input.payload_rules)?;
    if input.rate_limit_per_minute.is_some_and(|n| !(0..=1_000_000).contains(&n)) {
        return Err(anyhow!("Route rate limit must be between 0 and 1000000"));
    }
    if input
        .icon
        .as_deref()
        .is_some_and(|icon| !templates::ICONS.contains(&icon))
    {
        return Err(anyhow!("Unknown route icon"));
    }
    if input.channel_ids.len() > 100 || input.fallback_channel_ids.len() > 100 {
        return Err(anyhow!(
            "Use at most 100 primary and 100 fallback recipients"
        ));
    }
    if input
        .fallback_channel_ids
        .iter()
        .any(|id| input.channel_ids.contains(id))
    {
        return Err(anyhow!("Primary and fallback recipients must be different"));
    }
    if input
        .channel_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != input.channel_ids.len()
        || input
            .fallback_channel_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.fallback_channel_ids.len()
    {
        return Err(anyhow!("Duplicate recipients are not allowed"));
    }
    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(anyhow!("Route name is required (maximum 100 characters)"));
    }
    if input
        .required_tag
        .as_ref()
        .map(|tag| tag.trim().len() > 64)
        .unwrap_or(false)
    {
        return Err(anyhow!("Tag must be at most 64 characters"));
    }
    Ok(())
}
pub(crate) async fn routes(
    State(s): State<AppState>,
    h: HeaderMap,
) -> ApiResult<Vec<RouteResponse>> {
    auth::authenticated(&h, &s).await?;
    let rows=sqlx::query_as::<_,Route>("SELECT id,name,slug,enabled,min_priority,required_tag,icon,message_template_id AS template_id,payload_rules,rate_limit_per_minute,created_at FROM routes ORDER BY created_at DESC").fetch_all(&s.db).await.map_err(db_error)?;
    let mut out = Vec::new();
    for route in rows {
        let ids: Vec<(Uuid,)> =
            sqlx::query_as("SELECT channel_id FROM route_channels WHERE route_id=$1")
                .bind(route.id)
                .fetch_all(&s.db)
                .await
                .map_err(db_error)?;
        let fallback: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT channel_id FROM route_fallback_channels WHERE route_id=$1 ORDER BY channel_id",
        )
        .bind(route.id)
        .fetch_all(&s.db)
        .await
        .map_err(db_error)?;
        out.push(RouteResponse {
            route,
            channels: ids.into_iter().map(|x| x.0).collect(),
            fallback_channel_ids: fallback.into_iter().map(|x| x.0).collect(),
        });
    }
    Ok(Json(out))
}
pub(crate) async fn create_route(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<RouteInput>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    valid_route(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let id = Uuid::new_v4();
    let slug = input
        .slug
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| generated_slug(&input.name, id));
    if !slug_valid(&slug) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid route endpoint"));
    }
    let token = key();
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let inserted=sqlx::query("INSERT INTO routes(id,name,slug,token_hash,enabled,min_priority,required_tag,icon,message_template_id,payload_rules,rate_limit_per_minute) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
 .bind(id).bind(input.name.trim()).bind(&slug).bind(hash(&token)).bind(input.enabled.unwrap_or(true)).bind(input.min_priority.unwrap_or(1).clamp(1,5)).bind(input.required_tag.map(|t|t.trim().to_owned()).filter(|t|!t.is_empty())).bind(input.icon.as_deref().unwrap_or("git-branch")).bind(input.template_id).bind(json!(input.payload_rules)).bind(input.rate_limit_per_minute.unwrap_or(0)).execute(&mut *tx).await;
    if inserted.is_err() {
        return Err(err(
            StatusCode::CONFLICT,
            "Route slug already exists or invalid input",
        ));
    }
    for cid in input.channel_ids {
        sqlx::query("INSERT INTO route_channels(route_id,channel_id) VALUES($1,$2)")
            .bind(id)
            .bind(cid)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    for cid in input.fallback_channel_ids {
        sqlx::query("INSERT INTO route_fallback_channels(route_id,channel_id) VALUES($1,$2)")
            .bind(id)
            .bind(cid)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(
        json!({"id":id,"token":token,"slug":slug,"route_name":input.name,"endpoint":format!("/h/{slug}")}),
    ))
}
pub(crate) async fn update_route(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<RouteInput>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    valid_route(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let old: Option<(String,)> = sqlx::query_as("SELECT slug FROM routes WHERE id=$1")
        .bind(id)
        .fetch_optional(&s.db)
        .await
        .map_err(db_error)?;
    let old = old
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Route not found"))?
        .0;
    if input
        .slug
        .as_ref()
        .map(|slug| slug != &old)
        .unwrap_or(false)
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Existing webhook endpoints cannot be changed",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("UPDATE routes SET name=$2,enabled=$3,min_priority=$4,required_tag=$5,icon=COALESCE($6,icon),message_template_id=$7,payload_rules=$8,rate_limit_per_minute=COALESCE($9,rate_limit_per_minute) WHERE id=$1")
 .bind(id).bind(input.name.trim()).bind(input.enabled.unwrap_or(true)).bind(input.min_priority.unwrap_or(1).clamp(1,5)).bind(input.required_tag.map(|t|t.trim().to_owned()).filter(|t|!t.is_empty())).bind(input.icon.as_deref()).bind(input.template_id).bind(json!(input.payload_rules)).bind(input.rate_limit_per_minute).execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("DELETE FROM route_channels WHERE route_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    for cid in input.channel_ids {
        sqlx::query("INSERT INTO route_channels(route_id,channel_id) VALUES($1,$2)")
            .bind(id)
            .bind(cid)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    sqlx::query("DELETE FROM route_fallback_channels WHERE route_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    for cid in input.fallback_channel_ids {
        sqlx::query("INSERT INTO route_fallback_channels(route_id,channel_id) VALUES($1,$2)")
            .bind(id)
            .bind(cid)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn delete_route(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    sqlx::query("DELETE FROM routes WHERE id=$1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn rotate_route(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let token = key();
    let result: Option<(String, String)> =
        sqlx::query_as("UPDATE routes SET token_hash=$2 WHERE id=$1 RETURNING slug,name")
            .bind(id)
            .bind(hash(&token))
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    let (slug, name) = result.ok_or_else(|| err(StatusCode::NOT_FOUND, "Route not found"))?;
    Ok(Json(
        json!({"token":token,"route_name":name,"endpoint":format!("/h/{slug}")}),
    ))
}
