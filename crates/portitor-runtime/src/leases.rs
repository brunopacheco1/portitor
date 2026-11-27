// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Leases in the database: which replica does a piece of work now, so
//! that several replicas on one database do it once (ADR 0029, decision
//! 6; TD-53, TD-54, TD-56). A replica is one running process of a
//! component; in these docs "process" means a replica only in "process
//! id" ([`crate::system::process_id`]).
//!
//! A lease names a piece of work by a kind and an id (for example
//! `outbox:transfer:provider` and a message's id, or `job` and
//! `dataset_harvest`), who holds it and when it ends, in seconds since
//! 1970. The holder is one claim, not one replica: the replica's process
//! id and a random id made for each claim. So a replica that lost a
//! lease, and took the same work again, holds two different leases: the
//! first can neither renew nor end the second. A replica takes a lease
//! only when nobody holds it or the holder's lease has ended: one
//! statement decides, so of several replicas that try at once exactly one
//! gets it (compare-and-swap). The holder renews it while the work runs
//! ([`HeldLease`]); a replica that dies stops renewing, and once the
//! lease has ended another replica takes the work over.
//!
//! A lease can be lost while its work still runs: the replica stalled, or
//! the database did not answer (a failover) until the lease ended.
//! [`HeldLease::lost`] then fires, and the work stops, so that it does
//! not run twice: once here, once in the replica that took it over. It
//! fires when a renewal finds another holder, or when renewals failed
//! until the next one would come too late (about 20 seconds without one).
//!
//! Every lease in Portitor, each lasting [`LEASE_SECONDS`]:
//!
//! | Work | Where | Kind, id |
//! |------|-------|----------|
//! | Sending one saved DSP message (TD-53) | connector, `outbox.rs` | `outbox:<process>[:<role>]`, the message's DSP pid and digest (`lease_id`) |
//! | A running push of a provider's transfer (TD-54) | connector, `transfer.rs` | `push`, the provider pid |
//! | A running upload received as consumer (TD-54) | connector, `receiving.rs` | `upload`, the consumer pid |
//! | The scheduled dataset harvest (TD-56) | connector, `harvest.rs` | `job`, `dataset_harvest` |
//! | A Gaia-X compliance run (TD-56) | identity, `gaiax.rs` | `job`, `gaiax_compliance` |
//! | The credential status check (TD-20, 1.4) | identity, `status_check.rs` | `job`, `credential_status_check` |
//! | A step of a provider's transfer (TD-48) | connector, `portitor-dsp` `transfer/provider/flow_steps.rs` | own columns: `transfer.lease_holder`, `lease_until` |
//! | Sending one credential delivery (TD-57) | issuer, `delivery.rs` | own columns: `outbox.lease_holder`, `lease_until` |
//! | The use of a `SQLite` file by one process (ADR 0029, D3) | every component, `database/sqlite_lease.rs` | own table `portitor_process_lease`, one row |
//!
//! Why three of them do not use the `lease` table: the transfer's lease
//! is taken in the same compare-and-swap update that records the step on
//! the transfer's row (`portitor-dsp` decides it, and knows no lease
//! table), and it came in 1.2, before this table. The issuer's delivery
//! lease marks the outbox row itself, so that one query finds the oldest
//! delivery that is due and free; the issuer has no `lease` table. The
//! `SQLite` file lease guards the file itself and is taken before any
//! migration, so it cannot be a row of the schema it protects; it goes
//! with `SQLite` in 2.0.
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
//! Leases use each replica's own clock, as EDC does: the hosts' clocks
//! must be in sync (NTP; DEPLOYING).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::database::{Database, DbError, Executor, to_db_int};
use crate::system::{new_uuid, now_secs};
use crate::values;

/// How long a lease lasts after it was taken or renewed, in seconds: 30,
/// as the lease on a provider's transfer (TD-48) and on an issuer's
/// delivery (TD-57), and about EDC's default.
pub const LEASE_SECONDS: u64 = 30;

/// How often a [`HeldLease`] is renewed while its work runs: a third of
/// [`LEASE_SECONDS`], so one late renewal does not lose it.
pub const RENEW_EVERY: Duration = Duration::from_secs(10);

/// How long a lease row is kept after it ended, in seconds. A replica
/// that stopped never deletes its rows; [`LeaseStore::forget_ended`]
/// does, an hour later.
const KEPT_AFTER_END: u64 = 3600;

/// Port: where the leases are kept.
pub trait LeaseStore: Send + Sync {
    /// Gives the lease on (`kind`, `id`) to `holder` until `until`, if
    /// nobody holds it, or its lease ended at `now` or before. Returns
    /// whether `holder` got it. A holder whose lease still runs does not
    /// get it again: one piece of work runs once, also in one replica.
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
    /// it (it ended and another replica took it).
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

