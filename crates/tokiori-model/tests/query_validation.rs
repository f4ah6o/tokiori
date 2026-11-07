//! A query is checked against the model before anything reaches SQL.

use chrono::NaiveDate;
use tokiori_model::{Hop, Limits, Query, parse_model_str, validate_query};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

const MODEL: &str = r#"
{"model":"organization","version":1,"time_zone":"Asia/Tokyo",
 "entities":[
   {"name":"employee","attributes":{"name":"text"}},
   {"name":"department","attributes":{"name":"text"}}
 ],
 "relations":[{"name":"membership","attributes":{"title":"text"},"roles":{
   "employee":{"entity":"employee","cardinality":"one"},
   "department":{"entity":"department","cardinality":"many"}}}]}
"#;

fn model() -> tokiori_model::Model {
    parse_model_str(MODEL).expect("valid model")
}

fn hop(from: &str, to: &str) -> Hop {
    Hop {
        relation_type: "membership".to_owned(),
        from_role: from.to_owned(),
        to_role: to.to_owned(),
    }
}

fn query(start_type: &str, hops: Vec<Hop>) -> Query {
    Query {
        start: tokiori_model::EntityRef {
            entity_type: start_type.to_owned(),
            id: "E001".to_owned(),
        },
        traversal: hops,
        select: tokiori_model::Select::Ids,
        valid_at: day(2027, 7, 10),
        known_revision: None,
        limits: Limits::default(),
    }
}

#[test]
fn a_query_over_declared_roles_is_valid() {
    let q = query("employee", vec![hop("employee", "department")]);
    assert!(validate_query(&model(), &q).is_ok());
}

#[test]
fn an_undeclared_relation_type_is_rejected() {
    let mut q = query("employee", vec![hop("employee", "department")]);
    q.traversal[0].relation_type = "supervision".to_owned();

    let err = validate_query(&model(), &q).expect_err("undeclared relation");
    assert_eq!(err.code(), "unknown_relation", "{err}");
}

#[test]
fn an_undeclared_role_is_rejected() {
    let mut q = query("employee", vec![hop("employee", "manager")]);
    q.traversal[0].to_role = "manager".to_owned();

    let err = validate_query(&model(), &q).expect_err("undeclared role");
    assert_eq!(err.code(), "unknown_role", "{err}");
}

#[test]
fn a_hop_that_does_not_continue_from_the_current_type_is_rejected() {
    // Starting at an employee, the first hop must consume the employee role.
    let q = query("employee", vec![hop("department", "employee")]);

    let err = validate_query(&model(), &q).expect_err("role does not match the position");
    assert_eq!(err.code(), "role_type_mismatch", "{err}");
}

#[test]
fn an_undeclared_start_entity_type_is_rejected() {
    let q = query("contractor", vec![]);

    let err = validate_query(&model(), &q).expect_err("undeclared entity type");
    assert_eq!(err.code(), "unknown_entity_type", "{err}");
}

#[test]
fn a_depth_beyond_the_maximum_is_rejected() {
    let mut q = query("employee", vec![]);
    q.limits = Limits {
        max_depth: Limits::MAX_DEPTH + 1,
        max_rows: 10,
    };

    let err = validate_query(&model(), &q).expect_err("depth over the maximum");
    assert_eq!(err.code(), "limit_too_large", "{err}");
}

#[test]
fn a_traversal_longer_than_the_depth_limit_is_rejected() {
    let mut q = query("employee", vec![hop("employee", "department")]);
    q.limits = Limits {
        max_depth: 0,
        max_rows: 10,
    };

    let err = validate_query(&model(), &q).expect_err("traversal exceeds depth");
    assert_eq!(err.code(), "limit_exceeded", "{err}");
}

#[test]
fn a_row_limit_beyond_the_maximum_is_rejected() {
    let mut q = query("employee", vec![]);
    q.limits = Limits {
        max_depth: 1,
        max_rows: Limits::MAX_ROWS + 1,
    };

    let err = validate_query(&model(), &q).expect_err("rows over the maximum");
    assert_eq!(err.code(), "limit_too_large", "{err}");
}

#[test]
fn a_zero_depth_query_returns_the_start_only() {
    let q = query("employee", vec![]);
    assert!(validate_query(&model(), &q).is_ok());
}

#[test]
fn selecting_an_undeclared_attribute_is_rejected() {
    let mut q = query("employee", vec![hop("employee", "department")]);
    q.select = tokiori_model::Select::Attribute {
        attribute: "salary".to_owned(),
    };

    let err = validate_query(&model(), &q).expect_err("undeclared attribute");
    assert_eq!(err.code(), "unknown_attribute", "{err}");
}

#[test]
fn selecting_a_declared_attribute_is_valid() {
    let mut q = query("employee", vec![hop("employee", "department")]);
    q.select = tokiori_model::Select::Attribute {
        attribute: "name".to_owned(),
    };

    assert!(validate_query(&model(), &q).is_ok());
}

#[test]
fn the_default_limits_are_within_the_maximum() {
    let limits = Limits::default();
    assert!(limits.max_depth <= Limits::MAX_DEPTH);
    assert!(limits.max_rows <= Limits::MAX_ROWS);
}
