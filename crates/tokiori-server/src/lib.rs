//! HTTP API server for Tokiori.
//!
//! Authorization lives in [`authorize`], a pure function from a token and a
//! requested scope to an [`AuthDecision`]. Handlers call it and act on the
//! decision; the checks are never re-implemented per route.

mod authorize;
mod error;
mod state;
mod tokens;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use tokiori_model::{Query as QueryAst, canonical_json};
use tokiori_pg::{CancelTransfer, OperationOutcome, OrgChart, ReadAttribute, ReadRole, Transfer};
use uuid::Uuid;

pub use authorize::{ApiToken, AuthDecision, DenialReason, authorize};
pub use error::ApiError;
pub use state::AppState;
pub use tokens::parse_tokens;

/// Routes served by the HTTP API.
///
/// Every route requires `Authorization: Bearer <token>` except `/health`, and
/// each token grants exactly one scope; the routes read and, for the operation
/// and outbox-ack routes, write in that scope.
pub fn router(state: AppState) -> axum::Router {
    axum::Router::new()
        .route("/health", get(health))
        .route("/v1/model", get(get_model))
        .route("/v1/entities/{entity_type}/{id}", get(get_entity))
        .route(
            "/v1/attributes/{entity_type}/{id}/{attribute}",
            get(get_attribute),
        )
        .route(
            "/v1/roles/{relation_type}/{relation_id}/{role}",
            get(get_role),
        )
        .route("/v1/query", post(post_query))
        .route("/v1/org-chart", get(get_org_chart))
        .route("/v1/operations/transfer", post(post_transfer))
        .route("/v1/operations/cancel", post(post_cancel))
        .route("/v1/outbox", get(get_outbox))
        .route("/v1/outbox/ack", post(post_outbox_ack))
        .with_state(state)
}

/// Axum handlers are async by contract, even when they never await.
#[allow(clippy::unused_async)]
async fn health() -> &'static str {
    "ok"
}

/// The latest model in the token's scope.
async fn get_model(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let model = state.store.latest_model(&token.scope).await?;
    let document: Value =
        serde_json::from_str(&canonical_json(&model)).expect("canonical model JSON is valid JSON");
    Ok(Json(json!({ "scope": token.scope, "model": document })))
}

/// One entity row, or 404.
///
/// The row is current state, so there is no replayable `valid_at` or
/// `known_revision` here.
async fn get_entity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((entity_type, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let Some(row) = state
        .store
        .get_entity(&token.scope, &entity_type, &id)
        .await?
    else {
        return Err(ApiError::not_found(format!("{entity_type} {id}")));
    };
    Ok(Json(json!({
        "scope": token.scope,
        "entity": {
            "entity_type": row.entity_type,
            "id": row.id,
            "model": row.model,
            "model_version": row.model_version,
            "attributes": row.attributes,
        },
    })))
}

/// The query string both temporal reads take.
#[derive(Debug, Deserialize)]
struct ReadQuery {
    /// The business date to ask about, `YYYY-MM-DD`. Required.
    valid_at: Option<String>,
    /// The revision to answer as of; absent means the newest.
    known_revision: Option<String>,
}

/// Parses the temporal read query; `valid_at` is required.
fn parse_read_query(query: &ReadQuery) -> Result<(NaiveDate, Option<u64>), ApiError> {
    let Some(raw) = query.valid_at.as_deref() else {
        return Err(ApiError::invalid_query("valid_at is required"));
    };
    let valid_at = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|err| ApiError::invalid_query(format!("valid_at must be YYYY-MM-DD: {err}")))?;
    let known_revision = query
        .known_revision
        .as_deref()
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|err| {
            ApiError::invalid_query(format!("known_revision must be an integer: {err}"))
        })?;
    Ok((valid_at, known_revision))
}

