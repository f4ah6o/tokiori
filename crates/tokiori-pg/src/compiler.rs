//! Compiling a validated query into parameterized SQL.
//!
//! The query has already been checked against the model, so every identifier
//! used here came from the model rather than from the caller. Values -- ids,
//! dates, revisions, limits -- are always bound as parameters, which is what
//! keeps user text out of the SQL structure.

use std::fmt::Write as _;

use tokiori_model::{Query, Select};

use crate::error::PgError;

/// A compiled query: SQL with numbered placeholders and the values to bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledQuery {
    pub sql: String,
    pub params: Vec<String>,
}

/// Compiles a query that has already passed [`tokiori_model::validate_query`].
///
/// The caller must validate first: this function trusts the names it is given
/// to exist in the model, and interpolates them as identifiers.
pub fn compile_query(query: &Query) -> Result<CompiledQuery, PgError> {
    let mut params = Vec::new();
    let mut sql = String::new();

    // The first hop starts from the entity the caller named.
    let mut next = 1usize;
    let mut bind = |params: &mut Vec<String>, value: String| {
        params.push(value);
        let placeholder = format!("${next}");
        next += 1;
        placeholder
    };

    let scope = bind(&mut params, "{scope}".to_owned());
    let start_id = bind(&mut params, query.start.id.clone());
    let valid_at = bind(&mut params, query.valid_at.to_string());
    let revision = bind(
        &mut params,
        query
            .known_revision
            .map_or_else(|| "\u{0}max".to_owned(), |r| r.to_string()),
    );
    let limit = bind(&mut params, query.limits.max_rows.to_string());

    // Each hop joins the relation's role versions for the current position and
    // the next one, both visible at the same revision and covering the date.
    let from_type = query.start.entity_type.clone();
    let joins = build_joins(query, &scope, &start_id, &valid_at, &revision);
    let last_alias = format!("r{}", query.traversal.len());
    let (projection, from_clause) = match &query.select {
        Select::Ids => {
            let column = if query.traversal.is_empty() {
                start_id.clone()
            } else {
                format!("{last_alias}.participant_id")
            };
            (format!("{column} AS id"), String::new())
        }
        Select::Attribute { attribute } => {
            let owner = if query.traversal.is_empty() {
                start_id.clone()
            } else {
                format!("{last_alias}.participant_id")
            };
            // The attribute's value at the same instant and revision, chosen by
            // the same rule the read path uses: newest knowledge wins.
            let from_clause = format!(
                " LEFT JOIN LATERAL ( \
                    SELECT f.value FROM fact_versions f \
                    WHERE f.scope_id = {scope} \
                      AND f.entity_type = '{from_type}' \
                      AND f.entity_id = {owner} \
                      AND f.attribute = '{attribute}' \
                      AND f.valid_from <= {valid_at}::date \
                      AND (f.valid_until IS NULL OR f.valid_until > {valid_at}::date) \
                      AND f.recorded_from <= {revision} \
                      AND (f.recorded_to IS NULL OR f.recorded_to >= {revision}) \
                    ORDER BY f.recorded_from DESC LIMIT 1 ) v ON true",
            );
            (format!("{owner} AS id, v.value AS value"), from_clause)
        }
    };

    let _ = write!(
        sql,
        "SELECT DISTINCT {projection} FROM relation_role_versions r0{joins}{from_clause} \
         WHERE r0.scope_id = {scope} AND r0.participant_id = {start_id} \
         LIMIT {limit}"
    );

    // The zero-depth case has no relation to anchor on; it selects the start.
    if query.traversal.is_empty() {
        sql = match &query.select {
            Select::Ids => format!(
                "SELECT {start_id} AS id FROM entities \
                 WHERE scope_id = {scope} AND entity_type = '{from_type}' AND id = {start_id} \
                 LIMIT {limit}"
            ),
            Select::Attribute { attribute } => format!(
                "SELECT e.id AS id, v.value AS value FROM entities e \
                 LEFT JOIN LATERAL ( \
                    SELECT f.value FROM fact_versions f \
                    WHERE f.scope_id = {scope} AND f.entity_type = '{from_type}' \
                      AND f.entity_id = e.id AND f.attribute = '{attribute}' \
                      AND f.valid_from <= {valid_at}::date \
                      AND (f.valid_until IS NULL OR f.valid_until > {valid_at}::date) \
                      AND f.recorded_from <= {revision} \
                      AND (f.recorded_to IS NULL OR f.recorded_to >= {revision}) \
                    ORDER BY f.recorded_from DESC LIMIT 1 ) v ON true \
                 WHERE e.scope_id = {scope} AND e.entity_type = '{from_type}' AND e.id = {start_id} \
                 LIMIT {limit}"
            ),
        };
    }

    // The scope parameter is a placeholder in the compiled SQL; the caller
    // supplies the real value when binding.
    Ok(CompiledQuery {
        sql: sql.replace(&scope, "$1"),
        params,
    })
}

/// Builds the join chain for a traversal.
///
/// Each hop joins two role versions of the same relation: the one the current
/// position fills and the one it moves to. Both must be visible at the same
/// revision and cover the same date, which is what keeps a multi-hop query
/// consistent with a single `known_revision`.
fn build_joins(
    query: &Query,
    scope: &str,
    start_id: &str,
    valid_at: &str,
    revision: &str,
) -> String {
    let mut joins = String::new();
    let mut from_id = start_id.to_owned();
    for (index, hop) in query.traversal.iter().enumerate() {
        let alias = format!("r{index}");
        let next_alias = format!("r{}", index + 1);
        let relation = &hop.relation_type;
        let from_role = &hop.from_role;
        let to_role = &hop.to_role;
        let visible = |a: &str| {
            format!(
                "{a}.valid_from <= {valid_at}::date                  AND ({a}.valid_until IS NULL OR {a}.valid_until > {valid_at}::date)                  AND {a}.recorded_from <= {revision}                  AND ({a}.recorded_to IS NULL OR {a}.recorded_to >= {revision})"
            )
        };
        let from_visible = visible(&alias);
        let to_visible = visible(&next_alias);
        let _ = write!(
            joins,
            " JOIN relation_role_versions {alias}              ON {alias}.scope_id = {scope}             AND {alias}.relation_type = '{relation}'             AND {alias}.role = '{from_role}'             AND {alias}.participant_id = {from_id}             AND {from_visible}              JOIN relation_role_versions {next_alias}              ON {next_alias}.scope_id = {alias}.scope_id             AND {next_alias}.relation_type = {alias}.relation_type             AND {next_alias}.relation_id = {alias}.relation_id             AND {next_alias}.role = '{to_role}'             AND {to_visible}"
        );
        from_id = format!("{next_alias}.participant_id");
    }
    joins
}
