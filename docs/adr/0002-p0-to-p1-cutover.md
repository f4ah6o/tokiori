# ADR 0002: P0 to P1 cutover (ORG-003)

- Status: accepted (P0), implemented in P1
- Context: [ROADMAP.md](../ROADMAP.md) §1 "P0 → P1 の移行契約", §5 gate 1

## What P0 guarantees

P0 stores **current state only**.

- `entities` holds one row per `(scope_id, entity_type, id)` with no history.
- `relations` holds `valid_from` / `valid_until`, so the *effective* interval of
  a relation is known, but **every change to it overwrites nothing and is not
  recorded**: P0 has no update path, only create.
- There is no recorded revision. Nothing in P0 can answer "what did the system
  believe on date D".

## Cutover rule

When P1 is introduced:

1. Stop P0 writes (or hold a compatible lock) for the scope.
2. Take a consistent snapshot of `entities`, `relations`, and `relation_roles`
   for that scope and fix the cutover instant `T0`.
3. Import the snapshot as the initial history of the first committed revision
   `R0` for that scope.
4. Release P1 writes.

## What history is *not* invented

- For entities, P0 records no effective start, so the only defensible valid
  range for imported facts is `[T0, ∞)`.
- For relations, `valid_from`/`valid_until` are preserved as-is because P0 did
  record them.
- A historical query for a time before `T0` (or a `known_revision` before `R0`)
  must return `HistoryUnavailable`, never `NotFound`. "We do not know" and "it
  did not exist" are different answers and must not be conflated.
- No `known_revision` for a time before `R0` may be fabricated.

## Re-run safety

The cutover must be idempotent: if it is interrupted after step 2 but before
step 3 commits, re-running must produce the same `R0` contents and must not
create a second initial revision. `ORG-003` in P1 must demonstrate that a
partial cutover never leaves two writers active for one scope.
