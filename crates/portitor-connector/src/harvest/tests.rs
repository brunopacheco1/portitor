// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The harvest run (ADR 0020, "A harvest run"), one test per rule, with a
//! fake source, fake checks and the in-memory store.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use portitor_dsp::catalog::CatalogDatasets;
use portitor_dsp::policy::CredentialTypes;
use serde_json::{Value, json};

use super::UpstreamSources;
use super::*;

const SERVICE: &str = "https://p.example/dsp/2025-1";

/// A source the test changes between runs; `Err` makes it fail.
struct Source(Mutex<Result<Vec<Value>, String>>);

impl Source {
    fn with(entries: Vec<Value>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Ok(entries))))
    }
    fn set(&self, entries: Vec<Value>) {
        *self.0.lock().unwrap() = Ok(entries);
    }
    fn fail(&self) {
        *self.0.lock().unwrap() = Err("unreachable".to_owned());
    }
}

impl DatasetSource for Source {
    fn name(&self) -> String {
        "test".to_owned()
    }
    fn snapshot(&self) -> Result<Snapshot, SourceError> {
        let entries = self.0.lock().unwrap().clone().map_err(SourceError)?;
        Ok(Snapshot {
            sha256: serde_json::to_string(&entries).unwrap(),
            entries,
        })
    }
}

/// A dataset entry that passes the DSP rules.
fn entry(id: &str) -> Value {
    json!({
        "@id": id, "@type": "Dataset",
        "hasPolicy": [{"@id": format!("{id}:offer"), "@type": "Offer", "permission": [{"action": "use"}]}],
        "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}],
        "portitor:source": format!("https://upstream.example/{id}")
    })
}

/// The same entry, broken: no offer.
fn broken(id: &str) -> Value {
    let mut broken = entry(id);
    broken["hasPolicy"] = json!([]);
    broken
}

fn dsp() -> Arc<dyn DatasetCheck> {
    Arc::new(DspCheck::new(SERVICE, &CredentialTypes::default()))
}

/// A started harvester on `source` and `store`, with `check`.
fn started(
    source: &Arc<Source>,
    store: &Arc<dyn DatasetStore>,
    check: Arc<dyn DatasetCheck>,
) -> DatasetHarvester {
    let harvester = DatasetHarvester::new(Arc::clone(source) as _, check, Arc::clone(store));
    harvester.start().unwrap();
    harvester
}

fn memory() -> Arc<dyn DatasetStore> {
    Arc::new(InMemoryDatasetStore::default())
}

/// The leases of process `holder` in `store`'s database, on Tokio's clock
/// (paused in the tests that schedule runs): a second of that clock is a
/// second for the leases.
fn leases_in(store: &crate::store::ConnectorStore, holder: &str) -> Leases {
    let begun = tokio::time::Instant::now();
    let clock = Arc::new(move || 1_800_000_000 + begun.elapsed().as_secs());
    Leases::with_clock(Arc::clone(store.database()) as _, holder, clock)
}

/// [`leases_in`] a database of their own.
fn leases(holder: &str) -> Leases {
    leases_in(&crate::store::ConnectorStore::in_memory().unwrap(), holder)
}

/// The newest job and its errors.
fn last_job(store: &Arc<dyn DatasetStore>) -> (HarvestJob, Vec<EntryError>) {
    let newest = store.jobs(None, 1).unwrap().remove(0);
    store.job(newest.id).unwrap().unwrap()
}

fn done(result: RunResult) -> HarvestJob {
    match result {
        RunResult::Done(job) => job,
        other => panic!("{other:?}"),
    }
}

fn ids(copy: &StoredCatalog) -> Vec<String> {
    copy.all().unwrap().into_iter().map(|d| d.id).collect()
}

#[test]
fn the_start_run_fills_the_catalog() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    let copy = harvester.catalog();
    assert_eq!(ids(&copy), ["a", "b"]);
    let served = copy.get("a").unwrap().unwrap();
    assert_eq!(served.distribution[0].access_service, SERVICE);
    // the upstream URL is kept for the data plane, never in the dataset
    assert_eq!(
        copy.upstream_of("a").unwrap().as_deref(),
        Some("https://upstream.example/a")
    );
    assert!(!serde_json::to_string(&served).unwrap().contains("upstream"));
    let (job, errors) = last_job(&store);
    assert_eq!(job.record.trigger, Trigger::Start);
    assert_eq!(job.record.outcome, JobOutcome::Succeeded);
    assert_eq!(job.record.counts.seen, 2);
    assert_eq!(job.record.counts.added, 2);
    assert_eq!(
        job.record.summary,
        "2 seen, 2 added, 0 updated, 0 unchanged, 0 withdrawn, 0 rejected"
    );
    assert!(errors.is_empty());
}

#[test]
fn an_unchanged_snapshot_touches_nothing() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let harvester = started(&source, &store, dsp());
    let before = store.datasets().unwrap();
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Unchanged);
    assert_eq!(job.record.trigger, Trigger::Interval);
    assert_eq!(job.record.counts.seen, 1);
    assert_eq!(job.record.counts.added + job.record.counts.unchanged, 0);
    assert_eq!(store.datasets().unwrap(), before);
}

