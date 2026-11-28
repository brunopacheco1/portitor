// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the identity component keeps (ADR 0015, "Storage"): the
//! participant's credentials as received, the credential requests it
//! waits on, the offers issuers sent, and the token ids its callers used
//! (TD-09). [`IdentityStore`] is the port; [`DatabaseStore`] the adapter,
//! on PostgreSQL (several replicas) or a `SQLite` file (one process).

use portitor_dcp::credential::Profile;
use portitor_dcp::message::CredentialObject;
use portitor_runtime::component_store::{ComponentSchema, ComponentStore, WithLeaseTable};
use portitor_runtime::database::{DbError, Executor, Migrations, Row, from_db_int, to_db_int};
use portitor_runtime::leases::LeaseStore;
use portitor_runtime::used_tokens::UsedTokenStore;
use portitor_runtime::values;

/// A store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Lets `?` turn a database error into a [`StoreError`] by itself, so the
/// SQL calls below need no `.map_err(...)`.
impl From<DbError> for StoreError {
    fn from(e: DbError) -> Self {
        Self(e.message)
    }
}

/// The columns of a stored credential as received, in the order of
/// the `INSERT` in [`insert_credential`].
const CREDENTIAL_COLUMNS: &str = "id, issuer, types, profile, expires_at, holder_pid, jwt";

/// The columns [`credential_from`] reads, in its order: those received,
/// then its recorded status (1.4, TD-20).
const CREDENTIAL_READ_COLUMNS: &str =
    "id, issuer, types, profile, expires_at, holder_pid, jwt, status_state, status_checked_at";

/// A credential the participant holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredential {
    /// The credential's `id`, or one Portitor gave it if it had none.
    pub id: String,
    /// The issuer's DID.
    pub issuer: String,
    /// Its types, for example `["VerifiableCredential", "MembershipCredential"]`.
    pub types: Vec<String>,
    /// Its DCP profile.
    pub profile: Profile,
    /// When it expires, seconds since 1970, if it does.
    pub expires_at: Option<i64>,
    /// The request it answered.
    pub holder_pid: String,
    /// The credential as received (a JWT).
    pub jwt: String,
    /// What the last check of its status lists found, and when (1.4,
    /// TD-20); none until one found an answer. Written only by
    /// [`IdentityStore::record_credential_status`]; ignored when a
    /// credential is stored.
    pub last_status_check: Option<CheckedStatus>,
}

/// What the status lists of a held credential say (TD-20). A credential
/// without status entries is [`HeldStatus::Valid`] once checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldStatus {
    /// No status bit is set.
    Valid,
    /// Its bit in a `revocation` list is set: it ends for good.
    Revoked,
    /// Its bit in another list is set (`suspension`, or a purpose
    /// Portitor does not know; a verifier refuses either): it may come
    /// back when the issuer clears the bit.
    Suspended,
    /// Its status cannot be checked, and not for a passing reason: a
    /// list is gone (404), signed by another party, or not a status list
    /// for it; its issuer's DID document cannot be used; or it has too
    /// many status entries. A verifier refuses such a credential, and
    /// the whole presentation with it, as it does a revoked one. It may
    /// come back when its lists can be read again.
    Uncheckable,
}

impl HeldStatus {
    /// How it is written: `VALID`, `REVOKED`, `SUSPENDED` or
    /// `UNCHECKABLE`.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Valid => "VALID",
            Self::Revoked => "REVOKED",
            Self::Suspended => "SUSPENDED",
            Self::Uncheckable => "UNCHECKABLE",
        }
    }

    fn from_text(text: &str) -> Result<Self, StoreError> {
        match text {
            "VALID" => Ok(Self::Valid),
            "REVOKED" => Ok(Self::Revoked),
            "SUSPENDED" => Ok(Self::Suspended),
            "UNCHECKABLE" => Ok(Self::Uncheckable),
            other => Err(StoreError(format!("unknown credential status {other}"))),
        }
    }

    /// Whether a credential in this state is shown to verifiers: not when
    /// revoked, suspended or uncheckable, as a verifier would refuse the
    /// whole presentation (DCP v1.0.1, 5.4.3, step 8).
    #[must_use]
    pub fn is_presented(self) -> bool {
        match self {
            Self::Valid => true,
            Self::Revoked | Self::Suspended | Self::Uncheckable => false,
        }
    }
}

/// A recorded status check of a held credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedStatus {
    /// What the status lists said.
    pub state: HeldStatus,
    /// When they were read, seconds since 1970.
    pub checked_at: u64,
}

/// Where a credential request stands. Only a request that is still
/// `Requested` takes a delivery; the issuer's answer, `Issued` or
/// `Rejected`, closes it (ADR 0015). A renewal is a new request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    /// Waiting for the issuer.
    Requested,
    /// The issuer delivered credentials.
    Issued,
    /// The issuer refused.
    Rejected,
}

impl RequestStatus {
    /// How the status is written: `REQUESTED`, `ISSUED` or `REJECTED`.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Requested => "REQUESTED",
            Self::Issued => "ISSUED",
            Self::Rejected => "REJECTED",
        }
    }

    fn from_text(text: &str) -> Result<Self, StoreError> {
        match text {
            "REQUESTED" => Ok(Self::Requested),
            "ISSUED" => Ok(Self::Issued),
            "REJECTED" => Ok(Self::Rejected),
            other => Err(StoreError(format!("unknown request status {other}"))),
        }
    }
}

/// A credential request the participant made, so the Storage API knows
/// which `CredentialMessage` to accept (DCP 6.5: `holderPid`).
///
/// A request is either sent to the issuer by this component (DCP 6.4,
/// with `credential_ids`, a status URL and an access token), or only
/// recorded, for an issuer that is asked out of band (the DCP TCK, the
/// test credential tool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    /// The holder's id of the request.
    pub holder_pid: String,
    /// The issuer that may answer it.
    pub issuer: String,
    /// Where it stands.
    pub status: RequestStatus,
    /// The ids (in the issuer's metadata) of the credentials asked for;
    /// empty for a request that was only recorded.
    pub credential_ids: Vec<String>,
    /// Where the issuer serves the request's status (the `Location` of
    /// its `201 Created`, DCP 6.4 and 6.8), once known.
    pub status_url: Option<String>,
    /// The `jti` of the access token sent to the issuer in the `token`
    /// claim (DCP 6.1); its delivery must carry this token back. None for
    /// a request that was only recorded.
    pub access_token_id: Option<String>,
    /// When the request was made, seconds since 1970 (0 for requests
    /// recorded before 0.6).
    pub created_at: u64,
    /// The issuer's id of the request (`issuerPid`), from its delivery,
    /// once delivered.
    pub issuer_pid: Option<String>,
    /// Why the issuer's answer to the request is not known (no answer in
    /// time, or a server error): the issuer may have taken it, so a
    /// delivery is still accepted. None once the issuer answered.
    pub send_error: Option<String>,
}