    /// Deletes the leases that ended before `before`: those of replicas
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
        // two replicas claim at once, the database lets one of them
        // change a row, and the other changes none
        let changed = self
            .execute(
                "INSERT INTO lease (kind, id, lease_holder, lease_until) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (kind, id) DO UPDATE
                     SET lease_holder = excluded.lease_holder, lease_until = excluded.lease_until
                     WHERE lease.lease_until <= $5",
                values![kind, id, holder, to_db_int(until), to_db_int(now)],
            )
            .map_err(lease_error)?;
        Ok(changed == 1)
    }

    fn extend(&self, kind: &str, id: &str, holder: &str, until: u64) -> Result<bool, String> {
        let changed = self
            .execute(
                "UPDATE lease SET lease_until = $4
                 WHERE kind = $1 AND id = $2 AND lease_holder = $3",
                values![kind, id, holder, to_db_int(until)],
            )
            .map_err(lease_error)?;
        Ok(changed == 1)
    }

    fn release(&self, kind: &str, id: &str, holder: &str) -> Result<(), String> {
        self.execute(
            "DELETE FROM lease WHERE kind = $1 AND id = $2 AND lease_holder = $3",
            values![kind, id, holder],
        )
        .map_err(lease_error)?;
        Ok(())
    }

    fn held(&self, kind: &str, now: u64) -> Result<Vec<String>, String> {
        let rows = self
            .query(
                "SELECT id FROM lease WHERE kind = $1 AND lease_until > $2",
                values![kind, to_db_int(now)],
            )
            .map_err(lease_error)?;
        let mut ids = Vec::with_capacity(rows.len());
        for row in &rows {
            ids.push(row.get(0).map_err(lease_error)?);
        }
        Ok(ids)
    }

    fn forget_ended(&self, before: u64) -> Result<u64, String> {
        self.execute(
            "DELETE FROM lease WHERE lease_until < $1",
            values![to_db_int(before)],
        )
        .map_err(lease_error)
    }
}

#[allow(clippy::needless_pass_by_value)] // used as map_err(lease_error)
fn lease_error(e: DbError) -> String {
    format!("leases: {e}")
}

