//! Unit tests for the four `authorize` rules and the denial codes.

use tokiori_server::{ApiToken, AuthDecision, DenialReason, authorize};

fn token(scope: &str) -> ApiToken {
    ApiToken {
        scope: scope.to_owned(),
        name: "ci".to_owned(),
    }
}

/// Panics unless `decision` is a denial.
fn denied_reason(decision: AuthDecision) -> DenialReason {
    let AuthDecision::Denied { reason } = decision else {
        panic!("expected a denial, got {decision:?}");
    };
    reason
}

#[test]
fn missing_token_is_denied_with_missing_token() {
    let reason = denied_reason(authorize(None, Some("a")));
    assert_eq!(reason, DenialReason::MissingToken);
    assert_eq!(reason.code(), "missing_token");
}

#[test]
fn missing_requested_scope_is_denied_with_missing_scope() {
    let reason = denied_reason(authorize(Some(&token("a")), None));
    assert_eq!(reason, DenialReason::MissingScope);
    assert_eq!(reason.code(), "missing_scope");
}

#[test]
fn different_scope_is_denied_with_scope_mismatch() {
    let reason = denied_reason(authorize(Some(&token("a")), Some("b")));
    assert_eq!(
        reason,
        DenialReason::ScopeMismatch {
            token_scope: "a".to_owned(),
            requested: "b".to_owned(),
        }
    );
    assert_eq!(reason.code(), "scope_mismatch");
}

#[test]
fn matching_scope_is_allowed_with_the_token_scope() {
    let decision = authorize(Some(&token("a")), Some("a"));
    assert_eq!(
        decision,
        AuthDecision::Allowed {
            scope: "a".to_owned()
        }
    );
}

#[test]
fn empty_scopes_still_match() {
    let decision = authorize(Some(&token("")), Some(""));
    assert_eq!(
        decision,
        AuthDecision::Allowed {
            scope: String::new()
        }
    );
}
