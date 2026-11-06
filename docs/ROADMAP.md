# Tokiori: P0–P4 実装ロードマップ

- 状態: **計画 (未実装)**。本書のコマンド、API、パス、テスト名は実装予定の契約であり、現在動作することを意味しない。
- 対象: PostgreSQL を永続化先とする汎用 Temporal + Relationship データモデリングエンジン
- 方針: TDD を実装の基本サイクルにし、sile/noprop による PBT と cargo-fuzz による fuzzing を P0 から継続する。
- 進行: **P0 → P1 → P2 → P3 → P4**。期間ではなく、再現可能な受入基準で次段階へ進む。

## 0. 北極星・スコープ

### 解決したい問題

業務概念が増えるたびに、エンティティ、所属などの関係、変更履歴、異動などの操作、監査ログのテーブルを手動で増やし続けたくない。開発者は **何が存在し、どのような関係があり、どう変化し得るか** を宣言する。Tokiori が PostgreSQL 上の物理表、時点管理、クエリ、変更記録を吸収する。

**目指す操作面（完成形の概念例・構文未確定）:** モデルを一度宣言し、社員と部署を登録する。異動は `Transfer(employee=E001, to=Planning, effective_on=2027-07-01, request_id=req-1)` と伝える。通常の取得では現在の所属を返し、調査時だけ `membership(E001, valid_at=2027-07-10, known_revision=r10)` のように照会する。利用者は履歴表・履歴行・イベント行を手動管理しない。**P0 は登録、P1 は時点照会、P2 は Transfer、P3 は横断検索、P4 は外部 API** を担当するため、これは現行 API の使用例ではない。

- **XTDB に学ぶ**: 有効時刻 (valid time) と記録時刻 (system/recorded time) を分け、ある時点の事実と「当時どう認識していたか」を問い合わせる。
- **TypeDB に学ぶ**: Entity、Relation、Role を第一級の概念として扱う。二者関係だけでなく、多項関係と関係の属性を表現する。
- **PostgreSQL を使う**: 一般的な PostgreSQL 18 を前提とし、専用サーバー、特殊な PostgreSQL 拡張のインストール、外部の XTDB/TypeDB インスタンスを必須にしない。必要なら PostgreSQL 付属の btree_gist 拡張を有効化する。

これは XTDB や TypeDB の API/ストレージ互換実装ではない。両者のアイデアを取り込んだ **Tokiori 独自の契約** を作る。

### 最初のゴールデンシナリオ

組織・社員・所属・異動をサンプルとして、ユーザーが履歴テーブルを定義せずに次を行えること。

1. 社員 E001 が 4 月 1 日から営業部に所属する。
2. 7 月 1 日付の企画部への異動を、事前または事後に記録する。
3. 6 月 30 日 / 7 月 1 日の所属と組織図を再現できる。
4. 遡及訂正後も **訂正前にシステムが知っていた所属** を再現できる。
5. 異動命令の ID、実行者、理由、リクエスト ID と、実際の関係変更を追跡できる。
6. 同じリクエストの再試行や競合する異動で、履歴が二重化・破損しない。

**二時点の最小受入 fixture (合成例):** revision `r10` では 7 月 1 日付で営業部 → 企画部へ異動すると記録し、`r11` で発効日を 7 月 15 日に遡及訂正する。日付はモデルの業務タイムゾーンで解釈する。

| 業務上の照会日 (valid) | `known_revision=r10` | `known_revision=r11` |
| --- | --- | --- |
| 7 月 10 日 | 企画部 | 営業部 |
| 7 月 16 日 | 企画部 | 企画部 |

現在値だけではこの違いは再現できない。P1 の受入では**訂正前の知識を維持**し、どの revision で照会したか明示する。

### 非目標 (P0–P4)

- XTDB のクエリ互換、TypeDB の推論エンジンの再現
- 完全なイベントソーシングへの移行、状態の再構築をイベントだけに依存させる設計
- 業務上の「異動」を、関係変更から**自動的に推論**すること (操作は明示的に定義する)
- グラフ分散処理、全文検索、任意規模の OLAP の最適化
- 外部購読者まで含む exactly-once イベント配信

### フェーズ早見表

| フェーズ | 利用者に新しくできること | 次段階へ進む証拠 |
| --- | --- | --- |
| **P0** モデル | Entity / Relation / Role を宣言して保存・取得 | PostgreSQL でモデル適用・CRUD・不正ロール拒否 |
| **P1** 時間 | 有効日と記録時点を指定し、訂正前も照会 | 二時点 fixture、競合、rollback、再起動 |
| **P2** 操作 | 異動・取消を原子的に適用し、意図を追跡 | 冪等性、失敗注入、outbox 再送 |
| **P3** 関係クエリ | SQL を書かずに時点組織図を取得 | 参照モデルとの一致、注入・探索上限 |
| **P4** 利用・配布 | SDK / API / 読取 UI で複数業務モデルを利用 | 権限境界、契約互換、復元、配布 E2E |