/// What a lease reads the time from: seconds since 1970.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The leases of one replica: the store, the id this replica holds them
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
    /// pass one id per "replica". Each claim adds its own random id to
    /// it (module doc).
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

    /// The process id of this replica, which each claim's holder starts
    /// with.
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
    /// The answer says whether it was taken, or held by another (another
    /// replica, or another task of this one), or whether the store failed
    /// (logged): see [`Claim`].
    #[must_use]
    pub fn claim(&self, kind: &str, id: &str) -> Claim {
        // a holder of its own for this claim (module doc)
        let holder = format!("{} {}", self.holder, new_uuid());
        let now = self.now();
        let until = now.saturating_add(LEASE_SECONDS);
        match self.store.claim(kind, id, &holder, until, now) {
            Ok(true) => Claim::Taken(HeldLease::renewed(self.clone(), kind, id, holder, until)),
            Ok(false) => Claim::HeldByOther,
            Err(error) => {
                tracing::error!(%error, lease = kind, id, "cannot take a lease");
                Claim::Failed
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
    /// stopped replicas left. A failure is logged; the next call tries
    /// again.
    pub fn forget_old(&self) {
        let before = self.now().saturating_sub(KEPT_AFTER_END);
        if let Err(error) = self.store.forget_ended(before) {
            tracing::warn!(%error, "cannot delete old leases");
        }
    }
}

/// What [`Leases::claim`] found.
#[derive(Debug)]
pub enum Claim {
    /// The lease is ours: do the work, and stop if it is lost
    /// ([`HeldLease::lost`]).
    Taken(HeldLease),
    /// Another holder has it: another replica, or another task of this
    /// one. Leave the work to it.
    HeldByOther,
    /// The store failed (logged), so nobody knows who holds it. Try again
    /// later; a request that asked for the work can be answered "busy,
    /// try again" (503), not "conflict".
    Failed,
}

impl Claim {
    /// The lease, if it was taken: for callers that leave the work alone
    /// both when another holds it and when the store failed (a later try
    /// takes it up).
    #[must_use]
    pub fn taken(self) -> Option<HeldLease> {
        match self {
            Claim::Taken(lease) => Some(lease),
            Claim::HeldByOther | Claim::Failed => None,
        }
    }
}

/// A lease this replica holds. It is renewed in the background while
/// the value lives (inside a Tokio runtime). Dropping it stops the
/// renewals but leaves the lease to end on its own, at most
/// [`LEASE_SECONDS`] later: what a replica that failed to record its work
/// does, so that the work is tried again after that pause.
/// [`HeldLease::release`] ends it at once.
pub struct HeldLease {
    leases: Leases,
    kind: String,
    id: String,
    /// The holder written in the store: this claim's own (module doc).
    holder: String,
    /// Says `true` once the lease is lost.
    lost: watch::Receiver<bool>,
    renewals: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for HeldLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldLease")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

impl HeldLease {
    /// A lease just taken by `holder`, ending at `until`, with its
    /// renewals started when there is a Tokio runtime. Without one
    /// (plain tests) it is never renewed, and never reported lost.
    fn renewed(leases: Leases, kind: &str, id: &str, holder: String, until: u64) -> Self {
        let (tell_lost, lost) = watch::channel(false);
        let renewal = Renewal {
            leases: leases.clone(),
            kind: kind.to_owned(),
            id: id.to_owned(),
            holder: holder.clone(),
            until,
        };
        let renewals = tokio::runtime::Handle::try_current()
            .ok()
            .map(|runtime| runtime.spawn(renewal.run(tell_lost)));
        Self {
            leases,
            kind: kind.to_owned(),
            id: id.to_owned(),
            holder,
            lost,
            renewals,
        }
    }

    /// The id of the leased work.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// A signal that fires when this lease is lost (module doc). The
    /// work selects on it (`tokio::select!`) and stops when it fires.
    #[must_use]
    pub fn lost(&self) -> LeaseLost {
        LeaseLost(self.lost.clone())
    }

    /// Whether this lease was lost already.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        *self.lost.borrow()
    }

    /// Ends the lease now: the work is done, and any replica may take
    /// what comes next. A failure is logged; the lease then ends on its
    /// own. A lease that was lost is left to its new holder.
    pub fn release(mut self) {
        self.stop_renewing();
        let leases = &self.leases;
        if let Err(error) = leases.store.release(&self.kind, &self.id, &self.holder) {
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
            .extend(&self.kind, &self.id, &self.holder, until);
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

/// Fires when a [`HeldLease`] is lost ([`HeldLease::lost`]). Cloning it
/// is cheap.
#[derive(Debug, Clone)]
pub struct LeaseLost(watch::Receiver<bool>);

impl LeaseLost {
    /// Finishes once the lease is lost; waits forever otherwise (also
    /// after the lease was released or dropped: its work is over then).
    pub async fn wait(mut self) {
        // an error only says that the renewals stopped; a lost lease
        // says `true` before they stop
        let _ = self.0.wait_for(|lost| *lost).await;
        if !*self.0.borrow() {
            std::future::pending::<()>().await;
        }
    }

    /// Whether the lease was lost already.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        *self.0.borrow()
    }
}

/// What the renewals of one [`HeldLease`] need.
struct Renewal {
    leases: Leases,
    kind: String,
    id: String,
    holder: String,
    /// When the lease ends, as last taken or renewed.
    until: u64,
}

impl Renewal {
    /// Renews the lease every [`RENEW_EVERY`] until it is lost, then
    /// says so on `tell_lost` (and logs it). Stopped from outside when
    /// the lease is released or dropped.
    async fn run(mut self, tell_lost: watch::Sender<bool>) {
        loop {
            tokio::time::sleep(RENEW_EVERY).await;
            if !self.renew_once() {
                break;
            }
        }
        let _ = tell_lost.send(true);
    }

    /// One renewal. Returns false when the lease is lost: another holder
    /// has it, or the renewals failed until the next one would come after
    /// the lease ends. Stopping then, before it ends, leaves a margin of
    /// one [`RENEW_EVERY`] before another replica can take the work.
    fn renew_once(&mut self) -> bool {
        let (kind, id) = (&self.kind, &self.id);
        let until = self.leases.now().saturating_add(LEASE_SECONDS);
        match self.leases.store.extend(kind, id, &self.holder, until) {
            Ok(true) => {
                self.until = until;
                true
            }
            Ok(false) => {
                // the lease ended before a renewal came (a stalled
                // replica), and another replica took the work over
                tracing::warn!(lease = %kind, %id, "a lease was lost; another process took the work over; its work here stops");
                false
            }
            Err(error) => {
                let next_try = self.leases.now().saturating_add(RENEW_EVERY.as_secs());
                if next_try < self.until {
                    // the store may answer again at the next renewal
                    tracing::warn!(%error, lease = %kind, %id, "cannot renew a lease; trying again");
                    return true;
                }
                tracing::error!(%error, lease = %kind, %id, "cannot renew a lease before it ends; its work here stops");
                false
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
        let held = one.claim("push", "t-1").taken().unwrap();
        assert!(matches!(two.claim("push", "t-1"), Claim::HeldByOther));
        assert_eq!(two.held("push").len(), 1);
        // the renewal task runs (paused clock: the sleep passes at once)
        tokio::time::sleep(RENEW_EVERY + Duration::from_millis(10)).await;
        assert!(!held.is_lost());
        held.release();
        assert!(two.held("push").is_empty());
        let again = two.claim("push", "t-1").taken().unwrap();
        assert_eq!(again.id(), "t-1");
        // dropped: still held until it ends
        drop(again);
        assert!(one.claim("push", "t-1").taken().is_none());
    }

    /// A clock the test sets, in seconds since 1970.
    fn set_clock(at: u64) -> (Arc<AtomicU64>, Clock) {
        let time = Arc::new(AtomicU64::new(at));
        let read = Arc::clone(&time);
        (time, Arc::new(move || read.load(Ordering::SeqCst)))
    }

    /// A replica stalls (or its database did not answer) until its
    /// lease ended, and another claim took the work over, here in the
    /// same replica: the first holder's work is told to stop at its next
    /// renewal, and its late release leaves the new holder's lease alone.
    #[tokio::test(start_paused = true)]
    async fn a_lease_taken_over_stops_the_first_holder() {
        let db: Arc<dyn LeaseStore> = Arc::new(Database::in_memory(&MIGRATIONS).unwrap());
        let (time, clock) = set_clock(1_000);
        let leases = Leases::with_clock(Arc::clone(&db), "p-1", clock);
        let first = leases.claim("push", "t-1").taken().unwrap();
        let first_lost = first.lost();
        // the lease ended, unrenewed, and a second claim takes the work
        time.store(1_000 + LEASE_SECONDS, Ordering::SeqCst);
        let second = leases.claim("push", "t-1").taken().unwrap();
        // the first one's next renewal finds the other holder
        tokio::time::timeout(RENEW_EVERY * 2, first_lost.wait())
            .await
            .unwrap();
        assert!(first.is_lost());
        assert!(!second.is_lost());
        // the first one ends late: the second one still holds the work
        first.release();
        assert_eq!(leases.held("push").len(), 1);
        assert!(matches!(leases.claim("push", "t-1"), Claim::HeldByOther));
        second.release();
        assert!(leases.held("push").is_empty());
    }

    /// The leases in a database whose writes fail while `down` is set.
    struct Outage {
        db: Database,
        down: AtomicBool,
    }

    impl Outage {
        fn check(&self) -> Result<(), String> {
            if self.down.load(Ordering::SeqCst) {
                return Err("the database does not answer".to_owned());
            }
            Ok(())
        }
    }

    impl LeaseStore for Outage {
        fn claim(
            &self,
            kind: &str,
            id: &str,
            holder: &str,
            until: u64,
            now: u64,
        ) -> Result<bool, String> {
            self.check()?;
            self.db.claim(kind, id, holder, until, now)
        }
        fn extend(&self, kind: &str, id: &str, holder: &str, until: u64) -> Result<bool, String> {
            self.check()?;
            self.db.extend(kind, id, holder, until)
        }
        fn release(&self, kind: &str, id: &str, holder: &str) -> Result<(), String> {
            self.check()?;
            self.db.release(kind, id, holder)
        }
        fn held(&self, kind: &str, now: u64) -> Result<Vec<String>, String> {
            self.check()?;
            self.db.held(kind, now)
        }
        fn forget_ended(&self, before: u64) -> Result<u64, String> {
            self.check()?;
            self.db.forget_ended(before)
        }
    }

    /// The database does not answer (a failover): one failed renewal is
    /// tried again, but once the next try would come after the lease
    /// ends, the work is told to stop, before another replica can take
    /// it. A claim that cannot reach the database says so.
    #[tokio::test(start_paused = true)]
    async fn renewals_that_fail_until_the_lease_ends_stop_the_work() {
        let outage = Arc::new(Outage {
            db: Database::in_memory(&MIGRATIONS).unwrap(),
            down: AtomicBool::new(false),
        });
        let (time, clock) = set_clock(1_000);
        let lease_store = Arc::clone(&outage) as Arc<dyn LeaseStore>;
        let leases = Leases::with_clock(lease_store, "p-1", clock);
        let held = leases.claim("push", "t-1").taken().unwrap();
        outage.down.store(true, Ordering::SeqCst);
        assert!(matches!(leases.claim("push", "t-2"), Claim::Failed));
        // first renewal fails at 1010: the next one, at 1020, still
        // comes before the end (1030)
        time.store(1_010, Ordering::SeqCst);
        tokio::time::sleep(RENEW_EVERY + Duration::from_millis(10)).await;
        assert!(!held.is_lost());
        // second one fails at 1020: the next would come too late
        time.store(1_020, Ordering::SeqCst);
        tokio::time::timeout(RENEW_EVERY * 2, held.lost().wait())
            .await
            .unwrap();
        assert!(held.is_lost());
    }
}
