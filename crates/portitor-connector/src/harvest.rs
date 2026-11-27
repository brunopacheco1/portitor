// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Dataset harvest (ADR 0020): datasets come from a source outside the
//! connector into a read copy, and every run is stored as a job.
//!
//! - [`DatasetSource`] (port) gives a full snapshot of the provider's
//!   datasets; [`FileSource`] reads the datasets file.
//! - [`DatasetCheck`] (port) checks one entry; [`DspCheck`] holds the DSP
//!   and ODRL rules, and [`BothChecks`] adds another check (SHACL) after it.
//! - [`DatasetStore`] (port) keeps the stored datasets, the jobs and their
//!   errors; the database adapter is in `store/harvest.rs`.
//! - `run` decides what a run changes, without I/O; [`DatasetHarvester`] runs it:
//!   at start, then on a schedule, one run at a time.
//! - [`StoredCatalog`] is what the DSP code and the data plane read: the
//!   stored datasets, read from the store on each request (ADR 0029, D4),
//!   so every process that shares the database answers the same.
//!
//! The rule: the stored datasets change only when a run succeeds, and a
//! bad entry never replaces a good one.
//!
//! Several processes may share the store (ADR 0029). The scheduled run
//! takes a lease, so one process runs it per interval ([`HARVEST_LEASE`]);
//! the other runs (at start, after new rules, a bundle job) need none.
//! What keeps every run right is the store: a run is written only if no
//! other run was written since it read the store, and is otherwise
//! planned again from the stored copy ([`DatasetStore::commit`]). A lock
//! in the process (`busy`) keeps its own runs apart.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::time::Duration;

use portitor_dsp::Dataset;
use portitor_dsp::catalog::ListedDataset;
/// The error of every store port. The harvest keeps its read copy in the
/// same database as the negotiations, so it reuses their store error
/// (named after negotiations, but it means "the store failed").
pub use portitor_dsp::negotiation::provider::StoreError;
use portitor_dsp::policy::AccessPolicy;
use portitor_runtime::leases::Leases;
use portitor_runtime::panics::spawn_supervised;
use portitor_runtime::telemetry::metrics;

mod catalog;
mod check;
mod memory;
mod run;
mod source;

pub use catalog::{StoredCatalog, UpstreamSources};
pub use check::{
    ACCESS_POLICY_KEY, BothChecks, CHECKS_VERSION, CheckKind, CheckedDataset, DatasetCheck,
    DspCheck, EntryFailure,
};
pub use memory::InMemoryDatasetStore;
pub(crate) use run::count;
pub use run::{MAX_REPORT_RESULTS, MAX_STORED_ERRORS, SOURCE_KEY};
pub use source::{
    DatasetSource, EmptySource, FileSource, FixedSource, MAX_DATASETS_FILE_BYTES, Snapshot,
    SourceError,
};

use crate::ports::Shared;
use crate::system::now_secs;

/// How many jobs are kept; older ones are deleted with their errors.
pub const KEPT_JOBS: u64 = 100;

/// Default time between two runs: one hour (`PORTITOR_HARVEST_INTERVAL_SECONDS`).
pub const DEFAULT_INTERVAL_SECONDS: u64 = 3_600;

/// Where a dataset of the read copy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetState {
    /// In the catalog.
    Active,
    /// No longer in the source. Kept for transfers under agreements
    /// already made.
    Withdrawn,
    /// Fails the rules now in force (checked again after they changed).
    /// Kept for transfers under agreements already made.
    Hidden,
}

impl DatasetState {
    /// Every value, for [`DatasetState::from_name`]; the names are in
    /// [`DatasetState::name`] only.
    const ALL: [Self; 3] = [Self::Active, Self::Withdrawn, Self::Hidden];