**通過規則:** 各フェーズの DoD は固定 fixture と実行ログで証明する。未実行・timeout・flaky は PASS にしない。後続変更で影響を受ける先行ゲートも再実行する。フェーズ名は完成度のラベルではなく、**その段階で利用者に保証できる契約**を指す。P0 の簡易 CLI と P4 の公開 CLI、P1 の個別時点参照と P3 の横断クエリを混同しない。

### 受入シナリオ台帳（実装予定）

以下の ID を仕様・テスト・PR・CI ログで共通に使う。fixture パスは予定であり、現時点にファイルや実行結果は存在しない。

| ID | 初めて通す段階 | 入力・操作 | 観測すべき結果 |
| --- | --- | --- | --- |
| `ORG-001` | P0 | Employee / Department / Membership を宣言・登録・取得。不正 role と別 scope 参照も試す | 正常な関係だけ保存され、不正入力は部分書込なしに拒否 |
| `ORG-002` | P0 | clean DB → model apply → 同一 model 再 apply → 再起動 → get | 同一 ID/属性/関係を返し、冪等である |
| `ORG-003` | P1 | P0 現在値を cutover し、cutover 前後で時点照会 | 導入以前の履歴は「不明」であり、存在しなかったと偽装しない |
| `ORG-004` | P1 | `r10` の 7/1 異動を `r11` で 7/15 に訂正。競合と rollback も試す | 冒頭の 2×2 表を再現し、部分 commit・矛盾 revision がない |
| `ORG-005` | P2 | Transfer の同一キー再送・同一キー異入力・途中失敗・outbox 再送 | 一度だけ状態更新、異入力 conflict、再送可能、購読側重複排除 |
| `ORG-006` | P3 | 二時点組織図クエリを循環・多項関係と共に実行 | 参照インタプリタと一致し、探索上限と scope を守る |
| `ORG-007` | P4 | 2 業務モデルを CLI / SDK / API / 読取 UI で照会。越境 ID と backup/restore も試す | 同一結果、越境拒否、復元後に同一の履歴が読める |

**証拠の保存規約:** 各 ID に (1) テスト名と fixture、(2) Red/Green または初回 PASS のログ、(3) 実行したコミット・PostgreSQL/ツールチェーン、(4) PBT の seed/件数、(5) fuzz の target/corpus/終了理由、(6) 未実行・失敗した項目を紐づける。P0 で `ORG-004` の動作を主張するなど、後段の契約を前倒しで「実装済み」と表示しない。

## 1. 技術スタックと責務

| レイヤー | 初期採用 | 責務 |
| --- | --- | --- |
| 言語 / ワークスペース | Rust stable / Cargo workspace | モデル・時間・実行エンジンを共有 |
| 永続化 | PostgreSQL 18 / SQL マイグレーション | ACID、範囲型、GiST、JSONB、ロック |
| DB 接続 | SQLx + Tokio | パラメータ化 SQL、明示的トランザクション |
| モデル定義 | 宣言的 JSON/YAML → 正規化された Rust AST | バージョン管理、型検証、意味検証 |
| クエリ | 型付き AST → パラメータ化 PostgreSQL SQL | 時点・関係の検索と計画の検査 |
| CLI | clap | 初期化、モデル適用、実行、クエリ、診断 |
| HTTP API (P4) | Axum + serde | 認証・認可境界、外部連携 |
| SDK (P4) | TypeScript、Python | 言語ごとの利用者向け API |
| 管理 UI (P4) | React / React Flow | モデル図・組織図・履歴の表示 |
| 検証 | cargo test / sile/noprop / cargo-fuzz | 単体・状態遷移・異常系 |
| DB 統合テスト | PostgreSQL 18 の隔離されたインスタンス | 実 SQL、競合、ロールバック、マイグレーション |

YAML は人間向け入力、正規化された AST/JSON は安定した内部表現とする。初期から独自言語パーサーを作らない。公開 DSL 構文と内部 SQL スキーマは独立して進化させる。

### 物理モデルの初期候補 (実装前に検証して決定)

- models / model_versions: モデル定義と互換性
- entities: 型付き実体の安定した ID とデータスコープ
- relations / relation_roles: 関係 ID と役割の参加者 (多項関係)。役割参加者の履歴も P1 の対象
- fact_versions: エンティティ属性・関係属性・役割の有効時刻と記録履歴
- operations / operation_effects: 操作の意図、冪等性キー、状態への効果
- outbox_events: コミットと同一トランザクションで発行する配信待ちイベント