/// One attribute as of a date and revision.
///
/// `value` is `null` when the fact did not hold; `known_revision` names the
/// revision the answer was resolved at, so the exact query can be replayed.
async fn get_attribute(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((entity_type, id, attribute)): Path<(String, String, String)>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let (valid_at, known_revision) = parse_read_query(&query)?;
    let answer = state
        .store
        .read_attribute(ReadAttribute {
            scope: token.scope.clone(),
            entity_type: entity_type.clone(),
            entity_id: id.clone(),
            attribute: attribute.clone(),
            valid_at,
            known_revision,
        })
        .await?;
    Ok(Json(json!({
        "scope": token.scope,
        "entity_type": entity_type,
        "id": id,
        "attribute": attribute,
        "value": answer.value,
        "known_revision": answer.known_revision,
        "valid_at": answer.valid_at.to_string(),
    })))
}

/// One role's participant as of a date and revision.
///
/// `participant_id` is `null` when the role was empty; `known_revision` and
/// `valid_at` mirror the attribute read so the query can be replayed.
async fn get_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((relation_type, relation_id, role)): Path<(String, String, String)>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let (valid_at, known_revision) = parse_read_query(&query)?;
    let answer = state
        .store
        .read_role(ReadRole {
            scope: token.scope.clone(),
            relation_type: relation_type.clone(),
            relation_id: relation_id.clone(),
            role: role.clone(),
            valid_at,
            known_revision,
        })
        .await?;
    Ok(Json(json!({
        "scope": token.scope,
        "relation_type": relation_type,
        "relation_id": relation_id,
        "role": role,
        "participant_id": answer.participant_id,
        "known_revision": answer.known_revision,
        "valid_at": answer.valid_at.to_string(),
    })))
}

/// The rows a query reached, in the token's scope.
///
/// The scope comes from the token, never from the body. A body that is not a
/// `Query` at all is 400 `invalid_request`; a body that is one but names
/// something the model does not declare is 400 `invalid_query`, mapped once in
/// the store's error conversion.
async fn post_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let query: QueryAst = serde_json::from_value(body).map_err(ApiError::invalid_request)?;
    let result = state.store.query(&token.scope, &query).await?;
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|row| json!({ "id": row.id, "value": row.value }))
        .collect();
    Ok(Json(json!({
        "scope": token.scope,
        "rows": rows,
        "truncated": result.truncated,
        "known_revision": result.known_revision,
    })))
}

/// Every employee and the department they were in, as of a date and revision.
///
/// The query string is the one the other temporal reads take: `valid_at` is
/// required, `known_revision` is optional and answered back as it was resolved,
/// so the exact chart a caller saw can be asked for again.
async fn get_org_chart(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let (valid_at, known_revision) = parse_read_query(&query)?;
    let resolved_revision = if let Some(revision) = known_revision {
        Some(revision)
    } else {
        state.store.latest_revision(&token.scope).await?
    };
    let rows = state
        .store
        .org_chart(OrgChart {
            scope: token.scope.clone(),
            valid_at,
            known_revision,
        })
        .await?;
    Ok(Json(json!({
        "scope": token.scope,
        "valid_at": valid_at.to_string(),
        "known_revision": resolved_revision,
        "rows": rows,
    })))
}

/// The body of `POST /v1/operations/transfer`.
///
/// The scope is never in the body: it comes from the token, so a caller can
/// never name another scope's employee or department.
#[derive(Debug, Deserialize)]
struct TransferBody {
    employee: String,
    to_department: String,
    /// The business date the move takes effect, `YYYY-MM-DD`.
    effective_on: String,
    /// The caller's request id; the idempotency key for this operation.
    request_id: String,
    reason: String,
    actor: String,
}

/// The body of `POST /v1/operations/cancel`. The scope comes from the token.
#[derive(Debug, Deserialize)]
struct CancelBody {
    /// The `request_id` of the transfer being undone.
    transfer_request_id: String,
    /// The caller's request id for the cancellation itself.
    cancel_request_id: String,
    reason: String,
    actor: String,
}

/// The body of `POST /v1/outbox/ack`.
#[derive(Debug, Deserialize)]
struct AckBody {
    event_ids: Vec<Uuid>,
}

