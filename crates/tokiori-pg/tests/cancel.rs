//! Cancel: undoing a transfer without erasing that it happened.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{
    CancelTransfer, NewEntity, NewRelation, OperationKey, PgStore, ReadRole, SetRole, Transfer,
};

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

fn cancel(fx: &common::Fixture, transfer_key: &str, cancel_key: &str) -> CancelTransfer {
    CancelTransfer {
        scope: fx.scope.clone(),
        transfer_request_id: transfer_key.to_owned(),
        cancel_request_id: cancel_key.to_owned(),
        reason: "mistaken transfer".to_owned(),
        actor: "bob".to_owned(),
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

#[tokio::test]
async fn cancelling_a_transfer_restores_the_previous_department() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    fx.store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-1"))
        .await
        .expect("cancel");

    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 7, 1))
            .await
            .as_deref(),
        Some("D-SALES"),
        "the employee is back in Sales from the original effective date"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_transfer_and_its_cancellation_both_remain_readable() {
    let Some(fx) = seeded().await else { return };
    let transferred = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");
    fx.store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-1"))
        .await
        .expect("cancel");

    // The transfer is still in the ledger, and still answers at its revision.
    let key = OperationKey {
        scope: fx.scope.clone(),
        operation_type: "transfer".to_owned(),
        idempotency_key: "req-1".to_owned(),
    };
    let record = fx
        .store
        .find_operation(&key)
        .await
        .expect("read")
        .expect("transfer recorded");
    assert_eq!(record.revision, transferred.revision);

    let at_transfer = fx
        .store
        .read_role(ReadRole {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            valid_at: day(2027, 7, 1),
            known_revision: Some(transferred.revision),
        })
        .await
        .expect("read at transfer");
    assert_eq!(
        at_transfer.participant_id.as_deref(),
        Some("D-PLANNING"),
        "at the transfer's revision the move is what the system believed"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn cancelling_twice_is_idempotent() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let first = fx
        .store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-1"))
        .await
        .expect("first cancel");
    let second = fx
        .store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-1"))
        .await
        .expect("replay");

    assert!(first.applied);
    assert!(!second.applied);
    assert_eq!(first.revision, second.revision);

    fx.cleanup().await;
}

#[tokio::test]
async fn cancelling_an_unknown_transfer_is_rejected() {
    let Some(fx) = seeded().await else { return };

    let err = fx
        .store
        .cancel_transfer(cancel(&fx, "nope", "cancel-1"))
        .await
        .expect_err("unknown transfer");
    assert_eq!(err.code(), "operation_not_found", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_cancelled_transfer_cannot_be_cancelled_again_by_a_new_request() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");
    fx.store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-1"))
        .await
        .expect("cancel");

    let err = fx
        .store
        .cancel_transfer(cancel(&fx, "req-1", "cancel-2"))
        .await
        .expect_err("already cancelled");
    assert_eq!(err.code(), "precondition_failed", "{err}");

    fx.cleanup().await;
}
