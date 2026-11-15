// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor identity component binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_identity::config::{Config, ManagementConfig};
use portitor_identity::gaiax::{GaiaxCompliance, HttpGaiaxCompliance};
use portitor_identity::issuance::HttpIssuerClient;
use portitor_identity::store::SqliteStore;
use portitor_identity::{Identity, Ports};
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::health;
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, bind_management, init_logging, serve, with_limits,
};

/// Without arguments, runs the identity component. `portitor-identity
/// health` asks the running component's readiness check instead (ADR
/// 0023; for Docker's `HEALTHCHECK`), and exits 0 when it passes.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => serve_identity(),
        [command] if command == "health" => health::probe(&ManagementConfig::from_env().bind),
        _ => {
            eprintln!("portitor-identity takes no arguments, except for: health");
            ExitCode::from(2)
        }
    }
}

#[tokio::main]
async fn serve_identity() -> ExitCode {
    init_logging();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "portitor-identity stopped");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    if config.allow_http {
        tracing::warn!("plain-http DIDs and URLs allowed (PORTITOR_ALLOW_HTTP_CALLBACKS)");
    }
    let key = read_key(&config)?;
    let sts_client_secret = ApiKey::read(&config.sts_client_secret_file)
        .map_err(|e| format!("PORTITOR_STS_CLIENT_SECRET_FILE: {e}"))?;
    let store = SqliteStore::open(&config.db_file)
        .map_err(|e| format!("cannot open {}: {e}", config.db_file.display()))?;
    let compliance = match &config.gaiax {
        Some(gaiax) => {
            let adapter = HttpGaiaxCompliance::new(&gaiax.compliance_url, config.allow_http)?;
            Some(Arc::new(adapter) as Arc<dyn GaiaxCompliance>)
        }
        None => None,
    };
    let ports = Ports {
        store: Arc::new(store),
        directory: Arc::new(WebDirectory::new(config.allow_http)?),
        issuers: Arc::new(HttpIssuerClient::new(config.allow_http)?),
        compliance,
    };
    let identity = Arc::new(Identity::new(&config, key, sts_client_secret, ports)?);
    if let Some(chain) = identity.certificate_chain() {
        tokio::spawn(portitor_identity::certificate::watch(chain));
    }
    if identity.gaiax_on() {
        tracing::info!("Gaia-X onboarding on (PORTITOR_GAIAX_COMPLIANCE_URL)");
        tokio::spawn(portitor_identity::gaiax::run_schedule(Arc::clone(
            &identity,
        )));
    }
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;

    // the management port always serves the health checks (ADR 0023);
    // the management API only with its key
    let checks = Arc::new(portitor_identity::health::readiness_checks(
        &identity,
        config.db_file.clone(),
    ));
    let api = if let Some(key_file) = &config.management.api_key_file {
        let key =
            ApiKey::read(key_file).map_err(|e| format!("PORTITOR_MANAGEMENT_API_KEY_FILE: {e}"))?;
        Some(portitor_identity::management::router(
            Arc::clone(&identity),
            key,
        ))
    } else {
        tracing::info!(
            "management API off (PORTITOR_MANAGEMENT_API_KEY_FILE not set); the management port serves the health checks only"
        );
        None
    };
    let management_listener = bind_management(&config.management.bind).await?;
    let management = Some((management_listener, health::management_app(checks, api)));
    tracing::info!(
        bind = %config.bind,
        did = %identity.participant_id(),
        kid = %identity.kid(),
        db = %config.db_file.display(),
        "portitor-identity listening"
    );
    // the public routes: anyone can call them
    let app = with_limits(
        portitor_identity::router(identity),
        REQUEST_TIMEOUT,
        MAX_CONCURRENT_REQUESTS,
    );
    serve((listener, app), management).await?;
    Ok(())
}

/// The participant's key, from the PEM file.
fn read_key(config: &Config) -> Result<SigningKey, String> {
    let pem = std::fs::read_to_string(&config.key_file).map_err(|e| {
        format!(
            "PORTITOR_IDENTITY_KEY_FILE: cannot read {}: {e}",
            config.key_file.display()
        )
    })?;
    SigningKey::from_pem(&pem).map_err(|e| format!("PORTITOR_IDENTITY_KEY_FILE: {e}"))
}