    /// The name stored and shown: `active`, `withdrawn` or `hidden`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Withdrawn => "withdrawn",
            Self::Hidden => "hidden",
        }
    }

    /// The state with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// One dataset of the read copy.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDataset {
    /// The dataset's `@id`.
    pub id: String,
    /// The entry as the source gave it, without `portitor:source`. It is
    /// checked again when the rules change.
    pub entry: serde_json::Value,
    /// The dataset as the catalog shows it.
    pub content: Dataset,
    /// Where the data plane fetches its data (`portitor:source`). Never
    /// shown.
    pub upstream: Option<String>,
    /// Who may see the dataset in the catalog (`portitor:accessPolicy`,
    /// ADR 0019, amendment in 1.4); `None`: every member. Never shown in
    /// the catalog. It stays in `entry` too, so a check with new rules
    /// reads it again.
    pub access_policy: Option<AccessPolicy>,
    /// SHA-256 of the content and the upstream, hex.
    pub content_sha256: String,
    /// Active, withdrawn or hidden.
    pub state: DatasetState,
    /// The `dct:conformsTo` the check added, if any.
    pub conforms_to: Option<String>,
    /// When a run first added it, in seconds since 1970.
    pub first_seen: u64,
    /// When a run last changed it (content or state).
    pub last_changed: u64,
    /// The job that last changed it. The store sets it on commit.
    pub last_job: u64,
}

/// What a run harvested: datasets, or the authority's bundle. Both are
/// kept in the same job log, so an operator sees in one place when a new
/// bundle came in and what it did to the datasets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// The datasets (ADR 0020).
    Datasets,
    /// The schema and vocabulary bundle (ADR 0005): a check for a new
    /// version. Its counts are all 0.
    Bundle,
}

impl JobKind {
    /// Every value, for [`JobKind::from_name`].
    const ALL: [Self; 2] = [Self::Datasets, Self::Bundle];

    /// The name stored and shown: `datasets` or `bundle`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Datasets => "datasets",
            Self::Bundle => "bundle",
        }
    }

    /// The kind with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// What started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The connector started.
    Start,
    /// The schedule (`PORTITOR_HARVEST_INTERVAL_SECONDS`, or
    /// `PORTITOR_BUNDLE_REFRESH_SECONDS` for the bundle).
    Interval,
    /// The rules changed (for example a new bundle version), so the stored
    /// datasets were checked again.
    Bundle,
}

impl Trigger {
    /// Every value, for [`Trigger::from_name`]; the names are in
    /// [`Trigger::name`] only.
    const ALL: [Self; 3] = [Self::Start, Self::Interval, Self::Bundle];

    /// The name stored and shown: `start`, `interval` or `bundle`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Interval => "interval",
            Self::Bundle => "bundle",
        }
    }

    /// The trigger with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// Every entry passed.
    Succeeded,
    /// Some entries were rejected; the others were taken.
    Partial,
    /// The snapshot is the same as the last run's; nothing was checked.
    Unchanged,
    /// The source could not be read; the copy was left as it was.
    Failed,
}

impl JobOutcome {
    /// Every value, for [`JobOutcome::from_name`]; the names are in
    /// [`JobOutcome::name`] only.
    const ALL: [Self; 4] = [
        Self::Succeeded,
        Self::Partial,
        Self::Unchanged,
        Self::Failed,
    ];

    /// The name stored and shown.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Unchanged => "unchanged",
            Self::Failed => "failed",
        }
    }

    /// The outcome with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// What a run did, counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Entries in the snapshot (or stored datasets checked again).
    pub seen: u64,
    /// Datasets that entered the catalog (new, or back after a withdrawal).
    pub added: u64,
    /// Datasets whose content or state changed.
    pub updated: u64,
    /// Datasets that passed with the same content.
    pub unchanged: u64,
    /// Datasets no longer in the snapshot.
    pub withdrawn: u64,
    /// Entries that failed a check.
    pub rejected: u64,
}

/// A run, as stored (without its id, which the store gives).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    /// What it harvested.
    pub kind: JobKind,
    /// When it started, in seconds since 1970.
    pub started_at: u64,
    /// When it ended.
    pub finished_at: u64,
    /// What started it.
    pub trigger: Trigger,
    /// The source's name, for example `file:/etc/portitor/datasets.json`.
    pub source: String,
    /// SHA-256 of the snapshot, hex; none when the source failed or for
    /// a check of the stored datasets.
    pub snapshot_sha256: Option<String>,
    /// How it ended.
    pub outcome: JobOutcome,
    /// What it did.
    pub counts: Counts,
    /// One line for people.
    pub summary: String,
}

/// A stored run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarvestJob {
    /// The job id; a newer job has a higher id.
    pub id: u64,
    /// The rest.
    pub record: JobRecord,
}

