# Tokiori: Proof-Driven Development（PDD）導入方針

- 状態: **提案・未実装**。Verus の証明、PDD Kit の実行、CI、実 DB テストを実施済みと主張しない。
- 参照: [GitHub Next PDD Kit](https://githubnext.com/projects/pdd-kit/)（GitHub Next / Microsoft Research、Proof-Driven Development の研究プロトタイプ）、[Verus](https://github.com/verus-lang/verus)。
- 対象: [ROADMAP.md](ROADMAP.md) の `ORG-001`〜`ORG-007`。既存の TDD / sile/noprop PBT / cargo-fuzz / PostgreSQL E2E は維持する。
- 目的: 人が承認した「業務上の意図」と、エージェントが生成した実装の間に、**検証可能な契約と証拠**を置く。

> この文書で導入するのは **PDD Kit に学んだ開発規律**。研究プロトタイプ自体の導入・実行可能性や、既存システム全体の形式的正しさを保証するものではない。同名の別プロジェクト「Product-Driven Development」の pdd-kit とは別物。

## 1. 仕様 → 実装 → 証拠のフロー

1. **Intent（人が判断）**: `ORG-` ID、利用者の言葉、正常/拒否条件、時刻/スコープ/版の意味、非目標を短い意思決定カードに記録する。曖昧さは実装前に ADR に切り出す。機械生成の仕様は承認前に正しいものと扱わない。
2. **Spec landscape**: 各要求を `formal`（証明予定）、`empirical`（テストで確認）、`unverified`（未検証）に分け、依存する外部前提を Trusted Computing Base（TCB）に列挙する。証明が扱わない範囲は明記する。
3. **Formal contract**: 対象を可能な限り純粋な Rust の関数とし、入力前提・不変条件・事後条件を Verus の仕様へ写す。自然言語で各条項の意味と `ORG-` ID を併記し、仕様側のレビューを実装と独立に行う。
4. **Red → Green → Refactor / prove**: 既存の失敗テストから実装する。証明対象は `Verus` のチェックを通し、証明できない境界には TDD / PBT / fuzz / 実 PostgreSQL E2E を適用する。証明に通すために要件や `ensures` を弱めた場合は、変更として再承認を要求する。
5. **Spec mutation**: 各重要な事後条件を破る具体的な mutant（例: 境界 `<` を `<=`、スコープ検査を削除）を作り、その条項が実際に拒否できるか確認する。mutant が通った場合は「弱い証明」ではなく **spec gap** として扱う。
6. **PR receipt**: 要求ごとの判定、使用した spec/実装 commit、verifier のバージョンとログ、TCB と対応テスト、実行していない検証、mutation 結果を記載する。機械チェックは intent の正しさを代行しない。

`formal` は実際に検証された仕様条項と実装経路にのみ付す。`Verus` に通った別実装のコピーを本番コードが利用しない場合、**本番の保証には数えない**。`empirical` は実行した入力条件での根拠であり、数学的証明と混同しない。

## 2. P0–P4 の証明候補と境界

| 段階 / ID | 候補となる純粋ロジックの契約（formal の検討対象） | 必須の empirical / trusted 境界 |
| --- | --- | --- |
| **P0 / ORG-001, 002** | モデル正規化の決定性、role 型・参加者 cardinality、異スコープ参照拒否 | serde 入力、SQLx 永続化、ユニーク制約、migration、rollback |
| **P1 / ORG-003, 004** | 半開区間 `[from,to)` の所属判定、期間分割、履歴への新 revision 追加後も旧 `known_revision` の照会値が変わらないこと、cutover 前の `HistoryUnavailable` | PostgreSQL の commit 順序と snapshot、ロック、分離レベル、同時書込、時計、永続化、再起動 |
| **P2 / ORG-005** | 同一スコープ/操作型/key/正規化済み入力の再適用は同じ論理結果、異入力は conflict、補償操作は既存履歴を消さない | DB トランザクションの原子性、鍵の競合、outbox/consumer 配信、障害注入 |
| **P3 / ORG-006** | 型付き Query AST の role/path/scope 検証、論理演算・探索結果の参照意味論 | SQL compiler と DB の一致、プレースホルダー束縛、探索上限、SQL injection 実試験 |
| **P4 / ORG-007** | API の純粋な権限判定（actor・scope・操作に対する allow/deny） | 認証情報、SDK serialization、HTTP middleware、DB の row filtering、backup/restore |

全フェーズに一律で Verus を適用しない。特に DB や外部ライブラリの事実を `external_body` 等の **仮定として宣言しただけでは証明されていない**。対応する仮定の意味・反証可能なテスト・破れた場合の影響を残す。

## 3. 最優先の具体例: ORG-004（P1 の訂正前知識）

### 承認対象の intent（正準 fixture）

- `r10`: 7/1 付で営業部から企画部へ異動したと記録。
- `r11`: 同じ異動の発効日を 7/15 に遡及訂正したと記録。
- `query(7/10, known_revision=r10) = 企画部`、`query(7/10, known_revision=r11) = 営業部`。
- `query(7/16, known_revision=r10) = 企画部`、`query(7/16, known_revision=r11) = 企画部`。
- `r11` を追加しても `r10` を使う問い合わせの意味は変わらない。revision は必ず同一 `scope_id` 内で解決する。
- cutover 前の未記録期間は `NotFound` でなく `HistoryUnavailable`。モデルの業務タイムゾーン・半開区間境界を前提として明示する。

### 検証の分担

- **Proof candidate**: 有効期間と、抽象的な「確定済み revision の列」から照会結果を求める純粋関数を proof-aware に実装し、旧 revision の非干渉性、境界条件、候補選択の一意性を証明する。一般的な列・期間に対する条項と、上記 fixture の具体例は分ける。
- **TCB**: DB が同一 revision の属性・役割・操作効果を整合した snapshot として取得できること、コミット済み revision の順序が正しいこと。これらは Verus の抽象入力への仮定であり、実 DB の並列実行・rollback・再起動・ロック後再評価で反証を試みる。
- **Mutation**: 境界日の `<` / `<=` の入替、古い revision を最新で上書き、scope の条件除去を個別に注入し、どの formal clause または empirical test が検出するか報告する。
- **Production wiring**: テスト対象の純粋関数を本番の P1 読取経路から呼ぶ。証明用の鏡像実装だけが PASS しても完了扱いしない。

## 4. 段階導入と CI

1. **P0:** 既存の 5 タスク・TDD/PBT/fuzz/DB E2E の進行を妨げない。純粋なモデル検証 1 件を Verus で proof-of-concept として試し、必要な Rust 機能・依存ライブラリ・時間/CI コストを ADR に記録する。PoC 未達を P0 本体の CRUD 完了と混同しない。
2. **P1 開始時:** P0 PoC の結果を基に `ORG-004` の formal 境界・TCB・proof obligation を ADR で決定する。Verus の制約で不成立なら、要件を無断で弱めず `unverified formal` として残し、PBT・fuzz・実 DB E2E を継続する。P1 の完了を「全体の形式証明済み」と称さない。
3. **P2 以降:** `ORG-005` の冪等性決定、`ORG-006` の AST 妥当性、`ORG-007` の認可判定から、リスクに応じて証明対象を拡張する。I/O と外部依存は引き続き trusted / empirical に分類する。
4. **CI:** proof の導入可能性を確認するまでは別の非必須 PoC job（未作成）。安定した verified core を本番に接続した段階で、Verus の **固定 revision/ツールチェーン**と対象 spec を commit し、既存ゲートと並ぶ required check に移す。失敗・timeout・未実行を PASS にしない。証明ジョブの失敗をテスト成功で上書きしない。

想定配置は `docs/verification/intent/`、`docs/verification/receipts/`、`crates/tokiori-verified/`（または core 内の proof-aware module）。これらは**予定**であり、P0 開始時に空のクレートを大量追加しない。

## 5. PR receipt の最小様式

| フィールド | 記録内容 |
| --- | --- |
| Requirement / intent | `ORG-...`、レビュー済み意図・ADR・対応する自然言語の仕様 |
| Formal | 証明された関数・条項、対応する本番経路、verifier revision、実行 commit、**PASS / FAIL / 未実行 / 対象外** |
| Trusted boundary | 仮定・外部システム・反証用テスト・テストの実行 commit |
| Empirical | TDD、noprop seed と件数、fuzz target/corpus/終了理由、PostgreSQL E2E |
| Spec mutation | 重要条項ごとに変異内容、拒否した証拠、検出不能な gap |
| Limitations | 要件外、未対応 Rust 機能、保証しない性質（例: 外部配信 exactly-once） |

**検収条件:** 要求は `formally-proven` / `empirically-checked` / `unverified` のいずれかに正確に分類される。spec 承認と検証結果の追跡ができ、CI の green を意図の正しさと誤認しない。

## 6. 参考資料

- GitHub Next: https://githubnext.com/projects/pdd-kit/
- Verus: https://github.com/verus-lang/verus
- Tokiori ロードマップ: [ROADMAP.md](ROADMAP.md)
