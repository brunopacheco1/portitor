// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Health checks (ADR 0023, section 1), shared by the three binaries.
//!
//! Each component serves Eclipse EDC's Observability API on its
//! management port, without the API key: `GET /api/check/liveness`,
//! `/readiness`, `/startup` and `/health`. Each answers HTTP 200 when
//! every check passes and 503 otherwise, with EDC's `HealthStatus` body
//! ([`HealthStatus`]). See EDC's
//! `extensions/common/api/api-observability/README.md`, and
//! `HealthStatus.java`, `HealthCheckResult.java` (package
//! `org.eclipse.edc.spi.system.health`) and `Failure.java`
//! (`org.eclipse.edc.spi.result`) in `spi/core-spi`.
//!
//! - Liveness checks nothing but the process: if it answers, it lives.
//! - Readiness runs the component's own checks ([`HealthChecks`]). Each
//!   component chooses them (its `SQLite` file, its key, ...), never a
//!   call to another participant.
//! - Startup and health are the same as readiness, as in EDC.
//!
//! A failed check names itself and says why, never a secret: the
//! management port faces the operator's network, but the text still ends
//! up in logs and dashboards.
//!
//! [`probe`] is the client side: the `health` subcommand of each binary,
//! for Docker's `HEALTHCHECK` (the images have no shell and no curl).

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::database::{Database, DatabaseSetting};
use utoipa::{OpenApi, ToSchema};

use crate::BoxFuture;

/// The start of every check path below (EDC's Observability API). The
/// request spans and measures leave these paths out
/// (`telemetry::http`).
pub const CHECK_PREFIX: &str = "/api/check/";

/// `GET` path of the liveness check.
pub const LIVENESS_PATH: &str = "/api/check/liveness";
/// `GET` path of the readiness check.
pub const READINESS_PATH: &str = "/api/check/readiness";
/// `GET` path of the startup check (the same as readiness).
pub const STARTUP_PATH: &str = "/api/check/startup";
/// `GET` path of the health check (the same as readiness).
pub const HEALTH_PATH: &str = "/api/check/health";

/// How long one check may take before it counts as failed. All checks
/// run at the same time, so a whole probe answers in about this time at
/// worst: well under the 1 s that Kubernetes waits by default.
pub const CHECK_TIMEOUT: Duration = Duration::from_millis(500);

/// How long the `health` subcommand ([`probe`]) waits for the answer.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The answer of every check route: EDC's `HealthStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthStatus {
    /// Whether every component is healthy. False when there is no
    /// component at all, as in EDC.
    pub is_system_healthy: bool,
    /// One result per check.
    pub component_results: Vec<ComponentResult>,
}

/// The result of one check: EDC's `HealthCheckResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentResult {
    /// What was checked, for example `database`.
    pub component: String,
    /// Whether the check passed.
    pub is_healthy: bool,
    /// Why it failed; `null` when it passed (always present).
    #[schema(required = true)]
    pub failure: Option<Failure>,
}

/// Why a check failed: EDC's `Failure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    /// What went wrong. Never a secret, key or token.
    pub messages: Vec<String>,
    /// The messages joined with `", "`, as EDC's `getFailureDetail()`.
    pub failure_detail: String,
}

impl HealthStatus {
    /// The status of `results`: healthy when there is at least one
    /// result and every one is healthy.
    #[must_use]
    pub fn of(results: Vec<ComponentResult>) -> Self {
        let all_healthy = results.iter().all(|result| result.is_healthy);
        Self {
            is_system_healthy: !results.is_empty() && all_healthy,
            component_results: results,
        }
    }
}

impl ComponentResult {
    /// The result of the check named `component`: `Ok` passed, `Err` holds
    /// the reason.
    #[must_use]
    pub fn new(component: &str, outcome: Result<(), String>) -> Self {
        match outcome {
            Ok(()) => Self {
                component: component.to_owned(),
                is_healthy: true,
                failure: None,
            },
            Err(reason) => Self {
                component: component.to_owned(),
                is_healthy: false,
                failure: Some(Failure {
                    failure_detail: reason.clone(),
                    messages: vec![reason],
                }),
            },
        }
    }
}

/// Port: one readiness check of a component. `Ok(())` when it passes,
/// otherwise why not (no secrets in it).
///
/// The check may wait on I/O; it is cut off after [`CHECK_TIMEOUT`]. A
/// check that only blocks (a `SQLite` query, a signature) is easier to
/// write with [`blocking`].
pub trait Check: Send + Sync {
    /// Runs the check.
    fn check(&self) -> BoxFuture<'_, Result<(), String>>;
}

