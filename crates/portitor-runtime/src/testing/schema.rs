// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What a store's schema holds, read back from the database: for the
//! test of each store that its PostgreSQL migrations end with the same
//! tables and columns as its `SQLite` migrations.

use std::collections::{BTreeMap, BTreeSet};

use crate::database::{Database, Executor};
use crate::values;

/// Each table of `db` with the names of its columns, both sorted. The
/// bookkeeping tables are left out: `portitor_schema` (PostgreSQL) and
/// `sqlite_sequence` (`SQLite`, for `AUTOINCREMENT`).
///
/// # Panics
/// The database cannot be read.
#[must_use]
pub fn columns(db: &Database) -> BTreeMap<String, BTreeSet<String>> {
    let sql = if db.is_postgres() {
        // the names are of type `sql_identifier`: read as text
        "SELECT table_name::text, column_name::text FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name <> 'portitor_schema'"
    } else {
        // `pragma_table_info` is the table form of `PRAGMA table_info`
        "SELECT m.name, c.name FROM sqlite_master m, pragma_table_info(m.name) c
             WHERE m.type = 'table' AND m.name <> 'sqlite_sequence'"
    };
    let mut db = db;
    let rows = match db.query(sql, values![]) {
        Ok(rows) => rows,
        Err(e) => panic!("cannot read the schema: {e}"),
    };
    let mut tables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        let (Ok(table), Ok(column)) = (row.get::<String>(0), row.get::<String>(1)) else {
            panic!("not a table and a column name: {row:?}");
        };
        tables.entry(table).or_default().insert(column);
    }
    tables
}
