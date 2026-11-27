// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor issuer component binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_issuer::config::{Config, DEFAULT_MANAGEMENT_BIND, DatabaseSetting, ManagementConfig};
use portitor_issuer::store::{DatabaseStore, MIGRATIONS};
use portitor_issuer::{Issuer, Ports, definitions};
use portitor_runtime::database::{Database, SQLITE_DEPRECATED};
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::health;
use portitor_runtime::http::HttpSender;
use portitor_runtime::internal_addresses::InternalAddresses;
use portitor_runtime::management::management_port;
use portitor_runtime::panics::spawn_supervised;
use portitor_runtime::request_size::RequestLimits;
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, run_component, serve, with_limits,
};

/// Without arguments, runs the issuer. `portitor-issuer bundle build
/// ...` builds and signs a schema and vocabulary bundle offline instead
/// (see `portitor_issuer::bundle_build`), and exits. `portitor-issuer
/// health` asks the running issuer's readiness check (ADR 0023; for
/// Docker's `HEALTHCHECK`), and exits 0 when it passes.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => serve_issuer(),
        // `rest @ ..`: every argument after `bundle`, as a slice
        [command, rest @ ..] if command == "bundle" => bundle_command(rest),
        [command] if command == "health" => {
            health::probe(&ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND).bind)
        }
        _ => {
            eprintln!(
                "portitor-issuer takes no arguments, except for:\n  portitor-issuer health\n  {}",
                portitor_issuer::bundle_build::USAGE
            );
            ExitCode::from(2)
        }
    }
}

/// `portitor-issuer bundle ...`: prints the result or the error.
fn bundle_command(args: &[OsString]) -> ExitCode {
    match portitor_issuer::bundle_build::run(args) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn serve_issuer() -> ExitCode {
    run_component("portitor-issuer", run()).await
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // first, so a wrong value stops the start before anything else
    let internal_addresses = InternalAddresses::from_env()?;
    let config = Config::from_env()?;
    let limits = RequestLimits::from_env()?;
    if config.allow_http {
        tracing::warn!("plain-http DIDs and URLs allowed (PORTITOR_ALLOW_HTTP)");
    }
    let key = read_key(&config)?;
    let definitions = read_definitions(&config)?;
    let database = Arc::new(open_database(&config)?);
    let store = DatabaseStore::new(Arc::clone(&database));
    // how long a holder's DID document is reused (R-41)
    let did_cache_seconds = portitor_runtime::did::cache_seconds_from_env()?;
    // holders choose their DID documents and credential services: both
    // with the block on internal addresses (R-17)
    internal_addresses.log_at_start();
    let directory =
        WebDirectory::for_partners(config.allow_http, did_cache_seconds, &internal_addresses)?;
    let sender = HttpSender::for_partners(config.allow_http, &internal_addresses)?;
    let ports = Ports {
        store: Arc::new(store),
        directory: Arc::new(directory),
        sender: Arc::new(sender),
    };
    let credential_key = key.public_jwk();
    let mut issuer = Issuer::new(&config, key, definitions, ports)?;
    for retired in issuer.retired_keys() {
        let kid = portitor_runtime::did_document::key_id(issuer.did(), retired);
        tracing::info!(kid = %kid, "retired key listed in the DID document; it does not sign");
    }
    if let Some(file) = &config.bundle_key_file {
        let bundle_key = portitor_issuer::bundles::read_public_key(file)?;
        if bundle_key == credential_key {
            return Err("PORTITOR_ISSUER_BUNDLE_KEY_FILE: the bundle key must not be the credential signing key".into());
        }
        if issuer.retired_keys().contains(&bundle_key) {
            return Err("PORTITOR_ISSUER_BUNDLE_KEY_FILE: the bundle key must not be a retired credential key".into());
        }
        issuer.add_bundle_key(&bundle_key);
        tracing::info!(kid = %portitor_issuer::bundles::bundle_kid(issuer.did()), "bundle key listed in the DID document");
    }
    if let Some(folder) = &config.bundle_dir {
        tracing::info!(folder = %folder.display(), "serving bundles at /bundles/");
    }
    let issuer = Arc::new(issuer);
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;

    // the management port always serves the health checks (ADR 0023);
    // the management API only with its key
    let checks = Arc::new(portitor_issuer::health::readiness_checks(
        &issuer,
        config.database.clone(),
        database,
    ));
    let (management_listener, management_app) =
        management_port(&config.management, checks, |key| {
            portitor_issuer::management::router(Arc::clone(&issuer), key)
        })
        .await?;
    let management = (
        management_listener,
        limits.on_management_port(management_app),
    );
    tracing::info!(
        bind = %config.bind,
        did = %issuer.did(),
        kid = %issuer.kid(),
        issuer_service = %config.issuer_service_url,
        db = %shown(&config),
        "portitor-issuer listening"
    );
    // the outbox runs until the process stops; supervised: started again
    // after a panic (portitor_runtime::panics)
    let outbox_issuer = Arc::clone(&issuer);
    spawn_supervised("issuer_outbox", move || {
        portitor_issuer::delivery::run_outbox(Arc::clone(&outbox_issuer))
    });
    // the public routes: anyone can call them. The size limit is inside
    // the time limit, so a slow body counts against the time too
    let app = limits.on_public_port(portitor_issuer::router(issuer));
    let app = with_limits(app, REQUEST_TIMEOUT, MAX_CONCURRENT_REQUESTS);
    serve((listener, app), Some(management)).await?;
    Ok(())
}

/// Opens the database and brings its schema up to date; warns once
/// when it is a `SQLite` file (deprecated since 1.3).
fn open_database(config: &Config) -> Result<Database, String> {
    let database = Database::open(&config.database, &MIGRATIONS)
        .map_err(|e| format!("cannot open the database {}: {e}", shown(config)))?;
    if !database.is_postgres() {
        tracing::warn!("{SQLITE_DEPRECATED}");
    }
    Ok(database)
}

/// The database, for the log: the `SQLite` file, or the PostgreSQL URL
/// without its query.
fn shown(config: &Config) -> String {
    match &config.database {
        DatabaseSetting::SqliteFile(path) => path.display().to_string(),
        DatabaseSetting::Postgres(settings) => settings.shown().to_owned(),
    }
}

/// The issuer's key, from the PEM file.
fn read_key(config: &Config) -> Result<SigningKey, String> {
    let pem = std::fs::read_to_string(&config.key_file).map_err(|e| {
        format!(
            "PORTITOR_ISSUER_KEY_FILE: cannot read {}: {e}",
            config.key_file.display()
        )
    })?;
    SigningKey::from_pem(&pem).map_err(|e| format!("PORTITOR_ISSUER_KEY_FILE: {e}"))
}

/// The credential definitions, checked.
fn read_definitions(config: &Config) -> Result<Vec<definitions::Definition>, String> {
    let text = std::fs::read_to_string(&config.credential_definitions_file).map_err(|e| {
        format!(
            "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE: cannot read {}: {e}",
            config.credential_definitions_file.display()
        )
    })?;
    definitions::parse(&text)
        .map_err(|e| format!("PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE: {e}"))
}
