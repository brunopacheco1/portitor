// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The freeze guard of `portitor-issuer`.
//!
//! From 0.13 the issuer's HTTP routes and environment variables are
//! frozen: `docs/STABILITY.md` lists them, and 1.x may add to the lists
//! but never remove or rename (semantic versioning). These tests compare
//! the lists with the code. The tools are in
//! `portitor_runtime::testing::frozen`.
//!
//! **When a test here fails**, its text names the route or the setting.
//!
//! - "Frozen, but gone from the code": you removed or renamed something
//!   users of 1.x rely on. Put it back. To rename, keep the old name
//!   working next to the new one.
//! - "In the code, but not frozen": you added something. That is
//!   allowed. Add its line to the right list in `docs/STABILITY.md`, in
//!   the same pull request, so that the reviewer sees the stable
//!   interface grow. From then on it cannot be removed.
//! - "the router does not serve it": the route is still documented but
//!   the router lost it. Put it back.
//!
//! Do not edit a list only to make a test pass: removing a line from
//! `docs/STABILITY.md` is a breaking change and needs a new major
//! version.
#![allow(clippy::unwrap_used, missing_docs)]

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use portitor_dcp::{DidDocument, SigningKey};
use portitor_issuer::config::{Config, DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_issuer::store::SqliteStore;
use portitor_issuer::{Issuer, Ports, definitions};
use portitor_runtime::BoxFuture;
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::Directory;
use portitor_runtime::health::{HealthChecks, management_app};
use portitor_runtime::http::Sender;
use portitor_runtime::testing::{TempDir, frozen};
use serde_json::Value;

/// One credential definition, as in `PORTITOR_ISSUER_CREDENTIALS_FILE`.
const DEFINITIONS: &str = r#"[
  {"id": "membership-vc20", "credentialType": "MembershipCredential",
   "profile": "vc20-bssl/jwt", "validityDays": 30,
   "credentialSchema": "https://example.com/membership-credential-schema.json",
   "claims": ["role"]}
]"#;

/// Knows no DID document: these tests only ask whether routes exist.
struct NoDirectory;

impl Directory for NoDirectory {
    fn did_document<'a>(&'a self, did: &'a str) -> BoxFuture<'a, Result<Arc<DidDocument>, String>> {
        Box::pin(async move { Err(format!("no document for {did}")) })
    }

    fn json<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { Err(format!("not served: {url}")) })
    }
}

/// Sends nothing.
struct NoSender;

impl Sender for NoSender {
    fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("not sent in this test".to_owned()) })
    }
}

/// The public router and the management API of a test issuer whose
/// issuer service URL is `https://issuer.example/issuer`, and which
/// serves bundles from the (empty) folder `bundles`.
fn routers(bundles: &Path) -> (Router, Router) {
    let config = Config {
        participant_id: "did:web:issuer.example".to_owned(),
        key_file: "unused".into(),
        retired_key_files: Vec::new(),
        credentials_file: "unused".into(),
        issuer_service_url: "https://issuer.example/issuer".to_owned(),
        bind: "unused".to_owned(),
        db_file: "unused".into(),
        management: ManagementConfig::health_only(DEFAULT_MANAGEMENT_BIND),
        allow_http: false,
        max_active_per_holder: 10,
        bundle_key_file: None,
        bundle_dir: Some(bundles.to_owned()),
    };
    let ports = Ports {
        store: Arc::new(SqliteStore::in_memory().unwrap()),
        directory: Arc::new(NoDirectory),
        sender: Arc::new(NoSender),
    };
    let issuer = Arc::new(
        Issuer::new(
            &config,
            SigningKey::generate().unwrap(),
            definitions::parse(DEFINITIONS).unwrap(),
            ports,
        )
        .unwrap(),
    );
    let public = portitor_issuer::router(Arc::clone(&issuer));
    let key = ApiKey::new("test-only-api-key-0123456789abcdef0123456789ab").unwrap();
    let management = portitor_issuer::management::router(issuer, key);
    (public, management)
}

/// Names the code holds that are no settings of a release build: the
/// old names of renamed settings. The service only refuses to start when
/// one of them is set (`REMOVED_SETTINGS` in `config.rs`).
const NOT_FROZEN: &[&str] = &[
    // renamed in 0.13 to PORTITOR_ALLOW_HTTP
    "PORTITOR_ALLOW_HTTP_CALLBACKS",
];

fn stability() -> String {
    frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/STABILITY.md").unwrap()
}

/// The settings in `docs/STABILITY.md` ("Settings of every binary" and
/// "Settings of portitor-issuer") are the environment variables the
/// issuer's code names: its own crate and `portitor-runtime`. See the top
/// of this file for what to do when it fails.
#[test]
fn frozen_settings() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let folders = [
        crates.join("portitor-issuer/src"),
        crates.join("portitor-runtime/src"),
    ];
    let folders: Vec<&Path> = folders.iter().map(std::path::PathBuf::as_path).collect();
    let result = frozen::check_settings(&stability(), "portitor-issuer", &folders, NOT_FROZEN);
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("portitor-issuer, public port") are
/// the routes of `docs/api/issuer-openapi.json`, and the public router
/// serves each of them (the bundle routes too: the test issuer has a
/// bundle folder). `{issuerService}` is `/issuer` here. See the top of
/// this file for what to do when it fails.
#[tokio::test]
async fn frozen_public_routes() {
    let frozen_routes =
        frozen::list_under(&stability(), "### portitor-issuer, public port").unwrap();
    let description =
        frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/api/issuer-openapi.json")
            .unwrap();
    let documented = frozen::openapi_routes(&description).unwrap();
    let bundles = TempDir::new("frozen-interface");
    let (public, _) = routers(bundles.path());
    let result = frozen::check_routes(
        "public routes of portitor-issuer",
        &frozen_routes,
        &documented,
        &public,
        &[("{issuerService}", "/issuer")],
    )
    .await;
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("Every binary, management port"
/// and "portitor-issuer, management port") are the routes of the
/// generated management `OpenAPI` description, and the management port
/// serves each of them. See the top of this file for what to do when it
/// fails.
#[tokio::test]
async fn frozen_management_routes() {
    let frozen_routes = frozen::management_routes(&stability(), "portitor-issuer").unwrap();
    let description = portitor_issuer::management::openapi_json();
    let documented = frozen::documented_management_routes(&description).unwrap();
    let bundles = TempDir::new("frozen-interface");
    let (_, management) = routers(bundles.path());
    // the whole management port: the health checks and the API
    let port = management_app(Arc::new(HealthChecks::new()), Some(management));
    let result = frozen::check_routes(
        "management routes of portitor-issuer",
        &frozen_routes,
        &documented,
        &port,
        &[],
    )
    .await;
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// Every management route that needs the API key documents HTTP 403,
/// reserved for tokens with limited rights in 1.x (ADR 0025, freeze
/// point 2). A new route must document it too: copy the `status = 403`
/// line of another route in `src/management.rs`.
#[test]
fn every_protected_route_documents_403() {
    let description = portitor_issuer::management::openapi_json();
    let missing = frozen::protected_routes_without_403(&description).unwrap();
    assert!(missing.is_empty(), "no 403 answer documented: {missing:?}");
}
