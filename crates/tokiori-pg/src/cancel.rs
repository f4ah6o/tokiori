//! Cancel: undoing a transfer as a new operation.
//!
//! A cancellation does not delete the transfer. It records a compensating
//! change, so the history shows both what was asked and what undid it.

use serde_json::{Value, json};
use tokiori_model::ValidTime;

use crate::error::PgError;
use crate::operations::{OperationKey, OperationRecord, input_digest, record_operation_in};
use crate::roles::{SetRole, set_role_at};
use crate::store::PgStore;
use crate::transfer::OperationOutcome;

/// A request to cancel a transfer.
#[derive(Debug, Clone)]
pub struct CancelTransfer {
    pub scope: String,
    /// The `request_id` of the transfer being undone.
    pub transfer_request_id: String,
    /// The caller's request id for the cancellation itself.
    pub cancel_request_id: String,
    pub reason: String,
    pub actor: String,
}

impl CancelTransfer {
    fn key(&self) -> OperationKey {
        OperationKey {
            scope: self.scope.clone(),
            operation_type: "cancel_transfer".to_owned(),
            idempotency_key: self.cancel_request_id.clone(),
        }
    }

    fn input(&self) -> Value {
        json!({ "transfer_request_id": self.transfer_request_id })
    }
}

/// What a cancellation restores, read from the transfer's recorded result.
struct Restored {
    employee: String,
    from_department: String,
    effective_on: chrono::NaiveDate,
}

impl Restored {
    fn from(transfer: &OperationRecord) -> Result<Self, PgError> {
        let text = |field: &str| {
            transfer.result[field]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| PgError::PreconditionFailed {
                    operation: "cancel_transfer".to_owned(),
                    detail: format!("the transfer record has no {field}"),
                })
        };
        let effective_on =
            text("effective_on")?
                .parse()
                .map_err(|_| PgError::PreconditionFailed {
                    operation: "cancel_transfer".to_owned(),
                    detail: "the transfer record has an unreadable effective date".to_owned(),
                })?;
        Ok(Self {
            employee: text("employee")?,
            from_department: text("from_department")?,
            effective_on,
        })
    }
}

/// A transfer can only be undone once; a second cancellation is a different
/// request and must not silently double-compensate.
async fn ensure_not_already_cancelled(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &CancelTransfer,
) -> Result<(), PgError> {
    let already: Option<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM operations \
         WHERE scope_id = $1 AND operation_type = 'cancel_transfer' \
           AND result->>'transfer_request_id' = $2 LIMIT 1",
    )
    .bind(&request.scope)
    .bind(&request.transfer_request_id)
    .fetch_optional(&mut **tx)
    .await?;

    if already.is_some() {
        return Err(PgError::PreconditionFailed {
            operation: "cancel_transfer".to_owned(),
            detail: format!(
                "transfer {} was already cancelled",
                request.transfer_request_id
            ),
        });
    }
    Ok(())
}

/// Finds the membership that connects an employee to any department.
async fn find_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &str,
    employee: &str,
) -> Result<String, PgError> {
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT relation_id FROM relation_role_versions \
         WHERE scope_id = $1 AND role = 'employee' AND participant_id = $2 \
         UNION ALL \
         SELECT relation_id FROM relation_roles \
         WHERE scope_id = $1 AND role = 'employee' AND participant_id = $2 \
         LIMIT 1",
    )
    .bind(scope)
    .bind(employee)
    .fetch_optional(&mut **tx)
    .await?;

    membership.ok_or_else(|| PgError::PreconditionFailed {
        operation: "cancel_transfer".to_owned(),
        detail: format!("{employee} has no membership"),
    })
}

impl PgStore {
    /// Undoes a transfer by recording the previous department again.
    pub async fn cancel_transfer(
        &self,
        request: CancelTransfer,
    ) -> Result<OperationOutcome, PgError> {
        let mut tx = self.pool().begin().await?;
        let key = request.key();
        let input = request.input();

        if let Some(existing) = self.find_operation(&key).await? {
            if existing.input_digest == input_digest(&input) {
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
                incoming: input_digest(&input),
            });
        }

        let transfer_key = OperationKey {
            scope: request.scope.clone(),
            operation_type: "transfer".to_owned(),
            idempotency_key: request.transfer_request_id.clone(),
        };
        let Some(transfer) = self.find_operation(&transfer_key).await? else {
            return Err(PgError::OperationNotFound {
                operation_type: transfer_key.operation_type,
                idempotency_key: transfer_key.idempotency_key,
            });
        };

        ensure_not_already_cancelled(&mut tx, &request).await?;
        let restored = Restored::from(&transfer)?;
        let membership = find_membership(&mut tx, &request.scope, &restored.employee).await?;

        let revision = crate::revision::next_revision_in(&mut tx, &request.scope).await?;

        set_role_at(
            &mut tx,
            &SetRole {
                scope: request.scope.clone(),
                relation_type: "membership".to_owned(),
                relation_id: membership,
                role: "department".to_owned(),
                participant_id: restored.from_department.clone(),
                valid_time: ValidTime::unbounded_from(restored.effective_on),
            },
            revision,
        )
        .await?;

        let result = json!({
            "transfer_request_id": request.transfer_request_id,
            "employee": restored.employee,
            "restored_department": restored.from_department,
            "effective_on": restored.effective_on,
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

        tx.commit().await?;
        Ok(OperationOutcome {
            revision,
            applied: true,
            result,
        })
    }
}
