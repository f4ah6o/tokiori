//! The single error type every handler returns.
//!
//! The body is always `{"code": ..., "message": ...}` so a client can branch
//! on the code without parsing prose, and every mapping from a store error to
//! a status is defined exactly once, in the `From` impl below.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokiori_pg::PgError;

use crate::authorize::DenialReason;

/// An error a handler returns: an HTTP status, a stable code, and prose.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    /// Builds the response for one authorization denial.
    ///
    /// A missing token is an authentication failure (401), a scope mismatch is
    /// forbidden (403), and a missing scope declaration is the handler's own
    /// bug (400).
    pub fn denied(reason: DenialReason) -> Self {
        let code = reason.code();
        let (status, message) = match reason {
            DenialReason::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "a bearer token is required".to_owned(),
            ),
            DenialReason::ScopeMismatch {
                token_scope,
                requested,
            } => (
                StatusCode::FORBIDDEN,
                format!("token scope {token_scope:?} does not grant {requested:?}"),
            ),
            DenialReason::MissingScope => (
                StatusCode::BAD_REQUEST,
                "the action did not declare a required scope".to_owned(),
            ),
        };
        Self {
            status,
            code: code.to_owned(),
            message,
        }
    }

    /// A 404 for something the request named but the store does not have.
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found".to_owned(),
            message: format!("{what} does not exist"),
        }
    }

    /// A 400 for a query string that does not parse.
    pub fn invalid_query(what: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_query".to_owned(),
            message: format!("invalid query: {what}"),
        }
    }

    /// A 400 for a request body that does not parse as the expected type.
    pub fn invalid_request(what: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request".to_owned(),
            message: format!("invalid request: {what}"),
        }
    }
}

impl From<PgError> for ApiError {
    fn from(err: PgError) -> Self {
        let (status, code) = match &err {
            PgError::NotFound { .. }
            | PgError::ModelNotFound { .. }
            | PgError::OperationNotFound { .. } => (StatusCode::NOT_FOUND, "not_found"),
            PgError::UnknownRevision { .. } => (StatusCode::BAD_REQUEST, "unknown_revision"),
            PgError::InvalidQuery(_) => (StatusCode::BAD_REQUEST, "invalid_query"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, err.code()),
        };
        Self {
            status,
            code: code.to_owned(),
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "code": self.code,
            "message": self.message,
        }));
        (self.status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokiori_model::QueryError;

    #[test]
    fn not_found_errors_map_to_404_not_found() {
        for err in [
            PgError::NotFound {
                what: "entity 1".to_owned(),
            },
            PgError::ModelNotFound {
                scope: "s".to_owned(),
            },
            PgError::OperationNotFound {
                operation_type: "transfer".to_owned(),
                idempotency_key: "k".to_owned(),
            },
        ] {
            let api = ApiError::from(err);
            assert_eq!(api.status, StatusCode::NOT_FOUND);
            assert_eq!(api.code, "not_found");
        }
    }

    #[test]
    fn query_errors_map_to_400() {
        let revision = ApiError::from(PgError::UnknownRevision {
            scope: "s".to_owned(),
            revision: 3,
            latest: Some(2),
        });
        assert_eq!(revision.status, StatusCode::BAD_REQUEST);
        assert_eq!(revision.code, "unknown_revision");

        let query = ApiError::from(PgError::InvalidQuery(QueryError::UnknownEntityType {
            entity_type: "ghost".to_owned(),
        }));
        assert_eq!(query.status, StatusCode::BAD_REQUEST);
        assert_eq!(query.code, "invalid_query");
    }

    #[test]
    fn other_errors_map_to_500_with_the_pg_code() {
        let api = ApiError::from(PgError::EmptyInterval {
            from: "2027-01-02".to_owned(),
            until: "2027-01-01".to_owned(),
        });
        assert_eq!(api.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(api.code, "empty_interval");
    }

    #[test]
    fn denial_reasons_map_to_their_status() {
        let missing = ApiError::denied(DenialReason::MissingToken);
        assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
        assert_eq!(missing.code, "missing_token");

        let mismatch = ApiError::denied(DenialReason::ScopeMismatch {
            token_scope: "assets".to_owned(),
            requested: "organization".to_owned(),
        });
        assert_eq!(mismatch.status, StatusCode::FORBIDDEN);
        assert_eq!(mismatch.code, "scope_mismatch");

        let no_scope = ApiError::denied(DenialReason::MissingScope);
        assert_eq!(no_scope.status, StatusCode::BAD_REQUEST);
        assert_eq!(no_scope.code, "missing_scope");
    }
}
