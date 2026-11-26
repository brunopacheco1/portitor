// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter: negotiations (as provider and as consumer), transfers (both
//! sides), data flows, the harvest's read copy and push receipts, in the
//! connector's database (`portitor_runtime::database`, ADR 0029):
//! PostgreSQL, which several replicas share, or a `SQLite` file for one
//! process (deprecated since 1.3). Transfers are in `store/transfers.rs`,
//! the other tables in the other files of `store/`.
//!
//! The SQL is written once, for both databases; only the schema differs
//! ([`MIGRATIONS`]). A write of several statements is one
//! `Database::transaction`. Offers and agreements are stored as their DSP
//! JSON, as received or sent.

use std::path::Path;
use std::sync::Arc;

use portitor_dsp::negotiation::consumer::{self, ConsumerNegotiation, ConsumerStore};
use portitor_dsp::negotiation::provider::{self, Negotiation, NegotiationStore, StoreError};
use portitor_dsp::negotiation::{NegotiationState, Transition};
use portitor_runtime::database::{
    Database, DatabaseSetting, DbError, Executor, FromValue, Migrations, Row, Value,
};
use portitor_runtime::values;

mod bundle_in_use;
mod consumer_transfers;
mod data_flows;
mod harvest;
mod push_receipts;
mod transfers;
mod used_tokens;
use serde::{Deserialize, Serialize};
pub use transfers::finalized_agreement;

/// The connector's schema, for each database. A database of another
/// component (identity, issuer) is refused.
pub const MIGRATIONS: Migrations = Migrations {
    component: "portitor-connector",
    sqlite: SQLITE_MIGRATIONS,
    postgres: POSTGRES_MIGRATIONS,
};

/// Schema migrations of the `SQLite` file, applied in order. The schema
/// version is the number of migrations applied (`PRAGMA user_version`).
/// Never edit a migration that has been released; add a new one, here and
/// in [`POSTGRES_MIGRATIONS`].
const SQLITE_MIGRATIONS: &[&str] = &[
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
    // 1.2: each data flow keeps its dataset's source, which the start
    // message names from now on (ADR 0028, step 1; `store/data_flows.rs`).
    // A flow of 1.1 gets the source its dataset has in the harvest's read
    // copy (the `dataset` table of this file) now; a flow whose dataset
    // has none there keeps NULL and is not served, as 1.1 did not serve it
    "ALTER TABLE data_flow ADD COLUMN source TEXT;
     UPDATE data_flow
        SET source = (SELECT upstream FROM dataset WHERE dataset.id = data_flow.dataset_id)",
    // 1.2: each data flow keeps the agreement and the consumer its start
    // message names, for the audit line (ADR 0028, step 1). A flow of 1.1
    // gets them from its transfer as provider (the flow id is the
    // transfer's provider pid) now; a flow without a transfer keeps empty
    // texts, which the audit line of 1.1 wrote for it too
    "ALTER TABLE data_flow ADD COLUMN agreement_id TEXT NOT NULL DEFAULT '';
     ALTER TABLE data_flow ADD COLUMN counter_party_id TEXT NOT NULL DEFAULT '';
     UPDATE data_flow
        SET agreement_id = COALESCE(
                (SELECT transfer.agreement_id FROM transfer
                  WHERE transfer.provider_pid = data_flow.id), ''),
            counter_party_id = COALESCE(
                (SELECT transfer.counter_party FROM transfer
                  WHERE transfer.provider_pid = data_flow.id), '')",
    // 1.2: the data plane step a provider's transfer write decided, run
    // after that write and then emptied (ADR 0028, decision 6; TD-48;
    // `portitor-dsp`, `transfer::provider::flow_steps`). NULL: none waits
    "ALTER TABLE transfer ADD COLUMN flow_step TEXT",
    // 1.2: the lease on a provider's transfer while its data plane step
    // runs, and its state before the write that recorded the step, so any
    // process that shares this file can run the step and report the
    // change (TD-48; `portitor-dsp`, `transfer::provider::flow_steps`).
    // `lease_holder`: the id of the process that holds it; `lease_until`:
    // when it ends, in seconds since 1970 (as `expires_at`). NULL: none.
    // The partial index serves the sweep, which looks for recorded steps
    // every few seconds (`TransferStore::recorded_steps`)
    "ALTER TABLE transfer ADD COLUMN state_before_step TEXT;
     ALTER TABLE transfer ADD COLUMN lease_holder TEXT;
     ALTER TABLE transfer ADD COLUMN lease_until INTEGER;
     CREATE INDEX transfer_recorded_step ON transfer (provider_pid) WHERE flow_step IS NOT NULL",
    // 1.3: the token ids used by callers, until their token expires, so
    // that a token is taken once by every process of the connector
    // (TD-09; `store/used_tokens.rs`, `portitor_runtime::used_tokens`)
    "CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at INTEGER NOT NULL
     );
     CREATE INDEX used_token_expires_at ON used_token (expires_at)",
    // 1.3 (catalog): the bundle version the connector uses, which every
    // process moves to (TD-55; `store/bundle_in_use.rs`). One row, id 1;
    // `issued` is the manifest's issue date. A file of 1.2 has none: the
    // cache folder's `.in-use.json` says which version was in use
    "CREATE TABLE bundle_in_use (
        id      INTEGER PRIMARY KEY CHECK (id = 1),
        version TEXT NOT NULL,
        issued  TEXT NOT NULL
     )",
];

