//! The operator workflow: apply a model, create rows, and read them back from
//! separate processes.
//!
//! Each command runs as its own `tokiori` process, so a value that survives is
//! one that really reached PostgreSQL rather than one held in process memory.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tokiori"))
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/organization")
        .join(name)
}

struct Run {
    success: bool,
    stdout: String,
    stderr: String,
}

fn tokiori(url: &str, scope: &str, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .arg("--database-url")
        .arg(url)
        .arg("--scope")
        .arg(scope)
        .arg("--format")
        .arg("json")
        .args(args)
        .output()
        .expect("run tokiori");
    Run {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Fails loudly when the binary under test is older than the source.
///
/// `CARGO_BIN_EXE_tokiori` points at the binary built for this test run; if it
/// lacks a command the test drives, the failure must say so instead of
/// surfacing later as a confusing assertion about a database row.
fn assert_binary_supports_commands() {
    let run = tokiori(
        "postgres://unused/unused",
        "scope",
        &["model", "validate", "--help"],
    );
    assert!(
        run.success,
        "the tokiori binary does not support `model validate`: {}",
        run.stderr
    );
}

fn json(run: &Run) -> Value {
    assert!(run.success, "command failed: {}", run.stderr);
    serde_json::from_str(&run.stdout).expect("json output")
}

fn unique_scope() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("cli-{}-{nanos}", std::process::id())
}

/// Creates a schema of this run's own and returns a URL that uses it.
///
/// The CLI tests used the shared `public` schema, so a run that overlapped
/// another -- or followed a run that panicked before cleaning up -- saw rows
/// it did not create. Each test now gets an empty schema of its own.
fn fresh_schema(url: &str) -> (String, String) {
    // A counter, not a clock: two tests starting in the same nanosecond would
    // otherwise ask for the same schema name and one would lose the race.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let schema = format!("tokiori_cli_{}_{id}", std::process::id());
    assert!(
        schema
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "generated schema name must be a plain identifier: {schema}"
    );

    // The CLI has no command for DDL, so the schema is created here over the
    // same connection string the tests then use.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let pool = sqlx::PgPool::connect(url)
            .await
            .expect("connect to create schema");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE"
        )))
        .execute(&pool)
        .await
        .expect("drop stale schema");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&pool)
            .await
            .expect("create schema");
        pool.close().await;
    });

    let separator = if url.contains('?') { '&' } else { '?' };
    let scoped = format!("{url}{separator}options=-c%20search_path%3D{schema}");
    (scoped, schema)
}

#[test]
fn the_organization_scenario_round_trips_through_separate_processes() {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no CLI evidence");
        return;
    };
    assert_binary_supports_commands();
    let (url, _schema) = fresh_schema(&url);
    let scope = unique_scope();
    let model_json = example("model.json");
    let model_json = model_json.to_str().expect("utf-8 path");
    let model_yaml = example("model.yaml");
    let model_yaml = model_yaml.to_str().expect("utf-8 path");

    let validated = json(&tokiori(
        &url,
        &scope,
        &["model", "validate", "--file", model_json],
    ));
    assert_eq!(validated["model"], "organization");

    let applied = json(&tokiori(
        &url,
        &scope,
        &["model", "apply", "--file", model_json],
    ));
    assert_eq!(applied["outcome"], "created");
    let reapplied = json(&tokiori(
        &url,
        &scope,
        &["model", "apply", "--file", model_yaml],
    ));
    assert_eq!(reapplied["outcome"], "unchanged");
    assert_eq!(reapplied["digest"], applied["digest"]);

    json(&tokiori(
        &url,
        &scope,
        &[
            "entity",
            "create",
            "--type",
            "employee",
            "--id",
            "E001",
            "--attributes",
            r#"{"name":"Alice","joined_on":"2027-04-01"}"#,
        ],
    ));
    json(&tokiori(
        &url,
        &scope,
        &[
            "entity",
            "create",
            "--type",
            "department",
            "--id",
            "D-SALES",
            "--attributes",
            r#"{"name":"Sales"}"#,
        ],
    ));
    json(&tokiori(
        &url,
        &scope,
        &[
            "relation",
            "create",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "employee=E001",
            "--role",
            "department=D-SALES",
            "--attributes",
            r#"{"title":"Account manager"}"#,
            "--valid-from",
            "2027-04-01",
        ],
    ));

    let employee = json(&tokiori(
        &url,
        &scope,
        &["entity", "get", "--type", "employee", "--id", "E001"],
    ));
    assert_eq!(employee["attributes"]["name"], "Alice");
    assert_eq!(employee["scope"], scope);

    let membership = json(&tokiori(
        &url,
        &scope,
        &["relation", "get", "--type", "membership", "--id", "M001"],
    ));
    assert_eq!(membership["roles"]["employee"], "E001");
    assert_eq!(membership["roles"]["department"], "D-SALES");
    assert_eq!(membership["valid_from"], "2027-04-01");
    assert!(membership["valid_until"].is_null());
}