#[test]
fn the_same_content_in_a_new_snapshot_is_unchanged() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    let mut changed = entry("b");
    changed["dct:title"] = json!("B");
    source.set(vec![entry("a"), changed]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Succeeded);
    assert_eq!(
        (job.record.counts.unchanged, job.record.counts.updated),
        (1, 1)
    );
    let stored = store.datasets().unwrap();
    assert_eq!(stored[0].last_job, 1, "a was not written again");
    assert_eq!(stored[1].last_job, 2);
    let title = harvester
        .catalog()
        .get("b")
        .unwrap()
        .unwrap()
        .metadata
        .get("dct:title")
        .cloned();
    assert_eq!(title, Some(json!("B")));
}

#[test]
fn a_rejected_entry_keeps_the_previous_good_version() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    source.set(vec![entry("a"), broken("b"), broken("c")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Partial);
    assert_eq!(job.record.counts.rejected, 2);
    assert_eq!(job.record.counts.withdrawn, 0, "b is still in the source");
    let copy = harvester.catalog();
    assert_eq!(ids(&copy), ["a", "b"]);
    assert_eq!(copy.get("b").unwrap().unwrap().has_policy.len(), 1);
    let (_, errors) = last_job(&store);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].position, Some(1));
    assert_eq!(errors[0].dataset_id.as_deref(), Some("b"));
    assert_eq!(errors[0].kind, CheckKind::Dsp);
    assert_eq!(errors[0].message, "dataset \"b\" has no offer (hasPolicy)");
    assert_eq!(errors[1].dataset_id.as_deref(), Some("c"));
}

#[test]
fn a_bad_upstream_or_a_non_object_rejects_only_that_entry() {
    let store = memory();
    let mut file_url = entry("b");
    file_url["portitor:source"] = json!("file:///etc/passwd");
    let source = Source::with(vec![entry("a"), file_url, json!("text"), json!({"@id": 7})]);
    let harvester = started(&source, &store, dsp());
    assert_eq!(ids(&harvester.catalog()), ["a"]);
    let (job, errors) = last_job(&store);
    assert_eq!(job.record.counts.rejected, 3);
    let kinds: Vec<CheckKind> = errors.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [CheckKind::Dsp, CheckKind::Json, CheckKind::Dsp]);
    assert!(errors[0].message.contains("portitor:source"), "{errors:?}");
    assert_eq!(errors[2].dataset_id, None);
    assert_eq!(errors[2].position, Some(3));
}

/// A source in the connector's object storage is taken as it is (ADR
/// 0027, section 3); an `s3:` URI without a key is refused.
#[test]
fn an_object_storage_source_is_accepted() {
    let store = memory();
    let mut in_storage = entry("a");
    in_storage["portitor:source"] = json!("s3://provider-data/health/a.csv");
    let mut no_key = entry("b");
    no_key["portitor:source"] = json!("s3://provider-data");
    let source = Source::with(vec![in_storage, no_key]);
    let harvester = started(&source, &store, dsp());
    assert_eq!(ids(&harvester.catalog()), ["a"]);
    assert_eq!(
        harvester.catalog().upstream_of("a").unwrap().as_deref(),
        Some("s3://provider-data/health/a.csv")
    );
    let (_, errors) = last_job(&store);
    assert!(errors[0].message.contains("s3://"), "{errors:?}");
}