/// The PostgreSQL schema (since 1.3), applied in order as the `SQLite`
/// migrations are. The first one is the schema of all the `SQLite`
/// migrations above at once, with the same tables and columns (checked by
/// `the_two_schemas_have_the_same_tables_and_columns`): `BIGINT` for whole
/// numbers, `TEXT` for text. Never edit a migration that has been
/// released; add a new one, here and in [`SQLITE_MIGRATIONS`].
///
/// Two differences, because a PostgreSQL database starts empty and has no
/// old rows to keep:
///
/// - `transfer_request` is a UNIQUE index: one transfer for each repeated
///   request, which `SQLite` files from before 0.12 cannot promise (see
///   migration 9 above);
/// - PostgreSQL has no `json_extract`: the schema defines one, with the
///   meaning `SQLite` gives it for the paths the connector uses
///   (`'$."@id"'`), so the agreement lookups and their indexes are the same
///   SQL on both databases.
const POSTGRES_MIGRATIONS: &[&str] = &[
    // 1.3: the schema of 1.2 (the 17 SQLite migrations)
    r#"CREATE FUNCTION json_extract(document TEXT, path TEXT) RETURNS TEXT
        LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE
        AS $$ SELECT jsonb_path_query_first(document::jsonb, path::jsonpath) #>> '{}' $$;
    CREATE TABLE negotiation (
        provider_pid     TEXT PRIMARY KEY,
        consumer_pid     TEXT NOT NULL,
        state            TEXT NOT NULL,
        callback_address TEXT NOT NULL,
        counter_party    TEXT NOT NULL,
        offer            TEXT NOT NULL,
        agreement        TEXT,
        pending          TEXT,
        version          BIGINT NOT NULL DEFAULT 0
    );
    CREATE INDEX negotiation_agreement
        ON negotiation (json_extract(agreement, '$."@id"'));
    CREATE TABLE consumer_negotiation (
        consumer_pid     TEXT PRIMARY KEY,
        provider_pid     TEXT,
        state            TEXT NOT NULL,
        provider_address TEXT NOT NULL,
        provider_id      TEXT NOT NULL,
        offer            TEXT NOT NULL,
        agreement        TEXT,
        pending          TEXT,
        version          BIGINT NOT NULL DEFAULT 0
    );
    CREATE INDEX consumer_negotiation_agreement
        ON consumer_negotiation (json_extract(agreement, '$."@id"'));
    CREATE TABLE transfer (
        provider_pid      TEXT PRIMARY KEY,
        consumer_pid      TEXT NOT NULL,
        state             TEXT NOT NULL,
        callback_address  TEXT NOT NULL,
        counter_party     TEXT NOT NULL,
        agreement_id      TEXT NOT NULL,
        dataset_id        TEXT NOT NULL,
        format            TEXT NOT NULL,
        pending           TEXT,
        version           BIGINT NOT NULL DEFAULT 0,
        data_address      TEXT,
        flow_step         TEXT,
        state_before_step TEXT,
        lease_holder      TEXT,
        lease_until       BIGINT
    );
    CREATE UNIQUE INDEX transfer_request ON transfer (counter_party, consumer_pid);
    CREATE INDEX transfer_recorded_step ON transfer (provider_pid) WHERE flow_step IS NOT NULL;
    CREATE TABLE data_flow (
        id               TEXT PRIMARY KEY,
        state            TEXT NOT NULL,
        dataset_id       TEXT NOT NULL,
        token_hash       TEXT NOT NULL,
        expires_at       BIGINT NOT NULL,
        push_address     TEXT,
        source           TEXT,
        agreement_id     TEXT NOT NULL DEFAULT '',
        counter_party_id TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE flow_membership (
        flow_id     TEXT PRIMARY KEY,
        memberships TEXT
    );
    CREATE TABLE consumer_transfer (
        consumer_pid     TEXT PRIMARY KEY,
        provider_pid     TEXT,
        state            TEXT NOT NULL,
        provider_address TEXT NOT NULL,
        provider_id      TEXT NOT NULL,
        agreement_id     TEXT NOT NULL,
        format           TEXT NOT NULL,
        data_address     TEXT,
        pending          TEXT,
        version          BIGINT NOT NULL DEFAULT 0
    );
    CREATE TABLE push_receipt (
        consumer_pid TEXT PRIMARY KEY,
        token_hash   TEXT NOT NULL,
        expires_at   BIGINT NOT NULL,
        object_key   TEXT
    );
    CREATE TABLE dataset (
        id             TEXT PRIMARY KEY,
        entry          TEXT NOT NULL,
        content        TEXT NOT NULL,
        upstream       TEXT,
        content_sha256 TEXT NOT NULL,
        state          TEXT NOT NULL,
        conforms_to    TEXT,
        first_seen     BIGINT NOT NULL,
        last_changed   BIGINT NOT NULL,
        last_job       BIGINT NOT NULL
    );
    CREATE TABLE harvest_job (
        id              BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
        started_at      BIGINT NOT NULL,
        finished_at     BIGINT NOT NULL,
        run_trigger     TEXT NOT NULL,
        source          TEXT NOT NULL,
        snapshot_sha256 TEXT,
        outcome         TEXT NOT NULL,
        seen            BIGINT NOT NULL,
        added           BIGINT NOT NULL,
        updated         BIGINT NOT NULL,
        unchanged       BIGINT NOT NULL,
        withdrawn       BIGINT NOT NULL,
        rejected        BIGINT NOT NULL,
        summary         TEXT NOT NULL,
        kind            TEXT NOT NULL DEFAULT 'datasets'
    );
    CREATE TABLE harvest_error (
        job_id         BIGINT NOT NULL,
        seq            BIGINT NOT NULL,
        entry_position BIGINT,
        dataset_id     TEXT,
        kind           TEXT NOT NULL,
        message        TEXT NOT NULL,
        report         TEXT,
        PRIMARY KEY (job_id, seq)
    );
    CREATE TABLE harvest_state (
        id              BIGINT PRIMARY KEY,
        rules_version   TEXT,
        snapshot_sha256 TEXT
    );
    INSERT INTO harvest_state (id, rules_version, snapshot_sha256) VALUES (1, NULL, NULL)"#,
    // 1.3: the token ids used by callers (TD-09), as SQLite migration 18
    "CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at BIGINT NOT NULL
     );
     CREATE INDEX used_token_expires_at ON used_token (expires_at)",
    // 1.3 (catalog): the bundle version the connector uses (TD-55), as
    // the SQLite migration of the same name
    "CREATE TABLE bundle_in_use (
        id      BIGINT PRIMARY KEY CHECK (id = 1),
        version TEXT NOT NULL,
        issued  TEXT NOT NULL
     )",
];

