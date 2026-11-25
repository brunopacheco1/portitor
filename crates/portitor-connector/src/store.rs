// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter: negotiations (as provider and as consumer) and transfers (as
//! provider) kept in a `SQLite` file. Transfers are in `store/transfers.rs`.
//!
//! One connector process owns one file (no sharing between replicas). The
//! SQL is kept portable (plain `CREATE TABLE`, `INSERT`, `UPDATE ... WHERE`)
//! so a `PostgreSQL` adapter can follow the same migrations. Offers and
//! agreements are stored as their DSP JSON, as received or sent.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use portitor_dsp::negotiation::consumer::{self, ConsumerNegotiation, ConsumerStore};
use portitor_dsp::negotiation::provider::{self, Negotiation, NegotiationStore, StoreError};
use portitor_dsp::negotiation::{NegotiationState, Transition};
use rusqlite::{Connection, Row, params};

mod consumer_transfers;
mod data_flows;
mod harvest;
mod push_receipts;
mod transfers;
use serde::{Deserialize, Serialize};
pub use transfers::finalized_agreement;

/// Schema migrations, applied in order. The schema version is the number
/// of migrations applied (`PRAGMA user_version`). Never edit a migration
/// that has been released; add a new one.
const MIGRATIONS: &[&str] = &[
    // 0.3: negotiations as provider
    "CREATE TABLE negotiation (
        provider_pid     TEXT PRIMARY KEY,
        consumer_pid     TEXT NOT NULL,
        state            TEXT NOT NULL,
        callback_address TEXT NOT NULL,
        counter_party    TEXT NOT NULL,
        offer            TEXT NOT NULL,
        agreement        TEXT
    )",
    // 0.3: negotiations as consumer
    "CREATE TABLE consumer_negotiation (
        consumer_pid     TEXT PRIMARY KEY,
        provider_pid     TEXT,
        state            TEXT NOT NULL,
        provider_address TEXT NOT NULL,
        provider_id      TEXT NOT NULL,
        offer            TEXT NOT NULL,
        agreement        TEXT
    )",
    // 0.3: the message on its way (outbox) and a version for
    // compare-and-update (ADR 0007)
    "ALTER TABLE negotiation ADD COLUMN pending TEXT;
     ALTER TABLE negotiation ADD COLUMN version INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE consumer_negotiation ADD COLUMN pending TEXT;
     ALTER TABLE consumer_negotiation ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
    // 0.4: transfers as provider (ADR 0013), with the same outbox and
    // version as negotiations
    "CREATE TABLE transfer (
        provider_pid     TEXT PRIMARY KEY,
        consumer_pid     TEXT NOT NULL,
        state            TEXT NOT NULL,
        callback_address TEXT NOT NULL,
        counter_party    TEXT NOT NULL,
        agreement_id     TEXT NOT NULL,
        dataset_id       TEXT NOT NULL,
        format           TEXT NOT NULL,
        pending          TEXT,
        version          INTEGER NOT NULL DEFAULT 0
    )",
    // 0.4: the built-in data plane's flows (ADR 0013); the token is kept
    // only as its SHA-256 hash
    "CREATE TABLE data_flow (
        id         TEXT PRIMARY KEY,
        state      TEXT NOT NULL,
        dataset_id TEXT NOT NULL,
        token_hash TEXT NOT NULL,
        expires_at INTEGER NOT NULL
    )",
    // 0.4: transfers as consumer (ADR 0014); `data_address` holds the
    // provider's data address with its access token, only while the
    // transfer can still be used
    "CREATE TABLE consumer_transfer (
        consumer_pid     TEXT PRIMARY KEY,
        provider_pid     TEXT,
        state            TEXT NOT NULL,
        provider_address TEXT NOT NULL,
        provider_id      TEXT NOT NULL,
        agreement_id     TEXT NOT NULL,
        format           TEXT NOT NULL,
        data_address     TEXT,
        pending          TEXT,
        version          INTEGER NOT NULL DEFAULT 0
    )",
    // 0.7: the read copy of harvested datasets, the harvest jobs with
    // their entry errors, and the rules version and snapshot hash of the
    // last run (ADR 0020); `entry` is the dataset as the source gave it,
    // checked again when the rules change; `content` is what the catalog
    // shows; times are seconds since 1970
    "CREATE TABLE dataset (
        id             TEXT PRIMARY KEY,
        entry          TEXT NOT NULL,
        content        TEXT NOT NULL,
        upstream       TEXT,
        content_sha256 TEXT NOT NULL,
        state          TEXT NOT NULL,
        conforms_to    TEXT,
        first_seen     INTEGER NOT NULL,
        last_changed   INTEGER NOT NULL,
        last_job       INTEGER NOT NULL
    );
    CREATE TABLE harvest_job (
        id              INTEGER PRIMARY KEY,
        started_at      INTEGER NOT NULL,
        finished_at     INTEGER NOT NULL,
        run_trigger     TEXT NOT NULL,
        source          TEXT NOT NULL,
        snapshot_sha256 TEXT,
        outcome         TEXT NOT NULL,
        seen            INTEGER NOT NULL,
        added           INTEGER NOT NULL,
        updated         INTEGER NOT NULL,
        unchanged       INTEGER NOT NULL,
        withdrawn       INTEGER NOT NULL,
        rejected        INTEGER NOT NULL,
        summary         TEXT NOT NULL
    );
    CREATE TABLE harvest_error (
        job_id         INTEGER NOT NULL,
        seq            INTEGER NOT NULL,
        entry_position INTEGER,
        dataset_id     TEXT,
        kind           TEXT NOT NULL,
        message        TEXT NOT NULL,
        report         TEXT,
        PRIMARY KEY (job_id, seq)
    );
    CREATE TABLE harvest_state (
        id              INTEGER PRIMARY KEY,
        rules_version   TEXT,
        snapshot_sha256 TEXT
    );
    INSERT INTO harvest_state (id, rules_version, snapshot_sha256) VALUES (1, NULL, NULL)",
    // 0.7: the job log also holds the bundle harvester's runs (ADR 0005);
    // every earlier job harvested datasets
    "ALTER TABLE harvest_job ADD COLUMN kind TEXT NOT NULL DEFAULT 'datasets';",
    // 0.12: a repeated transfer request is found by its consumer and
    // consumer pid (DSP 2025-1, "Transfer Request Message": idempotent
    // on `consumerPid`; TD-34). A plain index, not a UNIQUE one: a file
    // from before 0.12 may hold a second transfer for a repeated request,
    // and must still open. `store/transfers.rs` (`insert`) keeps new rows
    // unique.
    "CREATE INDEX transfer_request ON transfer (counter_party, consumer_pid)",
    // 0.12: a transfer request names its agreement, and the provider looks
    // the negotiation up by it (`store/transfers.rs`, `finalized`). Without
    // an index every stored negotiation was read: 8 ms on an empty file,
    // 38 ms with 10,000 negotiations (docs/BENCHMARKS.md). The expression
    // must stay exactly the one of that query, or SQLite does not use the
    // index.
    "CREATE INDEX negotiation_agreement
        ON negotiation (json_extract(agreement, '$.\"@id\"'))",
    // 0.12: the same lookup on the consumer's side, when it starts a
    // transfer under an agreement it holds (`store/consumer_transfers.rs`)
    "CREATE INDEX consumer_negotiation_agreement
        ON consumer_negotiation (json_extract(agreement, '$.\"@id\"'))",
    // 1.1: the consumer's memberships, kept with each data flow, so the
    // data plane can check them again on every request (ADR 0027, section
    // 7.2; `store/data_flows.rs`). `memberships` is their JSON list. Every
    // flow of 1.0 gets a row with NULL: it started before memberships
    // were kept, and is served until its token expires.
    "CREATE TABLE flow_membership (
        flow_id     TEXT PRIMARY KEY,
        memberships TEXT
    );
    INSERT INTO flow_membership (flow_id, memberships) SELECT id, NULL FROM data_flow",
    // 1.1: push transfers (ADR 0027). As provider, the consumer's data
    // address with its token, kept with the transfer and with its data
    // flow until they end (the columns are emptied then). As consumer, the
    // hash of the token our receiving route takes, and the key of the
    // object the data went into once it arrived
    "ALTER TABLE transfer ADD COLUMN data_address TEXT;
     ALTER TABLE data_flow ADD COLUMN push_address TEXT;
     CREATE TABLE push_receipt (
        consumer_pid TEXT PRIMARY KEY,
        token_hash   TEXT NOT NULL,
        expires_at   INTEGER NOT NULL,
        object_key   TEXT
     )",
    // 1.2: the data plane step a provider's transfer write decided, run
    // after that write and then emptied (ADR 0028, decision 6; TD-48;
    // `portitor-dsp`, `transfer::provider::flow_steps`). NULL: none waits
    "ALTER TABLE transfer ADD COLUMN flow_step TEXT",
];

