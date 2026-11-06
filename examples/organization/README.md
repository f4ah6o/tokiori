# Organization example (P0)

Employees, departments, and memberships declared as a model, then registered
and read back without writing any business table by hand.

## 1. Start PostgreSQL 18

```sh
docker run -d --name tokiori-pg \
  -e POSTGRES_USER=tokiori -e POSTGRES_PASSWORD=tokiori -e POSTGRES_DB=tokiori \
  -p 55432:5432 postgres:18.6-alpine
export DATABASE_URL=postgres://tokiori:tokiori@localhost:55432/tokiori
```

## 2. Validate the model (no database needed)

```sh
cargo run -p tokiori-cli -- model validate --file examples/organization/model.json
```

`model.yaml` is the same model in YAML; both produce the same digest.

## 3. Apply the model

```sh
cargo run -p tokiori-cli -- model apply --file examples/organization/model.json
```

Re-applying the same model prints `unchanged`. Changing the model under the
same `version` is rejected: raise the version instead.

## 4. Register the golden scenario

```sh
cargo build -p tokiori-cli
TOKIORI=target/debug/tokiori ./examples/organization/seed.sh
```

Every command writes to the scope named by `--scope` (default `default`).
`seed.sh` reads `TOKIORI_SCOPE`, so a second run can use a fresh scope:

```sh
TOKIORI_SCOPE=demo TOKIORI=target/debug/tokiori ./examples/organization/seed.sh
```

Or step by step:

```sh
tokiori entity create --type employee --id E001 --attributes '{"name":"Alice","joined_on":"2027-04-01"}'
tokiori entity create --type department --id D-SALES --attributes '{"name":"Sales"}'
tokiori relation create --type membership --id M001 \
  --role employee=E001 --role department=D-SALES \
  --attributes '{"title":"Account manager"}' --valid-from 2027-04-01
```

## 5. Read it back

```sh
tokiori --format json entity get --type employee --id E001
tokiori --format json relation get --type membership --id M001
```

## What is rejected

Each of these exits non-zero and writes `{"code": ..., "message": ...}` to
stderr, and leaves no rows behind:

| Attempt | Code |
| --- | --- |
| A role referencing an undeclared entity | `invalid_model` |
| A department in the `employee` role | `entity_type_mismatch` |
| A participant from another scope | `scope_mismatch` |
| Registering `E001` twice | `duplicate_id` |
| An attribute the model does not declare | `unknown_attribute` |
| Writing before the model is applied | `model_not_found` |
| Same version, different definition | `model_version_conflict` |

## What P0 does not do

This example registers **current state**. There is no way to ask "who was in
Sales on 2027-06-30" yet: point-in-time queries and the transfer operation are
P1 and P2. See [../../docs/ROADMAP.md](../../docs/ROADMAP.md).