#[test]
fn a_dataset_missing_from_the_snapshot_is_withdrawn() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    source.set(vec![entry("a")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.counts.withdrawn, 1);
    let copy = harvester.catalog();
    // out of the catalog and of contract requests ...
    assert_eq!(ids(&copy), ["a"]);
    assert_eq!(copy.get("b").unwrap(), None);
    // ... still there for transfers under agreements already made
    assert_eq!(copy.get_for_agreement("b").unwrap().unwrap().id, "b");
    assert_eq!(
        copy.upstream_of("b").unwrap().as_deref(),
        Some("https://upstream.example/b")
    );
    let stored = store.datasets().unwrap();
    assert_eq!(stored[1].state, DatasetState::Withdrawn);
    // a second run does not withdraw it again
    source.set(vec![entry("a"), entry("c")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(
        (job.record.counts.added, job.record.counts.withdrawn),
        (1, 0)
    );
}

#[test]
fn a_withdrawn_dataset_that_comes_back_is_active_again() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    source.set(vec![entry("a")]);
    done(harvester.run(Trigger::Interval));
    let first_seen = store.datasets().unwrap()[1].first_seen;
    source.set(vec![entry("b"), entry("a")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(
        (job.record.counts.added, job.record.counts.unchanged),
        (1, 1)
    );
    assert_eq!(ids(&harvester.catalog()), ["a", "b"]);
    let b = &store.datasets().unwrap()[1];
    assert_eq!(b.state, DatasetState::Active);
    assert_eq!(b.first_seen, first_seen);
}

#[test]
fn a_failed_source_leaves_the_copy() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let harvester = started(&source, &store, dsp());
    let before = store.datasets().unwrap();
    source.fail();
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Failed);
    assert_eq!(job.record.snapshot_sha256, None);
    assert!(job.record.summary.contains("unreachable"), "{job:?}");
    assert_eq!(store.datasets().unwrap(), before);
    assert_eq!(ids(&harvester.catalog()), ["a"]);
    // and the next good run is not taken for "unchanged"
    source.set(vec![entry("a")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Unchanged);
}

#[test]
fn a_first_start_with_a_failing_source_has_an_empty_catalog() {
    let store = memory();
    let source = Source::with(Vec::new());
    source.fail();
    let harvester = started(&source, &store, dsp());
    assert!(harvester.catalog().all().unwrap().is_empty());
    let (job, _) = last_job(&store);
    assert_eq!(
        (job.record.trigger, job.record.outcome),
        (Trigger::Start, JobOutcome::Failed)
    );
}

#[test]
fn of_two_entries_with_one_id_the_first_wins() {
    let store = memory();
    let mut second = entry("a");
    second["dct:title"] = json!("second");
    let source = Source::with(vec![entry("a"), second, broken("b"), entry("b")]);
    let harvester = started(&source, &store, dsp());
    let copy = harvester.catalog();
    assert_eq!(
        copy.get("a").unwrap().unwrap().metadata.get("dct:title"),
        None
    );
    // the first b fails, so there is no b; the second is still a duplicate
    assert_eq!(ids(&copy), ["a"]);
    let (job, errors) = last_job(&store);
    assert_eq!(job.record.counts.rejected, 3);
    assert_eq!(errors[0].position, Some(1));
    assert!(errors[0].message.contains("defined twice"), "{errors:?}");
    assert_eq!(errors[2].position, Some(3));
}

/// A rules version that refuses the datasets named in `refuse`; the
/// version is chosen by the test.
struct Rules {
    version: &'static str,
    refuse: &'static [&'static str],
}

impl DatasetCheck for Rules {
    fn rules_version(&self) -> String {
        self.version.to_owned()
    }
    fn check(&self, entry: &Value) -> Result<CheckedDataset, EntryFailure> {
        let checked = dsp().check(entry)?;
        if self.refuse.contains(&checked.dataset.id.as_str()) {
            return Err(EntryFailure {
                kind: CheckKind::Shacl,
                message: format!("{} breaks the shapes", checked.dataset.id),
                report: Some(json!({"sh:conforms": false})),
            });
        }
        Ok(CheckedDataset {
            conforms_to: Some(format!("https://shapes.example/{}", self.version)),
            ..checked
        })
    }
}

#[test]
fn new_rules_check_the_stored_datasets_again_at_start() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let v1 = Arc::new(Rules {
        version: "v1",
        refuse: &[],
    });
    let first = started(&source, &store, v1);
    let a = first.catalog().get("a").unwrap().unwrap();
    assert_eq!(
        a.metadata.get("dct:conformsTo"),
        Some(&json!({"@id": "https://shapes.example/v1", "@type": "dct:Standard"}))
    );

    // a restart with new rules that refuse b, while the source is down:
    // b is hidden, with the reason stored, in a `bundle` job
    source.fail();
    let v2 = Arc::new(Rules {
        version: "v2",
        refuse: &["b"],
    });
    let second = started(&source, &store, v2);
    let jobs = store.jobs(None, 10).unwrap();
    assert_eq!(jobs[0].record.outcome, JobOutcome::Failed);
    let bundle = &jobs[1];
    assert_eq!(bundle.record.trigger, Trigger::Bundle);
    assert_eq!(bundle.record.outcome, JobOutcome::Partial);
    assert_eq!(
        (bundle.record.counts.seen, bundle.record.counts.updated),
        (2, 1)
    );
    let (_, errors) = store.job(bundle.id).unwrap().unwrap();
    assert_eq!(errors[0].kind, CheckKind::Shacl);
    assert_eq!(errors[0].position, None);
    assert_eq!(errors[0].report, Some(json!({"sh:conforms": false})));
    let copy = second.catalog();
    assert_eq!(ids(&copy), ["a"]);
    assert_eq!(copy.get_for_agreement("b").unwrap().unwrap().id, "b");
    assert_eq!(store.datasets().unwrap()[1].state, DatasetState::Hidden);
    assert_eq!(
        copy.get("a")
            .unwrap()
            .unwrap()
            .metadata
            .get("dct:conformsTo"),
        Some(&json!({"@id": "https://shapes.example/v2", "@type": "dct:Standard"}))
    );

    // the same rules again: no `bundle` job
    let jobs_before = store.jobs(None, 100).unwrap().len();
    let v2 = Arc::new(Rules {
        version: "v2",
        refuse: &["b"],
    });
    started(&source, &store, v2);
    assert_eq!(store.jobs(None, 100).unwrap().len(), jobs_before + 1);

    // rules under which b passes: it is active again, and the start run
    // checks every entry although the snapshot is unchanged
    source.set(vec![entry("a"), entry("b")]);
    let v3 = Arc::new(Rules {
        version: "v3",
        refuse: &[],
    });
    let third = started(&source, &store, v3);
    assert_eq!(ids(&third.catalog()), ["a", "b"]);
    let jobs = store.jobs(None, 2).unwrap();
    assert_eq!(jobs[1].record.trigger, Trigger::Bundle);
    assert_eq!(jobs[1].record.outcome, JobOutcome::Succeeded);
    assert_eq!(jobs[0].record.outcome, JobOutcome::Succeeded);
}

