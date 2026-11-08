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
