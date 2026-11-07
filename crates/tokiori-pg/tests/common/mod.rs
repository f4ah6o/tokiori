//! Shared fixtures for the PostgreSQL acceptance tests.
//!
//! Each test gets its own schema so parallel tests never observe each other's
//! rows, and the connection string comes from `TOKIORI_TEST_DATABASE_URL`.
//! When the variable is unset the tests are skipped loudly instead of passing
//! silently: a skipped acceptance test is not evidence.
//!
//! Each integration test compiles this module separately, so a helper used by
//! one test file is dead code in the others.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use tokiori_model::{Model, parse_model_str};
use tokiori_pg::{PgStore, migrate};

static NEXT_SCHEMA: AtomicU64 = AtomicU64::new(0);

pub const ORGANIZATION_MODEL: &str = r#"
{"model":"organization","version":1,"time_zone":"Asia/Tokyo",
 "entities":[
   {"name":"employee","attributes":{"name":"text"}},
   {"name":"department","attributes":{"name":"text"}}
 ],
 "relations":[{"name":"membership","attributes":{"title":"text"},"roles":{
   "employee":{"entity":"employee","cardinality":"one"},
   "department":{"entity":"department","cardinality":"many"}}}]}
"#;

pub fn organization_model() -> Model {
    parse_model_str(ORGANIZATION_MODEL).expect("fixture model is valid")
}

/// A store bound to a fresh, empty schema.
///
/// Call [`Fixture::cleanup`] at the end of every test. `Drop` cannot do it:
/// dropping a schema is async, and a panic inside the test must not also panic
/// the cleanup.
pub struct Fixture {
    pub store: PgStore,
    pub scope: String,
    schema: String,
    admin: PgPool,
}

/// Returns `None` when `TOKIORI_TEST_DATABASE_URL` is not set, printing why.
pub async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no PostgreSQL evidence");
        return None;
    };

    let id = NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed);
    let scope = format!("scope-{}-{id}", std::process::id());
    let schema = format!("tokiori_test_{}_{id}", std::process::id());
    // The only interpolated value is a process id and a counter; assert the
    // shape before handing it to the database as an identifier.
    assert!(
        schema
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated schema name must be a plain identifier: {schema}"
    );

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to test database");
    sqlx::query(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE"
    )))
    .execute(&admin)
    .await
    .expect("drop stale schema");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create schema");
    let separator = if url.contains('?') { '&' } else { '?' };
    let scoped_url = format!("{url}{separator}options=-c%20search_path%3D{schema}");
    let pool: PgPool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&scoped_url)
        .await
        .expect("connect to test schema");
    migrate(&pool).await.expect("apply migrations");

    Some(Fixture {
        store: PgStore::new(pool),
        scope,
        schema,
        admin,
    })
}

impl Fixture {
    /// Drops the test schema and closes both pools.
    ///
    /// Without this every run leaves a schema behind, and a shared database
    /// accumulates them until it is unusable.
    pub async fn cleanup(self) {
        self.store.pool().close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await
        .expect("drop test schema");
        self.admin.close().await;
    }
}

pub fn attributes(pairs: &[(&str, &str)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), serde_json::Value::String((*v).to_owned())))
        .collect()
}
