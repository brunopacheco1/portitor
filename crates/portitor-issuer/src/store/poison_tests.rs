// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery in `conn()`
//! takes the poisoned lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! The issuer's store writes more than one statement only inside a
//! transaction (`issue`, `revoke`).

use std::sync::Arc;

use portitor_runtime::sqlite::testing;

use super::*;

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

/// The store's own operations work on the poisoned lock too: a
/// participant is registered, read back and removed.
#[test]
fn a_participant_is_registered_and_read_after_a_panic() {
    let store = Arc::new(SqliteStore::in_memory().unwrap());
    let held = Arc::clone(&store);
    let panicked = std::thread::spawn(move || {
        let _conn = held.conn();
        panic!("while the lock is held");
    })
    .join();
    assert!(panicked.is_err());
    assert!(store.0.is_poisoned());

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
