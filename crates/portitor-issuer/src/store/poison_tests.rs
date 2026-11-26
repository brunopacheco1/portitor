// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery of the
//! `SQLite` database (`portitor_runtime::database`) takes the poisoned
//! lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! The issuer's store writes more than one statement only inside a
//! transaction (`issue`, `revoke`).

use std::sync::{Arc, MutexGuard};

use portitor_runtime::database::Database;
use portitor_runtime::sqlite::testing;
use rusqlite::Connection;

use super::{DatabaseStore, IssuerStore, Participant};
use serde_json::Map;

/// How the shared checks reach this store: the `SQLite` connection of its
/// database, taken as the store takes it (the code under test), and its
/// lock.
const ACCESS: testing::Access<DatabaseStore> = testing::Access { conn, is_poisoned };

/// The store's `SQLite` connection, through its lock (a poisoned one
/// too).
fn conn(store: &DatabaseStore) -> MutexGuard<'_, Connection> {
    match store.db().sqlite_connection() {
        Some(conn) => conn,
        None => panic!("these tests run on SQLite"),
    }
}

/// Whether a panic poisoned the lock of the store's `SQLite` connection.
fn is_poisoned(store: &DatabaseStore) -> bool {
    match store.db() {
        Database::Sqlite(conn, _) => conn.is_poisoned(),
        Database::Postgres(_) => false,
    }
}

#[test]
fn a_panic_inside_a_transaction_rolls_it_back() {
    let store = DatabaseStore::in_memory().unwrap();
    testing::assert_a_panic_inside_a_transaction_rolls_it_back(store, &ACCESS);
}

#[test]
fn a_panic_after_a_single_statement_keeps_it() {
    let store = DatabaseStore::in_memory().unwrap();
    testing::assert_a_panic_after_a_single_statement_keeps_it(store, &ACCESS);
}

/// The store's own operations work on the poisoned lock too: a
/// participant is registered, read back and removed.
#[test]
fn a_participant_is_registered_and_read_after_a_panic() {
    let store = Arc::new(DatabaseStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let _conn = conn(&held);
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert!(is_poisoned(&store));

    let participants: &dyn IssuerStore = store.as_ref();
    let holder = Participant {
        did: "did:web:holder.example".to_owned(),
        claims: Map::new(),
        registered_at: 1_794_484_800,
    };
    assert!(participants.add_participant(&holder).unwrap());
    let found = participants.participant("did:web:holder.example").unwrap();
    assert_eq!(found, Some(holder));
    assert!(
        participants
            .delete_participant("did:web:holder.example")
            .unwrap()
    );
    assert!(participants.participants().unwrap().is_empty());
}
