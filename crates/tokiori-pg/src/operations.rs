//! The operation ledger.
//!
//! An operation is the record of a business intent: who asked, under which
//! request, and what the system decided. The state itself lives in the P1
//! history; this ledger answers "why did that change, and was it already done".

use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Postgres;
use sqlx::Transaction;

use crate::error::PgError;
use crate::store::PgStore;

/// Identifies one request.
///
/// The key is namespaced by scope and operation type, so the same string used
/// for a transfer and a cancel, or in two scopes, is three different requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationKey {
    pub scope: String,
    pub operation_type: String,
    pub idempotency_key: String,
}

/// A recorded operation.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationRecord {
    pub key: OperationKey,
    pub input_digest: String,
    pub revision: u64,
    pub actor: String,
    pub reason: String,
    pub result: Value,
    pub created_at: DateTime<Utc>,
}

/// The digest that decides whether two requests are the same request.
///
/// It covers the normalized input only: the actor and reason are metadata
/// about the request, not part of what was asked.
#[must_use]
pub fn input_digest(input: &Value) -> String {
    let canonical = serde_json::to_string(input).expect("input is JSON");
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Looks up an existing operation, or records a new one.
///
/// Returns the digest of the recorded input. A replay (same key, same input)
/// is reported as [`PgError::OperationReplayed`] so the caller can return the
/// stored result instead of applying the effect again.
pub(crate) async fn record_operation_in(
    tx: &mut Transaction<'_, Postgres>,
    key: &OperationKey,
    input: &Value,
    revision: u64,
    actor: &str,
    reason: &str,
    result: &Value,
) -> Result<String, PgError> {
    let digest = input_digest(input);

    // `RETURNING` tells us whether this call is the one that created the row.
    // Without it, a replay and a fresh record look identical afterwards.
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO operations \
         (scope_id, operation_type, idempotency_key, input_digest, revision, actor, reason, result) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (scope_id, operation_type, idempotency_key) DO NOTHING \
         RETURNING input_digest",
    )
    .bind(&key.scope)
    .bind(&key.operation_type)
    .bind(&key.idempotency_key)
    .bind(&digest)
    .bind(i64::try_from(revision).expect("revision fits"))
    .bind(actor)
    .bind(reason)
    .bind(result)
    .fetch_optional(&mut **tx)
    .await?;

    if inserted.is_some() {
        return Ok(digest);
    }

    // The key was already used. Whether that is a replay or a conflict
    // depends on the input that was recorded the first time.
    let stored: String = sqlx::query_scalar(
        "SELECT input_digest FROM operations \
         WHERE scope_id = $1 AND operation_type = $2 AND idempotency_key = $3",
    )
    .bind(&key.scope)
    .bind(&key.operation_type)
    .bind(&key.idempotency_key)
    .fetch_one(&mut **tx)
    .await?;

    if stored == digest {
        Err(PgError::OperationReplayed {
            operation_type: key.operation_type.clone(),
            idempotency_key: key.idempotency_key.clone(),
        })
    } else {
        Err(PgError::IdempotencyConflict {
            operation_type: key.operation_type.clone(),
            idempotency_key: key.idempotency_key.clone(),
            stored,
            incoming: digest,
        })
    }
}

impl PgStore {
    /// Records an operation outside any larger transaction.
    ///
    /// Most callers should use an operation that applies its effect too; this
    /// exists for tests and for operations whose effect is recorded elsewhere.
    pub async fn record_operation(
        &self,
        key: OperationKey,
        input: &Value,
        revision: u64,
        actor: &str,
        reason: &str,
        result: &Value,
    ) -> Result<String, PgError> {
        let mut tx = self.pool().begin().await?;
        let digest =
            record_operation_in(&mut tx, &key, input, revision, actor, reason, result).await?;
        tx.commit().await?;
        Ok(digest)
    }

    /// Reads a recorded operation, if the key was used.
    pub async fn find_operation(
        &self,
        key: &OperationKey,
    ) -> Result<Option<OperationRecord>, PgError> {
        let row: Option<(String, i64, String, String, Value, DateTime<Utc>)> = sqlx::query_as(
            "SELECT input_digest, revision, actor, reason, result, created_at FROM operations \
             WHERE scope_id = $1 AND operation_type = $2 AND idempotency_key = $3",
        )
        .bind(&key.scope)
        .bind(&key.operation_type)
        .bind(&key.idempotency_key)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(
            |(input_digest, revision, actor, reason, result, created_at)| OperationRecord {
                key: key.clone(),
                input_digest,
                revision: u64::try_from(revision).expect("positive"),
                actor,
                reason,
                result,
                created_at,
            },
        ))
    }
}
