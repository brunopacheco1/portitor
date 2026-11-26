// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The freeze guard of `portitor-connector`.
//!
//! From 0.13 the connector's HTTP routes and environment variables are
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

use portitor_connector::management::ApiKey;
use portitor_connector::{Config, Ports};
use portitor_runtime::database::DatabaseSetting;
use portitor_runtime::health::{HealthChecks, management_app};
use portitor_runtime::testing::frozen;

mod common;
use common::{NoSender, PROVIDER, authenticating};

/// Names the code holds that are no settings of a release build.
const NOT_FROZEN: &[&str] = &[
    // removed in 0.2; the connector only refuses to start when it is set
    "PORTITOR_DATASETS",
    // renamed in 0.13 to PORTITOR_ALLOW_HTTP; refused in the same way
    "PORTITOR_ALLOW_HTTP_CALLBACKS",
    // read only by a build with the `tck` feature (the DSP TCK)
    "PORTITOR_TCK_AUTHORIZATION",
    // read only by the tests, on a PostgreSQL test server
    // (`portitor_runtime::testing::databases`)
    "PORTITOR_TEST_DATABASE_URL",
    "PORTITOR_TEST_DATABASE_PASSWORD",
    "PORTITOR_TEST_REQUIRE_POSTGRES",
];

fn stability() -> String {
    frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/STABILITY.md").unwrap()
}

fn config() -> Config {
    Config {
        participant_id: PROVIDER.into(),
        public_url: "https://provider.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http: false,
        database: DatabaseSetting::SqliteFile("unused.db".into()),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// With object storage and a push bucket, so that the router has the
/// receiving route of push transfers (1.1). Nothing calls the store.
fn ports() -> Ports {
    let mut ports = authenticating(common::ports(Arc::new(NoSender)), PROVIDER);
    ports.object_storage = Some(Arc::new(common::fake_s3::client("http://127.0.0.1:9")));
    ports
}

/// `request` as sent by the tests' consumer (the test verifier takes
/// `Bearer <did>` at its word).
async fn from_the_consumer(
    mut request: axum::http::Request<axum::body::Body>,
) -> axum::http::Request<axum::body::Body> {
    let caller = common::bearer(common::CONSUMER).parse().unwrap();
    let headers = request.headers_mut();
    headers.insert(axum::http::header::AUTHORIZATION, caller);
    request
}

/// The settings in `docs/STABILITY.md` ("Settings of every binary" and
/// "Settings of portitor-connector") are the environment variables the
/// connector's code names: its own crate, `portitor-dataplane` (linked
/// in, ADR 0028), `portitor-bundle` and `portitor-runtime`. See the top of this file for what to do when it
/// fails.
#[test]
fn frozen_settings() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let folders = [
        crates.join("portitor-connector/src"),
        crates.join("portitor-dataplane/src"),
        crates.join("portitor-bundle/src"),
        crates.join("portitor-runtime/src"),
    ];
    let folders: Vec<&Path> = folders.iter().map(std::path::PathBuf::as_path).collect();
    let result = frozen::check_settings(&stability(), "portitor-connector", &folders, NOT_FROZEN);
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("portitor-connector, public port")
/// are the routes of `docs/api/openapi.json`, and the public router
/// serves each of them. See the top of this file for what to do when it
/// fails.
#[tokio::test]
async fn frozen_public_routes() {
    let frozen_routes =
        frozen::list_under(&stability(), "### portitor-connector, public port").unwrap();
    let description =
        frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/api/openapi.json").unwrap();
    let documented = frozen::openapi_routes(&description).unwrap();
    let router = portitor_connector::router(&config(), ports()).unwrap();
    // Every request of the check comes from a named caller. Without
    // one, the connector answers 404 on every DSP path before it looks
    // at the method (ADR 0016), also on the path of a route that is
    // gone when another route's `{providerPid}` takes its place: `POST
    // .../negotiations/request` would count as served by `GET
    // .../negotiations/{providerPid}`.
    let router = router.layer(axum::middleware::map_request(from_the_consumer));
    let result = frozen::check_routes(
        "public routes of portitor-connector",
        &frozen_routes,
        &documented,
        &router,
        &[],
    )
    .await;
    if let Err(difference) = result {
        panic!("{difference}");
    }
}

/// The routes in `docs/STABILITY.md` ("Every binary, management port"
/// and "portitor-connector, management port") are the routes of the
/// generated management `OpenAPI` description, and the management port
/// serves each of them. See the top of this file for what to do when it
/// fails.
#[tokio::test]
async fn frozen_management_routes() {
    let frozen_routes = frozen::management_routes(&stability(), "portitor-connector").unwrap();
    let description = portitor_connector::management::openapi_json();
    let documented = frozen::documented_management_routes(&description).unwrap();
    let (_, negotiator) = portitor_connector::build(&config(), ports()).unwrap();
    let key = ApiKey::new("test-only-0123456789abcdef0123456789abcdef").unwrap();
    let api = portitor_connector::management::router(negotiator, key);
    // the whole management port: the health checks and the API
    let port = management_app(Arc::new(HealthChecks::new()), Some(api));
    let result = frozen::check_routes(
        "management routes of portitor-connector",
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
