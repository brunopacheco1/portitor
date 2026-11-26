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

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("url", &self.shown)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Reads a PostgreSQL connection URI
    /// (`postgresql://user@host:5432/database?sslmode=verify-full`, the
    /// form libpq reads) and the password from its file.
    ///
    /// TLS: `sslmode` may be `verify-full` (the default when it is
    /// missing), `verify-ca` or `require`, which all mean the same here:
    /// TLS, with the server's certificate checked against the system's CA
    /// certificates (and `sslrootcert`, a PEM file, when given) and its
    /// host name. `disable` turns TLS off. `prefer` and `allow` are
    /// refused: they fall back to an unencrypted connection without
    /// saying so.
    ///
    /// # Errors
    /// The URL is not a `postgresql://` URI, holds a password (it goes in
    /// `PORTITOR_DATABASE_PASSWORD_FILE`), has an `sslmode` named above as
    /// refused, names no user, host or database, or `sslrootcert` cannot
    /// be read.
    pub fn from_url(url: &str, password: Option<String>) -> Result<Self, String> {
        let (base, tls_options) = split_tls_options(url)?;
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
        config.connect_timeout(CONNECT_TIMEOUT);
        config.application_name("portitor");
        let tls = if tls_options.disable {
            config.ssl_mode(tokio_postgres::config::SslMode::Disable);
            None
        } else {
            config.ssl_mode(tokio_postgres::config::SslMode::Require);
            Some(super::postgres_tls::client_config(
                tls_options.root_cert.as_deref(),
            )?)
        };
        Ok(Self {
            config,
            tls,
            shown: base_without_query(url),
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
}

/// The TLS options of the URL, which this module reads itself
/// (`tokio-postgres` knows only `disable`, `prefer` and `require`).
struct TlsOptions {
    disable: bool,
    root_cert: Option<String>,
}

/// The URL without `sslmode` and `sslrootcert`, and what they said.
fn split_tls_options(url: &str) -> Result<(String, TlsOptions), String> {
    if !(url.starts_with("postgresql://") || url.starts_with("postgres://")) {
        return Err("PORTITOR_DATABASE_URL must start with postgresql://".to_owned());
    }
    let mut options = TlsOptions {
        disable: false,
        root_cert: None,
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
    settings: Settings,
    /// The database thread's runtime, which drives the connections.
    runtime: tokio::runtime::Handle,
    /// Dropped with the pool: ends the database thread.
    _stop: tokio::sync::oneshot::Sender<()>,
    idle: Mutex<Idle>,
    returned: Condvar,
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
        PgRunner(taken.client()?).query(sql, values)
    }

    /// Runs one statement and returns how many rows it changed.
    ///
    /// # Errors
    /// The database refused it or could not be reached.
    pub fn execute(&self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let mut taken = self.take()?;
        PgRunner(taken.client()?).execute(sql, values)
    }

    /// [`super::Database::transaction`] on PostgreSQL.
    pub(super) fn transaction<T>(
        &self,
        work: &mut dyn FnMut(&mut dyn Executor) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let mut tries = 0;
        loop {
            tries += 1;
            let mut taken = self.take()?;
            match run_once(taken.client()?, work) {
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
        let client = taken.client()?;
        block_on(async {
            let tx = client.transaction().await.map_err(pg_error)?;
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
                    return Ok(Taken {
                        pool: self,
                        client: Some(client),
                    });
                }
            }
            if idle.open < POOL_SIZE {
                idle.open += 1;
                drop(idle);
                return match self.connect() {
                    Ok(client) => Ok(Taken {
                        pool: self,
                        client: Some(client),
                    }),
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

/// Drives one connection's socket until it closes.
async fn drive<F>(connection: F, shown: String)
where
    F: Future<Output = Result<(), tokio_postgres::Error>>,
{
    if let Err(e) = connection.await {
        tracing::warn!(database = %shown, error = %e, "database connection closed");
    }
}

/// A connection taken from the pool; given back when dropped.
struct Taken<'p> {
    pool: &'p Pool,
    /// Always `Some` until `drop` gives it back.
    client: Option<Client>,
}

impl Taken<'_> {
    fn client(&mut self) -> Result<&mut Client, DbError> {
        self.client
            .as_mut()
            .ok_or_else(|| DbError::new("database connection already given back"))
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            self.pool.lock_idle().clients.push(client);
            self.pool.returned.notify_one();
        }
    }
}

/// Statements on one PostgreSQL client.
struct PgRunner<'c>(&'c mut Client);

impl Executor for PgRunner<'_> {
    fn query(&mut self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let params = params(values);
        let rows = block_on(self.0.query(sql, &params)).map_err(pg_error)?;
        rows.iter().map(row_from_pg).collect()
    }

    fn execute(&mut self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let params = params(values);
        block_on(self.0.execute(sql, &params)).map_err(pg_error)
    }
}

/// Statements inside one PostgreSQL transaction.
struct PgTx<'t, 'c>(&'t tokio_postgres::Transaction<'c>);

impl Executor for PgTx<'_, '_> {
    fn query(&mut self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let params = params(values);
        let rows = block_on(self.0.query(sql, &params)).map_err(pg_error)?;
        rows.iter().map(row_from_pg).collect()
    }

    fn execute(&mut self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let params = params(values);
        block_on(self.0.execute(sql, &params)).map_err(pg_error)
    }
}

/// One try of a transaction. Rolled back when `work` fails: the
/// transaction is dropped without `commit`.
fn run_once<T>(
    client: &mut Client,
    work: &mut dyn FnMut(&mut dyn Executor) -> Result<T, DbError>,
) -> Result<T, DbError> {
    let tx = block_on(
        client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::Serializable)
            .start(),
    )
    .map_err(pg_error)?;
    let out = work(&mut PgTx(&tx))?;
    block_on(tx.commit()).map_err(pg_error)?;
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
/// database thread, so no tokio runtime is needed on this thread.
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
