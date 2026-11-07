//! Transfer: a business operation that moves an employee between departments.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, PgStore, ReadRole, SetRole, Transfer};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Seeds an employee in Sales with a membership, and a Planning department.
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
        .expect("baseline role");

    Some(fx)
}

fn transfer(fx: &common::Fixture, key: &str) -> Transfer {
    Transfer {
        scope: fx.scope.clone(),
        employee: "E001".to_owned(),
        to_department: "D-PLANNING".to_owned(),
        effective_on: day(2027, 7, 1),
        request_id: key.to_owned(),
        reason: "annual rotation".to_owned(),
        actor: "alice".to_owned(),
    }
}

async fn department_at(store: &PgStore, scope: &str, on: NaiveDate) -> Option<String> {
    let answer = store
        .read_role(ReadRole {
            scope: scope.to_owned(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            valid_at: on,
            known_revision: None,
        })
        .await
        .expect("read");
    answer.participant_id
}

#[tokio::test]
async fn a_transfer_moves_the_employee_on_the_effective_date() {
    let Some(fx) = seeded().await else { return };

    let outcome = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    assert!(outcome.revision > 0);
    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 6, 30))
            .await
            .as_deref(),
        Some("D-SALES")
    );
    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 7, 1))
            .await
            .as_deref(),
        Some("D-PLANNING")
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn a_transfer_records_the_operation_with_its_actor_and_reason() {
    let Some(fx) = seeded().await else { return };

    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let key = tokiori_pg::OperationKey {
        scope: fx.scope.clone(),
        operation_type: "transfer".to_owned(),
        idempotency_key: "req-1".to_owned(),
    };
    let record = fx
        .store
        .find_operation(&key)
        .await
        .expect("read")
        .expect("recorded");

    assert_eq!(record.actor, "alice");
    assert_eq!(record.reason, "annual rotation");
    assert_eq!(record.result["to_department"], "D-PLANNING");

    fx.cleanup().await;
}

#[tokio::test]
async fn transferring_to_the_current_department_is_rejected() {
    let Some(fx) = seeded().await else { return };
    let mut request = transfer(&fx, "req-1");
    request.to_department = "D-SALES".to_owned();

    let err = fx
        .store
        .transfer(request)
        .await
        .expect_err("no-op transfer");
    assert_eq!(err.code(), "precondition_failed", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn transferring_an_unknown_employee_is_rejected_without_a_partial_write() {
    let Some(fx) = seeded().await else { return };
    let mut request = transfer(&fx, "req-1");
    request.employee = "E999".to_owned();

    let err = fx
        .store
        .transfer(request)
        .await
        .expect_err("unknown employee");
    assert_eq!(err.code(), "precondition_failed", "{err}");

    // Nothing was recorded: no operation, no role change.
    let key = tokiori_pg::OperationKey {
        scope: fx.scope.clone(),
        operation_type: "transfer".to_owned(),
        idempotency_key: "req-1".to_owned(),
    };
    assert!(
        fx.store.find_operation(&key).await.expect("read").is_none(),
        "a rejected operation must not be recorded"
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
async fn transferring_to_an_unknown_department_is_rejected() {
    let Some(fx) = seeded().await else { return };
    let mut request = transfer(&fx, "req-1");
    request.to_department = "D-NOWHERE".to_owned();

    let err = fx
        .store
        .transfer(request)
        .await
        .expect_err("unknown department");
    assert_eq!(err.code(), "precondition_failed", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_future_transfer_leaves_the_present_alone() {
    let Some(fx) = seeded().await else { return };
    let mut request = transfer(&fx, "req-1");
    request.effective_on = day(2027, 9, 1);

    fx.store.transfer(request).await.expect("transfer");

    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 7, 1))
            .await
            .as_deref(),
        Some("D-SALES"),
        "the present is untouched"
    );
    assert_eq!(
        department_at(&fx.store, &fx.scope, day(2027, 9, 15))
            .await
            .as_deref(),
        Some("D-PLANNING"),
        "the future transfer takes effect on its own date"
    );

    fx.cleanup().await;
}
