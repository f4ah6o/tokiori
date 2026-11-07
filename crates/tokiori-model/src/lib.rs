//! Declarative model AST for Tokiori.
//!
//! A model declares which entities exist, which attributes they carry, and which
//! relations connect them through roles. The crate is pure: it performs no I/O and
//! knows nothing about PostgreSQL, so the same validation runs in the CLI, in the
//! persistence layer, and in property tests.

mod canonical;
mod error;
mod model;
mod resolution;
mod time;
mod valid_time;

pub use canonical::{canonical_json, model_digest};
pub use error::{InvalidModel, ModelError, ModelErrorCode, ParseError};
pub use model::{
    AttributeType, Cardinality, Entity, EntityDef, Model, ModelDocument, Relation, RelationDef,
    Role, RoleDef, normalize, parse_model_str,
};
pub use resolution::{FactVersion, Resolution, resolve, resolve_cloned};
pub use time::{TimeError, business_day_start};
pub use valid_time::{ValidTime, ValidTimeError};
