// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The harvested datasets file (`portitor_connector::harvest`, ADR 0020):
//! the input is the file, read by `FileSource` as the connector reads it,
//! and each entry is checked with the DSP and ODRL rules and then with
//! a bundle's dataset shapes (`BothChecks`), as a connector with a bundle
//! does. Nothing may panic; every entry is counted once; every dataset
//! in the catalog has an offer and a distribution and a unique `@id`;
//! and a second run on the same file changes nothing.
#![no_main]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use libfuzzer_sys::fuzz_target;
use portitor_connector::harvest::{
    BothChecks, DatasetCheck, DatasetHarvester, DatasetStore, DspCheck, FileSource,
    InMemoryDatasetStore, Trigger,
};
use portitor_connector::shacl::BundleRules;
use portitor_dcp::bundle::{self, FileEntry, Manifest, MediaType, Role};
use portitor_dsp::catalog::CatalogDatasets;
use portitor_dsp::policy::CredentialTypes;
use portitor_dsp::Dataset;

const PUBLIC: &str = "http://publications.europa.eu/resource/authority/access-right/PUBLIC";

/// Dataset shapes for the `PUBLIC` access right: a title, dates, a
/// distribution with a format, and an `https` landing page.
const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix dcat: <http://www.w3.org/ns/dcat#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:shapes:Dataset> a sh:NodeShape ; sh:targetClass dcat:Dataset ;
  sh:property [ sh:path dct:title ; sh:minCount 1 ; sh:datatype xsd:string ] ;
  sh:property [ sh:path dct:issued ; sh:maxCount 1 ;
                sh:or ( [ sh:datatype xsd:date ] [ sh:datatype xsd:dateTime ] ) ] ;
  sh:property [ sh:path dcat:distribution ; sh:minCount 1 ; sh:node <urn:shapes:Distribution> ] ;
  sh:property [ sh:path dcat:landingPage ; sh:pattern \"^https://\" ; sh:severity sh:Warning ] .
<urn:shapes:Distribution> sh:property [ sh:path dct:format ; sh:minCount 1 ] .";

/// The checks a connector with this bundle runs on each entry: the DSP
/// and ODRL rules, then the bundle's shapes.
static CHECK: LazyLock<Arc<dyn DatasetCheck>> = LazyLock::new(|| {
    let shapes_entry = FileEntry {
        path: "public.ttl".to_owned(),
        sha256: bundle::sha256_hex(SHAPES.as_bytes()),
        media_type: MediaType::Turtle,
        role: Role::Datasets {
            access_rights: vec![PUBLIC.to_owned()],
        },
    };
    let manifest = Manifest {
        version: "2026.11.0".to_owned(),
        issued: "2026-11-17T00:00:00Z".to_owned(),
        authority: "did:web:authority.example".to_owned(),
        profile: "https://profile.example/1".to_owned(),
        files: vec![shapes_entry],
    };
    let files = BTreeMap::from([("public.ttl".to_owned(), SHAPES.as_bytes().to_vec())]);
    let verified = bundle::check_files(manifest, files).expect("bundle");
    let rules = BundleRules::from_bundle(&verified).expect("rules");
    let shacl = rules.dataset_check().expect("dataset shapes");
    let dsp = Arc::new(DspCheck::new(
        "https://provider.example/dsp/2025-1",
        &CredentialTypes::default(),
    ));
    Arc::new(BothChecks::new(dsp, shacl))
});

/// The datasets file, one per process (the fuzzer may run several).
static FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::temp_dir().join(format!(
        "portitor-fuzz-datasets-{}.json",
        std::process::id()
    ))
});

/// The datasets in the catalog now, without their access policies.
fn catalog_datasets(harvester: &DatasetHarvester) -> Vec<Dataset> {
    harvester
        .catalog()
        .all_with_access_policies()
        .expect("catalog")
        .into_iter()
        .map(|listed| listed.dataset)
        .collect()
}

/// Checks the catalog after a run: offers, distributions, unique ids.
fn check_catalog(harvester: &DatasetHarvester) {
    let mut ids = BTreeSet::new();
    for dataset in catalog_datasets(harvester) {
        assert!(
            !dataset.has_policy.is_empty(),
            "{} has no offer",
            dataset.id
        );
        assert!(
            !dataset.distribution.is_empty(),
            "{} has no distribution",
            dataset.id
        );
        assert!(
            ids.insert(dataset.id.clone()),
            "{} is listed twice",
            dataset.id
        );
    }
}

/// Checks the newest job's counts after a full run: each entry is
/// counted once. (A run that finds the same file only counts the
/// entries seen.)
fn check_last_job(store: &InMemoryDatasetStore) {
    let jobs = store.jobs(None, 1).expect("jobs");
    let Some(job) = jobs.first() else {
        return;
    };
    let counts = job.record.counts;
    assert_eq!(
        counts.added + counts.updated + counts.unchanged + counts.rejected,
        counts.seen,
        "{counts:?}"
    );
}

fuzz_target!(|data: &[u8]| {
    if std::fs::write(FILE.as_path(), data).is_err() {
        return;
    }
    let store = Arc::new(InMemoryDatasetStore::default());
    let harvester = DatasetHarvester::new(
        Arc::new(FileSource::new(FILE.as_path())),
        Arc::clone(&CHECK),
        Arc::clone(&store) as Arc<dyn DatasetStore>,
    );
    harvester.start().expect("start");
    check_last_job(&store);
    check_catalog(&harvester);
    let first = catalog_datasets(&harvester);
    // the same file again: nothing changes
    let _ = harvester.run(Trigger::Interval);
    assert_eq!(catalog_datasets(&harvester), first);
});