/// Why one entry was rejected.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryError {
    /// The entry's position in the snapshot, from 0; none for a stored
    /// dataset checked again.
    pub position: Option<u64>,
    /// The dataset's `@id`, if it could be read.
    pub dataset_id: Option<String>,
    /// The check that failed.
    pub kind: CheckKind,
    /// What is wrong.
    pub message: String,
    /// The check's full report (a SHACL validation report), if any.
    pub report: Option<serde_json::Value>,
}

/// A stored job with its entry errors, in the order the run found them.
pub type JobWithErrors = (HarvestJob, Vec<EntryError>);

/// What the store keeps besides the datasets and jobs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HarvestStatus {
    /// The rules version ([`DatasetCheck::rules_version`]) the copy was
    /// last checked with.
    pub rules_version: Option<String>,
    /// SHA-256 of the last snapshot taken; none forces a full run.
    pub snapshot_sha256: Option<String>,
    /// How many runs were written: each commit counts it up. A run
    /// writes only if it is still the number it read before it read the
    /// datasets (compare-and-swap, [`DatasetStore::commit`]).
    pub version: u64,
}

/// Everything one run writes, in one transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    /// The job.
    pub job: JobRecord,
    /// The entries rejected.
    pub errors: Vec<EntryError>,
    /// Datasets added or changed (their `last_job` is set to the new job).
    pub writes: Vec<StoredDataset>,
    /// The status after the run. Its `version` is the one the run read
    /// before it read anything else; the store counts it up.
    pub status: HarvestStatus,
}

/// Port: where the read copy, the jobs and their errors are kept.
pub trait DatasetStore: Send + Sync {
    /// Every dataset of the copy, in any state, in id order.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn datasets(&self) -> Result<Vec<StoredDataset>, StoreError>;

    /// The catalog: the active datasets as the catalog shows them, with
    /// their access policies, in id order (the order of the bytes of their
    /// ids, on every database).
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn catalog(&self) -> Result<Vec<ListedDataset>, StoreError>;

    /// The access policies of the active datasets, each once, in no set
    /// order: what a catalog request needs to know which credentials to
    /// ask for.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn access_policies(&self) -> Result<Vec<AccessPolicy>, StoreError>;

    /// One dataset, in any state.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn dataset(&self, id: &str) -> Result<Option<StoredDataset>, StoreError>;

    /// The rules version and the last snapshot hash.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn status(&self) -> Result<HarvestStatus, StoreError>;

    /// Writes one run, all or nothing: the job (with a new id, one above
    /// the newest), its errors, the datasets and the status, and deletes
    /// the jobs older than the last [`KEPT_JOBS`] with their errors.
    /// Returns the new job's id.
    ///
    /// Several processes may run at once (ADR 0029): the run is written
    /// only if no other run was written since it read the status, which
    /// the status's `version` tells. Otherwise nothing is written and the
    /// answer is `None`: the run was planned from a copy that is no
    /// longer the stored one, and must be planned again.
    ///
    /// # Errors
    ///
    /// The store cannot be written; nothing was written.
    fn commit(&self, commit: &Commit) -> Result<Option<u64>, StoreError>;

    /// Up to `limit` jobs older than job `before` (all when `None`), newest
    /// first.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn jobs(&self, before: Option<u64>, limit: usize) -> Result<Vec<HarvestJob>, StoreError>;

    /// One job with its entry errors, in the order the run found them.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn job(&self, id: u64) -> Result<Option<JobWithErrors>, StoreError>;
}

/// The harvest store chosen at start (see [`Shared`]).
pub type AnyDatasetStore = Shared<dyn DatasetStore>;

/// The adapters behind the harvest (part of [`crate::Ports`]).
#[derive(Clone)]
pub struct HarvestPorts {
    /// Where the read copy and the jobs are kept.
    pub store: AnyDatasetStore,
    /// Where the datasets come from. `None`: the datasets file of
    /// [`crate::Config::datasets_file`], or no datasets without one.
    pub source: Option<Arc<dyn DatasetSource>>,
    /// A check run after the DSP rules ([`DspCheck`]) on each entry, for
    /// example the SHACL shapes of a bundle. `None`: the DSP rules only.
    pub extra_check: Option<Arc<dyn DatasetCheck>>,
}