/// Adapter: negotiations in a `SQLite` file.
#[derive(Debug)]
pub struct SqliteNegotiations(Mutex<Connection>);

impl SqliteNegotiations {
    /// Opens (or creates) the database file and brings its schema up to
    /// date.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or created, it is not a `SQLite` database,
    /// or it was written by a newer Portitor version.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_at(Some(path))
    }

    /// An in-memory database, for tests.
    ///
    /// # Errors
    ///
    /// `SQLite` cannot allocate the database.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::open_at(None)
    }

    /// The file at `path`, or an in-memory database when `None`, opened
    /// and migrated the way every Portitor store is (`portitor_runtime::sqlite`).
    fn open_at(path: Option<&Path>) -> Result<Self, StoreError> {
        let conn = portitor_runtime::sqlite::open(path, MIGRATIONS).map_err(StoreError)?;
        Ok(Self(Mutex::new(conn)))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // SQLite keeps the file consistent even if a holder panicked
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn query<T>(
        &self,
        sql: &str,
        args: impl rusqlite::Params,
        from_row: fn(&Row<'_>) -> Result<T, StoreError>,
    ) -> Result<Vec<T>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(sql).map_err(err)?;
        let mut rows = stmt.query(args).map_err(err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            out.push(from_row(row)?);
        }
        Ok(out)
    }
}

