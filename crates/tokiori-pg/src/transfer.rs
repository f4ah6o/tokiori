//! Transfer: moving an employee between departments as one operation.
//!
//! The operation, the role change, and the event commit together. A rejected
//! precondition therefore leaves no trace, and a retried request is recognised
//! by its key instead of moving the employee twice.

use chrono::NaiveDate;
use serde_json::{Value, json};
use tokiori_model::ValidTime;

use crate::error::PgError;
use crate::operations::{OperationKey, record_operation_in};
use crate::roles::StoredRoleVersion;
use crate::roles::{SetRole, set_role_at};
use crate::store::PgStore;

/// A request to transfer an employee to another department.
#[derive(Debug, Clone)]
pub struct Transfer {
    pub scope: String,
    pub employee: String,
    pub to_department: String,
    pub effective_on: NaiveDate,
    /// The caller's request id; the idempotency key for this operation.
    pub request_id: String,
    pub reason: String,
    pub actor: String,
}

/// What a transfer did.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationOutcome {
    /// The revision the change was recorded under.
    pub revision: u64,
    /// Whether this call applied the change, as opposed to replaying one.
    pub applied: bool,
    /// The recorded result, identical for an apply and a replay.
    pub result: Value,
}

impl Transfer {
    /// The normalized input that decides whether two requests are the same.
    fn input(&self) -> Value {
        json!({
            "employee": self.employee,
            "to_department": self.to_department,
            "effective_on": self.effective_on,
        })
    }

    fn key(&self) -> OperationKey {
        OperationKey {
            scope: self.scope.clone(),
            operation_type: "transfer".to_owned(),
            idempotency_key: self.request_id.clone(),
        }
    }
}

impl PgStore {
    /// Applies a transfer, or returns the result of an earlier identical one.
    pub async fn transfer(&self, request: Transfer) -> Result<OperationOutcome, PgError> {
        let mut tx = self.pool().begin().await?;
        let key = request.key();
        let input = request.input();

        // A replay must return the original result without touching state.
        if let Some(existing) = self.find_operation(&key).await? {
            if existing.input_digest == crate::operations::input_digest(&input) {
                tx.rollback().await?;
                return Ok(OperationOutcome {
                    revision: existing.revision,
                    applied: false,
                    result: existing.result,
                });
            }
            return Err(PgError::IdempotencyConflict {
                operation_type: key.operation_type,
                idempotency_key: key.idempotency_key,
                stored: existing.input_digest,
                incoming: crate::operations::input_digest(&input),
            });
        }

        let membership = self.require_membership(&mut tx, &request).await?;
        let current = self
            .current_department(&mut tx, &request, &membership)
            .await?;

        if current.as_deref() == Some(request.to_department.as_str()) {
            return Err(PgError::PreconditionFailed {
                operation: "transfer".to_owned(),
                detail: format!(
                    "{} is already in {} on {}",
                    request.employee, request.to_department, request.effective_on
                ),
            });
        }

        let revision = crate::revision::next_revision_in(&mut tx, &request.scope).await?;
        let from_department = current.clone();

        // Opening the new department closes whatever covered that date: the
        // write path cuts the overlap out of the old version, so the old
        // department keeps its own start and simply ends on the effective date.
        set_role_at(
            &mut tx,
            &SetRole {
                scope: request.scope.clone(),
                relation_type: "membership".to_owned(),
                relation_id: membership,
                role: "department".to_owned(),
                participant_id: request.to_department.clone(),
                valid_time: ValidTime::unbounded_from(request.effective_on),
            },
            revision,
        )
        .await?;

        // The previous department is part of the recorded result so a
        // cancellation can restore it without re-deriving history.
        let result = json!({
            "employee": request.employee,
            "from_department": from_department,
            "to_department": request.to_department,
            "effective_on": request.effective_on,
        });
        record_operation_in(
            &mut tx,
            &key,
            &input,
            revision,
            &request.actor,
            &request.reason,
            &result,
        )
        .await?;
        crate::outbox::enqueue_in(
            &mut tx,
            &request.scope,
            revision,
            "transfer.applied",
            &result,
        )
        .await?;

        tx.commit().await?;
        Ok(OperationOutcome {
            revision,
            applied: true,
            result,
        })
    }
}

