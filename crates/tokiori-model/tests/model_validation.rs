//! What a model document must be rejected for, and what a valid one must preserve.

use std::collections::BTreeMap;

use tokiori_model::{
    AttributeType, Cardinality, EntityDef, ModelDocument, ModelErrorCode, ParseError, RelationDef,
    RoleDef, canonical_json, model_digest, normalize, parse_model_str,
};

fn role(entity: &str, cardinality: Cardinality) -> RoleDef {
    RoleDef {
        entity: entity.to_owned(),
        cardinality,
    }
}

fn organization_document() -> ModelDocument {
    ModelDocument {
        model: "organization".to_owned(),
        version: 1,
        time_zone: "Asia/Tokyo".to_owned(),
        entities: vec![
            EntityDef {
                name: "employee".to_owned(),
                attributes: BTreeMap::from([("name".to_owned(), AttributeType::Text)]),
            },
            EntityDef {
                name: "department".to_owned(),
                attributes: BTreeMap::from([("name".to_owned(), AttributeType::Text)]),
            },
        ],
        relations: vec![RelationDef {
            name: "membership".to_owned(),
            roles: BTreeMap::from([
                ("employee".to_owned(), role("employee", Cardinality::One)),
                (
                    "department".to_owned(),
                    role("department", Cardinality::Many),
                ),
            ]),
            attributes: BTreeMap::new(),
        }],
    }
}

fn codes(doc: ModelDocument) -> Vec<ModelErrorCode> {
    match normalize(doc) {
        Ok(_) => Vec::new(),
        Err(invalid) => invalid
            .errors
            .iter()
            .map(tokiori_model::ModelError::code)
            .collect(),
    }
}

#[test]
fn a_role_referencing_an_undefined_entity_is_rejected_before_any_write() {
    let mut doc = organization_document();
    doc.relations[0]
        .roles
        .insert("manager".to_owned(), role("person", Cardinality::One));

    let invalid = normalize(doc).expect_err("undefined entity must be rejected");
    assert_eq!(invalid.errors.len(), 1, "{invalid:?}");
    assert_eq!(invalid.errors[0].code(), ModelErrorCode::UnknownEntity);
    assert!(invalid.errors[0].to_string().contains("person"));
}

#[test]
fn duplicate_entity_declarations_are_rejected() {
    let mut doc = organization_document();
    doc.entities.push(EntityDef {
        name: "employee".to_owned(),
        attributes: BTreeMap::new(),
    });

    assert_eq!(codes(doc), vec![ModelErrorCode::DuplicateEntity]);
}

#[test]
fn duplicate_relation_declarations_are_rejected() {
    let mut doc = organization_document();
    doc.relations.push(doc.relations[0].clone());

    assert_eq!(codes(doc), vec![ModelErrorCode::DuplicateRelation]);
}

#[test]
fn a_relation_without_roles_is_rejected() {
    let mut doc = organization_document();
    doc.relations[0].roles.clear();

    assert_eq!(codes(doc), vec![ModelErrorCode::EmptyRoles]);
}

#[test]
fn an_unknown_iana_time_zone_is_rejected() {
    let mut doc = organization_document();
    doc.time_zone = "Mars/Olympus_Mons".to_owned();

    assert_eq!(codes(doc), vec![ModelErrorCode::InvalidTimeZone]);
}

#[test]
fn version_zero_is_rejected() {
    let mut doc = organization_document();
    doc.version = 0;

    assert_eq!(codes(doc), vec![ModelErrorCode::InvalidVersion]);
}

#[test]
fn names_outside_the_identifier_grammar_are_rejected() {
    let mut doc = organization_document();
    doc.entities[0].name = "Employee".to_owned();
    doc.relations[0].roles.get_mut("employee").unwrap().entity = "Employee".to_owned();

    assert_eq!(codes(doc), vec![ModelErrorCode::InvalidEntityName]);
}

#[test]
fn engine_owned_attribute_names_are_reserved() {
    let mut doc = organization_document();
    doc.entities[0]
        .attributes
        .insert("id".to_owned(), AttributeType::Text);

    assert_eq!(codes(doc), vec![ModelErrorCode::ReservedAttribute]);
}

#[test]
fn every_problem_is_reported_in_one_pass() {
    let mut doc = organization_document();
    doc.version = 0;
    doc.time_zone = "Nowhere/Special".to_owned();

    let codes = codes(doc);
    assert_eq!(codes.len(), 2, "{codes:?}");
    assert!(codes.contains(&ModelErrorCode::InvalidVersion));
    assert!(codes.contains(&ModelErrorCode::InvalidTimeZone));
}

#[test]
fn normalization_does_not_depend_on_declaration_order() {
    let doc = organization_document();
    let mut reversed = doc.clone();
    reversed.entities.reverse();
    reversed.relations[0].roles = BTreeMap::from([
        (
            "department".to_owned(),
            role("department", Cardinality::Many),
        ),
        ("employee".to_owned(), role("employee", Cardinality::One)),
    ]);

    let a = normalize(doc).expect("valid model");
    let b = normalize(reversed).expect("valid model");
    assert_eq!(canonical_json(&a), canonical_json(&b));
    assert_eq!(model_digest(&a), model_digest(&b));
}

#[test]
fn the_digest_changes_when_the_model_changes() {
    let base = normalize(organization_document()).expect("valid model");
    let mut changed_doc = organization_document();
    changed_doc.version = 2;
    let changed = normalize(changed_doc).expect("valid model");

    assert!(model_digest(&base).starts_with("sha256:"));
    assert_ne!(model_digest(&base), model_digest(&changed));
}

#[test]
fn an_unknown_field_is_rejected_rather_than_ignored() {
    let input = r#"{"model":"organization","version":1,"time_zone":"Asia/Tokyo","entities":[],"relations":[],"owners":[]}"#;
    assert!(matches!(
        parse_model_str(input),
        Err(ParseError::Shape { .. })
    ));
}

#[test]
fn yaml_input_is_accepted() {
    let input = r"
model: organization
version: 1
time_zone: Asia/Tokyo
entities:
  - name: employee
  - name: department
relations:
  - name: membership
    roles:
      employee: {entity: employee, cardinality: one}
      department: {entity: department, cardinality: many}
";

    let model = parse_model_str(input).expect("valid yaml model");
    assert_eq!(model.time_zone.to_string(), "Asia/Tokyo");
    assert_eq!(
        model.relations["membership"].roles["department"].cardinality,
        Cardinality::Many
    );
}

#[test]
fn empty_input_is_rejected() {
    assert_eq!(parse_model_str("   \n"), Err(ParseError::Empty));
}

#[test]
fn a_semantically_invalid_document_fails_to_parse() {
    let input = r#"{"model":"organization","version":1,"time_zone":"Asia/Tokyo","entities":[],"relations":[{"name":"membership","roles":{"employee":{"entity":"employee","cardinality":"one"}}}]}"#;
    assert!(parse_model_str(input).is_err());
}
