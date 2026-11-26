// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! PostgreSQL behind the synchronous store ports (ADR 0029).
//!
//! The store ports are plain functions, called from request handlers
//! that run on tokio. `tokio-postgres` is asynchronous, so this module
//! bridges the two:
//!
//! - each connection's socket is driven by a task on a small runtime of
//!   its own, on its own thread (the "database thread");
//! - a store's call waits for its answer on the calling thread
//!   ([`block_on`]), the way it waited for the `SQLite` lock before.
//!
//! A client's query only passes messages to its connection task, so
//! waiting for it needs no tokio runtime on the calling thread.
//!
//! The calling thread is usually one of tokio's worker threads: a
//! request handler or a background loop calls a store port, which is a
//! plain function. The worker is then parked for the database's round
//! trip, and for the short sleep before a transaction that collided runs
//! again ([`TRIES`]); other tasks wait for another worker meanwhile.
//! Besides requests, two loops call the database from async code this
//! way: the connector's outbox loop (`take_up`, every second) and each
//! running download's or push's state watchdog in the data plane
//! (`flows.get`, every 5 seconds). The cost is measured and accepted for
//! now (TD-40; asynchronous store ports are planned for 1.12).
//!
//! Why not tokio's own ways to wait: `Handle::block_on` panics when
//! called from async code, which runs on a worker; `spawn_blocking` (or `block_in_place`) would need every caller
//! up to the request handler to be async, or to know that it is on a
//! worker, and the store ports are synchronous (TD-40). [`block_on`]
//! here only parks the calling thread until the database thread wakes it.

use std::cell::Cell;
use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;
use std::time::{Duration, Instant};

use tokio_postgres::types::{IsNull, ToSql, Type, to_sql_checked};
use tokio_postgres::{Client, Config};

use super::{DbError, Executor, Migrations, Row, Value};

/// How many times a transaction runs when PostgreSQL keeps answering
/// that it collided with another one.
pub const TRIES: u32 = 10;

/// Connections a process keeps open at most.
const POOL_SIZE: usize = 10;

/// How long a call waits for a free connection before it fails, as the
/// `SQLite` busy timeout did.
const WAIT_FOR_A_CONNECTION: Duration = Duration::from_secs(5);

/// How long connecting may take, unless the URL says otherwise
/// (`connect_timeout`, in seconds, as libpq reads it).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one statement may run on the server, unless the URL's
/// `options` set `statement_timeout`: as long as a lease
/// ([`crate::leases::LEASE_SECONDS`]), since a call that takes longer has
/// lost its lease anyway.
const STATEMENT_TIMEOUT: &str = "30s";

/// How long a call waits for the server's answer before it gives up and
/// drops the connection: the statement timeout and 10 seconds for the
/// network, so a slow statement ends with the server's own error first.
/// It ends the wait on a connection that died without closing (a
/// failover, a network cut), which would otherwise block the calling
/// thread for good.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(40);

/// How long sent data may stay unacknowledged before the connection is
/// closed, unless the URL says otherwise (`tcp_user_timeout`, in
/// milliseconds, as libpq reads it): a lease's length.
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a connection stays idle before TCP checks that the server is
/// still there, and how often it checks then, unless the URL says
/// otherwise (`keepalives_idle`, `keepalives_interval`, in seconds, as
/// libpq reads them). With [`TCP_USER_TIMEOUT`], a pooled connection to
/// a server that went away is closed within about 30 seconds, instead of
/// the system's two hours.
const KEEPALIVES_IDLE: Duration = Duration::from_secs(15);
const KEEPALIVES_INTERVAL: Duration = Duration::from_secs(5);

/// The lock every Portitor process takes while it migrates, so that
/// replicas that start together migrate one after the other: the text
/// "portitor" as a number.
const MIGRATION_LOCK: i64 = 0x706f_7274_6974_6f72;

/// What `PORTITOR_DATABASE_URL` and `PORTITOR_DATABASE_PASSWORD_FILE` say.
#[derive(Clone)]
pub struct Settings {
    config: Config,
    tls: Option<Arc<rustls::ClientConfig>>,
    /// The URL without its password, for logs and errors.
    shown: String,
    /// A test database, dropped once the last copy of these settings is
    /// gone ([`Settings::dropping_the_database`]).
    #[cfg(any(test, feature = "test-support"))]
    test_database: Option<Arc<TestDatabase>>,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("url", &self.shown)
            .finish_non_exhaustive()
    }
}

