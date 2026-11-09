use crate::*;
use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};
use axum::http::{header, HeaderValue};
use tokio::sync::Semaphore;

static PASSWORD_WORK: Semaphore = Semaphore::const_new(4);

#[derive(Clone, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing)]
    pub preferences: Value,
}

#[derive(Serialize)]
pub struct Principal {
    pub id: Option<Uuid>,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub auth_method: &'static str,
    pub preferences: preferences::Preferences,
}
impl From<User> for Principal {
    fn from(u: User) -> Self {
        Self {
            id: Some(u.id),
            username: u.username,
            display_name: u.display_name,
            role: u.role,
            auth_method: "password",
            preferences: preferences::Preferences::from_value(u.preferences),
        }
    }
}

pub fn password_valid(password: &str, username: &str) -> Result<()> {
    let count = password.chars().count();
    if !(12..=128).contains(&count)
        || password.len() > 512
        || !password.chars().any(|c| c.is_ascii_uppercase())
        || !password.chars().any(|c| c.is_ascii_lowercase())
        || !password.chars().any(|c| c.is_ascii_digit())
        || !password
            .chars()
            .any(|c| !c.is_alphanumeric() && !c.is_whitespace())
        || password.chars().any(char::is_control)
    {
        return Err(anyhow!("Password must have 12–128 characters, uppercase and lowercase letters, a number and a symbol"));
    }
    let lower = password.to_lowercase();
    if (!username.is_empty() && lower.contains(&username.to_lowercase()))
        || ["password", "qwerty", "123456", "letmein"]
            .iter()
            .any(|word| lower.contains(word))
    {
        return Err(anyhow!(
            "Password must not contain the username or a common password sequence"
        ));
    }
    Ok(())
}

fn username_valid(username: &str) -> bool {
    (3..=64).contains(&username.len())
        && username
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}

async fn password_hash(password: String) -> Result<String> {
    let permit = PASSWORD_WORK.acquire().await?;
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|_| anyhow!("Password hashing failed"))
    })
    .await??;
    drop(permit);
    Ok(hash)
}

pub(crate) async fn verify(password: String, encoded: String) -> bool {
    let Ok(permit) = PASSWORD_WORK.acquire().await else {
        return false;
    };
    let verified = verify_password(password, encoded).await;
    drop(permit);
    verified
}

async fn verify_password(password: String, encoded: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&encoded)
            .map(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

pub(crate) fn session_token(h: &HeaderMap) -> Option<&str> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(name, _)| *name == "relayix-session")
        .map(|(_, value)| value)
}

pub async fn authenticated(
    h: &HeaderMap,
    s: &AppState,
) -> std::result::Result<Principal, (StatusCode, Json<Value>)> {
    let supplied = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(token) = supplied {
        if !s.admin.is_empty()
            && token.len() == s.admin.len()
            && bool::from(token.as_bytes().ct_eq(s.admin.as_bytes()))
        {
            let (prefs,): (Value,) =
                sqlx::query_as("SELECT preferences FROM api_preferences WHERE id=true")
                    .fetch_one(&s.db)
                    .await
                    .map_err(db_error)?;
            return Ok(Principal {
                id: None,
                username: "api-admin".into(),
                display_name: "API administrator".into(),
                role: "admin".into(),
                auth_method: "token",
                preferences: preferences::Preferences::from_value(prefs),
            });
        }
        return Err(unauthorized());
    }
    // Cookie authentication uses a custom header to prevent cross-site requests, including login CSRF.
    if h.get("x-relay-client").and_then(|v| v.to_str().ok()) != Some("web") {
        return Err(unauthorized());
    }
    let token = session_token(h).ok_or_else(unauthorized)?;
    let user=sqlx::query_as::<_,User>("SELECT u.id,u.username,u.display_name,u.role,u.enabled,u.created_at,u.preferences FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.token_hash=$1 AND s.expires_at>NOW() AND u.enabled=true AND (s.mfa_verified OR NOT EXISTS(SELECT 1 FROM user_totp t WHERE t.user_id=u.id AND t.secret IS NOT NULL))")
        .bind(hash(token)).fetch_optional(&s.db).await.map_err(db_error)?.ok_or_else(unauthorized)?;
    Ok(user.into())
}

