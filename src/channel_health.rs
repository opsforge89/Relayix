//! PostgreSQL-coordinated channel cooldowns across both queues and all replicas.
use crate::*;

pub async fn lock(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, channel: Uuid) -> Result<()> {
    sqlx::query("SELECT id FROM channels WHERE id=$1 FOR UPDATE")
        .bind(channel)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(())
}
/// Called only after the queue's attempt fence has confirmed ownership, with its channel locked.
pub async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    channel: Uuid,
    result: &Result<()>,
    attempt: i32,
    base: i32,
) -> Result<()> {
    match result {
        Ok(()) => {
            sqlx::query("DELETE FROM channel_health WHERE channel_id=$1 AND (paused_until IS NULL OR paused_until<=NOW())")
                .bind(channel).execute(&mut **tx).await?;
        }
        Err(error) => {
            let plan = retry_policy::plan(error, attempt, base);
            sqlx::query("INSERT INTO channel_health(channel_id,consecutive_failures,failure_signature,last_error,paused_until) VALUES($1,1,$2,$3,CASE WHEN $4::int>0 THEN NOW()+$4::int*INTERVAL '1 second' ELSE NULL END) ON CONFLICT(channel_id) DO UPDATE SET consecutive_failures=CASE WHEN channel_health.failure_signature=EXCLUDED.failure_signature THEN LEAST(channel_health.consecutive_failures+1,1000000) ELSE 1 END,failure_signature=EXCLUDED.failure_signature,last_error=EXCLUDED.last_error,paused_until=CASE WHEN channel_health.paused_until IS NOT NULL OR (channel_health.failure_signature=EXCLUDED.failure_signature AND channel_health.consecutive_failures>=2) OR $4::int>0 THEN GREATEST(COALESCE(channel_health.paused_until,NOW()),NOW()+GREATEST(60,$4::int)*INTERVAL '1 second') ELSE NULL END,updated_at=NOW()")
                .bind(channel).bind(plan.signature).bind(error.to_string().chars().take(512).collect::<String>()).bind(plan.cooldown).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

pub async fn status(db: &PgPool, channel: Uuid) -> Result<Value> {
    let value: Option<(Value,)> = sqlx::query_as("SELECT jsonb_build_object('consecutive_failures',consecutive_failures,'last_error',last_error,'paused_until',paused_until,'state',CASE WHEN paused_until>NOW() THEN 'paused' WHEN paused_until IS NOT NULL THEN 'probe' ELSE 'healthy' END) FROM channel_health WHERE channel_id=$1")
        .bind(channel).fetch_optional(db).await?;
    Ok(value.map(|v| v.0).unwrap_or_else(|| json!({"state":"healthy","consecutive_failures":0,"paused_until":null,"last_error":null})))
}
