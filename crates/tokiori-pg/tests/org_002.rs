//! ORG-002: applying a model is idempotent, a conflicting re-apply changes
//! nothing, and a restart reads back exactly what was written.

mod common;

use std::collections::BTreeMap;

use chrono::NaiveDate;
use common::{attributes, fixture, organization_model};
use tokiori_model::{ValidTime, canonical_json};
use tokiori_pg::{ApplyOutcome, EntityRow, NewEntity, NewRelation, PgStore, RelationRow};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

#[tokio::test]
async fn applying_the_same_model_twice_changes_nothing() {
    let Some(fx) = fixture().await else { return };
    let model = organization_model();

    let first = fx
        .store
        .apply_model(&fx.scope, &model)
        .await
        .expect("apply");
    let second = fx
        .store
        .apply_model(&fx.scope, &model)
        .await
        .expect("re-apply");

    assert_eq!(first, ApplyOutcome::Created);
    assert_eq!(second, ApplyOutcome::Unchanged);
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM models")
        .fetch_one(fx.store.pool())
        .await
        .expect("count models");
    assert_eq!(versions, 1);
}

#[tokio::test]
async fn a_different_definition_under_the_same_version_is_rejected_and_leaves_no_rows() {
    let Some(fx) = fixture().await else { return };
    let original = organization_model();
    fx.store
        .apply_model(&fx.scope, &original)
        .await
        .expect("apply");

    let mut changed = original.clone();
    changed.entities.get_mut("employee").expect("employee");
    changed
        .entities
        .get_mut("employee")
        .expect("employee")
        .attributes
        .insert("badge".to_owned(), tokiori_model::AttributeType::Text);

    let err = fx
        .store
        .apply_model(&fx.scope, &changed)
        .await
        .expect_err("same version, different contents must be rejected");
    assert_eq!(err.code(), "model_version_conflict", "{err}");

    let stored = fx.store.latest_model(&fx.scope).await.expect("load model");
    assert_eq!(canonical_json(&stored), canonical_json(&original));
}

#[tokio::test]
async fn a_restarted_process_reads_back_the_same_ids_attributes_and_relations() {
    let Some(fx) = fixture().await else { return };
    let model = organization_model();
    let scope = fx.scope.clone();
    fx.store.apply_model(&scope, &model).await.expect("apply");

    fx.store
        .create_entity(NewEntity {
            scope: scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: attributes(&[("name", "Alice")]),
        })
        .await
        .expect("create employee");
    fx.store
        .create_entity(NewEntity {
            scope: scope.clone(),
            entity_type: "department".to_owned(),
            id: "D-SALES".to_owned(),
            attributes: attributes(&[("name", "Sales")]),
        })
        .await
        .expect("create department");
    fx.store
        .create_relation(NewRelation {
            scope: scope.clone(),
            relation_type: "membership".to_owned(),
            id: "M001".to_owned(),
            roles: BTreeMap::from([
                ("employee".to_owned(), "E001".to_owned()),
                ("department".to_owned(), "D-SALES".to_owned()),
            ]),
            attributes: attributes(&[("title", "Account manager")]),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("create membership");

    // A new pool with no shared session state stands in for a restarted process.
    let restarted = PgStore::new(fx.store.pool().clone());
    let entity: EntityRow = restarted
        .get_entity(&scope, "employee", "E001")
        .await
        .expect("read employee")
        .expect("employee exists");
    assert_eq!(entity.attributes["name"], serde_json::json!("Alice"));
    assert_eq!(entity.model, "organization");

    let relation: RelationRow = restarted
        .get_relation(&scope, "membership", "M001")
        .await
        .expect("read membership")
        .expect("membership exists");
    assert_eq!(relation.roles["employee"], "E001");
    assert_eq!(relation.roles["department"], "D-SALES");
    assert_eq!(
        relation.attributes["title"],
        serde_json::json!("Account manager")
    );
    assert_eq!(
        relation.valid_time,
        ValidTime::unbounded_from(day(2027, 4, 1))
    );

    let reloaded = restarted.latest_model(&scope).await.expect("load model");
    assert_eq!(canonical_json(&reloaded), canonical_json(&model));
}