/// Equal when they connect the same way: the same connection settings,
/// URL, and TLS on or off.
impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.config == other.config
            && self.shown == other.shown
            && self.tls.is_some() == other.tls.is_some()
    }
}

impl Eq for Settings {}

impl Settings {
    /// Reads a PostgreSQL connection URI
    /// (`postgresql://user@host:5432/database?sslmode=verify-full`, the
    /// form libpq reads) and the password from its file.
    ///
    /// TLS: `sslmode` may be `verify-full` (the default when it is
    /// missing), `verify-ca` or `require`, which all mean the same here:
    /// TLS, with the server's certificate and host name checked. The
    /// certificate must chain to a CA of `sslrootcert`, as in libpq: a PEM
    /// file, whose CAs are then the only ones trusted, or `system`, the
    /// system's CA certificates. Without `sslrootcert` the system's CA
    /// certificates are used (libpq would read `~/.postgresql/root.crt`;
    /// a container has none). `disable` turns TLS off. `prefer` and
    /// `allow` are refused: they fall back to an unencrypted connection
    /// without saying so.
    ///
    /// Timeouts, each as libpq reads it from the URL, with Portitor's
    /// default when the URL does not set it: `connect_timeout` (seconds,
    /// default 10), `tcp_user_timeout` (milliseconds, default 30000),
    /// `keepalives_idle` and `keepalives_interval` (seconds, defaults 15
    /// and 5), and `statement_timeout` in `options`
    /// (`options=-c%20statement_timeout%3D60s`, default 30 s).
    ///
    /// # Errors
    /// The URL is not a `postgresql://` URI, holds a password (it goes in
    /// `PORTITOR_DATABASE_PASSWORD_FILE`), has an `sslmode` named above as
    /// refused, names no user, host or database, or `sslrootcert` cannot
    /// be read.
    pub fn from_url(url: &str, password: Option<String>) -> Result<Self, String> {
        let (base, own) = split_own_options(url)?;
        let mut config: Config = base
            .parse()
            .map_err(|e| format!("PORTITOR_DATABASE_URL: {e}"))?;
        if config.get_password().is_some() {
            return Err("PORTITOR_DATABASE_URL holds a password; put it in \
                 PORTITOR_DATABASE_PASSWORD_FILE"
                .to_owned());
        }
        if config.get_user().is_none() || config.get_dbname().is_none() {
            return Err("PORTITOR_DATABASE_URL must name a user and a database".to_owned());
        }
        if config.get_hosts().is_empty() {
            return Err("PORTITOR_DATABASE_URL must name a host".to_owned());
        }
        if let Some(password) = password {
            config.password(password);
        }
        config.application_name("portitor");
        set_timeouts(&mut config, &base, own.tcp_user_timeout);
        let tls = if own.disable {
            config.ssl_mode(tokio_postgres::config::SslMode::Disable);
            None
        } else {
            config.ssl_mode(tokio_postgres::config::SslMode::Require);
            Some(super::postgres_tls::client_config(
                own.root_cert.as_deref(),
            )?)
        };
        Ok(Self {
            config,
            tls,
            shown: base_without_query(url),
            #[cfg(any(test, feature = "test-support"))]
            test_database: None,
        })
    }

    /// The URL without its query, for logs.
    #[must_use]
    pub fn shown(&self) -> &str {
        &self.shown
    }

    /// The password, for the settings tests of the binaries (it is not in
    /// the `Debug` text, so they cannot see otherwise that a new password
    /// file was read).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn password(&self) -> Option<&[u8]> {
        self.config.get_password()
    }

    /// These settings, which drop the test database `name` from the
    /// server of `admin` once the last copy of them (and every pool
    /// opened with them) is gone, so test runs leave no databases behind.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn dropping_the_database(mut self, admin: &Self, name: &str) -> Self {
        self.test_database = Some(Arc::new(TestDatabase {
            admin: admin.clone(),
            name: name.to_owned(),
        }));
        self
    }
}

