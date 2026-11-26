// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The one rule for every setting (0.13), checked for each setting of
//! `portitor-connector` that `docs/STABILITY.md` lists: spaces around a
//! value are dropped, an empty value means "not set", and a required
//! setting that is missing stops the start with a text that names it.
//! Also: the switches (`true` or not), and the names 0.13 took away.
//!
//! The table below has one line per setting. **A new setting gets a
//! line here** (the first test fails until it has one). The checks are
//! in `portitor_runtime::testing::settings_rule`.
#![allow(clippy::unwrap_used, missing_docs)]

use portitor_bundle::source::BundleConfig;
use portitor_connector::auth::AuthSettings;
use portitor_connector::management::{DEFAULT_MANAGEMENT_BIND, ManagementConfig};
use portitor_connector::{Config, REMOVED_SETTINGS};
use portitor_dataplane::object_storage;
use portitor_runtime::request_size::RequestLimits;
use portitor_runtime::testing::settings_rule::{
    Case, Read, SWITCH_TRUTH_TABLE, check_binary_refuses_to_start, check_removed, check_rule,
    switch_truth_table,
};
use portitor_runtime::testing::{Lookup, TempDir, frozen};

/// Settings of `docs/STABILITY.md` that the table leaves out, because no
/// function reads them from a lookup: `main` or the `OpenTelemetry` code
/// reads them from the process environment
/// (`portitor_runtime::settings::text_from_env`).
const READ_FROM_THE_ENVIRONMENT_ONLY: [&str; 5] = [
    "PORTITOR_BIND",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_SERVICE_NAME",
];

const MEMBERSHIP_SCOPE: &str = "org.eclipse.dspace.dcp.vc.type:MembershipCredential:read";
const RESEARCH_SCOPE: &str = "org.eclipse.dspace.dcp.vc.type:ResearchCredential:read";

/// Where the bundle comes from in a table: the two settings exclude
/// each other, so each has its own table.
#[derive(Clone, Copy)]
enum BundleFrom {
    Url,
    Folder,
}

