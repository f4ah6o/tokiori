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

#[test]
fn the_organization_scenario_round_trips_through_separate_processes() {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no CLI evidence");
        return;
    };
    assert_binary_supports_commands();
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

#[test]
fn a_correction_is_visible_through_the_cli_at_both_revisions() {
    let Ok(url) = std::env::var("TOKIORI_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: TOKIORI_TEST_DATABASE_URL is not set; no CLI evidence");
        return;
    };
    let scope = unique_scope();
    let model = example("model.json");
    let model = model.to_str().expect("utf-8 path");

    json(&tokiori(&url, &scope, &["model", "apply", "--file", model]));
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
    json(&tokiori(
        &url,
        &scope,
        &[
            "entity",
            "create",
            "--type",
            "department",
            "--id",
            "D-PLANNING",
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
            "--valid-from",
            "2027-04-01",
        ],
    ));

    // r1: baseline, r2: move on 7/1, r3/r4: correct the move to 7/15.
    json(&tokiori(
        &url,
        &scope,
        &[
            "role",
            "set",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "department",
            "--participant",
            "D-SALES",
            "--valid-from",
            "2027-04-01",
        ],
    ));
    let moved = json(&tokiori(
        &url,
        &scope,
        &[
            "role",
            "set",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "department",
            "--participant",
            "D-PLANNING",
            "--valid-from",
            "2027-07-01",
        ],
    ));
    assert_eq!(moved["revision"], 2);
    json(&tokiori(
        &url,
        &scope,
        &[
            "role",
            "set",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "department",
            "--participant",
            "D-SALES",
            "--valid-from",
            "2027-07-01",
            "--valid-until",
            "2027-07-15",
        ],
    ));
    let corrected = json(&tokiori(
        &url,
        &scope,
        &[
            "role",
            "set",
            "--type",
            "membership",
            "--id",
            "M001",
            "--role",
            "department",
            "--participant",
            "D-PLANNING",
            "--valid-from",
            "2027-07-15",
        ],
    ));
    assert_eq!(corrected["revision"], 4);

    let before = json(&tokiori(
        &url,
        &scope,
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
            "2027-07-10",
            "--known-revision",
            "2",
        ],
    ));
    let after = json(&tokiori(
        &url,
        &scope,
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
            "2027-07-10",
            "--known-revision",
            "4",
        ],
    ));

    assert_eq!(
        before["participant"], "D-PLANNING",
        "r2 believed the move was on 7/1"
    );
    assert_eq!(after["participant"], "D-SALES", "r4 knows it was on 7/15");
}