#[allow(clippy::needless_pass_by_value)] // used as map_err(err)
fn err(e: rusqlite::Error) -> StoreError {
    StoreError(e.to_string())
}

fn state_text(s: NegotiationState) -> Result<String, StoreError> {
    match serde_json::to_value(s) {
        Ok(serde_json::Value::String(text)) => Ok(text),
        _ => Err(StoreError(format!("cannot store state {s:?}"))),
    }
}

fn json<T: Serialize>(v: &T) -> Result<String, StoreError> {
    serde_json::to_string(v).map_err(|e| StoreError(e.to_string()))
}

fn version(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| StoreError(format!("version {v} out of range")))
}

/// Column `i` of a row.
fn col<T: rusqlite::types::FromSql>(row: &Row<'_>, i: usize) -> Result<T, StoreError> {
    row.get(i).map_err(err)
}

fn parsed<T: for<'de> Deserialize<'de>>(what: &str, text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError(format!("stored {what}: {e}")))
}

fn parsed_state(text: String) -> Result<NegotiationState, StoreError> {
    serde_json::from_value(serde_json::Value::String(text))
        .map_err(|e| StoreError(format!("stored state: {e}")))
}

fn parsed_version(v: i64) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(|_| StoreError(format!("stored version {v}")))
}

/// How a pending message is stored (JSON in the `pending` column). The
/// negotiation or transfer it belongs to is the row; `S` is its state type.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMessage<S> {
    url: String,
    status_url: String,
    body: serde_json::Value,
    /// The transition's stable name (`Transition::name`).
    transition: String,
    to: S,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    initial: bool,
}

impl<S> StoredMessage<S> {
    fn new(
        url: &str,
        status_url: &str,
        body: &serde_json::Value,
        transition: &str,
        to: S,
        initial: bool,
    ) -> Self {
        Self {
            url: url.to_owned(),
            status_url: status_url.to_owned(),
            body: body.clone(),
            transition: transition.to_owned(),
            to,
            initial,
        }
    }
}

impl StoredMessage<NegotiationState> {
    fn transition(&self) -> Result<Transition, StoreError> {
        Transition::from_name(&self.transition)
            .ok_or_else(|| StoreError(format!("stored transition {}", self.transition)))
    }
}

fn provider_pending(o: Option<&provider::Outgoing>) -> Result<Option<String>, StoreError> {
    o.map(|o| {
        json(&StoredMessage::new(
            &o.url,
            &o.status_url,
            &o.body,
            o.transition.name(),
            o.to,
            false,
        ))
    })
    .transpose()
}

fn consumer_pending(o: Option<&consumer::Outgoing>) -> Result<Option<String>, StoreError> {
    o.map(|o| {
        json(&StoredMessage::new(
            &o.url,
            &o.status_url,
            &o.body,
            o.transition.name(),
            o.to,
            o.initial,
        ))
    })
    .transpose()
}

const NEGOTIATION_COLUMNS: &str = "provider_pid, consumer_pid, state, callback_address, \
     counter_party, offer, agreement, pending, version";

