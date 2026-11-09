//! Opt-in administrator TOTP enrollment, one-use codes and recovery credentials.
use crate::*;
use totp_rs::{Builder, Secret, Totp};

fn authenticator(secret: &str, account: &str) -> Result<Totp> {
    let secret = Secret::try_from_base32(secret)
        .map_err(|_| anyhow!("Invalid stored authenticator secret"))?;
    Builder::new()
        .with_secret(secret)
        .with_issuer(Some("Relayix"))
        .with_account_name(account)
        .build()
        .map_err(|_| anyhow!("Unable to configure authenticator"))
}
async fn failed(db: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE user_totp SET failures=LEAST(failures+1,100),locked_until=CASE WHEN failures>=4 THEN NOW()+INTERVAL '15 minutes' ELSE locked_until END WHERE user_id=$1")
        .bind(id).execute(db).await?;
    Ok(())
}
fn recovery_hash(code: &str) -> String {
    hash(&code.trim().replace('-', "").to_ascii_lowercase())
}

/// A matched step or recovery code is consumed under a database row lock across replicas.
pub async fn consume(db: &PgPool, id: Uuid, code: &str) -> Result<(bool, bool)> {
    let mut tx = db.begin().await?;
    let row: Option<(Option<String>, i64, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT secret,last_step,locked_until FROM user_totp WHERE user_id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((Some(secret), last, locked)) = row else {
        return Ok((true, false));
    };
    if locked.is_some_and(|date| date > Utc::now()) {
        return Ok((false, true));
    }
    let step = if code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) {
        authenticator(&secret, "account")?
            .check(code, Utc::now().timestamp().max(0) as u64)
            .filter(|step| last < 0 || *step > last as u64)
    } else {
        None
    };
    let recovery = if step.is_none() && (32..=40).contains(&code.len()) {
        sqlx::query("DELETE FROM totp_recovery_codes WHERE user_id=$1 AND code_hash=$2")
            .bind(id)
            .bind(recovery_hash(code))
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1
    } else {
        false
    };
    let valid = step.is_some() || recovery;
    if valid {
        sqlx::query("UPDATE user_totp SET last_step=COALESCE($2,last_step),failures=0,locked_until=NULL WHERE user_id=$1")
            .bind(id).bind(step.map(|s|s as i64)).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE user_totp SET failures=LEAST(failures+1,100),locked_until=CASE WHEN failures>=4 THEN NOW()+INTERVAL '15 minutes' ELSE locked_until END WHERE user_id=$1")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((valid, true))
}
#[derive(Deserialize)]
pub struct PasswordInput {
    password: String,
    #[serde(default)]
    code: String,
}
#[derive(Deserialize)]
pub struct CodeInput {
    code: String,
}
async fn account(
    s: &AppState,
    h: &HeaderMap,
) -> std::result::Result<auth::Principal, (StatusCode, Json<Value>)> {
    let actor = auth::require_admin(h, s).await?;
    if actor.id.is_none() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Sign in with username and password to manage two-factor authentication",
        ));
    }
    Ok(actor)
}
async fn reauthenticate(
    s: &AppState,
    h: &HeaderMap,
    password: &str,
) -> std::result::Result<auth::Principal, (StatusCode, Json<Value>)> {
    let actor = account(s, h).await?;
    let id = actor.id.unwrap();
    sqlx::query("INSERT INTO user_totp(user_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    let locked: bool = sqlx::query_scalar(
        "SELECT COALESCE(locked_until>NOW(),false) FROM user_totp WHERE user_id=$1",
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(db_error)?;
    if locked {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many verification attempts; retry in 15 minutes",
        ));
    }
    let encoded: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&s.db)
        .await
        .map_err(db_error)?;
    if password.len() > 512 || !auth::verify(password.to_owned(), encoded).await {
        failed(&s.db, id).await.map_err(db_error)?;
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "Current password is incorrect",
        ));
    }
    Ok(actor)
}
async fn revoke_other_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    h: &HeaderMap,
) -> sqlx::Result<()> {
    let current = hash(auth::session_token(h).unwrap_or(""));
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
        .bind(id)
        .bind(current)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn state(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Value> {
    let actor = account(&s, &h).await?;
    let id = actor.id.unwrap();
    let enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_totp WHERE user_id=$1 AND secret IS NOT NULL)",
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(db_error)?;
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM totp_recovery_codes WHERE user_id=$1")
            .bind(id)
            .fetch_one(&s.db)
            .await
            .map_err(db_error)?;
    Ok(Json(
        json!({"enabled":enabled,"recovery_codes_remaining":remaining}),
    ))
}
pub async fn setup(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PasswordInput>,
) -> ApiResult<Value> {
    let actor = reauthenticate(&s, &h, &input.password).await?;
    let id = actor.id.unwrap();
    let secret = Secret::generate().to_base32();
    let auth = authenticator(&secret, &actor.username).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to prepare authenticator",
        )
    })?;
    let uri = auth.to_url().map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to prepare authenticator",
        )
    })?;
    let updated=sqlx::query("UPDATE user_totp SET pending_secret=$2,pending_expires_at=NOW()+INTERVAL '10 minutes' WHERE user_id=$1 AND secret IS NULL")
        .bind(id).bind(&secret).execute(&s.db).await.map_err(db_error)?;
    if updated.rows_affected() != 1 {
        return Err(err(
            StatusCode::CONFLICT,
            "Two-factor authentication is already enabled",
        ));
    }
    Ok(Json(
        json!({"secret":secret,"uri":uri,"expires_in":600,"algorithm":"SHA1","digits":6,"period":30}),
    ))
}
pub async fn enable(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<CodeInput>,
) -> ApiResult<Value> {
    let actor = account(&s, &h).await?;
    let id = actor.id.unwrap();
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let row:Option<(String,Option<DateTime<Utc>>)>=sqlx::query_as("SELECT pending_secret,locked_until FROM user_totp WHERE user_id=$1 AND secret IS NULL AND pending_secret IS NOT NULL AND pending_expires_at>NOW() FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let (secret, locked) = row.ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            "Enrollment expired; start setup again",
        )
    })?;
    if locked.is_some_and(|date| date > Utc::now()) {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many verification attempts; retry in 15 minutes",
        ));
    }
    let step = authenticator(&secret, &actor.username)
        .ok()
        .and_then(|auth| auth.check(input.code.trim(), Utc::now().timestamp().max(0) as u64));
    let Some(step) = step else {
        tx.rollback().await.map_err(db_error)?;
        failed(&s.db, id).await.map_err(db_error)?;
        return Err(err(StatusCode::BAD_REQUEST, "Invalid authenticator code"));
    };
    sqlx::query("UPDATE user_totp SET secret=pending_secret,pending_secret=NULL,pending_expires_at=NULL,enabled_at=NOW(),last_step=$2,failures=0,locked_until=NULL WHERE user_id=$1")
        .bind(id).bind(step as i64).execute(&mut *tx).await.map_err(db_error)?;
    sqlx::query("DELETE FROM totp_recovery_codes WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let mut codes = Vec::new();
    for _ in 0..10 {
        let code = Uuid::new_v4().simple().to_string();
        sqlx::query("INSERT INTO totp_recovery_codes(user_id,code_hash) VALUES($1,$2)")
            .bind(id)
            .bind(recovery_hash(&code))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        codes.push(code);
    }
    revoke_other_sessions(&mut tx, id, &h)
        .await
        .map_err(db_error)?;
    sqlx::query("UPDATE sessions SET mfa_verified=true WHERE user_id=$1 AND token_hash=$2")
        .bind(id)
        .bind(hash(auth::session_token(&h).unwrap_or("")))
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"enabled":true,"recovery_codes":codes})))
}
pub async fn disable(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PasswordInput>,
) -> ApiResult<Value> {
    let actor = reauthenticate(&s, &h, &input.password).await?;
    let id = actor.id.unwrap();
    let enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_totp WHERE user_id=$1 AND secret IS NOT NULL)",
    )
    .bind(id)
    .fetch_one(&s.db)
    .await
    .map_err(db_error)?;
    if !enabled
        || !consume(&s.db, id, input.code.trim())
            .await
            .map_err(|_| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unable to verify authenticator",
                )
            })?
            .0
    {
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "Invalid authenticator or recovery code",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("DELETE FROM user_totp WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM totp_recovery_codes WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    revoke_other_sessions(&mut tx, id, &h)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"enabled":false})))
}
pub async fn recover(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let actor = auth::require_admin(&h, &s).await?;
    if actor.auth_method != "token" {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Administrator API token required for lost-authenticator recovery",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("DELETE FROM user_totp WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM totp_recovery_codes WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc6238_vectors_and_one_step_matching() {
        let secret = Secret::new_stack(*b"12345678901234567890").to_base32();
        let auth = authenticator(&secret, "test").unwrap();
        assert_eq!(auth.generate(59).to_string(), "287082");
        assert!(auth.check("287082", 59).is_some());
        assert!(auth.check("287082", 119).is_none());
        assert!(authenticator("invalid!", "test").is_err());
    }
}
