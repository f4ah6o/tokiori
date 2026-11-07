//! HTTP API server for Tokiori.
//!
//! Authorization lives in [`authorize`], a pure function from a token and a
//! requested scope to an [`AuthDecision`]. Handlers call it and act on the
//! decision; the checks are never re-implemented per route.

mod authorize;

pub use authorize::{ApiToken, AuthDecision, DenialReason, authorize};

/// Routes served by the HTTP API. Only the health probe exists so far; more
/// endpoints come later.
pub fn router() -> axum::Router {
    axum::Router::new().route("/health", axum::routing::get(health))
}

/// Axum handlers are async by contract, even when they never await.
#[allow(clippy::unused_async)]
async fn health() -> &'static str {
    "ok"
}