**物理テーブル数を 3 に固定しない**。減らすべきは開発者が業務ごとに手設計するテーブル数であり、整合性のための内部テーブルは許容する。JSONB は柔軟な属性表現に使い、ID、時間範囲、ロール、テナント境界など重要な制約を全て JSONB のみに押し込まない。

**モデル制約の区別:** 「Membership に employee role が 1 つ必要」という **1 件の関係内の role cardinality** と、「ある社員の同時点の主所属は 1 つ」という **関係間の時間制約** は異なる。前者は P0 のモデル検証対象、後者は P1 の時間的整合性対象。兼務や複数所属を一般エンジン側で勝手に禁止しない。

**後付けしない境界:** P0 の永続化キーと操作 API に `scope_id` (将来のテナント境界) と `model_version` を含める。P0 は単一スコープ運用でよいが、異なるスコープの ID や参照を混在させない。P4 の認証・認可導入時に既存の事実へスコープを推測して付与する移行を避ける。

**P0 → P1 の移行契約:** P0 は現在状態の CRUD とし、P1 導入前の変更について二時点監査を保証しない。P1 導入時は P0 の確定状態を初期履歴として取り込み、それ以前の `known_revision` を捏造しない。既存 ID・`scope_id`・`model_version` を保ち、再実行・中断・rollback を実 DB で検証する。

- **cutover の境界:** スコープごとに一貫した P0 スナップショットと cutover 時刻 `T0` を確定し、P1 の最初の committed revision `R0` に取り込む。P0 が有効開始日を保持していなければ、移行した事実の保証可能な valid 範囲は原則 `[T0, ∞)` とする。`T0` より前の所属・存在・削除履歴を推測しない。別の有効開始日を採用する場合は外部根拠と明示的な migration 方針が必要。
- **「不明」と「不在」の区別:** cutover より前の未記録期間に対する historical query は `HistoryUnavailable` 相当の明示的な結果を返し、対象が存在しなかったことを表す `NotFound` と混同しない。スコープの `R0` より前の記録時点への照会も同様。エラー名は API 設計時に確定する。
- **切替・再実行:** P0 書込を停止または互換なロックで遮断し、スナップショット取得・初期 revision 確定・P1 書込解禁を整合した手順にする。途中失敗時に P0 と P1 の二重 writer が成立しないことを `ORG-003` で検証する。

### 時間・整合性の基本契約

- 有効期間は半開区間 **[from, to)**、時刻は UTC で保存し、表示タイムゾーンと区別する。無期限の終端を扱い、空期間は拒否する。
- 日付のみの業務入力 (例: 7 月 1 日付の異動) は、モデルが宣言する IANA 業務タイムゾーンに従って時刻境界へ変換する。暗黙のサーバーローカル時刻変換はしない。日付型を直接保持するかは P1 ADR で決める。
- 有効時刻は業務上の効力発生日、記録時刻はシステムがそれを記録・認識した時点。両者を混ぜない。
- 履歴上の訂正は過去バージョンを不可視化するのではなく、**新しい知識として新バージョンを記録**する。記録済み内容の通常 UPDATE/DELETE は許さず、システム期間を閉じるメタデータ更新だけを厳格に許容する設計を比較する。
- 「ある時点に知っていた内容」は、**確定した変更の順序**を一意に示す **recorded revision** により決定論的に再現する。revision は **`scope_id` ごとに単調増加**し、スコープをまたいで比較しない。事実・関係の役割・操作効果・outbox は同一 revision の整合したスナップショットとして読む。単なる採番順やトランザクション開始時刻をコミット順とみなさず、revision の連番に欠番がないことも公開契約にしない。
- P1 の正準クエリ引数は `known_revision`。`known_at` (壁時計) は確定 revision への写像と同時刻のタイブレーク規則を ADR とテストで決めてから公開する。記録時刻を DB コミット時刻と無条件に同一視しない。
- P1 はまず書き込みの記録順序を直列化する等の単純で正しい方式を採り、測定後に細粒度化する。**スコープ別 revision head 行のロック → 最新状態と事前条件の再検証 → revision 付与 → 履歴反映 → commit** を初期案として ADR で固定する。待機前の古いスナップショットを再利用しないよう分離レベル・競合再試行方針を定め、rollback・再起動・読取中の同時書込を実 DB で検証する。
- 「所属は同期間に 1 つまで」等の制約はモデルごとに明示する。兼務等はモデルを変更して許容できる。**異なる revision に属する同じ有効期間の版は重なってよい**が、同一確定スナップショット内の制約違反は不可。
- 参加者の失効・削除と関係の有効期間の整合性は P1 で検証する。孤児関係を暗黙に残したり、自動 cascade で履歴を消したりしない。
- PostgreSQL 18 の WITHOUT OVERLAPS / PERIOD は**有効時間**の整合性に利用可能だが、二時点履歴の記録時刻管理まで自動提供するわけではない。全バージョンへ無条件に WITHOUT OVERLAPS を適用しない。

