// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The database every Portitor store writes to: PostgreSQL, shared by
//! several replicas, or a `SQLite` file for one process (deprecated in 1.3,
//! removed in 2.0; ADR 0029).
//!
//! A store writes its SQL once, for both databases, and runs it through
//! [`Database`]:
//!
//! - placeholders are `$1`, `$2`, ... (PostgreSQL's form; for `SQLite`
//!   they are rewritten to `?1`, `?2`, ...);
//! - values go in as [`Value`] (text, whole numbers, bytes, null) and come
//!   back as a [`Row`] of them;
//! - a write of several statements is one [`Database::transaction`]. On
//!   PostgreSQL it runs at the `SERIALIZABLE` level and is tried again
//!   when the database says two transactions collided, so a store sees the
//!   same "one writer at a time" behaviour it always had with `SQLite`.
//!
//! The SQL that differs between the two databases is the schema only:
//! each store gives its migrations for both ([`Migrations`]).

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::Connection;

pub mod postgres;
mod postgres_tls;
pub mod sqlite_lease;

/// A value going into a statement or coming out of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// SQL `NULL`.
    Null,
    /// A whole number (`INTEGER` in `SQLite`, `BIGINT` in PostgreSQL).
    Int(i64),
    /// Text.
    Text(String),
    /// Bytes (`BLOB` in `SQLite`, `BYTEA` in PostgreSQL).
    Bytes(Vec<u8>),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Self::Int(i64::from(v))
    }
}

