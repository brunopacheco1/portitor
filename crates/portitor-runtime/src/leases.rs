// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Leases in the database: which process does a piece of work now, so
//! that several processes on one database do it once (ADR 0029, decision
//! 6; TD-53, TD-54, TD-56).
//!
//! A lease names a piece of work by a kind and an id (for example
//! `outbox:transfer:provider` and a process id, or `job` and
//! `dataset_harvest`), the process that holds it (a random id made at
//! start, [`crate::system::process_id`]) and when it ends, in seconds
//! since 1970. A process takes a lease only when nobody holds it or the
//! holder's lease has ended: one statement decides, so of several
//! processes that try at once exactly one gets it (compare-and-swap).
//! The holder renews it while the work runs ([`HeldLease`]); a process
//! that dies stops renewing, and once the lease has ended another process
//! takes the work over.
//!
//! Modelled on Eclipse EDC's lease table (`edc_lease`, in
//! `extensions/common/sql/sql-lease`): one row per leased entity, with
//! who holds it and until when. Each component that uses leases has the
//! table in its own schema:
//!
//! ```sql
//! CREATE TABLE lease (
//!     kind         TEXT NOT NULL,
//!     id           TEXT NOT NULL,
//!     lease_holder TEXT NOT NULL,
//!     lease_until  BIGINT NOT NULL,  -- seconds since 1970 (INTEGER in SQLite)
//!     PRIMARY KEY (kind, id)
//! );
//! ```
//!
//! Leases use each process's own clock, as EDC does: the hosts' clocks
//! must be in sync (NTP; DEPLOYING).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use crate::database::{Database, DbError, Executor};
use crate::system::now_secs;
use crate::values;

/// How long a lease lasts after it was taken or renewed, in seconds: 30,
/// as the lease on a provider's transfer (TD-48) and on an issuer's
/// delivery (TD-57), and about EDC's default.
pub const LEASE_SECONDS: u64 = 30;

/// How often a [`HeldLease`] is renewed while its work runs: a third of
/// [`LEASE_SECONDS`], so one late renewal does not lose it.
pub const RENEW_EVERY: Duration = Duration::from_secs(10);

/// How long a lease row is kept after it ended, in seconds. A process
/// that stopped never deletes its rows; [`LeaseStore::forget_ended`]
/// does, an hour later.
const KEPT_AFTER_END: u64 = 3600;

/// Port: where the leases are kept.
pub trait LeaseStore: Send + Sync {
    /// Gives the lease on (`kind`, `id`) to `holder` until `until`, if
    /// nobody holds it, or its lease ended at `now` or before. Returns
    /// whether `holder` got it. A holder whose lease still runs does not
    /// get it again: one piece of work runs once, also in one process.
    ///
    /// # Errors
    /// The store fails; the text says why.
    fn claim(
        &self,
        kind: &str,
        id: &str,
        holder: &str,
        until: u64,
        now: u64,
    ) -> Result<bool, String>;

    /// Moves the end of `holder`'s lease on (`kind`, `id`) to `until`.
    /// Returns false, and changes nothing, when `holder` no longer holds
    /// it (it ended and another process took it).
    ///
    /// # Errors
    /// The store fails.
    fn extend(&self, kind: &str, id: &str, holder: &str, until: u64) -> Result<bool, String>;

    /// Ends `holder`'s lease on (`kind`, `id`) now. Nothing happens when
    /// `holder` does not hold it.
    ///
    /// # Errors
    /// The store fails.
    fn release(&self, kind: &str, id: &str, holder: &str) -> Result<(), String>;

    /// The ids of kind `kind` whose lease still runs at `now`, whoever
    /// holds them.
    ///
    /// # Errors
    /// The store fails.
    fn held(&self, kind: &str, now: u64) -> Result<Vec<String>, String>;

    /// Deletes the leases that ended before `before`: those of processes
    /// that stopped. Returns how many.
    ///
    /// # Errors
    /// The store fails.
    fn forget_ended(&self, before: u64) -> Result<u64, String>;
}

