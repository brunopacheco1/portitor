// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The harvest's read copy, jobs and entry errors, in the same database
//! (ADR 0020). A run's changes are written in one transaction.

use portitor_dsp::catalog::ListedDataset;
use portitor_dsp::policy::{AccessPolicy, add_once};
use portitor_runtime::database::{DbError, Executor, Row, Value};
use portitor_runtime::values;

use super::{ConnectorStore, StoreError, col, json, parsed};
use crate::harvest::{
    CheckKind, Commit, Counts, DatasetState, DatasetStore, EntryError, HarvestJob, HarvestStatus,
    JobKind, JobOutcome, JobRecord, JobWithErrors, KEPT_JOBS, StoredDataset, Trigger, count,
};

/// A number as the database stores it.
fn int(n: u64) -> Result<i64, StoreError> {
    i64::try_from(n).map_err(|_| StoreError(format!("number {n} out of range")))
}

/// A stored number back.
fn uint(n: i64) -> Result<u64, StoreError> {
    u64::try_from(n).map_err(|_| StoreError(format!("stored number {n} out of range")))
}

/// A stored name back, with `from_name` of its type. `from_name` is a
/// function pointer (`fn(&str) -> Option<T>`): a plain function passed as
/// a value, such as `DatasetState::from_name`.
fn named<T>(what: &str, text: &str, from_name: fn(&str) -> Option<T>) -> Result<T, StoreError> {
    from_name(text).ok_or_else(|| StoreError(format!("stored {what} {text:?}")))
}

/// The columns of `dataset`; the job's id (`last_job`) is last, as the
/// last placeholder of a write ([`with_job_id`]).
const DATASET_COLUMNS: &str = "id, entry, content, upstream, content_sha256, state, \
     conforms_to, first_seen, last_changed, access_policy, last_job";

fn dataset_from_row(row: &Row) -> Result<StoredDataset, StoreError> {
    Ok(StoredDataset {
        id: col(row, 0)?,
        entry: parsed("dataset entry", &col::<String>(row, 1)?)?,
        content: parsed("dataset", &col::<String>(row, 2)?)?,
        upstream: col(row, 3)?,
        content_sha256: col(row, 4)?,
        state: named(
            "dataset state",
            &col::<String>(row, 5)?,
            DatasetState::from_name,
        )?,
        conforms_to: col(row, 6)?,
        first_seen: uint(col(row, 7)?)?,
        last_changed: uint(col(row, 8)?)?,
        access_policy: access_policy_from(col(row, 9)?)?,
        last_job: uint(col(row, 10)?)?,
    })
}

/// A stored access policy back; `None` when the column is NULL.
fn access_policy_from(text: Option<String>) -> Result<Option<AccessPolicy>, StoreError> {
    match text {
        Some(text) => Ok(Some(parsed("access policy", &text)?)),
        None => Ok(None),
    }
}

/// The columns of `harvest_job` but its `id`, which the database gives
/// (an insert names these; a `SELECT` puts `id` in front of them).
const JOB_COLUMNS_WITHOUT_ID: &str = "started_at, finished_at, run_trigger, source, \
     snapshot_sha256, outcome, seen, added, updated, unchanged, withdrawn, rejected, summary, \
     kind";

/// A row of `SELECT id, ` and then [`JOB_COLUMNS_WITHOUT_ID`].
fn job_from_row(row: &Row) -> Result<HarvestJob, StoreError> {
    Ok(HarvestJob {
        id: uint(col(row, 0)?)?,
        record: JobRecord {
            kind: named("job kind", &col::<String>(row, 14)?, JobKind::from_name)?,
            started_at: uint(col(row, 1)?)?,
            finished_at: uint(col(row, 2)?)?,
            trigger: named("trigger", &col::<String>(row, 3)?, Trigger::from_name)?,
            source: col(row, 4)?,
            snapshot_sha256: col(row, 5)?,
            outcome: named("outcome", &col::<String>(row, 6)?, JobOutcome::from_name)?,
            counts: Counts {
                seen: uint(col(row, 7)?)?,
                added: uint(col(row, 8)?)?,
                updated: uint(col(row, 9)?)?,
                unchanged: uint(col(row, 10)?)?,
                withdrawn: uint(col(row, 11)?)?,
                rejected: uint(col(row, 12)?)?,
            },
            summary: col(row, 13)?,
        },
    })
}

