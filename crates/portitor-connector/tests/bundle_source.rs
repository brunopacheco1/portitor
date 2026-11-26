// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The signed bundle end to end, in one process (ADR 0005): the issuer
//! builds bundles with `bundle build` and serves them, `latest.json` and
//! its DID document over plain HTTP on 127.0.0.1; the connector's
//! [`BundleSource`] fetches them and its [`BundleHarvester`] takes them,
//! at start and when a newer version comes, with plain HTTP allowed (as
//! `PORTITOR_ALLOW_HTTP` allows it for local tests).
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use portitor_bundle::source::{
    BundleConfig, BundleLocation, BundleSource, BundleSourceError, VersionChoice, source_for,
};
use portitor_bundle::store::{CacheFolder, IN_USE_FILE, KEY_FILE};
use portitor_connector::bundle_harvest::{
    BundleHarvester, BundleListener, SharedBundleVersion, Started,
};
use portitor_connector::harvest::{JobKind, JobOutcome, JobRecord, Trigger};
use portitor_connector::store::ConnectorStore;
use portitor_dcp::SigningKey;
use portitor_dcp::bundle::BundleError;
use portitor_issuer::bundle_build::{Options, build};
use portitor_issuer::config::Config;
use portitor_issuer::store::DatabaseStore;
use portitor_issuer::{Issuer, Ports};
use portitor_runtime::directory::WebDirectory;
use portitor_runtime::http::HttpSender;

const VERSION: &str = "2026.11.0";
const PUBLIC: &str = "http://publications.europa.eu/resource/authority/access-right/PUBLIC";
const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n";
const CONTEXT: &str = r#"{"@context": {"dct": "http://purl.org/dc/terms/"}}"#;

/// A new empty folder under the system's temporary folder.
fn scratch() -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "portitor-bundle-source-{}",
        portitor_runtime::system::new_uuid()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// The authority: an issuer on 127.0.0.1 with a bundle key, serving one
/// built bundle.
struct Authority {
    did: String,
    bundle_key: SigningKey,
    /// The issuer's credential key, listed under `assertionMethod` next
    /// to the bundle key, and its id.
    credential_key: SigningKey,
    credential_kid: String,
    bundles: PathBuf,
    url: String,
    server: tokio::task::JoinHandle<()>,
}

impl Authority {
    /// Builds the bundle, then starts the issuer. Without
    /// `list_bundle_key`, the DID document does not list the bundle key.
    async fn start(list_bundle_key: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let did = format!("did:web:127.0.0.1%3A{port}");
        let bundle_key = SigningKey::generate().unwrap();
        let bundles = scratch();
        build(
            &build_options(&did, &bundles, VERSION, "2026-11-14T00:00:00Z"),
            &bundle_key,
        )
        .unwrap();
        let config = Config {
            participant_id: did.clone(),
            key_file: "unused".into(),
            retired_key_files: Vec::new(),
            credential_definitions_file: "unused".into(),
            issuer_service_url: format!("http://127.0.0.1:{port}/issuer"),
            bind: "unused".to_owned(),
            database: portitor_runtime::database::DatabaseSetting::SqliteFile("unused".into()),
            management: portitor_issuer::config::ManagementConfig::health_only(
                portitor_issuer::config::DEFAULT_MANAGEMENT_BIND,
            ),
            allow_http: true,
            max_active_per_holder: 10,
            bundle_key_file: None,
            bundle_dir: Some(bundles.clone()),
        };
        let ports = Ports {
            store: Arc::new(DatabaseStore::in_memory().unwrap()),
            directory: Arc::new(WebDirectory::new(true, 300).unwrap()),
            sender: Arc::new(HttpSender::new(true).unwrap()),
        };
        let credential_key = SigningKey::generate().unwrap();
        let mut issuer = Issuer::new(
            &config,
            SigningKey::from_pkcs8(credential_key.pkcs8()).unwrap(),
            Vec::new(),
            ports,
        )
        .unwrap();
        let credential_kid = issuer.kid().to_owned();
        if list_bundle_key {
            issuer.add_bundle_key(&bundle_key.public_jwk());
        }
        let app = portitor_issuer::router(Arc::new(issuer));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://127.0.0.1:{port}/bundles/"),
            did,
            bundle_key,
            credential_key,
            credential_kid,
            bundles,
            server,
        }
    }

    /// The connector's settings for these bundles, over HTTP, following
    /// the newest version (no schedule: the tests call `check`).
    fn settings(&self, cache: &Path) -> BundleConfig {
        BundleConfig {
            location: BundleLocation::Url(self.url.clone()),
            version: VersionChoice::Latest,
            authority: self.did.clone(),
            cache_dir: cache.to_owned(),
            refresh_seconds: 0,
            allow_http: true,
        }
    }

    /// Builds and publishes another version, issued on `issued`.
    fn publish(&self, version: &str, issued: &str) {
        build(
            &build_options(&self.did, &self.bundles, version, issued),
            &self.bundle_key,
        )
        .unwrap();
    }

    /// Points `latest.json` at `version`, as a host could.
    fn point_latest_at(&self, version: &str) {
        let latest = serde_json::json!({ "version": version }).to_string();
        std::fs::write(self.bundles.join("latest.json"), latest).unwrap();
    }

    /// Stops serving, as if the authority's host were down.
    async fn stop(self) {
        self.server.abort();
        let _ = self.server.await;
    }
}