/// Adapter: every store of the connector, on its database. Cloning it
/// is cheap: the clones share the database.
#[derive(Debug, Clone)]
pub struct ConnectorStore {
    database: Arc<Database>,
}

impl ConnectorStore {
    /// The stores on `database`, opened with [`MIGRATIONS`]
    /// (`Database::open(&setting, &MIGRATIONS)`).
    #[must_use]
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }

    /// Opens (or creates) the `SQLite` file at `path` and brings its
    /// schema up to date; for tests and tools that work on a file.
    ///
    /// # Errors
    ///
    /// The file cannot be opened or created, it is not a `SQLite` database,
    /// or it was written by a newer Portitor version.
    pub fn open_sqlite_file(path: &Path) -> Result<Self, StoreError> {
        let setting = DatabaseSetting::SqliteFile(path.to_path_buf());
        let database = Database::open(&setting, &MIGRATIONS).map_err(err)?;
        Ok(Self::new(Arc::new(database)))
    }

    /// An in-memory `SQLite` database, for tests.
    ///
    /// # Errors
    ///
    /// `SQLite` cannot allocate the database.
    pub fn in_memory() -> Result<Self, StoreError> {
        let database = Database::in_memory(&MIGRATIONS).map_err(err)?;
        Ok(Self::new(Arc::new(database)))
    }

    /// The database the stores are on.
    #[must_use]
    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    /// Runs a `SELECT` and reads each row with `from_row`.
    fn query<T>(
        &self,
        sql: &str,
        values: &[Value],
        from_row: fn(&Row) -> Result<T, StoreError>,
    ) -> Result<Vec<T>, StoreError> {
        // `Executor` is implemented for `&Database`: a reference to the
        // database is what runs statements outside a transaction
        let mut database: &Database = &self.database;
        let rows = database.query(sql, values).map_err(err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(from_row(row)?);
        }
        Ok(out)
    }

    /// [`ConnectorStore::query`] for the first row only, if there is one.
    fn query_first<T>(
        &self,
        sql: &str,
        values: &[Value],
        from_row: fn(&Row) -> Result<T, StoreError>,
    ) -> Result<Option<T>, StoreError> {
        Ok(self.query(sql, values, from_row)?.into_iter().next())
    }

    /// Runs one statement that changes rows; returns how many it changed.
    fn execute(&self, sql: &str, values: &[Value]) -> Result<u64, StoreError> {
        let mut database: &Database = &self.database;
        database.execute(sql, values).map_err(err)
    }

    /// Runs `work` as one transaction (`Database::transaction`). On
    /// PostgreSQL `work` may run more than once, so it must only read and
    /// write through `tx`. It returns the database's own error
    /// (`DbError`), so that a collision with another transaction is seen
    /// and the transaction tried again.
    fn transaction<T>(
        &self,
        work: impl FnMut(&mut dyn Executor) -> Result<T, DbError>,
    ) -> Result<T, StoreError> {
        self.database.transaction(work).map_err(err)
    }
}