fn negotiation_from_row(row: &Row<'_>) -> Result<Negotiation, StoreError> {
    let provider_pid: String = col(row, 0)?;
    let counter_party: String = col(row, 4)?;
    let pending = col::<Option<String>>(row, 7)?
        .map(|p| -> Result<provider::Outgoing, StoreError> {
            let m: StoredMessage<NegotiationState> = parsed("pending message", &p)?;
            Ok(provider::Outgoing {
                provider_pid: provider_pid.clone(),
                transition: m.transition()?,
                url: m.url,
                status_url: m.status_url,
                body: m.body,
                to: m.to,
                counter_party: counter_party.clone(),
            })
        })
        .transpose()?;
    Ok(Negotiation {
        consumer_pid: col(row, 1)?,
        state: parsed_state(col(row, 2)?)?,
        callback_address: col(row, 3)?,
        counter_party,
        offer: parsed("offer", &col::<String>(row, 5)?)?,
        agreement: col::<Option<String>>(row, 6)?
            .map(|a| parsed("agreement", &a))
            .transpose()?,
        pending,
        version: parsed_version(col(row, 8)?)?,
        provider_pid,
    })
}

impl NegotiationStore for SqliteNegotiations {
    fn get(&self, provider_pid: &str) -> Result<Option<Negotiation>, StoreError> {
        let sql = format!("SELECT {NEGOTIATION_COLUMNS} FROM negotiation WHERE provider_pid = ?1");
        Ok(self
            .query(&sql, params![provider_pid], negotiation_from_row)?
            .into_iter()
            .next())
    }

    fn insert(&self, negotiation: &Negotiation) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO negotiation ({NEGOTIATION_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        );
        self.conn()
            .execute(
                &sql,
                params![
                    negotiation.provider_pid,
                    negotiation.consumer_pid,
                    state_text(negotiation.state)?,
                    negotiation.callback_address,
                    negotiation.counter_party,
                    json(&negotiation.offer)?,
                    negotiation.agreement.as_ref().map(json).transpose()?,
                    provider_pending(negotiation.pending.as_ref())?,
                    version(negotiation.version)?,
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        // compare-and-update in one statement: a concurrent change makes
        // the WHERE clause miss and nothing is written
        let changed = self
            .conn()
            .execute(
                "UPDATE negotiation
                 SET consumer_pid = ?2, state = ?3, callback_address = ?4, counter_party = ?5,
                     offer = ?6, agreement = ?7, pending = ?8, version = ?9
                 WHERE provider_pid = ?1 AND version = ?10",
                params![
                    negotiation.provider_pid,
                    negotiation.consumer_pid,
                    state_text(negotiation.state)?,
                    negotiation.callback_address,
                    negotiation.counter_party,
                    json(&negotiation.offer)?,
                    negotiation.agreement.as_ref().map(json).transpose()?,
                    provider_pending(negotiation.pending.as_ref())?,
                    version(negotiation.version)?,
                    version(expected)?,
                ],
            )
            .map_err(err)?;
        Ok(changed == 1)
    }

    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        let sql = format!(
            "SELECT {NEGOTIATION_COLUMNS} FROM negotiation WHERE pending IS NOT NULL
             ORDER BY provider_pid"
        );
        self.query(&sql, [], negotiation_from_row)
    }
}

const CONSUMER_COLUMNS: &str = "consumer_pid, provider_pid, state, provider_address, \
     provider_id, offer, agreement, pending, version";

fn consumer_negotiation_from_row(row: &Row<'_>) -> Result<ConsumerNegotiation, StoreError> {
    let consumer_pid: String = col(row, 0)?;
    let provider_id: String = col(row, 4)?;
    let pending = col::<Option<String>>(row, 7)?
        .map(|p| -> Result<consumer::Outgoing, StoreError> {
            let m: StoredMessage<NegotiationState> = parsed("pending message", &p)?;
            Ok(consumer::Outgoing {
                consumer_pid: consumer_pid.clone(),
                transition: m.transition()?,
                url: m.url,
                status_url: m.status_url,
                body: m.body,
                to: m.to,
                initial: m.initial,
                counter_party: provider_id.clone(),
            })
        })
        .transpose()?;
    Ok(ConsumerNegotiation {
        provider_pid: col(row, 1)?,
        state: parsed_state(col(row, 2)?)?,
        provider_address: col(row, 3)?,
        provider_id,
        offer: parsed("offer", &col::<String>(row, 5)?)?,
        agreement: col::<Option<String>>(row, 6)?
            .map(|a| parsed("agreement", &a))
            .transpose()?,
        pending,
        version: parsed_version(col(row, 8)?)?,
        consumer_pid,
    })
}

impl ConsumerStore for SqliteNegotiations {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerNegotiation>, StoreError> {
        let sql =
            format!("SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation WHERE consumer_pid = ?1");
        Ok(self
            .query(&sql, params![consumer_pid], consumer_negotiation_from_row)?
            .into_iter()
            .next())
    }