/// Adapter: the `lease` table of a component's database.
impl LeaseStore for Database {
    fn claim(
        &self,
        kind: &str,
        id: &str,
        holder: &str,
        until: u64,
        now: u64,
    ) -> Result<bool, String> {
        // one statement: a new row, or the row of a lease that ended. When
        // two processes claim at once, the database lets one of them
        // change a row, and the other changes none
        let mut db = self;
        let changed = db
            .execute(
                "INSERT INTO lease (kind, id, lease_holder, lease_until) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (kind, id) DO UPDATE
                     SET lease_holder = excluded.lease_holder, lease_until = excluded.lease_until
                     WHERE lease.lease_until <= $5",
                values![kind, id, holder, seconds(until), seconds(now)],
            )
            .map_err(lease_error)?;
        Ok(changed == 1)
    }

    fn extend(&self, kind: &str, id: &str, holder: &str, until: u64) -> Result<bool, String> {
        let mut db = self;
        let changed = db
            .execute(
                "UPDATE lease SET lease_until = $4
                 WHERE kind = $1 AND id = $2 AND lease_holder = $3",
                values![kind, id, holder, seconds(until)],
            )
            .map_err(lease_error)?;
        Ok(changed == 1)
    }

    fn release(&self, kind: &str, id: &str, holder: &str) -> Result<(), String> {
        let mut db = self;
        db.execute(
            "DELETE FROM lease WHERE kind = $1 AND id = $2 AND lease_holder = $3",
            values![kind, id, holder],
        )
        .map_err(lease_error)?;
        Ok(())
    }

    fn held(&self, kind: &str, now: u64) -> Result<Vec<String>, String> {
        let mut db = self;
        let rows = db
            .query(
                "SELECT id FROM lease WHERE kind = $1 AND lease_until > $2",
                values![kind, seconds(now)],
            )
            .map_err(lease_error)?;
        let mut ids = Vec::with_capacity(rows.len());
        for row in &rows {
            ids.push(row.get(0).map_err(lease_error)?);
        }
        Ok(ids)
    }

    fn forget_ended(&self, before: u64) -> Result<u64, String> {
        let mut db = self;
        db.execute(
            "DELETE FROM lease WHERE lease_until < $1",
            values![seconds(before)],
        )
        .map_err(lease_error)
    }
}

#[allow(clippy::needless_pass_by_value)] // used as map_err(lease_error)
fn lease_error(e: DbError) -> String {
    format!("leases: {e}")
}

/// Seconds since 1970 as the database keeps them: a signed 64-bit number.
fn seconds(at: u64) -> i64 {
    i64::try_from(at).unwrap_or(i64::MAX)
}

/// What a lease reads the time from: seconds since 1970.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The leases of one process: the store, the id this process holds them
/// under, and the clock. Cloning it is cheap.
#[derive(Clone)]
pub struct Leases {
    store: Arc<dyn LeaseStore>,
    holder: String,
    clock: Clock,
}

impl std::fmt::Debug for Leases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leases")
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

impl Leases {
    /// The leases in `store`, held as `holder`: the process id
    /// ([`crate::system::process_id`]) in a running component; tests
    /// pass one id per "replica".
    #[must_use]
    pub fn new(store: Arc<dyn LeaseStore>, holder: &str) -> Self {
        Self::with_clock(store, holder, Arc::new(now_secs))
    }

    /// [`Leases::new`], reading the time from `clock` instead of the
    /// system's clock: for tests on Tokio's paused clock.
    #[must_use]
    pub fn with_clock(store: Arc<dyn LeaseStore>, holder: &str, clock: Clock) -> Self {
        Self {
            store,
            holder: holder.to_owned(),
            clock,
        }
    }

    /// The time now, in seconds since 1970, as the leases read it.
    #[must_use]
    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// The id the leases are held under.
    #[must_use]
    pub fn holder(&self) -> &str {
        &self.holder
    }

    /// The store.
    #[must_use]
    pub fn store(&self) -> &Arc<dyn LeaseStore> {
        &self.store
    }

    /// Takes the lease on (`kind`, `id`) for [`LEASE_SECONDS`], if nobody
    /// holds it. Inside a Tokio runtime, the lease is then renewed every
    /// [`RENEW_EVERY`] until the [`HeldLease`] is released or dropped.
    /// `None` when another process holds it, or the store failed (logged):
    /// the work is then left to the holder, or to a later try.
    #[must_use]
    pub fn claim(&self, kind: &str, id: &str) -> Option<HeldLease> {
        let now = self.now();
        let until = now.saturating_add(LEASE_SECONDS);
        match self.store.claim(kind, id, &self.holder, until, now) {
            Ok(true) => Some(HeldLease::renewed(self.clone(), kind, id)),
            Ok(false) => None,
            Err(error) => {
                tracing::error!(%error, lease = kind, id, "cannot take a lease");
                None
            }
        }
    }

