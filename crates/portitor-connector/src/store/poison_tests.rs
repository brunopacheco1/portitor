// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery in `conn()`
//! takes the poisoned lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! Negotiations, transfers and data flows are written with one statement
//! each, a harvest run in one transaction: both cases are checked.

use std::sync::Arc;

use portitor_runtime::sqlite::testing;

use super::SqliteNegotiations;

/// How the shared checks reach this store: its own `conn()` (the code
/// under test) and its lock.
const ACCESS: testing::Access<SqliteNegotiations> = testing::Access {
    conn: SqliteNegotiations::conn,
    is_poisoned: |store| store.0.is_poisoned(),
};

#[test]
fn a_panic_inside_a_transaction_rolls_it_back() {
    let store = SqliteNegotiations::in_memory().unwrap();
    testing::assert_a_panic_inside_a_transaction_rolls_it_back(store, &ACCESS);
}

#[test]
fn a_panic_after_a_single_statement_keeps_it() {
    let store = SqliteNegotiations::in_memory().unwrap();
    testing::assert_a_panic_after_a_single_statement_keeps_it(store, &ACCESS);
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