fn build_options(did: &str, output: &Path, version: &str, issued: &str) -> Options {
    let input = scratch();
    std::fs::create_dir_all(input.join("shapes")).unwrap();
    std::fs::write(input.join("shapes/public.ttl"), SHAPES).unwrap();
    std::fs::write(input.join("dct.jsonld"), CONTEXT).unwrap();
    let description = serde_json::json!({"files": [
        {"path": "shapes/public.ttl", "role": {"type": "datasets", "accessRights": [PUBLIC]}},
        {"path": "dct.jsonld", "role": {"type": "context", "url": "https://contexts.example/dct"}}
    ]});
    std::fs::write(input.join("bundle.json"), description.to_string()).unwrap();
    Options {
        input,
        output: output.to_owned(),
        version: version.to_owned(),
        profile: "https://healthdcat-ap.example/release-8".to_owned(),
        authority: did.to_owned(),
        kid: format!("{did}#bundle-key"),
        key: "unused.pem".into(),
        issued: Some(issued.to_owned()),
        not_latest: false,
    }
}

/// The source for `settings`, resolving DIDs over plain HTTP.
fn source(settings: &BundleConfig) -> Arc<dyn BundleSource> {
    source_for(settings, Arc::new(WebDirectory::new(true, 300).unwrap())).unwrap()
}

/// A connector start: the bundle harvester takes a version.
async fn start(settings: &BundleConfig) -> Result<Started, String> {
    let store = Arc::new(CacheFolder::new(&settings.cache_dir));
    // a new, empty database each time: the version in use comes from
    // the cache folder, as for a database of 1.2
    let database = Arc::new(ConnectorStore::in_memory().unwrap());
    BundleHarvester::start(settings.clone(), source(settings), store, database).await
}

/// A listener that keeps what it hears.
#[derive(Default)]
struct Heard {
    jobs: Mutex<Vec<JobRecord>>,
    rules_changed: Mutex<usize>,
    rules_moved: Mutex<usize>,
}

impl BundleListener for Heard {
    fn record(&self, job: JobRecord) {
        self.jobs.lock().unwrap().push(job);
    }

    fn rules_changed(&self) {
        *self.rules_changed.lock().unwrap() += 1;
    }

    fn rules_moved(&self) {
        *self.rules_moved.lock().unwrap() += 1;
    }
}

#[tokio::test]
async fn the_connector_takes_the_bundle_the_issuer_serves() {
    let authority = Authority::start(true).await;
    let cache = scratch();
    let settings = authority.settings(&cache);
    assert_eq!(source(&settings).latest().await.unwrap(), VERSION);
    let fetched = source(&settings).fetch(VERSION).await.unwrap();
    let bundle = &fetched.bundle;
    assert_eq!(bundle.version(), VERSION);
    assert_eq!(bundle.issued(), "2026-11-14T00:00:00Z");
    assert_eq!(bundle.profile(), "https://healthdcat-ap.example/release-8");
    let shapes = bundle.datasets_files_for(PUBLIC);
    assert_eq!(shapes[0].bytes, SHAPES.as_bytes());
    assert_eq!(
        bundle
            .contexts()
            .get("https://contexts.example/dct")
            .copied(),
        Some(CONTEXT.as_bytes())
    );
    let started = start(&settings).await.unwrap();
    assert_eq!(started.harvester.copy().current().version(), VERSION);
    assert_eq!(started.job.kind, JobKind::Bundle);
    assert_eq!(started.job.trigger, Trigger::Start);
    assert_eq!(started.job.outcome, JobOutcome::Succeeded);
    assert!(started.job.snapshot_sha256.is_some(), "the manifest's hash");
    // kept in the cache, with the key it was checked with, and in use
    let cached = cache.join(VERSION);
    for file in [
        "manifest.json",
        "manifest.jws",
        KEY_FILE,
        "shapes/public.ttl",
    ] {
        assert!(cached.join(file).is_file(), "{file} cached");
    }
    assert!(cache.join(IN_USE_FILE).is_file());
    // a second start replaces the cached copy
    assert!(start(&settings).await.is_ok());
    authority.stop().await;
}

