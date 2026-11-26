// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the `SQLite` lock is held: the lock recovery of
//! `portitor_runtime::database` takes the poisoned lock, and the database
//! is as `SQLite` left it. (`SQLite` only: PostgreSQL has no such lock.)
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! Negotiations, transfers and data flows are written with one statement
//! each, a harvest run in one transaction: both cases are checked.

use std::sync::{Arc, MutexGuard};

use portitor_runtime::database::Database;
use portitor_runtime::sqlite::testing;
use rusqlite::Connection;

use super::ConnectorStore;

/// How the shared checks reach this store: the `SQLite` connection
/// under it (`Database::sqlite_connection`, the code under test) and
/// its lock.
const ACCESS: testing::Access<ConnectorStore> = testing::Access {
    conn: sqlite_connection,
    is_poisoned,
};

/// The store's `SQLite` connection, with the lock taken, also a poisoned
/// one.
fn sqlite_connection(store: &ConnectorStore) -> MutexGuard<'_, Connection> {
    store.database.sqlite_connection().unwrap()
}

/// Whether a panic poisoned the store's `SQLite` lock.
fn is_poisoned(store: &ConnectorStore) -> bool {
    match store.database.as_ref() {
        Database::Sqlite { conn, .. } => conn.is_poisoned(),
        Database::Postgres(_) => false,
    }
}

#[test]
fn a_panic_inside_a_transaction_rolls_it_back() {
    let store = ConnectorStore::in_memory().unwrap();
    testing::assert_a_panic_inside_a_transaction_rolls_it_back(store, &ACCESS);
}

#[test]
fn a_panic_after_a_single_statement_keeps_it() {
    let store = ConnectorStore::in_memory().unwrap();
    testing::assert_a_panic_after_a_single_statement_keeps_it(store, &ACCESS);
}

/// The store's own operations work on the poisoned lock too: a transfer
/// request is written once, found again, and its repeat adds nothing.
#[test]
fn a_transfer_is_stored_and_found_after_a_panic() {
    use portitor_dsp::transfer::TransferState;
    use portitor_dsp::transfer::provider::{Transfer, TransferStore};

    let store = Arc::new(ConnectorStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let _conn = sqlite_connection(&held);
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert!(is_poisoned(&store));

    let transfer = Transfer {
        provider_pid: "urn:uuid:p".to_owned(),
        consumer_pid: "urn:uuid:c".to_owned(),
        state: TransferState::Requested,
        callback_address: "https://c.example/cb".to_owned(),
        counter_party: "did:web:c".to_owned(),
        agreement_id: "urn:uuid:a".to_owned(),
        dataset_id: "d".to_owned(),
        format: "HttpData-PULL".to_owned(),
        data_address: None,
        pending: None,
        flow_step: None,
        state_before_step: None,
        lease: None,
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
