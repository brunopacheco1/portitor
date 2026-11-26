// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What a store's schema holds, read back from the database: for the
//! test of each store that its PostgreSQL migrations end with the same
//! schema as its `SQLite` migrations: the same tables and columns, column
//! types, `NOT NULL`, primary keys and unique indexes.

use std::collections::{BTreeMap, BTreeSet};

use crate::database::{Database, Executor, Row};
use crate::values;

/// One table, as both databases can describe it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// Each column by name.
    pub columns: BTreeMap<String, Column>,
    /// The primary key's columns, in order; empty when there is none.
    pub primary_key: Vec<String>,
    /// The columns of each unique index (and `UNIQUE` constraint) other
    /// than the primary key, each in index order.
    pub unique: BTreeSet<Vec<String>>,
}

/// One column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The type, the same word for both databases: `integer` (`INTEGER`,
    /// `BIGINT`, `SMALLINT`), `text`, `bytes` (`BLOB`, `BYTEA`); any other
    /// type as the database names it, in lower case.
    pub kind: String,
    /// Whether `NULL` is refused. A primary key's columns refuse it on
    /// both (`SQLite` would take `NULL` in a `TEXT PRIMARY KEY`, but no
    /// store writes one).
    pub not_null: bool,
}

/// Each table of `db`. The bookkeeping tables are left out:
/// `portitor_schema` (PostgreSQL), `sqlite_sequence` (`SQLite`, for
/// `AUTOINCREMENT`) and `portitor_process_lease` (`SQLite` files,
/// `database::sqlite_lease`).
///
/// # Panics
/// The database cannot be read.
#[must_use]
pub fn tables(db: &Database) -> BTreeMap<String, Table> {
    let mut tables = BTreeMap::new();
    if db.is_postgres() {
        read_postgres(db, &mut tables);
    } else {
        read_sqlite(db, &mut tables);
    }
    for table in tables.values_mut() {
        for name in &table.primary_key {
            if let Some(column) = table.columns.get_mut(name) {
                column.not_null = true;
            }
        }
    }
    tables
}

fn read_postgres(db: &Database, tables: &mut BTreeMap<String, Table>) {
    // the names are of type `sql_identifier`: read as text
    let columns = query(
        db,
        "SELECT table_name::text, column_name::text, data_type::text,
                (is_nullable = 'NO')::int
             FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name <> 'portitor_schema'",
    );
    for row in columns {
        let column = Column {
            kind: kind(&text(&row, 2)),
            not_null: number(&row, 3) == 1,
        };
        let table = tables.entry(text(&row, 0)).or_default();
        table.columns.insert(text(&row, 1), column);
    }
    // each column of each unique index, in index order
    let indexes = query(
        db,
        "SELECT t.relname::text, i.relname::text, ix.indisprimary::int, a.attname::text
             FROM pg_index ix
             JOIN pg_class t ON t.oid = ix.indrelid
             JOIN pg_class i ON i.oid = ix.indexrelid
             JOIN pg_namespace n ON n.oid = t.relnamespace
             CROSS JOIN LATERAL unnest(ix.indkey::int2[]) WITH ORDINALITY AS k(attnum, position)
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
             WHERE n.nspname = current_schema() AND ix.indisunique
                 AND t.relname <> 'portitor_schema'
             ORDER BY 1, 2, k.position",
    );
    add_indexes(tables, &indexes);
}