#[tokio::test]
async fn a_newer_version_is_taken_without_a_restart() {
    let authority = Authority::start(true).await;
    let cache = scratch();
    let started = start(&authority.settings(&cache)).await.unwrap();
    let harvester = Arc::new(started.harvester);
    let heard = Arc::new(Heard::default());
    let listener: Arc<dyn BundleListener> = heard.clone();
    // nothing new
    let job = harvester.run(Trigger::Interval, &listener).await;
    assert_eq!(job.outcome, JobOutcome::Unchanged, "{}", job.summary);
    assert_eq!(*heard.rules_changed.lock().unwrap(), 0);
    // the authority publishes a newer version
    authority.publish("2026.12.0", "2026-12-01T00:00:00Z");
    let copy = harvester.copy();
    let job = harvester.run(Trigger::Interval, &listener).await;
    assert_eq!(job.outcome, JobOutcome::Succeeded, "{}", job.summary);
    assert_eq!(job.kind, JobKind::Bundle);
    assert!(
        job.summary.contains("2026.12.0 in use (was 2026.11.0)"),
        "{}",
        job.summary
    );
    assert_eq!(
        copy.current().version(),
        "2026.12.0",
        "the copy handed out changed too"
    );
    assert_eq!(harvester.in_use().version, "2026.12.0");
    assert_eq!(
        *heard.rules_changed.lock().unwrap(),
        1,
        "the datasets are checked again"
    );
    assert_eq!(
        heard.jobs.lock().unwrap().len(),
        2,
        "both runs in the job log"
    );
    // the old version's copy is gone from the cache; a restart takes the new one
    assert!(!cache.join(VERSION).exists());
    let again = start(&authority.settings(&cache)).await.unwrap();
    assert_eq!(again.harvester.in_use().version, "2026.12.0");
    authority.stop().await;
}

#[tokio::test]
async fn an_older_version_is_never_taken() {
    let authority = Authority::start(true).await;
    let cache = scratch();
    let started = start(&authority.settings(&cache)).await.unwrap();
    // a version issued before the one in use, named by latest.json
    authority.publish("2026.10.0", "2026-10-01T00:00:00Z");
    authority.point_latest_at("2026.10.0");
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed);
    assert!(
        job.summary.contains("never replaced by an older one"),
        "{}",
        job.summary
    );
    assert!(!changed);
    assert_eq!(started.harvester.copy().current().version(), VERSION);
    // nor at the next start: the version in use is kept, and the run says why
    let restarted = start(&authority.settings(&cache)).await.unwrap();
    assert_eq!(restarted.harvester.in_use().version, VERSION);
    assert_eq!(restarted.job.outcome, JobOutcome::Failed);
    assert!(
        restarted.job.summary.contains("keeping version 2026.11.0"),
        "{}",
        restarted.job.summary
    );
    authority.stop().await;
}

/// Versions issued on the same day are ordered by their time
/// (`dcterms:issued` as an `xsd:dateTime`): the later one is taken, and
/// the connector is never moved back to the earlier one.
#[tokio::test]
async fn versions_of_the_same_day_are_ordered_by_their_time() {
    let authority = Authority::start(true).await;
    let started = start(&authority.settings(&scratch())).await.unwrap();
    authority.publish("2026.11.1", "2026-11-14T09:00:00Z");
    authority.publish("2026.11.2", "2026-11-14T15:30:00Z");
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Succeeded, "{}", job.summary);
    assert!(changed);
    assert_eq!(started.harvester.in_use().version, "2026.11.2");
    assert_eq!(started.harvester.in_use().issued, "2026-11-14T15:30:00Z");
    // pointed back at the morning version: refused
    authority.point_latest_at("2026.11.1");
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed);
    assert!(
        job.summary.contains("never replaced by an older one"),
        "{}",
        job.summary
    );
    assert!(!changed);
    assert_eq!(started.harvester.in_use().version, "2026.11.2");
    authority.stop().await;
}

#[tokio::test]
async fn a_new_version_that_fails_a_check_leaves_the_rules_in_use() {
    let authority = Authority::start(true).await;
    let started = start(&authority.settings(&scratch())).await.unwrap();
    authority.publish("2026.12.0", "2026-12-01T00:00:00Z");
    std::fs::write(authority.bundles.join("2026.12.0/dct.jsonld"), "{}").unwrap();
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed);
    assert!(job.summary.contains("dct.jsonld"), "{}", job.summary);
    assert!(!changed);
    assert_eq!(started.harvester.in_use().version, VERSION);
    // latest.json that cannot be read: the same
    std::fs::write(authority.bundles.join("latest.json"), "{").unwrap();
    let (job, _) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed);
    assert!(job.summary.starts_with("latest.json"), "{}", job.summary);
    authority.stop().await;
}

