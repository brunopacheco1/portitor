// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The receipts of push transfers as consumer (ADR 0027), in the same
//! `SQLite` file: the hash of each token, never the token.

use rusqlite::{Row, params};

use super::{SqliteNegotiations, StoreError, col, err};
use crate::receiving::{PushReceipt, PushReceipts};

fn receipt_from_row(row: &Row<'_>) -> Result<PushReceipt, StoreError> {
    let expires_at: i64 = col(row, 1)?;
    Ok(PushReceipt {
        token_hash: col(row, 0)?,
        expires_at: u64::try_from(expires_at).unwrap_or(0),
        object_key: col(row, 2)?,
    })
}

impl PushReceipts for SqliteNegotiations {
    fn keep_token(
        &self,
        consumer_pid: &str,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<(), StoreError> {
        let expires_at = i64::try_from(expires_at).unwrap_or(i64::MAX);
        self.conn()
            .execute(
                "INSERT INTO push_receipt (consumer_pid, token_hash, expires_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (consumer_pid)
                 DO UPDATE SET token_hash = ?2, expires_at = ?3",
                params![consumer_pid, token_hash, expires_at],
            )
            .map_err(err)?;
        Ok(())
    }

    fn receipt(&self, consumer_pid: &str) -> Result<Option<PushReceipt>, StoreError> {
        let found = self.query(
            "SELECT token_hash, expires_at, object_key FROM push_receipt
             WHERE consumer_pid = ?1",
            params![consumer_pid],
            receipt_from_row,
        )?;
        Ok(found.into_iter().next())
    }

    fn set_object_key(&self, consumer_pid: &str, object_key: &str) -> Result<(), StoreError> {
        self.conn()
            .execute(
                "UPDATE push_receipt SET object_key = ?2 WHERE consumer_pid = ?1",
                params![consumer_pid, object_key],
            )
            .map_err(err)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_new_token_replaces_the_old_one_and_keeps_the_object_key() {
        let store = SqliteNegotiations::in_memory().unwrap();
        assert_eq!(store.receipt("c").unwrap(), None);
        store.keep_token("c", "hash-1", 10).unwrap();
        store.set_object_key("c", "c").unwrap();
        store.keep_token("c", "hash-2", 20).unwrap();
        assert_eq!(
            store.receipt("c").unwrap(),
            Some(PushReceipt {
                token_hash: "hash-2".to_owned(),
                expires_at: 20,
                object_key: Some("c".to_owned()),
            })
        );
    }
}
