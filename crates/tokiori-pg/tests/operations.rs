//! The operation ledger: what was asked, and what was decided.

mod common;

use common::fixture;
use tokiori_pg::OperationKey;

fn key(scope: &str, kind: &str, id: &str) -> OperationKey {
    OperationKey {
        scope: scope.to_owned(),
        operation_type: kind.to_owned(),
        idempotency_key: id.to_owned(),
    }
}

#[tokio::test]
async fn recording_an_operation_stores_its_input_digest() {
    let Some(fx) = fixture().await else { return };

    let digest = fx
        .store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &serde_json::json!({ "employee": "E001", "to": "D-PLANNING" }),
            1,
            "alice",
            "annual rotation",
            &serde_json::json!({ "ok": true }),
        )
        .await
        .expect("record");

    assert!(digest.starts_with("sha256:"), "{digest}");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_same_key_and_input_is_recognised_as_a_replay() {
    let Some(fx) = fixture().await else { return };
    let input = serde_json::json!({ "employee": "E001", "to": "D-PLANNING" });

    let first = fx
        .store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &input,
            1,
            "alice",
            "rotation",
            &serde_json::json!({ "ok": true }),
        )
        .await
        .expect("first");

    let err = fx
        .store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &input,
            2,
            "alice",
            "rotation",
            &serde_json::json!({ "ok": true }),
        )
        .await
        .expect_err("a replay must not record a second operation");

    assert_eq!(err.code(), "operation_replayed", "{err}");
    assert_eq!(first.len(), 71, "sha256 hex with prefix");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_same_key_with_a_different_input_is_a_conflict() {
    let Some(fx) = fixture().await else { return };

    fx.store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &serde_json::json!({ "to": "D-PLANNING" }),
            1,
            "alice",
            "rotation",
            &serde_json::json!({}),
        )
        .await
        .expect("first");

    let err = fx
        .store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &serde_json::json!({ "to": "D-SALES" }),
            2,
            "alice",
            "rotation",
            &serde_json::json!({}),
        )
        .await
        .expect_err("different input under the same key");

    assert_eq!(err.code(), "idempotency_conflict", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn keys_are_namespaced_by_operation_type() {
    let Some(fx) = fixture().await else { return };
    let input = serde_json::json!({ "x": 1 });

    fx.store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &input,
            1,
            "alice",
            "r",
            &serde_json::json!({}),
        )
        .await
        .expect("transfer");

    // The same key under a different operation type is a different request.
    fx.store
        .record_operation(
            key(&fx.scope, "cancel", "req-1"),
            &input,
            2,
            "alice",
            "r",
            &serde_json::json!({}),
        )
        .await
        .expect("cancel with the same key is independent");

    fx.cleanup().await;
}

#[tokio::test]
async fn keys_are_namespaced_by_scope() {
    let Some(fx) = fixture().await else { return };
    let other = format!("{}-other", fx.scope);
    let input = serde_json::json!({ "x": 1 });

    fx.store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &input,
            1,
            "alice",
            "r",
            &serde_json::json!({}),
        )
        .await
        .expect("scope a");
    fx.store
        .record_operation(
            key(&other, "transfer", "req-1"),
            &input,
            1,
            "bob",
            "r",
            &serde_json::json!({}),
        )
        .await
        .expect("scope b is independent");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_recorded_operation_can_be_read_back() {
    let Some(fx) = fixture().await else { return };

    fx.store
        .record_operation(
            key(&fx.scope, "transfer", "req-1"),
            &serde_json::json!({ "to": "D-PLANNING" }),
            7,
            "alice",
            "rotation",
            &serde_json::json!({ "ok": true }),
        )
        .await
        .expect("record");

    let found = fx
        .store
        .find_operation(&key(&fx.scope, "transfer", "req-1"))
        .await
        .expect("read")
        .expect("exists");

    assert_eq!(found.revision, 7);
    assert_eq!(found.actor, "alice");
    assert_eq!(found.reason, "rotation");
    assert_eq!(found.result, serde_json::json!({ "ok": true }));

    fx.cleanup().await;
}