/// A check that runs `work`, a function that blocks (`SQLite`, signing),
/// on tokio's thread pool for blocking work, so it does not hold up the
/// server's own threads.
pub fn blocking<F>(work: F) -> Arc<dyn Check>
where
    // `Send + Sync`: other threads run it; `'static`: it borrows nothing
    // that could end before it (it owns what it needs, often through
    // `move`), since the check is kept for as long as the server runs
    F: Fn() -> Result<(), String> + Send + Sync + 'static,
{
    Arc::new(Blocking(Arc::new(work)))
}

/// The adapter behind [`blocking`]. The function sits in an `Arc` so
/// that each run can hand a pointer to it to another thread.
struct Blocking(Arc<dyn Fn() -> Result<(), String> + Send + Sync>);

impl Check for Blocking {
    fn check(&self) -> BoxFuture<'_, Result<(), String>> {
        let work = Arc::clone(&self.0);
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || work()).await {
                Ok(outcome) => outcome,
                // the function panicked: the check fails, the server goes on
                Err(_) => Err("the check stopped unexpectedly".to_owned()),
            }
        })
    }
}

/// The readiness checks of one component, each with its name, in the
/// order they are shown.
#[derive(Default)]
pub struct HealthChecks {
    checks: Vec<Named>,
}

/// One check of [`HealthChecks`].
struct Named {
    name: String,
    check: Arc<dyn Check>,
    /// Whether the check passed the last time it ran; `None` before the
    /// first run. Only a change is logged (see [`change_to_log`]), not every
    /// probe: orchestrators probe every few seconds.
    last_healthy: std::sync::Mutex<Option<bool>>,
}

impl std::fmt::Debug for HealthChecks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self
            .checks
            .iter()
            .map(|named| named.name.as_str())
            .collect();
        f.debug_struct("HealthChecks")
            .field("checks", &names)
            .finish()
    }
}

impl HealthChecks {
    /// No checks yet: readiness answers 503 until one is added (EDC's
    /// rule for an empty list).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the check `name`.
    pub fn add(&mut self, name: &str, check: Arc<dyn Check>) {
        self.checks.push(Named {
            name: name.to_owned(),
            check,
            last_healthy: std::sync::Mutex::new(None),
        });
    }

    /// Runs every check at the same time, each cut off after `timeout`,
    /// and collects the results in the order the checks were added.
    pub async fn run(&self, timeout: Duration) -> HealthStatus {
        // start every check on its own task first ...
        let mut running = Vec::new();
        for named in &self.checks {
            let check = Arc::clone(&named.check);
            let task = tokio::spawn(async move {
                match tokio::time::timeout(timeout, check.check()).await {
                    Ok(outcome) => outcome,
                    Err(_) => Err(format!("no answer within {} ms", timeout.as_millis())),
                }
            });
            running.push((named, task));
        }
        // ... then wait for each in turn: the slowest sets the time
        let mut results = Vec::new();
        for (named, task) in running {
            let outcome = match task.await {
                Ok(outcome) => outcome,
                Err(_) => Err("the check stopped unexpectedly".to_owned()),
            };
            named.log_change(&outcome);
            results.push(ComponentResult::new(&named.name, outcome));
        }
        HealthStatus::of(results)
    }
}

impl Named {
    /// Logs `outcome` when it differs from the check's last one: a WARN
    /// when the check starts failing, an INFO when it passes again.
    fn log_change(&self, outcome: &Result<(), String>) {
        let healthy = outcome.is_ok();
        // a poisoned lock only means a panic while it was held; the
        // `Option<bool>` in it is still valid
        let mut last = self
            .last_healthy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = last.replace(healthy);
        match (change_to_log(previous, healthy), outcome) {
            (Some(Change::Failed), Err(reason)) => {
                tracing::warn!(component = %self.name, %reason, "readiness check failed");
            }
            (Some(Change::Recovered), _) => {
                tracing::info!(component = %self.name, "readiness check passes again");
            }
            _ => {}
        }
    }
}

/// A change of a check's result worth a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// It fails now, and did not before (or never ran).
    Failed,
    /// It passes now, and failed before.
    Recovered,
}

/// The change from `previous` (`None`: the check never ran) to `healthy`,
/// if it is one to log. A first run that passes is not logged.
fn change_to_log(previous: Option<bool>, healthy: bool) -> Option<Change> {
    match (previous, healthy) {
        (Some(false), true) => Some(Change::Recovered),
        (None | Some(true), false) => Some(Change::Failed),
        _ => None,
    }
}