/// A test database, dropped with the last settings that name it.
#[cfg(any(test, feature = "test-support"))]
struct TestDatabase {
    admin: Settings,
    name: String,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestDatabase {
    fn drop(&mut self) {
        // FORCE (PostgreSQL 13 and later): also when a connection of a
        // test is still open
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        let dropped = Pool::start(&self.admin).and_then(|pool| pool.execute(&sql, &[]));
        if let Err(e) = dropped {
            eprintln!("cannot drop the test database {}: {e}", self.name);
        }
    }
}

/// Sets each timeout the URL `base` does not set to Portitor's default.
/// `tcp_user_timeout` is what the URL said, read here in milliseconds as
/// libpq does (`tokio-postgres` would read seconds).
fn set_timeouts(config: &mut Config, base: &str, tcp_user_timeout: Option<Duration>) {
    if !has_parameter(base, "connect_timeout") {
        config.connect_timeout(CONNECT_TIMEOUT);
    }
    // 0 means "the system's default", as in libpq
    match tcp_user_timeout {
        None => {
            config.tcp_user_timeout(TCP_USER_TIMEOUT);
        }
        Some(t) if t.is_zero() => {}
        Some(t) => {
            config.tcp_user_timeout(t);
        }
    }
    if !has_parameter(base, "keepalives_idle") {
        config.keepalives_idle(KEEPALIVES_IDLE);
    }
    if !has_parameter(base, "keepalives_interval") {
        config.keepalives_interval(KEEPALIVES_INTERVAL);
    }
    let options = config.get_options().unwrap_or("").to_owned();
    if !options.contains("statement_timeout") {
        let ours = format!("-c statement_timeout={STATEMENT_TIMEOUT}");
        config.options(format!("{options} {ours}").trim());
    }
}

/// Whether the query of `url` has the parameter `name`.
fn has_parameter(url: &str, name: &str) -> bool {
    let query = url.split_once('?').map_or("", |(_, q)| q);
    query
        .split('&')
        .any(|pair| pair.split_once('=').map_or(pair, |(key, _)| key) == name)
}

/// The URL options this module reads itself: `tokio-postgres` knows only
/// the `sslmode`s `disable`, `prefer` and `require`, no `sslrootcert`,
/// and reads `tcp_user_timeout` in seconds where libpq reads
/// milliseconds.
struct OwnOptions {
    disable: bool,
    /// A PEM file, or `system`.
    root_cert: Option<String>,
    tcp_user_timeout: Option<Duration>,
}

/// The URL without `sslmode`, `sslrootcert` and `tcp_user_timeout`, and
/// what they said.
fn split_own_options(url: &str) -> Result<(String, OwnOptions), String> {
    if !(url.starts_with("postgresql://") || url.starts_with("postgres://")) {
        return Err("PORTITOR_DATABASE_URL must start with postgresql://".to_owned());
    }
    let mut options = OwnOptions {
        disable: false,
        root_cert: None,
        tcp_user_timeout: None,
    };
    let Some((base, query)) = url.split_once('?') else {
        return Ok((url.to_owned(), options));
    };
    let mut kept = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "sslmode" => match value {
                "verify-full" | "verify-ca" | "require" => {}
                "disable" => options.disable = true,
                other => {
                    return Err(format!(
                        "PORTITOR_DATABASE_URL: sslmode={other} is refused; use verify-full \
                         (or disable, only on a private network)"
                    ));
                }
            },
            "sslrootcert" => options.root_cert = Some(percent_decoded(value)?),
            "tcp_user_timeout" => {
                let millis: u64 = value.parse().map_err(|_| {
                    format!("PORTITOR_DATABASE_URL: tcp_user_timeout={value} is not milliseconds")
                })?;
                options.tcp_user_timeout = Some(Duration::from_millis(millis));
            }
            _ => kept.push(pair),
        }
    }
    let base = if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    Ok((base, options))
}

fn percent_decoded(value: &str) -> Result<String, String> {
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| format!("PORTITOR_DATABASE_URL: {e}"))
}

fn base_without_query(url: &str) -> String {
    url.split_once('?').map_or(url, |(base, _)| base).to_owned()
}

/// A pool of PostgreSQL connections, for one process.
pub struct Pool {
    // dropped in this order: the connections, the database thread, then
    // the settings (which may drop a test database)
    idle: Mutex<Idle>,
    returned: Condvar,
    /// The database thread's runtime, which drives the connections.
    runtime: tokio::runtime::Handle,
    /// Dropped with the pool: ends the database thread.
    _stop: tokio::sync::oneshot::Sender<()>,
    settings: Settings,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("database", &self.settings.shown)
            .finish_non_exhaustive()
    }
}

/// The connections not in use, and how many exist in all.
struct Idle {
    clients: Vec<Client>,
    open: usize,
}

impl Pool {
    /// Starts the database thread, connects, and brings the schema up to
    /// date under a lock that replicas share.
    ///
    /// # Errors
    /// The database cannot be reached, a migration fails, the schema is
    /// newer than this Portitor, or it belongs to another component.
    pub fn open(settings: &Settings, migrations: &Migrations) -> Result<Self, DbError> {
        let pool = Self::start(settings)?;
        pool.migrate(migrations)?;
        Ok(pool)
    }