#[tokio::test]
async fn a_fixed_version_never_changes() {
    let authority = Authority::start(true).await;
    let mut settings = authority.settings(&scratch());
    settings.version = VersionChoice::Fixed(VERSION.to_owned());
    settings.refresh_seconds = 60;
    assert_eq!(settings.refresh_every(), None);
    let started = start(&settings).await.unwrap();
    authority.publish("2026.12.0", "2026-12-01T00:00:00Z");
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Unchanged);
    assert!(!changed);
    assert_eq!(started.harvester.in_use().version, VERSION);
    authority.stop().await;
}

#[tokio::test]
async fn the_cache_is_used_when_the_authority_is_down() {
    let authority = Authority::start(true).await;
    let cache = scratch();
    let settings = authority.settings(&cache);
    start(&settings).await.unwrap();
    authority.stop().await;
    // the host is down: the version in use, from the cache, checked again
    let started = start(&settings).await.unwrap();
    assert_eq!(started.harvester.in_use().version, VERSION);
    assert_eq!(
        started.job.outcome,
        JobOutcome::Failed,
        "the run says latest.json failed"
    );
    // a fixed version: its cached copy
    let mut fixed = settings.clone();
    fixed.version = VersionChoice::Fixed(VERSION.to_owned());
    let started = start(&fixed).await.unwrap();
    assert_eq!(started.harvester.in_use().version, VERSION);
    // a cached copy that was changed is not used
    std::fs::write(cache.join(VERSION).join("shapes/public.ttl"), "changed").unwrap();
    let error = start(&settings).await.unwrap_err();
    assert!(error.contains("SHA-256"), "{error}");
    // nor a cached copy of another version than the fixed one
    fixed.version = VersionChoice::Fixed("2026.12.0".to_owned());
    let error = start(&fixed).await.unwrap_err();
    assert!(error.contains("no usable cached copy"), "{error}");
}

#[tokio::test]
async fn without_any_cache_a_down_authority_is_an_error() {
    let authority = Authority::start(true).await;
    let settings = authority.settings(&scratch());
    authority.stop().await;
    let error = start(&settings).await.unwrap_err();
    assert!(error.starts_with("latest.json"), "{error}");
    let mut fixed = settings;
    fixed.version = VersionChoice::Fixed(VERSION.to_owned());
    let error = start(&fixed).await.unwrap_err();
    assert!(error.contains("no usable cached copy"), "{error}");
}

#[tokio::test]
async fn a_key_the_did_document_does_not_list_is_refused() {
    let authority = Authority::start(false).await;
    let error = source(&authority.settings(&scratch()))
        .fetch(VERSION)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            BundleSourceError::Invalid(BundleError::KeyNotInDocument { .. })
        ),
        "{error}"
    );
    authority.stop().await;
}

#[tokio::test]
async fn another_authority_or_version_is_refused() {
    let authority = Authority::start(true).await;
    let mut other_authority = authority.settings(&scratch());
    other_authority.authority = "did:web:127.0.0.1%3A1".to_owned();
    assert!(matches!(
        source(&other_authority).fetch(VERSION).await,
        Err(BundleSourceError::Invalid(BundleError::WrongSigner { .. }))
    ));
    // a folder named 2026.12.0 that holds version 2026.11.0
    copy_folder(
        &authority.bundles.join(VERSION),
        &authority.bundles.join("2026.12.0"),
    );
    assert!(matches!(
        source(&authority.settings(&scratch()))
            .fetch("2026.12.0")
            .await,
        Err(BundleSourceError::Invalid(BundleError::WrongVersion { .. }))
    ));
    authority.stop().await;
}

/// Copies the files of `from` (and its sub-folders) to `to`.
fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test]
async fn a_bundle_signed_with_the_credential_key_is_refused() {
    let authority = Authority::start(true).await;
    // re-sign the served manifest with the credential key, which the
    // DID document also lists under assertionMethod
    let folder = authority.bundles.join(VERSION);
    let manifest = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
    let raw = serde_json::value::RawValue::from_string(manifest).unwrap();
    let jws = authority
        .credential_key
        .sign_json(
            &authority.credential_kid,
            portitor_dcp::bundle::JWS_TYPE,
            &raw,
        )
        .unwrap();
    std::fs::write(folder.join("manifest.jws"), jws).unwrap();
    let error = source(&authority.settings(&scratch()))
        .fetch(VERSION)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            BundleSourceError::Invalid(BundleError::WrongSigner { .. })
        ),
        "{error}"
    );
    authority.stop().await;
}