impl std::fmt::Debug for HarvestPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HarvestPorts")
    }
}

impl Default for HarvestPorts {
    /// The datasets in memory, the source from the configuration, DSP
    /// rules only.
    fn default() -> Self {
        Self {
            store: Shared(Arc::new(InMemoryDatasetStore::default())),
            source: None,
            extra_check: None,
        }
    }
}

/// Runs harvests: at start, then on a schedule; one run at a time.
pub struct DatasetHarvester {
    source: Arc<dyn DatasetSource>,
    check: Arc<dyn DatasetCheck>,
    store: Arc<dyn DatasetStore>,
    /// Held while a run is going on.
    busy: Mutex<()>,
    /// Set once [`DatasetHarvester::start`] has run (readiness, ADR 0023).
    started: AtomicBool,
}

impl std::fmt::Debug for DatasetHarvester {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DatasetHarvester")
    }
}

/// What [`DatasetHarvester::run`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResult {
    /// The run ended and was stored.
    Done(HarvestJob),
    /// Another run was going on; this one was skipped.
    Busy,
    /// The store failed; nothing was written.
    StoreFailed(StoreError),
}

impl DatasetHarvester {
    /// A harvester that takes snapshots from `source`, checks each entry
    /// with `check` and keeps the copy in `store`. Call [`DatasetHarvester::start`]
    /// before serving.
    #[must_use]
    pub fn new(
        source: Arc<dyn DatasetSource>,
        check: Arc<dyn DatasetCheck>,
        store: Arc<dyn DatasetStore>,
    ) -> Self {
        Self {
            source,
            check,
            store,
            busy: Mutex::new(()),
            started: AtomicBool::new(false),
        }
    }

    /// The catalog the DSP code and the data plane use: read from the
    /// store on each call, never from memory.
    #[must_use]
    pub fn catalog(&self) -> StoredCatalog {
        StoredCatalog::new(Arc::clone(&self.store))
    }

    /// The store, for the management API.
    #[must_use]
    pub fn store(&self) -> &Arc<dyn DatasetStore> {
        &self.store
    }

