// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! `SQLite` files: opening one with the settings a Portitor store needs,
//! and bringing its schema up to date (ADR 0006: one file per process, no
//! sharing between replicas).

use std::path::Path;

use rusqlite::Connection;

/// Opens (or creates) the database at `path`, or an in-memory one when
/// `path` is `None` (tests), and applies `migrations`.
///
/// # Errors
/// The file cannot be opened or created, it is not a `SQLite` database, a
/// migration fails, or it was written by a newer version.
pub fn open(path: Option<&Path>, migrations: &[&str]) -> Result<Connection, String> {
    let mut conn = match path {
        Some(path) => Connection::open(path),
        None => Connection::open_in_memory(),
    }
    .map_err(sql_error)?;
    // WAL: readers do not block the writer; FULL: what was acknowledged to
    // the other party survives a power cut
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sql_error)?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(sql_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(sql_error)?;
    migrate(&mut conn, migrations)?;
    Ok(conn)
}

/// Applies the migrations not applied yet, in one transaction. The schema
/// version is the number of migrations applied (`PRAGMA user_version`).
/// Never edit a released migration; add a new one.
///
/// # Errors
/// A migration fails, or the file has more migrations than `migrations`
/// (written by a newer version).
pub fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<(), String> {
    let tx = conn.transaction().map_err(sql_error)?;
    let stored: i64 = tx
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sql_error)?;
    let known = i64::try_from(migrations.len()).unwrap_or(i64::MAX);
    let Ok(applied) = usize::try_from(stored) else {
        return Err(format!("invalid schema version {stored}"));
    };
    if stored > known {
        return Err(format!(
            "database schema version {stored} is newer than this Portitor ({known})"
        ));
    }
    for sql in &migrations[applied..] {
        tx.execute_batch(sql).map_err(sql_error)?;
    }
    tx.pragma_update(None, "user_version", known)
        .map_err(sql_error)?;
    tx.commit().map_err(sql_error)
}

/// A `SQLite` error as the text this module's functions return.
#[allow(clippy::needless_pass_by_value)] // so it can be written map_err(sql_error)
fn sql_error(e: rusqlite::Error) -> String {
    e.to_string()
}

/// The checks each store's tests run on a panic while its lock is held
/// (`store/poison_tests.rs` in the connector, the identity component and
/// the issuer), written once here.
///
/// From 0.12 release builds unwind (TD-41), so a panic while a store's
/// lock is held leaves the lock "poisoned", and the lock recovery in
/// the store's `conn()` runs in production. Taking the lock all the
/// same is safe because `SQLite` keeps the database consistent: a write
/// of one statement is complete or not there, and a write of several is
/// one transaction, rolled back when the panic drops it.
///
/// The checks write with plain SQL on tables of their own, so they do
/// not depend on a store's schema.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use std::sync::{Arc, MutexGuard};

    use rusqlite::Connection;

    /// How the checks reach a store of type `S`: two plain functions,
    /// given by the store's test.
    pub struct Access<S> {
        /// The store's own `conn()`, the code under test: takes the lock,
        /// also a poisoned one.
        pub conn: fn(&S) -> MutexGuard<'_, Connection>,
        /// Whether a panic poisoned the store's lock
        /// (`Mutex::is_poisoned`).
        pub is_poisoned: fn(&S) -> bool,
    }

    /// A write of several statements is a `rusqlite` transaction, which
    /// rolls back when it is dropped without `commit`, also by a panic:
    /// checks that nothing of it is kept and that `store` works after.
    ///
    /// # Panics
    /// When `store` does not behave so.
    pub fn assert_a_panic_inside_a_transaction_rolls_it_back<S>(store: S, access: &Access<S>)
    where
        // the store is shared with the thread that panics
        S: Send + Sync + 'static,
    {
        let store = Arc::new(store);
        panic_while_locked(&store, access, write_in_a_transaction_then_panic);
        assert_eq!(
            tables_named(store.as_ref(), access, "half_done"),
            0,
            "rolled back"
        );
        assert_usable(store.as_ref(), access);
    }

    /// A write of one statement is complete before anything can panic:
    /// checks that it is kept and that `store` works after.
    ///
    /// # Panics
    /// When `store` does not behave so.
    pub fn assert_a_panic_after_a_single_statement_keeps_it<S>(store: S, access: &Access<S>)
    where
        S: Send + Sync + 'static,
    {
        let store = Arc::new(store);
        panic_while_locked(&store, access, write_one_statement_then_panic);
        assert_eq!(tables_named(store.as_ref(), access, "written"), 1, "kept");
        assert_usable(store.as_ref(), access);
    }

    /// Runs `write_then_panic` on the store's connection, with the lock
    /// held, on another thread (as a request would be), and checks that
    /// it panicked.
    fn panic_while_locked<S>(
        store: &Arc<S>,
        access: &Access<S>,
        write_then_panic: fn(&mut Connection),
    ) where
        S: Send + Sync + 'static,
    {
        let held = Arc::clone(store);
        let conn = access.conn;
        let thread = std::thread::spawn(move || {
            let mut locked = conn(&held);
            write_then_panic(&mut locked);
        });
        assert!(thread.join().is_err(), "the write did not panic");
    }

    fn write_in_a_transaction_then_panic(conn: &mut Connection) {
        let tx = worked(conn.transaction());
        worked(tx.execute_batch("CREATE TABLE half_done (x TEXT)"));
        panic!("while the lock is held and a transaction is open");
    }

    fn write_one_statement_then_panic(conn: &mut Connection) {
        worked(conn.execute_batch("CREATE TABLE written (x TEXT)"));
        panic!("while the lock is held");
    }

    /// The store works: no transaction is left open, and a write is kept.
    fn assert_usable<S>(store: &S, access: &Access<S>) {
        assert!((access.is_poisoned)(store), "the panic poisoned the lock");
        let conn = (access.conn)(store);
        assert!(conn.is_autocommit(), "no transaction is left open");
        worked(conn.execute_batch("CREATE TABLE after_panic (x TEXT)"));
        // free the lock: `tables_named` takes it again
        drop(conn);
        assert_eq!(tables_named(store, access, "after_panic"), 1);
    }

    /// How many tables are named `name`.
    fn tables_named<S>(store: &S, access: &Access<S>, name: &str) -> i64 {
        let sql = "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1";
        let conn = (access.conn)(store);
        worked(conn.query_row(sql, [name], |row| row.get(0)))
    }

    /// The value of `result`; an SQL error fails the test.
    fn worked<T>(result: rusqlite::Result<T>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("SQL failed in a test: {error}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_newer_files_are_refused() {
        let mut conn = open(None, &["CREATE TABLE a (x TEXT)"]).unwrap();
        migrate(
            &mut conn,
            &["CREATE TABLE a (x TEXT)", "CREATE TABLE b (y TEXT)"],
        )
        .unwrap();
        conn.execute("INSERT INTO b (y) VALUES ('ok')", []).unwrap();
        let err = migrate(&mut conn, &["CREATE TABLE a (x TEXT)"]).unwrap_err();
        assert!(err.contains("newer"), "{err}");
    }
}