/// The folder adapter, on the issuer's own output folder.
fn folder_settings(authority: &Authority) -> BundleConfig {
    BundleConfig {
        location: BundleLocation::Folder(authority.bundles.clone()),
        version: VersionChoice::Latest,
        authority: authority.did.clone(),
        cache_dir: scratch(),
        refresh_seconds: 0,
        allow_http: true,
    }
}

#[tokio::test]
async fn a_bundle_folder_is_checked_like_a_download() {
    let authority = Authority::start(true).await;
    let folder = authority.bundles.join(VERSION);
    let settings = folder_settings(&authority);
    assert_eq!(source(&settings).latest().await.unwrap(), VERSION);
    // a file the manifest does not list is ignored
    std::fs::write(folder.join("notes.txt"), "ignored").unwrap();
    let fetched = source(&settings).fetch(VERSION).await.unwrap();
    assert_eq!(fetched.bundle.files().len(), 2);
    assert!(start(&settings).await.is_ok());
    // a changed file
    std::fs::write(folder.join("dct.jsonld"), "{}").unwrap();
    assert_eq!(
        source(&settings).fetch(VERSION).await.unwrap_err(),
        BundleSourceError::Invalid(BundleError::DigestMismatch("dct.jsonld".to_owned()))
    );
    // a missing file
    std::fs::remove_file(folder.join("dct.jsonld")).unwrap();
    assert!(matches!(
        source(&settings).fetch(VERSION).await,
        Err(BundleSourceError::Unavailable(_))
    ));
    authority.stop().await;
}

// ---- ADR 0005, amended in 0.13: a file whose role this version does not
// ---- know is skipped; a bundle of which nothing is known is refused

impl Authority {
    /// Publishes `version` as a later version of the build tool would:
    /// built as usual, then `change` is applied to the manifest (it adds
    /// or changes roles this version does not know) and the manifest is
    /// signed again with the bundle key. This version's `bundle build`
    /// refuses such roles, so the test signs by hand.
    fn publish_from_a_later_build_tool(
        &self,
        version: &str,
        issued: &str,
        change: impl Fn(&mut serde_json::Value),
    ) {
        self.publish(version, issued);
        let folder = self.bundles.join(version);
        let text = std::fs::read_to_string(folder.join("manifest.json")).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
        change(&mut manifest);
        let text = manifest.to_string();
        std::fs::write(folder.join("manifest.json"), &text).unwrap();
        let raw = serde_json::value::RawValue::from_string(text).unwrap();
        let kid = format!("{}#bundle-key", self.did);
        let jws = self
            .bundle_key
            .sign_json(&kid, portitor_dcp::bundle::JWS_TYPE, &raw)
            .unwrap();
        std::fs::write(folder.join("manifest.jws"), jws).unwrap();
    }
}

/// Adds a file with a role of a later version to the manifest. The file
/// itself is not put on disk: a connector that skips it never reads it.
fn add_a_policies_file(manifest: &mut serde_json::Value) {
    let later = serde_json::json!({
        "path": "policies/odrl.jsonld",
        "sha256": portitor_dcp::bundle::sha256_hex(b"never read"),
        "mediaType": "application/ld+json",
        "role": {"type": "policies", "appliesTo": ["offers"], "strict": true}
    });
    manifest["files"].as_array_mut().unwrap().push(later);
}

/// Gives every file a role of a later version.
fn make_every_role_unknown(manifest: &mut serde_json::Value) {
    for file in manifest["files"].as_array_mut().unwrap() {
        file["role"] = serde_json::json!({"type": "policies"});
    }
}