    /// The start: checks the stored datasets again if the rules changed
    /// (a `bundle` job), then runs a harvest (a `start` job). A source
    /// that fails leaves the stored datasets (none on the first start).
    ///
    /// # Errors
    ///
    /// The stored datasets cannot be read.
    pub fn start(&self) -> Result<(), StoreError> {
        let _running = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        let rules_changed = self.recheck_if_rules_changed()?;
        // new rules: every entry of the source is checked again, even if
        // the snapshot is the same, as some rejected before may pass now
        if let Err(e) = self.run_now(Trigger::Start, rules_changed) {
            tracing::error!(
                error = %e,
                "harvest: cannot store the run; the catalog keeps the stored datasets"
            );
        }
        tracing::info!(datasets = self.catalog().active_count()?, "catalog ready");
        self.started.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Whether the start run ([`DatasetHarvester::start`]) is done: the
    /// catalog has been loaded and harvested once.
    #[must_use]
    pub fn started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    /// The rules changed while the connector runs (a new bundle version,
    /// ADR 0005): waits for a run going on, checks the stored datasets
    /// again (a `bundle` job), then runs a harvest that checks every entry
    /// of the source (also a `bundle` job), as some rejected before may
    /// pass now. Blocking.
    pub fn rules_changed(&self) -> RunResult {
        let _running = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = self.recheck_if_rules_changed() {
            tracing::error!(error = %e, "harvest: cannot check the stored datasets again");
        }
        match self.run_now(Trigger::Bundle, true) {
            Ok(job) => RunResult::Done(job),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "harvest: cannot store the run; the catalog is unchanged"
                );
                RunResult::StoreFailed(e)
            }
        }
    }

    /// Another process of the connector took a new bundle version, and
    /// this process moved to it (TD-55): checks the stored datasets again
    /// with the new rules, unless the store says they were checked with
    /// them already (by that process). The check clears the snapshot
    /// hash, so the next run checks every entry of the source. Blocking.
    pub fn rules_moved(&self) {
        let _running = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = self.recheck_if_rules_changed() {
            tracing::error!(error = %e, "harvest: cannot check the stored datasets again");
        }
    }

    /// Adds a run of the bundle harvester (kind `bundle`) to the job log.
    /// It writes no dataset and keeps the harvest status. Waits for a
    /// run going on, so the status it keeps is the latest. Blocking.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    pub fn record_bundle_run(&self, job: &JobRecord) -> Result<u64, StoreError> {
        let _running = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        until_committed("bundle job", || {
            let commit = Commit {
                job: job.clone(),
                errors: Vec::new(),
                writes: Vec::new(),
                status: self.store.status()?,
            };
            let id = self.store.commit(&commit)?;
            if id.is_some() {
                measure_run(&commit);
            }
            Ok(id)
        })
    }

    /// Checks the stored datasets again if the rules are not the ones
    /// they were last checked with. Returns whether they changed. A check
    /// that cannot be stored is logged; the caller goes on.
    fn recheck_if_rules_changed(&self) -> Result<bool, StoreError> {
        let rules = self.check.rules_version();
        let checked_with =
            |status: &HarvestStatus| status.rules_version.as_deref() == Some(rules.as_str());
        if checked_with(&self.store.status()?) {
            return Ok(false);
        }
        let rechecked = until_committed("check again", || {
            let status = self.store.status()?;
            if checked_with(&status) {
                // another process checked them with these rules meanwhile
                return Ok(Some(()));
            }
            self.recheck(&rules, status.version)
        });
        if let Err(e) = rechecked {
            tracing::error!(error = %e, "harvest: cannot check the stored datasets again");
        }
        Ok(true)
    }

    /// One run, unless one is going on. Blocking: it reads the source and
    /// writes the store.
    pub fn run(&self, trigger: Trigger) -> RunResult {
        // try_lock: a run that finds another one going on is skipped. A
        // lock left "poisoned" by a run that panicked is taken all the
        // same: it guards no data, it only says that a run is going on
        let _running = match self.busy.try_lock() {
            Ok(running) => running,
            Err(TryLockError::Poisoned(left_by_a_panic)) => left_by_a_panic.into_inner(),
            Err(TryLockError::WouldBlock) => {
                tracing::info!(
                    trigger = trigger.name(),
                    "harvest skipped: a run is going on"
                );
                return RunResult::Busy;
            }
        };
        match self.run_now(trigger, false) {
            Ok(job) => RunResult::Done(job),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "harvest: cannot store the run; the catalog is unchanged"
                );
                RunResult::StoreFailed(e)
            }
        }
    }

    /// Runs a harvest every `every`, in the background, the first one
    /// `every` after now (the start run is done by [`DatasetHarvester::start`]).
    /// A zero `every` means no schedule (`None`). Must be called inside
    /// the Tokio runtime.
    ///
    /// Several processes on one database run one harvest per interval
    /// between them (TD-56): a run needs the job's lease in `leases`
    /// ([`HARVEST_LEASE`]), renewed while it runs, and kept after it until
    /// the next run is due. A process that finds the lease held skips its
    /// turn. A process that stops in a run leaves its lease to end; the
    /// next tick of any process after that runs.
    #[must_use]
    pub fn schedule(
        self: &Arc<Self>,
        every: Duration,
        leases: &Leases,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if every.is_zero() {
            return None;
        }
        let harvester = Arc::clone(self);
        let leases = leases.clone();
        // supervised, as every task that runs as long as the process
        // (the closure makes the task anew after a panic, with its own
        // pointer to the harvester). A run that panics does not end the
        // schedule at all: it runs on its own thread, see below
        let schedule = move || {
            let harvester = Arc::clone(&harvester);
            let leases = leases.clone();
            async move {
                let first = tokio::time::Instant::now() + every;
                let mut ticks = tokio::time::interval_at(first, every);
                // a tick missed while a run was going on is skipped, not made up
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    ticks.tick().await;
                    run_if_due(&harvester, &leases, every).await;
                }
            }
        };
        Some(spawn_supervised("dataset_harvest", schedule))
    }

    /// Takes a snapshot, decides the changes, and commits them. `full`:
    /// check every entry even if the snapshot is unchanged. Planned again
    /// when another process wrote a run meanwhile ([`until_committed`]).
    fn run_now(&self, trigger: Trigger, full: bool) -> Result<HarvestJob, StoreError> {
        until_committed("harvest", || self.run_once(trigger, full))
    }

    /// One try of [`DatasetHarvester::run_now`]: `None` when another run
    /// was written since this one read the status.
    fn run_once(&self, trigger: Trigger, full: bool) -> Result<Option<HarvestJob>, StoreError> {
        let started_at = now_secs();
        let status = self.store.status()?;
        // the job record of this run: only how it ended differs
        let job = |snapshot_sha256, outcome, counts, summary| JobRecord {
            kind: JobKind::Datasets,
            started_at,
            finished_at: now_secs(),
            trigger,
            source: self.source.name(),
            snapshot_sha256,
            outcome,
            counts,
            summary,
        };
        let snapshot = match self.source.snapshot() {
            Ok(snapshot) => snapshot,
            Err(e) => {
                let summary = format!("source failed: {e}; the copy is unchanged");
                let failed = job(None, JobOutcome::Failed, Counts::default(), summary);
                return self.commit(failed, run::Plan::default(), status);
            }
        };
        if !full && status.snapshot_sha256.as_deref() == Some(snapshot.sha256.as_str()) {
            let counts = Counts {
                seen: run::count(snapshot.entries.len()),
                ..Counts::default()
            };
            let summary = "the snapshot is unchanged".to_owned();
            let unchanged = job(
                Some(snapshot.sha256),
                JobOutcome::Unchanged,
                counts,
                summary,
            );
            return self.commit(unchanged, run::Plan::default(), status);
        }
        let stored = self.stored_by_id()?;
        let plan = run::plan_harvest(&stored, snapshot.entries, self.check.as_ref(), started_at);
        let done = job(
            Some(snapshot.sha256.clone()),
            run::outcome_of(&plan.counts),
            plan.counts,
            run::summary(&plan.counts),
        );
        let after = HarvestStatus {
            rules_version: Some(self.check.rules_version()),
            snapshot_sha256: Some(snapshot.sha256),
            // what this run read, before the datasets
            version: status.version,
        };
        self.commit(done, plan, after)
    }

    /// Checks the stored active and hidden datasets again with the rules
    /// now in force, `rules` (a `bundle` job), and clears the snapshot hash
    /// so the next run checks every entry of the source again. `version`:
    /// the status's, read before the datasets. `None` when another run
    /// was written since.
    fn recheck(&self, rules: &str, version: u64) -> Result<Option<()>, StoreError> {
        let started_at = now_secs();
        let stored = self.stored_by_id()?;
        let after = HarvestStatus {
            rules_version: Some(rules.to_owned()),
            snapshot_sha256: None,
            version,
        };
        let plan = run::plan_recheck(&stored, self.check.as_ref(), started_at);
        if plan.counts.seen == 0 {
            // nothing stored to check: the start run that follows checks
            // every entry and remembers the rules
            return Ok(Some(()));
        }
        let job = JobRecord {
            kind: JobKind::Datasets,
            started_at,
            finished_at: now_secs(),
            trigger: Trigger::Bundle,
            source: "read copy".to_owned(),
            snapshot_sha256: None,
            outcome: run::outcome_of(&plan.counts),
            counts: plan.counts,
            summary: format!("rules changed to {rules}: {}", run::summary(&plan.counts)),
        };
        let committed = self.commit(job, plan, after)?;
        Ok(committed.map(|_| ()))
    }

    fn stored_by_id(&self) -> Result<BTreeMap<String, StoredDataset>, StoreError> {
        let mut by_id = BTreeMap::new();
        for dataset in self.store.datasets()? {
            by_id.insert(dataset.id.clone(), dataset);
        }
        Ok(by_id)
    }

    /// Commits a run and logs it. The next request reads what it wrote.
    /// `None`, with nothing written, when another run was written since
    /// this one read `status` ([`DatasetStore::commit`]).
    fn commit(
        &self,
        job: JobRecord,
        plan: run::Plan,
        status: HarvestStatus,
    ) -> Result<Option<HarvestJob>, StoreError> {
        let commit = Commit {
            job,
            errors: plan.errors,
            writes: plan.writes,
            status,
        };
        let Some(id) = self.store.commit(&commit)? else {
            return Ok(None);
        };
        log_run(id, &commit);
        measure_run(&commit);
        Ok(Some(HarvestJob {
            id,
            record: commit.job,
        }))
    }
}