pub async fn require_admin(
    h: &HeaderMap,
    s: &AppState,
) -> std::result::Result<Principal, (StatusCode, Json<Value>)> {
    let user = authenticated(h, s).await?;
    if user.role != "admin" {
        return Err(err(StatusCode::FORBIDDEN, "Administrator role required"));
    }
    Ok(user)
}

pub async fn bootstrap(db: &PgPool) -> Result<()> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await?;
    if count.0 > 0 {
        return Ok(());
    }
    let username = env::var("ADMIN_USERNAME")
        .unwrap_or_else(|_| "admin".into())
        .to_lowercase();
    let Some(password) = env::var("ADMIN_PASSWORD").ok().filter(|v| !v.is_empty()) else {
        tracing::warn!("No user accounts exist. Set ADMIN_USERNAME and ADMIN_PASSWORD or create an administrator using the admin API token");
        return Ok(());
    };
    if !username_valid(&username) {
        return Err(anyhow!("Invalid bootstrap ADMIN_USERNAME"));
    }
    password_valid(&password, &username)?;
    let encoded = password_hash(password).await?;
    sqlx::query("INSERT INTO users(id,username,display_name,password_hash,role) VALUES($1,$2,'Administrator',$3,'admin') ON CONFLICT(username) DO NOTHING")
        .bind(Uuid::new_v4()).bind(username).bind(encoded).execute(db).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct Login {
    #[serde(default)]
    code: String,
    username: String,
    password: String,
}
pub async fn login(
    State(s): State<AppState>,
    h: HeaderMap,
    axum::Extension(client): axum::Extension<client_ip::ClientIp>,
    Json(input): Json<Login>,
) -> std::result::Result<(HeaderMap, Json<Principal>), (StatusCode, Json<Value>)> {
    if h.get("x-relay-client").and_then(|v| v.to_str().ok()) != Some("web") {
        return Err(err(StatusCode::FORBIDDEN, "Missing client header"));
    }
    let username = input.username.trim().to_lowercase();
    if username.len() > 64 || input.password.len() > 512 {
        return Err(unauthorized());
    }
    let attempt_key = hash(&format!("{}:{username}", client.key()));
    let account_key = hash(&format!("account:{username}"));
    sqlx::query("DELETE FROM login_attempts WHERE window_started_at<NOW()-INTERVAL '15 minutes'")
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    // Check failure budgets after joining the bounded password-work queue. A
    // distributed burst must not pre-admit unlimited checks before failures land.
    let password_permit = PASSWORD_WORK.acquire().await.map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Password verification unavailable",
        )
    })?;
    let failures: Option<(i32,)> =
        sqlx::query_as("SELECT failures FROM login_attempts WHERE username=$1")
            .bind(&attempt_key)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    if failures.map(|r| r.0 >= 10).unwrap_or(false) {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts. Try again in 15 minutes",
        ));
    }
    // After 30 distributed failures, admit at most one password verification every
    // two seconds. Rejections do not extend the gate or the 15-minute failure window.
    // Atomic UPDATE also coordinates competing processes; correct passwords may
    // still succeed in the next short slot rather than being locked out for 15 minutes.
    let admitted: bool = sqlx::query_scalar(
        "WITH admitted AS (UPDATE login_attempts SET next_allowed_at=NOW()+INTERVAL '2 seconds' WHERE username=$1 AND failures>=30 AND next_allowed_at<=NOW() RETURNING 1) SELECT EXISTS(SELECT 1 FROM admitted) OR NOT EXISTS(SELECT 1 FROM login_attempts WHERE username=$1 AND failures>=30)"
    ).bind(&account_key).fetch_one(&s.db).await.map_err(db_error)?;
    if !admitted {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Login temporarily slowed; retry in 2 seconds",
        ));
    }
    let row: Option<(Uuid, String, bool)> =
        sqlx::query_as("SELECT id,password_hash,enabled FROM users WHERE username=$1")
            .bind(&username)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    // Perform the same expensive hash work for unknown users.
    let encoded = match &row {
        Some((_, encoded, _)) => encoded.clone(),
        None => s.dummy_password_hash.as_ref().clone(),
    };
    let password_valid = verify_password(input.password, encoded).await
        && row.as_ref().map(|r| r.2).unwrap_or(false);
    let (valid,mfa_verified) = if password_valid {
        crate::totp::consume(&s.db,row.as_ref().unwrap().0,input.code.trim()).await
            .map_err(|_|err(StatusCode::INTERNAL_SERVER_ERROR,"Unable to verify two-factor authentication"))?
    } else {(false,false)};
    if !valid {
        sqlx::query("INSERT INTO login_attempts(username,failures) VALUES($1,1),($2,1) ON CONFLICT(username) DO UPDATE SET failures=login_attempts.failures+1")
            .bind(&attempt_key).bind(&account_key).execute(&s.db).await.map_err(db_error)?;
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "Invalid username, password or verification code",
        ));
    }
    let id = row.unwrap().0;
    sqlx::query("DELETE FROM login_attempts WHERE username=$1 OR username=$2")
        .bind(&attempt_key)
        .bind(&account_key)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    drop(password_permit);
    sqlx::query("DELETE FROM sessions WHERE expires_at<NOW()")
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    let token = key();
    sqlx::query(
        "INSERT INTO sessions(token_hash,user_id,expires_at,mfa_verified) VALUES($1,$2,NOW()+INTERVAL '7 days',$3)",
    )
    .bind(hash(&token))
    .bind(id)
    .bind(mfa_verified)
    .execute(&s.db)
    .await
    .map_err(db_error)?;
    let user=sqlx::query_as::<_,User>("SELECT id,username,display_name,role,enabled,created_at,preferences FROM users WHERE id=$1").bind(id).fetch_one(&s.db).await.map_err(db_error)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "relayix-session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800{}",
            if s.secure_cookies { "; Secure" } else { "" }
        ))
        .unwrap(),
    );
    Ok((headers, Json(user.into())))
}

