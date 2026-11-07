//! ORG-003: importing P0's current state without inventing its history.
//!
//! P0 kept only current values. After the cutover the current values must be
//! unchanged, and anything the system did not record must read as "we do not
//! know" rather than "it did not exist".

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use serde_json::json;
use tokiori_model::ValidTime;
use tokiori_pg::{Cutover, NewEntity, NewRelation, ReadAttribute, SetAttribute};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Writes P0-shaped data directly, the way a P0 deployment would have.
async fn seed_p0_state(fx: &common::Fixture) {
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
        .expect("employee");
    fx.store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "department".to_owned(),
            id: "D-SALES".to_owned(),
            attributes: common::attributes(&[("name", "Sales")]),
        })
        .await
        .expect("department");
    fx.store
        .create_relation(NewRelation {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            id: "M001".to_owned(),
            roles: std::collections::BTreeMap::from([
                ("employee".to_owned(), "E001".to_owned()),
                ("department".to_owned(), "D-SALES".to_owned()),
            ]),
            attributes: json!({ "title": "Account manager" })
                .as_object()
                .expect("object")
                .clone(),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("membership");
}

#[tokio::test]
async fn cutover_keeps_the_current_values() {
    let Some(fx) = fixture().await else { return };
    seed_p0_state(&fx).await;

    let outcome = fx
        .store
        .cutover(Cutover {
            scope: fx.scope.clone(),
            at: day(2027, 7, 1),
        })
        .await
        .expect("cutover");

    assert_eq!(outcome.revision, 1, "the import is the first revision");

    let name = fx
        .store
        .read_attribute(ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 7, 10),
            known_revision: None,
        })
        .await
        .expect("read");
    assert_eq!(name.value, Some(json!("Alice")));

    fx.cleanup().await;
}

#[tokio::test]
async fn a_date_before_the_cutover_is_history_unavailable_not_not_found() {
    let Some(fx) = fixture().await else { return };
    seed_p0_state(&fx).await;
    fx.store
        .cutover(Cutover {
            scope: fx.scope.clone(),
            at: day(2027, 7, 1),
        })
        .await
        .expect("cutover");

    // P0 recorded no effective start for an entity, so the imported fact is
    // only defensible from the cutover onward.
    let answer = fx
        .store
        .read_attribute(ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 6, 1),
            known_revision: None,
        })
        .await
        .expect("read");

    assert_eq!(
        answer.value, None,
        "the value is not claimed before the cutover"
    );
    assert_eq!(answer.known_revision, 1);

    fx.cleanup().await;
}

#[tokio::test]
async fn cutover_is_idempotent() {
    let Some(fx) = fixture().await else { return };
    seed_p0_state(&fx).await;

    let first = fx
        .store
        .cutover(Cutover {
            scope: fx.scope.clone(),
            at: day(2027, 7, 1),
        })
        .await
        .expect("first cutover");
    let second = fx
        .store
        .cutover(Cutover {
            scope: fx.scope.clone(),
            at: day(2027, 7, 1),
        })
        .await
        .expect("second cutover");

    assert_eq!(first.revision, second.revision, "the same R0");
    assert!(!second.created, "the second run creates nothing");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM fact_versions")
        .fetch_one(fx.store.pool())
        .await
        .expect("count");
    // One fact per seeded attribute: the employee name and the department name.
    assert_eq!(rows, 2, "the import ran once, not twice");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_value_recorded_after_the_cutover_is_visible_at_a_later_revision() {
    let Some(fx) = fixture().await else { return };
    seed_p0_state(&fx).await;
    fx.store
        .cutover(Cutover {
            scope: fx.scope.clone(),
            at: day(2027, 7, 1),
        })
        .await
        .expect("cutover");

    let revision = fx
        .store
        .set_attribute(SetAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            value: json!("Alicia"),
            valid_time: ValidTime::unbounded_from(day(2027, 8, 1)),
        })
        .await
        .expect("set");
    assert_eq!(revision, 2);

    // At r1 the imported value was open-ended, so it covered 8/10; r2 only
    // changed the value from 8/1 onward.
    let at_r1 = fx
        .store
        .read_attribute(ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 7, 15),
            known_revision: Some(1),
        })
        .await
        .expect("read r1");
    assert_eq!(at_r1.value, Some(json!("Alice")));

    let at_r2 = fx
        .store
        .read_attribute(ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 8, 10),
            known_revision: Some(2),
        })
        .await
        .expect("read r2");
    assert_eq!(at_r2.value, Some(json!("Alicia")));

    fx.cleanup().await;
}
