# ADR 0004: Local CI reproduction with actrun

- Status: accepted
- Context: [ROADMAP.md](../ROADMAP.md) §4, `.github/workflows/ci.yml`

## Decision

The workflow in `.github/workflows/ci.yml` is the single source of truth for
CI, and it must also run locally with [mizchi/actrun](https://github.com/mizchi/actrun).
`actrun.toml` in the repository root declares how the local run differs.

```sh
actrun workflow run .github/workflows/ci.yml --workspace-mode local
```

## Why actrun

It runs the real workflow file rather than a hand-written script, so a change
to CI is testable before it lands. It needs no cloud runner and no separate
workflow dialect.

## What actrun cannot do, and how the workflow adapts

| Limitation | Adaptation |
| --- | --- |
| No `SERVICE_HOST` / `SERVICE_PORT` for services | the acceptance job sets `TOKIORI_TEST_DATABASE_URL=...127.0.0.1...` when `ACTRUN_LOCAL` is set |
| Service `options` are split on whitespace | `pg_isready` takes no `-U`; the service env selects the user |
| Service health wait is shorter than a 5 s interval | the interval is 3 s |
| Folded scalars (`>-`) in service `options` fail to parse | `options` stays on one line |
| `${{ }}` inside `with:` is not resolved | `actions/cache` is overridden locally |
| `dtolnay/rust-toolchain` is not implemented | overridden to report the local toolchain |
| `taiki-e/install-action` cannot install cargo-fuzz | overridden to a no-op; the fuzz steps are guarded by `ACTRUN_LOCAL` |
| The workspace is restored after a run | commit before running; uncommitted edits are not what gets tested |

## What GitHub does that actrun accepts and rejects

`jobs.<job_id>.if` may only use `github`, `needs`, `vars`, and `inputs`.
actrun accepts `env` there, GitHub rejects the **whole file** and no job runs.
This was the first failure on PR #2 and it was invisible to the local lint.

## Consequences

- A local pass is **not** a GitHub pass. Both are recorded separately in
  [p0-evidence.md](../testing/p0-evidence.md), and a claim of "CI passes" must
  name which runner produced it.
- The fuzz job is skipped locally. Local fuzz evidence comes from running
  `cargo fuzz` directly, not from actrun.
- Workflow edits that only actrun needs must keep working on GitHub; the
  guards use `ACTRUN_LOCAL`, which GitHub never sets.
- `actrun.toml` is committed so the local run is reproducible, and it is the
  only place that describes the differences.
