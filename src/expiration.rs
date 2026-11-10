//! Expired queued work is terminal; active sends may finish and history remains retained.
use crate::*;
pub async fn expire(db: &PgPool) -> Result<()> {
    sqlx::query("WITH due AS (SELECT id FROM deliveries WHERE expires_at<=NOW() AND (status='queued' OR (status='processing' AND locked_at<NOW()-INTERVAL '2 minutes')) ORDER BY expires_at,id FOR UPDATE SKIP LOCKED LIMIT 100), closed AS (UPDATE deliveries d SET status='expired',locked_at=NULL,error='Message TTL expired',error_category='expired' FROM due WHERE d.id=due.id RETURNING d.id) UPDATE delivery_attempts SET status='interrupted',finished_at=NOW(),error='Message TTL expired after worker lease loss' WHERE delivery_id IN (SELECT id FROM closed) AND status='processing'")
        .execute(db).await?;
    sqlx::query("UPDATE alert_deliveries SET status='expired',locked_at=NULL,error='Message TTL expired',error_category='expired' WHERE id IN (SELECT id FROM alert_deliveries WHERE expires_at<=NOW() AND (status='queued' OR (status='processing' AND locked_at<NOW()-INTERVAL '2 minutes')) ORDER BY expires_at,id FOR UPDATE SKIP LOCKED LIMIT 100)").execute(db).await?;
    Ok(())
}
pub async fn ensure_live(db: &PgPool, id: Uuid) -> Result<()> {
    let expired: bool =
        sqlx::query_scalar("SELECT COALESCE(expires_at<=NOW(),false) FROM deliveries WHERE id=$1")
            .bind(id)
            .fetch_one(db)
            .await?;
    if expired {
        return Err(retry_policy::message("Message TTL expired"));
    }
    Ok(())
}
