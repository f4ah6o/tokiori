//! Revision numbering.
//!
//! A revision is the unit of "what the system knew" in one scope. Writers take
//! the next number under a row lock, so two concurrent writers never share one
//! and a reader can name an exact point in the history.

use sqlx::PgPool;

use crate::error::PgError;

/// The next revision to assign in a scope.
///
/// The counter is advanced in its own transaction. A writer that later rolls
/// back leaves a gap in the sequence, which is why the contract is
/// monotonicity rather than "no gaps": a gap is harmless, a duplicate is not.
pub async fn next_revision(pool: &PgPool, scope: &str) -> Result<u64, PgError> {
    let mut tx = pool.begin().await?;

    // Insert the counter on first use, then lock the row and advance it.
    sqlx::query(
        "INSERT INTO scope_revisions (scope_id, last_revision) VALUES ($1, 0) \
         ON CONFLICT (scope_id) DO NOTHING",
    )
    .bind(scope)
    .execute(&mut *tx)
    .await?;

    let last: i64 = sqlx::query_scalar(
        "UPDATE scope_revisions SET last_revision = last_revision + 1, updated_at = now() \
         WHERE scope_id = $1 RETURNING last_revision",
    )
    .bind(scope)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(u64::try_from(last).expect("revision is positive"))
}

/// The newest revision assigned in a scope, or `None` if none has been.
pub async fn latest_revision(pool: &PgPool, scope: &str) -> Result<Option<u64>, PgError> {
    let last: Option<i64> =
        sqlx::query_scalar("SELECT last_revision FROM scope_revisions WHERE scope_id = $1")
            .bind(scope)
            .fetch_optional(pool)
            .await?;

    Ok(last.map(|value| u64::try_from(value).expect("revision is positive")))
}
