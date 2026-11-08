//! Keep a claimed queue attempt owned during transport waits and I/O.
use crate::*;
use std::future::Future;

#[derive(Clone, Copy)]
pub enum Queue {
    Delivery,
    Alert,
}
impl Queue {
    fn renewal(self) -> &'static str {
        match self {
            Self::Delivery=>"UPDATE deliveries SET locked_at=NOW() WHERE id=$1 AND attempts=$2 AND status='processing' AND locked_at>=NOW()-INTERVAL '2 minutes'",
            Self::Alert=>"UPDATE alert_deliveries SET locked_at=NOW() WHERE id=$1 AND attempts=$2 AND status='processing' AND locked_at>=NOW()-INTERVAL '2 minutes'",
        }
    }
}
/// Serialize claims for one channel across both queues and all app replicas.
pub async fn claim_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    queue: Queue,
    limit: i64,
) -> Result<Option<Uuid>> {
    let table = match queue {
        Queue::Delivery => "deliveries",
        Queue::Alert => "alert_deliveries",
    };
    let active = "(SELECT COUNT(*) FROM deliveries d WHERE d.channel_id=c.id AND d.status='processing' AND d.locked_at>=NOW()-INTERVAL '2 minutes') + (SELECT COUNT(*) FROM alert_deliveries a WHERE a.channel_id=c.id AND a.status='processing' AND a.locked_at>=NOW()-INTERVAL '2 minutes')";
    let mut candidate = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT c.id FROM channels c JOIN LATERAL (SELECT MIN(d.created_at) AS oldest FROM ",
    );
    candidate
        .push(table)
        .push(" d WHERE d.channel_id=c.id AND ((d.status='queued' AND d.next_attempt_at<=NOW()) OR (d.status='processing' AND d.locked_at<NOW()-INTERVAL '2 minutes'))) ready ON ready.oldest IS NOT NULL WHERE ")
        .push(active)
        .push(" < ")
        .push_bind(limit)
        .push(" ORDER BY ready.oldest,c.id FOR UPDATE OF c SKIP LOCKED LIMIT 1");
    let channel: Option<Uuid> = candidate
        .build_query_scalar()
        .fetch_optional(&mut **tx)
        .await?;
    let Some(channel) = channel else {
        return Ok(None);
    };
    // A claim may commit between the candidate statement's snapshot and its
    // channel lock. Recount in a fresh statement while holding that lock.
    let mut capacity = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT (");
    capacity
        .push(active)
        .push(") < ")
        .push_bind(limit)
        .push(" FROM channels c WHERE c.id=")
        .push_bind(channel);
    let capacity: bool = capacity
        .build_query_scalar()
        .fetch_one(&mut **tx)
        .await?;
    Ok(capacity.then_some(channel))
}

async fn renew(db: &PgPool, queue: Queue, id: Uuid, number: i32) -> Result<()> {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query(queue.renewal())
            .bind(id)
            .bind(number)
            .execute(db),
    )
    .await
    .map_err(|_| anyhow!("Delivery lease renewal timed out"))??;
    if result.rows_affected() != 1 {
        return Err(anyhow!("Delivery lease ownership lost"));
    }
    Ok(())
}
pub async fn guard<T>(
    db: &PgPool,
    queue: Queue,
    id: Uuid,
    number: i32,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    // Never start (or revive) an attempt whose lease has already expired or changed owners.
    renew(db, queue, id, number).await?;
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(20),
        Duration::from_secs(20),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tokio::pin!(work);
    loop {
        tokio::select! {
            biased;
            _=heartbeat.tick()=>renew(db,queue,id,number).await?,
            result=&mut work=>return result,
        }
    }
}
