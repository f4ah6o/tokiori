//! Bounds: a query stops, and says when it stopped early.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::{EntityRef, Hop, Limits, Query, Select, ValidTime};
use tokiori_pg::{NewEntity, NewRelation, SetRole};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Two employees sharing a department, so a hop back reaches more than one.
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

    for (membership, employee) in [("M001", "E001"), ("M002", "E002")] {
        fx.store
            .create_relation(NewRelation {
                scope: fx.scope.clone(),
                relation_type: "membership".to_owned(),
                id: membership.to_owned(),
                roles: std::collections::BTreeMap::from([
                    ("employee".to_owned(), employee.to_owned()),
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
                relation_id: membership.to_owned(),
                role: "department".to_owned(),
                participant_id: "D-SALES".to_owned(),
                valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
            })
            .await
            .expect("baseline");
    }
    Some(fx)
}

fn hop(from: &str, to: &str) -> Hop {
    Hop {
        relation_type: "membership".to_owned(),
        from_role: from.to_owned(),
        to_role: to.to_owned(),
    }
}

fn query(hops: Vec<Hop>, limits: Limits) -> Query {
    Query {
        start: EntityRef {
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
        },
        traversal: hops,
        select: Select::Ids,
        valid_at: day(2027, 7, 10),
        known_revision: None,
        limits,
    }
}

#[tokio::test]
async fn a_traversal_that_returns_to_the_start_stops_at_the_depth_limit() {
    let Some(fx) = seeded().await else { return };

    // employee -> department -> employee -> department -> ...
    let hops = vec![
        hop("employee", "department"),
        hop("department", "employee"),
        hop("employee", "department"),
        hop("department", "employee"),
    ];
    let result = fx
        .store
        .query(
            &fx.scope,
            &query(
                hops,
                Limits {
                    max_depth: 4,
                    max_rows: 100,
                },
            ),
        )
        .await
        .expect("a cyclic traversal terminates");

    // Both employees are reachable; the cycle does not loop forever.
    assert!(result.rows.len() <= 2, "{:?}", result.rows);

    fx.cleanup().await;
}

#[tokio::test]
async fn a_traversal_longer_than_the_depth_limit_is_refused() {
    let Some(fx) = seeded().await else { return };
    let hops = vec![
        hop("employee", "department"),
        hop("department", "employee"),
        hop("employee", "department"),
    ];

    let err = fx
        .store
        .query(
            &fx.scope,
            &query(
                hops,
                Limits {
                    max_depth: 2,
                    max_rows: 100,
                },
            ),
        )
        .await
        .expect_err("deeper than allowed");
    assert_eq!(err.code(), "invalid_query", "{err}");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_row_budget_smaller_than_the_result_truncates() {
    let Some(fx) = seeded().await else { return };

    let result = fx
        .store
        .query(
            &fx.scope,
            &query(
                vec![hop("employee", "department"), hop("department", "employee")],
                Limits {
                    max_depth: 2,
                    max_rows: 1,
                },
            ),
        )
        .await
        .expect("query");

    assert_eq!(result.rows.len(), 1);
    assert!(result.truncated, "the caller must know rows were dropped");

    fx.cleanup().await;
}

#[tokio::test]
async fn a_row_budget_larger_than_the_result_is_not_truncated() {
    let Some(fx) = seeded().await else { return };

    let result = fx
        .store
        .query(
            &fx.scope,
            &query(
                vec![hop("employee", "department")],
                Limits {
                    max_depth: 1,
                    max_rows: 100,
                },
            ),
        )
        .await
        .expect("query");

    assert!(!result.truncated);

    fx.cleanup().await;
}

#[tokio::test]
async fn an_absurd_limit_is_refused_before_reaching_the_database() {
    let Some(fx) = seeded().await else { return };

    let err = fx
        .store
        .query(
            &fx.scope,
            &query(
                vec![hop("employee", "department")],
                Limits {
                    max_depth: 1,
                    max_rows: Limits::MAX_ROWS + 1,
                },
            ),
        )
        .await
        .expect_err("over the maximum");
    assert_eq!(err.code(), "invalid_query", "{err}");

    fx.cleanup().await;
}
