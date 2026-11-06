# Tokiori

**Temporal & Relational Data Engine — Relationships through time.**

Tokiori は PostgreSQL 上で、業務上の **Entity / Relation / Role / Operation** を宣言し、履歴管理と時点検索を一貫したモデルで扱うことを目指す OSS プロジェクトです。

- **Temporal:** 有効時刻と記録時刻を分離し、過去・未来・遡及訂正・当時の認識を問い合わせる。
- **Relational:** 所属などの関係を第一級の概念として扱い、多項関係・属性・制約を定義する。
- **Operational:** 異動などの業務操作と、その結果として生じる状態変更・イベントを監査可能に記録する。
- **PostgreSQL-first:** 利用者に業務ごとの履歴テーブルを手作業で設計させない。

## 想定する利用の流れ（未実装）

1. 社員・部署・所属のモデルを宣言して、実体と関係を登録する（P0）。
2. 有効日と記録 revision を指定して当時の状態を調べる（P1）。P0 で記録していない変更履歴は復元せず、P1 への移行前の時点は「履歴不明」として区別する。
3. 異動を単一の業務操作として記録・再試行する（P2）。組織図などの横断照会を行う（P3）。
4. SDK / API / 読取専用 UI から複数業務モデルを利用する（P4）。

通常の参照は現在値を取得し、過去の確認が必要な場合だけ時点を明示する設計です。履歴テーブルを利用者が直接操作する必要はありません。

## Roadmap

**[P0–P4 実装ロードマップ (docs/ROADMAP.md)](docs/ROADMAP.md)**

P0 宣言的モデル / 汎用 CRUD・最小 CLI → P1 Bitemporal Engine → P2 Operation / Event Engine → P3 Query Compiler → P4 安定 CLI / SDK / API / UI。

実装は **TDD** を基本とし、P0 から [sile/noprop](https://github.com/sile/noprop) による **PBT** と **cargo-fuzz** による **fuzzing** を継続します。さらに [Proof-Driven Development 方針](docs/PDD.md) に沿って、人が承認する仕様、Verus 形式検証の候補、信頼境界、PR の検証証拠を段階的に整備します（PDD/Verus は未実装）。各段階の受入条件・失敗再現・CI 方針、および実装で追跡する 7 件の受入シナリオ（`ORG-001`〜`ORG-007`）はロードマップに記載しています。

> **Status:** 設計・計画段階です。実装、CLI、API、SDK、テスト、CI はまだ提供していません。
