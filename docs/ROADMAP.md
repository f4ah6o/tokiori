# Tokiori: P0–P4 実装ロードマップ

- 状態: **計画 (未実装)**。本書のコマンド、API、パス、テスト名は実装予定の契約であり、現在動作することを意味しない。
- 対象: PostgreSQL を永続化先とする汎用 Temporal + Relationship データモデリングエンジン
- 方針: TDD を実装の基本サイクルにし、sile/noprop による PBT と cargo-fuzz による fuzzing を P0 から継続する。
- 進行: **P0 → P1 → P2 → P3 → P4**。期間ではなく、再現可能な受入基準で次段階へ進む。

## 0. 北極星・スコープ

### 解決したい問題

業務概念が増えるたびに、エンティティ、所属などの関係、変更履歴、異動などの操作、監査ログのテーブルを手動で増やし続けたくない。開発者は **何が存在し、どのような関係があり、どう変化し得るか** を宣言する。Tokiori が PostgreSQL 上の物理表、時点管理、クエリ、変更記録を吸収する。

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

### 非目標 (P0–P4)

- XTDB のクエリ互換、TypeDB の推論エンジンの再現
- 完全なイベントソーシングへの移行、状態の再構築をイベントだけに依存させる設計
- 業務上の「異動」を、関係変更から**自動的に推論**すること (操作は明示的に定義する)
- グラフ分散処理、全文検索、任意規模の OLAP の最適化
- 外部購読者まで含む exactly-once イベント配信

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
- entities: 型付き実体の安定した ID
- relations / relation_roles: 関係 ID と役割の参加者 (多項関係)
- fact_versions: エンティティ属性・関係属性・役割の有効時刻と記録履歴
- operations / operation_effects: 操作の意図、冪等性キー、状態への効果
- outbox_events: コミットと同一トランザクションで発行する配信待ちイベント

**物理テーブル数を 3 に固定しない**。減らすべきは開発者が業務ごとに手設計するテーブル数であり、整合性のための内部テーブルは許容する。JSONB は柔軟な属性表現に使い、ID、時間範囲、ロール、テナント境界など重要な制約を全て JSONB のみに押し込まない。

### 時間・整合性の基本契約

- 有効期間は半開区間 **[from, to)**、時刻は UTC で保存し、表示タイムゾーンと区別する。無期限の終端を扱う。
- 有効時刻は業務上の効力発生日、記録時刻はシステムがそれを記録・認識した時点。両者を混ぜない。
- 履歴上の訂正は過去バージョンを不可視化するのではなく、**新しい知識として新バージョンを記録**する。記録済み内容の通常 UPDATE/DELETE は許さず、システム期間を閉じるメタデータ更新だけを厳格に許容する設計を比較する。
- 「ある時点に知っていた内容」は、コミット順を一意に示す **recorded revision** により決定論的に再現する。壁時計時刻による検索は revision へのマッピングを定義する。単なる連番取得順やトランザクション開始時刻をコミット順とみなさない。
- P1 で最初は書き込みの記録順序を直列化する等の単純で正しい方式を採り、競合制御を測定後に細粒度化する。ロールバック、クラッシュ、同時更新で矛盾が生じないことを実 DB で検証する。
- 「所属は同期間に 1 つまで」等の制約はモデルごとに明示する。兼務等はモデルを変更して許容できる。
- PostgreSQL 18 の WITHOUT OVERLAPS / PERIOD は**有効時間**の整合性に利用可能だが、二時点履歴の記録時刻管理まで自動提供するわけではない。全バージョンへ無条件に WITHOUT OVERLAPS を適用しない。

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
- 乱数 seed を明示的に受け取り、失敗時に **seed、ケース数、入力 trace、モデルバージョン、PostgreSQL バージョン、Tokiori のコミット** を記録して再実行可能にする。
- 通常 PR は各対象プロパティ **1,000 ケース**を基準に開始し、夜間は **10,000 ケース**へ拡大。実行時間を測定して継続的に調整する。重要な境界はランダム性に頼らず固定例も用意する。
- noprop は組み込みの **自動 shrinking / 永続失敗 corpus** を提供しない。失敗 trace は Tokiori 側で保存し、シーケンス削除・引数単純化等の独立した縮小手順を開発する。最小化した入力を通常の regression fixture に追加する。
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
- [ ] PostgreSQL 初期マイグレーション、安定 ID、最低限の汎用エンティティ/関係永続化
- [ ] model validate / apply、entity create/get、relation create/get の CLI (最低限)
- [ ] 組織サンプルと手順を fixture として追加