/// One line per setting: a value, another value, and whether the
/// connector refuses to start without it while all the others are set.
/// `secrets` holds the two STS secret files (they are read at start).
fn table(secrets: &TempDir, bundle_from: BundleFrom) -> Vec<Case> {
    let secret = secrets.write("sts-secret", "test-only-secret\n");
    let other_secret = secrets.write("other-sts-secret", "test-only-other-secret\n");
    let two_scopes = format!("{MEMBERSHIP_SCOPE}, {RESEARCH_SCOPE}");
    let bundle = match bundle_from {
        // "required": with the other bundle settings set
        BundleFrom::Url => Case::required(
            "PORTITOR_BUNDLE_URL",
            "https://authority.example/bundles",
            "https://authority.example/other",
        ),
        BundleFrom::Folder => Case::required("PORTITOR_BUNDLE_DIR", "/bundles", "/other"),
    };
    vec![
        Case::required(
            "PORTITOR_PARTICIPANT_ID",
            "did:web:connector.example",
            "did:web:other.example",
        ),
        Case::required(
            "PORTITOR_PUBLIC_URL",
            "https://connector.example",
            "https://other.example",
        ),
        Case::optional("PORTITOR_DB_FILE", "/data/a.db", "/data/b.db"),
        Case::optional("PORTITOR_ALLOW_HTTP", "true", "false"),
        Case::optional("PORTITOR_MANAGEMENT_BIND", "0.0.0.0:9001", "0.0.0.0:9002"),
        Case::optional("PORTITOR_MANAGEMENT_API_KEY_FILE", "/run/key", "/run/other"),
        Case::optional("PORTITOR_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::optional("PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::optional("PORTITOR_DID_CACHE_SECONDS", "10", "20"),
        Case::optional("PORTITOR_DATASETS_FILE", "/config/a.json", "/config/b.json"),
        Case::optional("PORTITOR_HARVEST_INTERVAL_SECONDS", "60", "120"),
        Case::optional("PORTITOR_DATA_TOKEN_SECONDS", "60", "120"),
        // optional for these functions; `main` refuses to start without
        // it (a release build has no other way to authenticate)
        Case::optional(
            "PORTITOR_STS_URL",
            "https://identity.example/sts/token",
            "https://other.example/sts/token",
        ),
        Case::optional("PORTITOR_STS_ALLOW_HTTP", "true", "false"),
        Case::optional("PORTITOR_STS_CLIENT_ID", "connector", "other"),
        Case::required(
            "PORTITOR_STS_CLIENT_SECRET_FILE",
            secret.to_str().unwrap(),
            other_secret.to_str().unwrap(),
        ),
        Case::required(
            "PORTITOR_TRUSTED_ISSUERS",
            "did:web:issuer.example",
            "did:web:issuer.example, did:web:other.example",
        ),
        Case::optional("PORTITOR_DCP_SCOPES", MEMBERSHIP_SCOPE, &two_scopes),
        Case::optional(
            "PORTITOR_POLICY_CREDENTIALS",
            "ResearchCredential",
            "ResearchCredential, OtherCredential",
        ),
        Case::optional("PORTITOR_STATUS_LIST_CACHE_SECONDS", "10", "20"),
        Case::optional(
            "PORTITOR_GAIAX_COMPLIANCE_ISSUERS",
            "did:web:compliance.example",
            "did:web:other.example",
        ),
        bundle,
        Case::optional("PORTITOR_BUNDLE_VERSION", "2026.11.0", "2026.12.0"),
        Case::optional("PORTITOR_BUNDLE_REFRESH_SECONDS", "60", "120"),
        Case::required(
            "PORTITOR_BUNDLE_AUTHORITY",
            "did:web:authority.example",
            "did:web:other.example",
        ),
        Case::optional("PORTITOR_BUNDLE_CACHE_DIR", "/cache/a", "/cache/b"),
        // object storage (ADR 0027): both keys, or neither
        Case::optional(
            "AWS_ENDPOINT_URL_S3",
            "https://s3.example",
            "https://other.example",
        ),
        Case::optional("AWS_REGION", "eu-central-1", "eu-west-1"),
        Case::required(
            "AWS_ACCESS_KEY_ID",
            "test-only-key-id",
            "test-only-other-id",
        ),
        Case::required(
            "AWS_SECRET_ACCESS_KEY",
            "test-only-secret-key",
            "test-only-other-key",
        ),
        Case::optional(
            "AWS_SESSION_TOKEN",
            "test-only-token",
            "test-only-other-token",
        ),
        Case::optional("PORTITOR_PUSH_BUCKET", "pushed", "other"),
    ]
}

/// Everything the connector reads from `var` at start, in the order of
/// `main`, as one text; or why it does not start.
fn read(var: &Lookup) -> Read {
    let config = Config::from_vars(var).map_err(|error| error.to_string())?;
    let auth = AuthSettings::from_vars(&config.participant_id, config.allow_http, var)?;
    // the `Debug` text of the settings leaves the secret out
    let secret = auth.as_ref().map(|auth| auth.sts_client_secret.clone());
    let bundle = BundleConfig::from_vars(var, &config.db_file, config.allow_http)
        .map_err(|error| error.to_string())?;
    let management = ManagementConfig::from_vars(DEFAULT_MANAGEMENT_BIND, var);
    let limits = RequestLimits::from_vars(var)?;
    let storage = object_storage::Settings::from_vars(var)?;
    // the `Debug` text leaves the keys out
    let keys = storage.as_ref().map(|s| {
        (
            s.access_key_id.clone(),
            s.secret_access_key.clone(),
            s.session_token.clone(),
        )
    });
    Ok(format!(
        "{config:?}\n{auth:?}\n{secret:?}\n{bundle:?}\n{management:?}\n{limits:?}\n{storage:?}\n{keys:?}"
    ))
}

#[test]
fn the_table_has_every_setting_of_the_stability_document() {
    let stability =
        frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/STABILITY.md").unwrap();
    let mut listed = frozen::list_under(&stability, "### Settings of every binary").unwrap();
    listed
        .append(&mut frozen::list_under(&stability, "### Settings of portitor-connector").unwrap());
    let secrets = TempDir::new("settings");
    let mut in_the_table = frozen::Names::new();
    for bundle_from in [BundleFrom::Url, BundleFrom::Folder] {
        for case in table(&secrets, bundle_from) {
            in_the_table.insert(case.name.to_owned());
        }
    }
    for name in READ_FROM_THE_ENVIRONMENT_ONLY {
        assert!(
            in_the_table.insert(name.to_owned()),
            "{name} is in both lists"
        );
    }
    assert_eq!(in_the_table, listed);
}

#[test]
fn every_setting_is_trimmed_and_an_empty_one_is_not_set() {
    let secrets = TempDir::new("settings");
    for bundle_from in [BundleFrom::Url, BundleFrom::Folder] {
        if let Err(failures) = check_rule(&table(&secrets, bundle_from), read) {
            panic!("{failures}");
        }
    }
}

/// `PORTITOR_DATASETS` (0.2) and `PORTITOR_ALLOW_HTTP_CALLBACKS` (0.13).
#[test]
fn a_removed_name_stops_the_start_unless_it_is_empty() {
    let secrets = TempDir::new("settings");
    let table = table(&secrets, BundleFrom::Url);
    if let Err(failures) = check_removed(REMOVED_SETTINGS, &table, read) {
        panic!("{failures}");
    }
}

/// `PORTITOR_ALLOW_HTTP` and `PORTITOR_STS_ALLOW_HTTP` are on for `true`
/// in any letter case and off for everything else, also for `1` and
/// `yes`, without an error.
#[test]
fn the_switches_are_on_only_for_true() {
    let secrets = TempDir::new("settings");
    let table = table(&secrets, BundleFrom::Url);
    let allow_http = |var: &Lookup| match Config::from_vars(var) {
        Ok(config) => Ok(config.allow_http),
        Err(error) => Err(error.to_string()),
    };
    assert_eq!(
        switch_truth_table("PORTITOR_ALLOW_HTTP", &table, allow_http),
        SWITCH_TRUTH_TABLE
    );
    let sts_allow_http = |var: &Lookup| {
        let auth = AuthSettings::from_vars("did:web:connector.example", false, var)?;
        Ok(auth.unwrap().sts_allow_http)
    };
    assert_eq!(
        switch_truth_table("PORTITOR_STS_ALLOW_HTTP", &table, sts_allow_http),
        SWITCH_TRUTH_TABLE
    );
}

/// The same through the real binary, which reads the process
/// environment: it stops without settings, and with an old name.
#[test]
fn the_binary_does_not_start_without_settings_or_with_a_removed_name() {
    let binary = env!("CARGO_BIN_EXE_portitor-connector");
    let checked =
        check_binary_refuses_to_start(binary, "PORTITOR_PARTICIPANT_ID", REMOVED_SETTINGS);
    if let Err(failures) = checked {
        panic!("{failures}");
    }
}