/// The liveness answer: one component, `process`, always healthy.
#[must_use]
pub fn liveness_status() -> HealthStatus {
    HealthStatus::of(vec![ComponentResult::new("process", Ok(()))])
}

/// 200 with the status when the system is healthy, 503 otherwise.
fn respond(status: HealthStatus) -> Response {
    let code = if status.is_system_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(status)).into_response()
}

/// The process answers: it lives. Checks nothing else.
#[utoipa::path(
    get,
    path = "/api/check/liveness",
    responses(
        (status = 200, description = "The process runs", body = HealthStatus),
    ),
    tag = "Health"
)]
pub async fn liveness() -> Response {
    respond(liveness_status())
}

/// Whether the component can serve: its own dependencies only (its
/// database, its key, ...), never other participants.
#[utoipa::path(
    get,
    path = "/api/check/readiness",
    responses(
        (status = 200, description = "Every check passed", body = HealthStatus),
        (status = 503, description = "A check failed; its failure says why", body = HealthStatus),
    ),
    tag = "Health"
)]
pub async fn readiness(State(checks): State<Arc<HealthChecks>>) -> Response {
    respond(checks.run(CHECK_TIMEOUT).await)
}

/// Whether the component has started: the same checks as readiness.
#[utoipa::path(
    get,
    path = "/api/check/startup",
    responses(
        (status = 200, description = "Every check passed", body = HealthStatus),
        (status = 503, description = "A check failed; its failure says why", body = HealthStatus),
    ),
    tag = "Health"
)]
pub async fn startup(state: State<Arc<HealthChecks>>) -> Response {
    readiness(state).await
}

/// The component's health: the same checks as readiness.
#[utoipa::path(
    get,
    path = "/api/check/health",
    responses(
        (status = 200, description = "Every check passed", body = HealthStatus),
        (status = 503, description = "A check failed; its failure says why", body = HealthStatus),
    ),
    tag = "Health"
)]
pub async fn health(state: State<Arc<HealthChecks>>) -> Response {
    readiness(state).await
}

/// The `OpenAPI` description of the four check routes, the same in each
/// component: merged into each management API's description
/// ([`with_health_api`]).
#[derive(OpenApi)]
#[openapi(
    paths(liveness, readiness, startup, health),
    components(schemas(HealthStatus, ComponentResult, Failure)),
    tags(
        (name = "Health", description = "Health checks in Eclipse EDC's Observability API shape (ADR 0023). They need no API key, and neither does GET /health (plain `ok`). Served even when the management API is off.")
    )
)]
pub struct HealthApi;

/// `doc` with the check routes, their schemas and the `Health` tag of
/// [`HealthApi`] added.
#[must_use]
pub fn with_health_api(mut doc: utoipa::openapi::OpenApi) -> utoipa::openapi::OpenApi {
    doc.merge(HealthApi::openapi());
    doc
}

/// The four check routes, open (no API key).
pub fn router(checks: Arc<HealthChecks>) -> Router {
    Router::new()
        .route(LIVENESS_PATH, get(liveness))
        .route(READINESS_PATH, get(readiness))
        .route(STARTUP_PATH, get(startup))
        .route(HEALTH_PATH, get(health))
        .with_state(checks)
}

/// What the management port serves: the four check routes, and the
/// management API (`api`, behind its key) when it is on.
pub fn management_app(checks: Arc<HealthChecks>, api: Option<Router>) -> Router {
    match api {
        Some(api) => router(checks).merge(api),
        None => router(checks),
    }
}

/// Check: the `SQLite` file at `path` answers. Opens it read only, apart
/// from the store's own connection, and reads its list of tables: the
/// file is there, readable, and a `SQLite` database. Blocking.
///
/// # Errors
/// The file cannot be opened or read; the text says why, without the
/// path.
pub fn sqlite_file_answers(path: &Path) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags};
    // read only, and never create the file: a missing file is a failure
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| format!("the SQLite file cannot be opened: {e}"))?;
    // the store's writes do not block a reader (WAL); wait a little anyway
    conn.busy_timeout(Duration::from_millis(200))
        .map_err(|e| format!("the SQLite file does not answer: {e}"))?;
    // the number itself does not matter: reading it proves the file is
    // a database that answers
    let _table_count: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .map_err(|e| format!("the SQLite file does not answer: {e}"))?;
    Ok(())
}