### 読取 API の最小契約（P1 から段階導入）

- `get(id)` は **最新の確定 revision とリクエスト基準時刻の valid time** の状態を返す。レスポンスに解決済み `valid_at`、`known_revision`、`model_version` を含め、同じ照会を再生可能にする。テストでは時計を固定する。
- `get(id, valid_at=V, known_revision=R)` は **R の確定時点に認識されていた V の状態**を返す。未確定・現在の `scope_id` に存在しない revision は明示的に拒否し、結果が存在しない場合と区別する。
- 複数関係を一画面で読む場合も、同じ `scope_id` と `known_revision` の整合した DB スナップショットを維持する。
- `known_at` は壁時計から revision への写像が実証できた場合のみ公開する **追加機能**。P1 の必須ゲートは `known_revision` とし、日付境界・DST・曖昧/存在しないローカル時刻を固定 fixture で検証する。

## 2. 開発規律: TDD / PBT / fuzzing を初日から

### TDD の必須サイクル

1. **Red**: 要件を「実行例 + 失敗するテスト」で表す。テスト名は **何を保証するか (What)** を記述する。
2. **Green**: 必要最小限の実装で通す。実装は処理の **方法 (How)** を示す。仕様をコードコメントに転記しない。
3. **Refactor**: 不変条件・公開 API を維持して整理。コードコメントが必要なら **なぜ別の実装を採らないか (Why not)** を残す。
4. **Regression**: 発見された障害は最小の再現入力を固定し、修正後は常に回帰テストとして残す。
5. **Commit**: コミットメッセージは変更を必要とした **理由 (Why)** を説明する。

各 PR の本文は **契約 / Red 証拠 / Green 証拠 / 回帰 / 未検証** を区別する。「未実行」を PASS としない。型チェックだけで完了としない。

### PBT: sile/noprop (採用)

- リポジトリ: https://github.com/sile/noprop
- ドキュメント: https://docs.rs/noprop/latest/noprop/
- 乱数 seed を明示的に受け取り、失敗時に **seed、ケース数、失敗ケース番号、入力 trace、モデルバージョン、PostgreSQL バージョン、Tokiori のコミット** を記録する。再実行には同じ seed と元のケース数、同一の generator/property、外部設定が必要。CI は固定 seed を基準にし、新 seed の探索を追加する。
- 通常 PR は各対象プロパティ **1,000 ケース**を基準に開始し、夜間は **10,000 ケース**へ拡大。実行時間を測定して継続的に調整する。重要な境界はランダム性に頼らず固定例も用意する。
- noprop は組み込みの **自動 shrinking / 永続失敗 corpus** を提供しない。失敗 trace は Tokiori 側で保存し、まず手動で最小 witness に縮小して通常の regression fixture に固定する。独立した自動縮小器は必要性を測定してから実装する。noprop のテストは `panic=unwind` を維持する。
- 状態遷移 PBT は **純粋な参照モデル**と PostgreSQL 実装の結果を比較する。モデルと実装が同一ロジックを共有して誤りを隠さない。
- テストが見逃した生成領域を把握するため、空期間、境界一致、遡及訂正、同時刻、循環関係、兼務、ロール欠落などを明示的にサンプリングする。

重要なプロパティ (P0–P3 で段階導入):

| 性質 | 内容 |
| --- | --- |
| Model validity | 未定義型・ロール、重複 ID、矛盾するモデルが拒否される |
| Round-trip | モデル AST の serialize → deserialize が意味を保存する |
| Interval normalization | 時間範囲の分割・正規化が観測可能な状態を変えない |
| No overlap | 単一所属制約に反する同期間の所属が確定しない |
| As-of determinism | 同一 valid 時刻 / recorded revision なら同一状態 |
| Correction preservation | 遡及訂正後も訂正前の認識を取得できる |
| Transaction atomicity | エラー時に部分効果、operation 記録、outbox が残らない |
| Idempotency | 同一操作キーの再試行で二重適用されない |
| Query equivalence | AST クエリと単純な参照インタプリタが一致する |
| Graph integrity | 関係の役割と参加者が型に合い、孤児関係を作らない |

