// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The one rule for every setting (0.13), checked for each setting of
//! `portitor-identity` that `docs/STABILITY.md` lists: spaces around a
//! value are dropped, an empty value means "not set", and a required
//! setting that is missing stops the start with a text that names it.
//! Also: the switch (`true` or not), and the names 0.13 took away.
//!
//! The table below has one line per setting. **A new setting gets a
//! line here** (the first test fails until it has one). The checks are
//! in `portitor_runtime::testing::settings_rule`.
#![allow(clippy::unwrap_used, missing_docs)]

use portitor_identity::config::{Config, REMOVED_SETTINGS};
use portitor_runtime::request_size::RequestLimits;
use portitor_runtime::testing::settings_rule::{
    Case, Read, SWITCH_TRUTH_TABLE, check_binary_refuses_to_start, check_removed, check_rule,
    switch_truth_table,
};
use portitor_runtime::testing::{Lookup, frozen};

/// Settings of `docs/STABILITY.md` that the table leaves out, because no
/// function reads them from a lookup: `main` or the `OpenTelemetry` code
/// reads them from the process environment
/// (`portitor_runtime::settings::text_from_env`). A start with a bad
/// `PORTITOR_DID_CACHE_SECONDS` is tested in `tests/health.rs`.
const READ_FROM_THE_ENVIRONMENT_ONLY: [&str; 5] = [
    "PORTITOR_DID_CACHE_SECONDS",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_SERVICE_NAME",
];

/// One line per setting: a value, another value, and whether the
/// identity refuses to start without it while all the others are set
/// (so with Gaia-X onboarding configured).
fn table() -> Vec<Case> {
    vec![
        Case::optional("PORTITOR_BIND", "0.0.0.0:9000", "0.0.0.0:9003"),
        Case::required(
            "PORTITOR_PARTICIPANT_ID",
            "did:web:identity.example",
            "did:web:other.example",
        ),
        Case::optional("PORTITOR_DB_FILE", "/data/a.db", "/data/b.db"),
        Case::optional("PORTITOR_ALLOW_HTTP", "true", "false"),
        Case::optional("PORTITOR_MANAGEMENT_BIND", "0.0.0.0:9001", "0.0.0.0:9002"),
        Case::optional("PORTITOR_MANAGEMENT_API_KEY_FILE", "/run/key", "/run/other"),
        Case::optional("PORTITOR_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::optional("PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::required(
            "PORTITOR_CREDENTIAL_SERVICE_URL",
            "https://identity.example/cs",
            "https://identity.example/other",
        ),
        Case::required(
            "PORTITOR_IDENTITY_KEY_FILE",
            "/run/key.pem",
            "/run/other.pem",
        ),
        Case::optional(
            "PORTITOR_IDENTITY_RETIRED_KEY_FILES",
            "/run/old.pem",
            "/run/old.pem, /run/older.pem",
        ),
        // required with PORTITOR_GAIAX_COMPLIANCE_URL, as the next two
        Case::required(
            "PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE",
            "/run/chain.pem",
            "/run/other-chain.pem",
        ),
        Case::optional("PORTITOR_STS_CLIENT_ID", "connector", "other"),
        Case::required(
            "PORTITOR_STS_CLIENT_SECRET_FILE",
            "/run/secret",
            "/run/other-secret",
        ),
        Case::required(
            "PORTITOR_TRUSTED_ISSUERS",
            "did:web:issuer.example",
            "did:web:issuer.example, did:web:other.example",
        ),
        Case::optional(
            "PORTITOR_SCOPE_TYPE_ALIASES",
            "example.vc.type",
            "other.vc.type",
        ),
        Case::optional(
            "PORTITOR_GAIAX_COMPLIANCE_URL",
            "https://compliance.example/v2",
            "https://other.example/v2",
        ),
        Case::required(
            "PORTITOR_GAIAX_COMPLIANCE_DID",
            "did:web:compliance.example:v2",
            "did:web:other.example:v2",
        ),
        Case::required(
            "PORTITOR_GAIAX_PARTICIPANT_FILE",
            "/config/participant.json",
            "/config/other.json",
        ),
    ]
}

/// Everything the identity reads from `var` at start, as one text; or
/// why it does not start.
fn read(var: &Lookup) -> Read {
    let config = Config::from_vars(var).map_err(|error| error.to_string())?;
    let limits = RequestLimits::from_vars(var)?;
    Ok(format!("{config:?}\n{limits:?}"))
}

#[test]
fn the_table_has_every_setting_of_the_stability_document() {
    let stability =
        frozen::repository_file(env!("CARGO_MANIFEST_DIR"), "docs/STABILITY.md").unwrap();
    let mut listed = frozen::list_under(&stability, "### Settings of every binary").unwrap();
    listed
        .append(&mut frozen::list_under(&stability, "### Settings of portitor-identity").unwrap());
    let mut in_the_table = frozen::Names::new();
    for case in table() {
        in_the_table.insert(case.name.to_owned());
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
    if let Err(failures) = check_rule(&table(), read) {
        panic!("{failures}");
    }
}

/// `PORTITOR_ALLOW_HTTP_CALLBACKS` and
/// `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` (0.13).
#[test]
fn a_removed_name_stops_the_start_unless_it_is_empty() {
    if let Err(failures) = check_removed(REMOVED_SETTINGS, &table(), read) {
        panic!("{failures}");
    }
}

/// `PORTITOR_ALLOW_HTTP` is on for `true` in any letter case and off for
/// everything else, also for `1` and `yes`, without an error.
#[test]
fn the_switch_is_on_only_for_true() {
    // without Gaia-X: its URL is checked against this switch
    let mut table = table();
    table.retain(|case| !case.name.starts_with("PORTITOR_GAIAX_"));
    let allow_http = |var: &Lookup| match Config::from_vars(var) {
        Ok(config) => Ok(config.allow_http),
        Err(error) => Err(error.to_string()),
    };
    assert_eq!(
        switch_truth_table("PORTITOR_ALLOW_HTTP", &table, allow_http),
        SWITCH_TRUTH_TABLE
    );
}

/// The same through the real binary, which reads the process
/// environment: it stops without settings, and with an old name.
#[test]
fn the_binary_does_not_start_without_settings_or_with_a_removed_name() {
    let binary = env!("CARGO_BIN_EXE_portitor-identity");
    let checked =
        check_binary_refuses_to_start(binary, "PORTITOR_PARTICIPANT_ID", REMOVED_SETTINGS);
    if let Err(failures) = checked {
        panic!("{failures}");
    }
}