/// Rules the test swaps while the harvester runs, as the bundle copy is
/// swapped when a new bundle version is taken.
struct Swappable(Mutex<Arc<dyn DatasetCheck>>);

impl DatasetCheck for Swappable {
    fn rules_version(&self) -> String {
        self.0.lock().unwrap().rules_version()
    }
    fn check(&self, entry: &Value) -> Result<CheckedDataset, EntryFailure> {
        let rules = Arc::clone(&self.0.lock().unwrap());
        rules.check(entry)
    }
}

/// A run of the bundle harvester, as it reaches the dataset harvester.
fn bundle_run(summary: &str) -> JobRecord {
    JobRecord {
        kind: JobKind::Bundle,
        started_at: 1,
        finished_at: 2,
        trigger: Trigger::Interval,
        source: "https://authority.example/bundles/".to_owned(),
        snapshot_sha256: Some("abc".to_owned()),
        outcome: JobOutcome::Succeeded,
        counts: Counts::default(),
        summary: summary.to_owned(),
    }
}

#[test]
fn new_rules_while_running_check_the_datasets_again() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    let rules = Arc::new(Swappable(Mutex::new(Arc::new(Rules {
        version: "v1",
        refuse: &[],
    }))));
    let harvester = started(&source, &store, Arc::clone(&rules) as _);
    assert_eq!(ids(&harvester.catalog()), ["a", "b"]);
    let status_before = store.status().unwrap();

    // the bundle run is logged next to the harvest runs; it changes no
    // dataset and keeps the harvest status
    let id = harvester
        .record_bundle_run(&bundle_run("version v2 in use (was v1)"))
        .unwrap();
    let (logged, errors) = store.job(id).unwrap().unwrap();
    assert_eq!(logged.record.kind, JobKind::Bundle);
    assert!(errors.is_empty());
    let after = HarvestStatus {
        // one more run written
        version: status_before.version + 1,
        ..status_before
    };
    assert_eq!(store.status().unwrap(), after);

    // new rules that refuse b: the stored datasets are checked again,
    // then every entry of the unchanged source, both as `bundle` runs
    *rules.0.lock().unwrap() = Arc::new(Rules {
        version: "v2",
        refuse: &["b"],
    });
    let run = done(harvester.rules_changed());
    assert_eq!(run.record.kind, JobKind::Datasets);
    assert_eq!(run.record.trigger, Trigger::Bundle);
    assert_eq!(run.record.counts.seen, 2, "every entry checked again");
    assert_eq!(ids(&harvester.catalog()), ["a"]);
    let jobs = store.jobs(None, 10).unwrap();
    let triggers: Vec<_> = jobs
        .iter()
        .map(|j| (j.record.kind, j.record.trigger))
        .collect();
    assert_eq!(
        triggers[..4],
        [
            (JobKind::Datasets, Trigger::Bundle),
            (JobKind::Datasets, Trigger::Bundle),
            (JobKind::Bundle, Trigger::Interval),
            (JobKind::Datasets, Trigger::Start),
        ]
    );
    assert_eq!(store.status().unwrap().rules_version.as_deref(), Some("v2"));
}

#[test]
fn rules_that_change_with_an_unchanged_source_check_every_entry() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b")]);
    started(
        &source,
        &store,
        Arc::new(Rules {
            version: "v1",
            refuse: &["b"],
        }),
    );
    // b was rejected, so only a is stored; new rules let b in although
    // the snapshot is the same
    let harvester = started(
        &source,
        &store,
        Arc::new(Rules {
            version: "v2",
            refuse: &[],
        }),
    );
    assert_eq!(ids(&harvester.catalog()), ["a", "b"]);
    assert_eq!(last_job(&store).0.record.counts.added, 1);
}

/// A copy stored by another Portitor version: its checks may differ, so
/// the stored datasets are checked again (a `bundle` job), and the start
/// run checks every entry although the snapshot is unchanged.
#[test]
fn a_new_portitor_version_checks_the_stored_datasets_again() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    started(&source, &store, dsp());
    let stored_rules = store.status().unwrap().rules_version.unwrap();
    assert!(
        stored_rules.contains(&format!("portitor {CHECKS_VERSION}")),
        "{stored_rules}"
    );
    // as if an older version had stored the copy: same rules otherwise
    let older = stored_rules.replace(CHECKS_VERSION, "0.0.1");
    store
        .commit(&Commit {
            job: JobRecord {
                kind: JobKind::Datasets,
                started_at: 0,
                finished_at: 0,
                trigger: Trigger::Start,
                source: "older".to_owned(),
                snapshot_sha256: None,
                outcome: JobOutcome::Unchanged,
                counts: Counts::default(),
                summary: String::new(),
            },
            errors: Vec::new(),
            writes: Vec::new(),
            status: HarvestStatus {
                rules_version: Some(older),
                ..store.status().unwrap()
            },
        })
        .unwrap();
    started(&source, &store, dsp());
    let jobs = store.jobs(None, 2).unwrap();
    assert_eq!(jobs[1].record.trigger, Trigger::Bundle);
    assert_eq!(jobs[1].record.counts.seen, 1);
    assert_eq!(jobs[0].record.trigger, Trigger::Start);
    assert_eq!(jobs[0].record.outcome, JobOutcome::Succeeded);
    assert_eq!(store.status().unwrap().rules_version, Some(stored_rules));
}