### fuzzing: cargo-fuzz / libFuzzer (採用)

- fuzz ターゲットは Rust の純粋関数または隔離したテスト環境から始める。
- 製品の通常 CI は **Rust stable**、`cargo-fuzz` は **別ジョブで日付指定の nightly を固定** で実行する。製品 crate に nightly を要求しない。
- **P0:** モデル JSON/YAML 入力、正規化 AST、範囲値、スキーマ検証
- **P1:** バージョン履歴の状態遷移列、遡及訂正、再構成と参照モデルの差分
- **P2:** Operation コマンド列、重複キー、失敗・取り消し・再試行
- **P3:** クエリ AST / フィルタ / 経路探索、型検査・SQL コンパイル
- **P4:** API 入力、SDK シリアライズ、認証境界 (不正入力は副作用なく拒否)

初期ターゲット名の候補: fuzz_model_input、fuzz_temporal_commands、fuzz_operation_commands、fuzz_query_ast。fuzz ハーネスは **panic しない**だけでなく、不変条件違反や参照モデルとの不一致を失敗にする。通常の不正入力は Result による拒否を正解とし、OS や DB への無制限な副作用を禁止する。

- PR: 保存 corpus を使う短時間の smoke (実行時間予算・実行可否は CI 導入時に確定)
- Nightly: ターゲットを長時間実行し、結果を artifacts に保存
- 発見した crash: 再現入力 → 最小化 → 回帰テスト → 修正 → 再 fuzz
- corpus、辞書、再現 seed は機密業務情報ではなく合成入力のみを保存する
- sanitizer、OOM、timeout、入力拒否、論理不変条件違反を異なる失敗分類として報告する

## 3. フェーズ別ロードマップ

### P0 — 宣言的モデルと汎用 CRUD

**目的:** 利用者が社員・部署・所属関係を手動 SQL テーブル定義なしで作れる。

**実装**
- [ ] Cargo workspace、フォーマット/lint、ライセンス、CI、PostgreSQL 18 テストコンテナ
- [ ] entity / relation / role / attribute / cardinality / constraint を表す versioned モデル AST
- [ ] YAML/JSON パーサー、正規化、型チェック、互換性エラー
- [ ] PostgreSQL 初期マイグレーション、安定 ID、`scope_id`・`model_version` を含む汎用エンティティ/関係永続化
- [ ] モデル適用の繰返し・中断・不整合の扱い (同一モデルは冪等、非互換は明示拒否)
- [ ] P1 初期履歴への移行を妨げない ID・モデルバージョン・保存契約を ADR で固定
- [ ] model validate / apply、entity create/get、relation create/get の CLI (最低限)
- [ ] 組織サンプルと手順を fixture として追加

**Red → Green の代表テスト (What)**
- 未定義 Department role がモデル適用前に拒否される
- 同じ ID の二重登録が整合性を壊さない
- Membership の employee ロールに Department を割り当てると拒否される
- 別スコープの entity を relation に参照させようとすると拒否される
- 同一モデルの再 apply で状態が変化せず、不正な変更は全体がロールバックする
- entity と relation の作成・取得が実 PostgreSQL 上で一致する

**PBT / fuzz**: モデル AST round-trip、参照整合性、パーサー任意入力の安全な拒否。

**出口 (DoD):** 既存の業務ごとの SQL テーブルを作らず、ゴールデンシナリオの社員・部署・所属の登録と取得が CLI + PostgreSQL 18 で PASS。モデル不正ケースと再実行可能な PBT/fuzz smoke も PASS。

### P1 — Bitemporal Engine

**目的:** 同じ関係を現在・過去・未来と、当時の認識の両面から取得できる。

**実装**
- [ ] valid_time 半開区間の標準化、open-ended 値、境界の扱い
- [ ] スコープ別 recorded revision / commit order の定義。`known_at` の確定 revision への写像は ADR で実現可能性を検証し、公開は必須としない
- [ ] 属性・関係・ロールの version close + insert を同一トランザクションで処理
- [ ] 現在の取得、`valid_at`、`known_revision` の読み取り API (`known_at` は写像検証後)
- [ ] 遡及訂正、将来発令、削除の tombstone/終了を明示的に表す
- [ ] 属する期間の重複防止、空白期間、競合・再試行ポリシー
- [ ] PostgreSQL 障害・ロールバック・並列更新の統合テスト