    /// Starts the database thread; connections are opened when needed.
    fn start(settings: &Settings) -> Result<Self, DbError> {
        let (runtime, stop) = start_database_thread()?;
        Ok(Self {
            settings: settings.clone(),
            runtime,
            _stop: stop,
            idle: Mutex::new(Idle {
                clients: Vec::new(),
                open: 0,
            }),
            returned: Condvar::new(),
        })
    }

    /// Runs one statement and returns its rows.
    ///
    /// # Errors
    /// The database refused it or could not be reached.
    pub fn query(&self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let mut taken = self.take()?;
        let (client, timed_out) = taken.parts()?;
        PgRunner { client, timed_out }.query(sql, values)
    }

    /// Runs one statement and returns how many rows it changed.
    ///
    /// # Errors
    /// The database refused it or could not be reached.
    pub fn execute(&self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let mut taken = self.take()?;
        let (client, timed_out) = taken.parts()?;
        PgRunner { client, timed_out }.execute(sql, values)
    }

    /// [`super::Database::transaction`] on PostgreSQL.
    pub(super) fn transaction<T>(
        &self,
        work: &mut dyn FnMut(&dyn Executor) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let mut tries = 0;
        loop {
            tries += 1;
            let mut taken = self.take()?;
            let (client, timed_out) = taken.parts()?;
            match run_once(client, timed_out, work) {
                Err(e) if is_collision(&e) && tries < TRIES => {
                    // another transaction won; ours rolled back: run it again
                    std::thread::sleep(Duration::from_millis(5 * u64::from(tries)));
                }
                result => return result,
            }
        }
    }

    /// Applies the migrations not applied yet, in one transaction, while
    /// holding [`MIGRATION_LOCK`]: a replica that starts at the same time
    /// waits, then finds them applied.
    fn migrate(&self, migrations: &Migrations) -> Result<(), DbError> {
        let mut taken = self.take()?;
        let (client, _) = taken.parts()?;
        // no time limit: waiting for another replica's migrations, or
        // migrating a large database, may take longer than a statement may
        block_on(async {
            let tx = client.transaction().await.map_err(pg_error)?;
            tx.batch_execute("SET LOCAL statement_timeout = 0")
                .await
                .map_err(pg_error)?;
            tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])
                .await
                .map_err(pg_error)?;
            tx.batch_execute(
                "CREATE TABLE IF NOT EXISTS portitor_schema (
                    component TEXT NOT NULL,
                    version   BIGINT NOT NULL
                 )",
            )
            .await
            .map_err(pg_error)?;
            let stored = tx
                .query("SELECT component, version FROM portitor_schema", &[])
                .await
                .map_err(pg_error)?;
            let applied = match stored.as_slice() {
                [] => {
                    tx.execute(
                        "INSERT INTO portitor_schema (component, version) VALUES ($1, 0)",
                        &[&migrations.component],
                    )
                    .await
                    .map_err(pg_error)?;
                    0
                }
                [row] => {
                    let component: String = row.get(0);
                    if component != migrations.component {
                        return Err(DbError::new(format!(
                            "this database holds the schema of {component}; give {} a \
                             database of its own",
                            migrations.component
                        )));
                    }
                    row.get::<_, i64>(1)
                }
                _ => return Err(DbError::new("portitor_schema has more than one row")),
            };
            let known = i64::try_from(migrations.postgres.len()).unwrap_or(i64::MAX);
            if applied > known {
                return Err(DbError::new(format!(
                    "database schema version {applied} is newer than this Portitor ({known})"
                )));
            }
            let from = usize::try_from(applied)
                .map_err(|_| DbError::new(format!("invalid schema version {applied}")))?;
            for sql in &migrations.postgres[from..] {
                tx.batch_execute(sql).await.map_err(pg_error)?;
            }
            tx.execute("UPDATE portitor_schema SET version = $1", &[&known])
                .await
                .map_err(pg_error)?;
            tx.commit().await.map_err(pg_error)
        })
    }

    /// A connection: an idle one, a new one when fewer than
    /// [`POOL_SIZE`] are open, or the next one given back.
    fn take(&self) -> Result<Taken<'_>, DbError> {
        let deadline = Instant::now() + WAIT_FOR_A_CONNECTION;
        let mut idle = self.lock_idle();
        loop {
            while let Some(client) = idle.clients.pop() {
                if client.is_closed() {
                    // the server or the network closed it: forget it
                    idle.open -= 1;
                } else {
                    return Ok(Taken::new(self, client));
                }
            }
            if idle.open < POOL_SIZE {
                idle.open += 1;
                drop(idle);
                return match self.connect() {
                    Ok(client) => Ok(Taken::new(self, client)),
                    Err(e) => {
                        self.lock_idle().open -= 1;
                        Err(e)
                    }
                };
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(DbError::new(format!(
                    "all {POOL_SIZE} database connections stayed busy for {}s",
                    WAIT_FOR_A_CONNECTION.as_secs()
                )));
            }
            idle = self
                .returned
                .wait_timeout(idle, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn lock_idle(&self) -> MutexGuard<'_, Idle> {
        // the list stays valid whatever a panicking holder was doing
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens a connection; its socket is driven on the database thread.
    fn connect(&self) -> Result<Client, DbError> {
        let config = self.settings.config.clone();
        let tls = self.settings.tls.clone();
        let shown = self.settings.shown.clone();
        let connecting = self.runtime.spawn(async move {
            // two branches that look the same: `config.connect` is generic
            // over the TLS type, and `connection` has a different type in
            // each, so they cannot share one `let`
            let client = if let Some(tls) = tls {
                let tls = super::postgres_tls::MakeRustlsConnect::new(tls);
                let (client, connection) = config.connect(tls).await?;
                tokio::spawn(drive(connection, shown));
                client
            } else {
                let (client, connection) = config.connect(tokio_postgres::NoTls).await?;
                tokio::spawn(drive(connection, shown));
                client
            };
            Ok::<_, tokio_postgres::Error>(client)
        });
        match block_on(connecting) {
            Ok(Ok(client)) => Ok(client),
            Ok(Err(e)) => Err(DbError::new(format!(
                "cannot connect to {}: {}",
                self.settings.shown,
                pg_error(e)
            ))),
            Err(e) => Err(DbError::new(format!("database thread: {e}"))),
        }
    }
}

/// Creates the empty database `name` on the server of `admin`, for
/// tests. `name` must be a plain identifier (letters, digits, `_`).
///
/// # Errors
/// `name` is not a plain identifier, or the server refused.
#[cfg(any(test, feature = "test-support"))]
pub fn create_database(admin: &Settings, name: &str) -> Result<(), DbError> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DbError::new(format!("not a plain database name: {name}")));
    }
    let pool = Pool::start(admin)?;
    // CREATE DATABASE takes no parameters and cannot run in a transaction
    pool.execute(&format!("CREATE DATABASE {name}"), &[])
        .map(|_| ())
}

