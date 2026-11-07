# ADR 0003: P0 verification boundary

- Status: accepted (P0)
- Context: [PDD.md](../PDD.md), [ROADMAP.md](../ROADMAP.md) §2

## Classification

| Requirement | Class | Evidence |
| --- | --- | --- |
| Model normalization is deterministic and order-independent | empirically-checked | unit tests + noprop 1,000 cases |
| A role referencing an undefined entity is rejected | empirically-checked | unit test, PBT injection |
| Canonical form round-trips | empirically-checked | PBT + fuzz |
| Half-open interval membership | empirically-checked | unit tests |
| Role/scope checks on write | empirically-checked | PostgreSQL E2E (ORG-001) |
| Model apply idempotency | empirically-checked | PostgreSQL E2E (ORG-002) |

Nothing in P0 is `formally-proven`. The Verus proof-of-concept described in
PDD §4 is **not executed**; it is not a P0 completion condition.

## Trusted computing base

- `serde_json` / `serde_yaml_ng`: input decoding.
- `chrono-tz`: IANA zone data and DST rules.
- PostgreSQL: ACID, unique constraints, foreign keys, `CHECK` on the valid
  interval.
- `sqlx`: statement preparation and parameter binding.

Each assumption has a test that would fail if it broke: unknown fields are
rejected (decoder strictness), DST gaps are rejected (zone data), duplicate ids
and cross-scope references are rejected by real SQL (database constraints),
and every value is bound as a parameter rather than interpolated.

## What is not covered

- Concurrency beyond the single-writer lock on the latest model row.
- Crash-during-commit recovery (relies on PostgreSQL).
- Any temporal query semantics; those are P1.