    /// The ids of kind `kind` whose lease runs now, whoever holds them.
    /// Empty when the store failed (logged): the claims that follow then
    /// decide.
    #[must_use]
    pub fn held(&self, kind: &str) -> BTreeSet<String> {
        match self.store.held(kind, self.now()) {
            Ok(ids) => ids.into_iter().collect(),
            Err(error) => {
                tracing::error!(%error, lease = kind, "cannot read the leases");
                BTreeSet::new()
            }
        }
    }

    /// Deletes the leases that ended more than an hour ago: those that
    /// stopped processes left. A failure is logged; the next call tries
    /// again.
    pub fn forget_old(&self) {
        let before = self.now().saturating_sub(KEPT_AFTER_END);
        if let Err(error) = self.store.forget_ended(before) {
            tracing::warn!(%error, "cannot delete old leases");
        }
    }
}

/// A lease this process holds. It is renewed in the background while
/// the value lives (inside a Tokio runtime). Dropping it stops the
/// renewals but leaves the lease to end on its own, at most
/// [`LEASE_SECONDS`] later: what a process that failed to record its work
/// does, so that the work is tried again after that pause.
/// [`HeldLease::release`] ends it at once.
pub struct HeldLease {
    leases: Leases,
    kind: String,
    id: String,
    renewals: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for HeldLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldLease")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl HeldLease {
    /// A lease just taken, with its renewals started when there is a
    /// Tokio runtime.
    fn renewed(leases: Leases, kind: &str, id: &str) -> Self {
        let renewals = tokio::runtime::Handle::try_current()
            .ok()
            .map(|runtime| runtime.spawn(renew(leases.clone(), kind.to_owned(), id.to_owned())));
        Self {
            leases,
            kind: kind.to_owned(),
            id: id.to_owned(),
            renewals,
        }
    }

    /// The id of the leased work.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Ends the lease now: the work is done, and any process may take
    /// what comes next. A failure is logged; the lease then ends on its
    /// own.
    pub fn release(mut self) {
        self.stop_renewing();
        let leases = &self.leases;
        if let Err(error) = leases.store.release(&self.kind, &self.id, &leases.holder) {
            tracing::warn!(%error, lease = %self.kind, id = %self.id, "cannot end a lease; it ends on its own");
        }
    }

    /// Stops renewing and keeps the lease until `until` (seconds since
    /// 1970): nobody takes the work before then. For a scheduled job,
    /// whose lease after a run says when the next run is due.
    pub fn hold_until(mut self, until: u64) {
        self.stop_renewing();
        let leases = &self.leases;
        let extended = leases
            .store
            .extend(&self.kind, &self.id, &leases.holder, until);
        if let Err(error) = extended {
            tracing::warn!(%error, lease = %self.kind, id = %self.id, "cannot keep a lease");
        }
    }

    fn stop_renewing(&mut self) {
        if let Some(renewals) = self.renewals.take() {
            renewals.abort();
        }
    }
}

impl Drop for HeldLease {
    fn drop(&mut self) {
        self.stop_renewing();
    }
}

