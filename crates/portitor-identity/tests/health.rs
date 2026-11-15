// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The health checks on the management port (ADR 0023): open without the
//! API key, readiness on the component's own dependencies only, and the
//! management API still behind its key.
#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::*;
use portitor_identity::health::readiness_checks;
use portitor_identity::store::SqliteStore;
use portitor_runtime::health::management_app;
use serde_json::{Value, json};

/// A `SQLite` file of its own for the test `name`.
fn db_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "portitor-identity-health-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("identity.db")
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

/// The management port of `world`, its checks reading `db_file`, with the
/// management API (`with_api`) or without.
fn port(world: &World, db_file: PathBuf, with_api: bool) -> Router {
    let checks = Arc::new(readiness_checks(&world.identity, db_file));
    let api = with_api.then(|| world.management.clone());
    management_app(checks, api)
}

fn components(body: &Value) -> Vec<(String, bool)> {
    body["componentResults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["component"].as_str().unwrap().to_owned(),
                r["isHealthy"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn ready_when_its_file_key_and_document_are() {
    let world = World::new();
    let file = db_file("ready");
    let _store = SqliteStore::open(&file).unwrap();
    let app = port(&world, file.clone(), true);
    for path in [
        "/api/check/readiness",
        "/api/check/startup",
        "/api/check/health",
    ] {
        let (status, body) = call(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert_eq!(body["isSystemHealthy"], true);
        assert_eq!(
            components(&body),
            [
                ("database".to_owned(), true),
                ("key".to_owned(), true),
                ("did-document".to_owned(), true)
            ]
        );
    }
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn not_ready_when_its_sqlite_file_is_gone() {
    let world = World::new();
    let file = db_file("gone");
    let store = SqliteStore::open(&file).unwrap();
    let app = port(&world, file.clone(), false);
    assert_eq!(
        call(&app, get("/api/check/readiness")).await.0,
        StatusCode::OK
    );
    // the volume lost the file (the store still holds it open)
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    let (status, body) = call(&app, get("/api/check/readiness")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["isSystemHealthy"], false);
    let database = &body["componentResults"][0];
    assert_eq!(database["component"], "database");
    assert_eq!(database["isHealthy"], false);
    let detail = database["failure"]["failureDetail"].as_str().unwrap();
    assert!(detail.contains("cannot be opened"), "{detail}");
    assert_eq!(database["failure"]["messages"], json!([detail]));
    // the others still pass; liveness checks nothing but the process
    assert_eq!(body["componentResults"][1]["isHealthy"], true);
    let (status, body) = call(&app, get("/api/check/liveness")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["componentResults"][0]["component"], "process");
    drop(store);
}

#[tokio::test]
async fn the_checks_are_open_and_the_api_keeps_its_key() {
    let world = World::new();
    let file = db_file("open");
    let _store = SqliteStore::open(&file).unwrap();
    // with the management API: the checks need no key, the API does
    let full = port(&world, file.clone(), true);
    assert_eq!(
        call(&full, get("/api/check/liveness")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&full, get("/api/check/readiness")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&full, get("/v1/credentials")).await.0,
        StatusCode::UNAUTHORIZED
    );
    let authed = Request::get("/v1/credentials")
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&full, authed).await.0, StatusCode::OK);
    assert_eq!(call(&full, get("/health")).await.0, StatusCode::OK);
    // without it: the checks only
    let checks_only = port(&world, file.clone(), false);
    assert_eq!(
        call(&checks_only, get("/api/check/readiness")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&checks_only, get("/v1/credentials")).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&checks_only, get("/v1/openapi.json")).await.0,
        StatusCode::NOT_FOUND
    );
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
