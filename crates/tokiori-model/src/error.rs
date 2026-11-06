use std::fmt;

use thiserror::Error;

/// A single semantic problem found while normalizing a model document.
///
/// Errors are collected rather than short-circuited so one run reports every
/// problem the author must fix.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelError {
    #[error("model name {name:?} is invalid; expected [a-z][a-z0-9_-]*")]
    InvalidModelName { name: String },
    #[error("model version must be at least 1, got {version}")]
    InvalidVersion { version: u32 },
    #[error("time zone {time_zone:?} is not an IANA time zone")]
    InvalidTimeZone { time_zone: String },
    #[error("entity name {name:?} is invalid; expected [a-z][a-z0-9_]*")]
    InvalidEntityName { name: String },
    #[error("relation name {name:?} is invalid; expected [a-z][a-z0-9_]*")]
    InvalidRelationName { name: String },
    #[error("attribute name {name:?} is invalid; expected [a-z][a-z0-9_]*")]
    InvalidAttributeName { name: String },
    #[error("role name {name:?} is invalid; expected [a-z][a-z0-9_]*")]
    InvalidRoleName { name: String },
    #[error("attribute {attribute:?} is reserved and cannot be declared")]
    ReservedAttribute { attribute: String },
    #[error("entity {entity:?} is declared more than once")]
    DuplicateEntity { entity: String },
    #[error("relation {relation:?} is declared more than once")]
    DuplicateRelation { relation: String },
    #[error("relation {relation:?} declares role {role:?} more than once")]
    DuplicateRole { relation: String, role: String },
    #[error("relation {relation:?} must declare at least one role")]
    EmptyRoles { relation: String },
    #[error("relation {relation:?} role {role:?} references undefined entity {entity:?}")]
    UnknownEntity {
        relation: String,
        role: String,
        entity: String,
    },
}

impl ModelError {
    /// Stable machine-readable code used by the CLI and by tests.
    #[must_use]
    pub fn code(&self) -> ModelErrorCode {
        match self {
            Self::InvalidModelName { .. } => ModelErrorCode::InvalidModelName,
            Self::InvalidVersion { .. } => ModelErrorCode::InvalidVersion,
            Self::InvalidTimeZone { .. } => ModelErrorCode::InvalidTimeZone,
            Self::InvalidEntityName { .. } => ModelErrorCode::InvalidEntityName,
            Self::InvalidRelationName { .. } => ModelErrorCode::InvalidRelationName,
            Self::InvalidAttributeName { .. } => ModelErrorCode::InvalidAttributeName,
            Self::InvalidRoleName { .. } => ModelErrorCode::InvalidRoleName,
            Self::ReservedAttribute { .. } => ModelErrorCode::ReservedAttribute,
            Self::DuplicateEntity { .. } => ModelErrorCode::DuplicateEntity,
            Self::DuplicateRelation { .. } => ModelErrorCode::DuplicateRelation,
            Self::DuplicateRole { .. } => ModelErrorCode::DuplicateRole,
            Self::EmptyRoles { .. } => ModelErrorCode::EmptyRoles,
            Self::UnknownEntity { .. } => ModelErrorCode::UnknownEntity,
        }
    }
}

/// The stable code of a [`ModelError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorCode {
    InvalidModelName,
    InvalidVersion,
    InvalidTimeZone,
    InvalidEntityName,
    InvalidRelationName,
    InvalidAttributeName,
    InvalidRoleName,
    ReservedAttribute,
    DuplicateEntity,
    DuplicateRelation,
    DuplicateRole,
    EmptyRoles,
    UnknownEntity,
}

impl ModelErrorCode {
    /// Snake-case identifier, stable across releases.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidModelName => "invalid_model_name",
            Self::InvalidVersion => "invalid_version",
            Self::InvalidTimeZone => "invalid_time_zone",
            Self::InvalidEntityName => "invalid_entity_name",
            Self::InvalidRelationName => "invalid_relation_name",
            Self::InvalidAttributeName => "invalid_attribute_name",
            Self::InvalidRoleName => "invalid_role_name",
            Self::ReservedAttribute => "reserved_attribute",
            Self::DuplicateEntity => "duplicate_entity",
            Self::DuplicateRelation => "duplicate_relation",
            Self::DuplicateRole => "duplicate_role",
            Self::EmptyRoles => "empty_roles",
            Self::UnknownEntity => "unknown_entity",
        }
    }
}

impl fmt::Display for ModelErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure to turn raw input text into a model document.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("input is empty")]
    Empty,
    #[error("input is neither valid JSON nor valid YAML: {message}")]
    Syntax { message: String },
    #[error("model document is not shaped as declared: {message}")]
    Shape { message: String },
}

/// Every problem found in one document, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("model is invalid: {errors:?}")]
pub struct InvalidModel {
    pub errors: Vec<ModelError>,
}
