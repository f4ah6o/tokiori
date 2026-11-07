use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use tokiori_model::{AttributeType, Cardinality, Model, ValidTime};

use crate::error::{ApplyOutcome, PgError};

/// Opens a connection pool.
pub async fn connect(database_url: &str) -> Result<PgPool, PgError> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Applies every embedded migration.
pub async fn migrate(pool: &PgPool) -> Result<(), PgError> {
    crate::MIGRATOR.run(pool).await?;
    Ok(())
}

/// Applies migrations from an explicit migrator, used by tests that own their
/// own schema fixtures.
pub async fn migrate_with(
    pool: &PgPool,
    migrator: &sqlx::migrate::Migrator,
) -> Result<(), PgError> {
    migrator.run(pool).await?;
    Ok(())
}

/// A request to create one entity.
#[derive(Debug, Clone)]
pub struct NewEntity {
    pub scope: String,
    pub entity_type: String,
    pub id: String,
    pub attributes: Map<String, Value>,
}

/// A stored entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityRow {
    pub scope: String,
    pub entity_type: String,
    pub id: String,
    pub model: String,
    pub model_version: i32,
    pub attributes: Map<String, Value>,
}

/// A request to create one relation instance.
#[derive(Debug, Clone)]
pub struct NewRelation {
    pub scope: String,
    pub relation_type: String,
    pub id: String,
    pub roles: BTreeMap<String, String>,
    pub attributes: Map<String, Value>,
    pub valid_time: ValidTime,
}

/// A stored relation together with its role participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRow {
    pub scope: String,
    pub relation_type: String,
    pub id: String,
    pub model: String,
    pub model_version: i32,
    pub attributes: Map<String, Value>,
    pub valid_time: ValidTime,
    /// Role name to participant entity id, in role order.
    pub roles: BTreeMap<String, String>,
}

