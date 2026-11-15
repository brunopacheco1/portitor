// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor identity component binary.

use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_identity::config::Config;
use portitor_identity::gaiax::{GaiaxCompliance, HttpGaiaxCompliance};
use portitor_identity::issuance::HttpIssuerClient;
use portitor_identity::store::SqliteStore;
use portitor_identity::{Identity, Ports};
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, bind_management, serve_with_trusted_routes,
    with_limits,
};

#[tokio::main]
async fn main() -> ExitCode {
    let telemetry = portitor_runtime::telemetry::init("portitor-identity");
    let result = run().await;
    // after the servers stopped (SIGTERM, Ctrl-C) or failed: send the
    // last spans and metrics
    telemetry.shutdown().await;
    match result {
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

    let management = if let Some(settings) = &config.management {
        let key = ApiKey::read(&settings.api_key_file)
            .map_err(|e| format!("PORTITOR_MANAGEMENT_API_KEY_FILE: {e}"))?;
        let listener = bind_management(&settings.bind).await?;
        let app = portitor_identity::management::router(Arc::clone(&identity), key);
        Some((listener, app))
    } else {
        tracing::info!("management API off (PORTITOR_MANAGEMENT_API_KEY_FILE not set)");
        None
    };
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
    // only the STS is called from inside the participant (by its own
    // connector), so only its caller's trace context is taken
    serve_with_trusted_routes((listener, app), management, &[portitor_identity::STS_ROUTE]).await?;
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
