// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor binary.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_bundle::source::{BundleConfig, source_for};
use portitor_bundle::store::CacheFolder;
use portitor_connector::auth::{AuthSettings, CallerVerifier, Prover, dcp_adapters};
use portitor_connector::bundle_harvest::BundleHarvester;
use portitor_connector::health::{Dependencies, OwnIdentityCheck, readiness_checks};
use portitor_connector::management::{DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_runtime::directory::{Directory, WebDirectory};
use portitor_runtime::health;
use portitor_runtime::management::management_port;
use portitor_runtime::request_size::{RequestLimits, limit_port};
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, run_component, serve, with_limits,
};

/// The verifier and the prover (ADR 0016).
type AuthAdapters = (Arc<dyn CallerVerifier>, Arc<dyn Prover>);

/// Without arguments, runs the connector. `portitor-connector health`
/// asks the running connector's readiness check instead (ADR 0023; for
/// Docker's `HEALTHCHECK`), and exits 0 when it passes.
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => serve_connector(),
        [command] if command == "health" => {
            health::probe(&ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND).bind)
        }
        _ => {
            eprintln!("portitor-connector takes no arguments, except for: health");
            ExitCode::from(2)
        }
    }
}

#[tokio::main]
async fn serve_connector() -> ExitCode {
    run_component("portitor-connector", run()).await
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = portitor_connector::Config::from_env()?;
    let limits = RequestLimits::from_env()?;
    let sender = portitor_connector::dsp_sender(config.allow_http_callbacks)?;
    if config.allow_http_callbacks {
        tracing::warn!("http callback addresses allowed (PORTITOR_ALLOW_HTTP_CALLBACKS)");
    }
    let store = Arc::new(
        portitor_connector::store::SqliteNegotiations::open(&config.db_file)
            .map_err(|e| format!("cannot open {}: {e}", config.db_file.display()))?,
    );
    #[cfg(feature = "tck")]
    let sender = portitor_connector::tck::LateStart(sender);
    let mut ports = portitor_connector::Ports::new(Arc::new(sender));
    ports.use_sqlite(&store);
    let settings = AuthSettings::from_env(&config.participant_id, config.allow_http_callbacks)?;
    if let Some(settings) = &settings
        && let Some(warning) = settings.policy_types_warning(&config.credential_types)
    {
        tracing::warn!("{warning}");
    }
    // readiness (ADR 0023): the start check of our identity, run again
    let identity_check = match &settings {
        Some(settings) => {
            let check = Arc::new(OwnIdentityCheck::new(settings)?);
            // runs until the process stops; the probes read its results
            check.spawn_refresh();
            Some(check)
        }
        None => None,
    };
    (ports.verifier, ports.prover) = auth_adapters(settings).await?;
    let bundle = start_bundle_harvest(&config, &mut ports).await?;
    // `build` runs the first harvest before it returns: blocking work (a
    // file read, SQLite, SHACL) on this async task, done once at start
    // before anything is served, like the bundle read above
    let (app, negotiator) = portitor_connector::build(&config, ports)?;
    schedule_harvest(&config, &negotiator);
    let mut bundle_harvester = None;
    if let Some((harvester, start_job)) = bundle {
        let listener = negotiator.bundle_listener();
        // the start run goes into the job log, which exists from `build` on
        listener.record(start_job);
        let harvester = Arc::new(harvester);
        bundle_harvester = Some(Arc::clone(&harvester));
        if harvester.schedule(listener).is_some() {
            tracing::info!(
                refresh_seconds = harvester.refresh_seconds(),
                "looking for new bundle versions"
            );
        } else {
            tracing::info!(
                "bundle: no new version is looked for (fixed PORTITOR_BUNDLE_VERSION, or PORTITOR_BUNDLE_REFRESH_SECONDS=0)"
            );
        }
    }
    // messages saved but not acknowledged before the last stop
    match negotiator.resume() {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "sending pending negotiation messages again"),
        Err(failure) => return Err(format!("cannot read pending messages: {failure:?}").into()),
    }
    let bind = std::env::var("PORTITOR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let listener = tokio::net::TcpListener::bind(&bind).await?;

    // the management port always serves the health checks (ADR 0023);
    // the management API only with its key
    let checks = Arc::new(readiness_checks(Dependencies {
        db_file: config.db_file.clone(),
        negotiator: negotiator.clone(),
        bundle: bundle_harvester,
        identity: identity_check,
    }));
    let settings = ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND);
    let (management_listener, management_app) = management_port(&settings, checks, |key| {
        portitor_connector::management::router(negotiator.clone(), key)
    })
    .await?;
    let management_app = limit_port(management_app, limits.management);
    tracing::info!(
        %bind,
        datasets = negotiator.catalog_size(),
        db = %config.db_file.display(),
        "portitor-connector listening"
    );
    // the public routes: anyone can call them. The size limit is inside
    // the time limit, so a slow body counts against the time too
    let app = limit_port(app, limits.public);
    let app = with_limits(app, REQUEST_TIMEOUT, MAX_CONCURRENT_REQUESTS);
    serve((listener, app), Some((management_listener, management_app))).await?;
    Ok(())
}

