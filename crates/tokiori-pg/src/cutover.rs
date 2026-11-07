//! Importing P0's current state as the first revision.
//!
//! P0 recorded only current values. The import makes them the initial history
//! without claiming anything P0 never recorded: an entity's value is only
//! asserted from the cutover date onward, because P0 kept no effective start.

use chrono::NaiveDate;
use serde_json::Value;

use crate::error::PgError;
use crate::store::PgStore;

/// A request to import a scope's P0 state.
#[derive(Debug, Clone)]
pub struct Cutover {
    pub scope: String,
    /// The instant after which imported facts are asserted.
    pub at: NaiveDate,
}

/// What the import did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutoverOutcome {
    /// The revision the state was imported as.
    pub revision: u64,
    /// Whether this call created it, as opposed to finding it already done.
    pub created: bool,
    pub entities: usize,
    pub relations: usize,
}

impl PgStore {
    /// Imports P0's current state as the scope's first revision.
    ///
    /// The import is idempotent: if the scope already has a revision, the
    /// existing one is reported and nothing is written. That makes an
    /// interrupted cutover safe to re-run, and it cannot produce two initial
    /// revisions.
    pub async fn cutover(&self, input: Cutover) -> Result<CutoverOutcome, PgError> {
        let mut tx = self.pool().begin().await?;

        // Serialize the import against every other writer in the scope.
        sqlx::query(
            "INSERT INTO scope_revisions (scope_id, last_revision) VALUES ($1, 0) \
             ON CONFLICT (scope_id) DO NOTHING",
        )
        .bind(&input.scope)
        .execute(&mut *tx)
        .await?;
        let current: i64 = sqlx::query_scalar(
            "SELECT last_revision FROM scope_revisions WHERE scope_id = $1 FOR UPDATE",
        )
        .bind(&input.scope)
        .fetch_one(&mut *tx)
        .await?;

        // The import is identified by its own ledger entry, not by the counter:
        // creating a relation allocates a revision too, so a P0-era scope can
        // already have a non-zero counter without having been imported.
        let imported: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM operations \
             WHERE scope_id = $1 AND operation_type = 'cutover' \
               AND idempotency_key = 'initial'",
        )
        .bind(&input.scope)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(revision) = imported {
            tx.rollback().await?;
            return Ok(CutoverOutcome {
                revision: u64::try_from(revision).expect("positive"),
                created: false,
                entities: 0,
                relations: 0,
            });
        }
        let _ = current;

        let revision = crate::revision::next_revision_in(&mut tx, &input.scope).await?;
        let recorded = i64::try_from(revision).expect("revision fits");

        let imported_entities = import_entities(&mut tx, &input.scope, input.at, recorded).await?;
        let imported_relations = import_relations(&mut tx, &input.scope, recorded).await?;

        // Record the import itself, so a re-run recognises it.
        crate::operations::record_operation_in(
            &mut tx,
            &crate::operations::OperationKey {
                scope: input.scope.clone(),
                operation_type: "cutover".to_owned(),
                idempotency_key: "initial".to_owned(),
            },
            &serde_json::json!({ "at": input.at }),
            revision,
            "cutover",
            "import P0 state",
            &serde_json::json!({ "revision": revision }),
        )
        .await?;

        tx.commit().await?;
        Ok(CutoverOutcome {
            revision,
            created: true,
            entities: imported_entities,
            relations: imported_relations,
        })
    }
}

/// Imports entity attributes as facts asserted from the cutover date.
///
/// P0 kept no effective start for an entity, so claiming any earlier date
/// would invent history.
async fn import_entities(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &str,
    at: chrono::NaiveDate,
    recorded: i64,
) -> Result<usize, PgError> {
    let entities: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT entity_type, id, attributes FROM entities WHERE scope_id = $1")
            .bind(scope)
            .fetch_all(&mut **tx)
            .await?;

    let mut imported = 0usize;
    for (entity_type, entity_id, attributes) in &entities {
        let Some(map) = attributes.as_object() else {
            continue;
        };
        for (attribute, value) in map {
            sqlx::query(
                "INSERT INTO fact_versions                  (scope_id, entity_type, entity_id, attribute, valid_from, recorded_from, value)                  VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(scope)
            .bind(entity_type)
            .bind(entity_id)
            .bind(attribute)
            .bind(at)
            .bind(recorded)
            .bind(value)
            .execute(&mut **tx)
            .await?;
        }
        imported += 1;
    }
    Ok(imported)
}

/// Imports relation roles, keeping the valid interval P0 did record.
async fn import_relations(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &str,
    recorded: i64,
) -> Result<usize, PgError> {
    let relations: Vec<(String, String, chrono::NaiveDate, Option<chrono::NaiveDate>)> =
        sqlx::query_as(
            "SELECT relation_type, id, valid_from, valid_until FROM relations WHERE scope_id = $1",
        )
        .bind(scope)
        .fetch_all(&mut **tx)
        .await?;

    let mut imported = 0usize;
    for (relation_type, relation_id, valid_from, valid_until) in &relations {
        let roles: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT role, participant_entity_type, participant_id FROM relation_roles              WHERE scope_id = $1 AND relation_type = $2 AND relation_id = $3",
        )
        .bind(scope)
        .bind(relation_type)
        .bind(relation_id)
        .fetch_all(&mut **tx)
        .await?;

        for (role, participant_entity_type, participant_id) in roles {
            sqlx::query(
                "INSERT INTO relation_role_versions                  (scope_id, relation_type, relation_id, role, valid_from, valid_until,                   recorded_from, participant_entity_type, participant_id)                  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(scope)
            .bind(relation_type)
            .bind(relation_id)
            .bind(role)
            .bind(valid_from)
            .bind(valid_until)
            .bind(recorded)
            .bind(participant_entity_type)
            .bind(participant_id)
            .execute(&mut **tx)
            .await?;
        }
        imported += 1;
    }
    Ok(imported)
}
