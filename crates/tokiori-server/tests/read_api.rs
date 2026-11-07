//! The authenticated read API over a real database, called through the router
//! without a socket.
//!
//! The database fixture is a local copy of the one in `tokiori-pg/tests`; the
//! server crate tests its routes end to end without reaching into the pg
//! crate's test module.

mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::NaiveDate;
use serde_json::{Value, json};
use tokiori_model::ValidTime;
use tokiori_pg::{NewEntity, NewRelation, PgStore, SetAttribute};
use tokiori_server::{ApiToken, AppState, router};
use tower::ServiceExt;

use common::{fixture, organization_model};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// An app whose one token grants the fixture scope. The token name doubles as
/// the secret the tests present.
fn app_for(fx: &common::Fixture) -> Router {
    router(AppState {
        store: fx.store.clone(),
        tokens: vec![ApiToken {
            name: "ci".to_owned(),
            scope: fx.scope.clone(),
        }],
    })
}

/// Applies the fixture model and creates one employee.
async fn seed(fx: &common::Fixture) {
    fx.store
        .apply_model(&fx.scope, &organization_model())
        .await
        .expect("apply model");
    fx.store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            id: "E001".to_owned(),
            attributes: common::attributes(&[("name", "Ada")]),
        })
        .await
        .expect("create entity");
}

/// Calls the router without a socket and returns the status and JSON body.
async fn get(app: Router, path: &str, authorization: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(path);
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    let request = builder.body(Body::empty()).expect("request");
    let response = app.oneshot(request).await.expect("infallible service");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, serde_json::from_slice(&body).expect("JSON body"))
}

#[tokio::test]
async fn health_needs_no_token_or_database() {
    // The health route never queries the store, so a lazy pool on a dummy URL
    // is enough and this test passes with no database at all.
    let pool = PgStore::new(
        sqlx::PgPool::connect_lazy("postgres://localhost/tokiori-unused")
            .expect("lazy pool never contacts the database"),
    );
    let app = router(AppState {
        store: pool,
        tokens: Vec::new(),
    });
    let request = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("infallible service");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    assert_eq!(body.as_ref(), b"ok");
}

#[tokio::test]
async fn request_without_token_is_401_missing_token() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    let app = app_for(&fx);
    let (status, json) = get(app, "/v1/entities/employee/E001", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["code"], "missing_token");
    fx.cleanup().await;
}

#[tokio::test]
async fn an_unknown_token_is_401_even_though_a_valid_one_exists() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    let app = app_for(&fx);

    // A token that is not in the list must not be accepted just because some
    // other token is. Without this case, accepting any presented token would
    // still pass every other test.
    let (status, json) = get(
        app.clone(),
        "/v1/entities/employee/E001",
        Some("Bearer not-a-real-token"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["code"], "missing_token");

    // The body must not leak the entity either.
    assert!(json.get("entity").is_none(), "{json}");

    fx.cleanup().await;
}
#[tokio::test]
async fn valid_token_reads_the_entity() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    let app = app_for(&fx);
    let (status, json) = get(app, "/v1/entities/employee/E001", Some("Bearer ci")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["scope"], json!(fx.scope));
    assert_eq!(json["entity"]["id"], "E001");
    assert_eq!(json["entity"]["attributes"]["name"], "Ada");
    fx.cleanup().await;
}

#[tokio::test]
async fn unknown_entity_id_is_404_not_found() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    let app = app_for(&fx);
    let (status, json) = get(app, "/v1/entities/employee/E999", Some("Bearer ci")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["code"], "not_found");
    fx.cleanup().await;
}

#[tokio::test]
async fn missing_valid_at_is_400_invalid_query() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    let app = app_for(&fx);
    let (status, json) = get(app, "/v1/attributes/employee/E001/name", Some("Bearer ci")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    fx.cleanup().await;
}

#[tokio::test]
async fn attribute_read_is_replayable() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    fx.store
        .set_attribute(SetAttribute {
            scope: fx.scope.clone(),
            entity_type: "employee".to_owned(),
            entity_id: "E001".to_owned(),
            attribute: "name".to_owned(),
            value: json!("Grace"),
            valid_time: ValidTime::unbounded_from(day(2027, 1, 1)),
        })
        .await
        .expect("set attribute");
    let app = app_for(&fx);

    let (status, json) = get(
        app.clone(),
        "/v1/attributes/employee/E001/name?valid_at=2027-06-01",
        Some("Bearer ci"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["value"], "Grace");
    assert_eq!(json["known_revision"].as_u64(), Some(1));
    assert_eq!(json["valid_at"], "2027-06-01");

    // A revision before the existence of any fact cannot be answered.
    let (status, json) = get(
        app.clone(),
        "/v1/attributes/employee/E001/name?valid_at=2027-06-01&known_revision=0",
        Some("Bearer ci"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "unknown_revision");

    // Before the change was recorded the fact simply did not hold.
    let (status, json) = get(
        app,
        "/v1/attributes/employee/E001/name?valid_at=2026-12-31",
        Some("Bearer ci"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["value"].is_null());
    assert_eq!(json["known_revision"].as_u64(), Some(1));
    fx.cleanup().await;
}

#[tokio::test]
async fn role_read_names_the_participant() {
    let Some(fx) = fixture().await else {
        return;
    };
    seed(&fx).await;
    fx.store
        .create_entity(NewEntity {
            scope: fx.scope.clone(),
            entity_type: "department".to_owned(),
            id: "D-SALES".to_owned(),
            attributes: common::attributes(&[("name", "Sales")]),
        })
        .await
        .expect("create department");
    fx.store
        .create_relation(NewRelation {
            scope: fx.scope.clone(),
            relation_type: "membership".to_owned(),
            id: "M001".to_owned(),
            roles: std::collections::BTreeMap::from([
                ("employee".to_owned(), "E001".to_owned()),
                ("department".to_owned(), "D-SALES".to_owned()),
            ]),
            attributes: common::attributes(&[]),
            valid_time: ValidTime::unbounded_from(day(2027, 4, 1)),
        })
        .await
        .expect("create relation");

    let app = app_for(&fx);
    let (status, json) = get(
        app,
        "/v1/roles/membership/M001/department?valid_at=2027-06-01",
        Some("Bearer ci"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["participant_id"], "D-SALES");
    assert_eq!(json["known_revision"].as_u64(), Some(1));
    assert_eq!(json["valid_at"], "2027-06-01");
    fx.cleanup().await;
}