#[test]
fn a_job_stores_at_most_the_first_errors() {
    let store = memory();
    let too_many = run::MAX_STORED_ERRORS + 5;
    let entries = (0..too_many).map(|n| broken(&format!("d{n}"))).collect();
    let source = Source::with(entries);
    started(&source, &store, dsp());
    let (job, errors) = last_job(&store);
    assert_eq!(job.record.counts.rejected, u64::try_from(too_many).unwrap());
    assert_eq!(errors.len(), run::MAX_STORED_ERRORS);
    assert_eq!(errors[0].dataset_id.as_deref(), Some("d0"));
    assert!(
        job.record
            .summary
            .ends_with("; only the first 1000 errors are stored, the others are in the log"),
        "{}",
        job.record.summary
    );
}

/// Refuses every entry with a report of `results` results.
struct LongReport {
    results: usize,
}

impl DatasetCheck for LongReport {
    fn rules_version(&self) -> String {
        "long".to_owned()
    }
    fn check(&self, _entry: &Value) -> Result<CheckedDataset, EntryFailure> {
        let results: Vec<Value> = (0..self.results).map(|n| json!({"n": n})).collect();
        Err(EntryFailure {
            kind: CheckKind::Shacl,
            message: "violations".to_owned(),
            report: Some(json!({"conforms": false, "results": results})),
        })
    }
}

#[test]
fn a_stored_report_keeps_only_the_first_results() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let results = run::MAX_REPORT_RESULTS + 7;
    started(&source, &store, Arc::new(LongReport { results }));
    let (_, errors) = last_job(&store);
    let report = errors[0].report.as_ref().unwrap();
    assert_eq!(
        report["results"].as_array().unwrap().len(),
        run::MAX_REPORT_RESULTS
    );
    assert_eq!(report["results"][0], json!({"n": 0}));
    assert_eq!(report["resultsNotStored"], json!(7));
    assert_eq!(report["conforms"], json!(false));

    // a short report is kept as it is
    let store = memory();
    started(&source, &store, Arc::new(LongReport { results: 2 }));
    let (_, errors) = last_job(&store);
    let report = errors[0].report.as_ref().unwrap();
    assert_eq!(report["results"].as_array().unwrap().len(), 2);
    assert!(report.get("resultsNotStored").is_none());
}

#[test]
fn every_stored_name_reads_back() {
    for state in DatasetState::ALL {
        assert_eq!(DatasetState::from_name(state.name()), Some(state));
    }
    for trigger in Trigger::ALL {
        assert_eq!(Trigger::from_name(trigger.name()), Some(trigger));
    }
    for outcome in JobOutcome::ALL {
        assert_eq!(JobOutcome::from_name(outcome.name()), Some(outcome));
    }
    for kind in JobKind::ALL {
        assert_eq!(JobKind::from_name(kind.name()), Some(kind));
    }
    assert_eq!(JobKind::from_name("Bundle"), None);
    assert_eq!(DatasetState::from_name("Active"), None);
    assert_eq!(JobOutcome::from_name(""), None);
}

#[test]
fn only_the_last_jobs_are_kept() {
    let store = memory();
    let source = Source::with(vec![broken("x")]);
    let harvester = started(&source, &store, dsp());
    for n in 0..KEPT_JOBS + 4 {
        source.set(vec![entry(&format!("d{n}")), broken("x")]);
        done(harvester.run(Trigger::Interval));
    }
    let jobs = store.jobs(None, 1_000).unwrap();
    assert_eq!(u64::try_from(jobs.len()).unwrap(), KEPT_JOBS);
    assert_eq!(jobs[0].id, KEPT_JOBS + 5);
    assert_eq!(jobs[jobs.len() - 1].id, 6);
    assert!(store.job(5).unwrap().is_none());
    // paging: older than job 8, newest first
    let page: Vec<u64> = store
        .jobs(Some(8), 3)
        .unwrap()
        .iter()
        .map(|j| j.id)
        .collect();
    assert_eq!(page, [7, 6]);
}

#[test]
fn one_run_at_a_time() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let harvester = started(&source, &store, dsp());
    let running = harvester.busy.lock().unwrap();
    assert_eq!(harvester.run(Trigger::Interval), RunResult::Busy);
    drop(running);
    assert!(matches!(
        harvester.run(Trigger::Interval),
        RunResult::Done(_)
    ));
}