/// Whether the database `name` exists on the server of `admin`, for
/// tests.
///
/// # Errors
/// The server cannot be reached.
#[cfg(any(test, feature = "test-support"))]
pub fn database_exists(admin: &Settings, name: &str) -> Result<bool, DbError> {
    let pool = Pool::start(admin)?;
    let rows = pool.query(
        "SELECT 1 FROM pg_database WHERE datname = $1",
        &[Value::from(name)],
    )?;
    Ok(!rows.is_empty())
}

/// Drives one connection's socket until it closes.
async fn drive<F>(connection: F, shown: String)
where
    F: Future<Output = Result<(), tokio_postgres::Error>>,
{
    if let Err(e) = connection.await {
        tracing::warn!(database = %shown, error = %e, "database connection closed");
    }
}

/// A connection taken from the pool; given back when dropped, unless
/// the server did not answer it in time.
struct Taken<'p> {
    pool: &'p Pool,
    /// Always `Some` until `drop` gives it back.
    client: Option<Client>,
    /// Set when a call on it waited longer than [`ANSWER_TIMEOUT`]: the
    /// connection may be dead, or still busy with that call, so it is
    /// closed instead of given back.
    timed_out: Cell<bool>,
}

impl<'p> Taken<'p> {
    fn new(pool: &'p Pool, client: Client) -> Self {
        Self {
            pool,
            client: Some(client),
            timed_out: Cell::new(false),
        }
    }

    /// The connection, and the flag its calls set when they time out.
    fn parts(&mut self) -> Result<(&mut Client, &Cell<bool>), DbError> {
        match self.client.as_mut() {
            Some(client) => Ok((client, &self.timed_out)),
            None => Err(DbError::new("database connection already given back")),
        }
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        let mut idle = self.pool.lock_idle();
        if self.timed_out.get() {
            // dropping the client closes the connection; a new one is
            // opened in its place when needed
            idle.open -= 1;
            drop(client);
        } else {
            idle.clients.push(client);
        }
        drop(idle);
        self.pool.returned.notify_one();
    }
}

