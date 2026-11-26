// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The health checks on the management port (ADR 0023): open without the
//! API key, readiness on the connector's own dependencies only, and the
//! management API still behind its key.
#![allow(clippy::unwrap_used, missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use portitor_connector::health::{Dependencies, readiness_checks};
use portitor_connector::management::{ApiKey, router};
use portitor_connector::store::{ConnectorStore, MIGRATIONS};
use portitor_connector::{Config, HttpSender, Negotiator};
use portitor_runtime::database::{Database, DatabaseSetting};
use portitor_runtime::health::management_app;
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::{CONSUMER, authenticating};

const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";

/// A folder of its own for the test `name`, with the connector's
/// `SQLite` file in it.
fn db_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "portitor-connector-health-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("connector.db")
}

/// A connector on `db_file`, started (its start harvest ran), and its
/// database.
fn negotiator(db_file: &std::path::Path) -> (Negotiator, Arc<Database>) {
    let store = Arc::new(ConnectorStore::open_sqlite_file(db_file).unwrap());
    let setting = DatabaseSetting::SqliteFile(db_file.to_path_buf());
    (started_on(&store, setting), Arc::clone(store.database()))
}

/// A connector on `store`, started.
fn started_on(store: &Arc<ConnectorStore>, database: DatabaseSetting) -> Negotiator {
    let config = Config {
        participant_id: CONSUMER.into(),
        public_url: "https://consumer.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http: false,
        database,
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    };
    let mut ports = common::ports(Arc::new(HttpSender::new(false).unwrap()));
    ports.use_database(store);
    let ports = authenticating(ports, CONSUMER);
    portitor_connector::build(&config, ports).unwrap().1
}

/// The management port: the checks on `database` and its `SQLite` file
/// `db_file` (`None` for PostgreSQL), with the management API
/// (`with_api`) or without.
fn port(
    negotiator: &Negotiator,
    database: &Arc<Database>,
    db_file: Option<PathBuf>,
    with_api: bool,
) -> Router {
    let checks = Arc::new(readiness_checks(Dependencies {
        database: Arc::clone(database),
        db_file,
        negotiator: negotiator.clone(),
        bundle: None,
        identity: None,
    }));
    let api = with_api.then(|| router(negotiator.clone(), ApiKey::new(KEY).unwrap()));
    management_app(checks, api)
}

async fn call(app: &Router, path: &str) -> (StatusCode, Value) {
    let request = Request::get(path).body(Body::empty()).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn ready_once_started_on_its_own_file() {
    let file = db_file("ready");
    let (negotiator, database) = negotiator(&file);
    let app = port(&negotiator, &database, Some(file.clone()), false);
    for path in [
        "/api/check/readiness",
        "/api/check/startup",
        "/api/check/health",
    ] {
        let (status, body) = call(&app, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            body,
            json!({"isSystemHealthy": true, "componentResults": [
                {"component": "database", "isHealthy": true, "failure": null},
                {"component": "harvest", "isHealthy": true, "failure": null}
            ]})
        );
    }
    let (status, body) = call(&app, "/api/check/liveness").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"isSystemHealthy": true, "componentResults": [
            {"component": "process", "isHealthy": true, "failure": null}
        ]})
    );
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// On PostgreSQL the database check asks the database itself. Runs when
/// there is a PostgreSQL test server.
#[tokio::test]
async fn ready_on_postgresql() {
    let Some(url) = portitor_runtime::testing::databases::postgres_url() else {
        return;
    };
    let settings = portitor_runtime::testing::databases::fresh_postgres_settings(&url);
    let setting = DatabaseSetting::Postgres(Box::new(settings));
    let database = Arc::new(Database::open(&setting, &MIGRATIONS).unwrap());
    let store = Arc::new(ConnectorStore::new(Arc::clone(&database)));
    let negotiator = started_on(&store, setting);
    let app = port(&negotiator, &database, None, false);
    let (status, body) = call(&app, "/api/check/readiness").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["componentResults"][0]["component"], "database");
    assert_eq!(body["componentResults"][0]["isHealthy"], true);
}

#[tokio::test]
async fn not_ready_when_its_sqlite_file_is_gone() {
    let file = db_file("gone");
    let (negotiator, database) = negotiator(&file);
    let app = port(&negotiator, &database, Some(file.clone()), false);
    // the volume lost the file (the store still holds it open)
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    let (status, body) = call(&app, "/api/check/readiness").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["isSystemHealthy"], false);
    let database = &body["componentResults"][0];
    assert_eq!(database["component"], "database");
    assert_eq!(database["isHealthy"], false);
    let detail = database["failure"]["failureDetail"].as_str().unwrap();
    assert!(
        detail.starts_with("the SQLite file cannot be opened"),
        "{detail}"
    );
    // the process itself is alive
    assert_eq!(call(&app, "/api/check/liveness").await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_checks_are_open_and_the_api_keeps_its_key() {
    let file = db_file("open");
    let (negotiator, database) = negotiator(&file);
    let full = port(&negotiator, &database, Some(file.clone()), true);
    assert_eq!(call(&full, "/api/check/readiness").await.0, StatusCode::OK);
    assert_eq!(call(&full, "/api/check/liveness").await.0, StatusCode::OK);
    assert_eq!(
        call(&full, "/v1/negotiations").await.0,
        StatusCode::UNAUTHORIZED
    );
    let authed = Request::get("/v1/negotiations")
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .body(Body::empty())
        .unwrap();
    let response = full.clone().oneshot(authed).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // unchanged: GET /health and the description, with the API on
    assert_eq!(call(&full, "/health").await.0, StatusCode::OK);
    assert_eq!(call(&full, "/v1/openapi.json").await.0, StatusCode::OK);
    // without the key file: the four checks and nothing else
    let checks_only = port(&negotiator, &database, Some(file.clone()), false);
    assert_eq!(
        call(&checks_only, "/api/check/readiness").await.0,
        StatusCode::OK
    );
    for path in ["/v1/negotiations", "/v1/openapi.json", "/health"] {
        assert_eq!(
            call(&checks_only, path).await.0,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// `portitor-connector health x` and `portitor-connector x` are usage errors: exit code 2, before
/// anything starts.
#[test]
fn unknown_arguments_exit_with_2() {
    for args in [&["health", "x"][..], &["x"][..]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_portitor-connector"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("takes no arguments"), "{stderr}");
    }
}
