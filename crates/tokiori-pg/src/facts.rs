//! Recording changes to facts.
//!
//! A change never edits a row. It closes the version that was current and
//! inserts a new one, both in one transaction, so a reader at an earlier
//! revision keeps seeing exactly what it saw before.

use chrono::NaiveDate;
use serde_json::Value;
use tokiori_model::ValidTime;

use crate::error::PgError;
use crate::store::{PgStore, check_attribute_value, load_declared_attribute};

/// A request to record a value for one attribute.
#[derive(Debug, Clone)]
pub struct SetAttribute {
    pub scope: String,
    pub entity_type: String,
    pub entity_id: String,
    pub attribute: String,
    pub value: Value,
    pub valid_time: ValidTime,
}

impl PgStore {
    /// Records a value, closing the versions it replaces.
    ///
    /// Returns the revision the change was recorded under.
    pub async fn set_attribute(&self, input: SetAttribute) -> Result<u64, PgError> {
        let mut tx = self.pool().begin().await?;

        let declared = load_declared_attribute(
            &mut tx,
            &input.scope,
            &input.entity_type,
            &input.entity_id,
            &input.attribute,
        )
        .await?;
        check_attribute_value(&input.attribute, declared, &input.value)?;

        let revision = crate::revision::next_revision_in(&mut tx, &input.scope).await?;

        // Close every still-current version that overlaps the new valid
        // interval, and cut the overlap out of it. The closed row keeps its
        // own history; only the part the new value takes over is removed.
        sqlx::query(
            "UPDATE fact_versions \
             SET recorded_to = $5, \
                 valid_until = CASE \
                     WHEN valid_from < $6 THEN $6 \
                     ELSE valid_until \
                 END \
             WHERE scope_id = $1 AND entity_type = $2 AND entity_id = $3 AND attribute = $4 \
               AND recorded_to IS NULL \
               AND (valid_until IS NULL OR valid_until > $6) \
               AND ($7::date IS NULL OR valid_from < $7)",
        )
        .bind(&input.scope)
        .bind(&input.entity_type)
        .bind(&input.entity_id)
        .bind(&input.attribute)
        .bind(i64::try_from(revision).expect("revision fits"))
        .bind(input.valid_time.from)
        .bind(input.valid_time.until)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO fact_versions \
             (scope_id, entity_type, entity_id, attribute, valid_from, valid_until, recorded_from, value) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&input.scope)
        .bind(&input.entity_type)
        .bind(&input.entity_id)
        .bind(&input.attribute)
        .bind(input.valid_time.from)
        .bind(input.valid_time.until)
        .bind(i64::try_from(revision).expect("revision fits"))
        .bind(&input.value)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(revision)
    }

    /// Ends a value at `until`, leaving the earlier period readable.
    ///
    /// This is a tombstone: the version is closed, not deleted, so a query for
    /// an earlier date still finds it.
    pub async fn end_attribute(
        &self,
        scope: &str,
        entity_type: &str,
        entity_id: &str,
        attribute: &str,
        until: NaiveDate,
    ) -> Result<u64, PgError> {
        let mut tx = self.pool().begin().await?;

        let revision = crate::revision::next_revision_in(&mut tx, scope).await?;

        let current: Option<NaiveDate> = sqlx::query_scalar(
            "SELECT valid_from FROM fact_versions \
             WHERE scope_id = $1 AND entity_type = $2 AND entity_id = $3 AND attribute = $4 \
               AND recorded_to IS NULL LIMIT 1",
        )
        .bind(scope)
        .bind(entity_type)
        .bind(entity_id)
        .bind(attribute)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(valid_from) = current else {
            return Err(PgError::NothingToEnd {
                entity_type: entity_type.to_owned(),
                entity_id: entity_id.to_owned(),
                attribute: attribute.to_owned(),
            });
        };
        if until <= valid_from {
            return Err(PgError::EmptyInterval {
                from: valid_from.to_string(),
                until: until.to_string(),
            });
        }

        sqlx::query(
            "UPDATE fact_versions SET recorded_to = $5, valid_until = $6 \
             WHERE scope_id = $1 AND entity_type = $2 AND entity_id = $3 AND attribute = $4 \
               AND recorded_to IS NULL",
        )
        .bind(scope)
        .bind(entity_type)
        .bind(entity_id)
        .bind(attribute)
        .bind(i64::try_from(revision).expect("revision fits"))
        .bind(until)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(revision)
    }
}