/// Waits for the server's answer to one call, at most
/// [`ANSWER_TIMEOUT`]; on timeout sets `timed_out`, so the connection is
/// not used again.
fn answer<T>(
    call: impl Future<Output = Result<T, tokio_postgres::Error>>,
    timed_out: &Cell<bool>,
) -> Result<T, DbError> {
    answer_within(call, ANSWER_TIMEOUT, timed_out)
}

/// [`answer`], with the time to wait given (for its test).
fn answer_within<T>(
    call: impl Future<Output = Result<T, tokio_postgres::Error>>,
    wait: Duration,
    timed_out: &Cell<bool>,
) -> Result<T, DbError> {
    if let Some(result) = block_on_until(call, Instant::now() + wait) {
        return result.map_err(pg_error);
    }
    timed_out.set(true);
    Err(DbError::new(format!(
        "the database did not answer within {}s",
        wait.as_secs()
    )))
}

/// Statements on one PostgreSQL client, outside a transaction.
///
/// [`PgTx`] below is the same code for a transaction. `Client` and
/// `Transaction` have the same `query` and `execute` methods; their
/// common trait in `tokio-postgres`, `GenericClient`, could only be used
/// as a generic parameter (its methods are generic). For two methods of
/// two lines each, writing them twice reads more easily than one generic
/// struct with its bounds.
struct PgRunner<'c> {
    client: &'c Client,
    timed_out: &'c Cell<bool>,
}

impl Executor for PgRunner<'_> {
    fn query(&self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let params = params(values);
        let rows = answer(self.client.query(sql, &params), self.timed_out)?;
        rows.iter().map(row_from_pg).collect()
    }

    fn execute(&self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let params = params(values);
        answer(self.client.execute(sql, &params), self.timed_out)
    }
}

/// Statements inside one PostgreSQL transaction.
struct PgTx<'t, 'c> {
    tx: &'t tokio_postgres::Transaction<'c>,
    timed_out: &'t Cell<bool>,
}

impl Executor for PgTx<'_, '_> {
    fn query(&self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let params = params(values);
        let rows = answer(self.tx.query(sql, &params), self.timed_out)?;
        rows.iter().map(row_from_pg).collect()
    }

    fn execute(&self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let params = params(values);
        answer(self.tx.execute(sql, &params), self.timed_out)
    }
}

/// One try of a transaction. Rolled back when `work` fails: the
/// transaction is dropped without `commit`.
fn run_once<T>(
    client: &mut Client,
    timed_out: &Cell<bool>,
    work: &mut dyn FnMut(&dyn Executor) -> Result<T, DbError>,
) -> Result<T, DbError> {
    let starting = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::Serializable)
        .start();
    let tx = answer(starting, timed_out)?;
    let out = work(&PgTx { tx: &tx, timed_out })?;
    answer(tx.commit(), timed_out)?;
    Ok(out)
}

/// Whether PostgreSQL said the transaction collided with another one and
/// may succeed when tried again.
fn is_collision(e: &DbError) -> bool {
    matches!(e.code.as_deref(), Some("40001" | "40P01"))
}

fn params(values: &[Value]) -> Vec<&(dyn ToSql + Sync)> {
    values.iter().map(|v| v as &(dyn ToSql + Sync)).collect()
}

#[allow(clippy::needless_pass_by_value)] // so it can be written map_err(pg_error)
fn pg_error(e: tokio_postgres::Error) -> DbError {
    let code = e.code().map(|c| c.code().to_owned());
    // the server's own message, when there is one, says more than the
    // client's "db error"
    let message = match e.as_db_error() {
        Some(db) => db.message().to_owned(),
        None => with_causes(&e),
    };
    DbError { message, code }
}

/// An error and the errors that caused it: "error performing TLS
/// handshake: invalid peer certificate: `NotValidForName`".
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        text.push_str(": ");
        text.push_str(&c.to_string());
        cause = c.source();
    }
    text
}

