// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The bundle version the connector uses, in its database (TD-55, ADR
//! 0029): every process of the connector moves to it
//! (`crate::bundle_harvest`). The bundle's files stay in each container's
//! cache folder.

use portitor_bundle::store::InUse;
use portitor_runtime::database::{DbError, Executor, Row};
use portitor_runtime::values;

use super::{ConnectorStore, StoreError, col};
use crate::bundle_harvest::SharedBundleVersion;

fn in_use_from_row(row: &Row) -> Result<InUse, StoreError> {
    Ok(InUse {
        version: col(row, 0)?,
        issued: col(row, 1)?,
    })
}

/// The recorded version, read in the transaction `tx`.
fn recorded(tx: &mut dyn Executor) -> Result<Option<InUse>, DbError> {
    let row = tx.query_one(
        "SELECT version, issued FROM bundle_in_use WHERE id = 1",
        values![],
    )?;
    match row {
        Some(row) => Ok(Some(InUse {
            version: row.get(0)?,
            issued: row.get(1)?,
        })),
        None => Ok(None),
    }
}

/// Records `to` in the transaction `tx`, as the only row.
fn record(tx: &mut dyn Executor, to: &InUse) -> Result<(), DbError> {
    let updated = tx.execute(
        "UPDATE bundle_in_use SET version = $1, issued = $2 WHERE id = 1",
        values![&to.version, &to.issued],
    )?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO bundle_in_use (id, version, issued) VALUES (1, $1, $2)",
            values![&to.version, &to.issued],
        )?;
    }
    Ok(())
}

/// Whether `to` may replace `current`: a version issued before the one
/// recorded never does (the same rule as `bundle_harvest::older_than`;
/// `YYYY-MM-DD...` dates compare as text).
fn may_replace(current: Option<&InUse>, to: &InUse) -> bool {
    match current {
        None => true,
        Some(current) => current.version == to.version || to.issued >= current.issued,
    }
}

impl SharedBundleVersion for ConnectorStore {
    fn bundle_in_use(&self) -> Result<Option<InUse>, StoreError> {
        self.query_first(
            "SELECT version, issued FROM bundle_in_use WHERE id = 1",
            values![],
            in_use_from_row,
        )
    }

    fn move_bundle_to(&self, to: &InUse, even_if_older: bool) -> Result<InUse, StoreError> {
        // read and write in one transaction: two processes that take a new
        // version at once leave the one issued last
        self.transaction(|tx| {
            let current = recorded(tx)?;
            if even_if_older || may_replace(current.as_ref(), to) {
                record(tx, to)?;
                return Ok(to.clone());
            }
            // `current` is there: may_replace is true without one
            Ok(current.unwrap_or_else(|| to.clone()))
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::on_every_database;

    fn version(version: &str, issued: &str) -> InUse {
        InUse {
            version: version.to_owned(),
            issued: issued.to_owned(),
        }
    }

    #[test]
    fn the_version_moves_forward_only_unless_forced() {
        on_every_database(|store| {
            assert_eq!(store.bundle_in_use().unwrap(), None);
            let first = version("2026.11.0", "2026-11-14T00:00:00Z");
            let newer = version("2026.12.0", "2026-12-01T00:00:00Z");
            assert_eq!(store.move_bundle_to(&first, false).unwrap(), first);
            assert_eq!(store.move_bundle_to(&newer, false).unwrap(), newer);
            // an older one is not taken: the answer is the version recorded
            assert_eq!(store.move_bundle_to(&first, false).unwrap(), newer);
            assert_eq!(store.bundle_in_use().unwrap(), Some(newer));
            // a fixed version (PORTITOR_BUNDLE_VERSION) is the operator's
            // choice, also an older one
            assert_eq!(store.move_bundle_to(&first, true).unwrap(), first);
            assert_eq!(store.bundle_in_use().unwrap(), Some(first));
        });
    }
}
