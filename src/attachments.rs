//! Bounded multipart ingestion, durable private files and acknowledged delivery steps.
use crate::*;
use axum::{extract::Multipart, response::IntoResponse};

pub const MAX_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_FILES: usize = 5;
pub const MAX_REQUEST: usize = 11 * 1024 * 1024;

#[derive(Clone, Serialize, FromRow)]
pub struct File {
    pub name: String,
    pub content_type: String,
    pub sha256: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}
impl File {
    fn new(name: String, content_type: String, data: Vec<u8>) -> Result<Self> {
        if name.is_empty()
            || name.len() > 240
            || name == "."
            || name == ".."
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        {
            return Err(anyhow!(
                "Use a filename of 1–240 bytes without paths or control characters"
            ));
        }
        if content_type.len() > 200
            || reqwest::multipart::Part::bytes(Vec::new())
                .mime_str(&content_type)
                .is_err()
        {
            return Err(anyhow!("Invalid attachment content type"));
        }
        Ok(Self {
            name,
            content_type,
            sha256: hex::encode(Sha256::digest(&data)),
            data,
        })
    }
}
pub async fn parse(
    mut multipart: Multipart,
) -> std::result::Result<(Value, Vec<File>), (StatusCode, Json<Value>)> {
    let mut payload = None;
    let mut files = Vec::new();
    let mut total = 0;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| err(StatusCode::BAD_REQUEST, "Invalid multipart upload"))?
    {
        let field_name = field.name().unwrap_or("").to_owned();
        let filename = field.file_name().map(str::to_owned);
        let mime = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_owned();
        if field_name != "payload" && field_name != "files" {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Use payload and files multipart fields",
            ));
        }
        if field_name == "payload" && (payload.is_some() || filename.is_some()) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Provide one JSON payload field",
            ));
        }
        if field_name == "files" && (files.len() >= MAX_FILES || filename.is_none()) {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Provide 1–5 files with filenames",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "Invalid multipart upload"))?
        {
            if field_name == "files" {
                total += chunk.len();
            }
            let limit = if field_name == "payload" {
                64 * 1024
            } else {
                MAX_BYTES
            };
            if bytes.len() + chunk.len() > limit || total > MAX_BYTES {
                return Err(err(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Use at most 10 MiB of files and 64 KiB of JSON per request",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if field_name == "payload" {
            payload = Some(serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                err(
                    StatusCode::BAD_REQUEST,
                    "payload must contain a JSON object",
                )
            })?);
        } else {
            files.push(
                File::new(filename.unwrap(), mime, bytes)
                    .map_err(|e| err(StatusCode::BAD_REQUEST, e))?,
            );
        }
    }
    if files.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Include at least one files field",
        ));
    }
    Ok((payload.unwrap_or_else(|| json!({})), files))
}
pub fn supported(kind: &str, cfg: &Value) -> bool {
    kind == "telegram" || kind == "mattermost" && mattermost::is_bot(cfg)
}
pub async fn store(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: Uuid,
    files: &[File],
) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    for (position, file) in files.iter().enumerate() {
        let result=sqlx::query("INSERT INTO event_attachments(id,event_id,position,name,content_type,sha256,data) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(Uuid::new_v4()).bind(event).bind(position as i32).bind(&file.name).bind(&file.content_type).bind(&file.sha256).bind(&file.data).execute(&mut **tx).await;
        if let Err(error) = result {
            if error
                .as_database_error()
                .is_some_and(|e| e.code().as_deref() == Some("P0001"))
            {
                return Err(err(
                    StatusCode::INSUFFICIENT_STORAGE,
                    "Attachment storage is full; retry after retained history expires",
                ));
            }
            return Err(db_error(error));
        }
    }
    Ok(())
}
pub async fn metadata(db: &PgPool, event: Uuid) -> sqlx::Result<Vec<Value>> {
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('id',id,'name',name,'content_type',content_type,'size',octet_length(data),'sha256',sha256) FROM event_attachments WHERE event_id=$1 ORDER BY position")
        .bind(event).fetch_all(db).await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}
pub async fn for_delivery(db: &PgPool, delivery: Uuid) -> sqlx::Result<Vec<File>> {
    sqlx::query_as("SELECT a.name,a.content_type,a.sha256,a.data FROM event_attachments a JOIN deliveries d ON d.event_id=a.event_id WHERE d.id=$1 ORDER BY a.position")
        .bind(delivery).fetch_all(db).await
}
pub async fn download(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((event, id)): Path<(Uuid, Uuid)>,
) -> std::result::Result<axum::response::Response, (StatusCode, Json<Value>)> {
    auth::authenticated(&h, &s).await?;
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT name,data FROM event_attachments WHERE event_id=$1 AND id=$2")
            .bind(event)
            .bind(id)
            .fetch_optional(&s.db)
            .await
            .map_err(db_error)?;
    let (name, data) = row.ok_or_else(|| err(StatusCode::NOT_FOUND, "Attachment not found"))?;
    let encoded = name
        .as_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let mut response = data.into_response();
    response.headers_mut().insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        "content-disposition",
        axum::http::HeaderValue::from_str(&format!(
            "attachment; filename=\"download\"; filename*=UTF-8''{encoded}"
        ))
        .map_err(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to encode filename",
            )
        })?,
    );
    Ok(response)
}