/// Takes the authority's signed bundle (ADR 0005), and applies its rules
/// to `ports` through the bundle harvester's copy: the SHACL check of
/// each harvested dataset and the guard on DSP messages. No bundle
/// configured: nothing changes (no SHACL checks, no message guard).
///
/// # Errors
/// A bundle is configured but no version can be used, or its shapes
/// cannot be read: the start stops, as for a bad signature.
async fn start_bundle_harvest(
    config: &portitor_connector::Config,
    ports: &mut portitor_connector::Ports,
) -> Result<
    Option<(BundleHarvester, portitor_connector::harvest::JobRecord)>,
    Box<dyn std::error::Error>,
> {
    let Some(bundle_config) = BundleConfig::from_env(&config.db_file, config.allow_http_callbacks)?
    else {
        return Ok(None);
    };
    let directory: Arc<dyn Directory> = Arc::new(WebDirectory::new(bundle_config.allow_http)?);
    let source = source_for(&bundle_config, directory)?;
    let store = Arc::new(CacheFolder::new(&bundle_config.cache_dir));
    let started = BundleHarvester::start(bundle_config, source, store).await?;
    ports.use_bundle_rules(&started.harvester.copy());
    Ok(Some((started.harvester, started.job)))
}

/// Starts the harvest schedule (`PORTITOR_HARVEST_INTERVAL_SECONDS`) and
/// says in the log what it is.
fn schedule_harvest(
    config: &portitor_connector::Config,
    negotiator: &portitor_connector::Negotiator,
) {
    if let Some(file) = &config.datasets_file {
        tracing::info!(
            file = %file.display(),
            interval_seconds = config.harvest_interval_seconds,
            "harvesting datasets"
        );
    }
    let every = std::time::Duration::from_secs(config.harvest_interval_seconds);
    if negotiator.schedule_harvest(every).is_none() {
        tracing::info!(
            "datasets are harvested only at start (PORTITOR_HARVEST_INTERVAL_SECONDS=0)"
        );
    }
}

/// The DCP adapters. `PORTITOR_STS_URL` and the other DCP settings are
/// required.
#[cfg(not(feature = "tck"))]
async fn auth_adapters(settings: Option<AuthSettings>) -> Result<AuthAdapters, String> {
    let settings = settings.ok_or(
        "PORTITOR_STS_URL is required: DSP requests are authenticated with DCP (ADR 0016)",
    )?;
    dcp_adapters(&settings).await
}

/// The DSP TCK build: the fixed header in `PORTITOR_TCK_AUTHORIZATION`
/// stands for the TCK, in both directions; DCP for everyone else when
/// `PORTITOR_STS_URL` is set (see `portitor_connector::tck`).
#[cfg(feature = "tck")]
async fn auth_adapters(settings: Option<AuthSettings>) -> Result<AuthAdapters, String> {
    use portitor_connector::auth::{NoTokens, RefuseAll};
    use portitor_connector::tck::{TckProver, TckVerifier};
    let (verifier, prover): AuthAdapters = if let Some(settings) = settings {
        dcp_adapters(&settings).await?
    } else {
        tracing::warn!(
            "TCK build without PORTITOR_STS_URL: only the TCK's fixed header is accepted"
        );
        (Arc::new(RefuseAll), Arc::new(NoTokens))
    };
    let fixed = std::env::var("PORTITOR_TCK_AUTHORIZATION")
        .ok()
        .filter(|header| !header.is_empty());
    Ok((
        Arc::new(TckVerifier {
            fixed: fixed.clone(),
            inner: verifier,
        }),
        Arc::new(TckProver {
            fixed,
            inner: prover,
        }),
    ))
}