impl From<bool> for Value {
    /// Stored as 1 or 0, as `SQLite` does.
    fn from(v: bool) -> Self {
        Self::Int(i64::from(v))
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&String> for Value {
    fn from(v: &String) -> Self {
        Self::Text(v.clone())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Self::Bytes(v)
    }
}

impl From<&[u8]> for Value {
    fn from(v: &[u8]) -> Self {
        Self::Bytes(v.to_vec())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

/// The values of a statement, written like `rusqlite::params!`:
/// `values![pid, state, version]`.
#[macro_export]
macro_rules! values {
    () => { &[] as &[$crate::database::Value] };
    ($($value:expr),+ $(,)?) => {
        &[$($crate::database::Value::from($value)),+] as &[$crate::database::Value]
    };
}

/// One row of a query's answer, its columns in the order of the `SELECT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row(pub Vec<Value>);

impl Row {
    /// Column `i`, read as `T`.
    ///
    /// # Errors
    /// No column `i`, or its value is not a `T` (text where a number was
    /// expected, `NULL` where a value was expected).
    pub fn get<T: FromValue>(&self, i: usize) -> Result<T, DbError> {
        let value = self
            .0
            .get(i)
            .ok_or_else(|| DbError::new(format!("no column {i}")))?;
        T::from_value(value).map_err(|wanted| {
            DbError::new(format!("column {i}: expected {wanted}, found {value:?}"))
        })
    }
}

/// A Rust type a column can be read as.
pub trait FromValue: Sized {
    /// The value as `Self`, or what was expected instead.
    ///
    /// # Errors
    /// The value is of another kind, with the kind that was expected.
    fn from_value(value: &Value) -> Result<Self, &'static str>;
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<Self, &'static str> {
        match value {
            Value::Int(v) => Ok(*v),
            _ => Err("a whole number"),
        }
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, &'static str> {
        match value {
            Value::Int(v) => Ok(*v != 0),
            _ => Err("1 or 0"),
        }
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, &'static str> {
        match value {
            Value::Text(v) => Ok(v.clone()),
            _ => Err("text"),
        }
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> Result<Self, &'static str> {
        match value {
            Value::Bytes(v) => Ok(v.clone()),
            _ => Err("bytes"),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Self, &'static str> {
        match value {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

/// What went wrong in the database, as text, with the SQLSTATE code when
/// PostgreSQL gave one (`23505` for a duplicate key, for example).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    /// What went wrong.
    pub message: String,
    /// PostgreSQL's SQLSTATE code, when there is one.
    pub code: Option<String>,
}

impl DbError {
    /// An error with a message and no code.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new(e.to_string())
    }
}

/// Runs statements: the [`Database`] itself, or one of its transactions.
pub trait Executor {
    /// Runs a `SELECT` (or a statement with `RETURNING`) and returns its
    /// rows.
    ///
    /// # Errors
    /// The database refused the statement or could not be reached.
    fn query(&mut self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError>;

    /// Runs a statement that changes rows and returns how many it changed.
    ///
    /// # Errors
    /// The database refused the statement or could not be reached.
    fn execute(&mut self, sql: &str, values: &[Value]) -> Result<u64, DbError>;

    /// [`Executor::query`] for at most one row.
    ///
    /// # Errors
    /// As [`Executor::query`].
    fn query_one(&mut self, sql: &str, values: &[Value]) -> Result<Option<Row>, DbError> {
        Ok(self.query(sql, values)?.into_iter().next())
    }
}

/// A store's schema, as migrations for each database. Each list is
/// applied in order; the schema version is the number applied. Never edit
/// a released migration; add a new one to both lists.
#[derive(Debug, Clone, Copy)]
pub struct Migrations {
    /// Who owns the schema (`portitor-connector`, `portitor-identity`,
    /// `portitor-issuer`). A PostgreSQL database holds the schema of one
    /// component; another component refuses it.
    pub component: &'static str,
    /// The `SQLite` migrations, since 0.3.
    pub sqlite: &'static [&'static str],
    /// The PostgreSQL migrations, since 1.3: the first one is the schema
    /// of 1.3 as a whole.
    pub postgres: &'static [&'static str],
}

/// Where a component keeps its state, from its settings. Two settings
/// are equal when they name the same file, or connect to the same
/// database the same way (for the tests of a component's settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseSetting {
    /// `PORTITOR_DB_FILE` (or its default): a `SQLite` file, one process.
    SqliteFile(PathBuf),
    /// `PORTITOR_DATABASE_URL`: PostgreSQL.
    Postgres(Box<postgres::Settings>),
}

/// Reads `PORTITOR_DATABASE_URL`, `PORTITOR_DATABASE_PASSWORD_FILE` and
/// `PORTITOR_DB_FILE`. Without a URL the component uses the `SQLite`
/// file, `default_file` unless `PORTITOR_DB_FILE` says otherwise.
///
/// # Errors
/// Both a URL and a file are set; a password file is set without a URL;
/// the URL or the password file cannot be read (see
/// [`postgres::Settings::from_url`]).
pub fn setting(
    var: impl Fn(&str) -> Option<OsString>,
    default_file: &str,
) -> Result<DatabaseSetting, String> {
    use crate::settings::text;
    let url = text(&var, "PORTITOR_DATABASE_URL");
    let password_file = text(&var, "PORTITOR_DATABASE_PASSWORD_FILE");
    let file = text(&var, "PORTITOR_DB_FILE");
    match (url, file) {
        (Some(_), Some(_)) => Err(
            "set PORTITOR_DATABASE_URL (PostgreSQL) or PORTITOR_DB_FILE (SQLite), not both"
                .to_owned(),
        ),
        (Some(url), None) => {
            let password = password_file.map(|path| read_password(&path)).transpose()?;
            postgres::Settings::from_url(&url, password)
                .map(|settings| DatabaseSetting::Postgres(Box::new(settings)))
        }
        (None, file) => {
            if password_file.is_some() {
                return Err(
                    "PORTITOR_DATABASE_PASSWORD_FILE is set without PORTITOR_DATABASE_URL"
                        .to_owned(),
                );
            }
            Ok(DatabaseSetting::SqliteFile(PathBuf::from(
                file.unwrap_or_else(|| default_file.to_owned()),
            )))
        }
    }
}

/// The password in `path`, without the line break at its end.
fn read_password(path: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("PORTITOR_DATABASE_PASSWORD_FILE {path}: {e}"))?;
    let password = text.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err(format!("PORTITOR_DATABASE_PASSWORD_FILE {path} is empty"));
    }
    Ok(password.to_owned())
}

/// The warning a component logs at start when it uses a `SQLite` file.
pub const SQLITE_DEPRECATED: &str = "SQLite is deprecated since 1.3 and is removed in 2.0; \
     set PORTITOR_DATABASE_URL to use PostgreSQL (DEPLOYING, \"Database\")";

/// A component's database: PostgreSQL, or a `SQLite` file (or memory, in
/// tests).
#[derive(Debug)]
pub enum Database {
    /// One `SQLite` connection; the lock makes one statement or
    /// transaction run at a time. A file comes with the lease that keeps
    /// other processes out of it (ADR 0029, D3; [`sqlite_lease`]); a
    /// database in memory has none.
    Sqlite(Mutex<Connection>, Option<sqlite_lease::ProcessLease>),
    /// A pool of PostgreSQL connections.
    Postgres(Box<postgres::Pool>),
}

impl Database {
    /// Opens the database `setting` names and brings its schema up to
    /// date.
    ///
    /// A `SQLite` file is taken for this process first: a second process
    /// on the same file refuses to start ([`sqlite_lease`]).
    ///
    /// # Errors
    /// The database cannot be opened or reached, another process uses the
    /// `SQLite` file, a migration fails, or the schema is newer than this
    /// Portitor (or another component's).
    pub fn open(setting: &DatabaseSetting, migrations: &Migrations) -> Result<Self, DbError> {
        match setting {
            DatabaseSetting::SqliteFile(path) => {
                // before the migrations: a file in use is not changed
                let lease = sqlite_lease::take(path)?;
                let conn =
                    crate::sqlite::open(Some(path), migrations.sqlite).map_err(DbError::new)?;
                Ok(Self::Sqlite(Mutex::new(conn), Some(lease)))
            }
            DatabaseSetting::Postgres(settings) => postgres::Pool::open(settings, migrations)
                .map(|pool| Self::Postgres(Box::new(pool))),
        }
    }