    fn insert(&self, negotiation: &ConsumerNegotiation) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO consumer_negotiation ({CONSUMER_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        );
        self.conn()
            .execute(
                &sql,
                params![
                    negotiation.consumer_pid,
                    negotiation.provider_pid,
                    state_text(negotiation.state)?,
                    negotiation.provider_address,
                    negotiation.provider_id,
                    json(&negotiation.offer)?,
                    negotiation.agreement.as_ref().map(json).transpose()?,
                    consumer_pending(negotiation.pending.as_ref())?,
                    version(negotiation.version)?,
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    fn replace_if(
        &self,
        negotiation: &ConsumerNegotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        // compare-and-update in one statement, as for the provider side
        let changed = self
            .conn()
            .execute(
                "UPDATE consumer_negotiation
                 SET provider_pid = ?2, state = ?3, provider_address = ?4, provider_id = ?5,
                     offer = ?6, agreement = ?7, pending = ?8, version = ?9
                 WHERE consumer_pid = ?1 AND version = ?10",
                params![
                    negotiation.consumer_pid,
                    negotiation.provider_pid,
                    state_text(negotiation.state)?,
                    negotiation.provider_address,
                    negotiation.provider_id,
                    json(&negotiation.offer)?,
                    negotiation.agreement.as_ref().map(json).transpose()?,
                    consumer_pending(negotiation.pending.as_ref())?,
                    version(negotiation.version)?,
                    version(expected)?,
                ],
            )
            .map_err(err)?;
        Ok(changed == 1)
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        let sql = format!(
            "SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation
             WHERE ?1 IS NULL OR consumer_pid > ?1
             ORDER BY consumer_pid LIMIT ?2"
        );
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(&sql, params![after, limit], consumer_negotiation_from_row)
    }

    fn pending(&self) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        let sql = format!(
            "SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation WHERE pending IS NOT NULL
             ORDER BY consumer_pid"
        );
        self.query(&sql, [], consumer_negotiation_from_row)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_dsp::negotiation::Agreement;

    /// Brings `conn` up to date, as `open` does.
    fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
        portitor_runtime::sqlite::migrate(conn, MIGRATIONS).map_err(StoreError)
    }

    /// A store on a connection a test prepared.
    fn setup(mut conn: Connection) -> Result<SqliteNegotiations, StoreError> {
        migrate(&mut conn)?;
        Ok(SqliteNegotiations(Mutex::new(conn)))
    }

    fn sample(pid: &str, state: NegotiationState) -> Negotiation {
        Negotiation {
            provider_pid: pid.into(),
            consumer_pid: "urn:uuid:c".into(),
            state,
            callback_address: "https://c.example/cb".into(),
            counter_party: "did:web:c.example".into(),
            offer: serde_json::from_value(serde_json::json!({
                "@id": "o", "@type": "Offer", "target": "d",
                "permission": [{"action": "use", "constraint": [
                    {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2030-01-01T00:00:00Z"}
                ]}]
            }))
            .unwrap(),
            agreement: None,
            pending: None,
            version: 0,
        }
    }

    fn agreement() -> Agreement {
        serde_json::from_value(serde_json::json!({
            "@id": "urn:uuid:a", "@type": "Agreement", "target": "d",
            "assigner": "did:web:p.example", "assignee": "did:web:c.example",
            "timestamp": "2026-11-12T12:00:00Z",
            "permission": [{"action": "use"}]
        }))
        .unwrap()
    }

    #[test]
    fn round_trip_and_compare_and_update() {
        let owned = SqliteNegotiations::in_memory().unwrap();
        let store: &dyn NegotiationStore = &owned;
        assert!(store.get("p").unwrap().is_none());
        let n = sample("p", NegotiationState::Requested);
        store.insert(&n).unwrap();
        assert_eq!(store.get("p").unwrap().unwrap(), n);
        assert!(store.insert(&n).is_err(), "duplicate provider pid");

        let mut agreed = n.clone();
        agreed.state = NegotiationState::Agreed;
        agreed.agreement = Some(agreement());
        agreed.version = 1;
        assert!(!store.replace_if(&agreed, 7).unwrap());
        assert_eq!(store.get("p").unwrap().unwrap(), n);
        assert!(store.replace_if(&agreed, 0).unwrap());
        assert_eq!(store.get("p").unwrap().unwrap(), agreed);
        // the same expected version again: another write came first
        assert!(!store.replace_if(&agreed, 0).unwrap());
        assert!(
            !store
                .replace_if(&sample("q", NegotiationState::Offered), 0)
                .unwrap()
        );
        assert!(store.pending().unwrap().is_empty());

        // a pending message round-trips and is listed
        let mut sending = agreed.clone();
        sending.version = 2;
        sending.pending = Some(provider::Outgoing {
            provider_pid: "p".into(),
            url: "https://c.example/cb/negotiations/urn:uuid:c/events".into(),
            body: serde_json::json!({"eventType": "FINALIZED"}),
            transition: Transition::Finalize,
            to: NegotiationState::Finalized,
            status_url: "https://c.example/cb/negotiations/urn:uuid:c".into(),
            counter_party: "did:web:c.example".into(),
        });
        assert!(store.replace_if(&sending, 1).unwrap());
        assert_eq!(store.get("p").unwrap().unwrap(), sending);
        assert_eq!(store.pending().unwrap(), vec![sending]);
    }

    #[test]
    fn file_survives_reopening() {
        let dir = std::env::temp_dir().join(format!("portitor-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("portitor.db");
        let _ = std::fs::remove_file(&path);
        {
            let store: &dyn NegotiationStore = &SqliteNegotiations::open(&path).unwrap();
            store
                .insert(&sample("p", NegotiationState::Requested))
                .unwrap();
        }
        let owned = SqliteNegotiations::open(&path).unwrap();
        let store: &dyn NegotiationStore = &owned;
        assert_eq!(
            store.get("p").unwrap().unwrap().state,
            NegotiationState::Requested
        );
        drop(owned);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_a_newer_schema_and_garbage() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        let e = setup(conn).unwrap_err();
        assert!(e.0.contains("newer"), "{e}");

        let conn = Connection::open_in_memory().unwrap();
        migrate(&mut Connection::open_in_memory().unwrap()).unwrap();
        let store = setup(conn).unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO negotiation VALUES ('x', 'c', 'NOT_A_STATE', 'u', 'p', '{}', NULL,
                                                 NULL, 0)",
                [],
            )
            .unwrap();
        assert!(
            NegotiationStore::get(&store, "x")
                .unwrap_err()
                .0
                .contains("state")
        );
        let offer = json(&sample("p", NegotiationState::Requested).offer).unwrap();
        for (pid, pending, version, what) in [
            ("y", r#"{"url":"u"}"#, 0, "pending message"),
            (
                "z",
                r#"{"url":"u","statusUrl":"s","body":{},"transition":"jump","to":"REQUESTED"}"#,
                0,
                "transition",
            ),
            ("w", "", -1, "version"),
        ] {
            store
                .conn()
                .execute(
                    "INSERT INTO negotiation VALUES (?1, 'c', 'REQUESTED', 'u', 'p', ?2, NULL,
                                                     NULLIF(?3, ''), ?4)",
                    params![pid, offer, pending, version],
                )
                .unwrap();
            let e = NegotiationStore::get(&store, pid).unwrap_err();
            assert!(e.0.contains(what), "{e}");
        }
    }

    fn consumer_sample(state: NegotiationState) -> ConsumerNegotiation {
        ConsumerNegotiation {
            consumer_pid: "urn:uuid:c".into(),
            provider_pid: None,
            state,
            provider_address: "https://p.example/dsp/2025-1".into(),
            provider_id: "did:web:p.example".into(),
            offer: sample("p", state).offer,
            agreement: None,
            pending: None,
            version: 0,
        }
    }

    #[test]
    fn consumer_round_trip_and_compare_and_update() {
        let store = SqliteNegotiations::in_memory().unwrap();
        let c: &dyn ConsumerStore = &store;
        assert!(c.get("urn:uuid:c").unwrap().is_none());
        let n = consumer_sample(NegotiationState::Requested);
        c.insert(&n).unwrap();
        assert!(c.insert(&n).is_err(), "duplicate consumer pid");
        assert_eq!(c.get("urn:uuid:c").unwrap().unwrap(), n);

        let mut agreed = n.clone();
        agreed.provider_pid = Some("urn:uuid:p".into());
        agreed.state = NegotiationState::Agreed;
        agreed.agreement = Some(agreement());
        agreed.version = 1;
        assert!(!c.replace_if(&agreed, 1).unwrap());
        assert!(c.replace_if(&agreed, 0).unwrap());
        assert_eq!(c.get("urn:uuid:c").unwrap().unwrap(), agreed);
        assert!(c.pending().unwrap().is_empty());
        let mut second = n.clone();
        second.consumer_pid = "urn:uuid:d".into();
        c.insert(&second).unwrap();
        let all = c.list(None, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], agreed);
        assert_eq!(c.list(Some("urn:uuid:c"), 10).unwrap(), vec![second]);
        assert_eq!(c.list(None, 1).unwrap().len(), 1);
        // the first request (no provider pid yet) round-trips and is listed
        let mut first = n.clone();
        first.consumer_pid = "urn:uuid:e".into();
        first.pending = Some(consumer::Outgoing {
            consumer_pid: "urn:uuid:e".into(),
            url: "https://p.example/dsp/2025-1/negotiations/request".into(),
            body: serde_json::json!({"@type": "ContractRequestMessage"}),
            transition: Transition::Request,
            to: NegotiationState::Requested,
            status_url: String::new(),
            initial: true,
            counter_party: "did:web:p.example".into(),
        });
        c.insert(&first).unwrap();
        assert_eq!(c.get("urn:uuid:e").unwrap().unwrap(), first);
        assert_eq!(c.pending().unwrap(), vec![first]);
        // the provider table is separate
        assert!(
            NegotiationStore::get(&store, "urn:uuid:p")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn upgrades_a_version_1_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO negotiation VALUES ('p', 'c', 'REQUESTED', 'u', 'x', ?1, NULL)",
            params![json(&sample("p", NegotiationState::Requested).offer).unwrap()],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
        let store = setup(conn).unwrap();
        let n = NegotiationStore::get(&store, "p").unwrap().unwrap();
        assert_eq!((n.pending, n.version), (None, 0));
        ConsumerStore::insert(&store, &consumer_sample(NegotiationState::Requested)).unwrap();
    }

    /// Runs logged before the bundle harvester (migration 8) were all
    /// dataset runs.
    #[test]
    fn harvest_jobs_from_before_the_kind_are_dataset_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &MIGRATIONS[..7]).unwrap();
        conn.execute(
            "INSERT INTO harvest_job (started_at, finished_at, run_trigger, source, \
             snapshot_sha256, outcome, seen, added, updated, unchanged, withdrawn, rejected, \
             summary) VALUES (1, 2, 'start', 'file', NULL, 'succeeded', 0, 0, 0, 0, 0, 0, '')",
            [],
        )
        .unwrap();
        let store = setup(conn).unwrap();
        let jobs = crate::harvest::DatasetStore::jobs(&store, None, 10).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].record.kind, crate::harvest::JobKind::Datasets);
    }

