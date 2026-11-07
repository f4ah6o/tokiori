# ADR 0005: Bitemporal engine

- Status: accepted (P1)
- Context: [ROADMAP.md](../ROADMAP.md) §5 gate 2, [ADR 0002](0002-p0-to-p1-cutover.md)

## Decision

A fact is never overwritten. Every change closes the version it replaces and
inserts a new one in the same transaction, under a revision number that is
monotonic within a scope.

Two intervals are stored side by side:

| Interval | Meaning |
| --- | --- |
| `[valid_from, valid_until)` | when the value was in effect in the business |
| `[recorded_from, recorded_to)` | which revisions knew it as the current value |

## Revision ordering and locking

- A revision is allocated from `scope_revisions` under `SELECT ... FOR UPDATE`.
  Writers in one scope therefore serialize on that row.
- The allocation and the facts it describes commit together, so a revision
  never exists without its facts.
- **Monotonicity is the contract, not a gap-free sequence.** A writer that
  rolls back leaves a hole; a hole is harmless, a duplicate is not.
- Isolation is the PostgreSQL default (`READ COMMITTED`). The row lock, not
  the isolation level, is what makes ordering correct.

## Choosing a version

The choice is made by `tokiori_model::resolve`, a pure function, and the read
path calls it rather than re-implementing the rule in SQL:

1. A version is visible when `recorded_from <= R` and it was not replaced
   before `R` (`recorded_to` is absent or `>= R`).
2. Among visible versions whose valid interval contains the date, the one with
   the **highest** `recorded_from` wins. A correction leaves the older version
   visible at its own revision, so both can match; the newer knowledge must win.
3. No match means `NotFound`; no match *and* `R` predates everything recorded
   means `HistoryUnavailable`. These are different answers.

## Why `known_at` is not published

The roadmap allows mapping a wall-clock `known_at` to a revision only if that
mapping is demonstrated. It is not: the revision counter advances when a
writer commits, and nothing records the commit instant in a way that survives
clock skew or a retry. Publishing `known_at` would imply a guarantee the
storage cannot make, so P1 exposes `known_revision` only.

## Why PostgreSQL `WITHOUT OVERLAPS` is not used

`WITHOUT OVERLAPS` constrains one interval per key. A bitemporal table has two
intervals, and the constraint cannot express "no two versions with the same
recorded interval overlap in valid time". Applying it to the valid interval
alone would forbid the older versions a correction must keep. Integrity is
therefore enforced by the write path (which closes before it inserts) and by
`CHECK` constraints on each interval's endpoints.

## Corrections must restate the vacated period

A correction that moves an effective date must also record what was true in
the period it vacates. The engine cannot infer it: if r10 says the move
happened on 7/1 and r11 says 7/15, r11 must also say the old value continued
to 7/15. Recording only the new date leaves the vacated period describing the
superseded belief. This is a property of the intent, not a limitation of the
storage, and `ORG-004` exercises it.

## Consequences

- Reading at the newest revision and reading at a named revision use the same
  code path.
- A revision that was never assigned in the scope is rejected as
  `unknown_revision` rather than answered with `NotFound`.
- The pure resolver is compared against an independent reference in the
  property tests, and the read path calls the tested function.