    /// An in-memory `SQLite` database with `migrations`, for tests.
    ///
    /// # Errors
    /// `SQLite` cannot allocate the database or a migration fails.
    pub fn in_memory(migrations: &Migrations) -> Result<Self, DbError> {
        let conn = crate::sqlite::open(None, migrations.sqlite).map_err(DbError::new)?;
        // one process by definition: no lease
        Ok(Self::Sqlite(Mutex::new(conn), None))
    }

    /// Whether this is PostgreSQL (several replicas allowed).
    #[must_use]
    pub fn is_postgres(&self) -> bool {
        matches!(self, Self::Postgres(_))
    }

    /// Runs `work` as one transaction: everything it wrote is kept when it
    /// returns `Ok`, nothing when it returns `Err` or panics.
    ///
    /// On PostgreSQL the transaction is `SERIALIZABLE`, and when the
    /// database answers that it collided with another one (SQLSTATE
    /// `40001` or `40P01`), `work` runs again, up to
    /// [`postgres::TRIES`] times. So `work` must only read and write
    /// through `tx`, with no other effect.
    ///
    /// # Errors
    /// What `work` returned, or the database's error.
    pub fn transaction<T>(
        &self,
        mut work: impl FnMut(&mut dyn Executor) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        match self {
            Self::Sqlite(conn, _) => {
                let mut conn = lock(conn);
                // takes the write lock at the start: a transaction that
                // reads, then writes, would otherwise fail at once
                // (SQLITE_BUSY, without waiting) when another connection
                // wrote in between, as the lease thread does every 10 s
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut runner = SqliteRunner(&tx);
                let out = work(&mut runner)?;
                tx.commit()?;
                Ok(out)
            }
            Self::Postgres(pool) => pool.transaction(&mut work),
        }
    }

    /// The `SQLite` connection, for the store tests that check what a
    /// panic leaves behind (`sqlite::testing`). `None` on PostgreSQL.
    #[must_use]
    pub fn sqlite_connection(&self) -> Option<MutexGuard<'_, Connection>> {
        match self {
            Self::Sqlite(conn, _) => Some(lock(conn)),
            Self::Postgres(_) => None,
        }
    }
}

