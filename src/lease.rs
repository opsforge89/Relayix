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
pub enum Claim<T> {
    Ready(T),
    Idle,
    Contended,
}
const PROXIED: &str =
    "c.kind='telegram' AND COALESCE(c.config->>'proxy_type','none') IN ('mtproto','webproxy')";
const ACTIVE: &str = "(SELECT COUNT(*) FROM deliveries d WHERE d.channel_id=c.id AND d.status='processing' AND d.locked_at>=NOW()-INTERVAL '2 minutes') + (SELECT COUNT(*) FROM alert_deliveries a WHERE a.channel_id=c.id AND a.status='processing' AND a.locked_at>=NOW()-INTERVAL '2 minutes')";
const PROXY_ACTIVE: &str = "(SELECT COUNT(*) FROM deliveries d JOIN channels p ON p.id=d.channel_id WHERE d.status='processing' AND d.locked_at>=NOW()-INTERVAL '2 minutes' AND p.kind='telegram' AND COALESCE(p.config->>'proxy_type','none') IN ('mtproto','webproxy')) + (SELECT COUNT(*) FROM alert_deliveries a JOIN channels p ON p.id=a.channel_id WHERE a.status='processing' AND a.locked_at>=NOW()-INTERVAL '2 minutes' AND p.kind='telegram' AND COALESCE(p.config->>'proxy_type','none') IN ('mtproto','webproxy'))";
fn candidate(
    queue: Queue,
    limit: i64,
    proxy_limit: i64,
    lock: bool,
) -> sqlx::QueryBuilder<sqlx::Postgres> {
    let table = match queue {
        Queue::Delivery => "deliveries",
        Queue::Alert => "alert_deliveries",
    };
    let authorized=match queue {
        Queue::Delivery=>"EXISTS(SELECT 1 FROM events e WHERE e.id=d.event_id AND relay_route_send(e.route_id,d.channel_id))",
        Queue::Alert=>"(d.owner_user_id IS NULL OR relay_channel_access(d.channel_id,d.owner_user_id,'send'))",
    };
    let mut query = sqlx::QueryBuilder::new("SELECT c.id,(");
    query.push(PROXIED).push(") FROM channels c LEFT JOIN channel_health h ON h.channel_id=c.id JOIN LATERAL (SELECT MIN(d.created_at) AS oldest FROM ")
        .push(table)
        .push(" d WHERE ").push(authorized).push(" AND d.channel_id=c.id AND ((d.status='queued' AND d.next_attempt_at<=NOW() AND d.not_before<=NOW() AND (d.expires_at IS NULL OR d.expires_at>NOW())) OR (d.status='processing' AND d.locked_at<NOW()-INTERVAL '2 minutes'))) ready ON ready.oldest IS NOT NULL WHERE NOT (c.manual_paused AND (c.manual_pause_until IS NULL OR c.manual_pause_until>NOW())) AND (h.paused_until IS NULL OR (h.paused_until<=NOW() AND (")
        .push(ACTIVE).push(")=0)) AND ")
        .push(ACTIVE).push(" < ").push_bind(limit)
        .push(" AND (NOT (").push(PROXIED).push(") OR (").push(PROXY_ACTIVE).push(") < ").push_bind(proxy_limit).push(") ORDER BY ready.oldest,c.id");
    if lock {
        query.push(" FOR UPDATE OF c SKIP LOCKED");
    }
    query.push(" LIMIT 1");
    query
}

/// Bound proxy workers globally, serialize channel claims, and distinguish lock races from idle queues.
pub async fn claim_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    queue: Queue,
    limit: i64,
    proxy_limit: i64,
) -> Result<Claim<Uuid>> {
    let row: Option<(Uuid, bool)> = candidate(queue, limit, proxy_limit, true)
        .build_query_as()
        .fetch_optional(&mut **tx)
        .await?;
    let Some((channel, proxied)) = row else {
        let available: Option<(Uuid, bool)> = candidate(queue, limit, proxy_limit, false)
            .build_query_as()
            .fetch_optional(&mut **tx)
            .await?;
        return Ok(if available.is_some() {
            Claim::Contended
        } else {
            Claim::Idle
        });
    };
    if proxied {
        // Different proxy channels share this nonblocking transaction lock. Normal
        // channels never acquire it, so they can claim while proxy capacity is busy.
        let owned: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(914028)")
            .fetch_one(&mut **tx)
            .await?;
        if !owned {
            return Ok(Claim::Contended);
        }
    }
    // Recount after locks: the original statement snapshot may precede another claim's commit.
    let mut capacity = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT (");
    capacity.push(ACTIVE).push(") < ").push_bind(limit);
    if proxied {
        capacity
            .push(" AND (")
            .push(PROXY_ACTIVE)
            .push(") < ")
            .push_bind(proxy_limit);
    }
    capacity.push(" AND NOT (c.manual_paused AND (c.manual_pause_until IS NULL OR c.manual_pause_until>NOW())) AND NOT EXISTS(SELECT 1 FROM channel_health h WHERE h.channel_id=c.id AND (h.paused_until>NOW() OR (h.paused_until IS NOT NULL AND (")
        .push(ACTIVE).push(")>0)))");
    capacity
        .push(" FROM channels c WHERE c.id=")
        .push_bind(channel);
    let capacity: bool = capacity.build_query_scalar().fetch_one(&mut **tx).await?;
    Ok(if capacity {
        Claim::Ready(channel)
    } else {
        Claim::Contended
    })
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
