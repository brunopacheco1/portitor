// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The `portitor-connector` binary: reads the settings, puts the adapters
//! together ([`portitor_connector::build`]), sends again the messages left
//! from the last run, and serves the public port and the management port.
//! `portitor-connector health` asks a running connector's readiness check
//! instead.

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use portitor_bundle::source::{BundleConfig, source_for};
use portitor_bundle::store::CacheFolder;
use portitor_connector::auth::{AuthSettings, CallerVerifier, Prover, dcp_adapters};
use portitor_connector::bundle_harvest::{BundleHarvester, SharedBundleVersion};
use portitor_connector::health::{Dependencies, OwnIdentityCheck, readiness_checks};
use portitor_connector::management::{DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_connector::store::{ConnectorStore, MIGRATIONS};
use portitor_runtime::database::{Database, DatabaseSetting, SQLITE_DEPRECATED};
use portitor_runtime::directory::{Directory, WebDirectory};
use portitor_runtime::health;
use portitor_runtime::management::management_port;
use portitor_runtime::request_size::RequestLimits;
use portitor_runtime::serve::{
    MAX_CONCURRENT_REQUESTS, REQUEST_TIMEOUT, run_component, serve, with_limits,
};
use portitor_runtime::settings;
use portitor_runtime::used_tokens::UsedTokenStore;

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
    let sender = portitor_connector::dsp_sender(config.allow_http)?;
    if config.allow_http {
        tracing::warn!("plain http allowed for every outgoing call (PORTITOR_ALLOW_HTTP)");
    }
    // one database for every store, shared with the readiness check
    let database = open_database(&config.database)?;
    let store = Arc::new(ConnectorStore::new(Arc::clone(&database)));
    #[cfg(feature = "tck")]
    let sender = portitor_connector::tck::LateStart(sender);
    let mut ports = portitor_connector::Ports::new(Arc::new(sender), &store);
    ports.object_storage = object_storage()?;
    let settings = AuthSettings::from_env(&config.participant_id, config.allow_http)?;
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
    // the callers' token ids go into the connector's database (TD-09)
    let used_token_ids = Arc::clone(&store) as Arc<dyn UsedTokenStore>;
    (ports.verifier, ports.prover) = auth_adapters(settings, used_token_ids).await?;
    let bundle = start_bundle_harvest(&config, &mut ports, &store).await?;
    // `build` runs the first harvest before it returns: blocking work (a
    // file read, the database, SHACL) on this async task, done once at start
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
        // the other processes of the connector may take a new version:
        // this one moves to it (TD-55)
        let _follow = harvester.follow_database(Arc::clone(&listener));
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
    // messages saved but not acknowledged before the last stop, and
    // pushes that should run, that no other process leases
    match negotiator.resume() {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "sending pending negotiation messages again"),
        Err(failure) => return Err(format!("cannot read pending messages: {failure:?}").into()),
    }
    // then the same every second, until the process stops: what another
    // process left, once its leases have ended (ADR 0029, TD-53, TD-54)
    let _outbox = negotiator.schedule_outbox(portitor_connector::OUTBOX_LOOP_EVERY);
    // data plane steps left over by a process that stopped (TD-48); runs
    // until the process stops, as the harvest schedule does
    let _sweep = negotiator.schedule_transfer_sweep(portitor_connector::TRANSFER_SWEEP_EVERY);
    let bind = settings::text_from_env("PORTITOR_BIND")
        .unwrap_or_else(|| portitor_connector::DEFAULT_BIND.to_owned());
    let listener = tokio::net::TcpListener::bind(&bind).await?;

    // the management port always serves the health checks (ADR 0023);
    // the management API only with its key
    let checks = Arc::new(readiness_checks(Dependencies {
        database,
        db_file: sqlite_file(&config.database),
        negotiator: negotiator.clone(),
        bundle: bundle_harvester,
        identity: identity_check,
    }));
    let settings = ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND);
    let (management_listener, management_app) = management_port(&settings, checks, |key| {
        portitor_connector::management::router(negotiator.clone(), key)
    })
    .await?;
    // the same time limit and number of requests at a time as the public
    // port: a call for a provider's catalog waits for that provider
    let management_app = limits.on_management_port(management_app);
    let management_app = with_limits(management_app, REQUEST_TIMEOUT, MAX_CONCURRENT_REQUESTS);
    tracing::info!(
        %bind,
        datasets = negotiator.catalog_size().map_err(|e| e.0)?,
        db = %database_name(&config.database),
        // the lease holder in the database (TD-48, ADR 0029): which
        // process this is
        process = portitor_connector::system::process_id(),
        "portitor-connector listening"
    );
    // the public routes: anyone can call them. The size limit is inside
    // the time limit, so a slow body counts against the time too
    let app = limits.on_public_port(app);
    let app = with_limits(app, REQUEST_TIMEOUT, MAX_CONCURRENT_REQUESTS);
    // push transfers received (ADR 0027): outside both limits, as a
    // dataset may be large and slow; the token is checked first, and the
    // route has limits of its own (`receiving.rs`)
    let app = app.merge(negotiator.receiving_routes());
    serve((listener, app), Some((management_listener, management_app))).await?;
    Ok(())
}