pub async fn dummy_hash() -> Result<String> {
    password_hash(key()).await
}
pub async fn me(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Principal> {
    Ok(Json(authenticated(&h, &s).await?))
}
pub async fn logout(
    State(s): State<AppState>,
    h: HeaderMap,
) -> std::result::Result<(HeaderMap, Json<Value>), (StatusCode, Json<Value>)> {
    authenticated(&h, &s).await?;
    if let Some(token) = session_token(&h) {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
            .bind(hash(token))
            .execute(&s.db)
            .await
            .map_err(db_error)?;
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("relayix-session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
    );
    Ok((headers, Json(json!({"ok":true}))))
}

pub async fn users(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Vec<User>> {
    require_admin(&h, &s).await?;
    Ok(Json(sqlx::query_as::<_,User>("SELECT id,username,display_name,role,enabled,created_at,preferences FROM users ORDER BY created_at").fetch_all(&s.db).await.map_err(db_error)?))
}
#[derive(Deserialize)]
pub struct UserInput {
    username: String,
    display_name: String,
    role: String,
    enabled: Option<bool>,
    password: Option<String>,
}
fn valid_user(input: &UserInput) -> Result<()> {
    if !username_valid(&input.username) {
        return Err(anyhow!(
            "Username must be 3–64 lowercase letters, digits, dots, underscores or hyphens"
        ));
    }
    if input.display_name.trim().is_empty() || input.display_name.len() > 100 {
        return Err(anyhow!("Display name is required (maximum 100 characters)"));
    }
    if !matches!(input.role.as_str(), "admin" | "member") {
        return Err(anyhow!("Invalid role"));
    }
    Ok(())
}
pub async fn create_user(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<UserInput>,
) -> ApiResult<Value> {
    require_admin(&h, &s).await?;
    valid_user(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let password = input.password.as_deref().unwrap_or("");
    password_valid(password, &input.username).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let encoded = password_hash(password.to_owned())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Password hashing failed"))?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,username,display_name,password_hash,role,enabled) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(id).bind(input.username).bind(input.display_name.trim()).bind(encoded).bind(input.role).bind(input.enabled.unwrap_or(true))
        .execute(&s.db).await.map_err(|e|if e.as_database_error().and_then(|e|e.code()).as_deref()==Some("23505") {err(StatusCode::CONFLICT,"Username already exists")}else{db_error(e)})?;
    Ok(Json(json!({"id":id})))
}
pub async fn update_user(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UserInput>,
) -> ApiResult<Value> {
    let actor = require_admin(&h, &s).await?;
    valid_user(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let enabled = input.enabled.unwrap_or(true);
    if actor.id == Some(id) && (!enabled || input.role != "admin") {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "You cannot disable or demote your own account",
        ));
    }
    let encoded = if let Some(password) = input.password.filter(|p| !p.is_empty()) {
        password_valid(&password, &input.username).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
        Some(
            password_hash(password)
                .await
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Password hashing failed"))?,
        )
    } else {
        None
    };
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(914027)")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let old: Option<(String, bool)> = sqlx::query_as("SELECT role,enabled FROM users WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?;
    let old = old.ok_or_else(|| err(StatusCode::NOT_FOUND, "User not found"))?;
    if old.0 == "admin" && old.1 && (!enabled || input.role != "admin") {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE role='admin' AND enabled=true")
                .fetch_one(&mut *tx)
                .await
                .map_err(db_error)?;
        if count.0 <= 1 {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "At least one active administrator is required",
            ));
        }
    }
    sqlx::query("UPDATE users SET username=$2,display_name=$3,role=$4,enabled=$5,password_hash=COALESCE($6,password_hash) WHERE id=$1")
        .bind(id).bind(input.username).bind(input.display_name.trim()).bind(input.role).bind(enabled).bind(&encoded).execute(&mut *tx).await.map_err(db_error)?;
    if encoded.is_some() || !enabled {
        sqlx::query("DELETE FROM sessions WHERE user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
pub async fn delete_user(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    let actor = require_admin(&h, &s).await?;
    if actor.id == Some(id) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "You cannot delete your own account",
        ));
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(914027)")
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE role='admin' AND enabled=true AND id<>$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    if count.0 == 0 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "At least one active administrator is required",
        ));
    }
    let deleted = sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    if deleted.rows_affected() == 0 {
        return Err(err(StatusCode::NOT_FOUND, "User not found"));
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
pub struct ProfileInput {
    display_name: String,
    current_password: Option<String>,
    new_password: Option<String>,
}
pub async fn profile(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> ApiResult<Value> {
    let actor = authenticated(&h, &s).await?;
    let id = actor.id.ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            "Sign in with username and password to edit a personal profile",
        )
    })?;
    if input.display_name.trim().is_empty() || input.display_name.len() > 100 {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Display name is required (maximum 100 characters)",
        ));
    }
    let mut encoded = None;
    if let Some(password) = input.new_password.filter(|p| !p.is_empty()) {
        password_valid(&password, &actor.username).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
        let (old,): (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&s.db)
            .await
            .map_err(db_error)?;
        if !verify(input.current_password.unwrap_or_default(), old).await {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Current password is incorrect",
            ));
        }
        encoded = Some(
            password_hash(password)
                .await
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Password hashing failed"))?,
        );
    }
    let mut tx = s.db.begin().await.map_err(db_error)?;
    sqlx::query(
        "UPDATE users SET display_name=$2,password_hash=COALESCE($3,password_hash) WHERE id=$1",
    )
    .bind(id)
    .bind(input.display_name.trim())
    .bind(&encoded)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    if encoded.is_some() {
        // Keep this session, revoke every other session after a password change.
        sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
            .bind(id)
            .bind(hash(session_token(&h).unwrap_or("")))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn password_hashes_are_salted_and_verified() {
        let password = format!("A7!b{}", key());
        let first = password_hash(password.clone()).await.unwrap();
        let second = password_hash(password.clone()).await.unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("$argon2id$"));
        assert!(verify(password, first.clone()).await);
        assert!(!verify(key(), first).await);
    }
}