/// Check: the component's database answers. A `SQLite` file is checked
/// as [`sqlite_file_answers`] does, apart from the store's connection;
/// PostgreSQL answers a `SELECT 1` on a connection of the pool. Blocking.
///
/// # Errors
/// The database cannot be reached or does not answer; the text says why.
pub fn database_answers(setting: &DatabaseSetting, db: &Database) -> Result<(), String> {
    use crate::database::Executor as _;
    match setting {
        DatabaseSetting::SqliteFile(path) => sqlite_file_answers(path),
        DatabaseSetting::Postgres(_) => db
            .query("SELECT 1", &[])
            .map(|_rows| ())
            .map_err(|e| format!("the database does not answer: {e}")),
    }
}

/// Check: `key` signs, and its signature verifies with its public key:
/// the key is loaded and usable. Signs a fixed test payload that is never
/// sent anywhere. Blocking (a fraction of a millisecond).
///
/// # Errors
/// Signing or verifying fails.
pub fn key_signs(key: &portitor_dcp::SigningKey) -> Result<(), String> {
    let payload = serde_json::json!({"purpose": "readiness check"});
    let jws = key
        .sign_json("readiness-check", "JWT", &payload)
        .map_err(|e| format!("the key cannot sign: {e}"))?;
    signed_by(&jws, &key.public_jwk())
}

/// Check: `jws` is a compact JWS signed by the key `public`.
///
/// # Errors
/// It is not a JWS, or not signed by that key.
pub fn signed_by(jws: &str, public: &portitor_dcp::Jwk) -> Result<(), String> {
    let parsed = portitor_dcp::Jws::parse(jws).map_err(|e| format!("not a signed JWT: {e}"))?;
    parsed
        .verify(public)
        .map_err(|e| format!("not signed by the component's key: {e}"))
}

/// The address to call for a server bound to `bind`: the same, except
/// that "every address" (`0.0.0.0`, `[::]`) becomes this machine
/// (`127.0.0.1`, `[::1]`).
#[must_use]
pub fn local_address(bind: &str) -> String {
    match bind.parse::<std::net::SocketAddr>() {
        Ok(mut address) => {
            if address.ip().is_unspecified() {
                let here = match address {
                    std::net::SocketAddr::V4(_) => std::net::IpAddr::from([127, 0, 0, 1]),
                    std::net::SocketAddr::V6(_) => {
                        std::net::IpAddr::from(std::net::Ipv6Addr::LOCALHOST)
                    }
                };
                address.set_ip(here);
            }
            address.to_string()
        }
        // a host name, such as localhost:8081: call it as it is
        Err(_) => bind.to_owned(),
    }
}

/// The `health` subcommand: calls `GET /api/check/readiness` on the
/// management server bound to `management_bind`, prints the answer, and
/// exits 0 on HTTP 200, 1 otherwise (no answer within [`PROBE_TIMEOUT`]
/// included). Starts its own small tokio runtime.
#[must_use]
pub fn probe(management_bind: &str) -> ExitCode {
    let url = format!("http://{}{READINESS_PATH}", local_address(management_bind));
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("health: cannot start: {error}");
            return ExitCode::FAILURE;
        }
    };
    // the timer is made inside the runtime, which drives it
    let call = async { tokio::time::timeout(PROBE_TIMEOUT, get_status(&url)).await };
    match runtime.block_on(call) {
        Ok(Ok((200, body))) => {
            println!("{body}");
            ExitCode::SUCCESS
        }
        Ok(Ok((status, body))) => {
            eprintln!("health: {url} answered HTTP {status}: {body}");
            ExitCode::FAILURE
        }
        Ok(Err(error)) => {
            eprintln!("health: {url}: {error}");
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!(
                "health: {url}: no answer within {} s",
                PROBE_TIMEOUT.as_secs()
            );
            ExitCode::FAILURE
        }
    }
}

/// The largest answer [`probe`] reads.
const MAX_PROBE_ANSWER: usize = 64 * 1024;

