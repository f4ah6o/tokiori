# Tokiori

**Temporal & Relational Data Engine — Relationships through time.**

Tokiori は PostgreSQL 上で、業務上の **Entity / Relation / Role / Operation** を宣言し、履歴管理と時点検索を一貫したモデルで扱うことを目指す OSS プロジェクトです。

- **Temporal:** 有効時刻と記録時刻を分離し、過去・未来・遡及訂正・当時の認識を問い合わせる。
- **Relational:** 所属などの関係を第一級の概念として扱い、多項関係・属性・制約を定義する。
- **Operational:** 異動などの業務操作と、その結果として生じる状態変更・イベントを監査可能に記録する。
- **PostgreSQL-first:** 利用者に業務ごとの履歴テーブルを手作業で設計させない。

## Roadmap

**[P0–P4 実装ロードマップ (docs/ROADMAP.md)](docs/ROADMAP.md)**

P0 宣言的モデル / 汎用 CRUD → P1 Bitemporal Engine → P2 Operation / Event Engine → P3 Query Compiler → P4 CLI / SDK / API / UI。

実装は **TDD** を基本とし、P0 から [sile/noprop](https://github.com/sile/noprop) による **PBT** と **cargo-fuzz** による **fuzzing** を継続します。各段階の受入条件・失敗再現・CI 方針はロードマップに記載しています。

> **Status:** 設計・計画段階です。実装、CLI、API、SDK、テスト、CI はまだ提供していません。
