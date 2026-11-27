// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! One process per `SQLite` file (ADR 0029, D3): the lease row.
//!
//! A `SQLite` file serves one process: two processes on one file would
//! each keep state in memory that the other does not see, and `SQLite`
//! itself is not built for writers in several containers. Before 1.3 this
//! rule was only written in DEPLOYING; now a second process refuses to
//! start.
//!
//! The process that opens the file writes a row in it: who holds the file
//! (a random id made when the file is opened) and until when, 30 seconds
//! from now. A thread of its own renews the row every 10 seconds, and the
//! row is removed when the database is closed. Another process that finds
//! a row whose time has not passed refuses to open the file. A process
//! that crashed leaves its row behind; the row's time then passes, and the
//! next process takes the file after at most 30 seconds.
//!
//! The table is made here, not by a component's migrations: it is the
//! same in all three images and does not exist on PostgreSQL, where
//! several processes share one database.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use super::{DbError, to_db_int};
use crate::system::{new_uuid, now_secs};

/// How long a lease lasts after it was taken or renewed, in seconds: as
/// long as every other lease (ADR 0029, section 6).
pub use crate::leases::LEASE_SECONDS;

/// How often the holder renews its lease: three times within a lease, so
/// one late renewal does not lose it.
const RENEW_EVERY: Duration = Duration::from_secs(10);

/// The table of the lease row. Its one row has `id` 1.
pub const LEASE_TABLE: &str = "portitor_process_lease";

/// The lease on a `SQLite` file, held while this value lives. Dropping it
/// stops the renewals and removes the row, so the next process can open
/// the file at once.
#[derive(Debug)]
pub struct ProcessLease {
    /// Closed on drop: that tells the renewing thread to stop.
    stop: Option<Sender<()>>,
    /// The renewing thread, waited for on drop so the row is gone when
    /// the drop returns.
    renewer: Option<JoinHandle<()>>,
}

impl Drop for ProcessLease {
    fn drop(&mut self) {
        // dropping the sender wakes the thread at once
        drop(self.stop.take());
        if let Some(renewer) = self.renewer.take() {
            let _ = renewer.join();
        }
    }
}

/// Takes the lease on the `SQLite` file at `path`, or refuses because
/// another process holds it. On success a thread renews the lease until
/// the returned value is dropped.
///
/// # Errors
/// Another process holds the file, or the file cannot be opened or
/// written.
pub fn take(path: &Path) -> Result<ProcessLease, DbError> {
    let mut conn = crate::sqlite::connect(Some(path)).map_err(DbError::new)?;
    let holder = new_uuid();
    take_on(&mut conn, path, &holder, now_secs())?;
    let (stop, stopped) = mpsc::channel::<()>();
    let file = path.to_path_buf();
    let renewer = std::thread::Builder::new()
        .name("sqlite-lease".to_owned())
        .spawn(move || {
            // until the sender is dropped: a timeout means "renew now"
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(RENEW_EVERY) {
                renew_logged(&conn, &file, &holder);
            }
            release(&conn, &holder);
        })
        .map_err(|e| DbError::new(format!("cannot start the SQLite lease thread: {e}")))?;
    Ok(ProcessLease {
        stop: Some(stop),
        renewer: Some(renewer),
    })
}

