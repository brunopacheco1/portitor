// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The token ids callers used (TD-09), in the connector's database: the
//! `used_token` table, as `portitor_runtime::used_tokens` describes it
//! (modelled on Eclipse EDC's `edc_jti_validation`). The DCP verifier
//! records each accepted token's `jti` here
//! (`auth::dcp::DcpVerifier`), so a token is taken once by every process
//! that shares the database, also after a restart.
//!
//! On the `SQLite` connection of [`SqliteNegotiations`] until the
//! connector's store moves to `portitor_runtime::database` (1.3); then
//! this adapter calls `portitor_runtime::used_tokens::use_token_id`.

use portitor_runtime::used_tokens::UsedTokenStore;
use rusqlite::params;

use super::SqliteNegotiations;

impl UsedTokenStore for SqliteNegotiations {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM used_token WHERE expires_at < ?1",
            [seconds(now)],
        )
        .map_err(|e| format!("used token ids: {e}"))?;
        // one statement: of two processes that insert the same id at
        // once, one changes a row, the other none
        let inserted = conn
            .execute(
                "INSERT INTO used_token (token_id, expires_at) VALUES (?1, ?2)
                     ON CONFLICT (token_id) DO NOTHING",
                params![token_id, seconds(expires_at)],
            )
            .map_err(|e| format!("used token ids: {e}"))?;
        Ok(inserted == 1)
    }
}

/// Seconds since 1970 as `SQLite` keeps them: a signed 64-bit number.
fn seconds(at: u64) -> i64 {
    i64::try_from(at).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_token_id_is_used_once_until_it_expires() {
        let store = SqliteNegotiations::in_memory().unwrap();
        assert!(store.use_token_id("j-1", 200, 100).unwrap());
        assert!(!store.use_token_id("j-1", 200, 150).unwrap(), "replayed");
        // at 201 the row of j-1 has expired and is deleted
        assert!(store.use_token_id("j-2", 300, 201).unwrap());
        assert!(store.use_token_id("j-1", 400, 201).unwrap());
    }

    /// Two stores on one file (two processes): a token id used through
    /// one is refused through the other.
    #[test]
    fn a_token_id_used_by_one_process_is_refused_by_the_other() {
        let dir = portitor_runtime::testing::TempDir::new("connector-used-tokens");
        let path = dir.path().join("connector.db");
        let first = SqliteNegotiations::open(&path).unwrap();
        let second = SqliteNegotiations::open(&path).unwrap();
        assert!(first.use_token_id("j-1", 200, 100).unwrap());
        assert!(!second.use_token_id("j-1", 200, 100).unwrap());
    }
}
