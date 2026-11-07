//! Fault injection: a failure mid-operation must leave nothing behind.
//!
//! The operation writes state, a ledger row, and an event. If any of those
//! fails, the others must not survive. These tests force the failure at each
//! step rather than trusting the transaction boundary by inspection.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, OperationKey, SetRole, Transfer};

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

async fn counts(fx: &common::Fixture) -> (i64, i64, i64) {
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM relation_role_versions")
        .fetch_one(fx.store.pool())
        .await
        .expect("roles");
    let operations: i64 = sqlx::query_scalar("SELECT count(*) FROM operations")
        .fetch_one(fx.store.pool())
        .await
        .expect("operations");
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
        .fetch_one(fx.store.pool())
        .await
        .expect("events");
    (roles, operations, events)
}

/// Makes the event insert fail for this scope, as a broken outbox would.
async fn break_outbox(fx: &common::Fixture) {
    sqlx::query(
        "CREATE FUNCTION fail_outbox() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'injected outbox failure'; END; $$ LANGUAGE plpgsql",
    )
    .execute(fx.store.pool())
    .await
    .expect("create function");
    sqlx::query(
        "CREATE TRIGGER fail_outbox BEFORE INSERT ON outbox_events \
         FOR EACH ROW EXECUTE FUNCTION fail_outbox()",
    )
    .execute(fx.store.pool())
    .await
    .expect("create trigger");
}

#[tokio::test]
async fn a_failure_writing_the_event_rolls_back_the_state_and_the_ledger() {
    let Some(fx) = seeded().await else { return };
    let before = counts(&fx).await;
    break_outbox(&fx).await;

    let err = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect_err("the injected failure must surface");
    assert_eq!(err.code(), "database_error", "{err}");

    assert_eq!(counts(&fx).await, before, "nothing was left behind");

    let key = OperationKey {
        scope: fx.scope.clone(),
        operation_type: "transfer".to_owned(),
        idempotency_key: "req-1".to_owned(),
    };
    assert!(
        fx.store.find_operation(&key).await.expect("read").is_none(),
        "a failed operation must not be recorded"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_operation_succeeds_once_the_failure_is_removed() {
    let Some(fx) = seeded().await else { return };
    break_outbox(&fx).await;
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect_err("first attempt fails");

    sqlx::query("DROP TRIGGER fail_outbox ON outbox_events")
        .execute(fx.store.pool())
        .await
        .expect("drop trigger");

    // The same request id is still usable: the failed attempt left no key.
    let outcome = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("retry succeeds");
    assert!(outcome.applied);

    let (_, operations, events) = counts(&fx).await;
    assert_eq!(operations, 1);
    assert_eq!(events, 1);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_failure_writing_the_ledger_leaves_the_state_alone() {
    let Some(fx) = seeded().await else { return };
    let before = counts(&fx).await;

    sqlx::query(
        "CREATE FUNCTION fail_operations() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'injected ledger failure'; END; $$ LANGUAGE plpgsql",
    )
    .execute(fx.store.pool())
    .await
    .expect("create function");
    sqlx::query(
        "CREATE TRIGGER fail_operations BEFORE INSERT ON operations \
         FOR EACH ROW EXECUTE FUNCTION fail_operations()",
    )
    .execute(fx.store.pool())
    .await
    .expect("create trigger");

    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect_err("the injected failure must surface");

    assert_eq!(counts(&fx).await, before, "the role change was rolled back");

    fx.cleanup().await;
}
