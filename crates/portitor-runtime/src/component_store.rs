// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The store of a component in its own database, written once for the
//! identity and the issuer (TD-71). [`ComponentStore`] holds the
//! database; each component names its schema with a [`ComponentSchema`]
//! and adds its own store port on top:
//!
//! ```ignore
//! pub struct IdentitySchema;
//! impl ComponentSchema for IdentitySchema {
//!     const MIGRATIONS: &'static Migrations = &MIGRATIONS;
//! }
//! pub type DatabaseStore = ComponentStore<IdentitySchema>;
//! impl IdentityStore for DatabaseStore { /* ... */ }
//! ```
//!
//! The used token ids ([`UsedTokenStore`]) and the leases
//! ([`LeaseStore`]) are the same tables in every component that has
//! them, so they are here, once. Every component has the `used_token`
//! table; only a schema marked [`WithLeaseTable`] has the `lease` table
//! (the identity; the issuer keeps its delivery leases on the outbox
//! rows, `portitor_runtime::leases`).

use std::marker::PhantomData;
use std::sync::Arc;

use crate::database::{Database, DatabaseSetting, DbError, Migrations};
use crate::leases::LeaseStore;
use crate::used_tokens::UsedTokenStore;

/// Names a component's schema: the migrations of its database.
pub trait ComponentSchema {
    /// The component's migrations, for both databases.
    const MIGRATIONS: &'static Migrations;
}

/// Marks a schema that has the `lease` table: only then is its store a
/// [`LeaseStore`]. A marker: the trait has nothing to implement.
pub trait WithLeaseTable: ComponentSchema {}

/// Adapter: a component's store in its database (PostgreSQL, or a
/// `SQLite` file), shared with the health check. `S` only says which
/// schema: it decides the migrations [`ComponentStore::open`] runs and
/// keeps the stores of two components apart as two types.
#[derive(Debug)]
pub struct ComponentStore<S> {
    database: Arc<Database>,
    // `S` is not stored; `PhantomData` lets the type name it anyway
    // ("A type that names what it does not hold", `docs/RUST-IDIOMS.md`)
    schema: PhantomData<S>,
}

/// Written by hand, not derived: a derived `Clone` would also ask `S`
/// to be `Clone`, though only the pointer to the database is copied.
impl<S> Clone for ComponentStore<S> {
    fn clone(&self) -> Self {
        Self {
            database: Arc::clone(&self.database),
            schema: PhantomData,
        }
    }
}

impl<S: ComponentSchema> ComponentStore<S> {
    /// The store in `database`, whose schema is `S::MIGRATIONS`.
    #[must_use]
    pub fn new(database: Arc<Database>) -> Self {
        Self {
            database,
            schema: PhantomData,
        }
    }

    /// Opens (or creates) the database `setting` names and brings its
    /// schema up to date.
    ///
    /// # Errors
    /// The database cannot be opened, or was written by a newer Portitor.
    pub fn open(setting: &DatabaseSetting) -> Result<Self, DbError> {
        Ok(Self::new(Arc::new(Database::open(setting, S::MIGRATIONS)?)))
    }

    /// An in-memory `SQLite` store, for tests.
    ///
    /// # Errors
    /// `SQLite` cannot allocate the database.
    pub fn in_memory() -> Result<Self, DbError> {
        Ok(Self::new(Arc::new(Database::in_memory(S::MIGRATIONS)?)))
    }

    /// The database, for the health check.
    #[must_use]
    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    /// The database, to run one statement on (`self.db().execute(...)`).
    #[must_use]
    pub fn db(&self) -> &Database {
        &self.database
    }
}

/// The `used_token` table: passed on to the database.
impl<S: ComponentSchema + Send + Sync> UsedTokenStore for ComponentStore<S> {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        self.db().use_token_id(token_id, expires_at, now)
    }
}

/// The `lease` table: passed on to the database.
impl<S: WithLeaseTable + Send + Sync> LeaseStore for ComponentStore<S> {
    fn claim(
        &self,
        kind: &str,
        id: &str,
        holder: &str,
        until: u64,
        now: u64,
    ) -> Result<bool, String> {
        self.db().claim(kind, id, holder, until, now)
    }

    fn extend(&self, kind: &str, id: &str, holder: &str, until: u64) -> Result<bool, String> {
        self.db().extend(kind, id, holder, until)
    }

    fn release(&self, kind: &str, id: &str, holder: &str) -> Result<(), String> {
        self.db().release(kind, id, holder)
    }

    fn held(&self, kind: &str, now: u64) -> Result<Vec<String>, String> {
        self.db().held(kind, now)
    }

    fn forget_ended(&self, before: u64) -> Result<u64, String> {
        self.db().forget_ended(before)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const TEST_MIGRATIONS: Migrations = Migrations {
        component: "portitor-test",
        sqlite: &[
            "CREATE TABLE used_token (token_id TEXT PRIMARY KEY, expires_at INTEGER NOT NULL)",
            "CREATE TABLE lease (kind TEXT NOT NULL, id TEXT NOT NULL, \
             lease_holder TEXT NOT NULL, lease_until INTEGER NOT NULL, PRIMARY KEY (kind, id))",
        ],
        postgres: &[],
    };

    #[derive(Debug)]
    struct TestSchema;

    impl ComponentSchema for TestSchema {
        const MIGRATIONS: &'static Migrations = &TEST_MIGRATIONS;
    }

    impl WithLeaseTable for TestSchema {}

    /// The two tables every component has are reached through the
    /// store, and a clone is the same database.
    #[test]
    fn used_tokens_and_leases_pass_on_to_the_database() {
        let store = ComponentStore::<TestSchema>::in_memory().unwrap();
        let other = store.clone();
        assert!(Arc::ptr_eq(store.database(), other.database()));

        assert!(store.use_token_id("t-1", 200, 100).unwrap());
        assert!(!other.use_token_id("t-1", 200, 100).unwrap(), "a replay");

        assert!(store.claim("job", "a", "one", 200, 100).unwrap());
        assert!(!other.claim("job", "a", "two", 200, 100).unwrap());
        assert_eq!(store.held("job", 100).unwrap(), ["a"]);
        assert!(store.extend("job", "a", "one", 300).unwrap());
        store.release("job", "a", "one").unwrap();
        assert!(other.claim("job", "a", "two", 200, 100).unwrap());
        assert_eq!(other.forget_ended(500).unwrap(), 1);
    }
}