/// Opens the connector's database and brings its schema up to date. A
/// `SQLite` file is deprecated since 1.3 (ADR 0029): the log says so
/// once, at start.
fn open_database(setting: &DatabaseSetting) -> Result<Arc<Database>, String> {
    if matches!(setting, DatabaseSetting::SqliteFile(_)) {
        tracing::warn!("{SQLITE_DEPRECATED}");
    }
    match Database::open(setting, &MIGRATIONS) {
        Ok(database) => Ok(Arc::new(database)),
        Err(e) => Err(format!("cannot open {}: {e}", database_name(setting))),
    }
}

/// The `SQLite` file of `setting`, if it names one.
fn sqlite_file(setting: &DatabaseSetting) -> Option<std::path::PathBuf> {
    match setting {
        DatabaseSetting::SqliteFile(file) => Some(file.clone()),
        DatabaseSetting::Postgres(_) => None,
    }
}

/// The database, for the log: the file, or the URL without its query
/// (it never holds a password).
fn database_name(setting: &DatabaseSetting) -> String {
    match setting {
        DatabaseSetting::SqliteFile(file) => file.display().to_string(),
        DatabaseSetting::Postgres(settings) => settings.shown().to_owned(),
    }
}

/// The object storage of the `AWS_*` settings and `PORTITOR_PUSH_BUCKET`
/// (ADR 0027), if they are set. The log names the endpoint and the
/// bucket, never the keys.
fn object_storage() -> Result<Option<Arc<portitor_dataplane::object_storage::ObjectStorage>>, String>
{
    use portitor_dataplane::object_storage::{ObjectStorage, Settings};
    let Some(settings) = Settings::from_env()? else {
        return Ok(None);
    };
    tracing::info!(
        endpoint = %settings.endpoint,
        push_bucket = settings.push_bucket.as_deref().unwrap_or("none"),
        "object storage"
    );
    Ok(Some(Arc::new(ObjectStorage::new(settings)?)))
}

/// Takes the authority's signed bundle (ADR 0005), or the version recorded
/// in `database` when another process of the connector took a newer one
/// (TD-55), and applies its rules to `ports` through the bundle
/// harvester's copy: the SHACL check of each harvested dataset and the
/// guard on DSP messages. No bundle
/// configured: nothing changes (no SHACL checks, no message guard).
///
/// # Errors
/// A bundle is configured but no version can be used, or its shapes
/// cannot be read: the start stops, as for a bad signature.
async fn start_bundle_harvest(
    config: &portitor_connector::Config,
    ports: &mut portitor_connector::Ports,
    database: &Arc<ConnectorStore>,
) -> Result<
    Option<(BundleHarvester, portitor_connector::harvest::JobRecord)>,
    Box<dyn std::error::Error>,
> {
    let default_cache_dir = portitor_connector::default_bundle_cache_dir(&config.database);
    let Some(bundle_config) = BundleConfig::from_env(&default_cache_dir, config.allow_http)? else {
        return Ok(None);
    };
    // the authority's DID document, reused as long as every other one
    let did_cache_seconds = portitor_runtime::did::cache_seconds_from_env()?;
    let directory: Arc<dyn Directory> = Arc::new(WebDirectory::new(
        bundle_config.allow_http,
        did_cache_seconds,
    )?);
    let source = source_for(&bundle_config, directory)?;
    let store = Arc::new(CacheFolder::new(&bundle_config.cache_dir));
    // the version in use is the connector's, in its database (TD-55)
    let shared = Arc::clone(database) as Arc<dyn SharedBundleVersion>;
    let started = BundleHarvester::start(bundle_config, source, store, shared).await?;
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
async fn auth_adapters(
    settings: Option<AuthSettings>,
    used_token_ids: Arc<dyn UsedTokenStore>,
) -> Result<AuthAdapters, String> {
    let settings = settings.ok_or(
        "PORTITOR_STS_URL is required: DSP requests are authenticated with DCP (ADR 0016)",
    )?;
    dcp_adapters(&settings, used_token_ids).await
}

/// The DSP TCK build: the fixed header in `PORTITOR_TCK_AUTHORIZATION`
/// stands for the TCK, in both directions; DCP for everyone else when
/// `PORTITOR_STS_URL` is set (see `portitor_connector::tck`).
#[cfg(feature = "tck")]
async fn auth_adapters(
    settings: Option<AuthSettings>,
    used_token_ids: Arc<dyn UsedTokenStore>,
) -> Result<AuthAdapters, String> {
    use portitor_connector::auth::{NoTokens, RefuseAll};
    use portitor_connector::tck::{TckProver, TckVerifier};
    let (verifier, prover): AuthAdapters = if let Some(settings) = settings {
        dcp_adapters(&settings, used_token_ids).await?
    } else {
        tracing::warn!(
            "TCK build without PORTITOR_STS_URL: only the TCK's fixed header is accepted"
        );
        (Arc::new(RefuseAll), Arc::new(NoTokens))
    };
    let fixed = settings::text_from_env("PORTITOR_TCK_AUTHORIZATION");
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
