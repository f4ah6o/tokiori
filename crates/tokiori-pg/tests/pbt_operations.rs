//! Stateful property test: a command sequence against a reference model.
//!
//! The model is deliberately naive: it tracks the department and the set of
//! applied request keys in plain Rust. If the database and the model disagree
//! after any command, one of them is wrong.

mod common;

use std::collections::BTreeMap;

use chrono::NaiveDate;
use common::{fixture, organization_model};
use noprop::TestCaseContext;
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, ReadRole, SetRole, Transfer};

const CASES: usize = 200;
const COMMANDS: usize = 8;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Move to Planning on 7/1 under a request id.
    Transfer(u8),
    /// Replay the same request id.
    Replay(u8),
    /// Move to Planning on 9/1 under a request id.
    LateTransfer(u8),
}

/// The reference model: which department applies from 7/1 and from 9/1.
#[derive(Debug, Default)]
struct Model {
    applied: BTreeMap<u8, (String, NaiveDate)>,
}

impl Model {
    fn department_on(&self, on: NaiveDate) -> String {
        let mut current = "D-SALES".to_owned();
        let mut best: Option<NaiveDate> = None;
        for (department, effective) in self.applied.values() {
            if *effective <= on && best.is_none_or(|b| *effective >= b) {
                current.clone_from(department);
                best = Some(*effective);
            }
        }
        current
    }
}

fn sample_command(ctx: &mut TestCaseContext, model: &Model) -> Command {
    let weights: &[u32] = if model.applied.is_empty() {
        &[3, 0, 2]
    } else {
        &[3, 3, 2]
    };
    match noprop::sample_weighted_index(ctx, weights) {
        0 => Command::Transfer(noprop::sample_u8_in(ctx, 0..=3)),
        1 => {
            let keys: Vec<u8> = model.applied.keys().copied().collect();
            Command::Replay(noprop::sample_choice(ctx, &keys))
        }
        // A late transfer uses its own key space: the same key with a different
        // effective date is a different request, and the engine rightly calls
        // that a conflict.
        _ => Command::LateTransfer(noprop::sample_u8_in(ctx, 4..=7)),
    }
}

async fn seed(fx: &common::Fixture) {
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
}

async fn department_on(fx: &common::Fixture, on: NaiveDate) -> String {
    fx.store
        .read_role(ReadRole {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            relation_id: "M001".to_owned(),
            role: "department".to_owned(),
            valid_at: on,
            known_revision: None,
        })
        .await
        .expect("read")
        .participant_id
        .unwrap_or_else(|| "(none)".to_owned())
}

/// noprop drives a synchronous closure, so the runner owns the thread and the
/// database work is awaited on a runtime it creates.
#[test]
fn a_command_sequence_matches_the_reference_model() -> noprop::TestResult {
    let seed_value = noprop::seed_from_env_or_time("TOKIORI_PROPTEST_SEED")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let mut runner = noprop::Runner::new(seed_value);

    for _ in 0..CASES {
        let Some(fx) = runtime.block_on(fixture()) else {
            return Ok(());
        };
        runtime.block_on(seed(&fx));

        // `Runner::run` takes an `Fn`, so the model and the trace live behind a
        // `RefCell`.
        let model = std::cell::RefCell::new(Model::default());
        let history = std::cell::RefCell::new(Vec::new());

        let outcome = runner.run(COMMANDS, |ctx| {
            let command = sample_command(ctx, &model.borrow());
            history.borrow_mut().push(command);
            let trace = format!("{:?}", history.borrow());

            runtime.block_on(async {
                // A replay must resend the original request exactly; sending the
                // same key with a different date is a conflict, not a replay.
                let (key, effective) = match command {
                    Command::Transfer(k) => (k, day(2027, 7, 1)),
                    Command::LateTransfer(k) => (k, day(2027, 9, 1)),
                    Command::Replay(k) => {
                        let recorded = model
                            .borrow()
                            .applied
                            .get(&k)
                            .map_or_else(|| day(2027, 7, 1), |(_, effective)| *effective);
                        (k, recorded)
                    }
                };
                let request = Transfer {
                    scope: fx.scope.clone(),
                    employee: "E001".to_owned(),
                    to_department: "D-PLANNING".to_owned(),
                    effective_on: effective,
                    request_id: format!("req-{key}"),
                    reason: "pbt".to_owned(),
                    actor: "pbt".to_owned(),
                };

                match fx.store.transfer(request).await {
                    Ok(_) => {
                        model
                            .borrow_mut()
                            .applied
                            .insert(key, ("D-PLANNING".to_owned(), effective));
                    }
                    Err(err) if err.code() == "precondition_failed" => {
                        // Already in the target department on that date.
                        assert_eq!(
                            model.borrow().department_on(effective),
                            "D-PLANNING",
                            "{trace}"
                        );
                    }
                    Err(err) => panic!("unexpected error {err} after {trace}"),
                }

                for on in [day(2027, 6, 1), day(2027, 7, 10), day(2027, 9, 15)] {
                    let actual = department_on(&fx, on).await;
                    let expected = model.borrow().department_on(on);
                    assert_eq!(actual, expected, "on {on} after {trace}");
                }
            });
            Ok(())
        });

        runtime.block_on(fx.cleanup());
        outcome?;
    }
    Ok(())
}
