//! Typed queries over relationships and time.
//!
//! A query names a starting entity, a path of roles to follow, and the instant
//! to ask about. It is validated against the model before any SQL is built, so
//! an undeclared role or attribute is a query error rather than a database
//! error, and user text never becomes a SQL identifier.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::Model;

/// The entity a query starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    pub entity_type: String,
    pub id: String,
}

/// One step of a traversal: the relation to cross and the roles at each end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hop {
    pub relation_type: String,
    /// The role the current position fills.
    pub from_role: String,
    /// The role whose participant the traversal moves to.
    pub to_role: String,
}

/// What a query returns for each reached entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Select {
    /// Only the entity ids.
    Ids,
    /// One declared attribute of the reached entity.
    Attribute { attribute: String },
}

/// Bounds on a traversal.
///
/// Unbounded traversal is not offered: a query that walks an arbitrary graph
/// can be made to run for an unbounded time, so every query states a depth and
/// a row budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_depth: usize,
    pub max_rows: usize,
}

impl Limits {
    /// The largest depth a query may ask for.
    pub const MAX_DEPTH: usize = 8;
    /// The largest row budget a query may ask for.
    pub const MAX_ROWS: usize = 10_000;
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_rows: 1_000,
        }
    }
}

/// A query over relationships as of a date and revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub start: EntityRef,
    pub traversal: Vec<Hop>,
    pub select: Select,
    pub valid_at: NaiveDate,
    /// `None` means the newest revision.
    pub known_revision: Option<u64>,
    pub limits: Limits,
}

/// Why a query cannot run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QueryError {
    #[error("entity type {entity_type:?} is not declared in the model")]
    UnknownEntityType { entity_type: String },
    #[error("relation type {relation_type:?} is not declared in the model")]
    UnknownRelation { relation_type: String },
    #[error("relation {relation_type:?} has no role {role:?}")]
    UnknownRole { relation_type: String, role: String },
    #[error(
        "role {role:?} of {relation_type:?} expects {expected:?} but the traversal is at {actual:?}"
    )]
    RoleTypeMismatch {
        relation_type: String,
        role: String,
        expected: String,
        actual: String,
    },
    #[error("attribute {attribute:?} is not declared on {entity_type:?}")]
    UnknownAttribute {
        entity_type: String,
        attribute: String,
    },
    #[error("limit {limit} exceeds the maximum {maximum}")]
    LimitTooLarge { limit: usize, maximum: usize },
    #[error("traversal of {hops} hop(s) exceeds the depth limit of {max_depth}")]
    LimitExceeded { hops: usize, max_depth: usize },
}

impl QueryError {
    /// Stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownEntityType { .. } => "unknown_entity_type",
            Self::UnknownRelation { .. } => "unknown_relation",
            Self::UnknownRole { .. } => "unknown_role",
            Self::RoleTypeMismatch { .. } => "role_type_mismatch",
            Self::UnknownAttribute { .. } => "unknown_attribute",
            Self::LimitTooLarge { .. } => "limit_too_large",
            Self::LimitExceeded { .. } => "limit_exceeded",
        }
    }
}

/// Checks a query against the model.
///
/// Every name in the query must resolve, and each hop must continue from the
/// entity type the previous hop reached. Doing this here means the compiler
/// only ever sees names that came from the model.
pub fn validate_query(model: &Model, query: &Query) -> Result<(), QueryError> {
    if query.limits.max_depth > Limits::MAX_DEPTH {
        return Err(QueryError::LimitTooLarge {
            limit: query.limits.max_depth,
            maximum: Limits::MAX_DEPTH,
        });
    }
    if query.limits.max_rows > Limits::MAX_ROWS {
        return Err(QueryError::LimitTooLarge {
            limit: query.limits.max_rows,
            maximum: Limits::MAX_ROWS,
        });
    }
    if query.traversal.len() > query.limits.max_depth {
        return Err(QueryError::LimitExceeded {
            hops: query.traversal.len(),
            max_depth: query.limits.max_depth,
        });
    }

    if !model.entities.contains_key(&query.start.entity_type) {
        return Err(QueryError::UnknownEntityType {
            entity_type: query.start.entity_type.clone(),
        });
    }

    let mut position = query.start.entity_type.clone();
    for hop in &query.traversal {
        let Some(relation) = model.relations.get(&hop.relation_type) else {
            return Err(QueryError::UnknownRelation {
                relation_type: hop.relation_type.clone(),
            });
        };
        let Some(from) = relation.roles.get(&hop.from_role) else {
            return Err(QueryError::UnknownRole {
                relation_type: hop.relation_type.clone(),
                role: hop.from_role.clone(),
            });
        };
        if from.entity != position {
            return Err(QueryError::RoleTypeMismatch {
                relation_type: hop.relation_type.clone(),
                role: hop.from_role.clone(),
                expected: from.entity.clone(),
                actual: position,
            });
        }
        let Some(to) = relation.roles.get(&hop.to_role) else {
            return Err(QueryError::UnknownRole {
                relation_type: hop.relation_type.clone(),
                role: hop.to_role.clone(),
            });
        };
        position.clone_from(&to.entity);
    }

    if let Select::Attribute { attribute } = &query.select {
        let Some(entity) = model.entities.get(&position) else {
            return Err(QueryError::UnknownEntityType {
                entity_type: position,
            });
        };
        if !entity.attributes.contains_key(attribute) {
            return Err(QueryError::UnknownAttribute {
                entity_type: position,
                attribute: attribute.clone(),
            });
        }
    }

    Ok(())
}
