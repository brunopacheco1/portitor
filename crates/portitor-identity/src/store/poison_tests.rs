// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held (TD-41: release builds unwind
//! from 0.12, so the lock recovery in `conn()` now runs in production).
//! The lock is "poisoned" afterwards; `conn()` takes it all the same,
//! which is safe because `SQLite` keeps the database consistent:
//! every write of more than one statement is one transaction, rolled back
//! when the panic drops it.
//!
//! The tests write with plain SQL on a table of their own, so they do
//! not depend on the store's schema.

use std::sync::Arc;

use super::*;

/// How many tables are named `name`.
fn tables_named(store: &SqliteStore, name: &str) -> i64 {
    let sql = "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1";
    store
        .conn()
        .query_row(sql, [name], |row| row.get(0))
        .unwrap()
}

/// The store works: no transaction is left open, and a write is kept.
fn assert_usable(store: &SqliteStore) {
    assert!(store.0.is_poisoned(), "the panic poisoned the lock");
    assert!(store.conn().is_autocommit(), "no transaction is left open");
    store
        .conn()
        .execute_batch("CREATE TABLE after_panic (x TEXT)")
        .unwrap();
    assert_eq!(tables_named(store, "after_panic"), 1);
}

/// A write of several statements is a `rusqlite` transaction, which rolls
/// back when it is dropped without `commit`, also by a panic.
#[test]
fn a_panic_inside_a_transaction_rolls_it_back() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    // the panic happens on another thread, as it would in a request
    let panicked = std::thread::spawn(move || {
        let mut conn = held.conn();
        let tx = conn.transaction().unwrap();
        tx.execute_batch("CREATE TABLE half_done (x TEXT)").unwrap();
        panic!("while the lock is held and a transaction is open");
    })
    .join();
    assert!(panicked.is_err());
    assert_eq!(tables_named(&store, "half_done"), 0, "rolled back");
    assert_usable(&store);
}

/// A write of one statement is complete before anything can panic: it
/// is kept, and the store works.
#[test]
fn a_panic_after_a_single_statement_keeps_it() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let conn = held.conn();
        conn.execute_batch("CREATE TABLE written (x TEXT)").unwrap();
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert_eq!(tables_named(&store, "written"), 1, "kept");
    assert_usable(&store);
}

/// The store's own operations work on the poisoned lock too: a Gaia-X
/// run (a transaction of two statements) is recorded and listed.
#[test]
fn a_gaiax_run_is_recorded_and_listed_after_a_panic() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let _conn = held.conn();
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert!(store.0.is_poisoned());

    let runs: &dyn IdentityStore = store.as_ref();
    let id = runs.start_gaiax_run(1_794_484_800).unwrap();
    runs.finish_gaiax_run(id, 1_794_484_801, RunOutcome::Succeeded, None)
        .unwrap();
    let listed = runs.gaiax_runs(10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].outcome, RunOutcome::Succeeded);
}