/// Log lines collected on the test's thread, as JSON.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    /// Collects until the guard is dropped.
    fn start() -> (Self, tracing::subscriber::DefaultGuard) {
        // other tests run at the same time on threads without a subscriber
        portitor_runtime::telemetry::testing::keep_a_second_subscriber();
        let log = Self::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        (log, tracing::subscriber::set_default(subscriber))
    }
    fn lines(&self) -> Vec<serde_json::Value> {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn a_newer_version_with_a_file_of_an_unknown_role_is_taken_and_the_file_is_skipped() {
    let authority = Authority::start(true).await;
    let settings = folder_settings(&authority);
    let started = start(&settings).await.unwrap();
    assert_eq!(started.job.summary, "version 2026.11.0 in use");
    assert!(
        started
            .harvester
            .copy()
            .current()
            .skipped_files()
            .is_empty()
    );

    authority.publish_from_a_later_build_tool(
        "2026.12.0",
        "2026-12-01T00:00:00Z",
        add_a_policies_file,
    );
    let (log, guard) = Log::start();
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    drop(guard);
    assert_eq!(job.outcome, JobOutcome::Succeeded, "{}", job.summary);
    assert!(changed);
    assert_eq!(
        job.summary,
        "version 2026.12.0 in use (was 2026.11.0); \
         1 file skipped (unknown role, its rules are not enforced)"
    );
    // the files it knows are used as before
    let rules = started.harvester.copy().current();
    assert_eq!(rules.version(), "2026.12.0");
    assert!(rules.dataset_check().is_some(), "the datasets shapes");
    let skipped = rules.skipped_files();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].path, "policies/odrl.jsonld");
    assert_eq!(skipped[0].role_type, "policies");

    // one WARN names the path and the role type
    let lines = log.lines();
    let warnings: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| {
            let message = line["fields"]["message"].as_str().unwrap_or_default();
            message.starts_with("bundle file skipped")
        })
        .collect();
    assert_eq!(warnings.len(), 1, "{lines:?}");
    assert_eq!(warnings[0]["level"], "WARN");
    assert_eq!(warnings[0]["fields"]["path"], "policies/odrl.jsonld");
    assert_eq!(warnings[0]["fields"]["role_type"], "policies");
    assert_eq!(warnings[0]["fields"]["version"], "2026.12.0");

    // the skipped file is not in the cache, and the cached copy is good:
    // a start without the authority's folder takes it, and says so again
    let cached = settings.cache_dir.join("2026.12.0");
    assert!(cached.join("shapes/public.ttl").is_file());
    assert!(!cached.join("policies").exists());
    std::fs::remove_dir_all(&authority.bundles).unwrap();
    let restarted = start(&settings).await.unwrap();
    assert_eq!(restarted.harvester.in_use().version, "2026.12.0");
    assert!(
        restarted.job.summary.contains("1 file skipped"),
        "{}",
        restarted.job.summary
    );
    authority.stop().await;
}

/// Adds two files of later roles whose media types this version does
/// not read (ADR 0005, amended in 0.13: any `type/subtype` of RFC 6838,
/// section 4.2), one with the longest role name allowed (64 letters).
fn add_files_with_other_media_types(manifest: &mut serde_json::Value) {
    let files = manifest["files"].as_array_mut().unwrap();
    files.push(serde_json::json!({
        "path": "rules/limits.json",
        "sha256": portitor_dcp::bundle::sha256_hex(b"never read"),
        "mediaType": "application/json",
        "role": {"type": "r".repeat(64)}
    }));
    files.push(serde_json::json!({
        "path": "rules/terms.xml",
        "sha256": portitor_dcp::bundle::sha256_hex(b"never read either"),
        "mediaType": "application/vnd.example.rules-v2+xml",
        "role": {"type": "terms", "media": ["any"], "since": 2}
    }));
}

/// The 0.13 change that the manifest tests of `portitor-dcp` cover for
/// the manifest alone, here through the connector: files whose media
/// type is outside this version's three are skipped like any file of an
/// unknown role, and the rest of the bundle is used.
#[tokio::test]
async fn skipped_files_may_have_media_types_this_version_does_not_read() {
    let authority = Authority::start(true).await;
    let settings = folder_settings(&authority);
    let started = start(&settings).await.unwrap();

    authority.publish_from_a_later_build_tool(
        "2026.12.0",
        "2026-12-01T00:00:00Z",
        add_files_with_other_media_types,
    );
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Succeeded, "{}", job.summary);
    assert!(changed);
    assert!(job.summary.contains("2 files skipped"), "{}", job.summary);
    let rules = started.harvester.copy().current();
    assert!(rules.dataset_check().is_some(), "the datasets shapes");
    let skipped = rules.skipped_files();
    assert_eq!(skipped.len(), 2);
    assert_eq!(skipped[0].path, "rules/limits.json");
    assert_eq!(skipped[0].role_type, "r".repeat(64));
    assert_eq!(skipped[1].path, "rules/terms.xml");
    assert_eq!(skipped[1].role_type, "terms");
    // neither was fetched: the cache holds only what is used
    let cached = settings.cache_dir.join("2026.12.0");
    assert!(cached.join("shapes/public.ttl").is_file());
    assert!(!cached.join("rules").exists());
    authority.stop().await;
}

/// Writes the role of the datasets shapes as `Datasets`.
fn write_the_datasets_role_in_other_letters(manifest: &mut serde_json::Value) {
    for file in manifest["files"].as_array_mut().unwrap() {
        if file["role"]["type"] == "datasets" {
            file["role"]["type"] = serde_json::json!("Datasets");
        }
    }
}

