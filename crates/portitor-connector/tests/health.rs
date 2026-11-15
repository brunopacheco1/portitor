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
use portitor_connector::store::SqliteNegotiations;
use portitor_connector::{Config, HttpSender, Negotiator, Ports};
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

/// A connector on `db_file`, started (its start harvest ran).
fn negotiator(db_file: &std::path::Path) -> Negotiator {
    let config = Config {
        participant_id: CONSUMER.into(),
        public_url: "https://consumer.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http_callbacks: false,
        db_file: db_file.to_path_buf(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    };
    let store = Arc::new(SqliteNegotiations::open(db_file).unwrap());
    let mut ports = Ports::new(Arc::new(HttpSender::new(false).unwrap()));
    ports.use_sqlite(&store);
    let ports = authenticating(ports, CONSUMER);
    portitor_connector::build(&config, ports).unwrap().1
}

/// The management port: the checks on `db_file`, with the management
/// API (`with_api`) or without.
fn port(negotiator: &Negotiator, db_file: PathBuf, with_api: bool) -> Router {
    let checks = Arc::new(readiness_checks(Dependencies {
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
    let negotiator = negotiator(&file);
    let app = port(&negotiator, file.clone(), false);
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

#[tokio::test]
async fn not_ready_when_its_sqlite_file_is_gone() {
    let file = db_file("gone");
    let negotiator = negotiator(&file);
    let app = port(&negotiator, file.clone(), false);
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
    let negotiator = negotiator(&file);
    let full = port(&negotiator, file.clone(), true);
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
    let checks_only = port(&negotiator, file.clone(), false);
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
