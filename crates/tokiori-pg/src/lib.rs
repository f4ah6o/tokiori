//! PostgreSQL persistence for Tokiori.
//!
//! One store operation is one transaction. Validation happens before any row is
//! written, and a rejected operation rolls back, so a failed request never
//! leaves a partially written relation behind.

mod cancel;
mod compiler;
mod cutover;
mod error;
mod facts;
mod operations;
mod outbox;
mod query_exec;
mod revision;
mod roles;
mod store;
mod transfer;

pub use cancel::CancelTransfer;
pub use compiler::{CompiledQuery, compile_query};
pub use cutover::{Cutover, CutoverOutcome};
pub use error::{ApplyOutcome, PgError};
pub use facts::{AttributeAnswer, ReadAttribute, SetAttribute};
pub use operations::{OperationKey, OperationRecord, input_digest};
pub use outbox::OutboxEvent;
pub use query_exec::{QueryResult, QueryRow};
pub use revision::{latest_revision, next_revision};
pub use roles::{ReadRole, RoleAnswer, SetRole};
pub use store::{
    EntityRow, NewEntity, NewRelation, PgStore, RelationRow, connect, migrate, migrate_with,
};
pub use transfer::{OperationOutcome, Transfer};

/// Migrations embedded at build time so the binary needs no migration directory
/// at run time.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