fn error_from_row(row: &Row) -> Result<EntryError, StoreError> {
    let position: Option<i64> = col(row, 0)?;
    let report: Option<String> = col(row, 4)?;
    Ok(EntryError {
        position: position.map(uint).transpose()?,
        dataset_id: col(row, 1)?,
        kind: named("check", &col::<String>(row, 2)?, CheckKind::from_name)?,
        message: col(row, 3)?,
        report: report.map(|r| parsed("report", &r)).transpose()?,
    })
}

fn status_from_row(row: &Row) -> Result<HarvestStatus, StoreError> {
    Ok(HarvestStatus {
        rules_version: col(row, 0)?,
        snapshot_sha256: col(row, 1)?,
        version: uint(col(row, 2)?)?,
    })
}

/// A run's rows, made before its transaction: the transaction only
/// writes them (it may run more than once, see
/// `ConnectorStore::transaction`). The job's id is known only inside the
/// transaction, so it is added at the end of each list of dataset and
/// error values there, as the last placeholder ([`with_job_id`]).
struct RunRows {
    job: Vec<Value>,
    datasets: Vec<Vec<Value>>,
    errors: Vec<Vec<Value>>,
    status: Vec<Value>,
    kept_jobs: i64,
}

/// The values of the job's row, without its id (the database gives it).
fn job_values(job: &JobRecord) -> Result<Vec<Value>, StoreError> {
    let counts = &job.counts;
    Ok(vec![
        Value::from(int(job.started_at)?),
        Value::from(int(job.finished_at)?),
        Value::from(job.trigger.name()),
        Value::from(&job.source),
        Value::from(job.snapshot_sha256.as_deref()),
        Value::from(job.outcome.name()),
        Value::from(int(counts.seen)?),
        Value::from(int(counts.added)?),
        Value::from(int(counts.updated)?),
        Value::from(int(counts.unchanged)?),
        Value::from(int(counts.withdrawn)?),
        Value::from(int(counts.rejected)?),
        Value::from(&job.summary),
        Value::from(job.kind.name()),
    ])
}

/// The values of a dataset's row, `$1` to `$10`, in the order of
/// [`DATASET_COLUMNS`]; the job's id (its `last_job`) is added as `$11`.
fn dataset_values(dataset: &StoredDataset) -> Result<Vec<Value>, StoreError> {
    let access_policy = dataset.access_policy.as_ref().map(json).transpose()?;
    Ok(vec![
        Value::from(&dataset.id),
        Value::from(json(&dataset.entry)?),
        Value::from(json(&dataset.content)?),
        Value::from(dataset.upstream.as_deref()),
        Value::from(&dataset.content_sha256),
        Value::from(dataset.state.name()),
        Value::from(dataset.conforms_to.as_deref()),
        Value::from(int(dataset.first_seen)?),
        Value::from(int(dataset.last_changed)?),
        Value::from(access_policy.as_deref()),
    ])
}

/// The values of an entry error's row, `$1` to `$6`; the job's id is
/// added as `$7`.
fn error_values(seq: usize, error: &EntryError) -> Result<Vec<Value>, StoreError> {
    Ok(vec![
        Value::from(int(count(seq))?),
        Value::from(error.position.map(int).transpose()?),
        Value::from(error.dataset_id.as_deref()),
        Value::from(error.kind.name()),
        Value::from(&error.message),
        Value::from(error.report.as_ref().map(json).transpose()?),
    ])
}

impl RunRows {
    fn of(commit: &Commit) -> Result<Self, StoreError> {
        let mut datasets = Vec::with_capacity(commit.writes.len());
        for dataset in &commit.writes {
            datasets.push(dataset_values(dataset)?);
        }
        let mut errors = Vec::with_capacity(commit.errors.len());
        for (seq, error) in commit.errors.iter().enumerate() {
            errors.push(error_values(seq, error)?);
        }
        let status = &commit.status;
        Ok(Self {
            job: job_values(&commit.job)?,
            datasets,
            errors,
            status: vec![
                Value::from(status.rules_version.as_deref()),
                Value::from(status.snapshot_sha256.as_deref()),
                Value::from(int(status.version)?),
            ],
            kept_jobs: int(KEPT_JOBS)?,
        })
    }
}

/// `values` with the job's id added at the end, as the last placeholder.
fn with_job_id(values: &[Value], job_id: i64) -> Vec<Value> {
    let mut all = Vec::with_capacity(values.len() + 1);
    all.extend_from_slice(values);
    all.push(Value::Int(job_id));
    all
}