/// How many times a run is planned when other processes keep writing
/// runs first. Each time one of them wins, so with fewer replicas than
/// this all of them get through, also when they start together.
const PLAN_TRIES: usize = 10;

/// Runs `attempt` until it writes its run. `None` from `attempt` says
/// that another run was written since it read the store (by another
/// process: in this one the `busy` lock keeps runs apart), so it plans
/// again from what is stored now. Why plan again rather than skip: the
/// other run may have been of another kind (a bundle job, a check with
/// new rules), so this one's work may still be needed; planned again, a
/// run whose work is done already finds it so (the same rules, the same
/// snapshot) and writes only its job.
fn until_committed<T>(
    what: &str,
    mut attempt: impl FnMut() -> Result<Option<T>, StoreError>,
) -> Result<T, StoreError> {
    for _ in 0..PLAN_TRIES {
        if let Some(done) = attempt()? {
            return Ok(done);
        }
        tracing::info!(
            run = what,
            "harvest: another process wrote a run first; planning again"
        );
    }
    Err(StoreError(format!(
        "{what}: other processes wrote runs first, {PLAN_TRIES} times"
    )))
}

/// The run's metrics: how it ended, and the rejected entries by check.
fn measure_run(commit: &Commit) {
    metrics::harvest_run_ended(commit.job.kind.name(), commit.job.outcome.name());
    for error in &commit.errors {
        metrics::datasets_rejected(error.kind.name(), 1);
    }
}

