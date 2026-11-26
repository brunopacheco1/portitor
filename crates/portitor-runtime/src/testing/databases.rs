// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The databases a store's tests run on: `SQLite` in memory, always, and
//! a new PostgreSQL database when `PORTITOR_TEST_DATABASE_URL` names a
//! server (CI does; on a laptop, see TESTING, "PostgreSQL").
//!
//! With `PORTITOR_TEST_REQUIRE_POSTGRES=true` (set in CI) a test fails
//! when the URL is missing, so the PostgreSQL half cannot be skipped
//! without anyone noticing.

use crate::database::postgres::Settings;
use crate::database::{Database, DatabaseSetting, Migrations};
use crate::system::new_uuid;

/// The test server, from `PORTITOR_TEST_DATABASE_URL`: a URL whose user
/// may create databases. Its password, if any, is in the URL's
/// `PORTITOR_TEST_DATABASE_PASSWORD`.
///
/// # Panics
/// `PORTITOR_TEST_REQUIRE_POSTGRES` is `true` and the URL is not set.
#[must_use]
pub fn postgres_url() -> Option<String> {
    let url = crate::settings::text_from_env("PORTITOR_TEST_DATABASE_URL");
    let required = crate::settings::text_from_env("PORTITOR_TEST_REQUIRE_POSTGRES")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    assert!(
        url.is_some() || !required,
        "PORTITOR_TEST_REQUIRE_POSTGRES is true but PORTITOR_TEST_DATABASE_URL is not set"
    );
    url
}

/// A new, empty PostgreSQL database on the test server, with
/// `migrations` applied, or `None` when there is no test server. Each
/// call makes its own database, so tests do not see each other's rows;
/// it is dropped from the server with the returned [`Database`].
///
/// # Panics
/// The test server cannot be reached or refuses to create the database.
#[must_use]
pub fn fresh_postgres(migrations: &Migrations) -> Option<Database> {
    let url = postgres_url()?;
    let settings = fresh_postgres_settings(&url);
    match Database::open(&DatabaseSetting::Postgres(Box::new(settings)), migrations) {
        Ok(db) => Some(db),
        Err(e) => panic!("cannot open the test database: {e}"),
    }
}

/// Makes a new, empty database on the test server and returns its
/// settings; for tests that open it more than once (two "replicas").
/// The database is dropped when the last copy of the settings, and the
/// last database opened with them, is dropped.
///
/// # Panics
/// The test server cannot be reached or refuses to create the database.
#[must_use]
pub fn fresh_postgres_settings(url: &str) -> Settings {
    let password = crate::settings::text_from_env("PORTITOR_TEST_DATABASE_PASSWORD");
    let admin = settings(url, password.clone());
    let name = format!(
        "portitor_test_{}",
        new_uuid().trim_start_matches("urn:uuid:").replace('-', "")
    );
    let created = crate::database::postgres::create_database(&admin, &name);
    if let Err(e) = created {
        panic!("cannot create the test database {name}: {e}");
    }
    settings(&with_database(url, &name), password).dropping_the_database(&admin, &name)
}

fn settings(url: &str, password: Option<String>) -> Settings {
    match Settings::from_url(url, password) {
        Ok(s) => s,
        Err(e) => panic!("PORTITOR_TEST_DATABASE_URL: {e}"),
    }
}

/// `url` with its database name replaced by `name`.
fn with_database(url: &str, name: &str) -> String {
    let (base, query) = url
        .split_once('?')
        .map_or((url, None), |(b, q)| (b, Some(q)));
    let after_scheme = base.find("://").map_or(0, |i| i + 3);
    let host_end = base[after_scheme..]
        .find('/')
        .map_or(base.len(), |i| after_scheme + i);
    let mut out = format!("{}/{name}", &base[..host_end]);
    if let Some(query) = query {
        out.push('?');
        out.push_str(query);
    }
    out
}

/// Every database the store tests run on: `SQLite` in memory, then
/// PostgreSQL when there is a test server.
///
/// # Panics
/// A database cannot be opened.
#[must_use]
pub fn all(migrations: &Migrations) -> Vec<Database> {
    let mut out = Vec::new();
    match Database::in_memory(migrations) {
        Ok(db) => out.push(db),
        Err(e) => panic!("in-memory SQLite: {e}"),
    }
    out.extend(fresh_postgres(migrations));
    out
}

#[cfg(test)]
mod tests {
    use super::with_database;

    #[test]
    fn the_database_name_is_replaced_and_the_query_kept() {
        assert_eq!(
            with_database("postgresql://u@h:5432/postgres?sslmode=disable", "t1"),
            "postgresql://u@h:5432/t1?sslmode=disable"
        );
        assert_eq!(
            with_database("postgresql://u@h", "t1"),
            "postgresql://u@h/t1"
        );
    }
}