impl CredentialRequest {
    /// A request for `issuer`, still open, with nothing sent yet.
    #[must_use]
    pub fn recorded(holder_pid: String, issuer: String, created_at: u64) -> Self {
        Self {
            holder_pid,
            issuer,
            status: RequestStatus::Requested,
            credential_ids: Vec::new(),
            status_url: None,
            access_token_id: None,
            created_at,
            issuer_pid: None,
            send_error: None,
        }
    }
}

/// What became of a delivery given to [`IdentityStore::deliver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// The answer closed the request; its credentials are stored.
    Stored,
    /// The same answer (same `issuerPid` and status) closed the request
    /// before: the issuer sent it again, for example because our first
    /// reply was lost. Nothing is stored twice.
    Repeated,
}

/// An issuer's answer to a request: its id of the request and the
/// status it gave (DCP 6.5, `CredentialMessage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer<'a> {
    /// The issuer's id of the request (`issuerPid`).
    pub issuer_pid: &'a str,
    /// `Issued` or `Rejected`.
    pub status: RequestStatus,
}

/// One credential an issuer offered (DCP 6.6).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOffer {
    /// The issuer's DID.
    pub issuer: String,
    /// The offered credential, as the issuer described it.
    pub credential: CredentialObject,
    /// When the offer arrived, seconds since 1970.
    pub received_at: u64,
}

/// Port: the identity component's storage. It keeps the used token ids
/// ([`UsedTokenStore`]) and the leases ([`LeaseStore`]) too.
pub trait IdentityStore: UsedTokenStore + LeaseStore + Send + Sync {
    /// Records a new request. Returns false if one with the same
    /// `holder_pid` exists already.
    ///
    /// # Errors
    /// The store fails.
    fn add_request(&self, request: &CredentialRequest) -> Result<bool, StoreError>;

    /// The request with this `holder_pid`, if any.
    ///
    /// # Errors
    /// The store fails.
    fn request(&self, holder_pid: &str) -> Result<Option<CredentialRequest>, StoreError>;

    /// Every request, newest first.
    ///
    /// # Errors
    /// The store fails.
    fn requests(&self) -> Result<Vec<CredentialRequest>, StoreError>;

    /// Keeps where the issuer serves the status of request `holder_pid`.
    ///
    /// # Errors
    /// The store fails.
    fn set_status_url(&self, holder_pid: &str, status_url: &str) -> Result<(), StoreError>;

    /// Keeps why the issuer's answer to request `holder_pid` is not
    /// known; the request stays open.
    ///
    /// # Errors
    /// The store fails.
    fn set_send_error(&self, holder_pid: &str, reason: &str) -> Result<(), StoreError>;

    /// Removes request `holder_pid` if it is still open, so a late
    /// delivery for it is refused; used when the issuer refused it, or
    /// was never asked. Returns false if there was no open request.
    ///
    /// # Errors
    /// The store fails.
    fn withdraw_request(&self, holder_pid: &str) -> Result<bool, StoreError>;

    /// Records an issuer's answer (`issuer_pid`, `status`) to a request
    /// that is still [`RequestStatus::Requested`], with the credentials it
    /// delivered, all at once; the answer closes the request. A credential
    /// with the `id` of one already held from the same issuer replaces
    /// it. The same answer again, for a request it closed, is
    /// [`Delivered::Repeated`] and stores nothing.
    ///
    /// # Errors
    /// The store fails, the request is unknown or closed by another
    /// answer, or a credential has the `id` of one from another issuer.
    /// Then nothing is stored.
    fn deliver(
        &self,
        holder_pid: &str,
        answer: Answer<'_>,
        credentials: &[StoredCredential],
    ) -> Result<Delivered, StoreError>;

    /// Every credential held.
    ///
    /// # Errors
    /// The store fails.
    fn credentials(&self) -> Result<Vec<StoredCredential>, StoreError>;

    /// Deletes a credential. Returns false if there was none with `id`.
    ///
    /// # Errors
    /// The store fails.
    fn delete_credential(&self, id: &str) -> Result<bool, StoreError>;

    /// Records what the status lists of credential `id` said (TD-20),
    /// if it is still the credential `jwt`: one replaced or deleted
    /// while its lists were read is left alone. Returns whether it was
    /// recorded.
    ///
    /// # Errors
    /// The store fails.
    fn record_credential_status(
        &self,
        id: &str,
        jwt: &str,
        status: CheckedStatus,
    ) -> Result<bool, StoreError>;

    /// Records offered credentials. An offer of the same credential id by
    /// the same issuer replaces the earlier one, so a repeated offer
    /// changes nothing.
    ///
    /// # Errors
    /// The store fails.
    fn record_offer(
        &self,
        issuer: &str,
        offered: &[CredentialObject],
        received_at: u64,
    ) -> Result<(), StoreError>;

    /// Every offer recorded, newest first.
    ///
    /// # Errors
    /// The store fails.
    fn offers(&self) -> Result<Vec<StoredOffer>, StoreError>;

    /// Replaces the credentials of the last Gaia-X compliance run (those
    /// whose `holder_pid` starts with [`GAIAX_HOLDER_PID_PREFIX`]) with
    /// `credentials`, all at once (ADR 0021). A credential with the `id`
    /// of one held from another issuer is not replaced.
    ///
    /// # Errors
    /// The store fails, or a credential has the `id` of one from another
    /// issuer. Then nothing is changed.
    fn replace_gaiax_credentials(&self, credentials: &[StoredCredential])
    -> Result<(), StoreError>;

    /// Records that a Gaia-X compliance run started at `started_at`, and
    /// returns its id. Only the last [`GAIAX_RUNS_KEPT`] runs are kept.
    ///
    /// # Errors
    /// The store fails.
    fn start_gaiax_run(&self, started_at: u64) -> Result<i64, StoreError>;

    /// Records how run `id` ended.
    ///
    /// # Errors
    /// The store fails.
    fn finish_gaiax_run(
        &self,
        id: i64,
        finished_at: u64,
        outcome: RunOutcome,
        error: Option<&str>,
    ) -> Result<(), StoreError>;

    /// Marks the runs still `RUNNING` as failed at `now`: the process that
    /// ran them stopped. Call it only while holding the lease of the runs
    /// (TD-56): then no other process runs one. Returns how many.
    ///
    /// # Errors
    /// The store fails.
    fn abandon_gaiax_runs(&self, now: u64) -> Result<usize, StoreError>;

    /// The last `limit` Gaia-X compliance runs, newest first.
    ///
    /// # Errors
    /// The store fails.
    fn gaiax_runs(&self, limit: usize) -> Result<Vec<GaiaxRun>, StoreError>;
}

