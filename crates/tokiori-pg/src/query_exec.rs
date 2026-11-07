//! Running a query, and the naive interpreter it is checked against.
//!
//! Two implementations answer the same question on purpose. The compiled one
//! runs the SQL the compiler produced; the naive one walks one hop at a time
//! through the existing read path. The property tests compare them, so a bug
//! in the compiler cannot hide behind an identical bug in the interpreter.

use serde_json::Value;
use sqlx::{AssertSqlSafe, Row};
use tokiori_model::{Query, Select, validate_query};

use crate::compiler::compile_query;
use crate::error::PgError;
use crate::roles::ReadRole;
use crate::store::PgStore;

/// One row a query returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRow {
    pub id: String,
    /// Present when the query selected an attribute.
    pub value: Option<String>,
}

/// What a query found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResult {
    pub rows: Vec<QueryRow>,
    /// True when the row budget stopped the query before it finished.
    pub truncated: bool,
    /// The revision the answer was resolved at.
    pub known_revision: Option<u64>,
}

impl PgStore {
    /// Runs a query through the compiled SQL.
    pub async fn query(&self, scope: &str, query: &Query) -> Result<QueryResult, PgError> {
        // A scope with no model has nothing to query; that is an empty scope,
        // not a broken query.
        let model = self.latest_model(scope).await.map_err(|err| match err {
            PgError::ModelNotFound { .. } => PgError::NotFound {
                what: format!("scope {scope}"),
            },
            other => other,
        })?;
        validate_query(&model, query).map_err(PgError::InvalidQuery)?;

        let known = match query.known_revision {
            Some(revision) => Some(revision),
            None => self.latest_revision(scope).await?,
        };
        if let Some(revision) = known
            && self
                .latest_revision(scope)
                .await?
                .is_none_or(|latest| revision > latest)
        {
            return Err(PgError::UnknownRevision {
                scope: scope.to_owned(),
                revision,
                latest: self.latest_revision(scope).await?,
            });
        }

        // The start must exist, otherwise "no rows" would hide a typo.
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE scope_id = $1 AND entity_type = $2 AND id = $3",
        )
        .bind(scope)
        .bind(&query.start.entity_type)
        .bind(&query.start.id)
        .fetch_optional(self.pool())
        .await?;
        if exists.is_none() {
            return Err(PgError::NotFound {
                what: format!("{} {}", query.start.entity_type, query.start.id),
            });
        }

        // The compiler binds the revision it is given, so resolve the newest
        // one first rather than asking it to mean "latest".
        let mut resolved = query.clone();
        resolved.known_revision = known;
        let compiled = compile_query(&model, &resolved)?;
        // The SQL is dynamic by design: its shape depends on the traversal.
        // It is safe because every identifier in it came from the validated
        // model and every value is bound, which the compiler tests assert.
        let mut statement = sqlx::query(AssertSqlSafe(compiled.sql.clone()));
        statement = statement.bind(scope);
        for param in &compiled.params[1..] {
            statement = statement.bind(param.clone());
        }
        let rows = statement.fetch_all(self.pool()).await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let value: Option<Value> = row.try_get("value").unwrap_or(None);
            out.push(QueryRow {
                id,
                value: value.map(|v| match v {
                    Value::String(text) => text,
                    other => other.to_string(),
                }),
            });
        }
        let truncated = out.len() >= query.limits.max_rows;

        Ok(QueryResult {
            rows: out,
            truncated,
            known_revision: known,
        })
    }

    /// Runs the same query one hop at a time, through the read path.
    ///
    /// Deliberately naive: no joins, no shared compiler code. It exists to
    /// disagree with the compiled path when one of them is wrong.
    pub async fn query_naive(&self, scope: &str, query: &Query) -> Result<QueryResult, PgError> {
        let model = self.latest_model(scope).await?;
        validate_query(&model, query).map_err(PgError::InvalidQuery)?;

        let known = match query.known_revision {
            Some(revision) => Some(revision),
            None => self.latest_revision(scope).await?,
        };

        let mut frontier = vec![query.start.id.clone()];
        for hop in &query.traversal {
            let mut next = Vec::new();
            for id in &frontier {
                let memberships: Vec<String> = sqlx::query_scalar(
                    "SELECT DISTINCT relation_id FROM relation_role_versions \
                     WHERE scope_id = $1 AND relation_type = $2 AND role = $3 AND participant_id = $4",
                )
                .bind(scope)
                .bind(&hop.relation_type)
                .bind(&hop.from_role)
                .bind(id)
                .fetch_all(self.pool())
                .await?;

                for membership in memberships {
                    let answer = self
                        .read_role(ReadRole {
                            scope: scope.to_owned(),
                            relation_type: hop.relation_type.clone(),
                            relation_id: membership,
                            role: hop.to_role.clone(),
                            valid_at: query.valid_at,
                            known_revision: known,
                        })
                        .await?;
                    if let Some(participant) = answer.participant_id
                        && !next.contains(&participant)
                    {
                        next.push(participant);
                    }
                }
            }
            frontier = next;
        }

        let mut rows = Vec::with_capacity(frontier.len());
        for id in &frontier {
            let value = match &query.select {
                Select::Ids => None,
                Select::Attribute { attribute } => {
                    let answer = self
                        .read_attribute(crate::facts::ReadAttribute {
                            scope: scope.to_owned(),
                            entity_type: model
                                .entities
                                .iter()
                                .find(|(_, entity)| entity.attributes.contains_key(attribute))
                                .map_or_else(String::new, |(name, _)| name.clone()),
                            entity_id: id.clone(),
                            attribute: attribute.clone(),
                            valid_at: query.valid_at,
                            known_revision: known,
                        })
                        .await?;
                    answer.value.map(|v| match v {
                        Value::String(text) => text,
                        other => other.to_string(),
                    })
                }
            };
            rows.push(QueryRow {
                id: id.clone(),
                value,
            });
        }
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.truncate(query.limits.max_rows);

        Ok(QueryResult {
            rows,
            truncated: false,
            known_revision: known,
        })
    }
}