**Red → Green の代表テスト (What)**
- 7 月 1 日ちょうどで旧所属は終わり新所属が始まる
- 8 月に 7 月の所属を訂正しても、7 月末当時の認識が残る
- 未来の異動が過去の照会に混入しない
- 同一社員の競合書き込みで確定履歴の重複・矛盾が生じない
- 全表を同じ `known_revision` で照会すると、部分的に新しい関係や操作効果が見えない
- r10/r11 の訂正 fixture で 7 月 10 日・16 日の表を完全一致で再現できる
- P0 からの初期履歴 migration を中断・再開しても ID・現在状態が保たれ、存在しない過去の知識を返さない
- ロック待機後の stale read と rollback 後の再試行でも revision 順序が破綻しない
- 参加者失効時の不正な孤児関係が拒否され、過去スナップショットの関係は保持される

**PBT / fuzz**: 任意の時点分割・補正・問い合わせ列を参照モデルと照合。境界、訂正、順序、重複を偏重生成する。

**出口 (DoD):** 所属の全時点問い合わせと遡及訂正が決定論的に再現できる。実 DB で競合・ロールバック・再起動を含む失敗試験 PASS。過去の認識を失わないことを fixture で実証。

### P2 — Operation / Event Engine

**目的:** 状態の変化と、業務上何を実行したかを対応づける。

**実装**
- [ ] 操作スキーマ (名前、入力、事前条件、効果、理由、actor)
- [ ] Transfer 等の命令を relation の変更へコンパイルする
- [ ] operation_id / request_id / idempotency_key と実行結果の台帳。冪等性キーはスコープ・操作型で名前空間を切り、**同一キー/異なる入力は conflict** にする
- [ ] operation と状態変更と outbox 追加の単一コミット
- [ ] 取消を「過去の消去」ではなく補償操作として表現する
- [ ] リトライ、重複到着、競合、途中失敗の規約
- [ ] outbox の再送・順不同・重複を前提にした購読者契約 (event_id、scope/revision、ack 前 crash の再送、購読側 deduplication)

**Red → Green の代表テスト (What)**
- 同一 Transfer request が 2 回届いても異動は 1 回で、同じ結果を返す
- 同一キーで異なる入力を送ると conflict になり、履歴・outbox は増えない
- 事前条件違反では状態・操作成功記録・イベントが書かれない
- 異動取消後にも異動と取消の経緯が確認できる
- outbox 配信に失敗しても DB 内の操作は失われず再送可能

**PBT / fuzz**: 命令列を状態機械と比較し、取消、二重送信、順序逆転、再試行を探索。

**出口 (DoD):** Transfer/Cancel の最小例を実装し、二重適用と部分コミットがないことを TDD + stateful PBT + fault injection で検証。外部配信について exactly-once を主張しない。

### P3 — Relationship Query / Query Compiler

**目的:** SQL のテーブル結合を直接意識せず、関係探索と時点検索を記述できる。

**実装**
- [ ] 型付き Query AST (entity、relation、role、predicate、valid_at、known_revision。`known_at` は有効化された場合のみ)
- [ ] AST からパラメータ化 SQL へのコンパイラ、入力リテラルの安全な束縛。SQL 識別子は検証済みモデルからだけ生成し、利用者入力を識別子へ連結しない
- [ ] 二者 / 多項関係の traversal、深さ・件数・処理量の上限と循環の停止規則
- [ ] 時点指定付き組織図、関係履歴、イベント provenance の参照 API
- [ ] EXPLAIN に基づく指数・インデックス設計、性能計測と回帰基準
- [ ] モデルバージョン変更後も旧記録が読める契約の明文化

**Red → Green の代表テスト (What)**
- ある時点の組織図と手計算した参照結果が一致する
- `valid_at` と `known_revision` を同時に変えたとき参照モデルに一致する
- 不正な role/path/属性名がコンパイル段階で拒否される
- 利用者入力が SQL 構造へ注入されない
- 複数テーブルの traversal が単一の `known_revision` と `scope_id` を保ち、循環・巨大探索は上限で停止する

**PBT / fuzz**: Query AST のランダム生成と参照インタプリタとの同値性、不正クエリの安全な拒否、計算量上限。

**出口 (DoD):** 組織図・所属履歴・訂正前履歴を Query API のみで取得でき、主要なクエリ計画と応答性能を測定して文書化。SQL injection と過大探索に対するテスト PASS。

### P4 — 再利用可能な製品化 (CLI / SDK / API / UI)

**目的:** 組織管理以外のアプリからも同じエンジンを使える。

