//! Strict sort allowlists; no user-provided identifier enters SQL.
use crate::*;
#[derive(Deserialize, Default)]
pub struct Sort {
    pub sort_by: Option<String>,
    pub sort_dir: Option<String>,
}
pub fn order(table: &str, sort: &Sort) -> Result<String, (StatusCode, Json<Value>)> {
    let by = sort.sort_by.as_deref().unwrap_or("created_at");
    let direction = match sort.sort_dir.as_deref().unwrap_or("desc") {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => return Err(err(StatusCode::BAD_REQUEST, "Invalid sort direction")),
    };
    if by == "title" && matches!(table, "deliveries" | "incoming" | "delivery_events") {
        return Ok(format!(
            " ORDER BY LOWER({}) {direction}, {}.id {direction}",
            message::TITLE_SQL,
            if table == "deliveries" { "d" } else { "e" }
        ));
    }
    let expression=match (table,by){
  ("delivery_events","created_at")=>"e.created_at",
  ("delivery_events","tags")=>"LOWER(CONCAT_WS(', ', (SELECT string_agg(tag, ', ') FROM jsonb_array_elements_text(e.tags) AS t(tag)), NULLIF(BTRIM(r.required_tag),'')))",
  ("delivery_events","route")=>"LOWER(r.name)",
  ("delivery_events","recipients")=>"(SELECT string_agg(LOWER(sort_c.name), ', ' ORDER BY LOWER(sort_c.name), sort_c.id) FROM deliveries sort_d JOIN channels sort_c ON sort_c.id=sort_d.channel_id WHERE sort_d.event_id=e.id)",
  ("deliveries","created_at")=>"d.created_at",
  ("deliveries","tags")=>"LOWER(CONCAT_WS(', ', (SELECT string_agg(tag, ', ') FROM jsonb_array_elements_text(e.tags) AS t(tag)), NULLIF(BTRIM(r.required_tag),'')))",("deliveries","route")=>"LOWER(r.name)",
  ("deliveries","channel")=>"LOWER(c.name)",("deliveries","status")=>"CASE WHEN d.status='queued' AND d.error IS NOT NULL THEN 'retrying' ELSE d.status END",
  ("deliveries","attempts")=>"d.attempts",
  ("incoming","created_at")=>"e.created_at",
  ("incoming","route")=>"LOWER(r.name)",("incoming","routing")=>"COALESCE(e.routing->>'reason','legacy')",
  ("incoming","deliveries")=>"(SELECT COUNT(*) FROM deliveries sort_d WHERE sort_d.event_id=e.id)",
  ("alert_history","created_at")=>"a.created_at",("alert_history","rule")=>"LOWER(COALESCE(p.name,'Deleted rule'))",
  ("alert_history","recipient")=>"LOWER(c.name)",("alert_history","status")=>"a.status",
  _=>return Err(err(StatusCode::BAD_REQUEST,"Invalid sort column")),
 };
    let alias = match table {
        "deliveries" => "d",
        "incoming" | "delivery_events" => "e",
        _ => "a",
    };
    Ok(format!(
        " ORDER BY {expression} {direction}, {alias}.id {direction}"
    ))
}
pub fn columns(id: &str) -> Option<&'static [&'static str]> {
    Some(match id {
        "deliveries" | "dashboard_deliveries" => &[
            "created_at",
            "title",
            "tags",
            "route",
            "recipients",
            "details",
        ],
        "incoming" => &[
            "created_at",
            "title",
            "route",
            "routing",
            "deliveries",
            "details",
        ],
        "templates" => &[
            "created_at",
            "name",
            "version",
            "updated_at",
            "routes",
            "actions",
        ],
        "users" => &["created_at", "user", "role", "status", "actions"],
        "alert_history" => &["created_at", "rule", "recipient", "status", "details"],
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_allowlisted_sort_expressions() {
        assert!(order("deliveries", &Sort::default())
            .unwrap()
            .contains("d.created_at DESC"));
        for key in ["title DESC; DROP TABLE events", "details", ""] {
            assert!(order(
                "deliveries",
                &Sort {
                    sort_by: Some(key.into()),
                    sort_dir: None
                }
            )
            .is_err());
        }
        assert!(order(
            "incoming",
            &Sort {
                sort_by: Some("title".into()),
                sort_dir: Some("asc".into())
            }
        )
        .unwrap()
        .contains("LOWER(COALESCE("));
    }
}
