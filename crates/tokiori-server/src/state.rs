//! The shared state handlers draw from: the store and the parsed tokens.

use axum::http::{HeaderMap, header::AUTHORIZATION};
use tokiori_pg::PgStore;

use crate::authorize::{ApiToken, AuthDecision, DenialReason, authorize};
use crate::error::ApiError;

/// State shared by every route.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The persistence layer, wired to one database.
    pub store: PgStore,
    /// The tokens parsed from `TOKIORI_API_TOKENS`.
    pub tokens: Vec<ApiToken>,
}

impl AppState {
    /// Authenticates a request from its headers and returns the token to act
    /// under.
    ///
    /// The client presents a token entry's name as the bearer value, and the
    /// entry's scope is the grant. The decision always goes through
    /// [`authorize`], so the rules stay in one place, even though a token here
    /// always grants its own scope and the mismatch branch is unreachable.
    /// An unknown token is indistinguishable from an absent one.
    pub fn authenticate(&self, headers: &HeaderMap) -> Result<ApiToken, ApiError> {
        let presented = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::denied(DenialReason::MissingToken))?;
        let token = self
            .tokens
            .iter()
            .find(|candidate| candidate.name == presented)
            .ok_or_else(|| ApiError::denied(DenialReason::MissingToken))?;
        match authorize(Some(token), Some(&token.scope)) {
            AuthDecision::Allowed { .. } => Ok(token.clone()),
            AuthDecision::Denied { reason } => Err(ApiError::denied(reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use sqlx::PgPool;

    /// A state over a lazy pool: no query runs in these tests, so no database
    /// is contacted.
    fn state(tokens: Vec<ApiToken>) -> AppState {
        let pool = PgPool::connect_lazy("postgres://localhost/tokiori-unused")
            .expect("lazy pool never contacts the database");
        AppState {
            store: PgStore::new(pool),
            tokens,
        }
    }

    fn token(name: &str, scope: &str) -> ApiToken {
        ApiToken {
            name: name.to_owned(),
            scope: scope.to_owned(),
        }
    }

    #[tokio::test]
    async fn absent_header_is_missing_token() {
        let state = state(vec![token("ci", "organization")]);
        let err = state
            .authenticate(&HeaderMap::new())
            .expect_err("no header");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.code, "missing_token");
    }

    #[tokio::test]
    async fn unknown_token_is_missing_token() {
        let state = state(vec![token("ci", "organization")]);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer nobody".parse().expect("valid value"));
        let err = state.authenticate(&headers).expect_err("unknown token");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.code, "missing_token");
    }

    #[tokio::test]
    async fn wrong_scheme_is_missing_token() {
        let state = state(vec![token("ci", "organization")]);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Basic ci".parse().expect("valid value"));
        let err = state.authenticate(&headers).expect_err("wrong scheme");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.code, "missing_token");
    }

    #[tokio::test]
    async fn known_token_is_allowed_under_its_own_scope() {
        let state = state(vec![token("ci", "organization")]);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer ci".parse().expect("valid value"));
        let token = state.authenticate(&headers).expect("known token");
        assert_eq!(token.scope, "organization");
    }
}
