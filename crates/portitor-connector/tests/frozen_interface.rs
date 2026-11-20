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
use portitor_runtime::health::{HealthChecks, management_app};
use portitor_runtime::testing::frozen;

mod common;
use common::{NoSender, PROVIDER, authenticating};

/// Whether the management API may have routes that are not in the frozen
/// list yet.
///
/// `true` only while release 0.13 is put together: the management call
/// that reads a provider's catalog (TD-38) is written on another branch.
/// **To finish:** add its routes to the list "portitor-connector,
/// management port" in `docs/STABILITY.md`, then set this to `false`.
/// From then on a new management route fails the test until it is
/// listed, as on the other ports.
const MANAGEMENT_ROUTES_MAY_BE_UNLISTED: bool = true;

/// Names the code holds that are no settings of a release build.
const NOT_FROZEN: &[&str] = &[
    // removed in 0.2; the connector only refuses to start when it is set
    "PORTITOR_DATASETS",
    // read only by a build with the `tck` feature (the DSP TCK)
    "PORTITOR_TCK_AUTHORIZATION",
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
        allow_http_callbacks: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

fn ports() -> Ports {
    authenticating(Ports::new(Arc::new(NoSender)), PROVIDER)
}

/// The settings in `docs/STABILITY.md` ("Settings of every binary" and
/// "Settings of portitor-connector") are the environment variables the
/// connector's code names: its own crate, `portitor-bundle` and
/// `portitor-runtime`. See the top of this file for what to do when it
/// fails.
#[test]
fn frozen_settings() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let folders = [
        crates.join("portitor-connector/src"),
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
/// fails, and [`MANAGEMENT_ROUTES_MAY_BE_UNLISTED`].
#[tokio::test]
async fn frozen_management_routes() {
    let frozen_routes = frozen::management_routes(&stability(), "portitor-connector").unwrap();
    let description = portitor_connector::management::openapi_json();
    let mut documented = frozen::documented_management_routes(&description).unwrap();
    if MANAGEMENT_ROUTES_MAY_BE_UNLISTED {
        // look only at the frozen routes: a route that is documented but
        // not frozen yet passes
        documented.retain(|route| frozen_routes.contains(route));
    }
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
