//! Bounded substitution and an opt-in test status rule; no scripts or network access.
use crate::*;

pub const ICONS: &[&str] = &[
    "git-branch",
    "rocket",
    "flask-conical",
    "database",
    "activity",
    "bell",
    "shield",
    "package",
    "server",
    "wrench",
    "layers",
    "zap",
];
#[derive(Clone, Serialize, Deserialize, FromRow)]
pub struct Template {
    #[serde(default)]
    pub computed_status: bool,
    #[serde(default = "passed_field")]
    pub passed_field: String,
    #[serde(default = "failed_field")]
    pub failed_field: String,
    pub id: Uuid,
    pub name: String,
    pub title: String,
    pub body: String,
    pub link_label: String,
    pub link_url: String,
    pub link_failure_only: bool,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Clone, Deserialize)]
pub struct Input {
    #[serde(default)]
    pub computed_status: bool,
    #[serde(default = "passed_field")]
    pub passed_field: String,
    #[serde(default = "failed_field")]
    pub failed_field: String,
    pub name: String,
    #[serde(default)]
    pub title: String,
    pub body: String,
    #[serde(default = "label")]
    pub link_label: String,
    #[serde(default)]
    pub link_url: String,
    #[serde(default = "yes")]
    pub link_failure_only: bool,
}
fn passed_field() -> String {
    "passed".into()
}
fn failed_field() -> String {
    "failed".into()
}
fn label() -> String {
    "Runner log".into()
}
fn yes() -> bool {
    true
}