/// The `holder_pid` of the credentials a Gaia-X compliance run stores
/// starts with this, followed by the credential's name (`legal-person`,
/// `terms-and-conditions`, `registration-number`, `compliance`).
pub const GAIAX_HOLDER_PID_PREFIX: &str = "gaiax:";

/// How many Gaia-X compliance runs are kept.
pub const GAIAX_RUNS_KEPT: i64 = 100;

/// How a Gaia-X compliance run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Still running.
    Running,
    /// The compliance credential was received, checked and stored.
    Succeeded,
    /// It failed; the credentials stored before stay.
    Failed,
}

impl RunOutcome {
    /// How the outcome is written: `RUNNING`, `SUCCEEDED` or `FAILED`.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
        }
    }

    fn from_text(text: &str) -> Result<Self, StoreError> {
        match text {
            "RUNNING" => Ok(Self::Running),
            "SUCCEEDED" => Ok(Self::Succeeded),
            "FAILED" => Ok(Self::Failed),
            other => Err(StoreError(format!("unknown run outcome {other}"))),
        }
    }
}

/// One Gaia-X compliance run (ADR 0021).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaiaxRun {
    /// The run's id, counting up.
    pub id: i64,
    /// When it started, seconds since 1970.
    pub started_at: u64,
    /// When it ended, seconds since 1970; none while it runs.
    pub finished_at: Option<u64>,
    /// How it ended.
    pub outcome: RunOutcome,
    /// Why it failed.
    pub error: Option<String>,
}

/// The `SQLite` schema migrations, applied in order (see
/// `portitor_runtime::sqlite`). Never edit one; add a new one, and the
/// same change to [`POSTGRES_MIGRATIONS`].
const SQLITE_MIGRATIONS: &[&str] = &[
    // 0.5: credential requests, credentials and offers (ADR 0015)
    "CREATE TABLE credential_request (
        holder_pid TEXT PRIMARY KEY,
        issuer_did TEXT NOT NULL,
        status     TEXT NOT NULL
    );
    CREATE TABLE credential (
        id         TEXT PRIMARY KEY,
        issuer     TEXT NOT NULL,
        types      TEXT NOT NULL,
        profile    TEXT NOT NULL,
        expires_at INTEGER,
        holder_pid TEXT NOT NULL,
        jwt        TEXT NOT NULL
    );
    CREATE TABLE credential_offer (
        issuer        TEXT NOT NULL,
        credential_id TEXT NOT NULL,
        credential    TEXT NOT NULL,
        received_at   INTEGER NOT NULL,
        PRIMARY KEY (issuer, credential_id)
    )",
    // 0.6: requests sent to the issuer by this component (DCP 6.4): what
    // was asked, where the status is, the access token sent (ADR 0018)
    "ALTER TABLE credential_request ADD COLUMN credential_ids TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE credential_request ADD COLUMN status_url TEXT;
    ALTER TABLE credential_request ADD COLUMN access_token_id TEXT;
    ALTER TABLE credential_request ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE credential_request ADD COLUMN issuer_pid TEXT;
    ALTER TABLE credential_request ADD COLUMN send_error TEXT",
    // 0.8: Gaia-X compliance runs (ADR 0021)
    "CREATE TABLE gaiax_run (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        started_at  INTEGER NOT NULL,
        finished_at INTEGER,
        outcome     TEXT NOT NULL,
        error       TEXT
    )",
    // 1.3: the token ids used by callers, until their token expires
    // (TD-09, `portitor_runtime::used_tokens`)
    "CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at INTEGER NOT NULL
    );
    CREATE INDEX used_token_expires_at ON used_token (expires_at)",
    // 1.3 (loops): the leases of work one process at a time does: the
    // Gaia-X compliance run (ADR 0029, decision 6; TD-56;
    // `portitor_runtime::leases`). `lease_holder`: the id of the process
    // that holds it; `lease_until`: when it ends, in seconds since 1970
    "CREATE TABLE lease (
        kind         TEXT NOT NULL,
        id           TEXT NOT NULL,
        lease_holder TEXT NOT NULL,
        lease_until  INTEGER NOT NULL,
        PRIMARY KEY (kind, id)
    )",
    // 1.4: what the last status check of a held credential found (TD-20):
    // `VALID`, `REVOKED`, `SUSPENDED` or `UNCHECKABLE` (`HeldStatus`), and
    // when, in seconds since 1970; NULL until a check found an answer
    "ALTER TABLE credential ADD COLUMN status_state TEXT;
    ALTER TABLE credential ADD COLUMN status_checked_at INTEGER",
];

/// The PostgreSQL schema migrations, since 1.3. The first one is the
/// schema of 1.3 as a whole: the `SQLite` migrations above, with
/// `BIGINT` for whole numbers and an identity column where `SQLite`
/// counted the rows. Never edit one; add a new one, and the same change
/// to [`SQLITE_MIGRATIONS`].
const POSTGRES_MIGRATIONS: &[&str] = &[
    // 1.3: the schema as a whole
    "CREATE TABLE credential_request (
        holder_pid      TEXT PRIMARY KEY,
        issuer_did      TEXT NOT NULL,
        status          TEXT NOT NULL,
        credential_ids  TEXT NOT NULL DEFAULT '[]',
        status_url      TEXT,
        access_token_id TEXT,
        created_at      BIGINT NOT NULL DEFAULT 0,
        issuer_pid      TEXT,
        send_error      TEXT
    );
    CREATE TABLE credential (
        id         TEXT PRIMARY KEY,
        issuer     TEXT NOT NULL,
        types      TEXT NOT NULL,
        profile    TEXT NOT NULL,
        expires_at BIGINT,
        holder_pid TEXT NOT NULL,
        jwt        TEXT NOT NULL
    );
    CREATE TABLE credential_offer (
        issuer        TEXT NOT NULL,
        credential_id TEXT NOT NULL,
        credential    TEXT NOT NULL,
        received_at   BIGINT NOT NULL,
        PRIMARY KEY (issuer, credential_id)
    );
    CREATE TABLE gaiax_run (
        id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
        started_at  BIGINT NOT NULL,
        finished_at BIGINT,
        outcome     TEXT NOT NULL,
        error       TEXT
    );
    CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at BIGINT NOT NULL
    );
    CREATE INDEX used_token_expires_at ON used_token (expires_at)",
    // 1.3 (loops): the leases of work one process at a time does, as the
    // SQLite migration of the same name
    "CREATE TABLE lease (
        kind         TEXT NOT NULL,
        id           TEXT NOT NULL,
        lease_holder TEXT NOT NULL,
        lease_until  BIGINT NOT NULL,
        PRIMARY KEY (kind, id)
    )",
    // 1.4: the credential's last status check, as the SQLite migration of
    // the same name (TD-20)
    "ALTER TABLE credential ADD COLUMN status_state TEXT;
    ALTER TABLE credential ADD COLUMN status_checked_at BIGINT",
];