/// `GET url` over plain HTTP (the management port has no TLS): the status
/// code and the body as text.
async fn get_status(url: &str) -> Result<(u16, String), String> {
    use http_body_util::{BodyExt as _, Empty};
    use hyper::body::Bytes;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    // the hyper client the binaries use, without TLS
    let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
    let uri: hyper::Uri = url.parse().map_err(|e| format!("invalid address: {e}"))?;
    let response = client.get(uri).await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = http_body_util::Limited::new(response.into_body(), MAX_PROBE_ANSWER)
        .collect()
        .await
        .map_err(|e| format!("answer not read: {e}"))?
        .to_bytes();
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    /// A check that always gives `outcome`.
    struct Fixed(Result<(), String>);

    impl Check for Fixed {
        fn check(&self) -> BoxFuture<'_, Result<(), String>> {
            let outcome = self.0.clone();
            Box::pin(async move { outcome })
        }
    }

    /// A check that never ends.
    struct Stuck;

    impl Check for Stuck {
        fn check(&self) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(std::future::pending())
        }
    }

    fn checks(list: Vec<(&str, Arc<dyn Check>)>) -> Arc<HealthChecks> {
        let mut checks = HealthChecks::new();
        for (name, check) in list {
            checks.add(name, check);
        }
        Arc::new(checks)
    }

    async fn get(app: &Router, path: &str) -> (StatusCode, Value) {
        let request = Request::get(path).body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn all_pass_200_in_edcs_shape() {
        let app = router(checks(vec![
            ("database", Arc::new(Fixed(Ok(())))),
            ("key", blocking(|| Ok(()))),
        ]));
        for path in [READINESS_PATH, STARTUP_PATH, HEALTH_PATH] {
            let (status, body) = get(&app, path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(
                body,
                json!({
                    "isSystemHealthy": true,
                    "componentResults": [
                        {"component": "database", "isHealthy": true, "failure": null},
                        {"component": "key", "isHealthy": true, "failure": null}
                    ]
                }),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn one_failure_503_and_says_which_and_why() {
        let app = router(checks(vec![
            ("database", Arc::new(Fixed(Ok(())))),
            ("key", blocking(|| Err("no key loaded".to_owned()))),
        ]));
        for path in [READINESS_PATH, STARTUP_PATH, HEALTH_PATH] {
            let (status, body) = get(&app, path).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(body["isSystemHealthy"], false);
            assert_eq!(body["componentResults"][0]["isHealthy"], true);
            assert_eq!(
                body["componentResults"][1],
                json!({"component": "key", "isHealthy": false,
                       "failure": {"messages": ["no key loaded"], "failureDetail": "no key loaded"}})
            );
        }
        // liveness checks nothing but the process
        let (status, body) = get(&app, LIVENESS_PATH).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"isSystemHealthy": true,
                   "componentResults": [{"component": "process", "isHealthy": true, "failure": null}]})
        );
    }

    #[tokio::test]
    async fn no_checks_is_not_healthy_as_in_edc() {
        let (status, body) = get(&router(checks(vec![])), READINESS_PATH).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            json!({"isSystemHealthy": false, "componentResults": []})
        );
    }

    /// The reasons of the failed checks of `status`.
    fn reasons(status: &HealthStatus) -> Vec<String> {
        status
            .component_results
            .iter()
            .filter_map(|r| r.failure.as_ref())
            .map(|f| f.failure_detail.clone())
            .collect()
    }

    #[tokio::test]
    async fn a_slow_check_fails_in_time() {
        let checks = checks(vec![
            ("stuck", Arc::new(Stuck)),
            ("ok", Arc::new(Fixed(Ok(())))),
        ]);
        let started = std::time::Instant::now();
        let status = checks.run(Duration::from_millis(50)).await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!status.is_system_healthy);
        assert_eq!(reasons(&status), ["no answer within 50 ms"]);
    }

    #[tokio::test]
    async fn a_panicking_check_fails_and_the_server_goes_on() {
        let checks = checks(vec![("panics", blocking(|| panic!("test-only panic")))]);
        // a long limit: printing the panic may be slow on a busy machine
        let status = checks.run(Duration::from_secs(10)).await;
        assert!(!status.is_system_healthy);
        assert_eq!(reasons(&status), ["the check stopped unexpectedly"]);
    }

    #[tokio::test]
    async fn without_the_api_only_the_checks_are_served() {
        let api = Router::new().route("/v1/things", axum::routing::get(|| async { "things" }));
        let open = management_app(checks(vec![]), None);
        let full = management_app(checks(vec![]), Some(api));
        let status = |app: &Router, path: &str| {
            let request = Request::get(path).body(Body::empty()).unwrap();
            let app = app.clone();
            async move { app.oneshot(request).await.unwrap().status() }
        };
        assert_eq!(status(&open, LIVENESS_PATH).await, StatusCode::OK);
        assert_eq!(status(&open, "/v1/things").await, StatusCode::NOT_FOUND);
        assert_eq!(status(&open, "/health").await, StatusCode::NOT_FOUND);
        assert_eq!(status(&full, LIVENESS_PATH).await, StatusCode::OK);
        assert_eq!(status(&full, "/v1/things").await, StatusCode::OK);
    }

    #[test]
    fn the_sqlite_file_must_be_there_and_a_database() {
        let dir = std::env::temp_dir().join(format!("portitor-health-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("store.db");
        let _store = crate::sqlite::open(Some(&file), &["CREATE TABLE a (x TEXT)"]).unwrap();
        assert_eq!(sqlite_file_answers(&file), Ok(()));
        // not a database
        let other = dir.join("not.db");
        std::fs::write(&other, "test-only: not a SQLite file, but long enough to have a header ......................................................").unwrap();
        let err = sqlite_file_answers(&other).unwrap_err();
        assert!(err.contains("does not answer"), "{err}");
        // gone: never created again
        let gone = dir.join("gone.db");
        let err = sqlite_file_answers(&gone).unwrap_err();
        assert!(err.contains("cannot be opened"), "{err}");
        assert!(!gone.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_loaded_key_signs_and_others_do_not_verify() {
        let key = portitor_dcp::SigningKey::generate().unwrap();
        assert_eq!(key_signs(&key), Ok(()));
        let other = portitor_dcp::SigningKey::generate().unwrap();
        let jws = other.sign_json("k", "JWT", &serde_json::json!({})).unwrap();
        let err = signed_by(&jws, &key.public_jwk()).unwrap_err();
        assert!(err.contains("not signed by"), "{err}");
        assert!(signed_by("", &key.public_jwk()).is_err());
    }

    #[test]
    fn only_changes_are_logged() {
        assert_eq!(change_to_log(None, true), None);
        assert_eq!(change_to_log(None, false), Some(Change::Failed));
        assert_eq!(change_to_log(Some(true), false), Some(Change::Failed));
        assert_eq!(change_to_log(Some(false), false), None);
        assert_eq!(change_to_log(Some(false), true), Some(Change::Recovered));
        assert_eq!(change_to_log(Some(true), true), None);
    }

    #[test]
    fn every_check_path_has_the_prefix() {
        for path in [LIVENESS_PATH, READINESS_PATH, STARTUP_PATH, HEALTH_PATH] {
            assert!(path.starts_with(CHECK_PREFIX), "{path}");
        }
    }

    #[test]
    fn every_address_is_called_on_this_machine() {
        assert_eq!(local_address("0.0.0.0:8081"), "127.0.0.1:8081");
        assert_eq!(local_address("[::]:8081"), "[::1]:8081");
        assert_eq!(local_address("127.0.0.1:8091"), "127.0.0.1:8091");
        assert_eq!(local_address("10.0.0.5:8071"), "10.0.0.5:8071");
        assert_eq!(local_address("localhost:8081"), "localhost:8081");
    }

    /// Serves `checks` on a port of this machine, from a thread with its
    /// own runtime (the probe makes its own), and returns the address.
    fn serve_in_background(checks: Arc<HealthChecks>) -> std::net::SocketAddr {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, router(checks)).await.unwrap();
            });
        });
        receiver.recv().unwrap()
    }

    #[test]
    fn the_probe_follows_the_status() {
        let healthy = serve_in_background(checks(vec![("ok", Arc::new(Fixed(Ok(()))))]));
        assert_eq!(probe(&healthy.to_string()), ExitCode::SUCCESS);
        let ill = serve_in_background(checks(vec![("ok", Arc::new(Fixed(Err("down".into()))))]));
        assert_eq!(probe(&ill.to_string()), ExitCode::FAILURE);
        // nobody listens there any more
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert_eq!(probe(&closed.to_string()), ExitCode::FAILURE);
    }

    /// A server that takes the connection and never answers: the probe
    /// gives up after its timeout and fails.
    #[test]
    fn the_probe_fails_in_time_when_nothing_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut open = Vec::new();
            // keep each connection open, unread and unanswered
            for socket in listener.incoming().flatten() {
                open.push(socket);
            }
        });
        let started = std::time::Instant::now();
        assert_eq!(probe(&address.to_string()), ExitCode::FAILURE);
        let took = started.elapsed();
        assert!(took >= PROBE_TIMEOUT, "{took:?}");
        assert!(took < PROBE_TIMEOUT + Duration::from_secs(2), "{took:?}");
    }
}
