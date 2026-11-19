// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A panic while the store's lock is held: the lock recovery in `conn()`
//! takes the poisoned lock, and the database is as `SQLite` left it.
//! The checks themselves, and why this is safe, are in
//! `portitor_runtime::sqlite::testing`, shared by the three stores.
//! Negotiations, transfers and data flows are written with one statement
//! each, a harvest run in one transaction: both cases are checked.

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