/// One line per run, one warning per rejected entry, and one per dataset
/// written without an upstream URL.
fn log_run(id: u64, commit: &Commit) {
    let job = &commit.job;
    for error in &commit.errors {
        tracing::warn!(
            job = id,
            position = error.position,
            dataset = error.dataset_id.as_deref().unwrap_or(""),
            check = error.kind.name(),
            error = %error.message,
            "harvest: dataset rejected"
        );
    }
    for dataset in &commit.writes {
        if dataset.state == DatasetState::Active && dataset.upstream.is_none() {
            tracing::warn!(
                job = id,
                dataset = %dataset.id,
                "harvest: dataset has no portitor:source; its transfers will be refused"
            );
        }
    }
    let counts = &job.counts;
    let outcome = job.outcome.name();
    if job.outcome == JobOutcome::Failed {
        tracing::warn!(
            job = id,
            trigger = job.trigger.name(),
            source = %job.source,
            outcome,
            summary = %job.summary,
            "harvest run"
        );
        return;
    }
    tracing::info!(
        job = id,
        trigger = job.trigger.name(),
        source = %job.source,
        outcome,
        seen = counts.seen,
        added = counts.added,
        updated = counts.updated,
        unchanged = counts.unchanged,
        withdrawn = counts.withdrawn,
        rejected = counts.rejected,
        "harvest run"
    );
}

/// The kind and the id of the lease of the scheduled dataset harvest in
/// the database (TD-56, `portitor_runtime::leases`).
pub const HARVEST_LEASE: (&str, &str) = ("job", "dataset_harvest");

/// One tick of [`DatasetHarvester::schedule`]: a run, if this process can
/// take the job's lease. After the run, the lease is kept until a second
/// before the next tick of this process, so that no process runs again
/// within the interval, and this process finds it ended at its next tick.
async fn run_if_due(harvester: &Arc<DatasetHarvester>, leases: &Leases, every: Duration) {
    let ticked = leases.now();
    let (kind, id) = HARVEST_LEASE;
    // held by another process in this interval, or the store failed
    // (logged): the next tick tries again
    let Some(lease) = leases.claim(kind, id).taken() else {
        tracing::debug!("harvest: another process runs it in this interval");
        return;
    };
    let runner = Arc::clone(harvester);
    // the run reads a file and writes the store: blocking work
    let ran = tokio::task::spawn_blocking(move || runner.run(Trigger::Interval));
    // a run that panicked: logged and counted; the schedule goes on with
    // the next tick
    if let Err(e) = ran.await {
        tracing::error!(error = %e, "harvest run stopped unexpectedly");
        metrics::task_panicked("dataset_harvest");
    }
    let next_due = ticked.saturating_add(every.as_secs()).saturating_sub(1);
    lease.hold_until(next_due);
}

#[cfg(test)]
mod tests;
