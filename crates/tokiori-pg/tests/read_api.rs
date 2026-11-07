//! Reading a fact as of a date and a revision.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use serde_json::json;
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, ReadAttribute, SetAttribute};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

async fn seeded() -> Option<common::Fixture> {
    let fx = fixture().await?;
    fx.store
        .apply_model(&fx.scope, &organization_model())
        .await
        .expect("apply model");
    fx.store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: common::attributes(&[("name", "Alice")]),
        })
        .await
        .expect("create employee");
    Some(fx)
}

fn read(fx: &common::Fixture, valid_at: NaiveDate, known: Option<u64>) -> ReadAttribute {
    ReadAttribute {
        scope: fx.scope.clone(),
        entity_type: "employee".to_owned(),
        entity_id: "E001".to_owned(),
        attribute: "name".to_owned(),
        valid_at,
        known_revision: known,
    }
}

fn set(fx: &common::Fixture, value: &str, valid: ValidTime) -> SetAttribute {
    SetAttribute {
        scope: fx.scope.clone(),
        entity_type: "employee".to_owned(),
        entity_id: "E001".to_owned(),
        attribute: "name".to_owned(),
        value: json!(value),
        valid_time: valid,
    }
}

#[tokio::test]
async fn the_latest_revision_is_used_when_none_is_named() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("set");

    let answer = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 10), None))
        .await
        .expect("read");

    assert_eq!(answer.value, Some(json!("Alice")));
    assert_eq!(answer.known_revision, 1);
    assert_eq!(answer.valid_at, day(2027, 7, 10));

    fx.cleanup().await;
}

#[tokio::test]
async fn an_earlier_revision_still_sees_the_old_value() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("r1");
    fx.store
        .set_attribute(set(
            &fx,
            "Alicia",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("r2");

    let at_r1 = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 10), Some(1)))
        .await
        .expect("read r1");
    let at_r2 = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 10), Some(2)))
        .await
        .expect("read r2");

    assert_eq!(at_r1.value, Some(json!("Alice")));
    assert_eq!(at_r2.value, Some(json!("Alicia")));

    fx.cleanup().await;
}

#[tokio::test]
async fn a_date_before_any_fact_is_not_found() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("set");

    let answer = fx
        .store
        .read_attribute(read(&fx, day(2027, 3, 1), None))
        .await
        .expect("read");
    assert_eq!(answer.value, None);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_revision_before_anything_was_recorded_is_history_unavailable() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("r1");
    fx.store
        .set_attribute(set(
            &fx,
            "Alicia",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("r2");

    // r1 is the first revision, so ask about a date the first version did not
    // cover while pretending to be earlier than anything recorded.
    let err = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 10), Some(0)))
        .await
        .expect_err("revision 0 predates everything");
    assert_eq!(err.code(), "unknown_revision", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_revision_from_another_scope_is_rejected() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("r1");

    let err = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 10), Some(99)))
        .await
        .expect_err("r99 does not exist in this scope");
    assert_eq!(err.code(), "unknown_revision", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn ending_a_value_makes_later_dates_not_found_but_keeps_earlier_ones() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("set");
    fx.store
        .end_attribute(&fx.scope, "employee", "E001", "name", day(2027, 7, 1))
        .await
        .expect("end");

    let before = fx
        .store
        .read_attribute(read(&fx, day(2027, 6, 1), None))
        .await
        .expect("read before");
    let after = fx
        .store
        .read_attribute(read(&fx, day(2027, 7, 1), None))
        .await
        .expect("read after");

    assert_eq!(before.value, Some(json!("Alice")));
    assert_eq!(after.value, None, "the tombstone ends the value on 7/1");

    fx.cleanup().await;
}
