// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The database layer on a real PostgreSQL server
//! (`PORTITOR_TEST_DATABASE_URL`; without it these tests do nothing,
//! unless `PORTITOR_TEST_REQUIRE_POSTGRES` is `true`, as in CI).
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::Arc;

use portitor_runtime::database::{Database, DatabaseSetting, DbError, Executor, Migrations};
use portitor_runtime::testing::databases;
use portitor_runtime::values;

const COUNTER: Migrations = Migrations {
    component: "portitor-test",
    sqlite: &["CREATE TABLE counter (id TEXT PRIMARY KEY, n INTEGER NOT NULL)"],
    postgres: &[
        "CREATE TABLE counter (id TEXT PRIMARY KEY, n BIGINT NOT NULL)",
        "CREATE TABLE note (text TEXT, data BYTEA, flag BOOLEAN)",
    ],
};

fn open(
    settings: &portitor_runtime::database::postgres::Settings,
    m: &Migrations,
) -> Result<Database, DbError> {
    Database::open(&DatabaseSetting::Postgres(Box::new(settings.clone())), m)
}

#[test]
fn values_go_in_and_come_back_unchanged() {
    let Some(db) = databases::fresh_postgres(&COUNTER) else {
        return;
    };
    (&db)
        .execute(
            "INSERT INTO note (text, data, flag) VALUES ($1, $2, $3)",
            values!["é ✓", vec![0_u8, 255], true],
        )
        .unwrap();
    (&db)
        .execute(
            "INSERT INTO note (text, data, flag) VALUES ($1, $2, $3)",
            values![None::<String>, None::<Vec<u8>>, None::<bool>],
        )
        .unwrap();
    let rows = (&db)
        .query(
            "SELECT text, data, flag FROM note ORDER BY text NULLS LAST",
            values![],
        )
        .unwrap();
    assert_eq!(rows[0].get::<String>(0).unwrap(), "é ✓");
    assert_eq!(rows[0].get::<Vec<u8>>(1).unwrap(), vec![0, 255]);
    assert!(rows[0].get::<bool>(2).unwrap());
    assert_eq!(rows[1].get::<Option<String>>(0).unwrap(), None);
    assert_eq!(rows[1].get::<Option<bool>>(2).unwrap(), None);
}

/// Two threads add 1 to the same counter 50 times each, every addition
/// a read then a write in one transaction: with SERIALIZABLE and the
/// tries again, no addition is lost.
#[test]
fn colliding_transactions_are_tried_again_and_lose_nothing() {
    let Some(url) = databases::postgres_url() else {
        return;
    };
    let settings = databases::fresh_postgres_settings(&url);
    // two pools: two "replicas" of one component
    let a = Arc::new(open(&settings, &COUNTER).unwrap());
    let b = Arc::new(open(&settings, &COUNTER).unwrap());
    (&*a)
        .execute("INSERT INTO counter (id, n) VALUES ('c', 0)", values![])
        .unwrap();
    let add = |db: Arc<Database>| {
        std::thread::spawn(move || {
            for _ in 0..50 {
                db.transaction(|tx| {
                    let n: i64 = tx
                        .query_one("SELECT n FROM counter WHERE id = 'c'", values![])?
                        .unwrap()
                        .get(0)?;
                    tx.execute("UPDATE counter SET n = $1 WHERE id = 'c'", values![n + 1])?;
                    Ok(())
                })
                .unwrap();
            }
        })
    };
    let (ta, tb) = (add(Arc::clone(&a)), add(Arc::clone(&b)));
    ta.join().unwrap();
    tb.join().unwrap();
    let n: i64 = (&*a)
        .query_one("SELECT n FROM counter", values![])
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert_eq!(n, 100);
}