impl ToSql for Value {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Int(v) => match *ty {
                Type::INT2 => i16::try_from(*v)?.to_sql(ty, out),
                Type::INT4 => i32::try_from(*v)?.to_sql(ty, out),
                Type::BOOL => (*v != 0).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            Self::Text(v) => v.to_sql(ty, out),
            Self::Bytes(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(ty: &Type) -> bool {
        <i64 as ToSql>::accepts(ty)
            || <i32 as ToSql>::accepts(ty)
            || <i16 as ToSql>::accepts(ty)
            || <bool as ToSql>::accepts(ty)
            || <String as ToSql>::accepts(ty)
            || <Vec<u8> as ToSql>::accepts(ty)
    }

    to_sql_checked!();
}

/// A PostgreSQL row as [`Value`]s, by the type of each column.
///
/// A column of any other type (`TEXT`, `VARCHAR`, `NAME`, ...) is read as
/// text. That is safe: Portitor's schemas use only `BIGINT`, `INTEGER`,
/// `SMALLINT`, `BOOLEAN`, `BYTEA` and text columns, and `try_get` checks
/// the type itself: a column that cannot be read as text (a `NUMERIC` or
/// a `TIMESTAMP`, say) gives an error, never a wrong value.
fn row_from_pg(row: &tokio_postgres::Row) -> Result<Row, DbError> {
    let mut cells = Vec::with_capacity(row.len());
    for (i, column) in row.columns().iter().enumerate() {
        let read =
            |e: tokio_postgres::Error| DbError::new(format!("column {}: {e}", column.name()));
        let cell = match *column.type_() {
            Type::INT8 => row
                .try_get::<_, Option<i64>>(i)
                .map_err(read)?
                .map(Value::Int),
            Type::INT4 => row
                .try_get::<_, Option<i32>>(i)
                .map_err(read)?
                .map(|v| Value::Int(i64::from(v))),
            Type::INT2 => row
                .try_get::<_, Option<i16>>(i)
                .map_err(read)?
                .map(|v| Value::Int(i64::from(v))),
            Type::BOOL => row
                .try_get::<_, Option<bool>>(i)
                .map_err(read)?
                .map(|v| Value::Int(i64::from(v))),
            Type::BYTEA => row
                .try_get::<_, Option<Vec<u8>>>(i)
                .map_err(read)?
                .map(Value::Bytes),
            _ => row
                .try_get::<_, Option<String>>(i)
                .map_err(read)?
                .map(Value::Text),
        };
        cells.push(cell.unwrap_or(Value::Null));
    }
    Ok(Row(cells))
}

/// Starts the thread whose runtime drives the connections' sockets. It
/// ends when the returned sender is dropped.
fn start_database_thread()
-> Result<(tokio::runtime::Handle, tokio::sync::oneshot::Sender<()>), DbError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| DbError::new(format!("database thread: {e}")))?;
    let handle = runtime.handle().clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    std::thread::Builder::new()
        .name("portitor-database".to_owned())
        .spawn(move || {
            // a dropped sender also ends the wait
            let _ = runtime.block_on(stopped);
        })
        .map_err(|e| DbError::new(format!("database thread: {e}")))?;
    Ok((handle, stop))
}

/// Runs `future` to its end on the calling thread, which sleeps while the
/// future waits. The futures given here only wait for messages from the
/// database thread, so no tokio runtime is needed on this thread. The
/// calling thread is often a tokio worker, parked meanwhile (module doc,
/// TD-40).
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
        std::thread::park();
    }
}

/// [`block_on`], but gives up at `deadline`: `None` then.
fn block_on_until<F: Future>(future: F, deadline: Instant) -> Option<F::Output> {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return Some(out);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        std::thread::park_timeout(left);
    }
}