/// Renews the lease on (`kind`, `id`) every [`RENEW_EVERY`], until it is
/// lost (logged) or the task is stopped.
async fn renew(leases: Leases, kind: String, id: String) {
    loop {
        tokio::time::sleep(RENEW_EVERY).await;
        let until = leases.now().saturating_add(LEASE_SECONDS);
        match leases.store.extend(&kind, &id, &leases.holder, until) {
            Ok(true) => {}
            Ok(false) => {
                // the lease ended before a renewal came (a stalled
                // process), and another process took the work over
                tracing::warn!(lease = %kind, %id, "a lease was lost; another process took the work over");
                return;
            }
            // the store may answer again at the next renewal
            Err(error) => tracing::warn!(%error, lease = %kind, %id, "cannot renew a lease"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::database::Migrations;

    const MIGRATIONS: Migrations = Migrations {
        component: "portitor-test",
        sqlite: &["CREATE TABLE lease (
                kind         TEXT NOT NULL,
                id           TEXT NOT NULL,
                lease_holder TEXT NOT NULL,
                lease_until  INTEGER NOT NULL,
                PRIMARY KEY (kind, id)
            )"],
        postgres: &["CREATE TABLE lease (
                kind         TEXT NOT NULL,
                id           TEXT NOT NULL,
                lease_holder TEXT NOT NULL,
                lease_until  BIGINT NOT NULL,
                PRIMARY KEY (kind, id)
            )"],
    };

    /// A lease goes to one holder until it ends; then another takes it.
    #[test]
    fn a_lease_is_held_by_one_process_until_it_ends() {
        for db in crate::testing::databases::all(&MIGRATIONS) {
            assert!(db.claim("job", "harvest", "p-1", 130, 100).unwrap());
            // nobody else gets it while it runs, not even p-1 again
            assert!(!db.claim("job", "harvest", "p-2", 159, 129).unwrap());
            assert!(!db.claim("job", "harvest", "p-1", 159, 129).unwrap());
            // another id of the same kind is free
            assert!(db.claim("job", "renewal", "p-2", 130, 100).unwrap());
            assert_eq!(db.held("job", 129).unwrap().len(), 2);
            // p-1 renews; p-2 cannot
            assert!(db.extend("job", "harvest", "p-1", 160).unwrap());
            assert!(!db.extend("job", "harvest", "p-2", 190).unwrap());
            // p-1 stops: at 160 its lease has ended, and p-2 takes over
            assert!(db.claim("job", "harvest", "p-2", 190, 160).unwrap());
            assert_eq!(db.held("job", 160).unwrap(), ["harvest"]);
            // p-1 comes back late: it holds nothing any more
            assert!(!db.extend("job", "harvest", "p-1", 200).unwrap());
            db.release("job", "harvest", "p-1").unwrap();
            assert_eq!(db.held("job", 170).unwrap(), ["harvest"]);
            // p-2 ends it: free at once
            db.release("job", "harvest", "p-2").unwrap();
            assert!(db.claim("job", "harvest", "p-1", 200, 170).unwrap());
            // old rows go
            assert_eq!(db.forget_ended(250).unwrap(), 2);
            assert!(db.held("job", 0).unwrap().is_empty());
        }
    }

    /// Two replicas (two pools on one PostgreSQL database) claim the same
    /// leases at once: each lease goes to exactly one of them.
    #[test]
    fn of_two_replicas_only_one_takes_each_lease() {
        let Some(url) = crate::testing::databases::postgres_url() else {
            return;
        };
        let settings = crate::testing::databases::fresh_postgres_settings(&url);
        let setting = crate::database::DatabaseSetting::Postgres(Box::new(settings));
        let replicas: Vec<Arc<Database>> = (0..2)
            .map(|_| Arc::new(Database::open(&setting, &MIGRATIONS).unwrap()))
            .collect();
        let threads: Vec<_> = replicas
            .iter()
            .enumerate()
            .map(|(n, db)| {
                let db = Arc::clone(db);
                std::thread::spawn(move || {
                    let mut won = 0;
                    for i in 0..50 {
                        let holder = format!("replica-{n}");
                        if db
                            .claim("outbox", &i.to_string(), &holder, 130, 100)
                            .unwrap()
                        {
                            won += 1;
                        }
                    }
                    won
                })
            })
            .collect();
        let won: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(won, 50);
        assert_eq!(replicas[0].held("outbox", 100).unwrap().len(), 50);
    }

    /// A held lease is renewed while it lives; dropped, it ends on its
    /// own; released, it ends at once.
    #[tokio::test(start_paused = true)]
    async fn a_held_lease_is_renewed_until_released() {
        let db: Arc<dyn LeaseStore> = Arc::new(Database::in_memory(&MIGRATIONS).unwrap());
        let one = Leases::new(Arc::clone(&db), "p-1");
        let two = Leases::new(Arc::clone(&db), "p-2");
        let held = one.claim("push", "t-1").unwrap();
        assert!(two.claim("push", "t-1").is_none());
        assert_eq!(two.held("push").len(), 1);
        // the renewal task runs (paused clock: the sleep passes at once)
        tokio::time::sleep(RENEW_EVERY + Duration::from_millis(10)).await;
        held.release();
        assert!(two.held("push").is_empty());
        let again = two.claim("push", "t-1").unwrap();
        assert_eq!(again.id(), "t-1");
        // dropped: still held until it ends
        drop(again);
        assert!(one.claim("push", "t-1").is_none());
    }
}
