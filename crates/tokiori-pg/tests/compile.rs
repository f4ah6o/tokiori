//! The compiler turns a validated query into parameterized SQL.

use chrono::NaiveDate;
use tokiori_model::{EntityRef, Hop, Limits, Query, Select, parse_model_str, validate_query};
use tokiori_pg::compile_query;

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

fn query(id: &str) -> Query {
    Query {
        start: EntityRef {
            entity_type: "employee".to_owned(),
            id: id.to_owned(),
        },
        traversal: vec![Hop {
            relation_type: "membership".to_owned(),
            from_role: "employee".to_owned(),
            to_role: "department".to_owned(),
        }],
        select: Select::Ids,
        valid_at: day(2027, 7, 10),
        known_revision: None,
        limits: Limits::default(),
    }
}

#[test]
fn the_generated_sql_binds_every_value() {
    let q = query("E001");
    validate_query(&model(), &q).expect("valid query");
    let compiled = compile_query(&q).expect("compiles");

    assert!(
        compiled.sql.contains("$1"),
        "{}: {}",
        compiled.sql,
        "placeholder"
    );
    assert!(!compiled.sql.contains("E001"), "the id must not be inlined");
    assert!(
        !compiled.sql.contains("2027"),
        "the date must not be inlined"
    );
    assert_eq!(compiled.params.len(), 5, "scope, id, date, revision, limit");
}

#[test]
fn a_hostile_id_stays_a_value() {
    let hostile = "E001'; DROP TABLE entities; --";
    let q = query(hostile);
    validate_query(&model(), &q).expect("valid query");
    let compiled = compile_query(&q).expect("compiles");

    assert!(
        !compiled.sql.contains("DROP"),
        "user text must never reach the SQL structure: {}",
        compiled.sql
    );
    assert!(
        compiled.params.iter().any(|p| p.contains("DROP")),
        "the hostile text is carried as a parameter"
    );
}

#[test]
fn identifiers_come_from_the_model_only() {
    let q = query("E001");
    let compiled = compile_query(&q).expect("compiles");

    // The relation and role names appear because they were validated against
    // the model; nothing else from the query is interpolated.
    assert!(compiled.sql.contains("membership"));
    assert!(compiled.sql.contains("department"));
}

#[test]
fn a_multi_hop_query_compiles_to_a_join_chain() {
    let mut q = query("E001");
    q.traversal.push(Hop {
        relation_type: "membership".to_owned(),
        from_role: "department".to_owned(),
        to_role: "employee".to_owned(),
    });
    validate_query(&model(), &q).expect("valid query");
    let compiled = compile_query(&q).expect("compiles");

    let joins = compiled.sql.matches("relation_role_versions").count();
    assert!(joins >= 2, "one join per hop: {}", compiled.sql);
}

#[test]
fn selecting_an_attribute_changes_the_projection_not_the_shape() {
    let mut q = query("E001");
    q.select = Select::Attribute {
        attribute: "name".to_owned(),
    };
    validate_query(&model(), &q).expect("valid query");
    let compiled = compile_query(&q).expect("compiles");

    assert!(compiled.sql.contains("fact_versions"), "{}", compiled.sql);
    assert!(!compiled.sql.contains("E001"));
}

#[test]
fn the_row_limit_is_bound_not_interpolated() {
    let q = query("E001");
    let compiled = compile_query(&q).expect("compiles");

    assert!(compiled.sql.contains("LIMIT"), "{}", compiled.sql);
    assert!(!compiled.sql.contains("1000"), "the limit is a parameter");
}