/// The identity component's schema, for both databases.
pub const MIGRATIONS: Migrations = Migrations {
    component: "portitor-identity",
    sqlite: SQLITE_MIGRATIONS,
    postgres: POSTGRES_MIGRATIONS,
};

/// The columns of a credential request, in the order of the `INSERT` in
/// [`DatabaseStore::add_request`] and of [`request_from`]'s reads.
const REQUEST_COLUMNS: &str = "holder_pid, issuer_did, status, credential_ids, status_url, \
     access_token_id, created_at, issuer_pid, send_error";

/// Names the identity component's schema ([`MIGRATIONS`]) for the shared
/// [`ComponentStore`].
#[derive(Debug, Clone, Copy)]
pub struct IdentitySchema;

impl ComponentSchema for IdentitySchema {
    const MIGRATIONS: &'static Migrations = &MIGRATIONS;
}

/// The identity's schema has the `lease` table (1.3), so its store is a
/// `LeaseStore`.
impl WithLeaseTable for IdentitySchema {}

/// Adapter: the store in the identity component's database (PostgreSQL, or
/// a `SQLite` file), shared with the health check. `new`, `open`,
/// `in_memory`, `database`, `db` and the used token ids and leases are
/// written once, in `portitor-runtime` ([`ComponentStore`], TD-71); the
/// [`IdentityStore`] methods are below.
pub type DatabaseStore = ComponentStore<IdentitySchema>;

impl IdentityStore for DatabaseStore {
    fn add_request(&self, request: &CredentialRequest) -> Result<bool, StoreError> {
        let credential_ids = to_json(&request.credential_ids)?;
        let added = self.db().execute(
            &format!(
                "INSERT INTO credential_request ({REQUEST_COLUMNS})
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (holder_pid) DO NOTHING"
            ),
            values![
                &request.holder_pid,
                &request.issuer,
                request.status.text(),
                credential_ids,
                request.status_url.as_deref(),
                request.access_token_id.as_deref(),
                to_db_int(request.created_at),
                request.issuer_pid.as_deref(),
                request.send_error.as_deref()
            ],
        )?;
        Ok(added == 1)
    }

    fn request(&self, holder_pid: &str) -> Result<Option<CredentialRequest>, StoreError> {
        let row = self.db().query_one(
            &format!("SELECT {REQUEST_COLUMNS} FROM credential_request WHERE holder_pid = $1"),
            values![holder_pid],
        )?;
        row.as_ref().map(request_from).transpose()
    }

