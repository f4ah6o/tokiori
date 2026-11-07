//! Recording changes: new values, corrections, future dates, and endings.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use serde_json::json;
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, SetAttribute};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

async fn seeded() -> Option<(common::Fixture, String)> {
    let fx = fixture().await?;
    fx.store
        .apply_model(&fx.scope, &organization_model())
        .await
        .expect("apply model");
    let employee = NewEntity {
        scope: fx.scope.clone(),
        entity_type: "employee".to_owned(),
        id: "E001".to_owned(),
        attributes: common::attributes(&[("name", "Alice")]),
    };
    fx.store
        .create_entity(employee)
        .await
        .expect("create employee");
    Some((fx, "E001".to_owned()))
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
async fn recording_a_value_creates_the_first_version() {
    let Some((fx, _)) = seeded().await else {
        return;
    };

    let revision = fx
        .store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("set attribute");

    assert_eq!(revision, 1);
    fx.cleanup().await;
}

#[tokio::test]
async fn a_correction_closes_the_old_version_instead_of_overwriting_it() {
    let Some((fx, _)) = seeded().await else {
        return;
    };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("first");

    let second = fx
        .store
        .set_attribute(set(
            &fx,
            "Alicia",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("correction");

    assert_eq!(second, 2);
    let rows: Vec<(i64, Option<i64>, serde_json::Value)> = sqlx::query_as(
        "SELECT recorded_from, recorded_to, value FROM fact_versions ORDER BY recorded_from",
    )
    .fetch_all(fx.store.pool())
    .await
    .expect("read versions");

    assert_eq!(rows.len(), 2, "both versions are kept");
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, Some(2), "the first version was closed by r2");
    assert_eq!(rows[1].0, 2);
    assert_eq!(rows[1].1, None);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_future_change_does_not_affect_the_present() {
    let Some((fx, _)) = seeded().await else {
        return;
    };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("first");

    // A move announced for July 1: the old value ends there, the new one starts.
    fx.store
        .set_attribute(set(
            &fx,
            "Alicia",
            ValidTime::unbounded_from(day(2027, 7, 1)),
        ))
        .await
        .expect("future value");

    let rows: Vec<(
        chrono::NaiveDate,
        Option<chrono::NaiveDate>,
        serde_json::Value,
    )> = sqlx::query_as(
        "SELECT valid_from, valid_until, value FROM fact_versions ORDER BY recorded_from",
    )
    .fetch_all(fx.store.pool())
    .await
    .expect("read versions");

    assert_eq!(rows.len(), 2, "the old version is closed, not duplicated");
    // The old version keeps the interval it asserted; it is superseded by
    // `recorded_to` rather than rewritten, so a query at the earlier revision
    // still sees it as open-ended.
    assert_eq!(rows[0].1, None, "the old version's interval is untouched");
    assert_eq!(rows[1].0, day(2027, 7, 1));

    // What changes is the resolved value: before the move date it is the old
    // one, from the move date it is the new one.
    let before = fx
        .store
        .read_attribute(tokiori_pg::ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 6, 30),
            known_revision: None,
        })
        .await
        .expect("read before");
    let after = fx
        .store
        .read_attribute(tokiori_pg::ReadAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            valid_at: day(2027, 7, 1),
            known_revision: None,
        })
        .await
        .expect("read after");

    assert_eq!(before.value, Some(serde_json::json!("Alice")));
    assert_eq!(after.value, Some(serde_json::json!("Alicia")));

    fx.cleanup().await;
}

#[tokio::test]
async fn ending_a_value_records_a_tombstone_rather_than_deleting_it() {
    let Some((fx, _)) = seeded().await else {
        return;
    };
    fx.store
        .set_attribute(set(
            &fx,
            "Alice",
            ValidTime::unbounded_from(day(2027, 4, 1)),
        ))
        .await
        .expect("first");

    fx.store
        .end_attribute(&fx.scope, "employee", "E001", "name", day(2027, 7, 1))
        .await
        .expect("end");

    let rows: Vec<(Option<chrono::NaiveDate>, serde_json::Value)> =
        sqlx::query_as("SELECT valid_until, value FROM fact_versions ORDER BY recorded_from")
            .fetch_all(fx.store.pool())
            .await
            .expect("read versions");

    assert_eq!(
        rows.len(),
        1,
        "ending closes the version, it does not delete it"
    );
    assert_eq!(rows[0].0, Some(day(2027, 7, 1)));

    fx.cleanup().await;
}

#[tokio::test]
async fn ending_a_value_that_does_not_exist_is_rejected() {
    let Some((fx, _)) = seeded().await else {
        return;
    };

    let err = fx
        .store
        .end_attribute(&fx.scope, "employee", "E001", "name", day(2027, 7, 1))
        .await
        .expect_err("nothing to end");
    assert_eq!(err.code(), "nothing_to_end", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn an_undeclared_attribute_cannot_be_recorded() {
    let Some((fx, _)) = seeded().await else {
        return;
    };

    let err = fx
        .store
        .set_attribute(SetAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "salary".to_owned(),
            value: json!(1),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect_err("undeclared attribute");
    assert_eq!(err.code(), "unknown_attribute", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_value_of_the_wrong_type_is_rejected() {
    let Some((fx, _)) = seeded().await else {
        return;
    };

    let err = fx
        .store
        .set_attribute(SetAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            value: json!(42),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect_err("wrong type");
    assert_eq!(err.code(), "invalid_attribute", "{err}");

    fx.cleanup().await;
}
