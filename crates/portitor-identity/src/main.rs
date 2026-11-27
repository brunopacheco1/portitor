// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor identity component binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_dcp::SigningKey;
use portitor_identity::config::{
    Config, DEFAULT_MANAGEMENT_BIND, DatabaseSetting, ManagementConfig,
};
use portitor_identity::gaiax::{GaiaxCompliance, HttpGaiaxCompliance};
use portitor_identity::issuance::HttpIssuerClient;
use portitor_identity::store::{DatabaseStore, MIGRATIONS};
use portitor_identity::{Identity, Ports};
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::database::{Database, SQLITE_DEPRECATED};
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::health;
use portitor_runtime::internal_addresses::InternalAddresses;
use portitor_runtime::management::management_port;
use portitor_runtime::panics::spawn_supervised;
use portitor_runtime::request_size::RequestLimits;
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
    // first, so a wrong value stops the start before anything else
    let internal_addresses = InternalAddresses::from_env()?;
    let config = Config::from_env()?;
    let limits = RequestLimits::from_env()?;
    if config.allow_http {
        tracing::warn!("plain-http DIDs and URLs allowed (PORTITOR_ALLOW_HTTP)");
    }
    let key = read_key(&config)?;
    let sts_client_secret = ApiKey::read(&config.sts_client_secret_file)
        .map_err(|e| format!("PORTITOR_STS_CLIENT_SECRET_FILE: {e}"))?;
    let database = Arc::new(open_database(&config)?);
    let store = DatabaseStore::new(Arc::clone(&database));
    let compliance = match &config.gaiax {
        Some(gaiax) => {
            let adapter = HttpGaiaxCompliance::new(&gaiax.compliance_url, config.allow_http)?;
            Some(Arc::new(adapter) as Arc<dyn GaiaxCompliance>)
        }
        None => None,
    };
    // how long another party's DID document is reused (R-41)
    let did_cache_seconds = portitor_runtime::did::cache_seconds_from_env()?;
    // callers and issuers choose their DID documents and the addresses
    // in them: with the block on internal addresses (R-17). The Gaia-X
    // compliance service above is the operator's choice: without it
    internal_addresses.log_at_start();
    let directory = Arc::new(WebDirectory::for_partners(
        config.allow_http,
        did_cache_seconds,
        &internal_addresses,
    )?);
    let issuers = HttpIssuerClient::for_partners(config.allow_http, &internal_addresses)?;
    let ports = Ports {
        store: Arc::new(store),
        directory: Arc::clone(&directory) as _,
        issuers: Arc::new(issuers),
        compliance,
        // the same partner sender as the DID documents (TD-20)
        status_lists: directory,
    };
    let identity = Arc::new(Identity::new(&config, key, sts_client_secret, ports)?);
    for retired in identity.retired_keys() {
        let kid = portitor_runtime::did_document::key_id(identity.participant_id(), retired);
        tracing::info!(kid = %kid, "retired key listed in the DID document; it does not sign");
    }
    if let Some(chain) = identity.certificate_chain() {
        // supervised: started again after a panic (portitor_runtime::panics)
        spawn_supervised("certificate_watch", move || {
            portitor_identity::certificate::watch(Arc::clone(&chain))
        });
    }
    if identity.gaiax_on() {
        tracing::info!("Gaia-X onboarding on (PORTITOR_GAIAX_COMPLIANCE_URL)");
        let scheduled = Arc::clone(&identity);
        spawn_supervised("gaiax_schedule", move || {
            portitor_identity::gaiax::run_schedule(Arc::clone(&scheduled))
        });
    }
    // the credential status check (TD-20), unless
    // PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS is 0
    if portitor_identity::status_check::schedule(&identity).is_some() {
        tracing::info!(
            every_seconds = config.credential_status_check_seconds,
            "credential status check on (PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS)"
        );
    }
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;

    // the management port always serves the health checks (ADR 0023);
    // the management API only with its key
    let checks = Arc::new(portitor_identity::health::readiness_checks(
        &identity,
        config.database.clone(),
        database,
    ));
    let (management_listener, management_app) =
        management_port(&config.management, checks, |key| {
            portitor_identity::management::router(Arc::clone(&identity), key)
        })
        .await?;
    let management = (
        management_listener,
        limits.on_management_port(management_app),
    );
    tracing::info!(
        bind = %config.bind,
        did = %identity.participant_id(),
        kid = %identity.kid(),
        db = %shown(&config),
        "portitor-identity listening"
    );
    // the public routes: anyone can call them. The size limit is inside
    // the time limit, so a slow body counts against the time too
    let app = limits.on_public_port(portitor_identity::router(identity));
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