**Red → Green の代表テスト (What)**
- 未定義 Department role がモデル適用前に拒否される
- 同じ ID の二重登録が整合性を壊さない
- Membership の employee ロールに Department を割り当てると拒否される
- entity と relation の作成・取得が実 PostgreSQL 上で一致する

**PBT / fuzz**: モデル AST round-trip、参照整合性、パーサー任意入力の安全な拒否。

**出口 (DoD):** 既存の業務ごとの SQL テーブルを作らず、ゴールデンシナリオの社員・部署・所属の登録と取得が CLI + PostgreSQL 18 で PASS。モデル不正ケースと再実行可能な PBT/fuzz smoke も PASS。

### P1 — Bitemporal Engine

**目的:** 同じ関係を現在・過去・未来と、当時の認識の両面から取得できる。

**実装**
- [ ] valid_time 半開区間の標準化、open-ended 値、境界の扱い
- [ ] recorded revision / commit order の定義、recorded_at との変換と監査
- [ ] version close + insert を同一トランザクションで処理
- [ ] 現在、valid_as_of、known_at / known_revision の読み取り API
- [ ] 遡及訂正、将来発令、削除の tombstone/終了を明示的に表す
- [ ] 属する期間の重複防止、空白期間、競合・再試行ポリシー
- [ ] PostgreSQL 障害・ロールバック・並列更新の統合テスト

**Red → Green の代表テスト (What)**
- 7 月 1 日ちょうどで旧所属は終わり新所属が始まる
- 8 月に 7 月の所属を訂正しても、7 月末当時の認識が残る
- 未来の異動が過去の照会に混入しない
- 同一社員の競合書き込みで確定履歴の重複・矛盾が生じない

**PBT / fuzz**: 任意の時点分割・補正・問い合わせ列を参照モデルと照合。境界、訂正、順序、重複を偏重生成する。

**出口 (DoD):** 所属の全時点問い合わせと遡及訂正が決定論的に再現できる。実 DB で競合・ロールバック・再起動を含む失敗試験 PASS。過去の認識を失わないことを fixture で実証。

### P2 — Operation / Event Engine

**目的:** 状態の変化と、業務上何を実行したかを対応づける。

**実装**
- [ ] 操作スキーマ (名前、入力、事前条件、効果、理由、actor)
- [ ] Transfer 等の命令を relation の変更へコンパイルする
- [ ] operation_id / request_id / idempotency_key と実行結果の台帳
- [ ] operation と状態変更と outbox 追加の単一コミット
- [ ] 取消を「過去の消去」ではなく補償操作として表現する
- [ ] リトライ、重複到着、競合、途中失敗の規約
- [ ] outbox の再送・順不同・重複を前提にした購読者契約

**Red → Green の代表テスト (What)**
- 同一 Transfer request が 2 回届いても異動は 1 回
- 事前条件違反では状態・操作成功記録・イベントが書かれない
- 異動取消後にも異動と取消の経緯が確認できる
- outbox 配信に失敗しても DB 内の操作は失われず再送可能

**PBT / fuzz**: 命令列を状態機械と比較し、取消、二重送信、順序逆転、再試行を探索。

**出口 (DoD):** Transfer/Cancel の最小例を実装し、二重適用と部分コミットがないことを TDD + stateful PBT + fault injection で検証。外部配信について exactly-once を主張しない。

### P3 — Relationship Query / Query Compiler

**目的:** SQL のテーブル結合を直接意識せず、関係探索と時点検索を記述できる。