/// Wakes the thread that waits in [`block_on`].
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn read(url: &str) -> Result<Settings, String> {
        Settings::from_url(url, None)
    }

    /// A URL with `sslrootcert` naming a file with a test CA; the file
    /// lives as long as the returned directory.
    fn with_root_cert(url: &str) -> (crate::testing::TempDir, String) {
        let dir = crate::testing::TempDir::new("postgres-settings");
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, super::super::postgres_tls::TEST_CA).unwrap();
        let url = format!("{url}&sslrootcert={}", path.display());
        (dir, url)
    }

    #[test]
    fn a_password_in_the_url_is_refused() {
        let err = read("postgresql://u:secret@h/db?sslmode=disable").unwrap_err();
        assert!(err.contains("holds a password"), "{err}");
        let err = read("postgresql://u@h/db?sslmode=disable&password=secret").unwrap_err();
        assert!(err.contains("holds a password"), "{err}");
    }

    #[test]
    fn user_host_and_database_are_required() {
        let err = read("postgresql://h/db?sslmode=disable").unwrap_err();
        assert!(err.contains("user and a database"), "{err}");
        let err = read("postgresql://u@h?sslmode=disable").unwrap_err();
        assert!(err.contains("user and a database"), "{err}");
        let err = read("mysql://u@h/db").unwrap_err();
        assert!(err.contains("postgresql://"), "{err}");
    }

    /// `prefer` and `allow` fall back to plain text without saying so.
    #[test]
    fn sslmodes_that_fall_back_to_plain_text_are_refused() {
        for mode in ["prefer", "allow"] {
            let err = read(&format!("postgresql://u@h/db?sslmode={mode}")).unwrap_err();
            assert!(err.contains(&format!("sslmode={mode} is refused")), "{err}");
        }
    }

    /// No `sslmode` means `verify-full`; `require` and `verify-ca` check
    /// as much as `verify-full`; `disable` turns TLS off.
    #[test]
    fn tls_is_on_and_checked_unless_disabled() {
        let (_dir, url) = with_root_cert("postgresql://u@h/db?application_name=x");
        let missing = read(&url).unwrap();
        assert!(missing.tls.is_some());
        for mode in ["verify-full", "verify-ca", "require"] {
            let given = read(&format!("{url}&sslmode={mode}")).unwrap();
            assert_eq!(given, missing, "{mode}");
        }
        let disabled = read("postgresql://u@h/db?sslmode=disable").unwrap();
        assert!(disabled.tls.is_none());
        assert_eq!(
            disabled.config.get_ssl_mode(),
            tokio_postgres::config::SslMode::Disable
        );
    }

    #[test]
    fn sslrootcert_is_a_readable_file_or_system() {
        let (_dir, url) = with_root_cert("postgresql://u@h/db?sslmode=verify-full");
        assert!(read(&url).unwrap().tls.is_some());
        let err = read("postgresql://u@h/db?sslrootcert=/nowhere/ca.pem").unwrap_err();
        assert!(err.contains("sslrootcert /nowhere/ca.pem"), "{err}");
        // the system's CAs, by name, as without sslrootcert
        let system = read("postgresql://u@h/db?sslrootcert=system").unwrap();
        assert!(system.tls.is_some());
        // the query is not shown in logs
        assert_eq!(system.shown(), "postgresql://u@h/db");
    }

    #[test]
    fn timeouts_have_defaults() {
        let settings = read("postgresql://u@h/db?sslmode=disable").unwrap();
        let config = &settings.config;
        assert_eq!(config.get_connect_timeout(), Some(&CONNECT_TIMEOUT));
        assert_eq!(config.get_tcp_user_timeout(), Some(&TCP_USER_TIMEOUT));
        assert!(config.get_keepalives());
        assert_eq!(config.get_keepalives_idle(), KEEPALIVES_IDLE);
        assert_eq!(config.get_keepalives_interval(), Some(KEEPALIVES_INTERVAL));
        assert_eq!(config.get_options(), Some("-c statement_timeout=30s"));
    }

    /// As libpq reads them: seconds, but milliseconds for
    /// `tcp_user_timeout`; `connect_timeout=0` waits without limit.
    #[test]
    fn timeouts_in_the_url_are_kept() {
        let settings = read(
            "postgresql://u@h/db?sslmode=disable&connect_timeout=0&tcp_user_timeout=2500\
             &keepalives_idle=60&keepalives_interval=7\
             &options=-c%20statement_timeout%3D5s",
        )
        .unwrap();
        let config = &settings.config;
        assert_eq!(config.get_connect_timeout(), None);
        assert_eq!(
            config.get_tcp_user_timeout(),
            Some(&Duration::from_millis(2500))
        );
        assert_eq!(config.get_keepalives_idle(), Duration::from_secs(60));
        assert_eq!(
            config.get_keepalives_interval(),
            Some(Duration::from_secs(7))
        );
        assert_eq!(config.get_options(), Some("-c statement_timeout=5s"));
        // other options are kept, and the statement timeout added
        let settings =
            read("postgresql://u@h/db?sslmode=disable&options=-c%20work_mem%3D8MB").unwrap();
        assert_eq!(
            settings.config.get_options(),
            Some("-c work_mem=8MB -c statement_timeout=30s")
        );
        let err = read("postgresql://u@h/db?sslmode=disable&tcp_user_timeout=soon").unwrap_err();
        assert!(err.contains("tcp_user_timeout=soon"), "{err}");
    }

    /// A call the server does not answer ends, and marks its connection
    /// so it is not given back to the pool.
    #[test]
    fn a_call_without_an_answer_times_out() {
        let timed_out = Cell::new(false);
        let never = std::future::pending::<Result<(), tokio_postgres::Error>>();
        let err = answer_within(never, Duration::from_millis(20), &timed_out).unwrap_err();
        assert!(err.message.contains("did not answer"), "{err}");
        assert!(timed_out.get());
    }
}
