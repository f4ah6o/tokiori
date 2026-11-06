//! ORG-001: only well-formed relations are stored, and rejected writes leave
//! nothing behind.

mod common;

use std::collections::BTreeMap;

use chrono::NaiveDate;
use common::{attributes, fixture, organization_model};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

async fn seed_people(store: &tokiori_pg::PgStore, scope: &str) -> (String, String) {
    store
        .apply_model(scope, &organization_model())
        .await
        .expect("apply model");
    store
        .create_entity(NewEntity {
            scope: scope.to_owned(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: attributes(&[("name", "Alice")]),
        })
        .await
        .expect("create employee");
    store
        .create_entity(NewEntity {
            scope: scope.to_owned(),
            entity_type: "department".to_owned(),
            id: "D-SALES".to_owned(),
            attributes: attributes(&[("name", "Sales")]),
        })
        .await
        .expect("create department");
    ("E001".to_owned(), "D-SALES".to_owned())
}

fn membership(scope: &str, id: &str, employee: &str, department: &str) -> NewRelation {
    NewRelation {
        scope: scope.to_owned(),
        relation_type: "membership".to_owned(),
        id: id.to_owned(),
        roles: BTreeMap::from([
            ("employee".to_owned(), employee.to_owned()),
            ("department".to_owned(), department.to_owned()),
        ]),
        attributes: attributes(&[("title", "Account manager")]),
        valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
    }
}

async fn relation_count(store: &tokiori_pg::PgStore) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM relations")
        .fetch_one(store.pool())
        .await
        .expect("count relations")
}

#[tokio::test]
async fn a_well_formed_membership_round_trips_through_postgres() {
    let Some(fx) = fixture().await else { return };
    let (employee, department) = seed_people(&fx.store, &fx.scope).await;

    let created = fx
        .store
        .create_relation(membership(&fx.scope, "M001", &employee, &department))
        .await
        .expect("create membership");
    assert_eq!(created.roles["employee"], "E001");
    assert_eq!(created.roles["department"], "D-SALES");

    let read = fx
        .store
        .get_relation(&fx.scope, "membership", "M001")
        .await
        .expect("read membership")
        .expect("membership exists");
    assert_eq!(read, created);

    let entity = fx
        .store
        .get_entity(&fx.scope, "employee", "E001")
        .await
        .expect("read employee")
        .expect("employee exists");
    assert_eq!(entity.attributes["name"], serde_json::json!("Alice"));
}

#[tokio::test]
async fn assigning_a_department_to_the_employee_role_is_rejected_without_a_partial_write() {
    let Some(fx) = fixture().await else { return };
    let (employee, department) = seed_people(&fx.store, &fx.scope).await;

    let mut bad = membership(&fx.scope, "M-BAD", &employee, &department);
    // The employee role is filled by a real department. The department role
    // keeps its own valid participant so the only defect is the role type.
    bad.roles.insert("employee".to_owned(), department.clone());

    let err = fx
        .store
        .create_relation(bad)
        .await
        .expect_err("a department cannot fill the employee role");
    assert_eq!(err.code(), "entity_type_mismatch", "{err}");
    assert!(err.to_string().contains("employee"), "{err}");
    assert_eq!(relation_count(&fx.store).await, 0);
    assert!(
        fx.store
            .get_relation(&fx.scope, "membership", "M-BAD")
            .await
            .expect("read")
            .is_none()
    );
}

#[tokio::test]
async fn referencing_an_entity_from_another_scope_is_rejected() {
    let Some(fx) = fixture().await else { return };
    let (employee, _department) = seed_people(&fx.store, &fx.scope).await;

    // The same model and ids exist, but only in a different scope.
    let other_scope = format!("{}-other", fx.scope);
    fx.store
        .apply_model(&other_scope, &organization_model())
        .await
        .expect("apply model in other scope");
    let foreign_department = "D-PLANNING".to_owned();
    fx.store
        .create_entity(NewEntity {
            scope: other_scope.clone(),
            entity_type: "department".to_owned(),
            id: foreign_department.clone(),
            attributes: attributes(&[("name", "Planning")]),
        })
        .await
        .expect("create department in other scope");
    assert!(
        fx.store
            .get_entity(&fx.scope, "department", &foreign_department)
            .await
            .expect("read")
            .is_none(),
        "the foreign department must not be visible in this scope"
    );

    let err = fx
        .store
        .create_relation(membership(
            &fx.scope,
            "M-CROSS",
            &employee,
            &foreign_department,
        ))
        .await
        .expect_err("a cross-scope participant must be rejected");
    assert_eq!(err.code(), "scope_mismatch", "{err}");
    assert_eq!(relation_count(&fx.store).await, 0);
}

#[tokio::test]
async fn a_relation_without_a_required_role_is_rejected() {
    let Some(fx) = fixture().await else { return };
    let (employee, department) = seed_people(&fx.store, &fx.scope).await;

    let mut incomplete = membership(&fx.scope, "M-PARTIAL", &employee, &department);
    incomplete.roles.remove("department");

    let err = fx
        .store
        .create_relation(incomplete)
        .await
        .expect_err("missing role must be rejected");
    assert_eq!(err.code(), "missing_role", "{err}");
    assert_eq!(relation_count(&fx.store).await, 0);
}

#[tokio::test]
async fn an_undeclared_role_name_is_rejected() {
    let Some(fx) = fixture().await else { return };
    let (employee, department) = seed_people(&fx.store, &fx.scope).await;

    let mut extra = membership(&fx.scope, "M-EXTRA", &employee, &department);
    extra.roles.insert("manager".to_owned(), employee.clone());

    let err = fx
        .store
        .create_relation(extra)
        .await
        .expect_err("undeclared role must be rejected");
    assert_eq!(err.code(), "unknown_role", "{err}");
    assert_eq!(relation_count(&fx.store).await, 0);
}

#[tokio::test]
async fn registering_the_same_entity_id_twice_leaves_the_original_intact() {
    let Some(fx) = fixture().await else { return };
    seed_people(&fx.store, &fx.scope).await;

    let err = fx
        .store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: attributes(&[("name", "Mallory")]),
        })
        .await
        .expect_err("duplicate id must be rejected");
    assert_eq!(err.code(), "duplicate_id", "{err}");

    let stored = fx
        .store
        .get_entity(&fx.scope, "employee", "E001")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(stored.attributes["name"], serde_json::json!("Alice"));
}

#[tokio::test]
async fn an_undeclared_attribute_is_rejected() {
    let Some(fx) = fixture().await else { return };
    fx.store
        .apply_model(&fx.scope, &organization_model())
        .await
        .expect("apply model");

    let err = fx
        .store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E002".to_owned(),
            attributes: attributes(&[("salary", "100")]),
        })
        .await
        .expect_err("undeclared attribute must be rejected");
    assert_eq!(err.code(), "unknown_attribute", "{err}");
}

#[tokio::test]
async fn writing_before_a_model_is_applied_is_rejected() {
    let Some(fx) = fixture().await else { return };

    let err = fx
        .store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: attributes(&[("name", "Alice")]),
        })
        .await
        .expect_err("no model means no writes");
    assert_eq!(err.code(), "model_not_found", "{err}");
}
