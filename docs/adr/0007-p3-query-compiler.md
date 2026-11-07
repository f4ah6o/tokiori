# ADR 0007: Typed queries over relationships and time

- Status: accepted (P3)
- Context: [ROADMAP.md](../ROADMAP.md) §5 gate 4, [ADR 0005](0005-p1-bitemporal-engine.md)

## Decision

A caller describes what to find as a value -- a start, a path of roles, a
select, an instant, and bounds -- and the engine compiles it to parameterized
SQL. The caller never writes a join, and never supplies a SQL identifier.

## Why validate before compiling

Every name in a query must resolve against the model: the start type, each
relation, each role, and any selected attribute. Each hop must also continue
from the type the previous hop reached.

Validating first is what makes the compiler safe. It only ever interpolates
names that came from the model, so user text cannot become an identifier. The
injection test puts `'; DROP TABLE entities; --` in an id and asserts both
that the SQL is unchanged and that the text is carried as a parameter.

## Why there is no unbounded traversal

A query states a depth and a row budget, and asking for more than the maximum
is rejected rather than clamped. A traversal that exceeds its depth is
refused; one that exceeds its row budget returns what it found and sets
`truncated`, so the caller can tell "that is all" from "that is all I asked
for".

Cycles need no special case: the depth bound stops them, and each hop is a
distinct pair of rows.

## Two implementations on purpose

The compiled path and a naive interpreter answer the same question. The
interpreter walks one hop at a time through the existing read path; the
property test compares them on random queries. A bug in the compiler cannot
hide behind an identical bug in the interpreter because they share no code.

## What the comparison found

Three rules had to be made explicit before the two agreed:

1. **Several versions can cover the same date.** Once closing a version stops
   rewriting its valid interval (ADR 0005's principle), a correction leaves
   both the old and the new version covering the date. The query must pick the
   newest knowledge, exactly as the read path does.
2. **A relation's roles live in two tables.** `create_relation` writes the
   current-state table for the P0 read path and the versioned table for the
   temporal path; a query traverses the latter.
3. **A selected attribute belongs to the type the traversal reached**, not to
   the start type.

## Consequences

- `known_at` remains unpublished, so a query names a revision.
- The org chart is built from the same compiled queries, so it cannot drift
  from a hand-written query about the same instant.
- Performance is bounded by the depth and row limits rather than by the shape
  of the data.
