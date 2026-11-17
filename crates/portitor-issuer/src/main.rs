// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor issuer component binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_issuer::config::{Config, DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_issuer::store::SqliteStore;
use portitor_issuer::{Issuer, Ports, definitions};
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::health;
use portitor_runtime::http::HttpSender;
use portitor_runtime::management::management_port;
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
    let config = Config::from_env()?;
    if config.allow_http {
        tracing::warn!("plain-http DIDs and URLs allowed (PORTITOR_ALLOW_HTTP_CALLBACKS)");
    }
    let key = read_key(&config)?;
    let definitions = read_definitions(&config)?;
    let store = SqliteStore::open(&config.db_file)
        .map_err(|e| format!("cannot open {}: {e}", config.db_file.display()))?;
    let ports = Ports {
        store: Arc::new(store),
        directory: Arc::new(WebDirectory::new(config.allow_http)?),
        sender: Arc::new(HttpSender::new(config.allow_http)?),
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
        config.db_file.clone(),
    ));
    let management = management_port(&config.management, checks, |key| {
        portitor_issuer::management::router(Arc::clone(&issuer), key)
    })
    .await?;
    tracing::info!(
        bind = %config.bind,
        did = %issuer.did(),
        kid = %issuer.kid(),
        issuer_service = %config.issuer_service_url,
        db = %config.db_file.display(),
        "portitor-issuer listening"
    );
    // the outbox runs until the process stops
    tokio::spawn(portitor_issuer::delivery::run_outbox(Arc::clone(&issuer)));
    // the public routes: anyone can call them
    let app = with_limits(
        portitor_issuer::router(issuer),
        REQUEST_TIMEOUT,
        MAX_CONCURRENT_REQUESTS,
    );
    serve((listener, app), Some(management)).await?;
    Ok(())
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
    let text = std::fs::read_to_string(&config.credentials_file).map_err(|e| {
        format!(
            "PORTITOR_ISSUER_CREDENTIALS_FILE: cannot read {}: {e}",
            config.credentials_file.display()
        )
    })?;
    definitions::parse(&text).map_err(|e| format!("PORTITOR_ISSUER_CREDENTIALS_FILE: {e}"))
}