**実装**
- [ ] CLI コマンド体系、機械可読 JSON 出力、診断とエラーコード
- [ ] Axum HTTP API、API スキーマ、認証・認可/権限モデル、変更監査
- [ ] TypeScript / Python SDK、互換性・シリアライズ contract tests
- [ ] モデル定義の差分・互換性検証と安全な migration plan
- [ ] React + React Flow の**読取専用**モデル図・時点組織図・監査ビュー (最小 UI は P4 受入対象、編集機能は非目標)
- [ ] Docker 配布例、通常のレンタルサーバー相当の PostgreSQL 接続手順
- [ ] サンプルを 2 業務領域へ拡張 (例: 組織/人事 + 資産管理)
- [ ] バージョニング、破壊的変更ポリシー、バックアップ/復元、公開ドキュメント
- [ ] CI/release workflow、security review、負荷・長期保持の検証

**Red → Green の代表テスト (What)**
- SDK → API → DB → SDK の結果が CLI と同じ
- 認可されていないテナントのデータ・履歴を参照できない (ID、時点、関係、操作 provenance のいずれも越境しない)
- モデル migration を中断・再開しても破損しない
- 2 つの業務サンプルが同じストレージスキーマで動く

**PBT / fuzz**: API request、SDK の入出力、マイグレーションの不変条件、認証境界、過大入力。

**出口 (DoD):** 利用手順だけで PostgreSQL 上に起動し、異なる 2 業務モデルを登録・運用可能。API/SDK/UI が文書化された契約で一致し、バックアップ復元と権限境界の E2E テスト PASS。CLI、サーバー、UI の配布形態を分離可能。

## 4. CI と検証証拠の設計

実装前の **提案コマンド**。CI を作成する段階で実際のワークスペース名・ターゲットに合わせて確定する。

| 実行タイミング | 実行対象 (予定) | 判定 |
| --- | --- | --- |
| PR / 毎 commit | cargo fmt --all -- --check | formatting |
| PR / 毎 commit | cargo clippy --workspace --all-targets -- -D warnings | lint |
| PR / 毎 commit | cargo test --workspace --all-targets | TDD / unit / integration / PBT |
| PR | PostgreSQL 18 を起動した migration + 時間・同時実行 E2E | SQL の実動作 |
| PR (nightly / Linux) | 保存済み corpus を使う `cargo +nightly-YYYY-MM-DD fuzz run` smoke (対象・時間は確定後) | crash / invariant |
| Nightly | noprop 大量ケース + すべての fuzz ターゲット | 長時間探索 |
| Nightly | 書き込み競合 / 障害注入 / migration compatibility | 一貫性 |
| Release | 2 業務サンプルの clean install + backup/restore | 配布受入 |

- すべての PBT/fuzz 失敗は **再現用情報を artifact に保存**する。テストが完了していない場合は timeout / cancelled / not-run を PASS としない。fuzz では toolchain、target、corpus hash、実行時間、終了理由を記録する。
- P0 で fuzz ジョブを構築するまでは smoke は未実行。導入後の起動不能もゲート未通過として、実装失敗と環境不備を区別する。
- **実行対象の段階化:** P0 は unit・モデル PBT・パーサー fuzz・PostgreSQL fixture、P1 は記録順序/訂正の競合 E2E、P2 は fault injection、P3 は query differential、P4 は認可・配布・復元を追加する。先行ゲートを後続で省略しない。
- 仕様変更に伴う fixture の更新は期待値の根拠をレビューする。スナップショットを盲目的に更新しない。
- テストの重みは unit → property → DB integration → E2E → fault injection の順に増やすが、各段階のゲートは省略しない。
- PBT と fuzz は役割が違う。PBT は業務不変条件と状態モデルの差分、fuzz は構文や想定外の入力列など探索面を広げる。互いに代替しない。
- コード未作成の現段階では、上のコマンドは **すべて未実行**。

### PR に必須の報告テンプレート

- **変更理由 (Why)**: 何の失敗・業務要件を解消するか
- **契約 (What)**: 前後で変わる観測可能な振る舞い
- **RED / GREEN**: 失敗を確認したテスト、変更後に通過したテスト
- **PBT**: seed、件数、結果 (PASS/FAIL/未実行)
- **fuzz**: ターゲット、時間、corpus、結果 (PASS/FAIL/未実行)
- **DB E2E**: PostgreSQL バージョン、並行数、障害条件、結果
- **未解決**: ブロッカーと対処、次の優先順位

## 5. 重要な意思決定ゲート

各ゲートで ADR (Architecture Decision Record) を残す。

1. **P0 開始時:** 正規化 AST とストレージ分離。モデルのバージョン互換規約。スコープ ID の配置と参照境界。P1 初期履歴への移行契約。
2. **P1 開始時:** スコープ別 recorded revision の確定順序・ロック・分離レベル・再試行。`known_at` 写像の公開可否。業務日付のタイムゾーン、時間境界と訂正手法。PostgreSQL temporal constraint の適用範囲。
3. **P2 開始時:** Operation を監査可能な意図として扱うか、event sourcing へ拡張するか。初期方針は前者。
4. **P3 開始時:** モデルクエリの表現力と安全性/性能のバランス。無制限 traversal を認めない。
5. **P4 開始時:** サーバー・SDK・UI の公開安定性と認証/テナント分離の責任範囲。

