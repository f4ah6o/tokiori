# P3 evidence

Execution log for the P3 exit criteria in [ROADMAP.md](../ROADMAP.md) §3.
Only runs recorded here count as evidence; anything not listed is **not run**.

## Environment

| Item | Value |
| --- | --- |
| Date | 2026-11-07 |
| Branch | `feat/20261107-p3-query` |
| Toolchain | rustc 1.99.1, cargo 1.99.1 |
| PostgreSQL | 18.6 (postgres:18.6-alpine) |
| PBT seed | `TOKIORI_PROPTEST_SEED=20261107` |

## Gates

| Gate | Command | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Unit / property | `cargo test --workspace --all-targets` | PASS (147 tests) |
| actrun | `actrun workflow run .github/workflows/ci.yml --workspace-mode local` | PASS |

## ORG-006

| Test | Observation |
| --- | --- |
| `the_chart_lists_each_employee_with_their_department` | two employees, two departments |
| `the_chart_follows_a_transfer_at_its_own_date` | Sales on 6/30, Planning on 7/1 |
| `the_chart_at_an_older_revision_shows_what_was_believed_then` | the chart at the pre-transfer revision still shows Sales |
| `the_chart_agrees_with_the_query_api` | the chart and a hand-written query give the same answer |

## Query safety

| Test | Observation |
| --- | --- |
| `a_hostile_id_stays_a_value` | `'; DROP TABLE entities; --` never reaches the SQL structure |
| `the_generated_sql_binds_every_value` | ids, dates, revisions, limits are all placeholders |
| `identifiers_come_from_the_model_only` | only model names appear as identifiers |
| `an_undeclared_role_is_rejected` | rejected before compilation |
| `a_hop_that_does_not_continue_from_the_current_type_is_rejected` | a meaningless path is unrepresentable |

## Bounds

| Test | Observation |
| --- | --- |
| `a_traversal_that_returns_to_the_start_stops_at_the_depth_limit` | a cyclic traversal terminates |
| `a_traversal_longer_than_the_depth_limit_is_refused` | refused, not clamped |
| `a_row_budget_smaller_than_the_result_truncates` | `truncated` is set |
| `an_absurd_limit_is_refused_before_reaching_the_database` | rejected at validation |

## Property tests

| Property | Cases | Result |
| --- | --- | --- |
| any query answers like the interpreter | 150 x 8 | PASS |

## Defects the tests found

1. **`create_relation` wrote roles to one table only**, so a membership was
   invisible to versioned traversal.
2. **The first hop's from-role was joined twice**, because the FROM clause
   already named it.
3. **A two-hop traversal reused the previous hop's alias**, which PostgreSQL
   refuses; each hop now has its own row for the from-role.
4. **A query with no named revision bound a sentinel** that broke UTF-8; the
   caller resolves the newest revision first.
5. **A selected attribute was looked up under the start type**, not the type
   the traversal reached.
6. **Cutover decided "already imported" from the revision counter**, which
   `create_relation` advances; it records its own ledger entry instead.
7. **Closing a version rewrote its valid interval**, so a query at an earlier
   revision stopped seeing what that revision believed. Closing now sets only
   `recorded_to`.
8. **The compiled query returned every visible version**, while the read path
   picks the newest; the join now restricts each role to the newest visible
   version.

## Not run

- `EXPLAIN`-based index tuning and a benchmark: not measured. The bounds are
  enforced by depth and row limits rather than by plan analysis.
- Verus proof of query validity: not attempted; `empirically-checked`.
- `ORG-007` and later: not implemented.
