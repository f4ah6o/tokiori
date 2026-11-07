# Tokiori

**Temporal & Relational Data Engine — Relationships through time.**

Tokiori は PostgreSQL 上で、業務上の **Entity / Relation / Role / Operation** を宣言し、履歴管理と時点検索を一貫したモデルで扱うことを目指す OSS プロジェクトです。

- **Temporal:** 有効時刻と記録時刻を分離し、過去・未来・遡及訂正・当時の認識を問い合わせる。
- **Relational:** 所属などの関係を第一級の概念として扱い、多項関係・属性・制約を定義する。
- **Operational:** 異動などの業務操作と、その結果として生じる状態変更・イベントを監査可能に記録する。
- **PostgreSQL-first:** 利用者に業務ごとの履歴テーブルを手作業で設計させない。

## いまできること（P0）

モデルを宣言し、社員・部署・所属を**現在値として**登録・取得できます。業務ごとの SQL テーブルを書く必要はありません。

```sh
# 1. モデルを検証（DB 不要）
cargo run -p tokiori-cli -- model validate --file examples/organization/model.json

# 2. PostgreSQL 18 に適用（同一モデルの再適用は no-op）
cargo run -p tokiori-cli -- model apply --file examples/organization/model.json

# 3. 登録
tokiori entity create --type employee --id E001 --attributes '{"name":"Alice"}'
tokiori entity create --type department --id D-SALES --attributes '{"name":"Sales"}'
tokiori relation create --type membership --id M001 \
  --role employee=E001 --role department=D-SALES --valid-from 2027-04-01

# 4. 取得（別プロセスからでも同じ結果）
tokiori --format json relation get --type membership --id M001
```

不正な入力は**部分書込なしに**拒否され、安定したコードを返します:

| 入力 | コード |
| --- | --- |
| 未定義 entity を参照する role | `invalid_model` |
| employee ロールに Department を割り当てる | `entity_type_mismatch` |
| 別 scope の entity を参照する | `scope_mismatch` |
| 同一 ID の二重登録 | `duplicate_id` |
| 同一 version で異なる定義を apply | `model_version_conflict` |

詳しい手順は [examples/organization/README.md](examples/organization/README.md)。

## 想定する利用の流れ

1. 社員・部署・所属のモデルを宣言して、実体と関係を登録する（**P0: 実装済み**）。
2. 有効日と記録 revision を指定して当時の状態を調べる（P1）。P0 で記録していない変更履歴は復元せず、P1 への移行前の時点は「履歴不明」として区別する。
3. 異動を単一の業務操作として記録・再試行する（P2）。組織図などの横断照会を行う（P3）。
4. SDK / API / 読取専用 UI から複数業務モデルを利用する（P4）。

通常の参照は現在値を取得し、過去の確認が必要な場合だけ時点を明示する設計です。履歴テーブルを利用者が直接操作する必要はありません。

## 開発

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

# PostgreSQL 18 を必要とする受入テスト（ORG-001 / ORG-002）
docker run -d --name tokiori-pg -e POSTGRES_USER=tokiori -e POSTGRES_PASSWORD=tokiori \
  -e POSTGRES_DB=tokiori -p 55432:5432 postgres:18.6-alpine
TOKIORI_TEST_DATABASE_URL=postgres://tokiori:tokiori@localhost:55432/tokiori \
  cargo test --workspace --all-targets
```

`TOKIORI_TEST_DATABASE_URL` が未設定のとき受入テストは **skip** され、PASS としては数えません。

### CI をローカルで再現する

`.github/workflows/ci.yml` は [mizchi/actrun](https://github.com/mizchi/actrun) でも実行できます。

```sh
npx @mizchi/actrun workflow run .github/workflows/ci.yml --workspace-mode local
```

ローカル実行の差分（サービス接続先、未対応 action の代替）は `actrun.toml` にあります。
**actrun の PASS は GitHub Actions の PASS ではありません** — fuzz ジョブはローカルでは skip され、
GitHub だけがワークフロー全体の構文制約（例: `jobs.<job_id>.if` で `env` を参照できない）を検出します。
詳細は [ADR 0004](docs/adr/0004-local-ci-with-actrun.md)。

actrun は実行後にワークスペースを元に戻すため、**未コミットの変更はテストされません**。実行前にコミットしてください。

| Crate | 責務 |
| --- | --- |
| `tokiori-model` | モデル AST・正規化・検証・canonical JSON・半開区間と業務日付 |
| `tokiori-pg` | SQLx による永続化。1 操作 = 1 トランザクション |
| `tokiori-cli` | 最小 CLI（P4 の安定 CLI 契約ではない） |

## Roadmap

**[P0–P4 実装ロードマップ (docs/ROADMAP.md)](docs/ROADMAP.md)**

P0 宣言的モデル / 汎用 CRUD・最小 CLI → P1 Bitemporal Engine → P2 Operation / Event Engine → P3 Query Compiler → P4 安定 CLI / SDK / API / UI。

実装は **TDD** を基本とし、P0 から [sile/noprop](https://github.com/sile/noprop) による **PBT** と **cargo-fuzz** による **fuzzing** を継続します。さらに [Proof-Driven Development 方針](docs/PDD.md) に沿って、人が承認する仕様、Verus 形式検証の候補、信頼境界、PR の検証証拠を段階的に整備します（PDD/Verus は未実装）。各段階の受入条件・失敗再現・CI 方針、および実装で追跡する 7 件の受入シナリオ（`ORG-001`〜`ORG-007`）はロードマップに記載しています。

## Status

**P0 実装済み。P1 以降は未実装。**

- 実装済み: モデル宣言・検証・冪等な適用、entity / relation の登録・取得、最小 CLI、noprop PBT、parser fuzz、PostgreSQL 18 での `ORG-001` / `ORG-002`。
- 未実装: 時点照会（`ORG-003` / `ORG-004`）、異動操作（`ORG-005`）、横断クエリ（`ORG-006`）、API / SDK / UI（`ORG-007`）、YAML の正式サポート、Verus 証明。
- 実行した検証と未実行の検証は [docs/testing/p0-evidence.md](docs/testing/p0-evidence.md) に記録しています。

設計判断は [docs/adr/](docs/adr/) にあります。
