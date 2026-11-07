//! Running a compiled query, and comparing it with a naive interpreter.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::{EntityRef, Hop, Limits, Query, Select, ValidTime};
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
        ("employee", "E002"),
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
    for (membership, employee, department) in
        [("M001", "E001", "D-SALES"), ("M002", "E002", "D-PLANNING")]
    {
        fx.store
            .create_relation(NewRelation {
                scope: fx.scope.clone(),
                relation_type: "membership".to_owned(),
                id: membership.to_owned(),
                roles: std::collections::BTreeMap::from([
                    ("employee".to_owned(), employee.to_owned()),
                    ("department".to_owned(), department.to_owned()),
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
                relation_id: membership.to_owned(),
                role: "department".to_owned(),
                participant_id: department.to_owned(),
                valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
            })
            .await
            .expect("baseline");
    }
    Some(fx)
}

fn query(id: &str, valid_at: NaiveDate, known: Option<u64>) -> Query {
    Query {
        start: EntityRef {
            entity_type: "employee".to_owned(),
            id: id.to_owned(),
        },
        traversal: vec![Hop {
            relation_type: "membership".to_owned(),
            from_role: "employee".to_owned(),
            to_role: "department".to_owned(),
        }],
        select: Select::Ids,
        valid_at,
        known_revision: known,
        limits: Limits::default(),
    }
}

#[tokio::test]
async fn a_one_hop_query_finds_the_department() {
    let Some(fx) = seeded().await else { return };

    let result = fx
        .store
        .query(&fx.scope, &query("E001", day(2027, 7, 10), None))
        .await
        .expect("query");

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].id, "D-SALES");
    assert!(!result.truncated);

    fx.cleanup().await;
}

#[tokio::test]
async fn an_unknown_start_is_not_found_rather_than_empty() {
    let Some(fx) = seeded().await else { return };

    let err = fx
        .store
        .query(&fx.scope, &query("E999", day(2027, 7, 10), None))
        .await
        .expect_err("unknown start");
    assert_eq!(err.code(), "not_found", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_date_before_the_membership_returns_no_rows() {
    let Some(fx) = seeded().await else { return };

    let result = fx
        .store
        .query(&fx.scope, &query("E001", day(2027, 1, 1), None))
        .await
        .expect("query");
    assert_eq!(result.rows, Vec::new());

    fx.cleanup().await;
}

#[tokio::test]
async fn a_transfer_is_visible_at_the_right_revision() {
    let Some(fx) = seeded().await else { return };
    let outcome = fx
        .store
        .transfer(Transfer {
            scope: fx.scope.clone(),
            employee: "E001".to_owned(),
            to_department: "D-PLANNING".to_owned(),
            effective_on: day(2027, 7, 1),
            request_id: "req-1".to_owned(),
            reason: "rotation".to_owned(),
            actor: "alice".to_owned(),
        })
        .await
        .expect("transfer");

    // Ask at the revision the membership was recorded at, which is before the
    // transfer: creating the relation now takes a revision of its own.
    let before = fx
        .store
        .query(&fx.scope, &query("E001", day(2027, 7, 10), Some(1)))
        .await
        .expect("query before");
    assert!(outcome.revision > 1, "the transfer is a later revision");
    let after = fx
        .store
        .query(&fx.scope, &query("E001", day(2027, 7, 10), None))
        .await
        .expect("query after");

    assert_eq!(before.rows[0].id, "D-SALES");
    assert_eq!(after.rows[0].id, "D-PLANNING");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_query_agrees_with_the_naive_interpreter() {
    let Some(fx) = seeded().await else { return };
    fx.store
        .transfer(Transfer {
            scope: fx.scope.clone(),
            employee: "E001".to_owned(),
            to_department: "D-PLANNING".to_owned(),
            effective_on: day(2027, 7, 1),
            request_id: "req-1".to_owned(),
            reason: "rotation".to_owned(),
            actor: "alice".to_owned(),
        })
        .await
        .expect("transfer");

    let latest = fx.store.latest_revision(&fx.scope).await.expect("revision");

    for on in [day(2027, 5, 1), day(2027, 7, 1), day(2027, 8, 1)] {
        for known in [None, Some(1), latest] {
            let compiled = fx
                .store
                .query(&fx.scope, &query("E001", on, known))
                .await
                .expect("compiled");
            let naive = fx
                .store
                .query_naive(&fx.scope, &query("E001", on, known))
                .await
                .expect("naive");
            assert_eq!(compiled.rows, naive.rows, "on {on} known {known:?}");
        }
    }

    fx.cleanup().await;
}

#[tokio::test]
async fn selecting_an_attribute_returns_its_value() {
    let Some(fx) = seeded().await else { return };

    // Record a name for the department so the attribute has a value to find.
    fx.store
        .set_attribute(tokiori_pg::SetAttribute {
            scope: fx.scope.clone(),
            entity_type: "department".to_owned(),
            entity_id: "D-SALES".to_owned(),
            attribute: "name".to_owned(),
            value: serde_json::json!("Sales"),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("set name");

    let mut q = query("E001", day(2027, 7, 10), None);
    q.select = Select::Attribute {
        attribute: "name".to_owned(),
    };

    let result = fx.store.query(&fx.scope, &q).await.expect("query");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].value.as_deref(), Some("Sales"));

    fx.cleanup().await;
}

#[tokio::test]
async fn a_zero_depth_query_returns_the_start() {
    let Some(fx) = seeded().await else { return };
    let mut q = query("E001", day(2027, 7, 10), None);
    q.traversal.clear();

    let result = fx.store.query(&fx.scope, &q).await.expect("query");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].id, "E001");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_query_never_sees_another_scope() {
    let Some(fx) = seeded().await else { return };
    let other = format!("{}-other", fx.scope);

    let err = fx
        .store
        .query(&other, &query("E001", day(2027, 7, 10), None))
        .await
        .expect_err("the employee is not in the other scope");
    assert_eq!(err.code(), "not_found", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn the_row_limit_truncates_and_says_so() {
    let Some(fx) = seeded().await else { return };
    let mut q = query("E001", day(2027, 7, 10), None);
    q.limits = Limits {
        max_depth: 1,
        max_rows: 0,
    };

    let result = fx.store.query(&fx.scope, &q).await.expect("query");
    assert_eq!(result.rows, Vec::new());
    assert!(result.truncated, "a truncated result must say so");

    fx.cleanup().await;
}