impl Executor for &Database {
    fn query(&mut self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        match *self {
            Database::Sqlite(conn, _) => SqliteRunner(&lock(conn)).query(sql, values),
            Database::Postgres(pool) => pool.query(sql, values),
        }
    }

    fn execute(&mut self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        match *self {
            Database::Sqlite(conn, _) => SqliteRunner(&lock(conn)).execute(sql, values),
            Database::Postgres(pool) => pool.execute(sql, values),
        }
    }
}

/// Takes the `SQLite` lock, also after a panic of its last holder:
/// `SQLite` keeps the file consistent (TD-41).
fn lock(conn: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Statements on one `SQLite` connection.
struct SqliteRunner<'c>(&'c Connection);

impl Executor for SqliteRunner<'_> {
    fn query(&mut self, sql: &str, values: &[Value]) -> Result<Vec<Row>, DbError> {
        let mut stmt = self.0.prepare_cached(&sqlite_placeholders(sql))?;
        let columns = stmt.column_count();
        let mut rows = stmt.query(rusqlite::params_from_iter(values.iter().map(to_sqlite)))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut cells = Vec::with_capacity(columns);
            for i in 0..columns {
                cells.push(from_sqlite(row.get_ref(i)?)?);
            }
            out.push(Row(cells));
        }
        Ok(out)
    }

    fn execute(&mut self, sql: &str, values: &[Value]) -> Result<u64, DbError> {
        let mut stmt = self.0.prepare_cached(&sqlite_placeholders(sql))?;
        let changed = stmt.execute(rusqlite::params_from_iter(values.iter().map(to_sqlite)))?;
        Ok(u64::try_from(changed).unwrap_or(u64::MAX))
    }
}

fn to_sqlite(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match value {
        Value::Null => S::Null,
        Value::Int(v) => S::Integer(*v),
        Value::Text(v) => S::Text(v.clone()),
        Value::Bytes(v) => S::Blob(v.clone()),
    }
}

fn from_sqlite(value: rusqlite::types::ValueRef<'_>) -> Result<Value, DbError> {
    use rusqlite::types::ValueRef as S;
    match value {
        S::Null => Ok(Value::Null),
        S::Integer(v) => Ok(Value::Int(v)),
        S::Text(v) => String::from_utf8(v.to_vec())
            .map(Value::Text)
            .map_err(|e| DbError::new(format!("stored text is not UTF-8: {e}"))),
        S::Blob(v) => Ok(Value::Bytes(v.to_vec())),
        S::Real(v) => Err(DbError::new(format!("unexpected real number {v}"))),
    }
}