/// On a paused clock (`start_paused`): time moves only when every task
/// waits, and not while a `spawn_blocking` run is going on, so each
/// `sleep` below lets exactly the runs that fall due in it happen.
#[tokio::test(start_paused = true)]
async fn the_schedule_runs_again_and_zero_means_only_at_start() {
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let harvester = Arc::new(started(&source, &store, dsp()));
    assert!(harvester.schedule(Duration::ZERO, &leases("p-1")).is_none());
    let job_count = || store.jobs(None, 10).unwrap().len();
    assert_eq!(job_count(), 1, "the start run");

    let schedule = harvester
        .schedule(Duration::from_secs(60), &leases("p-1"))
        .unwrap();
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(job_count(), 1, "nothing before the first interval");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(job_count(), 2, "one run at 60 s");
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(job_count(), 3, "one more at 120 s");
    schedule.abort();

    let jobs = store.jobs(None, 10).unwrap();
    assert_eq!(jobs[0].record.trigger, Trigger::Interval);
    assert_eq!(jobs[0].record.outcome, JobOutcome::Unchanged);
}

/// Two processes ("replicas") on one database, each with its schedule:
/// one harvest per interval between them, not one each (TD-56). The
/// replica that started second ticks 10 seconds later and finds the run
/// of the interval done.
#[tokio::test(start_paused = true)]
async fn two_replicas_run_one_scheduled_harvest_per_interval() {
    let leases_store = crate::store::ConnectorStore::in_memory().unwrap();
    let store = memory();
    let source = Source::with(vec![entry("a")]);
    let one = Arc::new(started(&source, &store, dsp()));
    let two = Arc::new(started(&source, &store, dsp()));
    let job_count = || store.jobs(None, 10).unwrap().len();
    assert_eq!(job_count(), 2, "the start runs");
    let every = Duration::from_secs(60);
    let first = one
        .schedule(every, &leases_in(&leases_store, "p-1"))
        .unwrap();
    tokio::time::sleep(Duration::from_secs(10)).await;
    let second = two
        .schedule(every, &leases_in(&leases_store, "p-2"))
        .unwrap();
    tokio::time::sleep(Duration::from_secs(51)).await;
    assert_eq!(job_count(), 3, "p-1 ran at 60 s");
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(job_count(), 3, "p-2 skipped its turn at 70 s");
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(job_count(), 4, "one run between 120 s and 130 s");
    // p-1 stops: p-2 runs from the next interval on
    first.abort();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(job_count(), 5, "p-2 ran at 190 s");
    second.abort();
}

/// A source with a bug: its next snapshot panics, once, when `.1` is
/// set. Otherwise it is the source in `.0`.
struct PanicsOnce(Arc<Source>, std::sync::atomic::AtomicBool);

impl DatasetSource for PanicsOnce {
    fn name(&self) -> String {
        self.0.name()
    }
    fn snapshot(&self) -> Result<Snapshot, SourceError> {
        let panic_now = self.1.swap(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!panic_now, "a bug in the source");
        self.0.snapshot()
    }
}

/// A run that panics (TD-41) must not block the runs after it: the next
/// run, by the schedule or from the management API, is done, not
/// skipped as if the panicked run were still going on.
#[test]
fn a_run_after_one_that_panicked_is_done() {
    let store = memory();
    let entries = Source::with(vec![entry("a")]);
    let source = Arc::new(PanicsOnce(
        Arc::clone(&entries),
        std::sync::atomic::AtomicBool::new(false),
    ));
    let harvester = Arc::new(DatasetHarvester::new(source.clone(), dsp(), store.clone()));
    harvester.start().unwrap();

    source.1.store(true, std::sync::atomic::Ordering::SeqCst);
    let runner = Arc::clone(&harvester);
    // on another thread, as the schedule runs it
    let panicked = std::thread::spawn(move || runner.run(Trigger::Interval)).join();
    assert!(panicked.is_err());

    entries.set(vec![entry("a"), entry("b")]);
    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.counts.added, 1);
    assert_eq!(ids(&harvester.catalog()), ["a", "b"]);
}

