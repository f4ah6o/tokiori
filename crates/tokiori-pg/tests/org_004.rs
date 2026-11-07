//! ORG-004: a retroactive correction keeps what the system knew before it.
//!
//! The accepted intent (docs/PDD.md §3) is a 2x2 table. Every cell is checked
//! against the real database, and the whole table is compared at once so a
//! failure reports which cell disagrees rather than just "something changed".

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use serde_json::json;
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, PgStore, ReadRole, SetRole};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// The 2x2 table from the roadmap and PDD, as data.
const EXPECTED: &[(u32, u32, &str, &str)] = &[
    // (month, day, known revision, department on that day)
    (7, 10, "r10", "D-PLANNING"),
    (7, 10, "r11", "D-SALES"),
    (7, 16, "r10", "D-PLANNING"),
    (7, 16, "r11", "D-PLANNING"),
];

async fn department_on(
    store: &PgStore,
    scope: &str,
    day_of_month: u32,
    known: Option<u64>,
) -> Option<String> {
    let answer = store
        .read_role(ReadRole {
            scope: scope.to_owned(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            valid_at: day(2027, 7, day_of_month),
            known_revision: known,
        })
        .await
        .expect("read");
    answer.participant_id
}

/// Builds the r10/r11 history and returns the two revisions.
async fn build_correction_history(fx: &common::Fixture) -> (u64, u64) {
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
    for (id, name) in [("D-SALES", "Sales"), ("D-PLANNING", "Planning")] {
        fx.store
            .create_entity(NewEntity {
                scope: fx.scope.clone(),
                entity_type: "department".to_owned(),
                id: id.to_owned(),
                attributes: common::attributes(&[("name", name)]),
            })
            .await
            .expect("department");
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
            attributes: json!({ "title": "Account manager" })
                .as_object()
                .expect("object")
                .clone(),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("membership");

    let set = |value: &str, valid: ValidTime| SetRole {
        scope: fx.scope.clone(),
        relation_type: "membership".to_owned(),
        relation_id: "M001".to_owned(),
        role: "department".to_owned(),
        participant_id: value.to_owned(),
        valid_time: valid,
    };

    // Baseline: the membership started in Sales on April 1. History must
    // include the starting value, otherwise a query before the move finds
    // nothing.
    fx.store
        .set_role(set("D-SALES", ValidTime::unbounded_from(day(2027, 4, 1))))
        .await
        .expect("baseline");

    // r10: the move to Planning takes effect on July 1.
    let r10 = fx
        .store
        .set_role(set(
            "D-PLANNING",
            ValidTime::unbounded_from(day(2027, 7, 1)),
        ))
        .await
        .expect("r10");

    // r11: the same move is corrected to take effect on July 15. A correction
    // states the whole history as it now stands, so it also re-asserts that
    // Sales continued through 7/14.
    fx.store
        .set_role(set(
            "D-SALES",
            ValidTime::bounded(day(2027, 7, 1), day(2027, 7, 15)).expect("non-empty"),
        ))
        .await
        .expect("r11 sales");
    let r11 = fx
        .store
        .set_role(set(
            "D-PLANNING",
            ValidTime::unbounded_from(day(2027, 7, 15)),
        ))
        .await
        .expect("r11");

    (r10, r11)
}

#[tokio::test]
async fn the_two_by_two_table_is_reproduced_exactly() {
    let Some(fx) = fixture().await else { return };
    let (r10, r11) = build_correction_history(&fx).await;

    let mut actual = Vec::new();
    for (month, day_of_month, label, _) in EXPECTED {
        let known = match *label {
            "r10" => r10,
            _ => r11,
        };
        let value = department_on(&fx.store, &fx.scope, *day_of_month, Some(known)).await;
        actual.push((*month, *day_of_month, *label, value));
    }

    let expected: Vec<(u32, u32, &str, Option<String>)> = EXPECTED
        .iter()
        .map(|(m, d, l, dept)| (*m, *d, *l, Some((*dept).to_owned())))
        .collect();

    assert_eq!(actual, expected, "the 2x2 table must match exactly");

    fx.cleanup().await;
}

#[tokio::test]
async fn adding_r11_does_not_change_what_r10_answers() {
    let Some(fx) = fixture().await else { return };
    let (r10, _) = build_correction_history(&fx).await;

    // Ask every day of July at r10, then again after r11 exists (it already
    // does), and confirm the answers are the same set.
    let before: Vec<Option<String>> = {
        let mut out = Vec::new();
        for d in 1..=31 {
            out.push(department_on(&fx.store, &fx.scope, d, Some(r10)).await);
        }
        out
    };

    // A later correction must not rewrite history at r10.
    fx.store
        .set_role(SetRole {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            participant_id: "D-SALES".to_owned(),
            valid_time: ValidTime::unbounded_from(day(2027, 8, 1)),
        })
        .await
        .expect("later correction");

    let after: Vec<Option<String>> = {
        let mut out = Vec::new();
        for d in 1..=31 {
            out.push(department_on(&fx.store, &fx.scope, d, Some(r10)).await);
        }
        out
    };

    assert_eq!(before, after, "a newer revision changed an older answer");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_correction_takes_effect_on_its_own_boundary() {
    let Some(fx) = fixture().await else { return };
    let (_, r11) = build_correction_history(&fx).await;

    assert_eq!(
        department_on(&fx.store, &fx.scope, 14, Some(r11))
            .await
            .as_deref(),
        Some("D-SALES"),
        "7/14 is still Sales under r11"
    );
    assert_eq!(
        department_on(&fx.store, &fx.scope, 15, Some(r11))
            .await
            .as_deref(),
        Some("D-PLANNING"),
        "7/15 is the first Planning day under r11"
    );

    fx.cleanup().await;
}
