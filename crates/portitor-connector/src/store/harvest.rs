// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The harvest's read copy, jobs and entry errors, in the same database
//! (ADR 0020). A run's changes are written in one transaction.

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

const DATASET_COLUMNS: &str = "id, entry, content, upstream, content_sha256, state, \
     conforms_to, first_seen, last_changed, last_job";

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
        last_job: uint(col(row, 9)?)?,
    })
}

const JOB_COLUMNS: &str = "id, started_at, finished_at, run_trigger, source, snapshot_sha256, \
     outcome, seen, added, updated, unchanged, withdrawn, rejected, summary, kind";

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
    })
}

/// A run's rows, made before its transaction: the transaction only
/// writes them (it may run more than once, see
/// `ConnectorStore::transaction`). The job's id is known only inside the
/// transaction, so every statement takes it as `$1`, and each list of
/// values here starts at `$2`.
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
    Ok(values![
        int(job.started_at)?,
        int(job.finished_at)?,
        job.trigger.name(),
        &job.source,
        job.snapshot_sha256.as_deref(),
        job.outcome.name(),
        int(counts.seen)?,
        int(counts.added)?,
        int(counts.updated)?,
        int(counts.unchanged)?,
        int(counts.withdrawn)?,
        int(counts.rejected)?,
        &job.summary,
        job.kind.name(),
    ]
    .to_vec())
}

/// The values of a dataset's row, `$2` and on (`$1` is the job's id,
/// its `last_job`).
fn dataset_values(dataset: &StoredDataset) -> Result<Vec<Value>, StoreError> {
    Ok(values![
        &dataset.id,
        json(&dataset.entry)?,
        json(&dataset.content)?,
        dataset.upstream.as_deref(),
        &dataset.content_sha256,
        dataset.state.name(),
        dataset.conforms_to.as_deref(),
        int(dataset.first_seen)?,
        int(dataset.last_changed)?,
    ]
    .to_vec())
}

/// The values of an entry error's row, `$2` and on (`$1` is the job's
/// id).
fn error_values(seq: usize, error: &EntryError) -> Result<Vec<Value>, StoreError> {
    Ok(values![
        int(count(seq))?,
        error.position.map(int).transpose()?,
        error.dataset_id.as_deref(),
        error.kind.name(),
        &error.message,
        error.report.as_ref().map(json).transpose()?,
    ]
    .to_vec())
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
            status: values![
                status.rules_version.as_deref(),
                status.snapshot_sha256.as_deref()
            ]
            .to_vec(),
            kept_jobs: int(KEPT_JOBS)?,
        })
    }
}

/// `values` with the job's id in front, as `$1`.
fn with_job_id(job_id: i64, values: &[Value]) -> Vec<Value> {
    let mut all = Vec::with_capacity(values.len() + 1);
    all.push(Value::Int(job_id));
    all.extend_from_slice(values);
    all
}

/// Writes one run in the transaction `tx` and returns the job's id.
///
/// The database gives the id (TD-56): the row id of `SQLite`, the
/// identity column of PostgreSQL. Two processes that write a run at once
/// get two ids, never the same one, and no process keeps a count of its
/// own.
fn write_run(tx: &mut dyn Executor, rows: &RunRows) -> Result<i64, DbError> {
    let insert_job = format!(
        "INSERT INTO harvest_job ({})
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING id",
        // every column but the id, which the database gives
        JOB_COLUMNS.trim_start_matches("id, ")
    );
    let id = match tx.query_one(&insert_job, &rows.job)? {
        Some(row) => row.get::<i64>(0)?,
        None => return Err(DbError::new("the new harvest job has no id")),
    };
    for dataset in &rows.datasets {
        write_dataset(tx, &with_job_id(id, dataset))?;
    }
    for error in &rows.errors {
        tx.execute(
            "INSERT INTO harvest_error
                 (job_id, seq, entry_position, dataset_id, kind, message, report)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &with_job_id(id, error),
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
    tx.execute(
        "UPDATE harvest_state SET rules_version = $1, snapshot_sha256 = $2 WHERE id = 1",
        &rows.status,
    )?;
    Ok(id)
}

/// Writes one dataset: an update of its row, or a new row. (Two plain
/// statements rather than an upsert; they run in the run's transaction.)
/// `values`: the job's id, then the dataset's columns.
fn write_dataset(tx: &mut dyn Executor, values: &[Value]) -> Result<(), DbError> {
    let updated = tx.execute(
        "UPDATE dataset
         SET entry = $3, content = $4, upstream = $5, content_sha256 = $6, state = $7,
             conforms_to = $8, first_seen = $9, last_changed = $10, last_job = $1
         WHERE id = $2",
        values,
    )?;
    if updated == 0 {
        let sql = format!(
            "INSERT INTO dataset ({DATASET_COLUMNS})
             VALUES ($2, $3, $4, $5, $6, $7, $8, $9, $10, $1)"
        );
        tx.execute(&sql, values)?;
    }
    Ok(())
}

impl DatasetStore for ConnectorStore {
    fn datasets(&self) -> Result<Vec<StoredDataset>, StoreError> {
        let sql = format!("SELECT {DATASET_COLUMNS} FROM dataset ORDER BY id");
        self.query(&sql, values![], dataset_from_row)
    }

    fn status(&self) -> Result<HarvestStatus, StoreError> {
        // the one row of harvest_state: both columns may be NULL
        let found = self.query_first(
            "SELECT rules_version, snapshot_sha256 FROM harvest_state WHERE id = 1",
            values![],
            status_from_row,
        )?;
        // no row (cannot happen after the migration): nothing known yet
        Ok(found.unwrap_or_default())
    }

    fn commit(&self, commit: &Commit) -> Result<u64, StoreError> {
        let rows = RunRows::of(commit)?;
        // on any error nothing is written
        let id = self.transaction(|tx| write_run(tx, &rows))?;
        uint(id)
    }

    fn jobs(&self, before: Option<u64>, limit: usize) -> Result<Vec<HarvestJob>, StoreError> {
        // no `before`: from the newest. The CAST gives PostgreSQL the type
        // of $1, which it cannot tell from `$1 IS NULL`
        let sql = format!(
            "SELECT {JOB_COLUMNS} FROM harvest_job
             WHERE CAST($1 AS BIGINT) IS NULL OR id < $1
             ORDER BY id DESC LIMIT $2"
        );
        let before = before.map(int).transpose()?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(&sql, values![before, limit], job_from_row)
    }

    fn job(&self, id: u64) -> Result<Option<JobWithErrors>, StoreError> {
        let id = int(id)?;
        let sql = format!("SELECT {JOB_COLUMNS} FROM harvest_job WHERE id = $1");
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
        assert_eq!(jobs.len(), 12);
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
        assert_eq!(restarted.copy().all().len(), 1);
        assert_eq!(
            store.jobs(None, 10).unwrap()[0].record.outcome,
            JobOutcome::Failed
        );
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
        assert_eq!(harvester.copy().all().len(), 1);
    }

    #[test]
    fn only_the_last_jobs_are_kept_with_their_errors() {
        on_every_database(only_the_last_jobs_are_kept_with_their_errors_on);
    }

    fn only_the_last_jobs_are_kept_with_their_errors_on(store: &ConnectorStore) {
        let store = Arc::new(store.clone());
        let status = HarvestStatus::default();
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
                status: status.clone(),
            };
            assert_eq!(store.commit(&commit).unwrap(), n + 1);
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
}
