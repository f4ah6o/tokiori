//! The pure authorization boundary for the HTTP API.
//!
//! `authorize` maps a token and a requested scope to a decision. It touches no
//! database, clock, or network, so every rule is unit-testable and
//! property-testable in isolation.

/// A bearer token: who it is for, and the one scope it grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    /// The scope the token grants.
    pub scope: String,
    /// Human-readable token name, for logs and listing.
    pub name: String,
}

/// Why a request was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenialReason {
    /// No token was presented.
    MissingToken,
    /// The token's scope does not match the requested scope.
    ScopeMismatch {
        /// Scope held by the token.
        token_scope: String,
        /// Scope the action asked for.
        requested: String,
    },
    /// The action did not declare which scope it needs.
    MissingScope,
}

impl DenialReason {
    /// Short machine-readable identifier for the denial, stable across versions.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingToken => "missing_token",
            Self::ScopeMismatch { .. } => "scope_mismatch",
            Self::MissingScope => "missing_scope",
        }
    }
}

/// The outcome of authorizing one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// The action may run.
    Allowed {
        /// The scope the action runs under (the token's scope).
        scope: String,
    },
    /// The action may not run.
    Denied {
        /// Why it was denied.
        reason: DenialReason,
    },
}

/// Decide whether `token` may run an action needing `requested_scope`.
///
/// The rules, in the order they are checked:
///
/// 1. no token -> [`DenialReason::MissingToken`]
/// 2. no requested scope -> [`DenialReason::MissingScope`]
/// 3. token scope different from requested scope -> [`DenialReason::ScopeMismatch`]
/// 4. otherwise allowed, under the token's scope
#[must_use]
pub fn authorize(token: Option<&ApiToken>, requested_scope: Option<&str>) -> AuthDecision {
    let Some(token) = token else {
        return AuthDecision::Denied {
            reason: DenialReason::MissingToken,
        };
    };
    let Some(requested) = requested_scope else {
        return AuthDecision::Denied {
            reason: DenialReason::MissingScope,
        };
    };
    if token.scope != requested {
        return AuthDecision::Denied {
            reason: DenialReason::ScopeMismatch {
                token_scope: token.scope.clone(),
                requested: requested.to_owned(),
            },
        };
    }
    AuthDecision::Allowed {
        scope: token.scope.clone(),
    }
}