/// A known role written in other letters (`Datasets`) is a mistake, not
/// a role of a later version. Skipping the file would leave datasets
/// unchecked with only a warning, so the new version is refused and the
/// version in use stays.
#[tokio::test]
async fn a_version_whose_datasets_role_is_written_in_other_letters_is_not_taken() {
    let authority = Authority::start(true).await;
    let settings = folder_settings(&authority);
    let started = start(&settings).await.unwrap();

    authority.publish_from_a_later_build_tool(
        "2026.12.0",
        "2026-12-01T00:00:00Z",
        write_the_datasets_role_in_other_letters,
    );
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed, "{}", job.summary);
    assert!(!changed);
    assert!(
        job.summary.contains("the type is written datasets")
            && job.summary.contains("keeping version 2026.11.0"),
        "{}",
        job.summary
    );
    assert_eq!(started.harvester.in_use().version, VERSION);
    let rules = started.harvester.copy().current();
    assert!(rules.dataset_check().is_some(), "the old rules still check");
    authority.stop().await;
}

/// Following `latest`: a newer version in which this connector knows no
/// role would give no rules at all. It is refused like any invalid
/// bundle, and the version in use stays, also after a restart.
#[tokio::test]
async fn a_newer_version_with_only_unknown_roles_is_not_taken() {
    let authority = Authority::start(true).await;
    let settings = folder_settings(&authority);
    let started = start(&settings).await.unwrap();

    authority.publish_from_a_later_build_tool(
        "2026.12.0",
        "2026-12-01T00:00:00Z",
        make_every_role_unknown,
    );
    assert_eq!(source(&settings).latest().await.unwrap(), "2026.12.0");
    assert_eq!(
        source(&settings).fetch("2026.12.0").await.unwrap_err(),
        BundleSourceError::Invalid(BundleError::NoKnownRole)
    );
    let (job, changed) = started.harvester.check(Trigger::Interval).await;
    assert_eq!(job.outcome, JobOutcome::Failed);
    assert!(!changed);
    assert!(
        job.summary
            .contains("no file has a role this version knows")
            && job.summary.contains("keeping version 2026.11.0"),
        "{}",
        job.summary
    );
    assert_eq!(started.harvester.in_use().version, VERSION);
    let rules = started.harvester.copy().current();
    assert_eq!(rules.version(), VERSION);
    assert!(rules.dataset_check().is_some(), "the old rules still check");

    // a restart keeps the version in use, and the run says why
    let restarted = start(&settings).await.unwrap();
    assert_eq!(restarted.harvester.in_use().version, VERSION);
    assert_eq!(restarted.job.outcome, JobOutcome::Failed);
    assert!(
        restarted.job.summary.contains("keeping version 2026.11.0"),
        "{}",
        restarted.job.summary
    );

    // a connector that has no version yet does not start with it
    let mut first_start = settings.clone();
    first_start.cache_dir = scratch();
    let error = start(&first_start).await.err().unwrap();
    assert!(
        error.contains("no file has a role this version knows"),
        "{error}"
    );
    authority.stop().await;
}

/// A process of a connector on `database`, with its own cache folder: the
/// connector's processes share the database, each container has its own
/// folder (TD-55).
async fn process_on(authority: &Authority, database: &Arc<ConnectorStore>) -> Started {
    let settings = authority.settings(&scratch());
    let store = Arc::new(CacheFolder::new(&settings.cache_dir));
    let shared = Arc::clone(database) as Arc<dyn SharedBundleVersion>;
    BundleHarvester::start(settings.clone(), source(&settings), store, shared)
        .await
        .unwrap()
}

/// The databases two processes of one connector can share: one `SQLite`
/// file serves one process, so its two "processes" share its connection;
/// on PostgreSQL each opens the database on its own.
fn shared_databases() -> Vec<(Arc<ConnectorStore>, Arc<ConnectorStore>)> {
    use portitor_connector::store::MIGRATIONS;
    use portitor_runtime::database::{Database, DatabaseSetting};
    let sqlite = Arc::new(ConnectorStore::in_memory().unwrap());
    let mut pairs = vec![(Arc::clone(&sqlite), sqlite)];
    if let Some(url) = portitor_runtime::testing::databases::postgres_url() {
        let settings = portitor_runtime::testing::databases::fresh_postgres_settings(&url);
        let setting = DatabaseSetting::Postgres(Box::new(settings));
        let open = || {
            let database = Database::open(&setting, &MIGRATIONS).unwrap();
            Arc::new(ConnectorStore::new(Arc::new(database)))
        };
        pairs.push((open(), open()));
    }
    pairs
}