#[test]
fn a_rejected_relation_reports_a_stable_code_and_exits_non_zero() {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no CLI evidence");
        return;
    };
    let (url, _schema) = fresh_schema(&url);
    let scope = unique_scope();
    let model_json = example("model.json");
    let model_json = model_json.to_str().expect("utf-8 path");

    json(&tokiori(
        &url,
        &scope,
        &["model", "apply", "--file", model_json],
    ));
    json(&tokiori(
        &url,
        &scope,
        &["entity", "create", "--type", "employee", "--id", "E001"],
    ));
    json(&tokiori(
        &url,
        &scope,
        &[
            "entity",
            "create",
            "--type",
            "department",
            "--id",
            "D-SALES",
        ],
    ));

    let run = tokiori(
        &url,
        &scope,
        &[
            "relation",
            "create",
            "--type",
            "membership",
            "--id",
            "M-BAD",
            "--role",
            "employee=D-SALES",
            "--role",
            "department=D-SALES",
            "--valid-from",
            "2027-04-01",
        ],
    );
    assert!(!run.success, "a department must not fill the employee role");
    let error: Value = serde_json::from_str(run.stderr.trim()).expect("json error on stderr");
    assert_eq!(error["code"], "entity_type_mismatch");
}

#[test]
fn an_invalid_model_file_is_rejected_before_any_database_access() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("broken.json");
    std::fs::write(
        &path,
        r#"{"model":"organization","version":1,"time_zone":"Asia/Tokyo","entities":[],"relations":[{"name":"membership","roles":{"employee":{"entity":"person","cardinality":"one"}}}]}"#,
    )
    .expect("write fixture");

    let run = tokiori(
        "postgres://unused/unused",
        "scope",
        &[
            "model",
            "validate",
            "--file",
            path.to_str().expect("utf-8 path"),
        ],
    );
    assert!(!run.success);
    let error: Value = serde_json::from_str(run.stderr.trim()).expect("json error on stderr");
    assert_eq!(error["code"], "invalid_model");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("person")
    );
}

/// Seeds the organization and records the ORG-004 history through the CLI.
///
/// Returns the scope so callers can query it.
fn seed_correction_history(url: &str) -> (String, u64, u64) {
    let scope = unique_scope();
    let model = example("model.json");
    let model = model.to_str().expect("utf-8 path");

    json(&tokiori(url, &scope, &["model", "apply", "--file", model]));
    for (entity_type, id) in [
        ("employee", "E001"),
        ("department", "D-SALES"),
        ("department", "D-PLANNING"),
    ] {
        json(&tokiori(
            url,
            &scope,
            &["entity", "create", "--type", entity_type, "--id", id],
        ));
    }
    json(&tokiori(
        url,
        &scope,
        &[
            "relation",
            "create",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "employee=E001",
            "--role",
            "department=D-SALES",
            "--valid-from",
            "2027-04-01",
        ],
    ));

    // Baseline, then the move on 7/1, then the correction to 7/15.
    set_department(url, &scope, "D-SALES", "2027-04-01", None);
    let moved = set_department(url, &scope, "D-PLANNING", "2027-07-01", None);
    set_department(url, &scope, "D-SALES", "2027-07-01", Some("2027-07-15"));
    let corrected = set_department(url, &scope, "D-PLANNING", "2027-07-15", None);
    (scope, moved, corrected)
}

fn set_department(
    url: &str,
    scope: &str,
    participant: &str,
    from: &str,
    until: Option<&str>,
) -> u64 {
    let mut args = vec![
        "role",
        "set",
        "--type",
        "membership",
        "--id",
        "M001",
        "--role",
        "department",
        "--participant",
        participant,
        "--valid-from",
        from,
    ];
    if let Some(until) = until {
        args.push("--valid-until");
        args.push(until);
    }
    let outcome = json(&tokiori(url, scope, &args));
    outcome["revision"].as_u64().expect("a revision")
}

fn department_at(url: &str, scope: &str, valid_at: &str, revision: &str) -> Value {
    json(&tokiori(
        url,
        scope,
        &[
            "role",
            "get",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "department",
            "--valid-at",
            valid_at,
            "--known-revision",
            revision,
        ],
    ))
}

#[test]
fn a_correction_is_visible_through_the_cli_at_both_revisions() {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no CLI evidence");
        return;
    };
    let (url, _schema) = fresh_schema(&url);
    let (scope, moved, corrected) = seed_correction_history(&url);

    // The revision numbers are read from the commands rather than assumed:
    // creating a relation now takes a revision of its own, and hard-coding
    // them made this test depend on how many writes happened to come before.
    let before = department_at(&url, &scope, "2027-07-10", &moved.to_string());
    let after = department_at(&url, &scope, "2027-07-10", &corrected.to_string());

    assert_eq!(
        before["participant"], "D-PLANNING",
        "the move's revision believed it took effect on 7/1"
    );
    assert_eq!(
        after["participant"], "D-SALES",
        "the correction knows it took effect on 7/15"
    );
}
