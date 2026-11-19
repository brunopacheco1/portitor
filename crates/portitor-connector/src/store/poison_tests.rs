// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held (TD-41: release builds unwind
//! from 0.12, so the lock recovery in `conn()` now runs in production).
//! The lock is "poisoned" afterwards; `conn()` takes it all the same,
//! which is safe because `SQLite` keeps the database consistent:
//! negotiations, transfers and data flows are written with one statement
//! each (complete or not there), and a harvest run in one transaction
//! (rolled back when the panic drops it).
//!
//! The tests write with plain SQL on a table of their own, so they do
//! not depend on the store's schema.

use std::sync::Arc;

use super::*;

/// How many tables are named `name`.
fn tables_named(store: &SqliteNegotiations, name: &str) -> i64 {
    let sql = "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1";
    store
        .conn()
        .query_row(sql, [name], |row| row.get(0))
        .unwrap()
}

/// The store works: no transaction is left open, and a write is kept.
fn assert_usable(store: &SqliteNegotiations) {
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
    let store = Arc::new(SqliteNegotiations::in_memory().unwrap());
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
    let store = Arc::new(SqliteNegotiations::in_memory().unwrap());
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

/// The store's own operations work on the poisoned lock too: a transfer
/// request is written once, found again, and its repeat adds nothing.
#[test]
fn a_transfer_is_stored_and_found_after_a_panic() {
    use portitor_dsp::transfer::TransferState;
    use portitor_dsp::transfer::provider::{Transfer, TransferStore};

    let store = Arc::new(SqliteNegotiations::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let _conn = held.conn();
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert!(store.0.is_poisoned());

    let transfer = Transfer {
        provider_pid: "urn:uuid:p".to_owned(),
        consumer_pid: "urn:uuid:c".to_owned(),
        state: TransferState::Requested,
        callback_address: "https://c.example/cb".to_owned(),
        counter_party: "did:web:c".to_owned(),
        agreement_id: "urn:uuid:a".to_owned(),
        dataset_id: "d".to_owned(),
        format: "HttpData-PULL".to_owned(),
        pending: None,
        version: 0,
    };
    let transfers: &dyn TransferStore = store.as_ref();
    assert!(transfers.insert(&transfer).unwrap());
    assert_eq!(transfers.get("urn:uuid:p").unwrap(), Some(transfer.clone()));
    let found = transfers.requested("did:web:c", "urn:uuid:c").unwrap();
    assert_eq!(found, Some(transfer.clone()));
    let mut repeat = transfer;
    repeat.provider_pid = "urn:uuid:p2".to_owned();
    assert!(!transfers.insert(&repeat).unwrap());
}
