// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Used token ids (`jti`, DCP v1.0.1, section 4.3.3, step 6) in the
//! database: a token accepted by one replica is refused by every other
//! one, also after a restart (TD-09).
//!
//! Modelled on Eclipse EDC's `edc_jti_validation` table: one row per id,
//! kept until the token expires. Each component has the table in its own
//! schema:
//!
//! ```sql
//! CREATE TABLE used_token (
//!     token_id   TEXT PRIMARY KEY,
//!     expires_at BIGINT NOT NULL  -- seconds since 1970 (INTEGER in SQLite)
//! );
//! CREATE INDEX used_token_expires_at ON used_token (expires_at);
//! ```
//!
//! A token id is "used" with one `INSERT ... ON CONFLICT DO NOTHING`: of
//! several replicas that get the same token at once, the database lets
//! exactly one insert the row, and the others see that no row changed.
//! Expired rows are deleted on the way, before each insert (cheap: the
//! index finds them).
//!
//! There is no limit on the number of ids, as there was on the list in
//! memory (100,000, against running out of memory): a token lives at
//! most an hour (`portitor_dcp::MAX_TOKEN_LIFETIME`), so the table holds
//! the tokens of the last hour, and a row is a few dozen bytes on disk.

use std::sync::{Mutex, PoisonError};

use portitor_dcp::{Error, JtiCache, UsedTokenIds};

use crate::database::{Database, DbError, Executor, to_db_int};
use crate::values;

/// Port: where a component records the token ids it accepted.
pub trait UsedTokenStore: Send + Sync {
    /// Records `token_id` as used until `expires_at` (seconds since 1970),
    /// and forgets the ids that expired before `now`. Returns false when
    /// the id was recorded before and has not expired: a replay.
    ///
    /// # Errors
    /// The store fails; the text says why.
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String>;
}

/// Adapter: the `used_token` table of a component's database.
impl UsedTokenStore for Database {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        use_token_id(self, token_id, expires_at, now).map_err(|e| format!("used token ids: {e}"))
    }
}

/// Adapter: a list in memory, for one process (tests, and checks of
/// tokens that are never presented anywhere). A full list is an error.
impl UsedTokenStore for Mutex<JtiCache> {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        // a panic while the lock was held leaves at most one more id
        let mut cache = self.lock().unwrap_or_else(PoisonError::into_inner);
        match cache.remember(token_id, expires_at, now) {
            Ok(()) => Ok(true),
            Err(Error::Invalid("jti replayed")) => Ok(false),
            Err(other) => Err(other.to_string()),
        }
    }
}

/// Records `token_id` in the `used_token` table of `db` until
/// `expires_at`, after deleting the rows that expired before `now`.
/// Returns false when the id is there already: a replay.
///
/// # Errors
/// The database fails.
pub fn use_token_id(
    db: &Database,
    token_id: &str,
    expires_at: u64,
    now: u64,
) -> Result<bool, DbError> {
    db.execute(
        "DELETE FROM used_token WHERE expires_at < $1",
        values![to_db_int(now)],
    )?;
    // one statement: when two replicas insert the same id at once, the
    // database lets one of them change a row, and the other changes none
    let inserted = db.execute(
        "INSERT INTO used_token (token_id, expires_at) VALUES ($1, $2)
             ON CONFLICT (token_id) DO NOTHING",
        values![token_id, to_db_int(expires_at)],
    )?;
    Ok(inserted == 1)
}

/// Lets [`portitor_dcp::validate_self_issued`] record the token id in a
/// [`UsedTokenStore`]. A store that fails refuses the token (and the
/// failure is logged): a token whose id could not be recorded could be
/// replayed.
pub struct RecordIn<'a>(pub &'a dyn UsedTokenStore);

impl UsedTokenIds for RecordIn<'_> {
    fn remember(&mut self, jti: &str, until: u64, now: u64) -> Result<(), Error> {
        match self.0.use_token_id(jti, until, now) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::Invalid("jti replayed")),
            Err(error) => {
                tracing::error!(%error, "cannot record a used token id");
                Err(Error::Invalid("the token id cannot be recorded"))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::database::Migrations;

    const MIGRATIONS: Migrations = Migrations {
        component: "portitor-test",
        sqlite: &["CREATE TABLE used_token (
                token_id   TEXT PRIMARY KEY,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX used_token_expires_at ON used_token (expires_at)"],
        postgres: &["CREATE TABLE used_token (
                token_id   TEXT PRIMARY KEY,
                expires_at BIGINT NOT NULL
            );
            CREATE INDEX used_token_expires_at ON used_token (expires_at)"],
    };

    #[test]
    fn an_id_is_used_once_until_it_expires() {
        for db in crate::testing::databases::all(&MIGRATIONS) {
            let store: &dyn UsedTokenStore = &db;
            assert!(store.use_token_id("j-1", 200, 100).unwrap());
            assert!(!store.use_token_id("j-1", 200, 150).unwrap(), "replayed");
            assert!(store.use_token_id("j-2", 300, 150).unwrap());
            // at 201, j-1 has expired: its row is deleted, j-2 stays
            assert!(store.use_token_id("j-3", 400, 201).unwrap());
            let left = db
                .query(
                    "SELECT token_id FROM used_token ORDER BY token_id",
                    values![],
                )
                .unwrap();
            let left: Vec<String> = left.iter().map(|row| row.get(0).unwrap()).collect();
            assert_eq!(left, ["j-2", "j-3"]);
        }
    }

    #[test]
    fn the_list_in_memory_says_the_same() {
        let store = Mutex::new(JtiCache::with_max(2));
        assert!(store.use_token_id("j-1", 200, 100).unwrap());
        assert!(!store.use_token_id("j-1", 200, 150).unwrap());
        assert!(store.use_token_id("j-2", 200, 150).unwrap());
        assert!(store.use_token_id("j-3", 200, 150).is_err(), "full");
    }

    #[test]
    fn a_replay_or_a_failing_store_refuses_the_token() {
        struct Failing;
        impl UsedTokenStore for Failing {
            fn use_token_id(&self, _: &str, _: u64, _: u64) -> Result<bool, String> {
                Err("down".to_owned())
            }
        }
        let memory = Mutex::new(JtiCache::default());
        assert_eq!(RecordIn(&memory).remember("j-1", 200, 100), Ok(()));
        assert_eq!(
            RecordIn(&memory).remember("j-1", 200, 100),
            Err(Error::Invalid("jti replayed"))
        );
        assert!(RecordIn(&Failing).remember("j-2", 200, 100).is_err());
    }

    /// Two replicas (two pools on one database) get the same token ids at
    /// once: each id is accepted exactly once.
    #[test]
    fn of_two_replicas_only_one_accepts_a_token() {
        let Some(url) = crate::testing::databases::postgres_url() else {
            return;
        };
        let settings = crate::testing::databases::fresh_postgres_settings(&url);
        let setting = crate::database::DatabaseSetting::Postgres(Box::new(settings));
        let replicas: Vec<std::sync::Arc<Database>> = (0..2)
            .map(|_| std::sync::Arc::new(Database::open(&setting, &MIGRATIONS).unwrap()))
            .collect();
        let threads: Vec<_> = replicas
            .iter()
            .map(|db| {
                let db = std::sync::Arc::clone(db);
                std::thread::spawn(move || {
                    let mut accepted = 0;
                    for n in 0..50 {
                        if db.use_token_id(&format!("j-{n}"), 200, 100).unwrap() {
                            accepted += 1;
                        }
                    }
                    accepted
                })
            })
            .collect();
        let accepted: u32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(accepted, 50, "each of the 50 ids accepted once");
    }
}
