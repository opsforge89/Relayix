//! Channel permissions are independent; only owners and administrators manage grants.
use crate::*;

pub type Check = std::result::Result<(), (StatusCode, Json<Value>)>;

// Only typed UUIDs and static permission names reach these SQL fragments.
// No user-supplied SQL, names, or identifiers are interpolated.
pub fn channel_scope(actor: &auth::Principal, column: &str, permission: &'static str) -> String {
    if actor.role == "admin" {
        "TRUE".into()
    } else if let Some(id) = actor.id {
        format!("relay_channel_access({column},'{id}'::uuid,'{permission}')")
    } else {
        "FALSE".into()
    }
}
pub fn route_scope(actor: &auth::Principal, column: &str) -> String {
    if actor.role == "admin" {
        "TRUE".into()
    } else if let Some(id) = actor.id {
        format!("relay_route_view({column},'{id}'::uuid)")
    } else {
        "FALSE".into()
    }
}
pub async fn channel(
    db: &PgPool,
    actor: &auth::Principal,
    id: Uuid,
    permission: &'static str,
) -> Check {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE id=$1 AND ($2 OR relay_channel_access(id,$3,$4)))")
        .bind(id).bind(actor.role == "admin").bind(actor.id).bind(permission).fetch_one(db).await.map_err(db_error)?;
    if !allowed {
        return Err(err(StatusCode::FORBIDDEN, "Channel permission required"));
    }
    Ok(())
}
pub async fn manage(db: &PgPool, actor: &auth::Principal, id: Uuid) -> Check {
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM channels WHERE id=$1 AND ($2 OR owner_user_id=$3))",
    )
    .bind(id)
    .bind(actor.role == "admin")
    .bind(actor.id)
    .fetch_one(db)
    .await
    .map_err(db_error)?;
    if !allowed {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Channel owner or administrator required",
        ));
    }
    Ok(())
}
pub async fn route(db: &PgPool, actor: &auth::Principal, id: Uuid, write: bool) -> Check {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM routes WHERE id=$1 AND ($2 OR (CASE WHEN $4 THEN owner_user_id=$3 ELSE relay_route_view(id,$3) END)))")
        .bind(id).bind(actor.role == "admin").bind(actor.id).bind(write).fetch_one(db).await.map_err(db_error)?;
    if !allowed {
        return Err(err(StatusCode::FORBIDDEN, "Route permission required"));
    }
    Ok(())
}
pub async fn event(db: &PgPool, actor: &auth::Principal, id: Uuid) -> Check {
    let rid: Option<Uuid> = sqlx::query_scalar("SELECT route_id FROM events WHERE id=$1")
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(db_error)?;
    route(
        db,
        actor,
        rid.ok_or_else(|| err(StatusCode::NOT_FOUND, "Event not found"))?,
        false,
    )
    .await
}
pub async fn delivery(db: &PgPool, actor: &auth::Principal, id: Uuid, send: bool) -> Check {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT channel_id,event_id FROM deliveries WHERE id=$1")
            .bind(id)
            .fetch_optional(db)
            .await
            .map_err(db_error)?;
    let (cid, eid) = row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Delivery not found"))?;
    event(db, actor, eid).await?;
    channel(db, actor, cid, if send { "send" } else { "view" }).await
}
pub async fn recipients(db: &PgPool, actor: &auth::Principal, ids: &[Uuid]) -> Check {
    for id in ids {
        channel(db, actor, *id, "send").await?;
    }
    Ok(())
}
pub async fn route_recipients(db: &PgPool, actor: &auth::Principal, id: Uuid) -> Check {
    route(db, actor, id, true).await?;
    let ids:Vec<Uuid>=sqlx::query_scalar("SELECT channel_id FROM route_channels WHERE route_id=$1 UNION SELECT channel_id FROM route_fallback_channels WHERE route_id=$1").bind(id).fetch_all(db).await.map_err(db_error)?;
    recipients(db, actor, &ids).await
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    user_id: Uuid,
    can_view: bool,
    can_edit: bool,
    can_send: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sharing {
    owner_user_id: Option<Uuid>,
    grants: Vec<Grant>,
}
pub async fn get(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let actor = auth::authenticated(&h, &s).await?;
    manage(&s.db, &actor, id).await?;
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_user_id FROM channels WHERE id=$1")
        .bind(id)
        .fetch_one(&s.db)
        .await
        .map_err(db_error)?;
    let grants:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('user_id',user_id,'can_view',can_view,'can_edit',can_edit,'can_send',can_send) FROM channel_grants WHERE channel_id=$1 ORDER BY user_id").bind(id).fetch_all(&s.db).await.map_err(db_error)?;
    // A minimal directory is available only to channel owners, never password/session data.
    let users:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('id',id,'username',username,'display_name',display_name) FROM users WHERE enabled ORDER BY username").fetch_all(&s.db).await.map_err(db_error)?;
    Ok(Json(
        json!({"owner_user_id":owner,"grants":grants.into_iter().map(|x|x.0).collect::<Vec<_>>(),"users":users.into_iter().map(|x|x.0).collect::<Vec<_>>()}),
    ))
}
pub async fn put(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Sharing>,
) -> ApiResult<Value> {
    let actor = auth::authenticated(&h, &s).await?;
    if input.grants.len() > 100
        || input
            .grants
            .iter()
            .map(|g| g.user_id)
            .collect::<std::collections::HashSet<_>>()
            .len()
            != input.grants.len()
    {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Use at most 100 distinct collaborators",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let owner: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT owner_user_id FROM channels WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
    let owner = owner.ok_or_else(|| err(StatusCode::NOT_FOUND, "Channel not found"))?;
    if actor.role != "admin" && (actor.id.is_none() || owner != actor.id) {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Channel owner or administrator required",
        ));
    }
    if owner != input.owner_user_id && actor.role != "admin" {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Administrator required to transfer ownership",
        ));
    }
    let mut ids = input.grants.iter().map(|g| g.user_id).collect::<Vec<_>>();
    if let Some(owner) = input.owner_user_id.filter(|v| Some(*v) != owner) {
        ids.push(owner);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT id) FROM users WHERE id=ANY($1) AND enabled")
            .bind(&ids)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    if count as usize != ids.iter().collect::<std::collections::HashSet<_>>().len() {
        return Err(err(StatusCode::BAD_REQUEST, "Choose active users"));
    }
    sqlx::query("UPDATE channels SET owner_user_id=$2 WHERE id=$1")
        .bind(id)
        .bind(input.owner_user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM channel_grants WHERE channel_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    for g in input.grants {
        if g.can_view || g.can_edit || g.can_send {
            sqlx::query("INSERT INTO channel_grants(channel_id,user_id,can_view,can_edit,can_send) VALUES($1,$2,$3,$4,$5)").bind(id).bind(g.user_id).bind(g.can_view).bind(g.can_edit).bind(g.can_send).execute(&mut *tx).await.map_err(db_error)?;
        }
    }
    audit::record(&mut tx, &actor, "channel.access", "channel", id, None)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