impl PgStore {
    /// Finds the membership that connects the employee to any department.
    async fn require_membership(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &Transfer,
    ) -> Result<String, PgError> {
        let employee: Option<String> = sqlx::query_scalar(
            "SELECT entity_type FROM entities WHERE scope_id = $1 AND id = $2 LIMIT 1",
        )
        .bind(&request.scope)
        .bind(&request.employee)
        .fetch_optional(&mut **tx)
        .await?;
        if employee.is_none() {
            return Err(PgError::PreconditionFailed {
                operation: "transfer".to_owned(),
                detail: format!("employee {} does not exist", request.employee),
            });
        }

        let department: Option<String> = sqlx::query_scalar(
            "SELECT entity_type FROM entities WHERE scope_id = $1 AND id = $2 LIMIT 1",
        )
        .bind(&request.scope)
        .bind(&request.to_department)
        .fetch_optional(&mut **tx)
        .await?;
        if department.is_none() {
            return Err(PgError::PreconditionFailed {
                operation: "transfer".to_owned(),
                detail: format!("department {} does not exist", request.to_department),
            });
        }

        // A membership created through the P0 API has its roles in the
        // current-state tables; one created through the P1 API has them in the
        // versioned tables. Either way the employee is linked to a membership.
        let membership: Option<String> = sqlx::query_scalar(
            "SELECT relation_id FROM relation_role_versions \
             WHERE scope_id = $1 AND role = 'employee' AND participant_id = $2 \
             UNION ALL \
             SELECT relation_id FROM relation_roles \
             WHERE scope_id = $1 AND role = 'employee' AND participant_id = $2 \
             LIMIT 1",
        )
        .bind(&request.scope)
        .bind(&request.employee)
        .fetch_optional(&mut **tx)
        .await?;

        membership.ok_or_else(|| PgError::PreconditionFailed {
            operation: "transfer".to_owned(),
            detail: format!("{} has no membership", request.employee),
        })
    }

    /// The department the employee is in on the effective date, if any.
    async fn current_department(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &Transfer,
        membership: &str,
    ) -> Result<Option<String>, PgError> {
        let rows: Vec<StoredRoleVersion> = sqlx::query_as(
            "SELECT valid_from, valid_until, recorded_from, recorded_to, participant_id \
             FROM relation_role_versions \
             WHERE scope_id = $1 AND relation_type = 'membership' AND relation_id = $2 \
               AND role = 'department' AND recorded_to IS NULL \
             UNION ALL \
             SELECT r.valid_from, r.valid_until, 0::bigint, NULL::bigint, rr.participant_id \
             FROM relation_roles rr \
             JOIN relations r \
               ON r.scope_id = rr.scope_id AND r.relation_type = rr.relation_type \
              AND r.id = rr.relation_id \
             WHERE rr.scope_id = $1 AND rr.relation_type = 'membership' \
               AND rr.relation_id = $2 AND rr.role = 'department' \
               AND NOT EXISTS ( \
                   SELECT 1 FROM relation_role_versions v \
                   WHERE v.scope_id = rr.scope_id AND v.relation_type = rr.relation_type \
                     AND v.relation_id = rr.relation_id AND v.role = rr.role )",
        )
        .bind(&request.scope)
        .bind(membership)
        .fetch_all(&mut **tx)
        .await?;

        let versions: Vec<tokiori_model::FactVersion<String>> = rows
            .into_iter()
            .map(|(from, until, recorded_from, recorded_to, value)| {
                let valid = match until {
                    None => ValidTime::unbounded_from(from),
                    Some(until) => ValidTime::bounded(from, until)
                        .expect("stored intervals are non-empty by constraint"),
                };
                tokiori_model::FactVersion {
                    valid,
                    recorded_from: u64::try_from(recorded_from).expect("positive"),
                    recorded_to: recorded_to.map(|to| u64::try_from(to).expect("positive")),
                    value,
                }
            })
            .collect();

        let latest = versions.iter().map(|v| v.recorded_from).max().unwrap_or(1);
        Ok(
            match tokiori_model::resolve_cloned(&versions, request.effective_on, latest) {
                tokiori_model::Resolution::Found(value) => Some(value),
                _ => None,
            },
        )
    }
}
