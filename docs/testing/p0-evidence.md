# P0 evidence

Execution log for the P0 exit criteria in [ROADMAP.md](../ROADMAP.md) §3.
Only runs recorded here count as evidence; anything not listed is **not run**.

## Environment

| Item | Value |
| --- | --- |
| Date | 2026-11-07 |
| Commit | `34aa511` + uncommitted P0 implementation |
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
| CI workflow | `.github/workflows/ci.yml` | **Not run** (added in this change; first run happens on push) |

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

## Not run

- `ORG-003` cutover: policy is decided in [ADR 0002](../adr/0002-p0-to-p1-cutover.md);
  no implementation or test exists (P1).
- Verus proof-of-concept (PDD §4): not attempted.
- Concurrent-writer stress beyond the single-writer model lock.
- `cargo machete` / `cargo deny` / license checks: no license is declared yet.
