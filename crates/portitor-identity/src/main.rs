// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor identity component binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_identity::config::{Config, DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_identity::gaiax::{GaiaxCompliance, HttpGaiaxCompliance};
use portitor_identity::issuance::HttpIssuerClient;
use portitor_identity::store::SqliteStore;
use portitor_identity::{Identity, Ports};
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::health;
use portitor_runtime::management::management_port;
use portitor_runtime::request_size::{RequestLimits, limit_port};
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, run_component, serve_with_trusted_routes, with_limits,
};

/// Without arguments, runs the identity component. `portitor-identity
/// health` asks the running component's readiness check instead (ADR
/// 0023; for Docker's `HEALTHCHECK`), and exits 0 when it passes.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => serve_identity(),
        [command] if command == "health" => {
            health::probe(&ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND).bind)
        }
        _ => {
            eprintln!("portitor-identity takes no arguments, except for: health");
            ExitCode::from(2)
        }
    }
}

#[tokio::main]
async fn serve_identity() -> ExitCode {
    run_component("portitor-identity", run()).await
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let limits = RequestLimits::from_env()?;
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
    let (management_listener, management_app) =
        management_port(&config.management, checks, |key| {
            portitor_identity::management::router(Arc::clone(&identity), key)
        })
        .await?;
    let management = (
        management_listener,
        limit_port(management_app, limits.management),
    );
    tracing::info!(
        bind = %config.bind,
        did = %identity.participant_id(),
        kid = %identity.kid(),
        db = %config.db_file.display(),
        "portitor-identity listening"
    );
    // the public routes: anyone can call them. The size limit is inside
    // the time limit, so a slow body counts against the time too
    let app = limit_port(portitor_identity::router(identity), limits.public);
    let app = with_limits(app, REQUEST_TIMEOUT, MAX_CONCURRENT_REQUESTS);
    // only the STS is called from inside the participant (by its own
    // connector), so only its caller's trace context is taken
    serve_with_trusted_routes(
        (listener, app),
        Some(management),
        &[portitor_identity::STS_ROUTE],
    )
    .await?;
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