/// TD-55: one process takes a new version; the other one moves to it
/// (from the authority: its container has no copy), and only checks the
/// stored datasets again, without a new harvest of the source.
#[tokio::test]
async fn a_version_one_process_takes_is_moved_to_by_the_other() {
    let authority = Authority::start(true).await;
    for (first_database, second_database) in shared_databases() {
        let first = process_on(&authority, &first_database).await.harvester;
        let second = process_on(&authority, &second_database).await.harvester;
        let version = first.in_use().version;
        assert!(!second.follow().await, "nothing new");

        let newer = format!("{version}.1");
        authority.publish(&newer, "2027-01-01T00:00:00Z");
        authority.point_latest_at(&newer);
        let (job, changed) = first.check(Trigger::Interval).await;
        assert!(changed, "{}", job.summary);
        assert_eq!(second.in_use().version, version, "not moved yet");
        let rules = second.copy();

        assert!(second.follow().await);
        assert_eq!(second.in_use().version, newer);
        assert_eq!(rules.current().version(), newer, "the rules in use moved");
        assert!(!second.follow().await, "moved once");

        // a process that starts now takes the connector's version
        let third = process_on(&authority, &second_database).await;
        assert_eq!(third.harvester.in_use().version, newer);
    }
    authority.stop().await;
}

/// A connector's `SQLite` file of 1.2 (its 17 migrations) has no
/// `bundle_in_use` row: the version in use is the one the cache folder's
/// `.in-use.json` names, kept also when latest.json names an older one,
/// and from then on recorded in the database.
#[tokio::test]
async fn a_database_of_1_2_takes_the_version_in_use_from_the_cache() {
    const MIGRATIONS_OF_1_2: usize = 17;
    let authority = Authority::start(true).await;
    authority.publish("2026.12.0", "2026-12-01T00:00:00Z");
    authority.point_latest_at("2026.12.0");
    let cache = scratch();
    let settings = authority.settings(&cache);
    // 1.2 took 2026.12.0, and its cache folder says so
    let first = start(&settings).await.unwrap();
    assert_eq!(first.harvester.in_use().version, "2026.12.0");
    drop(first);
    let dir = portitor_runtime::testing::TempDir::new("bundle-database-of-1-2");
    let path = dir.path().join("connector.db");
    let migrations = portitor_connector::store::MIGRATIONS.sqlite;
    drop(portitor_runtime::sqlite::open(Some(&path), &migrations[..MIGRATIONS_OF_1_2]).unwrap());
    let database = Arc::new(ConnectorStore::open_sqlite_file(&path).unwrap());
    assert_eq!(database.bundle_in_use().unwrap(), None);

    authority.point_latest_at(VERSION);
    let store = Arc::new(CacheFolder::new(&settings.cache_dir));
    let shared = Arc::clone(&database) as Arc<dyn SharedBundleVersion>;
    let started = BundleHarvester::start(settings.clone(), source(&settings), store, shared)
        .await
        .unwrap();
    assert_eq!(started.harvester.in_use().version, "2026.12.0");
    let recorded = database.bundle_in_use().unwrap().unwrap();
    assert_eq!(recorded.version, "2026.12.0");
    authority.stop().await;
}

/// TD-55: a process that starts while latest.json names an older version
/// (a host that moved back) keeps the connector's version, also from a
/// container with no copy of it.
#[tokio::test]
async fn a_new_process_takes_the_connectors_version_not_an_older_latest() {
    let authority = Authority::start(true).await;
    let database = Arc::new(ConnectorStore::in_memory().unwrap());
    authority.publish("2026.12.0", "2026-12-01T00:00:00Z");
    authority.point_latest_at("2026.12.0");
    let first = process_on(&authority, &database).await;
    assert_eq!(first.harvester.in_use().version, "2026.12.0");
    authority.point_latest_at(VERSION);
    let second = process_on(&authority, &database).await;
    assert_eq!(second.harvester.in_use().version, "2026.12.0");
    assert_eq!(
        second.job.outcome,
        JobOutcome::Failed,
        "{}",
        second.job.summary
    );
    assert!(
        second.job.summary.contains("keeping version 2026.12.0"),
        "{}",
        second.job.summary
    );
    authority.stop().await;
}

/// The listener of a process that moved: only the stored datasets are
/// checked again (`rules_moved`), never a new harvest (`rules_changed`).
#[tokio::test]
async fn a_process_that_moved_tells_its_listener_once() {
    let authority = Authority::start(true).await;
    let database = Arc::new(ConnectorStore::in_memory().unwrap());
    let first = process_on(&authority, &database).await.harvester;
    let second = Arc::new(process_on(&authority, &database).await.harvester);
    let heard = Arc::new(Heard::default());
    let follow = second.follow_database(heard.clone()).unwrap();
    authority.publish("2027.1.0", "2027-01-01T00:00:00Z");
    authority.point_latest_at("2027.1.0");
    let (_, changed) = first.check(Trigger::Interval).await;
    assert!(changed);
    // the follow loop looks every 5 seconds
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while *heard.rules_moved.lock().unwrap() == 0 {
        assert!(std::time::Instant::now() < deadline, "not moved");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(second.in_use().version, "2027.1.0");
    assert_eq!(*heard.rules_changed.lock().unwrap(), 0);
    follow.abort();
    authority.stop().await;
}
