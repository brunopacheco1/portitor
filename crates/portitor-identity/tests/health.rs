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

/// `portitor-identity health x` and `portitor-identity x` are usage errors: exit code 2, before
/// anything starts.
#[test]
fn unknown_arguments_exit_with_2() {
    for args in [&["health", "x"][..], &["x"][..]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_portitor-identity"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("takes no arguments"), "{stderr}");
    }
}

/// The identity reads `PORTITOR_DID_CACHE_SECONDS` at start (it resolves
/// the DIDs of issuers and verifiers): a value out of bounds stops the
/// process before it listens, and the log names the setting.
#[test]
fn a_did_cache_time_out_of_bounds_stops_the_start() {
    use portitor_runtime::testing::TempDir;

    let dir = TempDir::new("identity-start");
    let key = include_str!("fixtures/test-only-identity-key.pem");
    let key_file = dir.write("test-only-key.pem", key);
    let secret_file = dir.write("sts-secret", STS_SECRET);
    for value in ["3601", "-1", "5m"] {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_portitor-identity"))
            // nothing of the test machine's own settings
            .env_clear()
            .env("PORTITOR_PARTICIPANT_ID", HOLDER)
            .env(
                "PORTITOR_CREDENTIAL_SERVICE_URL",
                "https://holder.example/credentials",
            )
            .env("PORTITOR_TRUSTED_ISSUERS", ISSUER)
            .env("PORTITOR_IDENTITY_KEY_FILE", &key_file)
            .env("PORTITOR_STS_CLIENT_SECRET_FILE", &secret_file)
            .env("PORTITOR_DB_FILE", dir.path().join("identity.db"))
            .env("PORTITOR_BIND", "127.0.0.1:0")
            .env("PORTITOR_DID_CACHE_SECONDS", value)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // the setting must stop it; a process still there after 30 s fails
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > std::time::Duration::from_secs(30) {
                child.kill().unwrap();
                panic!("{value}: the identity started instead of stopping");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        let mut logged = String::from_utf8_lossy(&output.stdout).into_owned();
        logged.push_str(&String::from_utf8_lossy(&output.stderr));
        assert!(!output.status.success(), "{value}: {logged}");
        assert!(
            logged.contains("PORTITOR_DID_CACHE_SECONDS has an invalid value"),
            "{value}: {logged}"
        );
    }
}
