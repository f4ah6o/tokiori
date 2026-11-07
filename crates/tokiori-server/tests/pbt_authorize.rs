//! Property test: `authorize` allows a request exactly when the requested
//! scope equals the token's scope.
//!
//! Scopes are sampled from a fixed list that includes the empty scope, so the
//! boundary between "a scope" and "no requested scope" stays visible.

use tokiori_server::{ApiToken, AuthDecision, DenialReason, authorize};

const SCOPES: [&str; 4] = ["", "a", "b", "s3"];

#[test]
fn allowed_iff_requested_scope_equals_token_scope() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let mut runner = noprop::Runner::new(seed);

    let outcome = runner.run(1000, |ctx| {
        let token_scope = noprop::sample_choice(ctx, &SCOPES);
        let requested = noprop::sample_choice(ctx, &SCOPES);

        let token = ApiToken {
            scope: token_scope.to_owned(),
            name: "pbt".to_owned(),
        };
        let decision = authorize(Some(&token), Some(requested));

        if requested == token_scope {
            assert_eq!(
                decision,
                AuthDecision::Allowed {
                    scope: token_scope.to_owned(),
                },
                "equal scopes must be allowed: token {token_scope:?}, requested {requested:?}"
            );
        } else {
            assert_eq!(
                decision,
                AuthDecision::Denied {
                    reason: DenialReason::ScopeMismatch {
                        token_scope: token_scope.to_owned(),
                        requested: requested.to_owned(),
                    },
                },
                "different scopes must be denied: token {token_scope:?}, requested {requested:?}"
            );
        }
        Ok(())
    });
    outcome?;
    Ok(())
}