### フェーズ間の依存

- P1 は P0 のモデル ID、型検証、DB 永続化に依存。
- P2 は P1 の原子的な履歴更新と recorded revision に依存。
- P3 は P0–P2 の安定した読み取り/操作契約に依存。
- P4 は P0–P3 の検証済み API に依存。
- **TDD、PBT、fuzz は P4 の仕上げ作業ではない。P0 から各フェーズの出口条件に含める。**

## 6. 将来の予定ディレクトリ構成

~~~text
Cargo.toml
crates/
  tokiori-model/         # 型付きモデル・AST・検証
  tokiori-temporal/      # 二時点演算と参照モデル
  tokiori-engine/        # 操作と整合性
  tokiori-pg/            # SQLx / PostgreSQL 永続化
  tokiori-query/         # Query AST / SQL compiler
  tokiori-cli/           # CLI
  tokiori-server/        # P4: HTTP API
fuzz/                    # cargo-fuzz ターゲット・合成 corpus
tests/                   # PostgreSQL integration と受入 fixture
examples/organization/   # golden scenario
examples/assets/         # P4 の第 2 業務領域
docs/
  ROADMAP.md
  adr/
  testing/
ui/                      # P4: 管理画面
sdks/                    # P4: TypeScript / Python
~~~

ディレクトリ名は未確定。不要な crate や UI を P0 で先取りして作らない。

## 7. 最初に着手する縦割りスライス

**P0 の推奨実装順:** (a) モデル parse/validate と失敗 fixture → (b) スコープ付き entity/relation の保存とロール整合性 → (c) CLI の round-trip → (d) clean DB / 再 apply / 失敗時 rollback。各スライスで Red → Green → Refactor を閉じ、PBT/fuzz の対象を増やす。これは実装順の提案であり、まだ実行結果ではない。

1. **RED**: 「Membership を定義・保存・取得する」受入テストを書く。SQLテーブルを利用者に定義させない。
2. **GREEN**: model AST → PostgreSQL 保存 → CLI 取得を最短距離で動かす。
3. **PBT**: noprop で Employee/Department/Membership の生成・参照・不正 role を確認し、seed 再生を通す。
4. **fuzz**: 任意のモデル入力が panic せず、有効/無効を区別して安全に処理されることを検証する。
5. **DB E2E**: clean PostgreSQL 18 で移行・CRUD・ロールバックを通し、実際の実行証拠を残す。

**P0 の第一スライスが通るまで、独自 DSL パーサー、巨大な UI、最適化、分散化には着手しない。**

### P0 実装タスクの切り方（順序付き・各タスクを小 PR にする）

1. **契約の固定:** `ORG-001` / `ORG-002` の正常・不正 fixture を書き、失敗を Red で確認する。モデル構文は最小限の JSON から開始し、YAML 対応を後から追加してよい。
2. **モデル検証:** AST / role / entity 型・scope・model_version を実装し、純粋な検証と noprop の round-trip を Green にする。DB はまだ要らない。
3. **永続化:** PostgreSQL 18 の clean schema・モデル apply・entity/relation 保存・トランザクション rollback。手作業の業務テーブル DDL は利用者に要求しない。
4. **操作面:** 最小 CLI を接続し、作成した DB を別プロセスから取得する。P4 の安定 CLI/HTTP/SDK 契約を先取りしない。
5. **再現可能なゲート:** `ORG-001` / `ORG-002` を新しい DB で通し、固定 seed の noprop と parser fuzz smoke を別々に実行する。実際に成功した結果のみ PASS と記録する。

**P0 完了判定:** 上の 5 件が動くことに加え、`ORG-003` の cutover 方針を ADR として確定する。P1 の本実装や過去時点照会が動くことは P0 完了条件ではない。

## 8. 参考資料

- PostgreSQL 18 temporal constraints: https://www.postgresql.org/docs/18/sql-createtable.html
- PostgreSQL temporal tables (system time の未対応範囲): https://www.postgresql.org/docs/18/ddl-temporal-tables.html
- sile/noprop: https://github.com/sile/noprop
- noprop API / seed / 再現 / 制約: https://docs.rs/noprop/latest/noprop/
- Rust fuzzing: https://rust-fuzz.github.io/book/cargo-fuzz.html
- XTDB: https://xtdb.com/
- TypeDB: https://typedb.com/
