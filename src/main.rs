mod attachments;
mod channel_api;
mod channel_config;
mod channel_health;
mod client_ip;
mod config_transfer;
mod delivery_api;
mod delivery_policy;
mod event_groups;
mod lease;
mod retry_policy;
mod route_api;
mod totp;
use channel_api::*;
use delivery_api::*;
use route_api::*;
mod alerts;
mod auth;
mod bulk_retry;
mod database;
mod delivery_groups;
mod diagnostics;
mod mattermost;
mod message;
mod network;
mod operations;
mod payload_rules;
mod preferences;
mod providers;
mod proxy;
mod routing;
mod runtime;
mod table;
mod telegram;
mod templates;
#[cfg(test)]
mod tests;
mod webhook_body;
use anyhow::{anyhow, Result};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rand::{distr::Alphanumeric, RngExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::{env, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    admin: Arc<String>,
    client: reqwest::Client,
    allow_http: bool,
    secure_cookies: bool,
    dummy_password_hash: Arc<String>,
    retry_seconds: i32,
    channel_concurrency: i64,
    proxy_concurrency: i64,
}
type ApiResult<T> = std::result::Result<Json<T>, (StatusCode, Json<Value>)>;
fn err(code: StatusCode, msg: impl ToString) -> (StatusCode, Json<Value>) {
    (code, Json(json!({"error":msg.to_string()})))
}
fn unauthorized() -> (StatusCode, Json<Value>) {
    err(
        StatusCode::UNAUTHORIZED,
        "Invalid credentials or expired session",
    )
}
fn db_error(e: sqlx::Error) -> (StatusCode, Json<Value>) {
    tracing::error!(error=%e, "database error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
}
fn hash(v: &str) -> String {
    hex::encode(Sha256::digest(v.as_bytes()))
}
fn key() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

fn monitoring_period(period: &str) -> Option<chrono::Duration> {
    Some(match period {
        "5m" => chrono::Duration::minutes(5),
        "15m" => chrono::Duration::minutes(15),
        "30m" => chrono::Duration::minutes(30),
        "1h" => chrono::Duration::hours(1),
        "24h" => chrono::Duration::days(1),
        "7d" => chrono::Duration::days(7),
        "30d" => chrono::Duration::days(30),
        _ => return None,
    })
}
#[derive(Deserialize)]
struct StatsQuery {
    period: Option<String>,
}
async fn stats(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(query): Query<StatsQuery>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let period = query.period.as_deref().unwrap_or("30d");
    let duration = monitoring_period(period)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Invalid monitoring period"))?;
    let to = Utc::now();
    let from = to - duration;
    let (total,sent,failed,pending,retrying):(i64,i64,i64,i64,i64)=sqlx::query_as("SELECT COUNT(*),COUNT(*) FILTER (WHERE status='sent'),COUNT(*) FILTER (WHERE status='failed'),COUNT(*) FILTER (WHERE status IN ('queued','processing')),COUNT(*) FILTER (WHERE status='queued' AND error IS NOT NULL) FROM deliveries WHERE created_at>=$1 AND created_at<=$2").bind(from).bind(to).fetch_one(&s.db).await.map_err(db_error)?;
    let (total30,sent30,failed30):(i64,i64,i64)=sqlx::query_as("SELECT COUNT(*),COUNT(*) FILTER (WHERE status='sent'),COUNT(*) FILTER (WHERE status='failed') FROM deliveries WHERE created_at>=NOW()-INTERVAL '30 days'").fetch_one(&s.db).await.map_err(db_error)?;
    Ok(Json(
        json!({"period":period,"from":from,"to":to,"total":total,"sent":sent,"failed":failed,"pending":pending,"retrying":retrying,"last_30_days":total30,"sent_30_days":sent30,"failed_30_days":failed30}),
    ))
}
async fn tags(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Vec<String>> {
    auth::authenticated(&h, &s).await?;
    let tags:Vec<(String,)>=sqlx::query_as("SELECT tag FROM (SELECT required_tag AS tag FROM routes WHERE required_tag IS NOT NULL AND required_tag<>'' UNION SELECT tag FROM event_tags) all_tags ORDER BY tag LIMIT 500").fetch_all(&s.db).await.map_err(db_error)?;
    Ok(Json(tags.into_iter().map(|(tag,)| tag).collect()))
}
async fn health(State(s): State<AppState>) -> ApiResult<Value> {
    match tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(&s.db),
    )
    .await
    {
        Ok(Ok(_)) => Ok(Json(
            json!({"status":"ok","name":"Relayix","version":env!("CARGO_PKG_VERSION"),"stage":"beta"}),
        )),
        _ => Err(err(StatusCode::SERVICE_UNAVAILABLE, "Database unavailable")),
    }
}

async fn no_store(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "relayix=info".into()),
        )
        .init();
    let admin_token = env::var("ADMIN_TOKEN").unwrap_or_default();
    if !admin_token.is_empty() && (admin_token.len() < 32 || admin_token.starts_with("CHANGE_ME")) {
        return Err(anyhow!("Set a random ADMIN_TOKEN of at least 32 chars"));
    }
    let port = env::var("PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| anyhow!("PORT must be between 1 and 65535"))?;
    for name in ["ALLOW_INSECURE_HTTP", "COOKIE_SECURE"] {
        if env::var(name).is_ok_and(|v| !matches!(v.as_str(), "" | "true" | "false")) {
            return Err(anyhow!("{name} must be true or false"));
        }
    }
    if env::var("RETRY_BASE_SECONDS").is_ok_and(|v| {
        v.parse::<i32>()
            .map(|n| !(1..=900).contains(&n))
            .unwrap_or(true)
    }) {
        return Err(anyhow!("RETRY_BASE_SECONDS must be between 1 and 900"));
    }
    let api_id = env::var("TELEGRAM_API_ID").unwrap_or_default();
    let api_hash = env::var("TELEGRAM_API_HASH").unwrap_or_default();
    if !api_id.is_empty() || !api_hash.is_empty() {
        if api_id.parse::<i32>().ok().filter(|id| *id > 0).is_none() {
            return Err(anyhow!("TELEGRAM_API_ID must be a positive integer; set it together with TELEGRAM_API_HASH"));
        }
        if api_hash.len() != 32 || !api_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(anyhow!(
                "TELEGRAM_API_HASH must contain 32 hexadecimal characters"
            ));
        }
    } else {
        tracing::info!("Telegram MTProto/WEB unavailable until TELEGRAM_API_ID and TELEGRAM_API_HASH are configured; direct/HTTP Bot API remains available");
    }
    let trusted_proxies = Arc::new(client_ip::TrustedProxies::parse(
        &env::var("TRUSTED_PROXIES").unwrap_or_default(),
    )?);
    if trusted_proxies.is_empty() {
        tracing::warn!("TRUSTED_PROXIES is empty: forwarding headers are ignored; clients behind a reverse proxy share request quotas and IP/username login lockouts");
    }
    let webhook_limit = runtime::setting("WEBHOOK_RATE_LIMIT_PER_MINUTE", 600, 1, 1_000_000)?;
    let policy = network::policy()?;
    let worker_count = runtime::setting("DELIVERY_WORKERS", 4, 1, 32)?;
    let proxy_concurrency = runtime::setting("DELIVERY_PROXY_CONCURRENCY", 2, 1, 32)?;
    let channel_concurrency = runtime::setting("DELIVERY_CHANNEL_CONCURRENCY", 2, 1, 32)?;
    let retention_days = runtime::setting("EVENT_RETENTION_DAYS", 30, 0, 3650)?;
    let db = database::connect().await?;
    sqlx::migrate!().run(&db).await.map_err(|_|anyhow!("Database migration failed; check schema permissions, migration history and database server logs"))?;
    auth::bootstrap(&db).await?;
    let dummy_password_hash = Arc::new(auth::dummy_hash().await?);
    let client = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(policy))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let s = AppState {
        db,
        admin: Arc::new(admin_token),
        client,
        allow_http: env::var("ALLOW_INSECURE_HTTP").unwrap_or_default() == "true",
        secure_cookies: env::var("COOKIE_SECURE").unwrap_or_default() == "true",
        dummy_password_hash,
        channel_concurrency: i64::from(channel_concurrency),
        proxy_concurrency: i64::from(proxy_concurrency),
        retry_seconds: env::var("RETRY_BASE_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(15)
            .clamp(1, 900),
    };
    let mut workers = tokio::task::JoinSet::new();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    for _ in 0..worker_count {
        workers.spawn(operations::worker(s.clone(), stop_rx.clone()));
    }
    workers.spawn(alerts::worker(s.clone(), stop_rx.clone()));
    workers.spawn(runtime::maintenance(s.db.clone(), retention_days, stop_rx));
    let api = Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/totp", get(totp::state))
        .route("/auth/totp/setup", post(totp::setup))
        .route("/auth/totp/enable", post(totp::enable))
        .route("/auth/totp/disable", post(totp::disable))
        .route("/users/{id}/totp/reset", post(totp::recover))
        .route("/auth/preferences", axum::routing::put(preferences::update))
        .route(
            "/auth/delivery-alerts",
            get(alerts::personal).put(alerts::save_personal),
        )
        .route("/auth/delivery-alerts/test", post(alerts::test_personal))
        .route(
            "/auth/delivery-alerts/history",
            get(alerts::personal_history),
        )
        .route(
            "/auth/delivery-alerts/history/{id}/retry",
            post(alerts::retry_personal),
        )
        .route("/tags", get(tags))
        .route("/auth/profile", axum::routing::put(auth::profile))
        .route("/users", get(auth::users).post(auth::create_user))
        .route(
            "/users/{id}",
            axum::routing::put(auth::update_user).delete(auth::delete_user),
        )
        .route("/alert-policies", get(alerts::list).post(alerts::create))
        .route(
            "/alert-policies/{id}",
            axum::routing::put(alerts::update).delete(alerts::delete),
        )
        .route("/alert-policies/{id}/test", post(alerts::test))
        .route("/alert-deliveries", get(alerts::history))
        .route("/alert-deliveries/{id}/retry", post(alerts::retry))
        .route("/config/export", get(config_transfer::export))
        .route(
            "/config/import",
            post(config_transfer::import).layer(DefaultBodyLimit::max(config_transfer::MAX_BYTES)),
        )
        .route("/channels", get(channels).post(create_channel))
        .route("/channels/diagnose", post(diagnostics::draft))
        .route("/channels/preview", post(message::preview_channel))
        .route("/stats", get(stats))
        .route(
            "/channels/{id}",
            axum::routing::put(update_channel).delete(delete_channel),
        )
        .route("/channels/{id}/test", post(test_channel))
        .route("/channels/{id}/pause", post(pause_channel))
        .route("/channels/{id}/resume", post(resume_channel))
        .route("/channels/{id}/backlog", get(channel_backlog))
        .route("/channels/{id}/diagnose", post(diagnostics::channel))
        .route("/routes", get(routes).post(create_route))
        .route(
            "/routes/{id}",
            axum::routing::put(update_route).delete(delete_route),
        )
        .route("/routes/{id}/rotate", post(rotate_route))
        .route("/routes/{id}/test", post(routing::test_route))
        .route("/routes/{id}/preview", post(routing::preview_route))
        .route("/events", get(operations::events))
        .route("/events/{id}", get(operations::event_details))
        .route("/events/{id}/attachments/{file_id}",get(attachments::download))
        .route("/templates", get(templates::list).post(templates::create))
        .route("/templates/preview", post(templates::preview))
        .route(
            "/templates/{id}",
            axum::routing::put(templates::update).delete(templates::delete),
        )
        .route("/templates/{id}/versions", get(templates::versions))
        .route(
            "/templates/{id}/versions/{version}/restore",
            post(templates::restore),
        )
        .route("/deliveries/{id}/attempts", get(operations::attempts))
        .route("/deliveries", get(deliveries))
        .route("/deliveries/{id}/retry", post(retry))
        .route("/deliveries/retry-batch/preview", post(bulk_retry::preview))
        .route("/deliveries/retry-batch/{id}", post(bulk_retry::execute))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"Unknown API endpoint"})),
            )
        })
        .layer(axum::middleware::from_fn(no_store));
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/h/{slug}", post(routing::ingest))
        .route("/h/{slug}/files",post(routing::ingest_files).layer(DefaultBodyLimit::max(attachments::MAX_REQUEST)))
        .nest("/api", api)
        .fallback_service(
            ServeDir::new("web/dist").not_found_service(ServeFile::new("web/dist/index.html")),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            webhook_limit,
            runtime::rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            trusted_proxies,
            client_ip::resolve,
        ))
        .layer(axum::middleware::from_fn(runtime::security_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(s);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!(%port,"Relayix started");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        runtime::shutdown().await;
        let _ = stop_tx.send(true);
    })
    .await?;
    // Finish claimed deliveries before exiting; leases recover any interrupted work.
    if tokio::time::timeout(Duration::from_secs(65), async {
        while workers.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        workers.abort_all();
    }
    Ok(())
}
