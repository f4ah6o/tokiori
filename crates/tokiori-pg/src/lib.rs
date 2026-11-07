//! PostgreSQL persistence for Tokiori.
//!
//! One store operation is one transaction. Validation happens before any row is
//! written, and a rejected operation rolls back, so a failed request never
//! leaves a partially written relation behind.

mod error;
mod facts;
mod revision;
mod store;

pub use error::{ApplyOutcome, PgError};
pub use facts::SetAttribute;
pub use revision::{latest_revision, next_revision};
pub use store::{
    EntityRow, NewEntity, NewRelation, PgStore, RelationRow, connect, migrate, migrate_with,
};

/// Migrations embedded at build time so the binary needs no migration directory
/// at run time.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
