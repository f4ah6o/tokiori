# Tokiori

**Temporal & Relational Data Engine — Relationships through time.**

Tokiori は PostgreSQL 上で、業務上の **Entity / Relation / Role / Operation** を宣言し、履歴管理と時点検索を一貫したモデルで扱うことを目指す OSS プロジェクトです。

- **Temporal:** 有効時刻と記録時刻を分離し、過去・未来・遡及訂正・当時の認識を問い合わせる。
- **Relational:** 所属などの関係を第一級の概念として扱い、多項関係・属性・制約を定義する。
- **Operational:** 異動などの業務操作と、その結果として生じる状態変更・イベントを監査可能に記録する。
- **PostgreSQL-first:** 利用者に業務ごとの履歴テーブルを手作業で設計させない。

## いまできること（P0・P1・P2・P3）

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

### 時点と、当時の認識（P1）

履歴は上書きされません。変更は新しい revision として記録され、古い revision は
その時点で知っていた内容を返し続けます。

所属の部署は**ロール**なので `role` で記録します（`attribute` は名前などの属性用）。

```sh
# 起点: 4/1 から営業部（r1）
tokiori role set --type membership --id M001 --role department \
  --participant D-SALES --valid-from 2027-04-01

# r2: 7/1 付の異動を記録
tokiori role set --type membership --id M001 --role department \
  --participant D-PLANNING --valid-from 2027-07-01

# r3/r4: 発効日を 7/15 に遡及訂正する。訂正は空いた期間も述べる必要がある
tokiori role set --type membership --id M001 --role department \
  --participant D-SALES --valid-from 2027-07-01 --valid-until 2027-07-15
tokiori role set --type membership --id M001 --role department \
  --participant D-PLANNING --valid-from 2027-07-15

# 7/10 の所属を、訂正前（r2）と訂正後（r4）で照会する
tokiori role get --type membership --id M001 --role department \
  --valid-at 2027-07-10 --known-revision 2   # D-PLANNING
tokiori role get --type membership --id M001 --role department \
  --valid-at 2027-07-10 --known-revision 4   # D-SALES
```

> 訂正が空いた期間を述べるのは利用者の責任です。r3 で「7/1〜7/14 は営業部だった」と
> 記録しない限り、その期間は r2 の認識（企画部）のままになります。

`--known-revision` を省略すると最新の revision で答えます。存在しない revision は
`unknown_revision` で拒否され、`not found` とは区別されます。

P0 の現在値は `tokiori cutover --at <DATE>` で最初の revision として取り込めます。
取り込み前の日付は「知らない」であって「存在しなかった」ではありません。

### 業務操作としての異動（P2）

異動は「状態を書き換える」のではなく「何を実行したか」として記録されます。
同じ `--request-id` を再送しても一度しか適用されず、同じキーで違う内容を送ると
`idempotency_conflict` になります。

```sh
tokiori operation transfer --employee E001 --to-department D-PLANNING \
  --effective-on 2027-07-01 --request-id req-1 --reason 'annual rotation' --actor alice

# 同じリクエストの再送: applied=true ではなく replayed になり、revision も同じ
tokiori operation transfer --employee E001 --to-department D-PLANNING \
  --effective-on 2027-07-01 --request-id req-1 --reason 'annual rotation' --actor alice

# 取消は過去を消さず、元に戻す新しい版を記録する
tokiori operation cancel --transfer-request-id req-1 --cancel-request-id cancel-1 \
  --reason 'mistaken transfer' --actor bob
```

状態変更・操作の台帳・outbox イベントは**同一コミット**で書かれます。途中で失敗すれば
3 つとも残りません。イベント配信は at-least-once で、購読側は `event_id` で重複排除します。

```sh
tokiori --format json outbox claim --limit 100
tokiori outbox ack --event-id <uuid>
```

### 関係をたどる（P3）

SQL の join を書かずに、関係と時点を指定して問い合わせられます。

```sh
# 社員が所属する部署（7 月 10 日時点）
tokiori query run --from-type employee --from E001 \
  --relation membership --path employee=department --valid-at 2027-07-10

# ある時点の組織図
tokiori query org-chart --valid-at 2027-07-10
```

`--known-revision` を付ければ、その時点で知っていた内容で答えられます。

- **探索は無制限ではありません。** `--max-depth`（既定 4、上限 8）と `--max-rows`（既定 1000、上限 10000）を必ず持ち、上限超過は拒否、行数打ち切りは `truncated` で明示します。
- **値はすべてパラメータとして束縛されます。** SQL 識別子は検証済みモデルからだけ生成し、利用者入力は構造に入りません。
- 未定義の role・関係・属性はコンパイル前に拒否されます。

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

**P0–P3 実装済み。P4 は未実装。**

- 実装済み（P0）: モデル宣言・検証・冪等な適用、entity / relation の登録・取得、最小 CLI、noprop PBT、parser fuzz、PostgreSQL 18 での `ORG-001` / `ORG-002`。
- 実装済み（P1）: 有効時刻と記録 revision の二時点履歴、遡及訂正、将来発令、tombstone、P0 からの cutover、`ORG-003` / `ORG-004`。
- 実装済み（P2）: 異動・取消の操作、冪等な再送と conflict、単一コミットの outbox、at-least-once 配信、`ORG-005`。
- 実装済み（P3）: 型付き Query AST、パラメータ化 SQL コンパイラ、探索の上限と循環停止、時点指定の組織図、`ORG-006`。
- 未実装: API / SDK / UI（`ORG-007`）、`known_at`、exactly-once 配信、EXPLAIN に基づく性能計測、Verus 証明。
- 実行した検証と未実行の検証は [docs/testing/p0-evidence.md](docs/testing/p0-evidence.md) と [docs/testing/p1-evidence.md](docs/testing/p1-evidence.md) に記録しています。

設計判断は [docs/adr/](docs/adr/) にあります。
