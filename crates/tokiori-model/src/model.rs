use std::collections::BTreeMap;

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::{InvalidModel, ModelError, ParseError};

/// Attribute names the engine owns on every entity or relation.
const RESERVED_ATTRIBUTES: [&str; 5] = [
    "id",
    "scope_id",
    "model_version",
    "created_at",
    "updated_at",
];

/// The value type an attribute may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    Text,
    Integer,
    Boolean,
    Date,
}

/// How many participants a role admits inside one relation instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    One,
    Many,
}

/// A role as written in the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDef {
    pub entity: String,
    pub cardinality: Cardinality,
}

/// An entity type as written in the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityDef {
    pub name: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeType>,
}

/// A relation type as written in the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationDef {
    pub name: String,
    #[serde(default)]
    pub roles: BTreeMap<String, RoleDef>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeType>,
}

/// The unnormalized, author-facing model document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDocument {
    pub model: String,
    pub version: u32,
    pub time_zone: String,
    #[serde(default)]
    pub entities: Vec<EntityDef>,
    #[serde(default)]
    pub relations: Vec<RelationDef>,
}

/// An entity type after normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    pub attributes: BTreeMap<String, AttributeType>,
}

/// A role after normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub entity: String,
    pub cardinality: Cardinality,
}

/// A relation type after normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    pub name: String,
    pub roles: BTreeMap<String, Role>,
    pub attributes: BTreeMap<String, AttributeType>,
}

/// The normalized model AST.
///
/// Collections are ordered maps so that serialization, digests, and error
/// ordering never depend on the order in which the author wrote the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub model: String,
    pub version: u32,
    pub time_zone: Tz,
    pub entities: BTreeMap<String, Entity>,
    pub relations: BTreeMap<String, Relation>,
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_model_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn check_attributes(attributes: &BTreeMap<String, AttributeType>, errors: &mut Vec<ModelError>) {
    for name in attributes.keys() {
        if !is_identifier(name) {
            errors.push(ModelError::InvalidAttributeName { name: name.clone() });
        }
        if RESERVED_ATTRIBUTES.contains(&name.as_str()) {
            errors.push(ModelError::ReservedAttribute {
                attribute: name.clone(),
            });
        }
    }
}

/// Validates and canonicalizes a document into a [`Model`].
///
/// Every problem found is reported; the function returns no partial model when
/// any error exists, so callers can never persist a half-validated definition.
pub fn normalize(doc: ModelDocument) -> Result<Model, InvalidModel> {
    let mut errors = Vec::new();

    if !is_model_name(&doc.model) {
        errors.push(ModelError::InvalidModelName {
            name: doc.model.clone(),
        });
    }
    if doc.version == 0 {
        errors.push(ModelError::InvalidVersion {
            version: doc.version,
        });
    }
    let time_zone = doc.time_zone.parse::<Tz>().ok();
    if time_zone.is_none() {
        errors.push(ModelError::InvalidTimeZone {
            time_zone: doc.time_zone.clone(),
        });
    }

    let entities = normalize_entities(&doc.entities, &mut errors);
    let relations = normalize_relations(&doc.relations, &entities, &mut errors);

    let Some(time_zone) = time_zone else {
        return Err(InvalidModel { errors });
    };
    if !errors.is_empty() {
        return Err(InvalidModel { errors });
    }
    Ok(Model {
        model: doc.model,
        version: doc.version,
        time_zone,
        entities,
        relations,
    })
}

fn normalize_entities(
    defs: &[EntityDef],
    errors: &mut Vec<ModelError>,
) -> BTreeMap<String, Entity> {
    let mut entities = BTreeMap::new();
    for def in defs {
        if !is_identifier(&def.name) {
            errors.push(ModelError::InvalidEntityName {
                name: def.name.clone(),
            });
        }
        if entities.contains_key(&def.name) {
            errors.push(ModelError::DuplicateEntity {
                entity: def.name.clone(),
            });
            continue;
        }
        check_attributes(&def.attributes, errors);
        entities.insert(
            def.name.clone(),
            Entity {
                name: def.name.clone(),
                attributes: def.attributes.clone(),
            },
        );
    }
    entities
}

fn normalize_relations(
    defs: &[RelationDef],
    entities: &BTreeMap<String, Entity>,
    errors: &mut Vec<ModelError>,
) -> BTreeMap<String, Relation> {
    let mut relations = BTreeMap::new();
    for def in defs {
        if !is_identifier(&def.name) {
            errors.push(ModelError::InvalidRelationName {
                name: def.name.clone(),
            });
        }
        if relations.contains_key(&def.name) {
            errors.push(ModelError::DuplicateRelation {
                relation: def.name.clone(),
            });
            continue;
        }
        if def.roles.is_empty() {
            errors.push(ModelError::EmptyRoles {
                relation: def.name.clone(),
            });
        }
        check_attributes(&def.attributes, errors);

        let mut roles = BTreeMap::new();
        for (role_name, role_def) in &def.roles {
            if !is_identifier(role_name) {
                errors.push(ModelError::InvalidRoleName {
                    name: role_name.clone(),
                });
            }
            if !entities.contains_key(&role_def.entity) {
                errors.push(ModelError::UnknownEntity {
                    relation: def.name.clone(),
                    role: role_name.clone(),
                    entity: role_def.entity.clone(),
                });
            }
            roles.insert(
                role_name.clone(),
                Role {
                    entity: role_def.entity.clone(),
                    cardinality: role_def.cardinality,
                },
            );
        }
        relations.insert(
            def.name.clone(),
            Relation {
                name: def.name.clone(),
                roles,
                attributes: def.attributes.clone(),
            },
        );
    }
    relations
}

/// Parses author input as JSON when it starts with an object brace, otherwise
/// as YAML, then normalizes it.
///
/// JSON is tried first for brace-initial input because JSON is the canonical
/// interchange form; YAML remains the human-friendly alternative.
pub fn parse_model_str(input: &str) -> Result<Model, ParseError> {
    let trimmed = input.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let doc: ModelDocument = if trimmed.starts_with('{') {
        match serde_json::from_str(trimmed) {
            Ok(doc) => doc,
            Err(json_err) => match serde_yaml_ng::from_str(trimmed) {
                Ok(doc) => doc,
                Err(_) => {
                    return Err(ParseError::Shape {
                        message: json_err.to_string(),
                    });
                }
            },
        }
    } else {
        serde_yaml_ng::from_str(trimmed).map_err(|err| ParseError::Syntax {
            message: err.to_string(),
        })?
    };
    normalize(doc).map_err(|invalid| ParseError::Syntax {
        message: invalid.to_string(),
    })
}
