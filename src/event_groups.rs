//! Every incoming event is retained; one leader owns each bounded grouping window's deliveries.
use crate::*;

pub struct Window {
    pub id: Uuid,
    pub leader: Uuid,
}
pub async fn find(
    tx: &mut sqlx::Transaction<'_,sqlx::Postgres>, route:Uuid, group_key:&str,
) -> Result<Option<Window>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind(format!("group:{route}:{group_key}")).execute(&mut **tx).await?;
    let row:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT id,leader_event_id FROM event_groups WHERE route_id=$1 AND group_key=$2 AND closes_at>clock_timestamp() AND (kind='dedup' OR occurrences<50) ORDER BY created_at DESC LIMIT 1 FOR UPDATE")
        .bind(route).bind(group_key).fetch_optional(&mut **tx).await?;
    let Some((id,leader))=row else{return Ok(None)};
    // Lock all leader deliveries before checking eligibility; claimers never lock group rows.
    let deliveries:Vec<(String,i32)>=sqlx::query_as("SELECT status,attempts FROM deliveries WHERE event_id=$1 ORDER BY id FOR UPDATE")
        .bind(leader).fetch_all(&mut **tx).await?;
    if deliveries.is_empty() || deliveries.iter().any(|(status,attempts)|status!="queued" || *attempts!=0) { return Ok(None); }
    Ok(Some(Window{id,leader}))
}
pub async fn presentation(db:&PgPool, mut payload:Value)->Result<Value> {
    let group=payload.as_object_mut().and_then(|p|p.remove("__relayix_group"));
    let Some(group)=group.and_then(|v|v.as_str().and_then(|s|Uuid::parse_str(s).ok())) else{return Ok(payload)};
    let info:Option<(String,i32)>=sqlx::query_as("SELECT kind,occurrences FROM event_groups WHERE id=$1").bind(group).fetch_optional(db).await?;
    let Some((kind,count))=info else{return Ok(payload)};
    if count<=1{return Ok(payload)}
    if kind=="dedup" {
        payload["message"]=json!(format!("{}\n\n×{}",message::event_message(&payload),count));
    } else {
        let members:Vec<(Value,)>=sqlx::query_as("SELECT COALESCE(rendered_payload,payload) FROM events WHERE group_id=$1 ORDER BY created_at,id LIMIT 50").bind(group).fetch_all(db).await?;
        let mut body=String::new();
        for (index,(event,)) in members.iter().enumerate() {
            let title:String=message::event_title(event).chars().take(80).collect();
            let text=message::event_message(event);
            let summary:String=text.chars().take(160).collect();
            let line=format!("{}. {}{}{}{}\n\n",index+1,title,if title.is_empty(){""}else{"\n"},summary,if summary.len()<text.len(){" [shortened]"}else{""});
            if body.len()+line.len()>8000 {body.push_str(&format!("{} more notification(s); view incoming history.\n",members.len()-index));break;}
            body.push_str(&line);
        }
        payload.as_object_mut().unwrap().remove("alertname");
        payload["title"]=json!(format!("Digest · {} notifications",members.len()));
        payload["message"]=json!(body);
    }
    Ok(payload)
}