/// A scheduled run that panics (TD-41) ends only that run: the catalog
/// keeps what it served, and the run of the next interval works again
/// and takes the source's changes.
#[tokio::test(start_paused = true)]
async fn the_schedule_goes_on_after_a_run_that_panicked() {
    let store = memory();
    let entries = Source::with(vec![entry("a")]);
    let source = Arc::new(PanicsOnce(
        Arc::clone(&entries),
        std::sync::atomic::AtomicBool::new(false),
    ));
    let harvester = Arc::new(DatasetHarvester::new(source.clone(), dsp(), store.clone()));
    harvester.start().unwrap();
    let copy = harvester.catalog();
    let job_count = || store.jobs(None, 10).unwrap().len();
    assert_eq!(job_count(), 1, "the start run");

    let schedule = harvester
        .schedule(Duration::from_secs(60), &leases("p-1"))
        .unwrap();
    entries.set(vec![entry("a"), entry("b")]);
    source.1.store(true, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(
        job_count(),
        1,
        "the run at 60 s panicked and stored nothing"
    );
    assert_eq!(ids(&copy), ["a"], "the catalog is as before");

    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(job_count(), 2, "the run at 120 s worked");
    assert_eq!(ids(&copy), ["a", "b"]);
    schedule.abort();
}

#[test]
fn both_checks_add_conforms_to_without_losing_one_already_there() {
    let store = memory();
    let mut with_one = entry("a");
    with_one["dct:conformsTo"] = json!({"@id": "https://other.example/profile"});
    let source = Source::with(vec![with_one, entry("b")]);
    let check = Arc::new(BothChecks::new(
        dsp(),
        Arc::new(Rules {
            version: "v1",
            refuse: &[],
        }),
    ));
    let harvester = started(&source, &store, check);
    let copy = harvester.catalog();
    assert_eq!(
        copy.get("a")
            .unwrap()
            .unwrap()
            .metadata
            .get("dct:conformsTo"),
        Some(
            &json!([{"@id": "https://other.example/profile"}, {"@id": "https://shapes.example/v1", "@type": "dct:Standard"}])
        )
    );
    assert_eq!(
        store.datasets().unwrap()[1].conforms_to.as_deref(),
        Some("https://shapes.example/v1")
    );
}

/// Policies are read strictly (ADR 0009, amendment of 2026-11-18): a
/// dataset whose offer has a term Portitor does not read is rejected with
/// an `odrl` error that names the dataset, the offer and the field. As
/// for every harvest check (ADR 0020), only that dataset stops: the
/// others are served, and its previous good version stays.
#[test]
fn an_unknown_policy_term_stops_only_that_dataset() {
    let store = memory();
    let source = Source::with(vec![entry("a"), entry("b"), entry("c"), entry("d")]);
    let harvester = started(&source, &store, dsp());
    let copy = harvester.catalog();
    assert_eq!(ids(&copy), ["a", "b", "c", "d"]);

    // b: a duty inside a permission; c: a misspelled key; d: `unit` on a
    // constraint; e is new and has an agreement's field on its offer
    let mut duty = entry("b");
    duty["hasPolicy"][0]["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    let mut typo = entry("c");
    typo["hasPolicy"][0]["prohibtion"] = json!([{"action": "distribute"}]);
    let mut unit = entry("d");
    unit["hasPolicy"][0]["permission"][0]["constraint"] = json!([{"leftOperand": "dateTime",
        "operator": "lteq", "rightOperand": "2030-12-31", "unit": "http://example.com/unit"}]);
    let mut new_one = entry("e");
    new_one["hasPolicy"][0]["assigner"] = json!("did:web:p.example");
    let mut changed = entry("a");
    changed["dct:title"] = json!("A, changed");
    source.set(vec![changed, duty, typo, unit, new_one]);

    let job = done(harvester.run(Trigger::Interval));
    assert_eq!(job.record.outcome, JobOutcome::Partial);
    assert_eq!(job.record.counts.rejected, 4);
    assert_eq!(job.record.counts.updated, 1);

    // the good dataset was updated; the bad ones keep their last good
    // version (without the term), and the new bad one is not served
    assert_eq!(ids(&copy), ["a", "b", "c", "d"]);
    assert_eq!(
        copy.get("a").unwrap().unwrap().metadata.get("dct:title"),
        Some(&json!("A, changed"))
    );
    let kept = serde_json::to_value(copy.get("b").unwrap().unwrap()).unwrap();
    assert_eq!(kept["hasPolicy"], entry("b")["hasPolicy"]);

    let (_, errors) = last_job(&store);
    let expected = [
        (
            "b",
            "offer \"b:offer\" is not understood: unknown field `duty`",
        ),
        (
            "c",
            "offer \"c:offer\" is not understood: unknown field `prohibtion`",
        ),
        (
            "d",
            "offer \"d:offer\" is not understood: unknown field `unit`",
        ),
        (
            "e",
            "dataset \"e\" has an offer with an assigner or an assignee",
        ),
    ];
    assert_eq!(errors.len(), expected.len());
    for (error, (dataset, message)) in errors.iter().zip(expected) {
        assert_eq!(error.dataset_id.as_deref(), Some(dataset));
        assert!(error.message.starts_with(message), "{}", error.message);
    }
    // the unknown fields are ODRL failures; the catalog rule is a DSP one
    let kinds: Vec<CheckKind> = errors.iter().map(|error| error.kind).collect();
    assert_eq!(
        kinds,
        [
            CheckKind::Odrl,
            CheckKind::Odrl,
            CheckKind::Odrl,
            CheckKind::Dsp
        ]
    );
}

/// The same at the first start, when there is no earlier good version:
/// a dataset whose offer has a term Portitor does not read is not
/// served, and the others are.
#[test]
fn an_unknown_policy_term_at_the_first_start_leaves_only_that_dataset_out() {
    let mut duty = entry("b");
    duty["hasPolicy"][0]["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    let mut party = entry("c");
    party["hasPolicy"][0]["assignee"] = json!("did:web:c.example");

    let store = memory();
    let source = Source::with(vec![entry("a"), duty, party, entry("d")]);
    let harvester = started(&source, &store, dsp());
    assert_eq!(ids(&harvester.catalog()), ["a", "d"]);

    let (job, errors) = last_job(&store);
    assert_eq!(job.record.trigger, Trigger::Start);
    assert_eq!(job.record.outcome, JobOutcome::Partial);
    assert_eq!(job.record.counts.added, 2);
    assert_eq!(job.record.counts.rejected, 2);
    let rejected: Vec<Option<&str>> = errors
        .iter()
        .map(|error| error.dataset_id.as_deref())
        .collect();
    assert_eq!(rejected, [Some("b"), Some("c")]);
}

/// The other forms of an offer that is not understood are `odrl`
/// failures that name the offer too: an unknown field deep inside a
/// logical constraint, `null` or one object in place of a list, and a
/// policy type that is not `Offer`. When only the second offer of a
/// dataset is at fault, the dataset still stops.
#[test]
fn each_form_of_an_offer_that_is_not_understood_is_an_odrl_failure() {
    let constraint = json!({"leftOperand": "dateTime", "operator": "lteq",
        "rightOperand": "2030-12-31"});
    let mut unit_inside = constraint.clone();
    unit_inside["unit"] = json!("http://example.com/unit");
    let cases = [
        (
            "/hasPolicy/0/permission/0/constraint",
            json!([{"and": [{"or": [unit_inside]}]}]),
            "unknown field `unit`",
        ),
        (
            "/hasPolicy/0/permission/0/constraint",
            json!([{"and": [], "uid": "urn:c"}]),
            "unknown field `uid` in a logical constraint",
        ),
        (
            "/hasPolicy/0/permission/0/constraint",
            constraint,
            "expected a sequence",
        ),
        ("/hasPolicy/0/permission", Value::Null, "invalid type: null"),
        ("/hasPolicy/0/@type", json!("Set"), "Offer"),
    ];
    let check = dsp();
    for (pointer, value, reason) in cases {
        let mut dataset = entry("a");
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        dataset.pointer_mut(parent).unwrap()[field] = value;
        let Err(failure) = check.check(&dataset) else {
            panic!("{pointer}: the dataset passed");
        };
        assert_eq!(failure.kind, CheckKind::Odrl, "{pointer}");
        let message = failure.message;
        assert!(
            message.starts_with("offer \"a:offer\" is not understood"),
            "{message}"
        );
        assert!(message.contains(reason), "{pointer}: {message}");
    }

    let mut second_offer = entry("a");
    second_offer["hasPolicy"] = json!([
        {"@id": "a:first", "@type": "Offer", "permission": [{"action": "use"}]},
        {"@id": "a:second", "@type": "Offer", "permission": [{"action": "use", "duty": []}]}
    ]);
    let Err(failure) = check.check(&second_offer) else {
        panic!("the dataset passed");
    };
    assert_eq!(failure.kind, CheckKind::Odrl);
    assert!(
        failure
            .message
            .starts_with("offer \"a:second\" is not understood: unknown field `duty`"),
        "{}",
        failure.message
    );
}

/// The DSP rules, and something done by "another process" the first time
/// an entry is checked: after the run read the stored datasets, before
/// it writes.
struct Meanwhile(Mutex<Option<Box<dyn FnOnce() + Send>>>);

impl DatasetCheck for Meanwhile {
    fn rules_version(&self) -> String {
        dsp().rules_version()
    }
    fn check(&self, entry: &Value) -> Result<CheckedDataset, EntryFailure> {
        let first = self.0.lock().unwrap().take();
        if let Some(other_process) = first {
            other_process();
        }
        dsp().check(entry)
    }
}

/// Two processes harvest at once (ADR 0029): one writes its run while
/// the other plans. The other's run, planned from a copy that is no
/// longer the stored one, is not written; it is planned again from the
/// stored copy, so the dataset the first one added is withdrawn, not
/// left in the catalog.
#[test]
fn a_run_planned_from_an_older_copy_is_planned_again() {
    let store = memory();
    let first_source = Source::with(vec![entry("a")]);
    let first = Arc::new(started(&first_source, &store, dsp()));
    let second_source = Source::with(vec![entry("a"), entry("c")]);
    let other = Arc::clone(&first);
    let meanwhile = Meanwhile(Mutex::new(Some(Box::new(move || {
        first_source.set(vec![entry("a"), entry("b")]);
        assert!(matches!(other.run(Trigger::Interval), RunResult::Done(_)));
    }))));
    let second = DatasetHarvester::new(second_source, Arc::new(meanwhile), Arc::clone(&store));
    let job = done(second.run(Trigger::Interval));
    assert_eq!(ids(&second.catalog()), ["a", "c"]);
    let withdrawn = store.dataset("b").unwrap().unwrap();
    assert_eq!(withdrawn.state, DatasetState::Withdrawn);
    assert_eq!(
        (job.record.counts.added, job.record.counts.withdrawn),
        (1, 1)
    );
    // the start, the first process's run, the second one's: no other
    assert_eq!(store.jobs(None, 10).unwrap().len(), 3);
}
