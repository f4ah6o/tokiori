# P2 evidence

Execution log for the P2 exit criteria in [ROADMAP.md](../ROADMAP.md) §3.
Only runs recorded here count as evidence; anything not listed is **not run**.

## Environment

| Item | Value |
| --- | --- |
| Date | 2026-11-07 |
| Branch | `feat/20261107-p2-operations` |
| Toolchain | rustc 1.99.1, cargo 1.99.1 |
| PostgreSQL | 18.6 (postgres:18.6-alpine) |
| PBT seed | `TOKIORI_PROPTEST_SEED=20261107` |

## Gates

| Gate | Command | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Unit / property | `cargo test --workspace --all-targets` | PASS (111 tests) |
| actrun | `actrun workflow run .github/workflows/ci.yml --workspace-mode local` | PASS |

## ORG-005

| Test | Observation |
| --- | --- |
| `the_same_request_twice_applies_once_and_returns_the_same_result` | `applied` then replay; same revision, same result, no second role version |
| `the_same_key_with_a_different_target_is_a_conflict_and_changes_nothing` | `idempotency_conflict`, history unchanged |
| `concurrent_replays_apply_the_transfer_once` | three concurrent writers, exactly one applied |
| `a_rejected_precondition_records_nothing` | no ledger row, no role version |
| `a_failure_writing_the_event_rolls_back_the_state_and_the_ledger` | injected trigger failure; all three tables unchanged |
| `a_failure_writing_the_ledger_leaves_the_state_alone` | injected trigger failure; role versions unchanged |
| `the_operation_succeeds_once_the_failure_is_removed` | same request id works after the failure is removed |
| `an_unacknowledged_event_is_delivered_again` | same `event_id` delivered twice |
| `an_acknowledged_event_is_not_delivered_again` | retired after ack |
| `acknowledging_twice_is_harmless` | second ack is a no-op |

## Stateful PBT

| Property | Cases | Result |
| --- | --- | --- |
| command sequence matches a reference model | 200 cases x 8 commands | PASS |

The model tracks the applied keys and the department each one implies; after
every command the database is compared with the model at three dates.

## Defects the tests found

1. **A replay was indistinguishable from a fresh record.** `ON CONFLICT DO
   NOTHING` reports success either way; `RETURNING` is needed to tell them
   apart.
2. **Closing a version rewrote its valid interval.** A later revision then
   changed what an earlier one believed. Closing now trims only to the new
   version's start, and only when that leaves a non-empty interval.
3. **An operation and its effect took different revisions.** The role write
   allocated its own number, so the transfer's recorded revision did not show
   the move.
4. **The transfer did not record the previous department**, so a cancellation
   had nothing to restore.
5. **A membership created through the P0 API was invisible to transfers**:
   its roles live in the current-state tables, not the versioned ones.
6. **The generator reused a key across different inputs**, which the engine
   correctly called a conflict; the model, not the engine, was wrong.

## Not run

- Exactly-once delivery: deliberately not implemented or claimed.
- Verus proof of the idempotency decision: not attempted; the decision is
  `empirically-checked`.
- `ORG-006` and later: not implemented.
