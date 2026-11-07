//! Recording and reading role participants.
//!
//! A membership's department is not an attribute: it is the participant filling
//! a role. Moving someone between departments therefore changes a role, and a
//! role has its own history.

use chrono::NaiveDate;
use tokiori_model::{FactVersion, ValidTime, resolve_cloned};

use crate::error::PgError;
use sqlx::{Postgres, Transaction};

use crate::store::PgStore;

/// A stored role row: valid interval, recorded interval, and participant.
pub(crate) type StoredRoleVersion = (NaiveDate, Option<NaiveDate>, i64, Option<i64>, String);

/// A request to record who fills a role.
#[derive(Debug, Clone)]
pub struct SetRole {
    pub scope: String,
    pub relation_type: String,
    pub relation_id: String,
    pub role: String,
    /// The participant entity id.
    pub participant_id: String,
    pub valid_time: ValidTime,
}

/// A request to read who filled a role.
#[derive(Debug, Clone)]
pub struct ReadRole {
    pub scope: String,
    pub relation_type: String,
    pub relation_id: String,
    pub role: String,
    pub valid_at: NaiveDate,
    pub known_revision: Option<u64>,
}

/// What a role read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAnswer {
    pub participant_id: Option<String>,
    pub known_revision: u64,
    pub valid_at: NaiveDate,
}

impl PgStore {
    /// Records a participant for a role, closing the version it replaces.
    pub async fn set_role(&self, input: SetRole) -> Result<u64, PgError> {
        let mut tx = self.pool().begin().await?;
        let revision = set_role_in(&mut tx, &input).await?;
        tx.commit().await?;
        Ok(revision)
    }
}

/// Applies a role change inside an existing transaction.
///
/// Operations call this so the role change, the operation record, and the
/// outbox event commit together or not at all.
pub(crate) async fn set_role_in(
    tx: &mut Transaction<'_, Postgres>,
    input: &SetRole,
) -> Result<u64, PgError> {
    {
        let model = PgStore::load_latest_model(tx, &input.scope).await?;
        let Some(relation) = model.relations.get(&input.relation_type) else {
            return Err(PgError::RelationTypeNotFound {
                relation_type: input.relation_type.clone(),
            });
        };
        let Some(role) = relation.roles.get(&input.role) else {
            return Err(PgError::UnknownRole {
                relation_type: input.relation_type.clone(),
                role: input.role.clone(),
            });
        };

        // The participant must exist in this scope and have the role's type.
        let actual: Option<String> = sqlx::query_scalar(
            "SELECT entity_type FROM entities WHERE scope_id = $1 AND id = $2 LIMIT 1",
        )
        .bind(&input.scope)
        .bind(&input.participant_id)
        .fetch_optional(&mut **tx)
        .await?;
        match actual {
            None => {
                return Err(PgError::ScopeMismatch {
                    scope: input.scope.clone(),
                    entity_type: role.entity.clone(),
                    id: input.participant_id.clone(),
                });
            }
            Some(found) if found != role.entity => {
                return Err(PgError::EntityTypeMismatch {
                    role: input.role.clone(),
                    expected: role.entity.clone(),
                    actual: found,
                });
            }
            Some(_) => {}
        }

        let revision = crate::revision::next_revision_in(tx, &input.scope).await?;
        let recorded = i64::try_from(revision).expect("revision fits");

        // Close overlapping current versions and cut the overlap out of them.
        sqlx::query(
            "UPDATE relation_role_versions \
             SET recorded_to = $5, \
                 valid_until = CASE WHEN valid_from < $6 THEN $6 ELSE valid_until END \
             WHERE scope_id = $1 AND relation_type = $2 AND relation_id = $3 AND role = $4 \
               AND recorded_to IS NULL \
               AND (valid_until IS NULL OR valid_until > $6) \
               AND ($7::date IS NULL OR valid_from < $7)",
        )
        .bind(&input.scope)
        .bind(&input.relation_type)
        .bind(&input.relation_id)
        .bind(&input.role)
        .bind(recorded)
        .bind(input.valid_time.from)
        .bind(input.valid_time.until)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO relation_role_versions \
             (scope_id, relation_type, relation_id, recorded_from, valid_from, valid_until, role, \
              participant_entity_type, participant_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&input.scope)
        .bind(&input.relation_type)
        .bind(&input.relation_id)
        .bind(recorded)
        .bind(input.valid_time.from)
        .bind(input.valid_time.until)
        .bind(&input.role)
        .bind(&role.entity)
        .bind(&input.participant_id)
        .execute(&mut **tx)
        .await?;

        Ok(revision)
    }
}

impl PgStore {
    /// Reads who filled a role as of a date and revision.
    pub async fn read_role(&self, input: ReadRole) -> Result<RoleAnswer, PgError> {
        let latest = crate::revision::latest_revision(self.pool(), &input.scope).await?;
        let known_revision = match (input.known_revision, latest) {
            (Some(requested), _) => requested,
            (None, Some(latest)) => latest,
            (None, None) => {
                return Ok(RoleAnswer {
                    participant_id: None,
                    known_revision: 0,
                    valid_at: input.valid_at,
                });
            }
        };
        if latest.is_none_or(|latest| known_revision > latest) || known_revision == 0 {
            return Err(PgError::UnknownRevision {
                scope: input.scope.clone(),
                revision: known_revision,
                latest,
            });
        }

        let rows: Vec<StoredRoleVersion> = sqlx::query_as(
            "SELECT valid_from, valid_until, recorded_from, recorded_to, participant_id \
             FROM relation_role_versions \
             WHERE scope_id = $1 AND relation_type = $2 AND relation_id = $3 AND role = $4",
        )
        .bind(&input.scope)
        .bind(&input.relation_type)
        .bind(&input.relation_id)
        .bind(&input.role)
        .fetch_all(self.pool())
        .await?;

        let versions: Vec<FactVersion<String>> = rows
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

        Ok(RoleAnswer {
            participant_id: match resolve_cloned(&versions, input.valid_at, known_revision) {
                tokiori_model::Resolution::Found(value) => Some(value),
                tokiori_model::Resolution::NotFound
                | tokiori_model::Resolution::HistoryUnavailable { .. } => None,
            },
            known_revision,
            valid_at: input.valid_at,
        })
    }
}