/// PostgreSQL-backed store for one database.
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Stores a model version, or confirms an identical one is already stored.
    ///
    /// A version is immutable once stored: re-applying the same definition is a
    /// no-op, and re-applying a different definition under the same version is
    /// rejected instead of silently rewriting history.
    pub async fn apply_model(&self, scope: &str, model: &Model) -> Result<ApplyOutcome, PgError> {
        let definition: Value = serde_json::from_str(&tokiori_model::canonical_json(model))
            .expect("canonical model JSON is valid JSON");
        let digest = tokiori_model::model_digest(model);
        let mut tx = self.pool.begin().await?;

        let existing: Option<String> = sqlx::query_scalar(
            "SELECT digest FROM models WHERE scope_id = $1 AND model = $2 AND version = $3 FOR UPDATE",
        )
        .bind(scope)
        .bind(&model.model)
        .bind(i64::from(model.version))
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(stored) = existing {
            tx.rollback().await?;
            if stored == digest {
                return Ok(ApplyOutcome::Unchanged);
            }
            return Err(PgError::ModelVersionConflict {
                model: model.model.clone(),
                version: model.version,
                stored,
                incoming: digest,
            });
        }

        sqlx::query(
            "INSERT INTO models (scope_id, model, version, time_zone, definition, digest) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(scope)
        .bind(&model.model)
        .bind(i64::from(model.version))
        .bind(model.time_zone.to_string())
        .bind(definition)
        .bind(digest)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(ApplyOutcome::Created)
    }

    /// Loads the newest applied model in a scope.
    pub async fn latest_model(&self, scope: &str) -> Result<Model, PgError> {
        let definition: Option<Value> = sqlx::query_scalar(
            "SELECT definition FROM models WHERE scope_id = $1 ORDER BY version DESC LIMIT 1",
        )
        .bind(scope)
        .fetch_optional(&self.pool)
        .await?;

        let Some(definition) = definition else {
            return Err(PgError::ModelNotFound {
                scope: scope.to_owned(),
            });
        };
        let text = serde_json::to_string(&definition).expect("stored definition is JSON");
        tokiori_model::parse_model_str(&text)
            .map_err(|err| PgError::Db(sqlx::Error::Decode(Box::new(err))))
    }

    /// Creates an entity, rejecting the write when the id already exists.
    pub async fn create_entity(&self, input: NewEntity) -> Result<EntityRow, PgError> {
        let mut tx = self.pool.begin().await?;
        let (model_name, model_version) = self.lock_latest_model(&mut tx, &input.scope).await?;
        let model = self
            .load_model_in(&mut tx, &input.scope, &model_name, model_version)
            .await?;

        let Some(entity) = model.entities.get(&input.entity_type) else {
            return Err(PgError::EntityTypeNotFound {
                entity_type: input.entity_type.clone(),
            });
        };
        check_attributes(&input.entity_type, &entity.attributes, &input.attributes)?;

        let attributes = Value::Object(input.attributes.clone());
        let inserted = sqlx::query(
            "INSERT INTO entities (scope_id, model, model_version, entity_type, id, attributes) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
        )
        .bind(&input.scope)
        .bind(&model_name)
        .bind(model_version)
        .bind(&input.entity_type)
        .bind(&input.id)
        .bind(attributes)
        .execute(&mut *tx)
        .await?;

        if inserted.rows_affected() == 0 {
            return Err(PgError::DuplicateId {
                scope: input.scope,
                entity_type: input.entity_type,
                id: input.id,
            });
        }
        tx.commit().await?;

        Ok(EntityRow {
            scope: input.scope,
            entity_type: input.entity_type,
            id: input.id,
            model: model_name,
            model_version,
            attributes: input.attributes,
        })
    }

    /// Reads one entity in a scope.
    pub async fn get_entity(
        &self,
        scope: &str,
        entity_type: &str,
        id: &str,
    ) -> Result<Option<EntityRow>, PgError> {
        let row: Option<(String, String, String, i32, Value)> = sqlx::query_as(
            "SELECT model, entity_type, id, model_version, attributes FROM entities \
             WHERE scope_id = $1 AND entity_type = $2 AND id = $3",
        )
        .bind(scope)
        .bind(entity_type)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(model, entity_type, id, model_version, attributes)| EntityRow {
                scope: scope.to_owned(),
                entity_type,
                id,
                model,
                model_version,
                attributes: as_object(attributes),
            },
        ))
    }

    /// Creates a relation after checking every role against the model and the
    /// scope of the participants.
    pub async fn create_relation(&self, input: NewRelation) -> Result<RelationRow, PgError> {
        let mut tx = self.pool.begin().await?;
        let (model_name, model_version) = self.lock_latest_model(&mut tx, &input.scope).await?;
        let model = self
            .load_model_in(&mut tx, &input.scope, &model_name, model_version)
            .await?;

        let Some(relation) = model.relations.get(&input.relation_type) else {
            return Err(PgError::RelationTypeNotFound {
                relation_type: input.relation_type.clone(),
            });
        };
        check_attributes(
            &input.relation_type,
            &relation.attributes,
            &input.attributes,
        )?;

        check_role_names(relation, &input)?;
        let participants = self.resolve_participants(&mut tx, &input, relation).await?;

        let attributes = Value::Object(input.attributes.clone());
        sqlx::query(
            "INSERT INTO relations \
             (scope_id, model, model_version, relation_type, id, attributes, valid_from, valid_until) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
        )
        .bind(&input.scope)
        .bind(&model_name)
        .bind(model_version)
        .bind(&input.relation_type)
        .bind(&input.id)
        .bind(attributes)
        .bind(input.valid_time.from)
        .bind(input.valid_time.until)
        .execute(&mut *tx)
        .await?;

        // The roles are written twice on purpose: the current-state table is
        // what the P0 read path returns, and the versioned table is what the
        // temporal read path and queries traverse. Writing only one of them
        // makes the relation invisible to the other.
        let revision = crate::revision::next_revision_in(&mut tx, &input.scope).await?;
        let recorded = i64::try_from(revision).expect("revision fits");

        for (role_name, entity_id) in &participants {
            let role = &relation.roles[role_name];
            sqlx::query(
                "INSERT INTO relation_roles \
                 (scope_id, relation_type, relation_id, role, participant_entity_type, participant_id) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&input.scope)
            .bind(&input.relation_type)
            .bind(&input.id)
            .bind(role_name)
            .bind(&role.entity)
            .bind(entity_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                "INSERT INTO relation_role_versions \
                 (scope_id, relation_type, relation_id, role, valid_from, valid_until, \
                  recorded_from, participant_entity_type, participant_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&input.scope)
            .bind(&input.relation_type)
            .bind(&input.id)
            .bind(role_name)
            .bind(input.valid_time.from)
            .bind(input.valid_time.until)
            .bind(recorded)
            .bind(&role.entity)
            .bind(entity_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(RelationRow {
            scope: input.scope,
            relation_type: input.relation_type,
            id: input.id,
            model: model_name,
            model_version,
            attributes: input.attributes,
            valid_time: input.valid_time,
            roles: participants,
        })
    }

    /// Reads one relation and its role participants in a scope.
    pub async fn get_relation(
        &self,
        scope: &str,
        relation_type: &str,
        id: &str,
    ) -> Result<Option<RelationRow>, PgError> {
        let row: Option<(String, i32, Value, NaiveDate, Option<NaiveDate>)> = sqlx::query_as(
            "SELECT model, model_version, attributes, valid_from, valid_until FROM relations \
             WHERE scope_id = $1 AND relation_type = $2 AND id = $3",
        )
        .bind(scope)
        .bind(relation_type)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((model, model_version, attributes, from, until)) = row else {
            return Ok(None);
        };

        let role_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT role, participant_id FROM relation_roles \
             WHERE scope_id = $1 AND relation_type = $2 AND relation_id = $3 ORDER BY role",
        )
        .bind(scope)
        .bind(relation_type)
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        let valid_time = match until {
            None => ValidTime::unbounded_from(from),
            Some(until) => ValidTime::bounded(from, until)
                .map_err(|err| PgError::Db(sqlx::Error::Decode(Box::new(err))))?,
        };

        Ok(Some(RelationRow {
            scope: scope.to_owned(),
            relation_type: relation_type.to_owned(),
            id: id.to_owned(),
            model,
            model_version,
            attributes: as_object(attributes),
            valid_time,
            roles: role_rows.into_iter().collect(),
        }))
    }

    /// Resolves every role participant, rejecting ids that are not visible in the
    /// scope and ids whose entity type does not match the role.
    async fn resolve_participants(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        input: &NewRelation,
        relation: &tokiori_model::Relation,
    ) -> Result<BTreeMap<String, String>, PgError> {
        let mut participants = BTreeMap::new();
        for (role_name, entity_id) in &input.roles {
            let role = &relation.roles[role_name];
            // Look the id up without filtering on type: a participant of the
            // wrong type is a modelling mistake the author must see, and it
            // must not be reported as a missing entity.
            let found: Option<String> = sqlx::query_scalar(
                "SELECT entity_type FROM entities WHERE scope_id = $1 AND id = $2 LIMIT 1",
            )
            .bind(&input.scope)
            .bind(entity_id)
            .fetch_optional(&mut **tx)
            .await?;

            match found {
                None => {
                    return Err(PgError::ScopeMismatch {
                        scope: input.scope.clone(),
                        entity_type: role.entity.clone(),
                        id: entity_id.clone(),
                    });
                }
                Some(actual) if actual != role.entity => {
                    return Err(PgError::EntityTypeMismatch {
                        role: role_name.clone(),
                        expected: role.entity.clone(),
                        actual,
                    });
                }
                Some(_) => {}
            }
            participants.insert(role_name.clone(), entity_id.clone());
        }
        Ok(participants)
    }

    /// The next revision to assign in this store's scope.
    pub async fn next_revision(&self, scope: &str) -> Result<u64, PgError> {
        crate::revision::next_revision(&self.pool, scope).await
    }

    /// The newest revision assigned in a scope, if any.
    pub async fn latest_revision(&self, scope: &str) -> Result<Option<u64>, PgError> {
        crate::revision::latest_revision(&self.pool, scope).await
    }

    /// Takes a row lock on the newest model so concurrent writers in one scope
    /// serialize on a stable definition.
    async fn lock_latest_model(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: &str,
    ) -> Result<(String, i32), PgError> {
        let row: Option<(String, i32)> = sqlx::query_as(
            "SELECT model, version FROM models WHERE scope_id = $1 \
             ORDER BY version DESC LIMIT 1 FOR UPDATE",
        )
        .bind(scope)
        .fetch_optional(&mut **tx)
        .await?;

        row.ok_or_else(|| PgError::ModelNotFound {
            scope: scope.to_owned(),
        })
    }

    /// Loads the newest applied model using an existing transaction.
    pub(crate) async fn load_latest_model(
        tx: &mut Transaction<'_, Postgres>,
        scope: &str,
    ) -> Result<Model, PgError> {
        let row: Option<(String, i32)> = sqlx::query_as(
            "SELECT model, version FROM models WHERE scope_id = $1 \
             ORDER BY version DESC LIMIT 1 FOR SHARE",
        )
        .bind(scope)
        .fetch_optional(&mut **tx)
        .await?;
        let Some((model, version)) = row else {
            return Err(PgError::ModelNotFound {
                scope: scope.to_owned(),
            });
        };
        let definition: Value = sqlx::query_scalar(
            "SELECT definition FROM models WHERE scope_id = $1 AND model = $2 AND version = $3",
        )
        .bind(scope)
        .bind(&model)
        .bind(version)
        .fetch_one(&mut **tx)
        .await?;
        let text = serde_json::to_string(&definition).expect("stored definition is JSON");
        tokiori_model::parse_model_str(&text)
            .map_err(|err| PgError::Db(sqlx::Error::Decode(Box::new(err))))
    }

    async fn load_model_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        scope: &str,
        model: &str,
        version: i32,
    ) -> Result<Model, PgError> {
        let definition: Value = sqlx::query_scalar(
            "SELECT definition FROM models WHERE scope_id = $1 AND model = $2 AND version = $3",
        )
        .bind(scope)
        .bind(model)
        .bind(version)
        .fetch_one(&mut **tx)
        .await?;

        let text = serde_json::to_string(&definition).expect("stored definition is JSON");
        tokiori_model::parse_model_str(&text)
            .map_err(|err| PgError::Db(sqlx::Error::Decode(Box::new(err))))
    }
}

/// Every provided role must be declared, and every declared role must be
/// provided.
fn check_role_names(
    relation: &tokiori_model::Relation,
    input: &NewRelation,
) -> Result<(), PgError> {
    for role in input.roles.keys() {
        if !relation.roles.contains_key(role) {
            return Err(PgError::UnknownRole {
                relation_type: input.relation_type.clone(),
                role: role.clone(),
            });
        }
    }
    for role_name in relation.roles.keys() {
        if !input.roles.contains_key(role_name) {
            return Err(PgError::MissingRole {
                relation_type: input.relation_type.clone(),
                role: role_name.clone(),
            });
        }
    }
    Ok(())
}

fn as_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Checks that a value fits the declared type of one attribute.
pub(crate) fn check_attribute_value(
    attribute: &str,
    declared: AttributeType,
    value: &Value,
) -> Result<(), PgError> {
    if value_matches(declared, value) {
        return Ok(());
    }
    Err(PgError::InvalidAttribute {
        owner: attribute.to_owned(),
        attribute: attribute.to_owned(),
        expected: type_name(declared),
        actual: value.to_string(),
    })
}

/// Loads the declared type of one attribute of one entity.
///
/// The entity must exist in the scope and the attribute must be declared on
/// its type; both are checked here so the caller cannot record a fact about
/// something the model does not describe.
pub(crate) async fn load_declared_attribute(
    tx: &mut Transaction<'_, Postgres>,
    scope: &str,
    entity_type: &str,
    entity_id: &str,
    attribute: &str,
) -> Result<AttributeType, PgError> {
    let model = PgStore::load_latest_model(tx, scope).await?;

    // The owner may be an entity or a relation instance; both carry declared
    // attributes and both can have a history.
    let declared = if let Some(entity) = model.entities.get(entity_type) {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE scope_id = $1 AND entity_type = $2 AND id = $3",
        )
        .bind(scope)
        .bind(entity_type)
        .bind(entity_id)
        .fetch_optional(&mut **tx)
        .await?;
        if exists.is_none() {
            return Err(PgError::ScopeMismatch {
                scope: scope.to_owned(),
                entity_type: entity_type.to_owned(),
                id: entity_id.to_owned(),
            });
        }
        entity.attributes.get(attribute).copied()
    } else if let Some(relation) = model.relations.get(entity_type) {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM relations WHERE scope_id = $1 AND relation_type = $2 AND id = $3",
        )
        .bind(scope)
        .bind(entity_type)
        .bind(entity_id)
        .fetch_optional(&mut **tx)
        .await?;
        if exists.is_none() {
            return Err(PgError::ScopeMismatch {
                scope: scope.to_owned(),
                entity_type: entity_type.to_owned(),
                id: entity_id.to_owned(),
            });
        }
        relation.attributes.get(attribute).copied()
    } else {
        return Err(PgError::EntityTypeNotFound {
            entity_type: entity_type.to_owned(),
        });
    };

    declared.ok_or_else(|| PgError::UnknownAttribute {
        owner: entity_type.to_owned(),
        attribute: attribute.to_owned(),
    })
}

