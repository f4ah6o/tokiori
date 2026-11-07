//! ORG-005: a request replayed is applied once, and a conflicting one is refused.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, OperationKey, PgStore, ReadRole, SetRole, Transfer};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

async fn seeded() -> Option<common::Fixture> {
    let fx = fixture().await?;
    fx.store
        .apply_model(&fx.scope, &organization_model())
        .await
        .expect("apply model");
    for (entity_type, id) in [
        ("employee", "E001"),
        ("department", "D-SALES"),
        ("department", "D-PLANNING"),
    ] {
        fx.store
            .create_entity(NewEntity {
                scope: fx.scope.clone(),
                entity_type: entity_type.to_owned(),
                id: id.to_owned(),
                attributes: common::attributes(&[("name", id)]),
            })
            .await
            .expect("entity");
    }
    fx.store
        .create_relation(NewRelation {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            id: "M001".to_owned(),
            roles: std::collections::BTreeMap::from([
                ("employee".to_owned(), "E001".to_owned()),
                ("department".to_owned(), "D-SALES".to_owned()),
            ]),
            attributes: serde_json::Map::new(),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("membership");
    fx.store
        .set_role(SetRole {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            participant_id: "D-SALES".to_owned(),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("baseline");
    Some(fx)
}

fn transfer(fx: &common::Fixture, key: &str) -> Transfer {
    Transfer {
        scope: fx.scope.clone(),
        employee: "E001".to_owned(),
        to_department: "D-PLANNING".to_owned(),
        effective_on: day(2027, 7, 1),
        request_id: key.to_owned(),
        reason: "rotation".to_owned(),
        actor: "alice".to_owned(),
    }
}

async fn department_at(store: &PgStore, scope: &str, on: NaiveDate) -> Option<String> {
    store
        .read_role(ReadRole {
            scope: scope.to_owned(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            valid_at: on,
            known_revision: None,
        })
        .await
        .expect("read")
        .participant_id
}

async fn role_version_count(store: &PgStore) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM relation_role_versions")
        .fetch_one(store.pool())
        .await
        .expect("count")
}

#[tokio::test]
async fn the_same_request_twice_applies_once_and_returns_the_same_result() {
    let Some(fx) = seeded().await else { return };

    let before = role_version_count(&fx.store).await;
    let first = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("first");
    let second = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("replay");

    assert!(first.applied);
    assert!(!second.applied, "the replay must not apply again");
    assert_eq!(first.revision, second.revision);
    assert_eq!(first.result, second.result);
    // One transfer opens one new version; the replay opens none.
    assert_eq!(
        role_version_count(&fx.store).await - before,
        1,
        "the replay added no version"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_same_key_with_a_different_target_is_a_conflict_and_changes_nothing() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("first");
    let before = role_version_count(&fx.store).await;

    let mut conflicting = transfer(&fx, "req-1");
    conflicting.to_department = "D-SALES".to_owned();
    conflicting.effective_on = day(2027, 8, 1);

    let err = fx
        .store
        .transfer(conflicting)
        .await
        .expect_err("same key, different input");
    assert_eq!(err.code(), "idempotency_conflict", "{err}");
    assert_eq!(
        role_version_count(&fx.store).await,
        before,
        "no history grew"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn concurrent_replays_apply_the_transfer_once() {
    let Some(fx) = seeded().await else { return };
    let store = fx.store.clone();
    let scope = fx.scope.clone();

    let before = role_version_count(&fx.store).await;
    let mut handles = Vec::new();
    for _ in 0..3 {
        let store = store.clone();
        let scope = scope.clone();
        handles.push(tokio::spawn(async move {
            store
                .transfer(Transfer {
                    scope,
                    employee: "E001".to_owned(),
                    to_department: "D-PLANNING".to_owned(),
                    effective_on: day(2027, 7, 1),
                    request_id: "req-1".to_owned(),
                    reason: "rotation".to_owned(),
                    actor: "alice".to_owned(),
                })
                .await
        }));
    }

    let mut applied = 0;
    let mut revisions = Vec::new();
    for handle in handles {
        match handle.await.expect("join") {
            Ok(outcome) => {
                if outcome.applied {
                    applied += 1;
                }
                revisions.push(outcome.revision);
            }
            // A concurrent writer may lose the race and be told to retry.
            Err(err) if err.code() == "operation_replayed" => {}
            Err(err) => panic!("unexpected error: {err}"),
        }
    }

    assert_eq!(applied, 1, "exactly one call applied the transfer");
    assert_eq!(role_version_count(&fx.store).await - before, 1);
    let _ = revisions;

    fx.cleanup().await;
}

#[tokio::test]
async fn a_rejected_precondition_records_nothing() {
    let Some(fx) = seeded().await else { return };
    let before = role_version_count(&fx.store).await;
    let mut request = transfer(&fx, "req-1");
    request.employee = "E999".to_owned();

    fx.store
        .transfer(request)
        .await
        .expect_err("unknown employee");

    let key = OperationKey {
        scope: fx.scope.clone(),
        operation_type: "transfer".to_owned(),
        idempotency_key: "req-1".to_owned(),
    };
    assert!(fx.store.find_operation(&key).await.expect("read").is_none());
    assert_eq!(
        role_version_count(&fx.store).await,
        before,
        "no version was added"
    );
    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 7, 1))
            .await
            .as_deref(),
        Some("D-SALES")
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn a_different_request_is_a_different_operation() {
    let Some(fx) = seeded().await else { return };

    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("first");

    let mut second = transfer(&fx, "req-2");
    second.to_department = "D-SALES".to_owned();
    second.effective_on = day(2027, 9, 1);
    let outcome = fx.store.transfer(second).await.expect("second transfer");

    assert!(outcome.applied);
    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 9, 15))
            .await
            .as_deref(),
        Some("D-SALES")
    );

    fx.cleanup().await;
}
