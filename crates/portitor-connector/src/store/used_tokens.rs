// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The token ids callers used (TD-09), in the connector's database: the
//! `used_token` table, as `portitor_runtime::used_tokens` describes it
//! (modelled on Eclipse EDC's `edc_jti_validation`). The DCP verifier
//! records each accepted token's `jti` here
//! (`auth::dcp::DcpVerifier`), so a token is taken once by every process
//! that shares the database, also after a restart.

use portitor_runtime::used_tokens::UsedTokenStore;

use super::ConnectorStore;

impl UsedTokenStore for ConnectorStore {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        self.database().use_token_id(token_id, expires_at, now)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_token_id_is_used_once_until_it_expires() {
        for database in portitor_runtime::testing::databases::all(&super::super::MIGRATIONS) {
            let store = ConnectorStore::new(std::sync::Arc::new(database));
            assert!(store.use_token_id("j-1", 200, 100).unwrap());
            assert!(!store.use_token_id("j-1", 200, 150).unwrap(), "replayed");
            // at 201 the row of j-1 has expired and is deleted
            assert!(store.use_token_id("j-2", 300, 201).unwrap());
            assert!(store.use_token_id("j-1", 400, 201).unwrap());
        }
    }

    /// Two stores on one file (two processes): a token id used through
    /// one is refused through the other.
    #[test]
    fn a_token_id_used_by_one_process_is_refused_by_the_other() {
        let dir = portitor_runtime::testing::TempDir::new("connector-used-tokens");
        let path = dir.path().join("connector.db");
        let first = ConnectorStore::open_sqlite_file(&path).unwrap();
        let second = ConnectorStore::open_sqlite_file(&path).unwrap();
        assert!(first.use_token_id("j-1", 200, 100).unwrap());
        assert!(!second.use_token_id("j-1", 200, 100).unwrap());
    }
}
