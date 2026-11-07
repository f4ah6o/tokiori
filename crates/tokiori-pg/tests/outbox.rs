//! The outbox: events written with the change, delivered at least once.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, SetRole, Transfer};

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

#[tokio::test]
async fn a_transfer_writes_an_event_in_the_same_commit() {
    let Some(fx) = seeded().await else { return };

    let outcome = fx
        .store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let events = fx.store.claim_outbox(&fx.scope, 10).await.expect("claim");

    assert_eq!(events.len(), 1, "one event for one transfer");
    assert_eq!(events[0].revision, outcome.revision);
    assert_eq!(events[0].event_type, "transfer.applied");
    assert_eq!(events[0].payload["to_department"], "D-PLANNING");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_rejected_transfer_writes_no_event() {
    let Some(fx) = seeded().await else { return };
    let mut request = transfer(&fx, "req-1");
    request.employee = "E999".to_owned();

    fx.store
        .transfer(request)
        .await
        .expect_err("unknown employee");

    assert!(
        fx.store
            .claim_outbox(&fx.scope, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a rejected operation must not emit an event"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn a_replayed_transfer_writes_no_second_event() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("first");
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("replay");

    let events = fx.store.claim_outbox(&fx.scope, 10).await.expect("claim");
    assert_eq!(events.len(), 1);

    fx.cleanup().await;
}

#[tokio::test]
async fn an_unacknowledged_event_is_delivered_again() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let first = fx
        .store
        .claim_outbox(&fx.scope, 10)
        .await
        .expect("first claim");
    assert_eq!(first.len(), 1);

    // The consumer crashed before acknowledging: the event comes back.
    let second = fx
        .store
        .claim_outbox(&fx.scope, 10)
        .await
        .expect("second claim");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].event_id, first[0].event_id, "same event, same id");

    fx.cleanup().await;
}

#[tokio::test]
async fn an_acknowledged_event_is_not_delivered_again() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let events = fx.store.claim_outbox(&fx.scope, 10).await.expect("claim");
    fx.store
        .ack_outbox(&[events[0].event_id])
        .await
        .expect("ack");

    let remaining = fx.store.claim_outbox(&fx.scope, 10).await.expect("claim");
    assert!(remaining.is_empty(), "an acknowledged event is retired");

    fx.cleanup().await;
}

#[tokio::test]
async fn acknowledging_twice_is_harmless() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");
    let events = fx.store.claim_outbox(&fx.scope, 10).await.expect("claim");

    fx.store
        .ack_outbox(&[events[0].event_id])
        .await
        .expect("first ack");
    fx.store
        .ack_outbox(&[events[0].event_id])
        .await
        .expect("second ack is a no-op");

    fx.cleanup().await;
}

#[tokio::test]
async fn events_are_scoped() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(transfer(&fx, "req-1"))
        .await
        .expect("transfer");

    let other = format!("{}-other", fx.scope);
    assert!(
        fx.store
            .claim_outbox(&other, 10)
            .await
            .expect("claim")
            .is_empty(),
        "a scope only sees its own events"
    );

    fx.cleanup().await;
}
