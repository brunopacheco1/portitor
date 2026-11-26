// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The receipts of push transfers as consumer (ADR 0027), in the same
//! database: the hash of each token, never the token.

use portitor_runtime::database::Row;
use portitor_runtime::values;

use super::{ConnectorStore, StoreError, col};
use crate::receiving::{PushReceipt, PushReceipts};

fn receipt_from_row(row: &Row) -> Result<PushReceipt, StoreError> {
    let expires_at: i64 = col(row, 1)?;
    Ok(PushReceipt {
        token_hash: col(row, 0)?,
        expires_at: u64::try_from(expires_at).unwrap_or(0),
        object_key: col(row, 2)?,
    })
}

impl PushReceipts for ConnectorStore {
    fn keep_token(
        &self,
        consumer_pid: &str,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<(), StoreError> {
        // a new token replaces the old one; `excluded` is the row that
        // could not be inserted. The object key stays.
        let expires_at = i64::try_from(expires_at).unwrap_or(i64::MAX);
        self.execute(
            "INSERT INTO push_receipt (consumer_pid, token_hash, expires_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (consumer_pid)
             DO UPDATE SET token_hash = excluded.token_hash, expires_at = excluded.expires_at",
            values![consumer_pid, token_hash, expires_at],
        )?;
        Ok(())
    }

    fn receipt(&self, consumer_pid: &str) -> Result<Option<PushReceipt>, StoreError> {
        self.query_first(
            "SELECT token_hash, expires_at, object_key FROM push_receipt
             WHERE consumer_pid = $1",
            values![consumer_pid],
            receipt_from_row,
        )
    }

    fn set_object_key(&self, consumer_pid: &str, object_key: &str) -> Result<(), StoreError> {
        self.execute(
            "UPDATE push_receipt SET object_key = $2 WHERE consumer_pid = $1",
            values![consumer_pid, object_key],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::on_every_database;

    #[test]
    fn a_new_token_replaces_the_old_one_and_keeps_the_object_key() {
        on_every_database(a_new_token_replaces_the_old_one_and_keeps_the_object_key_on);
    }

    fn a_new_token_replaces_the_old_one_and_keeps_the_object_key_on(store: &ConnectorStore) {
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
