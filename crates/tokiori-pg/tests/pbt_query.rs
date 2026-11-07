//! Property test: any query the compiler accepts answers like the interpreter.

mod common;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use noprop::TestCaseContext;
use tokiori_model::{EntityRef, Hop, Limits, Query, Select, ValidTime};
use tokiori_pg::{NewEntity, NewRelation, SetRole, Transfer};

const CASES: usize = 150;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Two employees, two departments, and a transfer, so traversals have choices.
async fn seed(fx: &common::Fixture) {
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
        [("M001", "E001", "D-SALES"), ("M002", "E002", "D-SALES")]
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
}

fn sample_query(ctx: &mut TestCaseContext, latest: Option<u64>) -> Query {
    let start = noprop::sample_choice(ctx, &["E001", "E002"]);
    let hops = noprop::sample_usize_in(ctx, 0..=2);
    let mut traversal = Vec::new();
    let mut position = "employee";
    for _ in 0..hops {
        let (from_role, to_role, next) = if position == "employee" {
            ("employee", "department", "department")
        } else {
            ("department", "employee", "employee")
        };
        traversal.push(Hop {
            relation_type: "membership".to_owned(),
            from_role: from_role.to_owned(),
            to_role: to_role.to_owned(),
        });
        position = next;
    }
    let select = if noprop::sample_bool(ctx) {
        Select::Ids
    } else {
        Select::Attribute {
            attribute: "name".to_owned(),
        }
    };
    let month = noprop::sample_u32_in(ctx, 3..=9);
    let day_of_month = noprop::sample_u32_in(ctx, 1..=28);
    // Ask at the newest revision half the time, at an older one otherwise.
    let known = match latest {
        Some(latest) if noprop::sample_bool(ctx) => Some(noprop::sample_u64_in(ctx, 1..=latest)),
        _ => None,
    };
    Query {
        start: EntityRef {
            entity_type: "employee".to_owned(),
            id: start.to_owned(),
        },
        traversal,
        select,
        valid_at: day(2027, month, day_of_month),
        known_revision: known,
        limits: Limits::default(),
    }
}

#[test]
fn any_query_answers_like_the_interpreter() -> noprop::TestResult {
    let seed_value = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let mut runner = noprop::Runner::new(seed_value);

    // One fixture for every case. Creating a fixture per case meant opening and
    // closing five connections 150 times inside a runtime this test drives by
    // hand, and the pool occasionally read from a connection it had just
    // closed ("unknown message type"). The cases do not need isolated schemas:
    // they only read, and the seed is written once.
    let Some(fx) = runtime.block_on(fixture()) else {
        return Ok(());
    };
    runtime.block_on(seed(&fx));
    let latest = runtime
        .block_on(fx.store.latest_revision(&fx.scope))
        .expect("revision");

    for _ in 0..CASES {
        let outcome = runner.run(8, |ctx| {
            let query = sample_query(ctx, latest);
            runtime.block_on(async {
                let compiled = fx.store.query(&fx.scope, &query).await;
                let naive = fx.store.query_naive(&fx.scope, &query).await;
                match (compiled, naive) {
                    (Ok(a), Ok(b)) => {
                        let mut a_rows = a.rows;
                        let mut b_rows = b.rows;
                        a_rows.sort_by(|x, y| x.id.cmp(&y.id));
                        b_rows.sort_by(|x, y| x.id.cmp(&y.id));
                        assert_eq!(a_rows, b_rows, "{query:?}");
                    }
                    (Err(a), Err(b)) => assert_eq!(a.code(), b.code(), "{query:?}"),
                    (a, b) => panic!("one path failed and the other did not: {a:?} vs {b:?}"),
                }
            });
            Ok(())
        });

        outcome?;
    }

    runtime.block_on(fx.cleanup());
    Ok(())
}