    fn requests(&self) -> Result<Vec<CredentialRequest>, StoreError> {
        let rows = self.db().query(
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM credential_request
                 ORDER BY created_at DESC, holder_pid"
            ),
            values![],
        )?;
        rows.iter().map(request_from).collect()
    }

    fn set_status_url(&self, holder_pid: &str, status_url: &str) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE credential_request SET status_url = $2 WHERE holder_pid = $1",
            values![holder_pid, status_url],
        )?;
        Ok(())
    }

    fn set_send_error(&self, holder_pid: &str, reason: &str) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE credential_request SET send_error = $2 WHERE holder_pid = $1",
            values![holder_pid, reason],
        )?;
        Ok(())
    }

    fn withdraw_request(&self, holder_pid: &str) -> Result<bool, StoreError> {
        let removed = self.db().execute(
            "DELETE FROM credential_request WHERE holder_pid = $1 AND status = $2",
            values![holder_pid, RequestStatus::Requested.text()],
        )?;
        Ok(removed == 1)
    }

    fn deliver(
        &self,
        holder_pid: &str,
        answer: Answer<'_>,
        credentials: &[StoredCredential],
    ) -> Result<Delivered, StoreError> {
        // an `Err` returned inside the transaction writes nothing
        let delivered = self.db().transaction(|tx| {
            // only an open request takes an answer, and only one: a second
            // delivery finds it closed
            let closed = tx.execute(
                "UPDATE credential_request SET status = $2, issuer_pid = $3, send_error = NULL
                     WHERE holder_pid = $1 AND status = $4",
                values![
                    holder_pid,
                    answer.status.text(),
                    answer.issuer_pid,
                    RequestStatus::Requested.text()
                ],
            )?;
            if closed == 0 {
                if closed_by(tx, holder_pid, answer)? {
                    return Ok(Delivered::Repeated);
                }
                return Err(DbError::new(format!(
                    "credential request {holder_pid} is not open"
                )));
            }
            for credential in credentials {
                // a credential id held from another issuer is not replaced
                insert_credential(tx, credential)?;
            }
            Ok(Delivered::Stored)
        })?;
        Ok(delivered)
    }

    fn credentials(&self) -> Result<Vec<StoredCredential>, StoreError> {
        let rows = self.db().query(
            &format!("SELECT {CREDENTIAL_READ_COLUMNS} FROM credential ORDER BY id"),
            values![],
        )?;
        rows.iter().map(credential_from).collect()
    }

    fn delete_credential(&self, id: &str) -> Result<bool, StoreError> {
        let deleted = self
            .db()
            .execute("DELETE FROM credential WHERE id = $1", values![id])?;
        Ok(deleted == 1)
    }

    fn record_credential_status(
        &self,
        id: &str,
        jwt: &str,
        status: CheckedStatus,
    ) -> Result<bool, StoreError> {
        // one statement, guarded by the JWT: a credential replaced since
        // its lists were read is not given the old one's status
        let recorded = self.db().execute(
            "UPDATE credential SET status_state = $3, status_checked_at = $4
                 WHERE id = $1 AND jwt = $2",
            values![id, jwt, status.state.text(), to_db_int(status.checked_at)],
        )?;
        Ok(recorded == 1)
    }

    fn record_offer(
        &self,
        issuer: &str,
        offered: &[CredentialObject],
        received_at: u64,
    ) -> Result<(), StoreError> {
        // the JSON first: the transaction only writes
        let mut rows = Vec::new();
        for credential in offered {
            rows.push((&credential.id, to_json(credential)?));
        }
        self.db().transaction(|tx| {
            for (credential_id, json) in &rows {
                tx.execute(
                    "INSERT INTO credential_offer (issuer, credential_id, credential, received_at)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (issuer, credential_id) DO UPDATE
                         SET credential = excluded.credential, received_at = excluded.received_at",
                    values![issuer, *credential_id, json, to_db_int(received_at)],
                )?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn offers(&self) -> Result<Vec<StoredOffer>, StoreError> {
        let rows = self.db().query(
            "SELECT issuer, credential, received_at FROM credential_offer
                 ORDER BY received_at DESC, issuer, credential_id",
            values![],
        )?;
        let mut found = Vec::new();
        for row in &rows {
            let json: String = row.get(1)?;
            found.push(StoredOffer {
                issuer: row.get(0)?,
                credential: from_json(&json)?,
                received_at: from_db_int(row.get(2)?),
            });
        }
        Ok(found)
    }

    fn replace_gaiax_credentials(
        &self,
        credentials: &[StoredCredential],
    ) -> Result<(), StoreError> {
        let prefix_length = i64::try_from(GAIAX_HOLDER_PID_PREFIX.len()).unwrap_or_default();
        self.db().transaction(|tx| {
            tx.execute(
                "DELETE FROM credential WHERE substr(holder_pid, 1, $2) = $1",
                values![GAIAX_HOLDER_PID_PREFIX, prefix_length],
            )?;
            for credential in credentials {
                insert_credential(tx, credential)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn start_gaiax_run(&self, started_at: u64) -> Result<i64, StoreError> {
        let id = self.db().transaction(|tx| {
            // the database gives the id (counting up); RETURNING reads it
            let row = tx.query_one(
                "INSERT INTO gaiax_run (started_at, outcome) VALUES ($1, $2) RETURNING id",
                values![to_db_int(started_at), RunOutcome::Running.text()],
            )?;
            let id: i64 = row
                .ok_or_else(|| DbError::new("no id for the new run"))?
                .get(0)?;
            tx.execute(
                "DELETE FROM gaiax_run WHERE id <= $1",
                values![id - GAIAX_RUNS_KEPT],
            )?;
            Ok(id)
        })?;
        Ok(id)
    }

    fn finish_gaiax_run(
        &self,
        id: i64,
        finished_at: u64,
        outcome: RunOutcome,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE gaiax_run SET finished_at = $2, outcome = $3, error = $4 WHERE id = $1",
            values![id, to_db_int(finished_at), outcome.text(), error],
        )?;
        Ok(())
    }

    fn abandon_gaiax_runs(&self, now: u64) -> Result<usize, StoreError> {
        let changed = self.db().execute(
            "UPDATE gaiax_run SET finished_at = $1, outcome = $2,
                 error = 'its process stopped' WHERE outcome = $3",
            values![
                to_db_int(now),
                RunOutcome::Failed.text(),
                RunOutcome::Running.text()
            ],
        )?;
        Ok(usize::try_from(changed).unwrap_or(usize::MAX))
    }

    fn gaiax_runs(&self, limit: usize) -> Result<Vec<GaiaxRun>, StoreError> {
        let rows = self.db().query(
            "SELECT id, started_at, finished_at, outcome, error FROM gaiax_run
                 ORDER BY id DESC LIMIT $1",
            values![i64::try_from(limit).unwrap_or(i64::MAX)],
        )?;
        let mut found = Vec::new();
        for row in &rows {
            let finished_at: Option<i64> = row.get(2)?;
            let outcome: String = row.get(3)?;
            found.push(GaiaxRun {
                id: row.get(0)?,
                started_at: from_db_int(row.get(1)?),
                finished_at: finished_at.map(from_db_int),
                outcome: RunOutcome::from_text(&outcome)?,
                error: row.get(4)?,
            });
        }
        Ok(found)
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError(e.to_string()))
}

/// Writes `credential`; one with the same `id` from the same issuer is
/// replaced, one from another issuer is not (an error).
fn insert_credential(tx: &dyn Executor, credential: &StoredCredential) -> Result<(), DbError> {
    let types =
        serde_json::to_string(&credential.types).map_err(|e| DbError::new(e.to_string()))?;
    let written = tx.execute(
        &format!(
            "INSERT INTO credential ({CREDENTIAL_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET types = excluded.types,
                 profile = excluded.profile, expires_at = excluded.expires_at,
                 holder_pid = excluded.holder_pid, jwt = excluded.jwt,
                 status_state = CASE WHEN credential.jwt = excluded.jwt
                     THEN credential.status_state END,
                 status_checked_at = CASE WHEN credential.jwt = excluded.jwt
                     THEN credential.status_checked_at END
             WHERE credential.issuer = excluded.issuer"
        ),
        values![
            &credential.id,
            &credential.issuer,
            types,
            credential.profile.alias(),
            credential.expires_at,
            &credential.holder_pid,
            &credential.jwt
        ],
    )?;
    if written == 0 {
        return Err(DbError::new(format!(
            "credential {} is held from another issuer",
            credential.id
        )));
    }
    Ok(())
}

/// Whether request `holder_pid` was closed by `answer` (the same
/// `issuerPid` and status) before.
fn closed_by(tx: &dyn Executor, holder_pid: &str, answer: Answer<'_>) -> Result<bool, DbError> {
    let found = tx.query_one(
        "SELECT 1 FROM credential_request
             WHERE holder_pid = $1 AND status = $2 AND issuer_pid = $3",
        values![holder_pid, answer.status.text(), answer.issuer_pid],
    )?;
    Ok(found.is_some())
}

/// A credential request from a row with the [`REQUEST_COLUMNS`], in
/// their order.
fn request_from(row: &Row) -> Result<CredentialRequest, StoreError> {
    let status: String = row.get(2)?;
    let credential_ids: String = row.get(3)?;
    Ok(CredentialRequest {
        holder_pid: row.get(0)?,
        issuer: row.get(1)?,
        status: RequestStatus::from_text(&status)?,
        credential_ids: from_json(&credential_ids)?,
        status_url: row.get(4)?,
        access_token_id: row.get(5)?,
        created_at: from_db_int(row.get(6)?),
        issuer_pid: row.get(7)?,
        send_error: row.get(8)?,
    })
}

/// A stored credential from a row with the [`CREDENTIAL_READ_COLUMNS`],
/// in their order.
fn credential_from(row: &Row) -> Result<StoredCredential, StoreError> {
    let types: String = row.get(2)?;
    let profile: String = row.get(3)?;
    let state: Option<String> = row.get(7)?;
    let checked_at: Option<i64> = row.get(8)?;
    // both are written together; none before the first check
    let last_status_check = match (state, checked_at) {
        (Some(state), Some(checked_at)) => Some(CheckedStatus {
            state: HeldStatus::from_text(&state)?,
            checked_at: from_db_int(checked_at),
        }),
        _ => None,
    };
    Ok(StoredCredential {
        id: row.get(0)?,
        issuer: row.get(1)?,
        types: from_json(&types)?,
        profile: Profile::from_alias(&profile)
            .ok_or_else(|| StoreError(format!("unknown profile {profile}")))?,
        expires_at: row.get(4)?,
        holder_pid: row.get(5)?,
        jwt: row.get(6)?,
        last_status_check,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use portitor_runtime::database::{Database, DatabaseSetting};

    use super::*;
    use portitor_runtime::testing::{databases, schema};
    use rusqlite::Connection;
    use std::path::Path;

    fn request(pid: &str) -> CredentialRequest {
        CredentialRequest::recorded(pid.to_owned(), "did:web:issuer.example".to_owned(), 1)
    }

    fn credential(id: &str, issuer: &str) -> StoredCredential {
        StoredCredential {
            id: id.to_owned(),
            issuer: issuer.to_owned(),
            types: vec![
                "VerifiableCredential".to_owned(),
                "MembershipCredential".to_owned(),
            ],
            profile: Profile::Vc11,
            expires_at: Some(2_000_000_000),
            holder_pid: "h-1".to_owned(),
            jwt: "a.b.c".to_owned(),
            last_status_check: None,
        }
    }

    /// The answer `issuer_pid` with `status`.
    fn answer(issuer_pid: &str, status: RequestStatus) -> Answer<'_> {
        Answer { issuer_pid, status }
    }

    fn check(store: &dyn IdentityStore) {
        use RequestStatus::{Issued, Rejected};
        assert!(store.add_request(&request("h-1")).unwrap());
        assert!(!store.add_request(&request("h-1")).unwrap(), "exists");
        assert_eq!(store.request("h-2").unwrap(), None);

        let issuer = "did:web:issuer.example";
        let delivered = store
            .deliver("h-1", answer("i-1", Issued), &[credential("c-1", issuer)])
            .unwrap();
        assert_eq!(delivered, Delivered::Stored);
        let read = store.request("h-1").unwrap().unwrap();
        assert_eq!(read.status, Issued);
        assert_eq!(read.issuer_pid.as_deref(), Some("i-1"));
        assert_eq!(store.credentials().unwrap(), [credential("c-1", issuer)]);

        // the same answer again (a lost reply): acknowledged, nothing
        // stored twice
        let again = store
            .deliver("h-1", answer("i-1", Issued), &[credential("c-2", issuer)])
            .unwrap();
        assert_eq!(again, Delivered::Repeated);
        assert_eq!(store.credentials().unwrap().len(), 1);

        // the answer closed the request: neither another delivery nor a
        // rejection changes it, and nothing more is stored
        for other in [
            answer("i-2", Issued),
            answer("i-2", Rejected),
            answer("i-1", Rejected),
        ] {
            let err = store
                .deliver("h-1", other, &[credential("c-2", issuer)])
                .unwrap_err();
            assert!(err.0.contains("is not open"), "{err}");
        }
        assert_eq!(store.request("h-1").unwrap().unwrap().status, Issued);
        assert_eq!(store.credentials().unwrap().len(), 1);
        let err = store
            .deliver("unknown", answer("i-1", Issued), &[])
            .unwrap_err();
        assert!(err.0.contains("is not open"), "{err}");

        // same id from another issuer: refused, nothing stored, the request
        // stays open
        assert!(store.add_request(&request("h-3")).unwrap());
        let err = store
            .deliver(
                "h-3",
                answer("i-3", Issued),
                &[
                    credential("c-2", issuer),
                    credential("c-1", "did:web:eve.example"),
                ],
            )
            .unwrap_err();
        assert!(err.0.contains("another issuer"));
        assert_eq!(store.credentials().unwrap().len(), 1);
        let open = store.request("h-3").unwrap().unwrap();
        assert_eq!(open.status, RequestStatus::Requested);
        assert_eq!(open.issuer_pid, None);

        // a rejection closes a request too, and clears why the issuer's
        // answer was unknown
        store.set_send_error("h-3", "no answer within 10s").unwrap();
        assert_eq!(
            store.request("h-3").unwrap().unwrap().send_error.as_deref(),
            Some("no answer within 10s")
        );
        store.deliver("h-3", answer("i-3", Rejected), &[]).unwrap();
        let rejected = store.request("h-3").unwrap().unwrap();
        assert_eq!(rejected.status, Rejected);
        assert_eq!(rejected.send_error, None);

        assert!(store.delete_credential("c-1").unwrap());
        assert!(!store.delete_credential("c-1").unwrap());
        check_sent_requests_and_offers(store);
        check_gaiax(store);
    }

    /// Run by [`check`]: Gaia-X credentials and runs.
    fn check_gaiax(store: &dyn IdentityStore) {
        let gaiax = |id: &str, name: &str| StoredCredential {
            holder_pid: format!("{GAIAX_HOLDER_PID_PREFIX}{name}"),
            ..credential(id, "did:web:holder.example")
        };
        let before = store.credentials().unwrap().len();
        store
            .replace_gaiax_credentials(&[gaiax("g-1", "legal-person"), gaiax("g-2", "compliance")])
            .unwrap();
        assert_eq!(store.credentials().unwrap().len(), before + 2);
        // the next run replaces them all, also one whose id changed
        store
            .replace_gaiax_credentials(&[gaiax("g-1", "legal-person"), gaiax("g-3", "compliance")])
            .unwrap();
        let ids: Vec<String> = store
            .credentials()
            .unwrap()
            .into_iter()
            .filter(|c| c.holder_pid.starts_with(GAIAX_HOLDER_PID_PREFIX))
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["g-1", "g-3"]);
        // the id of a credential held from another issuer: refused, and
        // nothing changes
        assert!(store.add_request(&request("h-9")).unwrap());
        let other = credential("c-9", "did:web:issuer.example");
        store
            .deliver("h-9", answer("i-9", RequestStatus::Issued), &[other])
            .unwrap();
        let taken = gaiax("c-9", "legal-person");
        assert!(store.replace_gaiax_credentials(&[taken]).is_err());
        assert_eq!(store.credentials().unwrap().len(), before + 3);
        assert!(store.delete_credential("c-9").unwrap());

        let first = store.start_gaiax_run(100).unwrap();
        store
            .finish_gaiax_run(first, 101, RunOutcome::Failed, Some("HTTP 500"))
            .unwrap();
        let second = store.start_gaiax_run(200).unwrap();
        let runs = store.gaiax_runs(10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[1],
            GaiaxRun {
                id: first,
                started_at: 100,
                finished_at: Some(101),
                outcome: RunOutcome::Failed,
                error: Some("HTTP 500".to_owned()),
            }
        );
        assert_eq!(runs[0].id, second);
        assert_eq!(runs[0].outcome, RunOutcome::Running);
        assert_eq!(store.abandon_gaiax_runs(300).unwrap(), 1);
        let abandoned = &store.gaiax_runs(1).unwrap()[0];
        assert_eq!(abandoned.outcome, RunOutcome::Failed);
        assert_eq!(abandoned.finished_at, Some(300));
        // only the last runs are kept
        for at in 0..120 {
            store.start_gaiax_run(at).unwrap();
        }
        assert_eq!(
            store.gaiax_runs(1_000).unwrap().len(),
            usize::try_from(GAIAX_RUNS_KEPT).unwrap()
        );
    }

    /// Run by [`check`], after its requests `h-1` and `h-3`.
    fn check_sent_requests_and_offers(store: &dyn IdentityStore) {
        let issuer = "did:web:issuer.example";
        // a request sent to the issuer keeps what was asked, the access
        // token's id and, once known, the status URL
        let sent = CredentialRequest {
            credential_ids: vec!["membership".to_owned()],
            access_token_id: Some("jti-1".to_owned()),
            created_at: 5,
            ..request("h-4")
        };
        assert!(store.add_request(&sent).unwrap());
        store
            .set_status_url("h-4", "https://issuer.example/requests/i-4")
            .unwrap();
        let read = store.request("h-4").unwrap().unwrap();
        assert_eq!(
            read,
            CredentialRequest {
                status_url: Some("https://issuer.example/requests/i-4".to_owned()),
                ..sent
            }
        );
        let listed: Vec<String> = store
            .requests()
            .unwrap()
            .into_iter()
            .map(|r| r.holder_pid)
            .collect();
        assert_eq!(listed, ["h-4", "h-1", "h-3"], "newest first");
        // only an open request can be withdrawn
        assert!(!store.withdraw_request("h-1").unwrap());
        assert!(store.withdraw_request("h-4").unwrap());
        assert_eq!(store.request("h-4").unwrap(), None);

        let offered: CredentialObject =
            serde_json::from_value(serde_json::json!({"id": "o-1", "credentialType": "A"}))
                .unwrap();
        store
            .record_offer(issuer, std::slice::from_ref(&offered), 10)
            .unwrap();
        store
            .record_offer(issuer, std::slice::from_ref(&offered), 20)
            .unwrap();
        let offers = store.offers().unwrap();
        assert_eq!(offers.len(), 1, "a repeated offer replaces the first");
        assert_eq!(offers[0].received_at, 20);
        assert_eq!(offers[0].credential, offered);
    }

    #[test]
    fn on_every_database() {
        for db in databases::all(&MIGRATIONS) {
            check(&DatabaseStore::new(Arc::new(db)));
        }
    }

    /// A credential's recorded status (TD-20): written for the JWT that
    /// was checked only, kept when the same JWT is stored again, cleared
    /// when another JWT replaces it.
    #[test]
    fn a_status_is_recorded_for_the_credential_that_was_checked() {
        for db in databases::all(&MIGRATIONS) {
            let store: &dyn IdentityStore = &DatabaseStore::new(Arc::new(db));
            let issuer = "did:web:issuer.example";
            store.add_request(&request("h-1")).unwrap();
            store
                .deliver(
                    "h-1",
                    answer("i-1", RequestStatus::Issued),
                    &[credential("c-1", issuer)],
                )
                .unwrap();
            assert_eq!(store.credentials().unwrap()[0].last_status_check, None);

            let revoked = CheckedStatus {
                state: HeldStatus::Revoked,
                checked_at: 100,
            };
            assert!(
                store
                    .record_credential_status("c-1", "a.b.c", revoked)
                    .unwrap()
            );
            assert_eq!(
                store.credentials().unwrap()[0].last_status_check,
                Some(revoked)
            );
            // another JWT (the credential was replaced meanwhile): not
            // recorded; an unknown id neither
            let valid = CheckedStatus {
                state: HeldStatus::Valid,
                checked_at: 200,
            };
            assert!(
                !store
                    .record_credential_status("c-1", "x.y.z", valid)
                    .unwrap()
            );
            assert!(
                !store
                    .record_credential_status("c-9", "a.b.c", valid)
                    .unwrap()
            );
            assert_eq!(
                store.credentials().unwrap()[0].last_status_check,
                Some(revoked)
            );

            // stored again with the same JWT: the status stays
            store.add_request(&request("h-2")).unwrap();
            store
                .deliver(
                    "h-2",
                    answer("i-2", RequestStatus::Issued),
                    &[credential("c-1", issuer)],
                )
                .unwrap();
            assert_eq!(
                store.credentials().unwrap()[0].last_status_check,
                Some(revoked)
            );
            // replaced by another JWT: not checked yet
            let reissued = StoredCredential {
                jwt: "d.e.f".to_owned(),
                ..credential("c-1", issuer)
            };
            store.add_request(&request("h-3")).unwrap();
            store
                .deliver("h-3", answer("i-3", RequestStatus::Issued), &[reissued])
                .unwrap();
            assert_eq!(store.credentials().unwrap()[0].last_status_check, None);
        }
    }

    /// A caller's token id is taken once, on every database (the table
    /// itself is tested in `portitor_runtime::used_tokens`).
    #[test]
    fn a_token_id_is_used_once() {
        for db in databases::all(&MIGRATIONS) {
            let store: &dyn IdentityStore = &DatabaseStore::new(Arc::new(db));
            assert!(store.use_token_id("j-1", 200, 100).unwrap());
            assert!(!store.use_token_id("j-1", 200, 100).unwrap());
        }
    }

    /// Two processes ("replicas") on one PostgreSQL database: a token id
    /// used on one is refused by the other (TD-09).
    #[test]
    fn a_token_id_used_on_one_replica_is_refused_by_the_other() {
        let Some(url) = databases::postgres_url() else {
            return;
        };
        let setting = DatabaseSetting::Postgres(Box::new(databases::fresh_postgres_settings(&url)));
        let a = DatabaseStore::open(&setting).unwrap();
        let b = DatabaseStore::open(&setting).unwrap();
        assert!(a.use_token_id("j-1", 200, 100).unwrap());
        assert!(!b.use_token_id("j-1", 200, 100).unwrap());
    }

    /// The PostgreSQL schema is the one the `SQLite` migrations end
    /// with: tables, columns, types, NOT NULL, primary keys and unique
    /// indexes.
    #[test]
    fn both_schemas_have_the_same_tables_and_columns() {
        let Some(postgres) = databases::fresh_postgres(&MIGRATIONS) else {
            return;
        };
        let sqlite = Database::in_memory(&MIGRATIONS).unwrap();
        assert_eq!(schema::tables(&postgres), schema::tables(&sqlite));
    }

    /// The file of `path` as the database setting.
    fn file(path: &Path) -> DatabaseSetting {
        DatabaseSetting::SqliteFile(path.to_owned())
    }

    /// The schema of 0.5 (its one migration), with one open request
    /// recorded for an issuer asked out of band.
    fn database_of_0_5(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SQLITE_MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO credential_request (holder_pid, issuer_did, status)
                 VALUES ('h-old', 'did:web:issuer.example', 'REQUESTED')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn a_database_of_0_5_is_brought_up_to_date() {
        let dir = std::env::temp_dir().join(format!(
            "portitor-identity-upgrade-{}",
            portitor_runtime::system::new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.db");
        database_of_0_5(&path);

        let store = DatabaseStore::open(&file(&path)).unwrap();
        // the new columns read their defaults
        let expected =
            CredentialRequest::recorded("h-old".to_owned(), "did:web:issuer.example".to_owned(), 0);
        assert_eq!(store.request("h-old").unwrap().unwrap(), expected);
        assert_eq!(store.requests().unwrap(), [expected]);
        // the request still takes its delivery
        let delivered = store
            .deliver(
                "h-old",
                answer("i-1", RequestStatus::Issued),
                &[credential("c-1", "did:web:issuer.example")],
            )
            .unwrap();
        assert_eq!(delivered, Delivered::Stored);
        drop(store);
        // and the file opens again, at the current version
        let again = DatabaseStore::open(&file(&path)).unwrap();
        assert_eq!(again.credentials().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The migrations of 1.2 (the last release before PostgreSQL).
    const MIGRATIONS_OF_1_2: usize = 3;

    /// The PostgreSQL migrations of 1.3 (the first release with
    /// PostgreSQL).
    const POSTGRES_MIGRATIONS_OF_1_3: usize = 2;
    /// The `SQLite` migrations of 1.3.
    const MIGRATIONS_OF_1_3: usize = 5;

    /// A PostgreSQL database written by 1.3, with a credential, opens
    /// with 1.4: the status columns of 1.4 are added, NULL (never
    /// checked), the credential reads back, and a status is recorded.
    #[test]
    fn a_postgres_database_of_1_3_opens_with_1_4() {
        let Some(url) = databases::postgres_url() else {
            return;
        };
        let setting = DatabaseSetting::Postgres(Box::new(databases::fresh_postgres_settings(&url)));
        let of_1_3 = Migrations {
            component: MIGRATIONS.component,
            sqlite: &SQLITE_MIGRATIONS[..MIGRATIONS_OF_1_3],
            postgres: &POSTGRES_MIGRATIONS[..POSTGRES_MIGRATIONS_OF_1_3],
        };
        let old = Database::open(&setting, &of_1_3).unwrap();
        let status_column = "SELECT column_name FROM information_schema.columns
             WHERE table_name = 'credential' AND column_name = 'status_state'";
        assert!(old.query(status_column, values![]).unwrap().is_empty());
        old.execute(
            "INSERT INTO credential_request (holder_pid, issuer_did, status, credential_ids,
                 status_url, access_token_id, created_at, issuer_pid, send_error)
                 VALUES ('h-1', 'did:web:issuer.example', 'ISSUED', '[\"membership\"]',
                     'https://issuer.example/requests/i-1', 'jti-1', 5, 'i-1', NULL)",
            values![],
        )
        .unwrap();
        old.execute(
            "INSERT INTO credential (id, issuer, types, profile, expires_at, holder_pid, jwt)
                 VALUES ('c-1', 'did:web:issuer.example',
                     '[\"VerifiableCredential\",\"MembershipCredential\"]',
                     'vc11-sl2021/jwt', 2000000000, 'h-1', 'a.b.c')",
            values![],
        )
        .unwrap();
        drop(old);

        let store = DatabaseStore::open(&setting).unwrap();
        let held = store.credentials().unwrap();
        assert_eq!(held, [credential("c-1", "did:web:issuer.example")]);
        assert_eq!(held[0].last_status_check, None);
        let revoked = CheckedStatus {
            state: HeldStatus::Revoked,
            checked_at: 100,
        };
        assert!(
            store
                .record_credential_status("c-1", "a.b.c", revoked)
                .unwrap()
        );
        assert_eq!(
            store.credentials().unwrap()[0].last_status_check,
            Some(revoked)
        );
    }

    /// A file written by 1.2, with a sent request, a credential and a
    /// Gaia-X run that was running when its process stopped, opens with
    /// 1.3: the rows read back, the run is ended as abandoned, a new run
    /// starts, and the leases and token ids of 1.3 work in it.
    #[test]
    fn a_file_of_1_2_opens_and_works() {
        let dir = portitor_runtime::testing::TempDir::new("identity-file-of-1-2");
        let path = dir.path().join("identity.db");
        let conn =
            portitor_runtime::sqlite::open(Some(&path), &SQLITE_MIGRATIONS[..MIGRATIONS_OF_1_2])
                .unwrap();
        conn.execute_batch(
            "INSERT INTO credential_request (holder_pid, issuer_did, status, credential_ids,
                 status_url, access_token_id, created_at, issuer_pid, send_error)
                 VALUES ('h-1', 'did:web:issuer.example', 'ISSUED', '[\"membership\"]',
                     'https://issuer.example/requests/i-1', 'jti-1', 5, 'i-1', NULL);
             INSERT INTO credential (id, issuer, types, profile, expires_at, holder_pid, jwt)
                 VALUES ('c-1', 'did:web:issuer.example',
                     '[\"VerifiableCredential\",\"MembershipCredential\"]',
                     'vc11-sl2021/jwt', 2000000000, 'h-1', 'a.b.c');
             INSERT INTO gaiax_run (started_at, finished_at, outcome, error)
                 VALUES (10, NULL, 'RUNNING', NULL);",
        )
        .unwrap();
        drop(conn);

        let store = DatabaseStore::open(&file(&path)).unwrap();
        let read = store.request("h-1").unwrap().unwrap();
        assert_eq!(read.status, RequestStatus::Issued);
        assert_eq!(read.credential_ids, ["membership"]);
        assert_eq!(read.issuer_pid.as_deref(), Some("i-1"));
        assert_eq!(
            store.credentials().unwrap(),
            [credential("c-1", "did:web:issuer.example")]
        );
        assert_eq!(store.abandon_gaiax_runs(20).unwrap(), 1);
        let next = store.start_gaiax_run(30).unwrap();
        let runs = store.gaiax_runs(10).unwrap();
        assert_eq!(runs[0].id, next);
        assert_eq!(runs[1].outcome, RunOutcome::Failed);
        assert!(store.claim("gaiax", "run", "p-1", 60, 30).unwrap());
        assert!(!store.claim("gaiax", "run", "p-2", 60, 30).unwrap());
        assert!(store.use_token_id("jti-2", 200, 30).unwrap());
    }

    #[test]
    fn in_a_file_that_is_opened_again() {
        let dir = std::env::temp_dir().join(format!(
            "portitor-identity-store-{}",
            portitor_runtime::system::new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.db");
        check(&DatabaseStore::open(&file(&path)).unwrap());
        let again = DatabaseStore::open(&file(&path)).unwrap();
        assert_eq!(again.offers().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod poison_tests;
