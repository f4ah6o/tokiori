//! The org chart: who is in which department, as of a date and revision.
//!
//! This is a convenience over the query API rather than a second engine: it
//! builds one query per employee and lets the compiled path answer it, so the
//! chart cannot disagree with a hand-written query about the same instant.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::PgError;
use crate::store::PgStore;
use tokiori_model::{EntityRef, Hop, Limits, Query, Select};

/// A request for the chart.
#[derive(Debug, Clone)]
pub struct OrgChart {
    pub scope: String,
    pub valid_at: NaiveDate,
    pub known_revision: Option<u64>,
}

/// One employee and the department they were in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRow {
    pub employee: String,
    pub department: String,
}

impl PgStore {
    /// Builds the chart for a scope at a date and revision.
    pub async fn org_chart(&self, request: OrgChart) -> Result<Vec<OrgRow>, PgError> {
        let employees: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM entities WHERE scope_id = $1 AND entity_type = 'employee' ORDER BY id",
        )
        .bind(&request.scope)
        .fetch_all(self.pool())
        .await?;

        let mut rows = Vec::with_capacity(employees.len());
        for employee in employees {
            let query = Query {
                start: EntityRef {
                    entity_type: "employee".to_owned(),
                    id: employee.clone(),
                },
                traversal: vec![Hop {
                    relation_type: "membership".to_owned(),
                    from_role: "employee".to_owned(),
                    to_role: "department".to_owned(),
                }],
                select: Select::Ids,
                valid_at: request.valid_at,
                known_revision: request.known_revision,
                limits: Limits::default(),
            };
            for row in self.query(&request.scope, &query).await?.rows {
                rows.push(OrgRow {
                    employee: employee.clone(),
                    department: row.id,
                });
            }
        }
        Ok(rows)
    }
}