#[allow(clippy::needless_pass_by_value)] // used as map_err(err)
fn err(e: DbError) -> StoreError {
    StoreError(e.message)
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
fn col<T: FromValue>(row: &Row, i: usize) -> Result<T, StoreError> {
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

fn negotiation_from_row(row: &Row) -> Result<Negotiation, StoreError> {
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

impl NegotiationStore for ConnectorStore {
    fn get(&self, provider_pid: &str) -> Result<Option<Negotiation>, StoreError> {
        let sql = format!("SELECT {NEGOTIATION_COLUMNS} FROM negotiation WHERE provider_pid = $1");
        self.query_first(&sql, values![provider_pid], negotiation_from_row)
    }

    fn insert(&self, negotiation: &Negotiation) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO negotiation ({NEGOTIATION_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
        );
        self.execute(
            &sql,
            values![
                &negotiation.provider_pid,
                &negotiation.consumer_pid,
                state_text(negotiation.state)?,
                &negotiation.callback_address,
                &negotiation.counter_party,
                json(&negotiation.offer)?,
                negotiation.agreement.as_ref().map(json).transpose()?,
                provider_pending(negotiation.pending.as_ref())?,
                version(negotiation.version)?,
            ],
        )?;
        Ok(())
    }

    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        // compare-and-update in one statement: a concurrent change makes
        // the WHERE clause miss and nothing is written
        let changed = self.execute(
            "UPDATE negotiation
             SET consumer_pid = $2, state = $3, callback_address = $4, counter_party = $5,
                 offer = $6, agreement = $7, pending = $8, version = $9
             WHERE provider_pid = $1 AND version = $10",
            values![
                &negotiation.provider_pid,
                &negotiation.consumer_pid,
                state_text(negotiation.state)?,
                &negotiation.callback_address,
                &negotiation.counter_party,
                json(&negotiation.offer)?,
                negotiation.agreement.as_ref().map(json).transpose()?,
                provider_pending(negotiation.pending.as_ref())?,
                version(negotiation.version)?,
                version(expected)?,
            ],
        )?;
        Ok(changed == 1)
    }

    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        let sql = format!(
            "SELECT {NEGOTIATION_COLUMNS} FROM negotiation WHERE pending IS NOT NULL
             ORDER BY provider_pid"
        );
        self.query(&sql, values![], negotiation_from_row)
    }
}

const CONSUMER_COLUMNS: &str = "consumer_pid, provider_pid, state, provider_address, \
     provider_id, offer, agreement, pending, version";

fn consumer_negotiation_from_row(row: &Row) -> Result<ConsumerNegotiation, StoreError> {
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

impl ConsumerStore for ConnectorStore {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerNegotiation>, StoreError> {
        let sql =
            format!("SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation WHERE consumer_pid = $1");
        self.query_first(&sql, values![consumer_pid], consumer_negotiation_from_row)
    }

    fn insert(&self, negotiation: &ConsumerNegotiation) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO consumer_negotiation ({CONSUMER_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
        );
        self.execute(
            &sql,
            values![
                &negotiation.consumer_pid,
                negotiation.provider_pid.as_deref(),
                state_text(negotiation.state)?,
                &negotiation.provider_address,
                &negotiation.provider_id,
                json(&negotiation.offer)?,
                negotiation.agreement.as_ref().map(json).transpose()?,
                consumer_pending(negotiation.pending.as_ref())?,
                version(negotiation.version)?,
            ],
        )?;
        Ok(())
    }

    fn replace_if(
        &self,
        negotiation: &ConsumerNegotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        // compare-and-update in one statement, as for the provider side
        let changed = self.execute(
            "UPDATE consumer_negotiation
             SET provider_pid = $2, state = $3, provider_address = $4, provider_id = $5,
                 offer = $6, agreement = $7, pending = $8, version = $9
             WHERE consumer_pid = $1 AND version = $10",
            values![
                &negotiation.consumer_pid,
                negotiation.provider_pid.as_deref(),
                state_text(negotiation.state)?,
                &negotiation.provider_address,
                &negotiation.provider_id,
                json(&negotiation.offer)?,
                negotiation.agreement.as_ref().map(json).transpose()?,
                consumer_pending(negotiation.pending.as_ref())?,
                version(negotiation.version)?,
                version(expected)?,
            ],
        )?;
        Ok(changed == 1)
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        // no `after`: from the first. The CAST gives PostgreSQL the type of
        // $1, which it cannot tell from `$1 IS NULL`
        let sql = format!(
            "SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation
             WHERE CAST($1 AS TEXT) IS NULL OR consumer_pid > $1
             ORDER BY consumer_pid LIMIT $2"
        );
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query(&sql, values![after, limit], consumer_negotiation_from_row)
    }

    fn pending(&self) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        let sql = format!(
            "SELECT {CONSUMER_COLUMNS} FROM consumer_negotiation WHERE pending IS NOT NULL
             ORDER BY consumer_pid"
        );
        self.query(&sql, values![], consumer_negotiation_from_row)
    }
}

/// Runs `test` on a store on each database of
/// `portitor_runtime::testing::databases::all`: `SQLite` in memory, and
/// PostgreSQL when there is a test server. Each run gets a new, empty
/// database. A failure names the database it happened on.
#[cfg(test)]
fn on_every_database(test: impl Fn(&ConnectorStore)) {
    for database in portitor_runtime::testing::databases::all(&MIGRATIONS) {
        let name = if database.is_postgres() {
            "PostgreSQL"
        } else {
            "SQLite"
        };
        // printed only when the test fails
        println!("on {name}");
        test(&ConnectorStore::new(Arc::new(database)));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::*;
    use portitor_dsp::negotiation::Agreement;
    use rusqlite::{Connection, params};

    /// How many migrations a file written by 1.0 has.
    const MIGRATIONS_OF_1_0: usize = 11;
    /// How many migrations a file written by 1.1 has.
    const MIGRATIONS_OF_1_1: usize = 13;

    /// Brings `conn` up to date, as `open` does.
    fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
        portitor_runtime::sqlite::migrate(conn, SQLITE_MIGRATIONS).map_err(StoreError)
    }

    /// A store on a `SQLite` connection a test prepared, as it is.
    fn on_connection(conn: Connection) -> ConnectorStore {
        ConnectorStore::new(Arc::new(Database::Sqlite(Mutex::new(conn), None)))
    }

    /// A store on a `SQLite` connection a test prepared, brought up to
    /// date.
    fn setup(mut conn: Connection) -> Result<ConnectorStore, StoreError> {
        migrate(&mut conn)?;
        Ok(on_connection(conn))
    }

    /// The `SQLite` connection under a store, for the tests of files.
    fn sqlite(store: &ConnectorStore) -> MutexGuard<'_, Connection> {
        store.database.sqlite_connection().unwrap()
    }

    /// The `SQLite` connection a store was on, to open it again.
    fn into_connection(store: ConnectorStore) -> Connection {
        match Arc::try_unwrap(store.database).unwrap() {
            Database::Sqlite(conn, _) => conn.into_inner().unwrap(),
            Database::Postgres(_) => panic!("not SQLite"),
        }
    }

    /// Each table and its columns, as `(table, column)` in order, from
    /// `sql`.
    fn columns(database: &Database, sql: &str) -> Vec<(String, String)> {
        let mut database = database;
        let rows = database.query(sql, values![]).unwrap();
        let mut out = Vec::new();
        for row in rows {
            out.push((row.get(0).unwrap(), row.get(1).unwrap()));
        }
        out
    }

    /// The PostgreSQL schema is the `SQLite` one: after the migrations,
    /// both have the same tables with the same columns. Runs when there is
    /// a PostgreSQL test server.
    #[test]
    fn the_two_schemas_have_the_same_tables_and_columns() {
        let Some(postgres) = portitor_runtime::testing::databases::fresh_postgres(&MIGRATIONS)
        else {
            return;
        };
        let sqlite = Database::in_memory(&MIGRATIONS).unwrap();
        let in_sqlite = columns(
            &sqlite,
            "SELECT m.name, p.name FROM sqlite_master AS m, pragma_table_info(m.name) AS p
             WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite%'
             ORDER BY m.name, p.name",
        );
        // `portitor_schema` holds the schema version, which `SQLite` keeps
        // in `PRAGMA user_version` instead
        let in_postgres = columns(
            &postgres,
            "SELECT table_name::text, column_name::text FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name <> 'portitor_schema'
             ORDER BY table_name, column_name",
        );
        assert!(in_sqlite.len() > 90, "{in_sqlite:?}");
        assert_eq!(in_sqlite, in_postgres);
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
        on_every_database(round_trip_and_compare_and_update_on);
    }

    fn round_trip_and_compare_and_update_on(owned: &ConnectorStore) {
        let store: &dyn NegotiationStore = owned;
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
            let store: &dyn NegotiationStore = &ConnectorStore::open_sqlite_file(&path).unwrap();
            store
                .insert(&sample("p", NegotiationState::Requested))
                .unwrap();
        }
        let owned = ConnectorStore::open_sqlite_file(&path).unwrap();
        let store: &dyn NegotiationStore = &owned;
        assert_eq!(
            store.get("p").unwrap().unwrap().state,
            NegotiationState::Requested
        );
        drop(owned);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_a_newer_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        let e = setup(conn).unwrap_err();
        assert!(e.0.contains("newer"), "{e}");
    }

    #[test]
    fn refuses_garbage() {
        on_every_database(refuses_garbage_on);
    }

    fn refuses_garbage_on(store: &ConnectorStore) {
        store
            .execute(
                "INSERT INTO negotiation VALUES ('x', 'c', 'NOT_A_STATE', 'u', 'p', '{}', NULL,
                                                 NULL, 0)",
                values![],
            )
            .unwrap();
        assert!(
            NegotiationStore::get(store, "x")
                .unwrap_err()
                .0
                .contains("state")
        );
        let offer = json(&sample("p", NegotiationState::Requested).offer).unwrap();
        for (pid, pending, version, what) in [
            ("y", r#"{"url":"u"}"#, 0_i64, "pending message"),
            (
                "z",
                r#"{"url":"u","statusUrl":"s","body":{},"transition":"jump","to":"REQUESTED"}"#,
                0,
                "transition",
            ),
            ("w", "", -1, "version"),
        ] {
            store
                .execute(
                    "INSERT INTO negotiation VALUES ($1, 'c', 'REQUESTED', 'u', 'p', $2, NULL,
                                                     NULLIF($3, ''), $4)",
                    values![pid, &offer, pending, version],
                )
                .unwrap();
            let e = NegotiationStore::get(store, pid).unwrap_err();
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
        on_every_database(consumer_round_trip_and_compare_and_update_on);
    }

    fn consumer_round_trip_and_compare_and_update_on(store: &ConnectorStore) {
        let c: &dyn ConsumerStore = store;
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
            NegotiationStore::get(store, "urn:uuid:p")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn upgrades_a_version_1_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SQLITE_MIGRATIONS[0]).unwrap();
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
        assert_eq!(version, i64::try_from(SQLITE_MIGRATIONS.len()).unwrap());
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
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..7]).unwrap();
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
            let plan: Vec<String> = sqlite(&store)
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
        let plan: Vec<String> = sqlite(&store)
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

    /// The same lookups on PostgreSQL use the same indexes, through the
    /// schema's `json_extract`. (With the few rows of a test PostgreSQL
    /// would rather read the table, so that is switched off here.)
    #[test]
    fn the_agreement_lookups_use_their_indexes_on_postgresql() {
        let Some(postgres) = portitor_runtime::testing::databases::fresh_postgres(&MIGRATIONS)
        else {
            return;
        };
        for (table, index) in [
            ("negotiation", "negotiation_agreement"),
            ("consumer_negotiation", "consumer_negotiation_agreement"),
        ] {
            let sql = format!(
                "EXPLAIN SELECT agreement FROM {table}
                   WHERE state = $1 AND json_extract(agreement, '$.\"@id\"') = $2"
            );
            let plan = postgres
                .transaction(|tx| {
                    tx.execute("SET LOCAL enable_seqscan = off", values![])?;
                    tx.query(&sql, values!["FINALIZED", "urn:uuid:a"])
                })
                .unwrap();
            let steps: Vec<String> = plan.iter().map(|row| row.get(0).unwrap()).collect();
            let wanted = format!("Index Scan using {index}");
            assert!(steps.iter().any(|step| step.contains(&wanted)), "{steps:?}");
        }
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
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..8]).unwrap();
        let written_by_0_11 = on_connection(conn);
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
        let store = setup(into_connection(written_by_0_11)).unwrap();
        let version: i64 = sqlite(&store)
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(SQLITE_MIGRATIONS.len()).unwrap());
        for (index, table) in [
            ("negotiation_agreement", "negotiation"),
            ("consumer_negotiation_agreement", "consumer_negotiation"),
        ] {
            let sql = "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = ?1";
            let indexed: String = sqlite(&store)
                .query_row(sql, [index], |r| r.get(0))
                .unwrap();
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
        assert!(setup(into_connection(store)).is_ok());
    }

    /// A file written by 0.11 (8 migrations) may hold two transfers for
    /// one repeated request (TD-34). Migration 9 must still apply: it adds
    /// the lookup index and keeps both rows; the first one answers.
    #[test]
    fn a_file_with_a_repeated_transfer_request_still_opens() {
        use portitor_dsp::transfer::provider::TransferStore;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..8]).unwrap();
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
        let version: i64 = sqlite(&store)
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, i64::try_from(SQLITE_MIGRATIONS.len()).unwrap());
        let index: String = sqlite(&store)
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
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..MIGRATIONS_OF_1_0])
            .unwrap();
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

    /// A file written by 1.1 (13 migrations) with data flows: 1.2 gives
    /// each the source its dataset has in the harvest's read copy
    /// (migration 14). A flow whose dataset has no source there, or is
    /// not there at all, keeps none and is not served.
    #[test]
    fn a_flow_from_1_1_gets_the_source_of_its_dataset() {
        use portitor_dataplane::FlowStore;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..MIGRATIONS_OF_1_1])
            .unwrap();
        for (dataset, upstream) in [
            ("d-url", Some("https://upstream.example/d")),
            ("d-s3", Some("s3://datasets/d.csv")),
            ("d-none", None),
        ] {
            conn.execute(
                "INSERT INTO dataset (id, entry, content, upstream, content_sha256, state, \
                 conforms_to, first_seen, last_changed, last_job) \
                 VALUES (?1, '{}', '{}', ?2, 'ab', 'withdrawn', NULL, 1, 1, 1)",
                params![dataset, upstream],
            )
            .unwrap();
        }
        for (flow, dataset) in [
            ("urn:uuid:url", "d-url"),
            ("urn:uuid:s3", "d-s3"),
            ("urn:uuid:none", "d-none"),
            ("urn:uuid:unknown", "d-unknown"),
        ] {
            conn.execute(
                "INSERT INTO data_flow (id, state, dataset_id, token_hash, expires_at) \
                 VALUES (?1, 'STARTED', ?2, 'ab', 1900000000)",
                params![flow, dataset],
            )
            .unwrap();
        }
        let store = setup(conn).unwrap();
        // `FlowStore::get`, not `store.get`: the store has several `get`
        // methods (one per port it implements), so the port is named
        let source_of = |flow: &str| FlowStore::get(&store, flow).unwrap().unwrap().source;
        assert_eq!(
            source_of("urn:uuid:url").as_deref(),
            Some("https://upstream.example/d")
        );
        assert_eq!(
            source_of("urn:uuid:s3").as_deref(),
            Some("s3://datasets/d.csv")
        );
        assert_eq!(source_of("urn:uuid:none"), None);
        assert_eq!(source_of("urn:uuid:unknown"), None);
    }

    /// A file written by 1.1 (13 migrations) with data flows: 1.2 gives
    /// each the agreement and the consumer of its transfer as provider
    /// (migration 15), for the audit line. A flow without a transfer gets
    /// empty texts, as its audit line had in 1.1.
    #[test]
    fn a_flow_from_1_1_gets_the_parties_of_its_transfer() {
        use portitor_dataplane::FlowStore;

        let mut conn = Connection::open_in_memory().unwrap();
        portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..MIGRATIONS_OF_1_1])
            .unwrap();
        conn.execute(
            "INSERT INTO transfer (provider_pid, consumer_pid, state, callback_address, \
             counter_party, agreement_id, dataset_id, format, pending, version) \
             VALUES ('urn:uuid:p', 'urn:uuid:c', 'STARTED', 'https://c.example/cb', \
             'did:web:c.example', 'urn:uuid:a', 'd', 'HttpData-PULL', NULL, 1)",
            [],
        )
        .unwrap();
        for flow in ["urn:uuid:p", "urn:uuid:alone"] {
            conn.execute(
                "INSERT INTO data_flow (id, state, dataset_id, token_hash, expires_at) \
                 VALUES (?1, 'STARTED', 'd', 'ab', 1900000000)",
                params![flow],
            )
            .unwrap();
        }
        let store = setup(conn).unwrap();
        // the port is named, as above: the store has several `get` methods
        let parties_of = |flow: &str| FlowStore::get(&store, flow).unwrap().unwrap().parties();
        let found = parties_of("urn:uuid:p");
        assert_eq!(
            (found.agreement_id.as_str(), found.counter_party_id.as_str()),
            ("urn:uuid:a", "did:web:c.example")
        );
        let alone = parties_of("urn:uuid:alone");
        assert_eq!(
            (alone.agreement_id.as_str(), alone.counter_party_id.as_str()),
            ("", "")
        );
    }

    /// A file of 1.2 before the lease (16 migrations) with a transfer
    /// whose data plane step is recorded, and one of 1.1 (13 migrations)
    /// with a transfer: migration 17 adds the lease, which both read back
    /// as none, so a recorded step counts as left over and is taken over
    /// at once (TD-48). The sweep's query uses its partial index.
    #[test]
    fn a_recorded_step_from_before_the_lease_reads_as_left_over() {
        use portitor_dsp::transfer::provider::{FlowStep, TransferStore};

        const MIGRATIONS_BEFORE_THE_LEASE: usize = 16;
        for (applied, flow_step) in [
            (MIGRATIONS_OF_1_1, None),
            (MIGRATIONS_BEFORE_THE_LEASE, Some("suspend")),
        ] {
            let mut conn = Connection::open_in_memory().unwrap();
            portitor_runtime::sqlite::migrate(&mut conn, &SQLITE_MIGRATIONS[..applied]).unwrap();
            conn.execute(
                "INSERT INTO transfer (provider_pid, consumer_pid, state, callback_address, \
                 counter_party, agreement_id, dataset_id, format, pending, version) \
                 VALUES ('urn:uuid:p', 'urn:uuid:c', 'SUSPENDED', 'https://c.example/cb', \
                 'did:web:c.example', 'urn:uuid:a', 'd', 'HttpData-PULL', NULL, 3)",
                [],
            )
            .unwrap();
            if let Some(step) = flow_step {
                conn.execute("UPDATE transfer SET flow_step = ?1", [step])
                    .unwrap();
            }
            let store = setup(conn).unwrap();
            let file: &dyn TransferStore = &store;
            let transfer = file.get("urn:uuid:p").unwrap().unwrap();
            assert_eq!((transfer.lease, transfer.state_before_step), (None, None));
            assert_eq!(transfer.version, 3);
            let expected_step = flow_step.and_then(FlowStep::from_name);
            assert_eq!(transfer.flow_step, expected_step);
            assert_eq!(
                file.recorded_steps().unwrap().len(),
                usize::from(flow_step.is_some())
            );
        }

        let store = setup(Connection::open_in_memory().unwrap()).unwrap();
        let plan: Vec<String> = sqlite(&store)
            .prepare(
                "EXPLAIN QUERY PLAN SELECT provider_pid FROM transfer
                   WHERE flow_step IS NOT NULL ORDER BY provider_pid",
            )
            .unwrap()
            .query_map([], |row| row.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let index = "INDEX transfer_recorded_step";
        assert!(plan.iter().any(|step| step.contains(index)), "{plan:?}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod poison_tests;