fn keys(text: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let following = &rest[start + 2..];
        let end = following
            .find("}}")
            .ok_or_else(|| anyhow!("Unclosed template variable"))?;
        let key = following[..end].trim();
        if key.is_empty()
            || key.len() > 128
            || !key.split('.').all(|part| {
                !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            })
        {
            return Err(anyhow!(
                "Use variables such as {{{{message}}}} or {{{{build.branch}}}}"
            ));
        }
        out.push(key.to_owned());
        rest = &following[end + 2..];
    }
    if out.len() > 100 {
        return Err(anyhow!("Use at most 100 variables per field"));
    }
    Ok(out)
}
fn validate(input: &Input) -> Result<()> {
    if !crate::payload_rules::path_valid(&input.passed_field)
        || !crate::payload_rules::path_valid(&input.failed_field)
        || input.passed_field == input.failed_field
        || [&input.passed_field, &input.failed_field]
            .iter()
            .any(|path| path.as_str() == "status" || path.starts_with("status."))
    {
        return Err(anyhow!("Use distinct counter paths, such as passed and failed; status is reserved for the result"));
    }

    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(anyhow!("Template name must contain 1–100 characters"));
    }
    if input.title.len() > 1024
        || input.body.trim().is_empty()
        || input.body.len() > 10000
        || input.link_url.len() > 2048
        || input.link_label.len() > 100
    {
        return Err(anyhow!("Template fields exceed their limits"));
    }
    if input.link_label.chars().any(char::is_control) {
        return Err(anyhow!("Link label cannot contain control characters"));
    }
    keys(&input.title)?;
    keys(&input.body)?;
    keys(&input.link_url)?;
    Ok(())
}
fn lookup(payload: &Value, key: &str) -> Option<String> {
    let mut value = payload;
    for part in key.split('.') {
        value = value.get(part)?;
    }
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(a)
            if a.iter()
                .all(|v| v.is_string() || v.is_number() || v.is_boolean()) =>
        {
            Some(
                a.iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        }
        _ => None,
    }
}
// Variable values remain literal inside the shared bold/link syntax.
fn literal(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if "\\*[]".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn substitute(
    text: &str,
    payload: &Value,
    escape: bool,
    optional: bool,
) -> Result<String, Vec<String>> {
    let mut out = String::new();
    let mut rest = text;
    let mut missing = vec![];
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let following = &rest[start + 2..];
        let end = following.find("}}").unwrap();
        let key = following[..end].trim();
        match lookup(payload, key) {
            Some(value) => out.push_str(&if escape && key != "message" {
                literal(&value)
            } else {
                value
            }),
            None if optional || matches!(key, "title" | "tags") => {}
            None => missing.push(key.to_owned()),
        }
        rest = &following[end + 2..];
    }
    out.push_str(rest);
    if missing.is_empty() {
        Ok(out)
    } else {
        missing.sort();
        missing.dedup();
        Err(missing)
    }
}
pub fn failed(payload: &Value) -> bool {
    payload["failed"].as_f64().is_some_and(|n| n > 0.)
        || payload["failed"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .is_some_and(|n| n > 0.)
        || payload["success"] == false
        || ["status", "job_status"].iter().any(|key| {
            payload[*key].as_str().is_some_and(|s| {
                matches!(
                    s.trim().to_lowercase().as_str(),
                    "failed" | "failure" | "error" | "canceled" | "cancelled" | "❌" | "✕" | "✖"
                )
            })
        })
}
pub fn status(payload: &Value, passed: &str, failed: &str) -> (String, Vec<String>) {
    let counter = |path: &str| crate::payload_rules::lookup(payload, path).and_then(Value::as_u64);
    let (passed_count, failed_count) = (counter(passed), counter(failed));
    let mut warnings = vec![];
    for (path, value) in [(passed, passed_count), (failed, failed_count)] {
        if value.is_none() {
            warnings.push(format!("{path}: expected a non-negative integer; no success status can be inferred from invalid counters"));
        }
    }
    let result = if failed_count.is_some_and(|n| n > 0) {
        "❌"
    } else if failed_count == Some(0) && passed_count.is_some_and(|n| n > 0) {
        "✅"
    } else {
        "⚠️"
    };
    (result.into(), warnings)
}
pub fn render(template: &Template, payload: &Value) -> Result<(Value, Value)> {
    let mut context = payload.clone();
    if context.get("priority").is_none() || context["priority"].is_null() {
        context["priority"] = json!(3);
    }
    let (computed, warnings) = if template.computed_status {
        let (status, warnings) = status(payload, &template.passed_field, &template.failed_field);
        context["status"] = json!(status);
        (Some(status), warnings)
    } else {
        (None, vec![])
    };
    let title = substitute(&template.title, &context, false, false);
    let body = substitute(&template.body, &context, true, false);
    let mut missing = vec![];
    if let Err(keys) = &title {
        missing.extend(keys.clone());
    }
    if let Err(keys) = &body {
        missing.extend(keys.clone());
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        return Ok((
            payload.clone(),
            json!({"applied":false,"reason":"missing_fields","missing_fields":missing,"computed_status":computed,"warnings":warnings}),
        ));
    }
    let title = title.unwrap();
    let mut body = body.unwrap();
    let mut link_added = false;
    if !template.link_url.is_empty() && (!template.link_failure_only || failed(&context)) {
        let link = substitute(&template.link_url, &context, false, true).unwrap_or_default();
        if let Ok(url) = url::Url::parse(&link) {
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && !link.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                let label = literal(if template.link_label.is_empty() {
                    "Runner log"
                } else {
                    &template.link_label
                });
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str(&format!(
                    "[{label}]({})",
                    url.as_str().replace('(', "%28").replace(')', "%29")
                ));
                link_added = true;
            }
        }
    }
    if title.len() > 256 || body.len() > 10000 {
        return Ok((
            payload.clone(),
            json!({"applied":false,"reason":"rendered_content_too_long","computed_status":computed,"warnings":warnings}),
        ));
    }
    let mut rendered = payload.clone();
    if let Some(status) = &computed {
        rendered["status"] = json!(status);
    }
    rendered.as_object_mut().unwrap().remove("alertname");
    rendered["title"] = json!(title);
    rendered["message"] = json!(body);
    Ok((
        rendered,
        json!({"applied":true,"reason":"rendered","link_added":link_added,"computed_status":computed,"warnings":warnings}),
    ))
}
pub async fn list(State(s): State<AppState>, h: HeaderMap) -> ApiResult<Vec<Template>> {
    auth::authenticated(&h, &s).await?;
    Ok(Json(
        sqlx::query_as("SELECT id,name,title,body,link_label,link_url,link_failure_only,computed_status,passed_field,failed_field,version,created_at,updated_at FROM message_templates ORDER BY LOWER(name),id")
            .fetch_all(&s.db)
            .await
            .map_err(db_error)?,
    ))
}
async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    template: &Template,
    author: &str,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO message_template_versions(template_id,version,snapshot,created_by) VALUES($1,$2,$3,$4)").bind(template.id).bind(template.version).bind(json!(template)).bind(author).execute(&mut **tx).await?;
    Ok(())
}
pub async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<Input>,
) -> ApiResult<Template> {
    let user = auth::authenticated(&h, &s).await?;
    validate(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let template:Template=sqlx::query_as("INSERT INTO message_templates(id,name,title,body,link_label,link_url,link_failure_only,computed_status,passed_field,failed_field) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id,name,title,body,link_label,link_url,link_failure_only,computed_status,passed_field,failed_field,version,created_at,updated_at").bind(Uuid::new_v4()).bind(input.name.trim()).bind(input.title).bind(input.body).bind(input.link_label).bind(input.link_url).bind(input.link_failure_only).bind(input.computed_status).bind(input.passed_field).bind(input.failed_field).fetch_one(&mut *tx).await.map_err(db_error)?;
    record(&mut tx, &template, &user.display_name)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(template))
}
async fn replace(s: &AppState, id: Uuid, input: Input, author: &str) -> ApiResult<Template> {
    validate(&input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut tx = s.db.begin().await.map_err(db_error)?;
    let template:Option<Template>=sqlx::query_as("UPDATE message_templates SET name=$2,title=$3,body=$4,link_label=$5,link_url=$6,link_failure_only=$7,computed_status=$8,passed_field=$9,failed_field=$10,version=version+1,updated_at=NOW() WHERE id=$1 RETURNING id,name,title,body,link_label,link_url,link_failure_only,computed_status,passed_field,failed_field,version,created_at,updated_at").bind(id).bind(input.name.trim()).bind(input.title).bind(input.body).bind(input.link_label).bind(input.link_url).bind(input.link_failure_only).bind(input.computed_status).bind(input.passed_field).bind(input.failed_field).fetch_optional(&mut *tx).await.map_err(db_error)?;
    let template = template.ok_or_else(|| err(StatusCode::NOT_FOUND, "Template not found"))?;
    record(&mut tx, &template, author).await.map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(template))
}
pub async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Input>,
) -> ApiResult<Template> {
    let user = auth::authenticated(&h, &s).await?;
    replace(&s, id, input, &user.display_name).await
}
#[derive(Deserialize)]
pub struct VersionQuery {
    before: Option<i32>,
}
pub async fn versions(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<VersionQuery>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM message_templates WHERE id=$1)")
            .bind(id)
            .fetch_one(&s.db)
            .await
            .map_err(db_error)?;
    if !exists {
        return Err(err(StatusCode::NOT_FOUND, "Template not found"));
    }
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('version',version,'snapshot',snapshot,'created_by',created_by,'created_at',created_at) FROM message_template_versions WHERE template_id=$1 AND ($2::int IS NULL OR version<$2) ORDER BY version DESC LIMIT 101").bind(id).bind(query.before).fetch_all(&s.db).await.map_err(db_error)?;
    let has_more = rows.len() > 100;
    let items = rows.into_iter().take(100).map(|r| r.0).collect::<Vec<_>>();
    let next_before = if has_more {
        items.last().map(|item| item["version"].clone())
    } else {
        None
    };
    Ok(Json(json!({"items":items,"next_before":next_before})))
}
pub async fn restore(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, version)): Path<(Uuid, i32)>,
) -> ApiResult<Template> {
    let user = auth::authenticated(&h, &s).await?;
    let snapshot: Option<Value> = sqlx::query_scalar(
        "SELECT snapshot FROM message_template_versions WHERE template_id=$1 AND version=$2",
    )
    .bind(id)
    .bind(version)
    .fetch_optional(&s.db)
    .await
    .map_err(db_error)?;
    let input: Input = serde_json::from_value(
        snapshot.ok_or_else(|| err(StatusCode::NOT_FOUND, "Template version not found"))?,
    )
    .map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Stored template version is invalid",
        )
    })?;
    replace(&s, id, input, &user.display_name).await
}
pub async fn delete(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    sqlx::query("DELETE FROM message_templates WHERE id=$1")
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Preview {
    #[serde(flatten)]
    input: Input,
    payload: Value,
}
pub async fn preview(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<Preview>,
) -> ApiResult<Value> {
    auth::authenticated(&h, &s).await?;
    validate(&input.input).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    if !input.payload.is_object() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Example payload must be a JSON object",
        ));
    }
    let now = Utc::now();
    let i = input.input;
    let t = Template {
        computed_status: i.computed_status,
        passed_field: i.passed_field,
        failed_field: i.failed_field,
        id: Uuid::nil(),
        name: i.name,
        title: i.title,
        body: i.body,
        link_label: i.link_label,
        link_url: i.link_url,
        link_failure_only: i.link_failure_only,
        version: 0,
        created_at: now,
        updated_at: now,
    };
    let (payload, result) =
        render(&t, &input.payload).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(
        json!({"title":message::event_title(&payload),"message":message::event_message(&payload),"result":result,"sent":false}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn template() -> Template {
        let now = Utc::now();
        Template{computed_status:false,passed_field:passed_field(),failed_field:failed_field(),id:Uuid::nil(),name:"Test results".into(),title:"{{test_type}} {{suite}}".into(),body:"Branch: **{{build.branch}}**\n\nStatus: {{status}}\nFailed: {{failed}}\nPassed: {{passed}}\nSkipped: {{skipped}}".into(),link_label:"Runner log".into(),link_url:"{{log_url}}".into(),link_failure_only:true,version:1,created_at:now,updated_at:now}
    }
    fn payload() -> Value {
        json!({"test_type":"UnitTests","suite":"ExampleSuite","build":{"branch":"feature/example"},"status":"✅","failed":0,"passed":40,"skipped":0,"log_url":"https://gitlab.example.org/demo/-/jobs/12345"})
    }
    #[test]
    fn calculated_status_prioritizes_failures() {
        for (p, want) in [
            (json!({"passed":40,"failed":2}), "❌"),
            (json!({"passed":40,"failed":0}), "✅"),
            (json!({"passed":0,"failed":0}), "⚠️"),
            (json!({"passed":40}), "⚠️"),
            (json!({"passed":40,"failed":"0"}), "⚠️"),
            (json!({"failed":2}), "❌"),
            (json!({"passed":-1,"failed":0}), "⚠️"),
        ] {
            assert_eq!(status(&p, "passed", "failed").0, want);
        }
    }
    #[test]
    fn computed_status_overrides_only_render_context() {
        let mut t = template();
        t.computed_status = true;
        let mut original = payload();
        original["failed"] = json!(2);
        let (rendered, result) = render(&t, &original).unwrap();
        assert!(rendered["message"].as_str().unwrap().contains("Status: ❌"));
        assert_eq!(original["status"], "✅");
        assert_eq!(result["computed_status"], "❌");
        let mut t = template();
        t.computed_status = true;
        t.body = "{{status}}".into();
        let (p, r) = render(&t, &json!({"test_type":"Tests","suite":"Unit"})).unwrap();
        assert_eq!(p["message"], "⚠️");
        assert_eq!(r["warnings"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn zero_counts_and_dotted_values() {
        let (p, r) = render(&template(), &payload()).unwrap();
        assert_eq!(p["title"], "UnitTests ExampleSuite");
        assert!(p["message"]
            .as_str()
            .unwrap()
            .contains("Failed: 0\nPassed: 40\nSkipped: 0"));
        assert_eq!(r["link_added"], false);
    }
    #[test]
    fn optional_failure_link() {
        let mut p = payload();
        p["failed"] = json!(2);
        let (rendered, r) = render(&template(), &p).unwrap();
        assert_eq!(r["link_added"], true);
        assert!(rendered["message"]
            .as_str()
            .unwrap()
            .contains("[Runner log](https://gitlab.example.org/demo/-/jobs/12345)"));
        for unsafe_url in [
            "javascript:alert(1)",
            "https://user:password@example.org",
            "https://example.org/a\nb",
            "",
        ] {
            p["log_url"] = json!(unsafe_url);
            assert_eq!(render(&template(), &p).unwrap().1["link_added"], false);
        }
        p.as_object_mut().unwrap().remove("log_url");
        assert_eq!(render(&template(), &p).unwrap().1["link_added"], false);
    }
    #[test]
    fn missing_fields_preserve_original() {
        let p = json!({"message":"Original **bold**"});
        let (rendered, r) = render(&template(), &p).unwrap();
        assert_eq!(rendered, p);
        assert_eq!(r["reason"], "missing_fields");
    }
    #[test]
    fn variables_are_literal_but_message_keeps_markup() {
        let mut t = template();
        t.title = "{{title}}".into();
        t.body = "{{suite}}\n{{message}}\n{{priority}}\n{{tags}}".into();
        t.link_url.clear();
        let (p, _) = render(
            &t,
            &json!({"suite":"**not bold** [x](https://example.org)","message":"**actual bold**"}),
        )
        .unwrap();
        assert_eq!(p["title"], "");
        assert_eq!(
            p["message"],
            "\\*\\*not bold\\*\\* \\[x\\](https://example.org)\n**actual bold**\n3\n"
        );
    }
    #[test]
    fn reject_expressions_and_unbounded_fields() {
        for key in [
            "{{",
            "{{ }}",
            "{{foo..bar}}",
            "{{#if status}}",
            "{{message | upper}}",
            "{{../../secret}}",
        ] {
            assert!(keys(key).is_err());
        }
        assert!(keys("{{build.branch}}").is_ok());
        let i = Input {
            computed_status: false,
            passed_field: passed_field(),
            failed_field: failed_field(),
            name: "Example".into(),
            title: "".into(),
            body: "{{message}}".repeat(101),
            link_label: label(),
            link_url: "".into(),
            link_failure_only: true,
        };
        assert!(validate(&i).is_err());
    }
    #[test]
    fn blank_heading_does_not_reuse_original_alert_name() {
        let mut t = template();
        t.title.clear();
        t.body = "{{message}}".into();
        t.link_url.clear();
        let (p, _) = render(&t, &json!({"alertname":"Original alert","message":"Body"})).unwrap();
        assert_eq!(message::event_title(&p), "");
        assert_eq!(message::event_message(&p), "Body");
    }
    #[test]
    fn failure_signals() {
        for p in [
            json!({"status":"failed"}),
            json!({"status":"❌"}),
            json!({"success":false}),
            json!({"failed":"2"}),
            json!({"job_status":"canceled"}),
        ] {
            assert!(failed(&p));
        }
        for p in [
            json!({}),
            json!({"failed":0}),
            json!({"status":"success"}),
            json!({"success":true}),
        ] {
            assert!(!failed(&p));
        }
    }
}