/// Writes one run in the transaction `tx` and returns the job's id;
/// `None`, with nothing written, when another run was written since this
/// one read the status (`DatasetStore::commit`).
///
/// The status is written first, and only if its version is still the one
/// the run read (compare-and-swap). Of two processes that write a run at
/// once, one changes the row; the other changes none (or, on PostgreSQL,
/// collides and runs again, and then changes none).
///
/// The database gives the job's id (TD-56): the row id of `SQLite`, the
/// identity column of PostgreSQL. Two processes that write a run at once
/// get two ids, never the same one, and no process keeps a count of its
/// own.
fn write_run(tx: &dyn Executor, rows: &RunRows) -> Result<Option<i64>, DbError> {
    let status_written = tx.execute(
        "UPDATE harvest_state
         SET rules_version = $1, snapshot_sha256 = $2, version = version + 1
         WHERE id = 1 AND version = $3",
        &rows.status,
    )?;
    if status_written == 0 {
        return Ok(None);
    }
    let insert_job = format!(
        "INSERT INTO harvest_job ({JOB_COLUMNS_WITHOUT_ID})
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING id"
    );
    let id = match tx.query_one(&insert_job, &rows.job)? {
        Some(row) => row.get::<i64>(0)?,
        None => return Err(DbError::new("the new harvest job has no id")),
    };
    for dataset in &rows.datasets {
        write_dataset(tx, &with_job_id(dataset, id))?;
    }
    for error in &rows.errors {
        tx.execute(
            "INSERT INTO harvest_error
                 (seq, entry_position, dataset_id, kind, message, report, job_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &with_job_id(error, id),
        )?;
    }
    // keep the newest KEPT_JOBS jobs. Ids count up but may have gaps (a
    // PostgreSQL identity skips the ids of transactions that ran again),
    // so the oldest kept job is looked up, not computed
    let oldest_kept = match tx.query_one(
        "SELECT id FROM harvest_job ORDER BY id DESC LIMIT 1 OFFSET $1",
        values![rows.kept_jobs - 1],
    )? {
        Some(row) => row.get::<i64>(0)?,
        // fewer jobs than that: nothing to delete
        None => 0,
    };
    tx.execute(
        "DELETE FROM harvest_error WHERE job_id < $1",
        values![oldest_kept],
    )?;
    tx.execute(
        "DELETE FROM harvest_job WHERE id < $1",
        values![oldest_kept],
    )?;
    Ok(Some(id))
}

/// Writes one dataset: an update of its row, or a new row. (Two plain
/// statements rather than an upsert; they run in the run's transaction.)
/// `values`: the dataset's columns, in the order of [`DATASET_COLUMNS`],
/// the job's id (`last_job`) last.
fn write_dataset(tx: &dyn Executor, values: &[Value]) -> Result<(), DbError> {
    let updated = tx.execute(
        "UPDATE dataset
         SET entry = $2, content = $3, upstream = $4, content_sha256 = $5, state = $6,
             conforms_to = $7, first_seen = $8, last_changed = $9, access_policy = $10,
             last_job = $11
         WHERE id = $1",
        values,
    )?;
    if updated == 0 {
        let sql = format!(
            "INSERT INTO dataset ({DATASET_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        );
        tx.execute(&sql, values)?;
    }
    Ok(())
}

/// The catalog's form of a dataset, from its `id`, `content` and
/// `access_policy` columns.
fn listed_from_row(row: &Row) -> Result<(String, ListedDataset), StoreError> {
    let id: String = col(row, 0)?;
    let listed = ListedDataset {
        dataset: parsed("dataset", &col::<String>(row, 1)?)?,
        access_policy: access_policy_from(col(row, 2)?)?,
    };
    Ok((id, listed))
}

impl DatasetStore for ConnectorStore {
    fn datasets(&self) -> Result<Vec<StoredDataset>, StoreError> {
        let sql = format!("SELECT {DATASET_COLUMNS} FROM dataset");
        let mut datasets = self.query(&sql, values![], dataset_from_row)?;
        // sorted here, not with ORDER BY: PostgreSQL sorts text by the
        // database's collation, which may differ from the bytes' order
        datasets.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(datasets)
    }

    fn catalog(&self) -> Result<Vec<ListedDataset>, StoreError> {
        // read on every catalog request (ADR 0029, D4): only the columns
        // the catalog needs
        let mut active = self.query(
            "SELECT id, content, access_policy FROM dataset WHERE state = $1",
            values![DatasetState::Active.name()],
            listed_from_row,
        )?;
        // the order of the bytes of the ids, as for `datasets`
        active.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(active.into_iter().map(|(_, listed)| listed).collect())
    }

    fn access_policies(&self) -> Result<Vec<AccessPolicy>, StoreError> {
        // read on every catalog request: the policies only, each once
        let texts = self.query(
            "SELECT DISTINCT access_policy FROM dataset
             WHERE state = $1 AND access_policy IS NOT NULL",
            values![DatasetState::Active.name()],
            |row| col::<String>(row, 0),
        )?;
        let mut policies: Vec<AccessPolicy> = Vec::new();
        for text in texts {
            let policy: AccessPolicy = parsed("access policy", &text)?;
            // the same policy may be written in two ways (key order)
            add_once(&mut policies, policy);
        }
        Ok(policies)
    }

    fn dataset(&self, id: &str) -> Result<Option<StoredDataset>, StoreError> {
        let sql = format!("SELECT {DATASET_COLUMNS} FROM dataset WHERE id = $1");
        self.query_first(&sql, values![id], dataset_from_row)
    }

    fn status(&self) -> Result<HarvestStatus, StoreError> {
        // the one row of harvest_state: both columns may be NULL
        let found = self.query_first(
            "SELECT rules_version, snapshot_sha256, version FROM harvest_state WHERE id = 1",
            values![],
            status_from_row,
        )?;
        // no row (cannot happen after the migration): nothing known yet
        Ok(found.unwrap_or_default())
    }

    fn commit(&self, commit: &Commit) -> Result<Option<u64>, StoreError> {
        let rows = RunRows::of(commit)?;
        // on any error nothing is written
        let id = self.transaction(|tx| write_run(tx, &rows))?;
        id.map(uint).transpose()
    }

    fn jobs(&self, before: Option<u64>, limit: usize) -> Result<Vec<HarvestJob>, StoreError> {
        // no `before`: from the newest. The CAST gives PostgreSQL the type
        // of $1, which it cannot tell from `$1 IS NULL`
        let sql = format!(
            "SELECT id, {JOB_COLUMNS_WITHOUT_ID} FROM harvest_job
             WHERE CAST($1 AS BIGINT) IS NULL OR id < $1
             ORDER BY id DESC LIMIT $2"
        );
        let before = before.map(int).transpose()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(&sql, values![before, limit], job_from_row)
    }

    fn job(&self, id: u64) -> Result<Option<JobWithErrors>, StoreError> {
        let id = int(id)?;
        let sql = format!("SELECT id, {JOB_COLUMNS_WITHOUT_ID} FROM harvest_job WHERE id = $1");
        let Some(job) = self.query_first(&sql, values![id], job_from_row)? else {
            return Ok(None);
        };
        let errors = self.query(
            "SELECT entry_position, dataset_id, kind, message, report FROM harvest_error
             WHERE job_id = $1 ORDER BY seq",
            values![id],
            error_from_row,
        )?;
        Ok(Some((job, errors)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use portitor_dsp::catalog::CatalogDatasets as _;
    use portitor_dsp::policy::CredentialTypes;
    use serde_json::json;

    use super::*;
    use crate::harvest::{DatasetHarvester, DspCheck, FixedSource, RunResult};
    use crate::store::on_every_database;

    fn entry(id: &str) -> serde_json::Value {
        json!({
            "@id": id, "@type": "Dataset",
            "hasPolicy": [{"@id": format!("{id}:offer"), "@type": "Offer", "permission": [{"action": "use"}]}],
            "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}],
            "portitor:source": format!("http://upstream.example/{id}")
        })
    }

    fn harvester(store: &Arc<ConnectorStore>, entries: Vec<serde_json::Value>) -> DatasetHarvester {
        let check = DspCheck::new("https://p.example/dsp/2025-1", &CredentialTypes::default());
        let harvester = DatasetHarvester::new(
            Arc::new(FixedSource::new(entries)),
            Arc::new(check),
            Arc::clone(store) as _,
        );
        harvester.start().unwrap();
        harvester
    }

    /// Two processes ("replicas") on one PostgreSQL database write runs
    /// at once: each run gets its own id from the database (TD-56).
    #[test]
    fn runs_of_two_replicas_get_different_ids() {
        let Some(replicas) = crate::store::two_replicas() else {
            return;
        };
        let threads: Vec<_> = replicas
            .into_iter()
            .map(|store| {
                std::thread::spawn(move || {
                    let store = Arc::new(store);
                    let harvester = harvester(&store, vec![entry("a")]);
                    for _ in 0..5 {
                        assert!(matches!(
                            harvester.run(Trigger::Interval),
                            RunResult::Done(_)
                        ));
                    }
                    store
                })
            })
            .collect();
        let stores: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let jobs = stores[0].jobs(None, 100).unwrap();
        // the start run and 5 more, on each replica
        assert_eq!(jobs.len(), 12, "{jobs:?}");
        let ids: std::collections::BTreeSet<u64> = jobs.iter().map(|job| job.id).collect();
        assert_eq!(ids.len(), 12, "no id given twice: {ids:?}");
    }

    #[test]
    fn the_copy_and_the_jobs_survive_a_restart() {
        on_every_database(the_copy_and_the_jobs_survive_a_restart_on);
    }

    fn the_copy_and_the_jobs_survive_a_restart_on(store: &ConnectorStore) {
        let store = Arc::new(store.clone());
        let mut broken = entry("b");
        broken["hasPolicy"] = json!([]);
        harvester(&store, vec![entry("a"), broken]);
        let stored = store.datasets().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, DatasetState::Active);
        assert_eq!(
            stored[0].upstream.as_deref(),
            Some("http://upstream.example/a")
        );
        assert_eq!(stored[0].last_job, 1);
        let (job, errors) = store.job(1).unwrap().unwrap();
        assert_eq!(job.record.outcome, JobOutcome::Partial);
        assert_eq!(job.record.counts.rejected, 1);
        assert_eq!(errors[0].kind, CheckKind::Dsp);
        assert_eq!(errors[0].position, Some(1));
        let status = store.status().unwrap();
        assert!(status.rules_version.unwrap().starts_with("dsp("));
        assert_eq!(status.snapshot_sha256, job.record.snapshot_sha256);

        // a restart with a source that is gone: the stored copy is served
        let check = DspCheck::new("https://p.example/dsp/2025-1", &CredentialTypes::default());
        let restarted = DatasetHarvester::new(
            Arc::new(crate::harvest::FileSource::new("/nonexistent.json")),
            Arc::new(check),
            Arc::clone(&store) as _,
        );
        restarted.start().unwrap();
        assert_eq!(restarted.catalog().all().unwrap().len(), 1);
        assert_eq!(
            store.jobs(None, 10).unwrap()[0].record.outcome,
            JobOutcome::Failed
        );
    }

    /// The access policy is stored with its dataset, read back, and
    /// listed once for the catalog (ADR 0019, amendment in 1.4); a
    /// dataset without one has none.
    #[test]
    fn the_access_policy_round_trips() {
        on_every_database(the_access_policy_round_trips_on);
    }

    fn the_access_policy_round_trips_on(store: &ConnectorStore) {
        let store = Arc::new(store.clone());
        let policy = json!({"permission": [{"action": "use", "constraint": [
            {"leftOperand": "MembershipCredential.role", "operator": "eq", "rightOperand": "lab"},
            {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2030-01-01"}]}]});
        let mut first_with_policy = entry("a");
        first_with_policy[crate::harvest::ACCESS_POLICY_KEY] = policy.clone();
        let mut second_with_policy = entry("b");
        second_with_policy[crate::harvest::ACCESS_POLICY_KEY] = policy.clone();
        let harvester = harvester(
            &store,
            vec![first_with_policy, second_with_policy, entry("c")],
        );
        let expected: AccessPolicy = serde_json::from_value(policy).unwrap();

        let stored = store.dataset("a").unwrap().unwrap();
        assert_eq!(stored.access_policy.as_ref(), Some(&expected));
        // kept in the entry, for a check with new rules; not in the content
        assert!(
            stored
                .entry
                .get(crate::harvest::ACCESS_POLICY_KEY)
                .is_some()
        );
        assert!(!json(&stored.content).unwrap().contains("accessPolicy"));
        assert_eq!(store.dataset("c").unwrap().unwrap().access_policy, None);

        let listed = store.catalog().unwrap();
        let policies: Vec<_> = listed.iter().map(|l| l.access_policy.is_some()).collect();
        assert_eq!(policies, vec![true, true, false]);
        assert_eq!(store.access_policies().unwrap(), vec![expected.clone()]);

        // a check with new rules (another credential type) reads it again
        // from the stored entry, at start
        let mut types = CredentialTypes::default();
        types.add("DataPermitCredential", "scope:permit");
        let restarted = DatasetHarvester::new(
            Arc::new(crate::harvest::FileSource::new("/nonexistent.json")),
            Arc::new(DspCheck::new("https://p.example/dsp/2025-1", &types)),
            Arc::clone(&store) as _,
        );
        restarted.start().unwrap();
        drop(harvester);
        assert_eq!(
            store.dataset("b").unwrap().unwrap().access_policy,
            Some(expected)
        );
    }

    /// A datasets source the test changes between runs.
    #[derive(Default)]
    struct Switch(std::sync::Mutex<Vec<serde_json::Value>>);

    impl crate::harvest::DatasetSource for Switch {
        fn name(&self) -> String {
            "switch".to_owned()
        }

        fn snapshot(&self) -> Result<crate::harvest::Snapshot, crate::harvest::SourceError> {
            let entries = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            Ok(crate::harvest::Snapshot {
                sha256: serde_json::to_string(&entries).unwrap(),
                entries,
            })
        }
    }

    /// TD-13: a dataset whose access policy changes in the source gets the
    /// new one on the next harvest; one whose access policy is removed
    /// has none after it, and the catalog's list follows.
    #[test]
    fn a_changed_or_removed_access_policy_is_written_on_the_next_harvest() {
        on_every_database(a_changed_or_removed_access_policy_is_written_on_the_next_harvest_on);
    }

    fn a_changed_or_removed_access_policy_is_written_on_the_next_harvest_on(
        store: &ConnectorStore,
    ) {
        let store = Arc::new(store.clone());
        let policy = |role: &str| {
            json!({"permission": [{"action": "use", "constraint": [
                {"leftOperand": "MembershipCredential.role", "operator": "eq", "rightOperand": role}]}]})
        };
        let with_policy = |role: &str| {
            let mut dataset_a = entry("a");
            dataset_a[crate::harvest::ACCESS_POLICY_KEY] = policy(role);
            dataset_a
        };
        let source = Arc::new(Switch::default());
        *source.0.lock().unwrap() = vec![with_policy("lab")];
        let check = DspCheck::new("https://p.example/dsp/2025-1", &CredentialTypes::default());
        let harvester = DatasetHarvester::new(
            Arc::clone(&source) as _,
            Arc::new(check),
            Arc::clone(&store) as _,
        );
        harvester.start().unwrap();
        let lab: AccessPolicy = serde_json::from_value(policy("lab")).unwrap();
        assert_eq!(
            store.dataset("a").unwrap().unwrap().access_policy,
            Some(lab)
        );

        // changed
        *source.0.lock().unwrap() = vec![with_policy("hospital")];
        assert!(matches!(
            harvester.run(Trigger::Interval),
            RunResult::Done(_)
        ));
        let hospital: AccessPolicy = serde_json::from_value(policy("hospital")).unwrap();
        let stored = store.dataset("a").unwrap().unwrap();
        assert_eq!(stored.access_policy, Some(hospital.clone()));
        assert_eq!(
            stored.entry.get(crate::harvest::ACCESS_POLICY_KEY),
            Some(&policy("hospital"))
        );
        assert_eq!(store.access_policies().unwrap(), vec![hospital]);

        // removed
        *source.0.lock().unwrap() = vec![entry("a")];
        assert!(matches!(
            harvester.run(Trigger::Interval),
            RunResult::Done(_)
        ));
        let stored = store.dataset("a").unwrap().unwrap();
        assert_eq!(stored.access_policy, None);
        assert_eq!(stored.entry.get(crate::harvest::ACCESS_POLICY_KEY), None);
        assert_eq!(store.access_policies().unwrap(), Vec::new());
        let listed = store.catalog().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].access_policy.is_none());
    }

    /// Makes every write of an entry error fail, as a full disk would:
    /// a trigger, written in each database's own SQL.
    fn make_entry_errors_fail(store: &ConnectorStore) {
        let statements: &[&str] = if store.database.is_postgres() {
            &[
                "CREATE FUNCTION disk_full() RETURNS trigger LANGUAGE plpgsql
                 AS $$ BEGIN RAISE EXCEPTION 'disk full'; END $$",
                "CREATE TRIGGER fail_errors BEFORE INSERT ON harvest_error
                 FOR EACH ROW EXECUTE FUNCTION disk_full()",
            ]
        } else {
            &["CREATE TRIGGER fail_errors BEFORE INSERT ON harvest_error
               BEGIN SELECT RAISE(ABORT, 'disk full'); END"]
        };
        for sql in statements {
            store.execute(sql, values![]).unwrap();
        }
    }

    /// A failure in the middle of a run's transaction (here: storing its
    /// entry errors, after the datasets) leaves the copy as it was.
    #[test]
    fn a_failure_mid_run_changes_nothing() {
        on_every_database(a_failure_mid_run_changes_nothing_on);
    }

    fn a_failure_mid_run_changes_nothing_on(store: &ConnectorStore) {
        let store = Arc::new(store.clone());
        let harvester = harvester(&store, vec![entry("a")]);
        let before = store.datasets().unwrap();
        make_entry_errors_fail(&store);
        let mut broken = entry("c");
        broken["hasPolicy"] = json!([]);
        let failing = DatasetHarvester::new(
            Arc::new(FixedSource::new(vec![entry("b"), broken])),
            Arc::new(DspCheck::new(
                "https://p.example/dsp/2025-1",
                &CredentialTypes::default(),
            )),
            Arc::clone(&store) as _,
        );
        let result = failing.run(crate::harvest::Trigger::Interval);
        assert!(matches!(result, RunResult::StoreFailed(_)), "{result:?}");
        assert_eq!(store.datasets().unwrap(), before);
        assert_eq!(store.jobs(None, 10).unwrap().len(), 1);
        assert_eq!(harvester.catalog().all().unwrap().len(), 1);
    }

    /// The catalog is read from the database on each request (TD-50,
    /// ADR 0029 D4): what a harvest wrote through one store is what the
    /// next request through another store on the same database sees, with
    /// nothing reloaded.
    fn a_harvest_is_seen_through_another_store(
        writer: &Arc<ConnectorStore>,
        reader: &Arc<ConnectorStore>,
    ) {
        let first = harvester(writer, vec![entry("a"), entry("b")]);
        // the reader's harvester only reads: its source is empty and its
        // start run fails, which keeps the stored datasets
        let other = DatasetHarvester::new(
            Arc::new(crate::harvest::FileSource::new("/nonexistent.json")),
            Arc::new(DspCheck::new(
                "https://p.example/dsp/2025-1",
                &CredentialTypes::default(),
            )),
            Arc::clone(reader) as Arc<dyn DatasetStore>,
        );
        let catalog = other.catalog();
        let ids = |catalog: &crate::harvest::StoredCatalog| -> Vec<String> {
            catalog.all().unwrap().into_iter().map(|d| d.id).collect()
        };
        assert_eq!(ids(&catalog), ["a", "b"]);

        // the writer's next run withdraws a and changes b's source
        let mut moved = entry("b");
        moved["portitor:source"] = json!("http://upstream.example/b2");
        let second = DatasetHarvester::new(
            Arc::new(FixedSource::new(vec![moved])),
            Arc::new(DspCheck::new(
                "https://p.example/dsp/2025-1",
                &CredentialTypes::default(),
            )),
            Arc::clone(writer) as Arc<dyn DatasetStore>,
        );
        assert!(matches!(second.run(Trigger::Interval), RunResult::Done(_)));
        drop(first);
        assert_eq!(ids(&catalog), ["b"]);
        assert_eq!(catalog.get("a").unwrap(), None, "withdrawn");
        assert_eq!(
            catalog.get_for_agreement("a").unwrap().unwrap().id,
            "a",
            "kept for agreements already made"
        );
        assert_eq!(
            crate::harvest::UpstreamSources::upstream_of(&catalog, "b").unwrap(),
            Some("http://upstream.example/b2".to_owned())
        );
    }

    #[test]
    fn a_harvest_through_one_store_is_seen_through_another_on_postgres() {
        let Some(url) = portitor_runtime::testing::databases::postgres_url() else {
            return;
        };
        let settings = portitor_runtime::testing::databases::fresh_postgres_settings(&url);
        let setting = portitor_runtime::database::DatabaseSetting::Postgres(Box::new(settings));
        let open = || {
            let database =
                portitor_runtime::database::Database::open(&setting, &super::super::MIGRATIONS)
                    .unwrap();
            Arc::new(ConnectorStore::new(Arc::new(database)))
        };
        // two pools on one database: two replicas of the connector
        a_harvest_is_seen_through_another_store(&open(), &open());
    }

    #[test]
    fn a_harvest_through_one_store_is_seen_through_another_on_every_database() {
        // two stores on one database (on SQLite, a file serves one
        // process: the two share its connection)
        on_every_database(|store| {
            let writer = Arc::new(store.clone());
            let reader = Arc::new(ConnectorStore::new(Arc::clone(store.database())));
            a_harvest_is_seen_through_another_store(&writer, &reader);
        });
    }

    /// The catalog lists ids in the order of their bytes on every
    /// database, as the copy in memory did: PostgreSQL's own order of
    /// text may follow a language's rules instead.
    #[test]
    fn the_catalog_is_in_the_order_of_the_ids_bytes() {
        on_every_database(|store| {
            let store = Arc::new(store.clone());
            let harvester = harvester(
                &store,
                vec![
                    entry("b"),
                    entry("B"),
                    entry("a-1"),
                    entry("a_1"),
                    entry("a"),
                ],
            );
            let ids: Vec<String> = harvester
                .catalog()
                .all()
                .unwrap()
                .into_iter()
                .map(|d| d.id)
                .collect();
            assert_eq!(ids, ["B", "a", "a-1", "a_1", "b"]);
        });
    }

    #[test]
    fn only_the_last_jobs_are_kept_with_their_errors() {
        on_every_database(only_the_last_jobs_are_kept_with_their_errors_on);
    }

    fn only_the_last_jobs_are_kept_with_their_errors_on(store: &ConnectorStore) {
        let store = Arc::new(store.clone());
        for n in 0..KEPT_JOBS + 3 {
            let commit = Commit {
                job: JobRecord {
                    kind: JobKind::Datasets,
                    started_at: n,
                    finished_at: n,
                    trigger: Trigger::Interval,
                    source: "test".to_owned(),
                    snapshot_sha256: None,
                    outcome: JobOutcome::Partial,
                    counts: Counts::default(),
                    summary: String::new(),
                },
                errors: vec![EntryError {
                    position: Some(n),
                    dataset_id: None,
                    kind: CheckKind::Shacl,
                    message: "m".to_owned(),
                    report: Some(json!({"conforms": false})),
                }],
                writes: Vec::new(),
                status: store.status().unwrap(),
            };
            assert_eq!(store.commit(&commit).unwrap(), Some(n + 1));
        }
        let jobs = store.jobs(None, 1_000).unwrap();
        assert_eq!(u64::try_from(jobs.len()).unwrap(), KEPT_JOBS);
        assert_eq!((jobs[0].id, jobs[99].id), (KEPT_JOBS + 3, 4));
        assert!(store.job(3).unwrap().is_none());
        let errors = store
            .query_first("SELECT COUNT(*) FROM harvest_error", values![], |row| {
                col::<i64>(row, 0)
            })
            .unwrap();
        assert_eq!(errors, Some(100));
        let (_, kept) = store.job(4).unwrap().unwrap();
        assert_eq!(kept[0].report, Some(json!({"conforms": false})));
        let page: Vec<u64> = store
            .jobs(Some(10), 3)
            .unwrap()
            .iter()
            .map(|j| j.id)
            .collect();
        assert_eq!(page, [9, 8, 7]);
    }

    #[test]
    fn a_run_is_written_only_on_the_status_it_read() {
        on_every_database(a_run_is_written_only_on_the_status_it_read_on);
    }

    /// Two runs read the same status; the first is written, the second
    /// is not (nothing of it: no job, no dataset, no status).
    fn a_run_is_written_only_on_the_status_it_read_on(store: &ConnectorStore) {
        let read = store.status().unwrap();
        let run = |summary: &str, rules: &str| Commit {
            job: JobRecord {
                kind: JobKind::Datasets,
                started_at: 1,
                finished_at: 2,
                trigger: Trigger::Interval,
                source: "test".to_owned(),
                snapshot_sha256: None,
                outcome: JobOutcome::Succeeded,
                counts: Counts::default(),
                summary: summary.to_owned(),
            },
            errors: Vec::new(),
            writes: Vec::new(),
            status: HarvestStatus {
                rules_version: Some(rules.to_owned()),
                ..read.clone()
            },
        };
        assert_eq!(store.commit(&run("first", "r1")).unwrap(), Some(1));
        assert_eq!(store.commit(&run("second", "r2")).unwrap(), None);
        let status = store.status().unwrap();
        assert_eq!(status.rules_version.as_deref(), Some("r1"));
        assert_eq!(status.version, read.version + 1);
        assert_eq!(store.jobs(None, 10).unwrap().len(), 1);
    }
}