**実装**
- [ ] 型付き Query AST (entity、relation、role、predicate、valid_at、known_at)
- [ ] AST からパラメータ化 SQL へのコンパイラ、入力リテラルの安全な束縛
- [ ] 二者 / 多項関係の traversal、深さ・件数の上限
- [ ] 時点指定付き組織図、関係履歴、イベント provenance の参照 API
- [ ] EXPLAIN に基づく指数・インデックス設計、性能計測と回帰基準
- [ ] モデルバージョン変更後も旧記録が読める契約の明文化

**Red → Green の代表テスト (What)**
- ある時点の組織図と手計算した参照結果が一致する
- valid_at と known_at を同時に変えたとき参照モデルに一致する
- 不正な role/path/属性名がコンパイル段階で拒否される
- 利用者入力が SQL 構造へ注入されない

**PBT / fuzz**: Query AST のランダム生成と参照インタプリタとの同値性、不正クエリの安全な拒否、計算量上限。

**出口 (DoD):** 組織図・所属履歴・訂正前履歴を Query API のみで取得でき、主要なクエリ計画と応答性能を測定して文書化。SQL injection と過大探索に対するテスト PASS。

### P4 — 再利用可能な製品化 (CLI / SDK / API / UI)

**目的:** 組織管理以外のアプリからも同じエンジンを使える。

**実装**
- [ ] CLI コマンド体系、機械可読 JSON 出力、診断とエラーコード
- [ ] Axum HTTP API、API スキーマ、認証・認可/権限モデル、変更監査
- [ ] TypeScript / Python SDK、互換性・シリアライズ contract tests
- [ ] モデル定義の差分・互換性検証と安全な migration plan
- [ ] React + React Flow のモデル図・時点組織図・監査ビュー (任意導入)
- [ ] Docker 配布例、通常のレンタルサーバー相当の PostgreSQL 接続手順
- [ ] サンプルを 2 業務領域へ拡張 (例: 組織/人事 + 資産管理)
- [ ] バージョニング、破壊的変更ポリシー、バックアップ/復元、公開ドキュメント
- [ ] CI/release workflow、security review、負荷・長期保持の検証

**Red → Green の代表テスト (What)**
- SDK → API → DB → SDK の結果が CLI と同じ
- 認可されていないテナントのデータ・履歴を参照できない
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
| PR | 保存済み corpus を使う cargo-fuzz smoke (対象・時間は確定後) | crash / invariant |
| Nightly | noprop 大量ケース + すべての fuzz ターゲット | 長時間探索 |
| Nightly | 書き込み競合 / 障害注入 / migration compatibility | 一貫性 |
| Release | 2 業務サンプルの clean install + backup/restore | 配布受入 |

- すべての PBT/fuzz 失敗は **再現用情報を artifact に保存**する。テストが完了していない場合は timeout / cancelled / not-run を PASS としない。
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

1. **P0 開始時:** 正規化 AST とストレージ分離。モデルのバージョン互換規約。
2. **P1 開始時:** recorded revision の定義と確定順序。時間境界と訂正手法。PostgreSQL temporal constraint の適用範囲。
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

1. **RED**: 「Membership を定義・保存・取得する」受入テストを書く。SQLテーブルを利用者に定義させない。
2. **GREEN**: model AST → PostgreSQL 保存 → CLI 取得を最短距離で動かす。
3. **PBT**: noprop で Employee/Department/Membership の生成・参照・不正 role を確認し、seed 再生を通す。
4. **fuzz**: 任意のモデル入力が panic せず、有効/無効を区別して安全に処理されることを検証する。
5. **DB E2E**: clean PostgreSQL 18 で移行・CRUD・ロールバックを通し、実際の実行証拠を残す。

**P0 の第一スライスが通るまで、独自 DSL パーサー、巨大な UI、最適化、分散化には着手しない。**

## 8. 参考資料

- PostgreSQL 18 temporal constraints: https://www.postgresql.org/docs/18/sql-createtable.html
- PostgreSQL temporal tables (system time の未対応範囲): https://www.postgresql.org/docs/19/ddl-temporal-tables.html
- sile/noprop: https://github.com/sile/noprop
- noprop API / seed / 再現 / 制約: https://docs.rs/noprop/latest/noprop/
- Rust fuzzing: https://rust-fuzz.github.io/book/cargo-fuzz.html
- XTDB: https://xtdb.com/
- TypeDB: https://typedb.com/
