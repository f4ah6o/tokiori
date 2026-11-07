//! Recording changes to facts.
//!
//! A change never edits a row. It closes the version that was current and
//! inserts a new one, both in one transaction, so a reader at an earlier
//! revision keeps seeing exactly what it saw before.

use chrono::NaiveDate;
use serde_json::Value;
use tokiori_model::{FactVersion, Resolution, ValidTime, resolve_cloned};

use crate::error::PgError;
use crate::store::{PgStore, check_attribute_value, load_declared_attribute};

/// A stored version row: valid interval, recorded interval, and value.
pub(crate) type StoredVersion = (NaiveDate, Option<NaiveDate>, i64, Option<i64>, Value);

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

        // Close the versions this change supersedes. Only `recorded_to` is set:
        // a version's valid interval is what that version *asserted*, and a
        // later revision must not rewrite it. Trimming it here would make a
        // query at an earlier revision stop seeing what it believed.
        sqlx::query(
            "UPDATE fact_versions SET recorded_to = $5 \
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

/// A request to read one attribute at a date and revision.
#[derive(Debug, Clone)]
pub struct ReadAttribute {
    pub scope: String,
    pub entity_type: String,
    pub entity_id: String,
    pub attribute: String,
    /// The business date to ask about.
    pub valid_at: NaiveDate,
    /// The revision to answer as of; `None` means the newest.
    pub known_revision: Option<u64>,
}

/// What a read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeAnswer {
    /// The value, or `None` when the fact did not hold on that date.
    pub value: Option<Value>,
    /// The revision the answer was resolved at.
    pub known_revision: u64,
    /// The date the answer is about.
    pub valid_at: NaiveDate,
}

impl PgStore {
    /// Reads one attribute as of a date and a revision.
    ///
    /// The rows are fetched here, but the choice of version is made by
    /// [`tokiori_model::resolve`], the same pure function the property tests
    /// check against a reference implementation.
    pub async fn read_attribute(&self, input: ReadAttribute) -> Result<AttributeAnswer, PgError> {
        let latest = crate::revision::latest_revision(self.pool(), &input.scope).await?;
        let known_revision = match (input.known_revision, latest) {
            (Some(requested), _) => requested,
            (None, Some(latest)) => latest,
            (None, None) => {
                return Ok(AttributeAnswer {
                    value: None,
                    known_revision: 0,
                    valid_at: input.valid_at,
                });
            }
        };

        // A revision that was never assigned in this scope cannot be answered;
        // returning "not found" would claim knowledge the system does not have.
        if latest.is_none_or(|latest| known_revision > latest) || known_revision == 0 {
            return Err(PgError::UnknownRevision {
                scope: input.scope.clone(),
                revision: known_revision,
                latest,
            });
        }

        let rows: Vec<StoredVersion> = sqlx::query_as(
            "SELECT valid_from, valid_until, recorded_from, recorded_to, value \
             FROM fact_versions \
             WHERE scope_id = $1 AND entity_type = $2 AND entity_id = $3 AND attribute = $4 \
               AND recorded_from <= $5",
        )
        .bind(&input.scope)
        .bind(&input.entity_type)
        .bind(&input.entity_id)
        .bind(&input.attribute)
        .bind(i64::try_from(known_revision).expect("revision fits"))
        .fetch_all(self.pool())
        .await?;

        let versions: Vec<FactVersion<Value>> = rows
            .into_iter()
            .map(|(from, until, recorded_from, recorded_to, value)| {
                let valid = match until {
                    None => ValidTime::unbounded_from(from),
                    Some(until) => ValidTime::bounded(from, until)
                        .expect("stored intervals are non-empty by constraint"),
                };
                FactVersion {
                    valid,
                    recorded_from: u64::try_from(recorded_from).expect("positive"),
                    recorded_to: recorded_to.map(|to| u64::try_from(to).expect("positive")),
                    value,
                }
            })
            .collect();

        let resolution = resolve_cloned(&versions, input.valid_at, known_revision);
        Ok(AttributeAnswer {
            value: match resolution {
                Resolution::Found(value) => Some(value),
                Resolution::NotFound | Resolution::HistoryUnavailable { .. } => None,
            },
            known_revision,
            valid_at: input.valid_at,
        })
    }
}