/// The response shape both operation endpoints take: it mirrors
/// [`OperationOutcome`] and names the scope it acted in.
fn operation_outcome_json(scope: &str, outcome: &OperationOutcome) -> Value {
    json!({
        "scope": scope,
        "applied": outcome.applied,
        "revision": outcome.revision,
        "result": outcome.result,
    })
}

/// Applies a transfer, or recognises a replay of one.
///
/// A repeated `request_id` with the same input answers `applied: false` with
/// the original revision; with different input it is 409
/// `idempotency_conflict`. A transfer the store rejects on a missing employee,
/// department, or membership is 409 `precondition_failed`.
async fn post_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let body: TransferBody = serde_json::from_value(body).map_err(ApiError::invalid_request)?;
    let effective_on =
        NaiveDate::parse_from_str(&body.effective_on, "%Y-%m-%d").map_err(|err| {
            ApiError::invalid_request(format!("effective_on must be YYYY-MM-DD: {err}"))
        })?;
    let outcome = state
        .store
        .transfer(Transfer {
            scope: token.scope.clone(),
            employee: body.employee,
            to_department: body.to_department,
            effective_on,
            request_id: body.request_id,
            reason: body.reason,
            actor: body.actor,
        })
        .await?;
    Ok(Json(operation_outcome_json(&token.scope, &outcome)))
}

/// Undoes a transfer by recording the previous department again.
///
/// The response shape is the transfer's: a replayed cancellation answers
/// `applied: false`, and a transfer that was already cancelled is 409 with the
/// store's code.
async fn post_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let body: CancelBody = serde_json::from_value(body).map_err(ApiError::invalid_request)?;
    let outcome = state
        .store
        .cancel_transfer(CancelTransfer {
            scope: token.scope.clone(),
            transfer_request_id: body.transfer_request_id,
            cancel_request_id: body.cancel_request_id,
            reason: body.reason,
            actor: body.actor,
        })
        .await?;
    Ok(Json(operation_outcome_json(&token.scope, &outcome)))
}

/// The `limit` query parameter of the outbox claim.
#[derive(Debug, Deserialize)]
struct OutboxQuery {
    limit: Option<String>,
}

/// Events claimed when the query omits `limit`.
const DEFAULT_OUTBOX_LIMIT: i64 = 100;
/// One claim never takes more than this many events.
const MAX_OUTBOX_LIMIT: i64 = 1000;

/// Parses and bounds the outbox claim's `limit`.
///
/// Above [`MAX_OUTBOX_LIMIT`] the request is clamped, not rejected, so a
/// caller that asks for "everything" gets a page instead of an error.
fn parse_outbox_limit(raw: Option<&str>) -> Result<i64, ApiError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_OUTBOX_LIMIT);
    };
    let parsed = raw
        .parse::<i64>()
        .map_err(|err| ApiError::invalid_query(format!("limit must be an integer: {err}")))?;
    if parsed < 0 {
        return Err(ApiError::invalid_query("limit must not be negative"));
    }
    Ok(parsed.min(MAX_OUTBOX_LIMIT))
}

/// Claims up to `limit` pending events for delivery.
///
/// Claiming is not consuming: an event stays pending until it is
/// acknowledged, so a consumer that crashes after a claim sees it again.
async fn get_outbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<OutboxQuery>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let limit = parse_outbox_limit(query.limit.as_deref())?;
    let events = state.store.claim_outbox(&token.scope, limit).await?;
    let events: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "event_id": event.event_id,
                "revision": event.revision,
                "event_type": event.event_type,
                "payload": event.payload,
                "created_at": event.created_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(Json(json!({ "scope": token.scope, "events": events })))
}

/// Acknowledges claimed events so they are never claimed again.
///
/// Acknowledging an already-acknowledged event is a no-op, so a consumer that
/// retries after a crash does not fail.
async fn post_outbox_ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let token = state.authenticate(&headers)?;
    let body: AckBody = serde_json::from_value(body).map_err(ApiError::invalid_request)?;
    let acknowledged = state
        .store
        .ack_outbox(&token.scope, &body.event_ids)
        .await?;
    Ok(Json(json!({
        "scope": token.scope,
        "acknowledged": acknowledged,
    })))
}