/// Takes the lease for `holder` at time `now`, in one transaction that
/// locks the file for writing first (`BEGIN IMMEDIATE`), so two processes
/// that start together cannot both take it.
fn take_on(conn: &mut Connection, path: &Path, holder: &str, now: u64) -> Result<(), DbError> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {LEASE_TABLE} (
            id          INTEGER PRIMARY KEY CHECK (id = 1),
            holder      TEXT NOT NULL,
            lease_until INTEGER NOT NULL
        )"
    ))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: Option<(String, i64)> = tx
        .query_row(
            &format!("SELECT holder, lease_until FROM {LEASE_TABLE} WHERE id = 1"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let now = to_db_int(now);
    if let Some((other, until)) = current
        && until > now
    {
        return Err(in_use(path, &other, until - now));
    }
    tx.execute(
        &format!(
            "INSERT INTO {LEASE_TABLE} (id, holder, lease_until) VALUES (1, ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET holder = ?1, lease_until = ?2"
        ),
        rusqlite::params![holder, now + to_db_int(LEASE_SECONDS)],
    )?;
    tx.commit()?;
    Ok(())
}

/// The error of a second process: what is wrong and what to do.
fn in_use(path: &Path, other: &str, seconds_left: i64) -> DbError {
    DbError::new(format!(
        "the SQLite file {} is in use by another process (lease holder {other}, \
         for {seconds_left} more seconds). A SQLite file serves one process only \
         (SQLite is deprecated since 1.3): to run several replicas, use PostgreSQL \
         (PORTITOR_DATABASE_URL). If the other process has stopped without closing \
         the file, start again after {seconds_left} seconds",
        path.display()
    ))
}

/// Renews the lease of `holder` until `now` + [`LEASE_SECONDS`]. Returns
/// whether `holder` still had it: `false` when another process took the
/// file over after this process missed its renewals for a whole lease
/// (for example while it was paused).
fn renew(conn: &Connection, holder: &str, now: u64) -> Result<bool, rusqlite::Error> {
    let changed = conn.execute(
        &format!("UPDATE {LEASE_TABLE} SET lease_until = ?2 WHERE id = 1 AND holder = ?1"),
        rusqlite::params![holder, to_db_int(now) + to_db_int(LEASE_SECONDS)],
    )?;
    Ok(changed == 1)
}

/// [`renew`], with what goes wrong in the log. The process goes on: the
/// renewal is tried again in [`RENEW_EVERY`].
fn renew_logged(conn: &Connection, file: &Path, holder: &str) {
    match renew(conn, holder, now_secs()) {
        Ok(true) => {}
        Ok(false) => tracing::error!(
            file = %file.display(),
            "SQLite: another process has taken this file over; two processes now use it, \
             which SQLite does not support: stop one of them"
        ),
        Err(e) => tracing::warn!(
            file = %file.display(),
            error = %e,
            "SQLite: the lease on the file cannot be renewed; trying again"
        ),
    }
}

/// Removes the lease row of `holder`, so the next process can open the
/// file at once. A row of another holder is left alone.
fn release(conn: &Connection, holder: &str) {
    let removed = conn.execute(
        &format!("DELETE FROM {LEASE_TABLE} WHERE id = 1 AND holder = ?1"),
        [holder],
    );
    if let Err(e) = removed {
        // the row then ends with its lease, at most LEASE_SECONDS later
        tracing::warn!(error = %e, "SQLite: the lease on the file cannot be released");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn lease_row(path: &Path) -> Option<(String, i64)> {
        let conn = Connection::open(path).unwrap();
        conn.query_row(
            &format!("SELECT holder, lease_until FROM {LEASE_TABLE} WHERE id = 1"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
    }

    #[test]
    fn a_second_holder_is_refused_until_the_lease_ends() {
        let dir = crate::testing::TempDir::new("sqlite-lease");
        let path = dir.path().join("a.db");
        let mut first = crate::sqlite::connect(Some(&path)).unwrap();
        take_on(&mut first, &path, "one", 1_000).unwrap();
        let mut second = crate::sqlite::connect(Some(&path)).unwrap();
        let refused = take_on(&mut second, &path, "two", 1_029).unwrap_err();
        assert!(
            refused.message.contains("in use by another process"),
            "{refused}"
        );
        assert!(refused.message.contains("lease holder one"), "{refused}");
        assert!(
            refused.message.contains("PORTITOR_DATABASE_URL"),
            "{refused}"
        );
        // the first holder stopped renewing (it crashed): at the end of
        // its lease the file is free
        take_on(&mut second, &path, "two", 1_030).unwrap();
        assert_eq!(lease_row(&path), Some(("two".to_owned(), 1_060)));
    }

    #[test]
    fn a_renewal_moves_the_end_and_a_lost_lease_is_seen() {
        let dir = crate::testing::TempDir::new("sqlite-lease");
        let path = dir.path().join("a.db");
        let mut conn = crate::sqlite::connect(Some(&path)).unwrap();
        take_on(&mut conn, &path, "one", 1_000).unwrap();
        assert!(renew(&conn, "one", 1_010).unwrap());
        assert_eq!(lease_row(&path), Some(("one".to_owned(), 1_040)));
        // another process took the file over after the lease ended
        let mut other = crate::sqlite::connect(Some(&path)).unwrap();
        take_on(&mut other, &path, "two", 1_041).unwrap();
        assert!(!renew(&conn, "one", 1_042).unwrap(), "lost");
        // releasing a lost lease leaves the new holder's row alone
        release(&conn, "one");
        assert_eq!(lease_row(&path), Some(("two".to_owned(), 1_071)));
    }

    #[test]
    fn dropping_the_lease_releases_the_file() {
        let dir = crate::testing::TempDir::new("sqlite-lease");
        let path = dir.path().join("a.db");
        let lease = take(&path).unwrap();
        assert!(lease_row(&path).is_some());
        assert!(take(&path).is_err(), "held");
        drop(lease);
        assert_eq!(lease_row(&path), None);
        let _again = take(&path).unwrap();
    }
}
