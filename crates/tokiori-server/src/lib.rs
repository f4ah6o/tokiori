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
use tokiori_pg::{OrgChart, ReadAttribute, ReadRole};

pub use authorize::{ApiToken, AuthDecision, DenialReason, authorize};
pub use error::ApiError;
pub use state::AppState;
pub use tokens::parse_tokens;

/// Routes served by the HTTP API.
///
/// Every route requires `Authorization: Bearer <token>` except `/health`, and
/// each token grants exactly one scope; the routes read in that scope.
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
