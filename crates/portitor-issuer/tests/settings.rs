// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The one rule for every setting (0.13), checked for each setting of
//! `portitor-issuer` that `docs/STABILITY.md` lists: spaces around a
//! value are dropped, an empty value means "not set", and a required
//! setting that is missing stops the start with a text that names it.
//! Also: the switch (`true` or not), and the names 0.13 took away.
//!
//! The table below has one line per setting. **A new setting gets a
//! line here** (the first test fails until it has one). The checks are
//! in `portitor_runtime::testing::settings_rule`.
#![allow(clippy::unwrap_used, missing_docs)]

use portitor_issuer::config::{Config, REMOVED_SETTINGS};
use portitor_runtime::request_size::RequestLimits;
use portitor_runtime::testing::settings_rule::{
    Case, Read, SWITCH_TRUTH_TABLE, check_binary_refuses_to_start, check_removed, check_rule,
    switch_truth_table,
};
use portitor_runtime::testing::{Lookup, TempDir, frozen, vars};

/// Settings of `docs/STABILITY.md` that the table leaves out, because no
/// function reads them from a lookup: `main` or the `OpenTelemetry` code
/// reads them from the process environment
/// (`portitor_runtime::settings::text_from_env`). A start with a bad
/// `PORTITOR_DID_CACHE_SECONDS` is tested in `tests/issuer.rs`.
const READ_FROM_THE_ENVIRONMENT_ONLY: [&str; 5] = [
    "PORTITOR_DID_CACHE_SECONDS",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_SERVICE_NAME",
];

/// The settings of PostgreSQL, which the table leaves out: set together
/// with `PORTITOR_DB_FILE`, as the table sets every setting, they stop
/// the start (one database or the other). Their rule is checked by
/// [`the_database_url_follows_the_rule`] and
/// [`the_password_file_is_read_with_the_url_only`].
const POSTGRES_SETTINGS: [&str; 2] = ["PORTITOR_DATABASE_URL", "PORTITOR_DATABASE_PASSWORD_FILE"];

/// One line per setting: a value, another value, and whether the issuer
/// refuses to start without it.
fn table() -> Vec<Case> {
    vec![
        Case::optional("PORTITOR_BIND", "0.0.0.0:9000", "0.0.0.0:9003"),
        Case::required(
            "PORTITOR_PARTICIPANT_ID",
            "did:web:issuer.example",
            "did:web:other.example",
        ),
        Case::optional("PORTITOR_DB_FILE", "/data/a.db", "/data/b.db"),
        Case::optional("PORTITOR_ALLOW_HTTP", "true", "false"),
        Case::optional("PORTITOR_MANAGEMENT_BIND", "0.0.0.0:9001", "0.0.0.0:9002"),
        Case::optional("PORTITOR_MANAGEMENT_API_KEY_FILE", "/run/key", "/run/other"),
        Case::optional("PORTITOR_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::optional("PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES", "1000", "2000"),
        Case::required(
            "PORTITOR_ISSUER_SERVICE_URL",
            "https://issuer.example/issuer",
            "https://issuer.example/other",
        ),
        Case::required("PORTITOR_ISSUER_KEY_FILE", "/run/key.pem", "/run/other.pem"),
        Case::optional(
            "PORTITOR_ISSUER_RETIRED_KEY_FILES",
            "/run/old.pem",
            "/run/old.pem, /run/older.pem",
        ),
        Case::required(
            "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE",
            "/config/definitions.json",
            "/config/other.json",
        ),
        Case::optional("PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER", "3", "4"),
        Case::optional(
            "PORTITOR_ISSUER_BUNDLE_KEY_FILE",
            "/run/bundle.jwk",
            "/run/other.jwk",
        ),
        Case::optional("PORTITOR_ISSUER_BUNDLE_DIR", "/bundles", "/other"),
    ]
}

/// Everything the issuer reads from `var` at start, as one text; or why
/// it does not start.
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
    listed.append(&mut frozen::list_under(&stability, "### Settings of portitor-issuer").unwrap());
    let mut in_the_table = frozen::Names::new();
    for case in table() {
        in_the_table.insert(case.name.to_owned());
    }
    for name in READ_FROM_THE_ENVIRONMENT_ONLY
        .into_iter()
        .chain(POSTGRES_SETTINGS)
    {
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

/// The table with `PORTITOR_DATABASE_URL` in place of
/// `PORTITOR_DB_FILE`: the rule holds for the URL too.
fn table_with_the_database_url() -> Vec<Case> {
    let mut table = table();
    table.retain(|case| case.name != "PORTITOR_DB_FILE");
    table.push(Case::optional(
        "PORTITOR_DATABASE_URL",
        "postgresql://portitor@db.example/a?sslmode=disable",
        "postgresql://portitor@db.example/b?sslmode=disable",
    ));
    table
}

#[test]
fn the_database_url_follows_the_rule() {
    if let Err(failures) = check_rule(&table_with_the_database_url(), read) {
        panic!("{failures}");
    }
}

/// The password file is read at start, trimmed and empty as every
/// setting, and only with the URL. What it holds is never shown, so
/// this test reads it through whether the start succeeds.
#[test]
fn the_password_file_is_read_with_the_url_only() {
    let dir = TempDir::new("settings-password");
    let password = dir.path().join("password");
    std::fs::write(&password, "s3cret\n").unwrap();
    let password = password.to_string_lossy().into_owned();
    let pairs_with = |extra: &[(&'static str, &str)], table: &[Case]| {
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        for case in table {
            pairs.push((case.name, &case.value));
        }
        pairs.extend_from_slice(extra);
        read(&vars(&pairs))
    };
    let postgres = table_with_the_database_url();
    let spaced = format!(" {password} ");
    for value in [password.as_str(), spaced.as_str(), ""] {
        let read = pairs_with(&[("PORTITOR_DATABASE_PASSWORD_FILE", value)], &postgres);
        assert!(read.is_ok(), "{value:?}: {read:?}");
    }
    let missing = pairs_with(
        &[("PORTITOR_DATABASE_PASSWORD_FILE", "/nowhere/password")],
        &postgres,
    );
    assert!(missing.unwrap_err().contains("/nowhere/password"));
    // with the SQLite file, a password file is a mistake
    let without_url = pairs_with(&[("PORTITOR_DATABASE_PASSWORD_FILE", &password)], &table());
    assert!(
        without_url
            .unwrap_err()
            .contains("PORTITOR_DATABASE_PASSWORD_FILE is set without PORTITOR_DATABASE_URL")
    );
}

/// `PORTITOR_ALLOW_HTTP_CALLBACKS` and `PORTITOR_ISSUER_CREDENTIALS_FILE`
/// (0.13).
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
    let allow_http = |var: &Lookup| match Config::from_vars(var) {
        Ok(config) => Ok(config.allow_http),
        Err(error) => Err(error.to_string()),
    };
    assert_eq!(
        switch_truth_table("PORTITOR_ALLOW_HTTP", &table(), allow_http),
        SWITCH_TRUTH_TABLE
    );
}

/// The same through the real binary, which reads the process
/// environment: it stops without settings, and with an old name.
#[test]
fn the_binary_does_not_start_without_settings_or_with_a_removed_name() {
    let binary = env!("CARGO_BIN_EXE_portitor-issuer");
    let checked =
        check_binary_refuses_to_start(binary, "PORTITOR_PARTICIPANT_ID", REMOVED_SETTINGS);
    if let Err(failures) = checked {
        panic!("{failures}");
    }
}
