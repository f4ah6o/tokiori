//! The outbox: events written with the change that caused them.
//!
//! Delivery is at-least-once. An event stays pending until a consumer
//! acknowledges it, so a crash between delivery and acknowledgement re-delivers
//! rather than loses. Consumers must deduplicate on `event_id`; this module
//! does not promise exactly-once.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Postgres;
use sqlx::Transaction;
use uuid::Uuid;

use crate::error::PgError;
use crate::store::PgStore;

/// An event waiting to be delivered.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub scope: String,
    pub revision: u64,
    pub event_type: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

/// Writes an event inside an existing transaction.
pub(crate) async fn enqueue_in(
    tx: &mut Transaction<'_, Postgres>,
    scope: &str,
    revision: u64,
    event_type: &str,
    payload: &Value,
) -> Result<Uuid, PgError> {
    let event_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO outbox_events (event_id, scope_id, revision, event_type, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind(scope)
    .bind(i64::try_from(revision).expect("revision fits"))
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(event_id)
}

impl PgStore {
    /// Claims pending events for delivery.
    ///
    /// Claiming does not remove an event: it stays pending until acknowledged,
    /// so the same event can be claimed again after a consumer crash.
    pub async fn claim_outbox(&self, scope: &str, limit: i64) -> Result<Vec<OutboxEvent>, PgError> {
        let mut tx = self.pool().begin().await?;
        let rows: Vec<(Uuid, i64, String, Value, DateTime<Utc>)> = sqlx::query_as(
            "UPDATE outbox_events SET claimed_at = now() \
             WHERE event_id IN ( \
                 SELECT event_id FROM outbox_events \
                 WHERE scope_id = $1 AND acknowledged_at IS NULL \
                 ORDER BY created_at \
                 LIMIT $2 \
                 FOR UPDATE SKIP LOCKED ) \
             RETURNING event_id, revision, event_type, payload, created_at",
        )
        .bind(scope)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(rows
            .into_iter()
            .map(
                |(event_id, revision, event_type, payload, created_at)| OutboxEvent {
                    event_id,
                    scope: scope.to_owned(),
                    revision: u64::try_from(revision).expect("positive"),
                    event_type,
                    payload,
                    created_at,
                },
            )
            .collect())
    }

    /// Retires events a consumer has processed.
    ///
    /// Acknowledging an already-acknowledged event is a no-op, so a consumer
    /// that retries after a crash does not fail.
    pub async fn ack_outbox(&self, event_ids: &[Uuid]) -> Result<u64, PgError> {
        if event_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            "UPDATE outbox_events SET acknowledged_at = now() \
             WHERE event_id = ANY($1) AND acknowledged_at IS NULL",
        )
        .bind(event_ids)
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
    }
}