#[test]
fn replicas_that_start_together_migrate_once() {
    let Some(url) = databases::postgres_url() else {
        return;
    };
    let settings = Arc::new(databases::fresh_postgres_settings(&url));
    let starts: Vec<_> = (0..4)
        .map(|_| {
            let settings = Arc::clone(&settings);
            std::thread::spawn(move || open(&settings, &COUNTER).map(|_| ()))
        })
        .collect();
    for start in starts {
        start.join().unwrap().unwrap();
    }
    let db = open(&settings, &COUNTER).unwrap();
    let version: i64 = (&db)
        .query_one("SELECT version FROM portitor_schema", values![])
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn another_component_and_a_newer_schema_are_refused() {
    let Some(url) = databases::postgres_url() else {
        return;
    };
    let settings = databases::fresh_postgres_settings(&url);
    drop(open(&settings, &COUNTER).unwrap());
    let other = Migrations {
        component: "portitor-other",
        ..COUNTER
    };
    let err = open(&settings, &other).unwrap_err();
    assert!(err.message.contains("schema of portitor-test"), "{err}");
    let older = Migrations {
        postgres: &COUNTER.postgres[..1],
        ..COUNTER
    };
    let err = open(&settings, &older).unwrap_err();
    assert!(err.message.contains("newer"), "{err}");
}

#[test]
fn a_failed_transaction_keeps_nothing_and_the_error_has_its_code() {
    let Some(db) = databases::fresh_postgres(&COUNTER) else {
        return;
    };
    let result: Result<(), DbError> = db.transaction(|tx| {
        tx.execute("INSERT INTO counter (id, n) VALUES ('x', 1)", values![])?;
        tx.execute("INSERT INTO counter (id, n) VALUES ('x', 2)", values![])?;
        Ok(())
    });
    assert_eq!(result.unwrap_err().code.as_deref(), Some("23505"));
    assert!(
        (&db)
            .query("SELECT id FROM counter", values![])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn every_test_database_runs_the_same_sql() {
    for db in databases::all(&COUNTER) {
        (&db)
            .execute(
                "INSERT INTO counter (id, n) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
                values!["a", 1_i64],
            )
            .unwrap();
        let changed = (&db)
            .execute(
                "INSERT INTO counter (id, n) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
                values!["a", 2_i64],
            )
            .unwrap();
        assert_eq!(changed, 0, "{db:?}");
        let rows = (&db).query(
            "SELECT id, n FROM counter WHERE id = $1 RETURNING",
            values!["a"],
        );
        assert!(rows.is_err(), "bad SQL is an error, not a panic");
    }
}

/// Store calls come from request handlers, on tokio's threads; waiting
/// for the database there must work, also on a one-thread runtime.
#[tokio::test(flavor = "current_thread")]
async fn calls_from_inside_a_tokio_runtime_work() {
    let Some(db) = databases::fresh_postgres(&COUNTER) else {
        return;
    };
    (&db)
        .execute("INSERT INTO counter (id, n) VALUES ('t', 1)", values![])
        .unwrap();
    let n: i64 = (&db)
        .query_one("SELECT n FROM counter", values![])
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert_eq!(n, 1);
}

/// With `PORTITOR_TEST_DATABASE_TLS_URL` (a server with TLS, its
/// certificate from a test CA for `localhost`, that CA in `sslrootcert`;
/// CI sets it), the connection is encrypted and the certificate checked:
/// a wrong host name is refused, and so are the system's CAs
/// (`sslrootcert=system`), which do not hold the test CA.
#[test]
fn tls_connections_check_the_server() {
    let Some(url) = portitor_runtime::settings::text_from_env("PORTITOR_TEST_DATABASE_TLS_URL")
    else {
        return;
    };
    let settings = databases::fresh_postgres_settings(&url);
    let db = open(&settings, &COUNTER).unwrap();
    let ssl: bool = (&db)
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            values![],
        )
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert!(ssl, "the connection is encrypted");
    let wrong_host = url.replacen("localhost", "127.0.0.1", 1);
    let settings =
        portitor_runtime::database::postgres::Settings::from_url(&wrong_host, None).unwrap();
    let err = open(&settings, &COUNTER).unwrap_err();
    assert!(err.message.contains("cannot connect"), "{err}");
    let (base, _) = url.split_once('?').unwrap();
    let system = format!("{base}?sslrootcert=system");
    let settings = portitor_runtime::database::postgres::Settings::from_url(&system, None).unwrap();
    let err = open(&settings, &COUNTER).unwrap_err();
    assert!(err.message.contains("certificate"), "{err}");
}

/// A statement may run 30 seconds unless the URL says otherwise.
#[test]
fn statements_have_a_time_limit() {
    let Some(db) = databases::fresh_postgres(&COUNTER) else {
        return;
    };
    let limit: String = (&db)
        .query_one("SHOW statement_timeout", values![])
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert_eq!(limit, "30s");
    let err = db
        .transaction(|tx| {
            tx.execute("SET LOCAL statement_timeout = 50", values![])?;
            tx.query("SELECT pg_sleep(1)", values![])
        })
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("57014"), "{err}");
}

/// A test database is dropped from the server once its settings and the
/// databases opened with them are gone, so test runs leave none behind.
#[test]
fn a_test_database_is_dropped_after_its_test() {
    let Some(url) = databases::postgres_url() else {
        return;
    };
    let settings = databases::fresh_postgres_settings(&url);
    let db = open(&settings, &COUNTER).unwrap();
    let name: String = (&db)
        .query_one("SELECT current_database()::text", values![])
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap();
    assert!(name.starts_with("portitor_test_"), "{name}");
    drop(settings);
    // still open: the database stays
    (&db).query("SELECT 1", values![]).unwrap();
    drop(db);
    let password = portitor_runtime::settings::text_from_env("PORTITOR_TEST_DATABASE_PASSWORD");
    let admin = portitor_runtime::database::postgres::Settings::from_url(&url, password).unwrap();
    let left = portitor_runtime::database::postgres::database_exists(&admin, &name).unwrap();
    assert!(!left, "{name} is still there");
}
