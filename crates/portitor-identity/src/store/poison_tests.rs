// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery in `conn()`
//! takes the poisoned lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! The identity store writes more than one statement only inside a
//! transaction.

use portitor_runtime::sqlite::testing;

use super::SqliteStore;

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
