use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{EntityDef, Model, RelationDef, RoleDef};

/// The author-facing document shape, rebuilt from a normalized model.
///
/// Canonical output deliberately uses the document shape rather than the
/// normalized maps: the file an operator reads, the file they can re-apply, and
/// the bytes the digest covers are then the same artifact.
#[derive(Serialize)]
struct CanonicalDocument<'a> {
    model: &'a str,
    version: u32,
    time_zone: String,
    entities: Vec<EntityDef>,
    relations: Vec<RelationDef>,
}

impl<'a> From<&'a Model> for CanonicalDocument<'a> {
    fn from(model: &'a Model) -> Self {
        let entities = model
            .entities
            .values()
            .map(|entity| EntityDef {
                name: entity.name.clone(),
                attributes: entity.attributes.clone(),
            })
            .collect();
        let relations = model
            .relations
            .values()
            .map(|relation| RelationDef {
                name: relation.name.clone(),
                roles: relation
                    .roles
                    .iter()
                    .map(|(name, role)| {
                        (
                            name.clone(),
                            RoleDef {
                                entity: role.entity.clone(),
                                cardinality: role.cardinality,
                            },
                        )
                    })
                    .collect(),
                attributes: relation.attributes.clone(),
            })
            .collect();
        Self {
            model: &model.model,
            version: model.version,
            time_zone: model.time_zone.to_string(),
            entities,
            relations,
        }
    }
}

/// Serializes the model to canonical JSON in document shape.
///
/// The output is deterministic for a given model and is itself valid author
/// input, which is what makes the stored digest a reliable identity for a
/// version and lets an applied model be exported and re-applied verbatim.
#[must_use]
pub fn canonical_json(model: &Model) -> String {
    let doc = CanonicalDocument::from(model);
    serde_json::to_string(&doc).expect("model serialization cannot fail")
}

/// SHA-256 digest of the canonical JSON, prefixed with the algorithm name.
#[must_use]
pub fn model_digest(model: &Model) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_json(model).as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