    /// The agreement lookups of a transfer request (provider) and of a
    /// transfer start (consumer) must use the indexes of migrations 10 and
    /// 11. If a query's expression and its index drift apart, `SQLite`
    /// reads every negotiation again and this test fails.
    #[test]
    fn the_agreement_lookups_use_their_indexes() {
        let store = setup(Connection::open_in_memory().unwrap()).unwrap();
        for (table, index) in [
            ("negotiation", "INDEX negotiation_agreement"),
            (
                "consumer_negotiation",
                "INDEX consumer_negotiation_agreement",
            ),
        ] {
            let sql = format!(
                "EXPLAIN QUERY PLAN SELECT agreement FROM {table}
                   WHERE state = ?1 AND json_extract(agreement, '$.\"@id\"') = ?2"
            );
            let plan: Vec<String> = store
                .conn()
                .prepare(&sql)
                .unwrap()
                .query_map(["FINALIZED", "urn:uuid:a"], |row| row.get(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert!(plan.iter().any(|step| step.contains(index)), "{plan:?}");
        }
        // 0.13: the negotiation behind an agreement, in any state
        // (`store/transfers.rs`, `negotiation_of`), uses the same index
        let plan: Vec<String> = store
            .conn()
            .prepare(
                "EXPLAIN QUERY PLAN SELECT provider_pid FROM negotiation
                   WHERE json_extract(agreement, '$.\"@id\"') = ?1",
            )
            .unwrap()
            .query_map(["urn:uuid:a"], |row| row.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let index = "INDEX negotiation_agreement";
        assert!(plan.iter().any(|step| step.contains(index)), "{plan:?}");
    }

    /// A file written by 0.11 (8 migrations) with negotiations in it, as
    /// provider and as consumer, with and without an agreement:
    /// migrations 10 and 11 add the agreement indexes, every row reads
    /// back as it was, and the agreement lookups find the agreements.
    #[test]
    fn a_file_from_0_11_with_negotiations_gets_the_agreement_indexes() {
        use portitor_dsp::transfer::consumer::HeldAgreements;
        use portitor_dsp::transfer::provider::Agreements;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &MIGRATIONS[..8]).unwrap();
        let written_by_0_11 = SqliteNegotiations(Mutex::new(conn));
        let mut finalized = sample("urn:uuid:p-finalized", NegotiationState::Finalized);
        finalized.agreement = Some(agreement());
        let requested = sample("urn:uuid:p-requested", NegotiationState::Requested);
        NegotiationStore::insert(&written_by_0_11, &finalized).unwrap();
        NegotiationStore::insert(&written_by_0_11, &requested).unwrap();
        let mut held = consumer_sample(NegotiationState::Finalized);
        held.agreement = Some(agreement());
        let mut asked = consumer_sample(NegotiationState::Requested);
        asked.consumer_pid = "urn:uuid:c-requested".into();
        ConsumerStore::insert(&written_by_0_11, &held).unwrap();
        ConsumerStore::insert(&written_by_0_11, &asked).unwrap();

        // 0.12 opens the file
        let store = setup(written_by_0_11.0.into_inner().unwrap()).unwrap();
        let version: i64 = store
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
        for (index, table) in [
            ("negotiation_agreement", "negotiation"),
            ("consumer_negotiation_agreement", "consumer_negotiation"),
        ] {
            let sql = "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = ?1";
            let indexed: String = store.conn().query_row(sql, [index], |r| r.get(0)).unwrap();
            assert_eq!(indexed, table);
        }

        let get = |pid: &str| NegotiationStore::get(&store, pid).unwrap();
        assert_eq!(get("urn:uuid:p-finalized"), Some(finalized));
        assert_eq!(get("urn:uuid:p-requested"), Some(requested));
        let get = |pid: &str| ConsumerStore::get(&store, pid).unwrap();
        assert_eq!(get("urn:uuid:c"), Some(held));
        assert_eq!(get("urn:uuid:c-requested"), Some(asked));

        let found = Agreements::finalized(&store, "urn:uuid:a").unwrap();
        assert_eq!(found, Some(agreement()));
        assert_eq!(Agreements::finalized(&store, "urn:uuid:b").unwrap(), None);
        let found = HeldAgreements::agreement(&store, "urn:uuid:a").unwrap();
        assert_eq!(found, Some(agreement()));
        let provider = HeldAgreements::provider_of(&store, "urn:uuid:a").unwrap();
        assert_eq!(provider.unwrap().id, "did:web:p.example");
        assert_eq!(
            HeldAgreements::provider_of(&store, "urn:uuid:b").unwrap(),
            None
        );

        // opening it once more applies no migration twice
        assert!(setup(store.0.into_inner().unwrap()).is_ok());
    }

    /// A file written by 0.11 (8 migrations) may hold two transfers for
    /// one repeated request (TD-34). Migration 9 must still apply: it adds
    /// the lookup index and keeps both rows; the first one answers.
    #[test]
    fn a_file_with_a_repeated_transfer_request_still_opens() {
        use portitor_dsp::transfer::provider::TransferStore;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &MIGRATIONS[..8]).unwrap();
        for provider_pid in ["urn:uuid:z-first", "urn:uuid:a-second"] {
            conn.execute(
                "INSERT INTO transfer (provider_pid, consumer_pid, state, callback_address, \
                 counter_party, agreement_id, dataset_id, format, pending, version) \
                 VALUES (?1, 'urn:uuid:c', 'STARTED', 'https://c.example/cb', 'did:web:c', \
                 'urn:uuid:a', 'd', 'HttpData-PULL', NULL, 1)",
                params![provider_pid],
            )
            .unwrap();
        }
        let store = setup(conn).unwrap();
        let version: i64 = store
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
        let index: String = store
            .conn()
            .query_row(
                "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = 'transfer_request'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, "transfer");
        // both rows are kept
        assert!(
            TransferStore::get(&store, "urn:uuid:a-second")
                .unwrap()
                .is_some()
        );
        // the one stored first answers a repeated request, and no third
        // row is added
        let found = store.requested("did:web:c", "urn:uuid:c").unwrap().unwrap();
        assert_eq!(found.provider_pid, "urn:uuid:z-first");
        let mut third = found.clone();
        third.provider_pid = "urn:uuid:third".to_owned();
        assert!(!TransferStore::insert(&store, &third).unwrap());
    }

    /// A file written by 1.0 (11 migrations) with a data flow: 1.1 marks
    /// the flow as started before memberships were kept (migration 12),
    /// and a new flow has nothing kept until the transfer keeps it.
    #[test]
    fn a_flow_from_1_0_is_marked_as_started_before_1_1() {
        use crate::flow_membership::MembershipStore as _;
        use crate::flow_membership::StoredMembership;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &MIGRATIONS[..11]).unwrap();
        conn.execute(
            "INSERT INTO data_flow (id, state, dataset_id, token_hash, expires_at) \
             VALUES ('urn:uuid:old', 'STARTED', 'd', 'ab', 1900000000)",
            [],
        )
        .unwrap();
        let store = setup(conn).unwrap();
        assert_eq!(
            store.memberships("urn:uuid:old").unwrap(),
            StoredMembership::StartedBefore1_1
        );
        assert_eq!(
            store.memberships("urn:uuid:new").unwrap(),
            StoredMembership::Missing
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod poison_tests;
