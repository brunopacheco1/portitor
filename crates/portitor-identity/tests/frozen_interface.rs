// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The freeze guard of `portitor-identity`.
//!
//! From 0.13 the identity's HTTP routes and environment variables are
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

mod common;

use std::path::Path;
use std::sync::Arc;

use common::World;
use portitor_runtime::health::{HealthChecks, management_app};
use portitor_runtime::testing::frozen;

/// Names the code holds that are no settings of a release build: the
/// old names of renamed settings. The service only refuses to start when
/// one of them is set (`REMOVED_SETTINGS` in `config.rs`).
const NOT_FROZEN: &[&str] = &[
    // renamed in 0.13 to PORTITOR_ALLOW_HTTP
    "PORTITOR_ALLOW_HTTP_CALLBACKS",
    // renamed in 0.13 to PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE
    "PORTITOR_IDENTITY_CERTIFICATE_CHAIN",
    // read only by the tests, on a PostgreSQL test server
    // (`portitor_runtime::testing::databases`)
    "PORTITOR_TEST_DATABASE_URL",
    "PORTITOR_TEST_DATABASE_PASSWORD",
    "PORTITOR_TEST_REQUIRE_POSTGRES",
];

fn stability() -> String {
    frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/STABILITY.md").unwrap()
}

/// The settings in `docs/STABILITY.md` ("Settings of every binary" and
/// "Settings of portitor-identity") are the environment variables the
/// identity's code names: its own crate and `portitor-runtime`. See the
/// top of this file for what to do when it fails.
#[test]
fn frozen_settings() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let folders = [
        crates.join("portitor-identity/src"),
        crates.join("portitor-runtime/src"),
    ];
    let folders: Vec<&Path> = folders.iter().map(std::path::PathBuf::as_path).collect();
    let result = frozen::check_settings(&stability(), "portitor-identity", &folders, NOT_FROZEN);
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("portitor-identity, public port")
/// are the routes of `docs/api/identity-openapi.json`, and the public
/// router serves each of them. The test identity's credential service
/// URL is `https://holder.example/cs`, so `{credentialService}` is `/cs`
/// here. See the top of this file for what to do when it fails.
#[tokio::test]
async fn frozen_public_routes() {
    let frozen_routes =
        frozen::list_under(&stability(), "### portitor-identity, public port").unwrap();
    let description =
        frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/api/identity-openapi.json")
            .unwrap();
    let documented = frozen::openapi_routes(&description).unwrap();
    let world = World::new();
    let result = frozen::check_routes(
        "public routes of portitor-identity",
        &frozen_routes,
        &documented,
        &world.app,
        &[("{credentialService}", "/cs")],
    )
    .await;
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("Every binary, management port"
/// and "portitor-identity, management port") are the routes of the
/// generated management `OpenAPI` description, and the management port
/// serves each of them. See the top of this file for what to do when it
/// fails.
#[tokio::test]
async fn frozen_management_routes() {
    let frozen_routes = frozen::management_routes(&stability(), "portitor-identity").unwrap();
    let description = portitor_identity::management::openapi_json();
    let documented = frozen::documented_management_routes(&description).unwrap();
    let world = World::new();
    // the whole management port: the health checks and the API
    let port = management_app(
        Arc::new(HealthChecks::new()),
        Some(world.management.clone()),
    );
    let result = frozen::check_routes(
        "management routes of portitor-identity",
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
    let description = portitor_identity::management::openapi_json();
    let missing = frozen::protected_routes_without_403(&description).unwrap();
    assert!(missing.is_empty(), "no 403 answer documented: {missing:?}");
}