fn check_attributes(
    owner: &str,
    declared: &BTreeMap<String, AttributeType>,
    provided: &Map<String, Value>,
) -> Result<(), PgError> {
    for (name, value) in provided {
        let Some(expected) = declared.get(name) else {
            return Err(PgError::UnknownAttribute {
                owner: owner.to_owned(),
                attribute: name.clone(),
            });
        };
        if !value_matches(*expected, value) {
            return Err(PgError::InvalidAttribute {
                owner: owner.to_owned(),
                attribute: name.clone(),
                expected: type_name(*expected),
                actual: value.to_string(),
            });
        }
    }
    Ok(())
}

fn value_matches(expected: AttributeType, value: &Value) -> bool {
    match expected {
        AttributeType::Text => matches!(value, Value::String(_)),
        AttributeType::Integer => matches!(value, Value::Number(n) if n.is_i64() || n.is_u64()),
        AttributeType::Boolean => matches!(value, Value::Bool(_)),
        AttributeType::Date => matches!(
            value,
            Value::String(s) if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
        ),
    }
}

fn type_name(t: AttributeType) -> &'static str {
    match t {
        AttributeType::Text => "text",
        AttributeType::Integer => "integer",
        AttributeType::Boolean => "boolean",
        AttributeType::Date => "date (YYYY-MM-DD)",
    }
}

#[allow(dead_code)]
fn _cardinality_is_used(c: Cardinality) -> Cardinality {
    c
}

#[allow(dead_code)]
fn _now() -> DateTime<Utc> {
    Utc::now()
}
