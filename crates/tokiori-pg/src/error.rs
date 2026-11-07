use thiserror::Error;

/// The result of applying a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The model version was not stored before and is now.
    Created,
    /// The identical model version was already stored; nothing changed.
    Unchanged,
}

/// A rejected store operation.
#[derive(Debug, Error)]
pub enum PgError {
    #[error("no model has been applied in scope {scope:?}")]
    ModelNotFound { scope: String },
    #[error("model {model:?} version {version} was already applied with different contents")]
    ModelVersionConflict {
        model: String,
        version: u32,
        stored: String,
        incoming: String,
    },
    #[error("entity type {entity_type:?} is not declared in the applied model")]
    EntityTypeNotFound { entity_type: String },
    #[error("relation type {relation_type:?} is not declared in the applied model")]
    RelationTypeNotFound { relation_type: String },
    #[error("relation type {relation_type:?} has no role {role:?}")]
    UnknownRole { relation_type: String, role: String },
    #[error("role {role:?} expects entity type {expected:?} but got {actual:?}")]
    EntityTypeMismatch {
        role: String,
        expected: String,
        actual: String,
    },
    #[error("relation type {relation_type:?} requires role {role:?}, which was not provided")]
    MissingRole { relation_type: String, role: String },
    #[error("role {role:?} admits one participant but {count} were provided")]
    CardinalityViolation { role: String, count: usize },
    #[error("{entity_type:?} {id:?} already exists in scope {scope:?}")]
    DuplicateId {
        scope: String,
        entity_type: String,
        id: String,
    },
    #[error("{entity_type:?} {id:?} is not visible in scope {scope:?}")]
    ScopeMismatch {
        scope: String,
        entity_type: String,
        id: String,
    },
    #[error("attribute {attribute:?} is not declared on {owner:?}")]
    UnknownAttribute { owner: String, attribute: String },
    #[error("attribute {attribute:?} on {owner:?} must be {expected} but got {actual}")]
    InvalidAttribute {
        owner: String,
        attribute: String,
        expected: &'static str,
        actual: String,
    },
    #[error("{entity_type:?} {entity_id:?} has no current value for {attribute:?}")]
    NothingToEnd {
        entity_type: String,
        entity_id: String,
        attribute: String,
    },
    #[error("revision {revision} was never assigned in scope {scope:?} (latest is {latest:?})")]
    UnknownRevision {
        scope: String,
        revision: u64,
        latest: Option<u64>,
    },
    #[error("operation {operation_type:?} {idempotency_key:?} was already recorded")]
    OperationReplayed {
        operation_type: String,
        idempotency_key: String,
    },
    #[error(
        "operation {operation_type:?} {idempotency_key:?} was already used with different input"
    )]
    IdempotencyConflict {
        operation_type: String,
        idempotency_key: String,
        stored: String,
        incoming: String,
    },
    #[error("valid interval starting {from} and ending {until} is empty")]
    EmptyInterval { from: String, until: String },
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

impl PgError {
    /// Stable machine-readable code used by the CLI and by tests.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::ModelNotFound { .. } => "model_not_found",
            Self::ModelVersionConflict { .. } => "model_version_conflict",
            Self::EntityTypeNotFound { .. } => "entity_type_not_found",
            Self::RelationTypeNotFound { .. } => "relation_type_not_found",
            Self::UnknownRole { .. } => "unknown_role",
            Self::EntityTypeMismatch { .. } => "entity_type_mismatch",
            Self::MissingRole { .. } => "missing_role",
            Self::CardinalityViolation { .. } => "cardinality_violation",
            Self::DuplicateId { .. } => "duplicate_id",
            Self::ScopeMismatch { .. } => "scope_mismatch",
            Self::UnknownAttribute { .. } => "unknown_attribute",
            Self::InvalidAttribute { .. } => "invalid_attribute",
            Self::NothingToEnd { .. } => "nothing_to_end",
            Self::UnknownRevision { .. } => "unknown_revision",
            Self::OperationReplayed { .. } => "operation_replayed",
            Self::IdempotencyConflict { .. } => "idempotency_conflict",
            Self::EmptyInterval { .. } => "empty_interval",
            Self::Db(_) => "database_error",
            Self::Migrate(_) => "migration_error",
        }
    }
}
