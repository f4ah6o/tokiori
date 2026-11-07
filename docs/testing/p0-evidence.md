# P0 evidence

Execution log for the P0 exit criteria in [ROADMAP.md](../ROADMAP.md) §3.
Only runs recorded here count as evidence; anything not listed is **not run**.

## Environment

| Item | Value |
| --- | --- |
| Date | 2026-11-07 |
| Commit | `f6b5ebe` (PR #2) |
| Toolchain | rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1 |
| PostgreSQL | 18.6 (postgres:18.6-alpine, aarch64) |
| Host | macOS 26.5.2, aarch64 |
| PBT seed | `TOKIORI_PROPTEST_SEED=20261107`, 1,000 cases per property |

## Gates

| Gate | Command | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Unit / property | `cargo test --workspace --all-targets` | PASS (40 tests) |
| PostgreSQL E2E | `TOKIORI_TEST_DATABASE_URL=... cargo test --workspace --all-targets` | PASS (11 DB tests) |
| Fuzz smoke | `cargo +nightly fuzz run fuzz_model_input -- -runs=50000 -max_total_time=120` | PASS (50,000 runs, 5 s, no crash) |
| GitHub Actions | `.github/workflows/ci.yml` on PR #2 | PASS (3 jobs, run 42322192333) |
| actrun (local) | `actrun workflow run .github/workflows/ci.yml --workspace-mode local` | PASS (2 jobs; fuzz skipped locally) |

## Test inventory

| File | Tests | Covers |
| --- | --- | --- |
| `crates/tokiori-model/tests/model_validation.rs` | 15 | model validity, canonical form, digests, JSON/YAML input |
| `crates/tokiori-model/tests/temporal.rs` | 7 | half-open intervals, business-day conversion, DST rejection |
| `crates/tokiori-model/tests/pbt_model.rs` | 4 × 1,000 cases | round trip, injected-defect rejection, order independence, parser safety |
| `crates/tokiori-pg/tests/org_001.rs` | 8 | ORG-001: role type, scope, missing role, duplicate id, unknown attribute, no model |
| `crates/tokiori-pg/tests/org_002.rs` | 3 | ORG-002: idempotent apply, conflicting re-apply, read-back after restart |
| `crates/tokiori-cli/tests/cli_roundtrip.rs` | 3 | CLI round trip across separate processes, stable error codes |

## ORG-001

| Test | Observation |
| --- | --- |
| `a_well_formed_membership_round_trips_through_postgres` | created relation equals the relation read back |
| `assigning_a_department_to_the_employee_role_is_rejected_without_a_partial_write` | `entity_type_mismatch`, `relations` count stays 0 |
| `referencing_an_entity_from_another_scope_is_rejected` | `scope_mismatch`, `relations` count stays 0 |
| `a_relation_without_a_required_role_is_rejected` | `missing_role`, no rows |
| `an_undeclared_role_name_is_rejected` | `unknown_role`, no rows |
| `registering_the_same_entity_id_twice_leaves_the_original_intact` | `duplicate_id`, original attributes unchanged |
| `an_undeclared_attribute_is_rejected` | `unknown_attribute` |
| `writing_before_a_model_is_applied_is_rejected` | `model_not_found` |

## ORG-002

| Test | Observation |
| --- | --- |
| `applying_the_same_model_twice_changes_nothing` | `Created` then `Unchanged`, one `models` row |
| `a_different_definition_under_the_same_version_is_rejected_and_leaves_no_rows` | `model_version_conflict`, stored model unchanged |
| `a_restarted_process_reads_back_the_same_ids_attributes_and_relations` | same id, attributes, roles, valid time, and model digest |

## Red → Green record

Failures found by the tests during implementation, each fixed in the source
rather than by weakening the test:

1. **Canonical JSON was not re-readable.** `canonical_json` serialized the
   normalized map shape while `parse_model_str` expects the document shape, so
   the PBT round trip failed (`invalid type: map, expected a sequence`).
   Fixed by emitting canonical JSON in document shape.
2. **Wrong-type participant was reported as missing.** A department placed in
   the `employee` role returned `scope_mismatch` because the lookup filtered on
   the expected type. Fixed by looking the id up without a type filter and then
   classifying the mismatch.
3. **Test fixtures were wrong, not the code.** Two ORG-001 tests initially
   referenced ids that existed in the current scope, and asserted on the
   iteration order of role checks. Both were corrected to test the intended
   behaviour.

## CI: GitHub Actions

Run [42322192333](https://github.com/f4ah6o/tokiori/actions/runs/42322192333) on PR #2, commit `f6b5ebe`:

| Job | Result | Evidence |
| --- | --- | --- |
| fmt / clippy / unit + property tests | PASS (30 s) | 40 tests |
| PostgreSQL 18 acceptance (ORG-001, ORG-002) | PASS (30 s) | 11 DB tests, no skip |
| fuzz smoke (parser) | PASS (54 s) | `Done 20000 runs in 1 second(s)` |

Earlier runs on this branch failed and are kept here because they are what
made the workflow correct:

| Failure | Cause | Fix |
| --- | --- | --- |
| Workflow never parsed; no job started | `jobs.<job_id>.if` read the `env` context, which GitHub does not allow there | guard moved to the steps |
| `no such command: fuzz` | `taiki-e/install-action@cargo-fuzz` is not a tool tag; it installed nothing | `@v2` with `tool: cargo-fuzz` |
| `-Zsanitizer` rejected | `rust-toolchain.toml` pinned stable, overriding the installed nightly | `fuzz/rust-toolchain.toml` pins nightly for that workspace |
| sanitizer vs static libc | `rustup override set nightly` made musl the default target | GNU target named explicitly |

## CI: actrun (local)

`@mizchi/actrun` 0.32.0, `actrun workflow run .github/workflows/ci.yml --workspace-mode local`:

```text
state=completed
fmt-clippy-test/step_1..6: success
postgres-e2e/step_1..5: success
fuzz-smoke/step_2..6: skipped
```

The fuzz job is skipped locally because actrun cannot provide the nightly
toolchain or cargo-fuzz. **An actrun pass is not a GitHub Actions pass**; the
two are recorded separately because they differ in what they can execute. See
[ADR 0004](../adr/0004-local-ci-with-actrun.md).

What actrun caught that GitHub could not:

| Defect | Why GitHub did not report it |
| --- | --- |
| `ports: - 55432:5432` was a YAML integer pair | GitHub also requires a string; the job would have failed later |
| folded `>-` service `options` | valid YAML, actrun-only parser limit |
| no `timeout-minutes` on any job | a hung job would run 6 hours |
| quoted `-U tokiori` in `--health-cmd` | actrun splits options on whitespace |
| 5 s health interval vs actrun's wait | actrun-only timing |

What GitHub caught that actrun could not:

| Defect | Why actrun did not report it |
| --- | --- |
| `env` in `jobs.<job_id>.if` | actrun accepts it; GitHub rejects the whole file |
| no-op `install-action` tag | actrun cannot run that action at all |
| musl target vs sanitizer | actrun cannot run cargo-fuzz |

## Not run

- `ORG-003` cutover: policy is decided in [ADR 0002](../adr/0002-p0-to-p1-cutover.md);
  no implementation or test exists (P1).
- Verus proof-of-concept (PDD §4): not attempted.
- Concurrent-writer stress beyond the single-writer model lock.
- `cargo machete` / `cargo deny` / license checks: no license is declared yet.