/// `$1`, `$2`, ... (PostgreSQL) rewritten as `?1`, `?2`, ... (`SQLite`).
/// A `$` inside a quoted string is left alone.
fn sqlite_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_quotes = false;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            in_quotes = !in_quotes;
        }
        let before_digit = chars.peek().is_some_and(char::is_ascii_digit);
        if c == '$' && !in_quotes && before_digit {
            out.push('?');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const MIGRATIONS: Migrations = Migrations {
        component: "test",
        sqlite: &["CREATE TABLE item (id TEXT PRIMARY KEY, n INTEGER NOT NULL, note TEXT)"],
        postgres: &["CREATE TABLE item (id TEXT PRIMARY KEY, n BIGINT NOT NULL, note TEXT)"],
    };

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn placeholders_are_rewritten_outside_quotes_only() {
        assert_eq!(
            sqlite_placeholders("SELECT '$1' FROM t WHERE a = $1 AND b = $12"),
            "SELECT '$1' FROM t WHERE a = ?1 AND b = ?12"
        );
    }

    #[test]
    fn rows_come_back_as_values_and_typed_reads_check_them() {
        let db = Database::in_memory(&MIGRATIONS).unwrap();
        let changed = (&db)
            .execute(
                "INSERT INTO item (id, n, note) VALUES ($1, $2, $3)",
                values!["a", 7_i64, None::<String>],
            )
            .unwrap();
        assert_eq!(changed, 1);
        let row = (&db)
            .query_one("SELECT id, n, note FROM item WHERE id = $1", values!["a"])
            .unwrap()
            .unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "a");
        assert_eq!(row.get::<i64>(1).unwrap(), 7);
        assert_eq!(row.get::<Option<String>>(2).unwrap(), None);
        assert!(row.get::<String>(1).is_err(), "a number is not text");
        assert!(row.get::<String>(3).is_err(), "no fourth column");
    }

    #[test]
    fn a_failed_transaction_keeps_nothing() {
        let db = Database::in_memory(&MIGRATIONS).unwrap();
        let result: Result<(), DbError> = db.transaction(|tx| {
            tx.execute(
                "INSERT INTO item (id, n) VALUES ($1, $2)",
                values!["a", 1_i64],
            )?;
            Err(DbError::new("changed my mind"))
        });
        assert!(result.is_err());
        let rows = (&db).query("SELECT id FROM item", values![]).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn the_setting_is_a_url_or_a_file_never_both() {
        let file = setting(vars(&[]), "default.db").unwrap();
        assert!(matches!(file, DatabaseSetting::SqliteFile(p) if p.as_os_str() == "default.db"));
        let err = setting(
            vars(&[
                ("PORTITOR_DATABASE_URL", "postgresql://portitor@db/portitor"),
                ("PORTITOR_DB_FILE", "x.db"),
            ]),
            "default.db",
        )
        .unwrap_err();
        assert!(err.contains("not both"), "{err}");
        let err = setting(
            vars(&[("PORTITOR_DATABASE_PASSWORD_FILE", "/run/secrets/db")]),
            "default.db",
        )
        .unwrap_err();
        assert!(err.contains("without PORTITOR_DATABASE_URL"), "{err}");
    }

    /// The lease thread writes the `SQLite` file from a connection of its
    /// own. A transaction that reads, then writes, while another
    /// connection writes in between, still succeeds: the other write
    /// waits for it.
    #[test]
    fn a_write_from_another_connection_waits_for_a_transaction() {
        let dir = crate::testing::TempDir::new("database-immediate");
        let path = dir.path().join("component.db");
        let setting = DatabaseSetting::SqliteFile(path.clone());
        let db = Database::open(&setting, &MIGRATIONS).unwrap();
        let mut other_writer = None;
        db.transaction(|tx| {
            let rows = tx.query("SELECT id FROM item", values![])?;
            let other_path = path.clone();
            other_writer.get_or_insert_with(|| {
                std::thread::spawn(move || {
                    let other = crate::sqlite::connect(Some(&other_path)).unwrap();
                    other
                        .execute("INSERT INTO item (id, n) VALUES ('other', 1)", [])
                        .unwrap();
                })
            });
            // time for the other connection to write, if it could
            std::thread::sleep(std::time::Duration::from_millis(200));
            tx.execute(
                "INSERT INTO item (id, n) VALUES ($1, $2)",
                values!["ours", i64::try_from(rows.len()).unwrap_or(0)],
            )
        })
        .unwrap();
        other_writer.unwrap().join().unwrap();
        let rows = (&db)
            .query("SELECT id FROM item ORDER BY id", values![])
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    /// ADR 0029, D3: a second process on one `SQLite` file refuses to
    /// start, with an error that says why and what to do; once the first
    /// one has closed the file, the next one opens it.
    #[test]
    fn a_second_open_of_one_sqlite_file_is_refused() {
        let dir = crate::testing::TempDir::new("database-one-process");
        let setting = DatabaseSetting::SqliteFile(dir.path().join("component.db"));
        let first = Database::open(&setting, &MIGRATIONS).unwrap();
        let refused = Database::open(&setting, &MIGRATIONS).unwrap_err();
        assert!(
            refused.message.contains("is in use by another process"),
            "{refused}"
        );
        assert!(refused.message.contains("use PostgreSQL"), "{refused}");
        drop(first);
        let again = Database::open(&setting, &MIGRATIONS).unwrap();
        (&again)
            .execute(
                "INSERT INTO item (id, n) VALUES ($1, $2)",
                values!["a", 1_i64],
            )
            .unwrap();
    }
}
