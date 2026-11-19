// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery in `conn()`
//! takes the poisoned lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! The identity store writes more than one statement only inside a
//! transaction.

use std::sync::Arc;

use portitor_runtime::sqlite::testing;

use super::{IdentityStore, RunOutcome, SqliteStore};

/// How the shared checks reach this store: its own `conn()` (the code
/// under test) and its lock.
const ACCESS: testing::Access<SqliteStore> = testing::Access {
    conn: SqliteStore::conn,
    is_poisoned: |store| store.0.is_poisoned(),
};

#[test]
fn a_panic_inside_a_transaction_rolls_it_back() {
    let store = SqliteStore::in_memory().unwrap();
    testing::assert_a_panic_inside_a_transaction_rolls_it_back(store, &ACCESS);
}

#[test]
fn a_panic_after_a_single_statement_keeps_it() {
    let store = SqliteStore::in_memory().unwrap();
    testing::assert_a_panic_after_a_single_statement_keeps_it(store, &ACCESS);
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
