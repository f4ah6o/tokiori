//! ORG-006: an org chart as of a date and revision, from the query API alone.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, OrgChart, SetRole, Transfer};

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

#[tokio::test]
async fn the_chart_lists_each_employee_with_their_department() {
    let Some(fx) = seeded().await else { return };

    let chart = fx
        .store
        .org_chart(OrgChart {
            scope: fx.scope.clone(),
            valid_at: day(2027, 7, 10),
            known_revision: None,
        })
        .await
        .expect("chart");

    assert_eq!(chart.len(), 2);
    assert!(
        chart
            .iter()
            .any(|row| row.employee == "E001" && row.department == "D-SALES")
    );
    assert!(
        chart
            .iter()
            .any(|row| row.employee == "E002" && row.department == "D-PLANNING")
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_chart_follows_a_transfer_at_its_own_date() {
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

    let before = fx
        .store
        .org_chart(OrgChart {
            scope: fx.scope.clone(),
            valid_at: day(2027, 6, 30),
            known_revision: None,
        })
        .await
        .expect("chart before");
    let after = fx
        .store
        .org_chart(OrgChart {
            scope: fx.scope.clone(),
            valid_at: day(2027, 7, 1),
            known_revision: None,
        })
        .await
        .expect("chart after");

    assert_eq!(
        before
            .iter()
            .find(|r| r.employee == "E001")
            .map(|r| r.department.as_str()),
        Some("D-SALES")
    );
    assert_eq!(
        after
            .iter()
            .find(|r| r.employee == "E001")
            .map(|r| r.department.as_str()),
        Some("D-PLANNING")
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_chart_at_an_older_revision_shows_what_was_believed_then() {
    let Some(fx) = seeded().await else { return };
    let latest_before = fx
        .store
        .latest_revision(&fx.scope)
        .await
        .expect("revision")
        .expect("a revision");

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

    let then = fx
        .store
        .org_chart(OrgChart {
            scope: fx.scope.clone(),
            valid_at: day(2027, 7, 10),
            known_revision: Some(latest_before),
        })
        .await
        .expect("chart then");

    assert_eq!(
        then.iter()
            .find(|r| r.employee == "E001")
            .map(|r| r.department.as_str()),
        Some("D-SALES"),
        "the chart at the old revision still shows the old department"
    );

    fx.cleanup().await;
}

#[tokio::test]
async fn the_chart_agrees_with_the_query_api() {
    let Some(fx) = seeded().await else { return };

    let chart = fx
        .store
        .org_chart(OrgChart {
            scope: fx.scope.clone(),
            valid_at: day(2027, 7, 10),
            known_revision: None,
        })
        .await
        .expect("chart");

    // The same question asked as a query: who is in each department.
    for row in &chart {
        let query = tokiori_model::Query {
            start: tokiori_model::EntityRef {
                entity_type: "employee".to_owned(),
                id: row.employee.clone(),
            },
            traversal: vec![tokiori_model::Hop {
                relation_type: "membership".to_owned(),
                from_role: "employee".to_owned(),
                to_role: "department".to_owned(),
            }],
            select: tokiori_model::Select::Ids,
            valid_at: day(2027, 7, 10),
            known_revision: None,
            limits: tokiori_model::Limits::default(),
        };
        let result = fx.store.query(&fx.scope, &query).await.expect("query");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].id, row.department);
    }

    fx.cleanup().await;
}
