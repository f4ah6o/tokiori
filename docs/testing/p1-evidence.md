# P1 evidence

Execution log for the P1 exit criteria in [ROADMAP.md](../ROADMAP.md) §3.
Only runs recorded here count as evidence; anything not listed is **not run**.

## Environment

| Item | Value |
| --- | --- |
| Date | 2026-11-07 |
| Branch | `feat/20261107-p1-bitemporal` |
| Toolchain | rustc 1.99.1, cargo 1.99.1 |
| PostgreSQL | 18.6 (postgres:18.6-alpine) |
| PBT seed | `TOKIORI_PROPTEST_SEED=20261107`, 1,000 cases per property |

## Gates

| Gate | Command | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Unit / property | `cargo test --workspace --all-targets` | PASS (77 tests) |
| PostgreSQL E2E | `TOKIORI_TEST_DATABASE_URL=... cargo test --workspace --all-targets` | PASS |
| actrun | `actrun workflow run .github/workflows/ci.yml --workspace-mode local` | PASS (fuzz skipped locally) |
| GitHub Actions | PR CI | see the PR checks |

## ORG-004: the two-by-two table

Built from a real database: r10 records the move to Planning effective 7/1;
r11 corrects it to 7/15 and restates that Sales continued through 7/14.

| Query date | `known_revision=r10` | `known_revision=r11` |
| --- | --- | --- |
| 7/10 | Planning | Sales |
| 7/16 | Planning | Planning |

| Test | Observation |
| --- | --- |
| `the_two_by_two_table_is_reproduced_exactly` | all four cells compared at once |
| `adding_r11_does_not_change_what_r10_answers` | every day of July at r10 is unchanged after a later correction |
| `the_correction_takes_effect_on_its_own_boundary` | 7/14 is Sales, 7/15 is Planning under r11 |

## ORG-003: cutover

| Test | Observation |
| --- | --- |
| `cutover_keeps_the_current_values` | imported state is readable at the cutover date |
| `a_date_before_the_cutover_is_history_unavailable_not_not_found` | no value is claimed before the cutover |
| `cutover_is_idempotent` | second run reports the same R0 and writes nothing |
| `a_value_recorded_after_the_cutover_is_visible_at_a_later_revision` | r1 and r2 answer differently, as they should |

## Property tests

| Property | Cases | Result |
| --- | --- | --- |
| `resolve` agrees with an independent reference | 1,000 | PASS |
| a newer revision never changes an older answer | 1,000 | PASS |

## Defects the tests found

1. **A closed version was invisible to later revisions.** The resolver treated
   `recorded_to` as exclusive, so a version closed at r12 disappeared at r99
   even though nothing replaced it.
2. **The newest knowledge did not win.** With a correction, both the old and
   the new version matched; the resolver returned whichever came first instead
   of the highest `recorded_from`.
3. **A future-dated change left the old value open-ended.** The close marked
   the version superseded but did not cut its valid interval, so the old value
   kept applying after the new one began.
4. **Role participants had no history.** A membership's department is a role,
   not an attribute; the write path only handled attributes.
5. **The role-versions table lacked its recorded-interval columns**, so a role
   change could not be closed.
6. **The read path filtered revisions in SQL.** Filtering `recorded_from <= R`
   in the query dropped the older version a correction still needs.

## Not run

- `known_at` (wall clock to revision): deliberately not implemented; see
  [ADR 0005](../adr/0005-p1-bitemporal-engine.md).
- Verus proof of the resolver: not attempted; the resolver is
  `empirically-checked` against a reference, not `formally-proven`.
- Crash-during-commit recovery: relies on PostgreSQL; no fault injection.
- `ORG-005` and later: not implemented.