pub struct Delivery<'a> {
    pub db: &'a PgPool,
    pub client: &'a reqwest::Client,
    pub cfg: &'a Value,
    pub files: &'a [File],
    pub id: Uuid,
    pub fingerprint: String,
}
impl Delivery<'_> {
    pub async fn state(&self, step: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT remote_id FROM attachment_delivery_steps WHERE delivery_id=$1 AND fingerprint=$2 AND step=$3")
            .bind(self.id).bind(&self.fingerprint).bind(step).fetch_optional(self.db).await?)
    }
    pub async fn record(&self, step: &str, remote: &str) -> Result<()> {
        sqlx::query("INSERT INTO attachment_delivery_steps(delivery_id,fingerprint,step,remote_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(self.id).bind(&self.fingerprint).bind(step).bind(remote).execute(self.db).await?;
        Ok(())
    }
}
pub async fn deliver(
    s: &AppState,
    c: &Channel,
    payload: &Value,
    priority: i32,
    id: Uuid,
    files: &[File],
) -> Result<()> {
    let title = message::event_title(payload);
    let body = message::event_message(payload);
    if files.is_empty() {
        return send(&s.client, c, title, body, priority, s.allow_http, id).await;
    }
    if !supported(&c.kind, &c.config) {
        return Err(retry_policy::message(
            "Attachments require Telegram or Mattermost bot/API mode",
        ));
    }
    valid_config(&c.kind, &c.config, s.allow_http).map_err(|_| {
        retry_policy::permanent("Invalid channel configuration; update channel settings")
    })?;
    network::validate(&c.config).await?;
    let rendered = message::render(&c.kind, &c.config, title, body);
    if c.kind == "telegram" && matches!(proxy::mode(&c.config), "mtproto" | "webproxy") {
        return telegram::send_files(&c.config, &rendered.text, id, rendered.html, files).await;
    }
    let client = proxy::client(&s.client, &c.config)?;
    let context = Delivery {
        db: &s.db,
        client: &client,
        cfg: &c.config,
        files,
        id,
        fingerprint: hash(&json!({"kind":c.kind,"config":c.config}).to_string()),
    };
    tokio::time::timeout(Duration::from_secs(60), async {
        if c.kind == "mattermost" {
            return mattermost::send_files(&context, &rendered.text).await;
        }
        if !rendered.text.is_empty() && context.state("text").await?.is_none() {
            send(&s.client, c, title, body, priority, s.allow_http, id).await?;
            context.record("text", "sent").await?;
        }
        if context.state("documents").await?.is_none() {
            let token = c.config["bot_token"].as_str().unwrap();
            if !token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-'))
            {
                return Err(retry_policy::permanent("Invalid bot token"));
            }
            let method = if files.len() == 1 {
                "sendDocument"
            } else {
                "sendMediaGroup"
            };
            let request = telegram_request(
                &client,
                &format!("https://api.telegram.org/bot{token}/{method}"),
                &c.config,
                files,
            )?;
            let response = request.send().await.map_err(|e| anyhow!(e.without_url()))?;
            let response = retry_policy::response(response).await?;
            let result: Value = response
                .json()
                .await
                .map_err(|_| anyhow!("Telegram returned an invalid document response"))?;
            if result["ok"] != true {
                return Err(retry_policy::message(
                    "Telegram did not accept the document",
                ));
            }
            context.record("documents", "sent").await?;
        }
        Ok(())
    })
    .await
    .map_err(|_| anyhow!("Attachment delivery timed out"))?
}
pub fn telegram_request(
    client: &reqwest::Client,
    url: &str,
    cfg: &Value,
    files: &[File],
) -> Result<reqwest::RequestBuilder> {
    use reqwest::multipart::{Form, Part};
    let mut form = Form::new().text("chat_id", cfg["chat_id"].as_str().unwrap_or("").to_owned());
    if let Some(topic) = telegram_thread(cfg)? {
        form = form.text("message_thread_id", topic.to_string());
    }
    let mut media = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let field = if files.len() == 1 {
            "document".to_owned()
        } else {
            format!("file{index}")
        };
        let part = Part::bytes(file.data.clone())
            .file_name(file.name.clone())
            .mime_str(&file.content_type)?;
        form = form.part(field.clone(), part);
        media.push(json!({"type":"document","media":format!("attach://{field}")}));
    }
    if files.len() > 1 {
        form = form.text("media", serde_json::to_string(&media)?);
    }
    Ok(client
        .post(url)
        .timeout(Duration::from_secs(45))
        .multipart(form))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_media_types_are_bounded_without_interpreting_paths() {
        for name in ["../report.xlsx", "a/b", "a\\b", "bad\nname", ".", ""] {
            assert!(
                File::new(name.into(), "application/octet-stream".into(), vec![0, 255]).is_err()
            );
        }
        assert!(File::new(
            "Отчёт.xlsx".into(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
            vec![0, 255]
        )
        .is_ok());
        assert!(File::new(
            "report.xlsx".into(),
            "text/plain\r\nInjected: yes".into(),
            vec![]
        )
        .is_err());
    }
    #[tokio::test]
    async fn bot_api_uploads_binary_files_with_topics_and_media_references() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for count in [1, 2] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
                        let size: usize = header
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse().ok())
                            })
                            .unwrap();
                        if data.len() >= end + 4 + size {
                            break;
                        }
                    }
                }
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}").await.unwrap();
                data
            });
            let file = File::new(
                "report.xlsx".into(),
                "application/octet-stream".into(),
                vec![0, 255, 128, 1],
            )
            .unwrap();
            let files = vec![file; count];
            let response = telegram_request(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &format!("http://{address}/upload"),
                &json!({"chat_id":"123","message_thread_id":"42"}),
                &files,
            )
            .unwrap()
            .send()
            .await
            .unwrap();
            assert!(response.status().is_success());
            let data = server.await.unwrap();
            assert!(data.windows(4).any(|w| w == [0, 255, 128, 1]));
            let body = String::from_utf8_lossy(&data);
            assert!(body.contains("message_thread_id") && body.contains("report.xlsx"));
            if count == 2 {
                assert!(body.contains("attach://file0") && body.contains("attach://file1"));
            }
        }
    }
}