fn read_sqlite(db: &Database, tables: &mut BTreeMap<String, Table>) {
    // `pragma_table_info` is the table form of `PRAGMA table_info`; `pk`
    // is the column's place in the primary key, 0 when not in it
    let columns = query(
        db,
        "SELECT m.name, c.name, c.type, c.\"notnull\", c.pk
             FROM sqlite_master m, pragma_table_info(m.name) c
             WHERE m.type = 'table'
                 AND m.name NOT IN ('sqlite_sequence', 'portitor_process_lease')
             ORDER BY m.name, c.pk",
    );
    for row in columns {
        let column = Column {
            kind: kind(&text(&row, 2)),
            not_null: number(&row, 3) == 1,
        };
        let table = tables.entry(text(&row, 0)).or_default();
        if number(&row, 4) > 0 {
            table.primary_key.push(text(&row, 1));
        }
        table.columns.insert(text(&row, 1), column);
    }
    // the primary key's own index (origin `pk`) is left out: the key is
    // read above, also when it has no index (`INTEGER PRIMARY KEY`)
    let indexes = query(
        db,
        "SELECT m.name, l.name, 0, c.name
             FROM sqlite_master m, pragma_index_list(m.name) l, pragma_index_info(l.name) c
             WHERE m.type = 'table' AND l.\"unique\" = 1 AND l.origin <> 'pk'
                 AND m.name NOT IN ('sqlite_sequence', 'portitor_process_lease')
             ORDER BY m.name, l.name, c.seqno",
    );
    add_indexes(tables, &indexes);
}

/// Adds the rows `(table, index, is primary key, column)`, in index
/// order, as primary keys and unique indexes.
fn add_indexes(tables: &mut BTreeMap<String, Table>, rows: &[Row]) {
    let mut by_index: BTreeMap<(String, String), (bool, Vec<String>)> = BTreeMap::new();
    for row in rows {
        let entry = by_index
            .entry((text(row, 0), text(row, 1)))
            .or_insert((number(row, 2) == 1, Vec::new()));
        entry.1.push(text(row, 3));
    }
    for ((table, _), (primary, columns)) in by_index {
        let table = tables.entry(table).or_default();
        if primary {
            table.primary_key = columns;
        } else {
            table.unique.insert(columns);
        }
    }
}

/// A column type as the same word for both databases.
fn kind(declared: &str) -> String {
    let lower = declared.to_ascii_lowercase();
    match lower.as_str() {
        "integer" | "bigint" | "smallint" => "integer".to_owned(),
        "blob" | "bytea" => "bytes".to_owned(),
        _ => lower,
    }
}

fn query(db: &Database, sql: &str) -> Vec<Row> {
    let mut db = db;
    match db.query(sql, values![]) {
        Ok(rows) => rows,
        Err(e) => panic!("cannot read the schema: {e}"),
    }
}

fn text(row: &Row, i: usize) -> String {
    match row.get::<String>(i) {
        Ok(v) => v,
        Err(e) => panic!("schema row {row:?}: {e}"),
    }
}

fn number(row: &Row, i: usize) -> i64 {
    match row.get::<i64>(i) {
        Ok(v) => v,
        Err(e) => panic!("schema row {row:?}: {e}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::database::Migrations;

    const MIGRATIONS: Migrations = Migrations {
        component: "schema-test",
        sqlite: &["CREATE TABLE t (
            id   TEXT PRIMARY KEY,
            n    INTEGER NOT NULL,
            data BLOB,
            a    TEXT,
            b    TEXT,
            UNIQUE (b, a)
        );
        CREATE TABLE r (id INTEGER PRIMARY KEY, note TEXT)"],
        postgres: &["CREATE TABLE t (
            id   TEXT PRIMARY KEY,
            n    BIGINT NOT NULL,
            data BYTEA,
            a    TEXT,
            b    TEXT,
            UNIQUE (b, a)
        );
        CREATE TABLE r (id BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, note TEXT)"],
    };

    #[test]
    fn a_schema_is_read_the_same_way_from_both_databases() {
        let sqlite = Database::in_memory(&MIGRATIONS).unwrap();
        let read = tables(&sqlite);
        let t = &read["t"];
        assert_eq!(t.primary_key, ["id"]);
        assert!(t.columns["id"].not_null);
        assert_eq!(t.columns["n"].kind, "integer");
        assert_eq!(t.columns["data"].kind, "bytes");
        assert!(!t.columns["data"].not_null);
        assert_eq!(
            t.unique,
            BTreeSet::from([vec!["b".to_owned(), "a".to_owned()]])
        );
        assert_eq!(read["r"].primary_key, ["id"]);
        if let Some(postgres) = crate::testing::databases::fresh_postgres(&MIGRATIONS) {
            assert_eq!(tables(&postgres), read);
        }
    }
}
