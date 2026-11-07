# ADR 0001: Normalized model AST and storage separation

- Status: accepted (P0)
- Context: [ROADMAP.md](../ROADMAP.md) §1, §5 gate 1

## Decision

Author input is parsed into an unnormalized document, then normalized into a
`Model` AST whose collections are `BTreeMap`s. The AST is the only thing the
storage layer and the CLI ever see. Canonical JSON is emitted in *document*
shape, sorted, so the bytes an operator reads, the bytes they can re-apply, and
the bytes the digest covers are the same artifact.

## Why not store the author's file verbatim

Two documents that differ only in declaration order describe the same model.
If the stored definition were the raw file, their digests would differ and the
same model could be stored twice under one version. Normalizing first makes the
digest a function of meaning, not of formatting.

## Version compatibility

- A `(scope_id, model, version)` triple is immutable once stored.
- Re-applying an identical definition returns `Unchanged` and writes nothing.
- Re-applying a different definition under the same version is rejected with
  `model_version_conflict`; the stored version is never rewritten.
- Any compatible change requires a new version number. P0 has no migration
  planner; that is P4 work.

## Scope boundary

`scope_id` is part of every primary key (`models`, `entities`, `relations`,
`relation_roles`). A relation may only reference entities in its own scope; the
store checks this inside the write transaction and rejects with
`scope_mismatch`. P0 runs with a single scope but never stores a fact without
one, so introducing tenant isolation later cannot require guessing which tenant
an existing fact belonged to.

## Consequences

- The CLI can validate a model with no database at all.
- The digest is stable across hosts because `time_zone` is normalized to its
  IANA name and maps are ordered.
- A future P1 cutover can read `models.definition` and rebuild the same AST.
