// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the issuer keeps (ADR 0018, "Storage"): the registered
//! participants, the credential requests and where they stand, the issued
//! credentials (id, holder, definition, status list position, dates; not
//! the signed JWT), the outbox of deliveries (ADR 0007) with the lease of
//! the process that sends each one (TD-57), the signed status lists
//! (TD-51), and the token ids callers used (TD-09). [`IssuerStore`] is
//! the port; [`DatabaseStore`] the adapter, on PostgreSQL (several
//! replicas) or a `SQLite` file (one process).
//!
//! Which credentials are revoked is the status list: a list is signed
//! from the revoked credentials and stored with each revocation, in one
//! transaction, so the two cannot disagree.

use std::collections::BTreeSet;
use std::sync::Arc;

use portitor_dcp::credential::Profile;
use portitor_dcp::issuance::STATUS_LIST_ENTRIES;
use portitor_dcp::message::RequestStatus;
use portitor_runtime::database::{Database, DatabaseSetting, DbError, Executor, Migrations, Row};
use portitor_runtime::system::random_below;
use portitor_runtime::used_tokens::UsedTokenStore;
use portitor_runtime::values;
use serde_json::{Map, Value};

/// A store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("store: {0}")]
pub struct StoreError(pub String);

/// Lets `?` turn a database error into a [`StoreError`] by itself.
impl From<DbError> for StoreError {
    fn from(e: DbError) -> Self {
        Self(e.message)
    }
}

/// A participant the authority registered: only these get credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    /// The participant's DID.
    pub did: String,
    /// What the authority says about it, for example `{"role":
    /// "provider"}`. Free JSON, copied into the credentials whose
    /// definition names the claim, so it is kept as JSON.
    pub claims: Map<String, Value>,
    /// When it was registered, seconds since 1970.
    pub registered_at: u64,
}

/// A holder's credential request (DCP 6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    /// The issuer's id for the request.
    pub issuer_pid: String,
    /// Who asked: the `sub` of its token.
    pub holder_did: String,
    /// The holder's id for the request.
    pub holder_pid: String,
    /// The ids of the credentials asked for, as in the issuer metadata.
    pub credential_ids: Vec<String>,
    /// The access token the holder's token carried (`token` claim), sent
    /// back with the delivery. Forgotten once the delivery ends.
    pub access_token: Option<String>,
    /// Where it stands: `RECEIVED`, then `ISSUED` or `REJECTED`.
    pub status: RequestStatus,
    /// When it arrived, seconds since 1970.
    pub received_at: u64,
}

/// What became of the delivery of a request's credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    /// Waiting in the outbox, or being retried.
    Pending,
    /// The holder's Storage API took it.
    Delivered,
    /// Every attempt failed; the credentials stay issued.
    Failed,
}

impl DeliveryState {
    /// The text kept in the store and shown by the management API.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Delivered => "DELIVERED",
            Self::Failed => "FAILED",
        }
    }

    fn from_text(text: &str) -> Result<Self, StoreError> {
        match text {
            "PENDING" => Ok(Self::Pending),
            "DELIVERED" => Ok(Self::Delivered),
            "FAILED" => Ok(Self::Failed),
            other => Err(StoreError(format!("unknown delivery state {other}"))),
        }
    }
}

/// A credential the issuer issued. The signed JWT is not kept: the holder
/// has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCredential {
    /// The credential's id (a `urn:uuid:`).
    pub id: String,
    /// The request it answered.
    pub issuer_pid: String,
    /// The holder's DID.
    pub holder_did: String,
    /// The definition it was issued from.
    pub definition_id: String,
    /// Its type, for example `MembershipCredential`.
    pub credential_type: String,
    /// Its profile; each profile has its own status list.
    pub profile: Profile,
    /// Its position in that status list.
    pub status_index: u64,
    /// Valid from, seconds since 1970.
    pub issued_at: u64,
    /// Valid until, seconds since 1970.
    pub expires_at: u64,
    /// When it was revoked, if it was.
    pub revoked_at: Option<u64>,
}

/// An issued credential with the state of its request's delivery, as the
/// management API lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedWithDelivery {
    /// The credential.
    pub credential: IssuedCredential,
    /// What became of its delivery.
    pub delivery: DeliveryState,
}

/// A `CredentialMessage` waiting in the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// The request it answers.
    pub issuer_pid: String,
    /// The holder to send it to.
    pub holder_did: String,
    /// The access token to send back, if the holder gave one.
    pub access_token: Option<String>,
    /// The message, as JSON.
    pub message: String,
    /// How many attempts failed so far.
    pub attempts: u32,
    /// When to try next, seconds since 1970.
    pub next_attempt_at: u64,
}

/// A lease on a delivery while a process sends it (TD-57, as Eclipse
/// EDC's lease columns): who holds it and until when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lease<'a> {
    /// The id of the process that holds it, made at its start.
    pub holder: &'a str,
    /// When it ends, seconds since 1970. A process that stopped while it
    /// held the lease leaves it to end; then another process may take
    /// the delivery.
    pub until: u64,
}

/// What [`IssuerStore::revoke`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revocation {
    /// Revoked now; the list of this profile is signed again and stored.
    Revoked(Profile),
    /// It was revoked before; its list was stored with that revocation.
    AlreadyRevoked(Profile),
    /// No credential with this id.
    NotFound,
}

/// One credential to issue for a request: from which definition, in
/// which profile (each profile has its own status list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The definition's id.
    pub definition_id: String,
    /// The definition's profile.
    pub profile: Profile,
}

/// What [`IssuerStore::issue`] needs besides the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueRules {
    /// The most credentials of one definition a holder may have that are
    /// neither revoked nor expired.
    pub max_active_per_holder: u64,
    /// The time, seconds since 1970: what has expired, and when the
    /// delivery is due.
    pub now: u64,
}

/// What [`IssuerStore::issue`] did. Only `Issued` wrote anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueOutcome {
    /// The request is `ISSUED`, its credentials recorded, its message in
    /// the outbox.
    Issued,
    /// The request is unknown or no longer `RECEIVED`.
    NotReceived,
    /// The holder has [`IssueRules::max_active_per_holder`] active
    /// credentials of this definition already.
    LimitReached(String),
    /// The status list of this profile has no free position.
    ListFull(Profile),
}

/// What [`SignCredentials`] returns: the signed credentials of one
/// request, ready to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRequest {
    /// The credentials to record, one per [`Planned`] credential.
    pub records: Vec<IssuedCredential>,
    /// The `CredentialMessage` that carries them, as JSON, for the outbox.
    pub message_json: String,
}

/// Signs a request's credentials once their status list positions are
/// chosen: gets one position per [`Planned`] credential, in the same
/// order. [`IssuerStore::issue`] calls it inside its transaction.
pub type SignCredentials<'a> = dyn FnMut(&[u64]) -> Result<SignedRequest, String> + 'a;

/// What [`SignStatusList`] returns: a status list credential, signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedStatusList {
    /// The credential, a JWT.
    pub credential: String,
    /// The id of the key that signed it.
    pub kid: String,
}

/// Signs the status list of a profile, given the positions of its
/// revoked credentials. [`IssuerStore::revoke`] and
/// [`IssuerStore::sign_status_list_if_needed`] call it inside their
/// transaction, so the list stored always has every revocation stored
/// with it.
pub type SignStatusList<'a> = dyn FnMut(Profile, &[u64]) -> Result<SignedStatusList, String> + 'a;

/// Port: the issuer's storage. It keeps the used token ids too
/// ([`UsedTokenStore`]).
pub trait IssuerStore: UsedTokenStore + Send + Sync {
    /// Registers a participant. Returns false if the DID is registered
    /// already (nothing changes).
    ///
    /// # Errors
    /// The store fails.
    fn add_participant(&self, participant: &Participant) -> Result<bool, StoreError>;

    /// The participant with this DID, if registered.
    ///
    /// # Errors
    /// The store fails.
    fn participant(&self, did: &str) -> Result<Option<Participant>, StoreError>;

    /// Every registered participant, by DID.
    ///
    /// # Errors
    /// The store fails.
    fn participants(&self) -> Result<Vec<Participant>, StoreError>;

    /// Removes a registration. Issued credentials stay valid until
    /// revoked. Returns false if there was none.
    ///
    /// # Errors
    /// The store fails.
    fn delete_participant(&self, did: &str) -> Result<bool, StoreError>;

    /// Records a new request. Returns false if the same holder made a
    /// request with the same `holder_pid` before (nothing is recorded).
    ///
    /// # Errors
    /// The store fails.
    fn add_request(&self, request: &CredentialRequest) -> Result<bool, StoreError>;

    /// The request with this `issuer_pid`, if any.
    ///
    /// # Errors
    /// The store fails.
    fn request(&self, issuer_pid: &str) -> Result<Option<CredentialRequest>, StoreError>;

    /// The `issuer_pid` of every request still `RECEIVED` (to decide after
    /// a restart).
    ///
    /// # Errors
    /// The store fails.
    fn received_requests(&self) -> Result<Vec<String>, StoreError>;

    /// Rejects a request that is still `RECEIVED`. Returns false if it is
    /// not (unknown, or decided already).
    ///
    /// # Errors
    /// The store fails.
    fn reject(&self, issuer_pid: &str) -> Result<bool, StoreError>;

    /// Issues a request that is still `RECEIVED`, all in one transaction:
    /// checks the holder's limit for each planned credential, picks a
    /// random free position in its status list (W3C Bitstring Status List
    /// v1.0: "implementations SHOULD assign indexes randomly"), has
    /// `sign` sign them, then marks the request `ISSUED`, records the
    /// credentials and puts the message into the outbox, due at
    /// [`IssueRules::now`]. A position is never given twice, and none is
    /// used up when the request is not issued.
    ///
    /// # Errors
    /// The store or `sign` fails; then nothing is written.
    fn issue(
        &self,
        issuer_pid: &str,
        planned: &[Planned],
        rules: IssueRules,
        sign: &mut SignCredentials<'_>,
    ) -> Result<IssueOutcome, StoreError>;

    /// The deliveries due at `now`, oldest first, whether a process holds
    /// them or not. Only a read: to send one, claim it with
    /// [`IssuerStore::claim_due_delivery`].
    ///
    /// # Errors
    /// The store fails.
    fn due_deliveries(&self, now: u64) -> Result<Vec<Delivery>, StoreError>;

    /// Claims the oldest delivery due at `now` that no process holds: a
    /// lease for `lease.holder` until `lease.until` (TD-57). Of several
    /// processes that claim at once, one gets each delivery. A lease that
    /// has ended (its holder stopped before it finished) is taken over.
    /// `None` when no delivery is due and free.
    ///
    /// # Errors
    /// The store fails.
    fn claim_due_delivery(
        &self,
        now: u64,
        lease: Lease<'_>,
    ) -> Result<Option<Delivery>, StoreError>;

    /// When the outbox has work next: the earliest time a waiting
    /// delivery is due and its lease, if any, has ended.
    ///
    /// # Errors
    /// The store fails.
    fn next_delivery_at(&self) -> Result<Option<u64>, StoreError>;

    /// A delivery attempt failed: try again at `next_attempt_at`, and the
    /// lease of `holder` ends. Returns false, and changes nothing, when
    /// `holder` no longer holds the delivery (its lease ended and another
    /// process took it).
    ///
    /// # Errors
    /// The store fails.
    fn retry_later(
        &self,
        issuer_pid: &str,
        holder: &str,
        attempts: u32,
        next_attempt_at: u64,
    ) -> Result<bool, StoreError>;

    /// The delivery ended (`Delivered` or `Failed`): it leaves the outbox
    /// and the access token is forgotten. Returns false, and changes
    /// nothing, when `holder` no longer holds the delivery.
    ///
    /// # Errors
    /// The store fails.
    fn end_delivery(
        &self,
        issuer_pid: &str,
        holder: &str,
        state: DeliveryState,
    ) -> Result<bool, StoreError>;

    /// The issued credentials, newest first; only those of `holder` if
    /// given.
    ///
    /// # Errors
    /// The store fails.
    fn issued(&self, holder: Option<&str>) -> Result<Vec<IssuedWithDelivery>, StoreError>;

    /// Revokes a credential at `now`, and stores the status list of its
    /// profile signed again by `sign`, in one transaction (TD-51; as
    /// Eclipse EDC's `IssuerService` does): every process serves the new
    /// list at once. Revocation cannot be undone.
    ///
    /// # Errors
    /// The store or `sign` fails; then nothing is written.
    fn revoke(
        &self,
        id: &str,
        now: u64,
        sign: &mut SignStatusList<'_>,
    ) -> Result<Revocation, StoreError>;

    /// The signed status list credential of `profile`, as stored by
    /// [`IssuerStore::revoke`] or
    /// [`IssuerStore::sign_status_list_if_needed`].
    ///
    /// # Errors
    /// The store fails.
    fn status_list(&self, profile: Profile) -> Result<Option<SignedStatusList>, StoreError>;

    /// At start: signs the status list of `profile` with `sign` and
    /// stores it, when none is stored or the stored one was signed by
    /// another key than `kid` (a key rotation). Returns whether it signed.
    /// Several processes that start at once store one list.
    ///
    /// # Errors
    /// The store or `sign` fails; then nothing is written.
    fn sign_status_list_if_needed(
        &self,
        profile: Profile,
        kid: &str,
        sign: &mut SignStatusList<'_>,
    ) -> Result<bool, StoreError>;

    /// The positions of the revoked credentials of `profile`.
    ///
    /// # Errors
    /// The store fails.
    fn revoked_indexes(&self, profile: Profile) -> Result<Vec<u64>, StoreError>;
}

/// The `SQLite` schema migrations, applied in order (see
/// `portitor_runtime::sqlite`). Never edit one; add a new one, and the
/// same change to [`POSTGRES_MIGRATIONS`].
const SQLITE_MIGRATIONS: &[&str] = &[
    // 0.6: the issuer (ADR 0018)
    "CREATE TABLE participant (
        did           TEXT PRIMARY KEY,
        claims        TEXT NOT NULL,
        registered_at INTEGER NOT NULL
    );
    CREATE TABLE credential_request (
        issuer_pid     TEXT PRIMARY KEY,
        holder_did     TEXT NOT NULL,
        holder_pid     TEXT NOT NULL,
        credential_ids TEXT NOT NULL,
        access_token   TEXT,
        status         TEXT NOT NULL,
        delivery       TEXT,
        received_at    INTEGER NOT NULL,
        UNIQUE (holder_did, holder_pid)
    );
    CREATE TABLE issued_credential (
        id              TEXT PRIMARY KEY,
        issuer_pid      TEXT NOT NULL REFERENCES credential_request (issuer_pid),
        holder_did      TEXT NOT NULL,
        definition_id   TEXT NOT NULL,
        credential_type TEXT NOT NULL,
        profile         TEXT NOT NULL,
        status_index    INTEGER NOT NULL,
        issued_at       INTEGER NOT NULL,
        expires_at      INTEGER NOT NULL,
        revoked_at      INTEGER,
        UNIQUE (profile, status_index)
    );
    CREATE INDEX issued_credential_holder ON issued_credential (holder_did);
    CREATE TABLE outbox (
        issuer_pid      TEXT PRIMARY KEY REFERENCES credential_request (issuer_pid),
        message         TEXT NOT NULL,
        attempts        INTEGER NOT NULL,
        next_attempt_at INTEGER NOT NULL
    )",
    // 1.3: several processes on one database.
    // `used_token`: the token ids used by callers, until their token
    // expires (TD-09, `portitor_runtime::used_tokens`).
    // `status_list`: the signed status list credential of each profile,
    // stored with each revocation, and `kid`, the key that signed it
    // (TD-51).
    // The lease on a delivery while a process sends it (TD-57):
    // `lease_holder`, the id of that process; `lease_until`, when the
    // lease ends, in seconds since 1970. NULL: none.
    "CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at INTEGER NOT NULL
    );
    CREATE INDEX used_token_expires_at ON used_token (expires_at);
    CREATE TABLE status_list (
        profile    TEXT PRIMARY KEY,
        credential TEXT NOT NULL,
        kid        TEXT NOT NULL
    );
    ALTER TABLE outbox ADD COLUMN lease_holder TEXT;
    ALTER TABLE outbox ADD COLUMN lease_until INTEGER",
];

/// The PostgreSQL schema migrations, since 1.3. The first one is the
/// schema of 1.3 as a whole: the `SQLite` migrations above, with
/// `BIGINT` for whole numbers. Never edit one; add a new one, and the
/// same change to [`SQLITE_MIGRATIONS`].
const POSTGRES_MIGRATIONS: &[&str] = &[
    // 1.3: the schema as a whole
    "CREATE TABLE participant (
        did           TEXT PRIMARY KEY,
        claims        TEXT NOT NULL,
        registered_at BIGINT NOT NULL
    );
    CREATE TABLE credential_request (
        issuer_pid     TEXT PRIMARY KEY,
        holder_did     TEXT NOT NULL,
        holder_pid     TEXT NOT NULL,
        credential_ids TEXT NOT NULL,
        access_token   TEXT,
        status         TEXT NOT NULL,
        delivery       TEXT,
        received_at    BIGINT NOT NULL,
        UNIQUE (holder_did, holder_pid)
    );
    CREATE TABLE issued_credential (
        id              TEXT PRIMARY KEY,
        issuer_pid      TEXT NOT NULL REFERENCES credential_request (issuer_pid),
        holder_did      TEXT NOT NULL,
        definition_id   TEXT NOT NULL,
        credential_type TEXT NOT NULL,
        profile         TEXT NOT NULL,
        status_index    BIGINT NOT NULL,
        issued_at       BIGINT NOT NULL,
        expires_at      BIGINT NOT NULL,
        revoked_at      BIGINT,
        UNIQUE (profile, status_index)
    );
    CREATE INDEX issued_credential_holder ON issued_credential (holder_did);
    CREATE TABLE outbox (
        issuer_pid      TEXT PRIMARY KEY REFERENCES credential_request (issuer_pid),
        message         TEXT NOT NULL,
        attempts        BIGINT NOT NULL,
        next_attempt_at BIGINT NOT NULL,
        lease_holder    TEXT,
        lease_until     BIGINT
    );
    CREATE TABLE used_token (
        token_id   TEXT PRIMARY KEY,
        expires_at BIGINT NOT NULL
    );
    CREATE INDEX used_token_expires_at ON used_token (expires_at);
    CREATE TABLE status_list (
        profile    TEXT PRIMARY KEY,
        credential TEXT NOT NULL,
        kid        TEXT NOT NULL
    )",
];

/// The issuer's schema, for both databases.
pub const MIGRATIONS: Migrations = Migrations {
    component: "portitor-issuer",
    sqlite: SQLITE_MIGRATIONS,
    postgres: POSTGRES_MIGRATIONS,
};

/// The columns of a participant, in the order of [`participant_from`]'s
/// reads.
const PARTICIPANT_COLUMNS: &str = "did, claims, registered_at";

/// The columns of an issued credential, in the order of
/// [`issued_from`]'s reads and of the `INSERT` in [`insert_issued`].
const ISSUED_COLUMNS: &str = "id, issuer_pid, holder_did, definition_id, credential_type, \
     profile, status_index, issued_at, expires_at, revoked_at";

/// The columns of a delivery, in the order of [`delivery_from`]'s reads:
/// the outbox `o` joined with its request `r`.
const DELIVERY_COLUMNS: &str =
    "o.issuer_pid, r.holder_did, r.access_token, o.message, o.attempts, o.next_attempt_at";

/// Adapter: the store in the issuer's database (PostgreSQL, or a
/// `SQLite` file), shared with the health check.
#[derive(Debug, Clone)]
pub struct DatabaseStore(Arc<Database>);

impl DatabaseStore {
    /// The store in `db`, whose schema is [`MIGRATIONS`].
    #[must_use]
    pub fn new(db: Arc<Database>) -> Self {
        Self(db)
    }

    /// Opens (or creates) the database `setting` names and brings its
    /// schema up to date.
    ///
    /// # Errors
    /// The database cannot be opened, or was written by a newer Portitor.
    pub fn open(setting: &DatabaseSetting) -> Result<Self, StoreError> {
        Ok(Self::new(Arc::new(Database::open(setting, &MIGRATIONS)?)))
    }

    /// An in-memory `SQLite` store, for tests.
    ///
    /// # Errors
    /// `SQLite` cannot allocate the database.
    pub fn in_memory() -> Result<Self, StoreError> {
        Ok(Self::new(Arc::new(Database::in_memory(&MIGRATIONS)?)))
    }

    /// The database, for the health check.
    #[must_use]
    pub fn database(&self) -> &Arc<Database> {
        &self.0
    }

    /// The database, to run one statement on (`self.db().execute(...)`).
    fn db(&self) -> &Database {
        &self.0
    }
}

impl UsedTokenStore for DatabaseStore {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        self.db().use_token_id(token_id, expires_at, now)
    }
}

impl IssuerStore for DatabaseStore {
    fn add_participant(&self, participant: &Participant) -> Result<bool, StoreError> {
        let claims = to_json(&participant.claims)?;
        let added = self.db().execute(
            &format!(
                "INSERT INTO participant ({PARTICIPANT_COLUMNS}) VALUES ($1, $2, $3)
                 ON CONFLICT (did) DO NOTHING"
            ),
            values![
                &participant.did,
                claims,
                to_sql_int(participant.registered_at)
            ],
        )?;
        Ok(added == 1)
    }

    fn participant(&self, did: &str) -> Result<Option<Participant>, StoreError> {
        let row = self.db().query_one(
            &format!("SELECT {PARTICIPANT_COLUMNS} FROM participant WHERE did = $1"),
            values![did],
        )?;
        row.as_ref().map(participant_from).transpose()
    }

    fn participants(&self) -> Result<Vec<Participant>, StoreError> {
        let rows = self.db().query(
            &format!("SELECT {PARTICIPANT_COLUMNS} FROM participant ORDER BY did"),
            values![],
        )?;
        rows.iter().map(participant_from).collect()
    }

    fn delete_participant(&self, did: &str) -> Result<bool, StoreError> {
        let deleted = self
            .db()
            .execute("DELETE FROM participant WHERE did = $1", values![did])?;
        Ok(deleted == 1)
    }

    fn add_request(&self, request: &CredentialRequest) -> Result<bool, StoreError> {
        let ids = to_json(&request.credential_ids)?;
        let added = self.db().execute(
            "INSERT INTO credential_request (issuer_pid, holder_did, holder_pid, credential_ids,
                 access_token, status, received_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT DO NOTHING",
            values![
                &request.issuer_pid,
                &request.holder_did,
                &request.holder_pid,
                ids,
                request.access_token.as_deref(),
                request.status.as_str(),
                to_sql_int(request.received_at)
            ],
        )?;
        Ok(added == 1)
    }

    fn request(&self, issuer_pid: &str) -> Result<Option<CredentialRequest>, StoreError> {
        let row = self.db().query_one(
            "SELECT issuer_pid, holder_did, holder_pid, credential_ids, access_token, status,
                 received_at FROM credential_request WHERE issuer_pid = $1",
            values![issuer_pid],
        )?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ids: String = row.get(3)?;
        let status: String = row.get(5)?;
        Ok(Some(CredentialRequest {
            issuer_pid: row.get(0)?,
            holder_did: row.get(1)?,
            holder_pid: row.get(2)?,
            credential_ids: from_json(&ids)?,
            access_token: row.get(4)?,
            status: status_from_text(&status)?,
            received_at: from_sql_int(row.get(6)?),
        }))
    }

    fn received_requests(&self) -> Result<Vec<String>, StoreError> {
        let rows = self.db().query(
            "SELECT issuer_pid FROM credential_request WHERE status = $1 ORDER BY received_at",
            values![RequestStatus::Received.as_str()],
        )?;
        let mut found = Vec::new();
        for row in &rows {
            found.push(row.get(0)?);
        }
        Ok(found)
    }

    fn reject(&self, issuer_pid: &str) -> Result<bool, StoreError> {
        // the access token is no longer needed: nothing is delivered
        let changed = self.db().execute(
            "UPDATE credential_request SET status = $2, access_token = NULL
                 WHERE issuer_pid = $1 AND status = $3",
            values![
                issuer_pid,
                RequestStatus::Rejected.as_str(),
                RequestStatus::Received.as_str()
            ],
        )?;
        Ok(changed == 1)
    }

    fn issue(
        &self,
        issuer_pid: &str,
        planned: &[Planned],
        rules: IssueRules,
        sign: &mut SignCredentials<'_>,
    ) -> Result<IssueOutcome, StoreError> {
        // Everything happens in one transaction: an `Err` (a `?`) writes
        // nothing. On PostgreSQL it may run again when it collided with
        // another process's; signing again is harmless.
        let outcome = self.db().transaction(|tx| {
            // 1. The request must still be RECEIVED.
            let Some(holder_did) = holder_if_received(tx, issuer_pid)? else {
                return Ok(IssueOutcome::NotReceived);
            };
            // 2. The holder must stay within its limit for each definition.
            if let Some(definition_id) = first_limit_reached(tx, &holder_did, planned, rules)? {
                return Ok(IssueOutcome::LimitReached(definition_id));
            }
            // 3. One free status list position per credential.
            let positions = match choose_positions(tx, planned)? {
                Positions::Chosen(positions) => positions,
                Positions::ListFull(profile) => return Ok(IssueOutcome::ListFull(profile)),
            };
            // 4. Sign, still inside the transaction, so a position is used
            // only if the credentials are written.
            let signed = sign(&positions).map_err(|e| DbError::new(format!("signing: {e}")))?;
            // 5. Write: the request is ISSUED, the credentials recorded,
            // the message in the outbox.
            tx.execute(
                "UPDATE credential_request SET status = $2, delivery = $3 WHERE issuer_pid = $1",
                values![
                    issuer_pid,
                    RequestStatus::Issued.as_str(),
                    DeliveryState::Pending.text()
                ],
            )?;
            for credential in &signed.records {
                insert_issued(tx, credential)?;
            }
            tx.execute(
                "INSERT INTO outbox (issuer_pid, message, attempts, next_attempt_at)
                     VALUES ($1, $2, 0, $3)",
                values![issuer_pid, &signed.message_json, to_sql_int(rules.now)],
            )?;
            Ok(IssueOutcome::Issued)
        })?;
        Ok(outcome)
    }

    fn due_deliveries(&self, now: u64) -> Result<Vec<Delivery>, StoreError> {
        let rows = self.db().query(
            &format!(
                "SELECT {DELIVERY_COLUMNS}
                 FROM outbox o JOIN credential_request r ON r.issuer_pid = o.issuer_pid
                 WHERE o.next_attempt_at <= $1 ORDER BY o.next_attempt_at, o.issuer_pid"
            ),
            values![to_sql_int(now)],
        )?;
        rows.iter().map(delivery_from).collect()
    }

    fn claim_due_delivery(
        &self,
        now: u64,
        lease: Lease<'_>,
    ) -> Result<Option<Delivery>, StoreError> {
        let claimed = self.db().transaction(|tx| {
            let Some(issuer_pid) = oldest_free_delivery(tx, now)? else {
                return Ok(None);
            };
            // compare-and-swap: the lease is taken only if the delivery
            // is still due and free. Another process that took it first
            // makes this change no row (or, on PostgreSQL, makes the
            // transaction collide and run again)
            let taken = tx.execute(
                "UPDATE outbox SET lease_holder = $2, lease_until = $3
                     WHERE issuer_pid = $1 AND next_attempt_at <= $4
                     AND (lease_until IS NULL OR lease_until <= $4)",
                values![
                    &issuer_pid,
                    lease.holder,
                    to_sql_int(lease.until),
                    to_sql_int(now)
                ],
            )?;
            if taken == 0 {
                return Ok(None);
            }
            delivery(tx, &issuer_pid)
        })?;
        Ok(claimed)
    }

    fn next_delivery_at(&self) -> Result<Option<u64>, StoreError> {
        // a delivery that another process holds is not due before its
        // lease ends: the later of the two times
        let row = self.db().query_one(
            "SELECT MIN(CASE WHEN lease_until > next_attempt_at THEN lease_until
                             ELSE next_attempt_at END)
                 FROM outbox",
            values![],
        )?;
        let next: Option<i64> = match row {
            Some(row) => row.get(0)?,
            None => None,
        };
        Ok(next.map(from_sql_int))
    }

    fn retry_later(
        &self,
        issuer_pid: &str,
        holder: &str,
        attempts: u32,
        next_attempt_at: u64,
    ) -> Result<bool, StoreError> {
        let changed = self.db().execute(
            "UPDATE outbox SET attempts = $3, next_attempt_at = $4,
                     lease_holder = NULL, lease_until = NULL
                 WHERE issuer_pid = $1 AND lease_holder = $2",
            values![issuer_pid, holder, attempts, to_sql_int(next_attempt_at)],
        )?;
        Ok(changed == 1)
    }

    fn end_delivery(
        &self,
        issuer_pid: &str,
        holder: &str,
        state: DeliveryState,
    ) -> Result<bool, StoreError> {
        let ended = self.db().transaction(|tx| {
            let removed = tx.execute(
                "DELETE FROM outbox WHERE issuer_pid = $1 AND lease_holder = $2",
                values![issuer_pid, holder],
            )?;
            if removed == 0 {
                // another process holds it now: it records the end
                return Ok(false);
            }
            tx.execute(
                "UPDATE credential_request SET delivery = $2, access_token = NULL
                     WHERE issuer_pid = $1",
                values![issuer_pid, state.text()],
            )?;
            Ok(true)
        })?;
        Ok(ended)
    }

    fn issued(&self, holder: Option<&str>) -> Result<Vec<IssuedWithDelivery>, StoreError> {
        let rows = self.db().query(
            "SELECT c.id, c.issuer_pid, c.holder_did, c.definition_id, c.credential_type,
                 c.profile, c.status_index, c.issued_at, c.expires_at, c.revoked_at,
                 r.delivery
                 FROM issued_credential c
                 JOIN credential_request r ON r.issuer_pid = c.issuer_pid
                 WHERE CAST($1 AS TEXT) IS NULL OR c.holder_did = $1
                 ORDER BY c.issued_at DESC, c.id",
            values![holder],
        )?;
        let mut found = Vec::new();
        for row in &rows {
            // `issue` sets `delivery` together with the credentials, so it
            // is never NULL here; should it be, the delivery has not
            // ended, which is what PENDING means
            let delivery: Option<String> = row.get(10)?;
            found.push(IssuedWithDelivery {
                credential: issued_from(row)?,
                delivery: DeliveryState::from_text(delivery.as_deref().unwrap_or("PENDING"))?,
            });
        }
        Ok(found)
    }

    fn revoke(
        &self,
        id: &str,
        now: u64,
        sign: &mut SignStatusList<'_>,
    ) -> Result<Revocation, StoreError> {
        let outcome = self.db().transaction(|tx| {
            let found = tx.query_one(
                "SELECT profile, revoked_at FROM issued_credential WHERE id = $1",
                values![id],
            )?;
            let Some(found) = found else {
                return Ok(Revocation::NotFound);
            };
            let profile = profile_from_text(&found.get::<String>(0)?)?;
            if found.get::<Option<i64>>(1)?.is_some() {
                return Ok(Revocation::AlreadyRevoked(profile));
            }
            tx.execute(
                "UPDATE issued_credential SET revoked_at = $2 WHERE id = $1",
                values![id, to_sql_int(now)],
            )?;
            // the new list, with this revocation, in the same transaction
            store_signed_list(tx, profile, sign)?;
            Ok(Revocation::Revoked(profile))
        })?;
        Ok(outcome)
    }

    fn revoked_indexes(&self, profile: Profile) -> Result<Vec<u64>, StoreError> {
        Ok(revoked_positions(&mut self.db(), profile)?)
    }

    fn status_list(&self, profile: Profile) -> Result<Option<SignedStatusList>, StoreError> {
        let row = self.db().query_one(
            "SELECT credential, kid FROM status_list WHERE profile = $1",
            values![profile.alias()],
        )?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(SignedStatusList {
            credential: row.get(0)?,
            kid: row.get(1)?,
        }))
    }

    fn sign_status_list_if_needed(
        &self,
        profile: Profile,
        kid: &str,
        sign: &mut SignStatusList<'_>,
    ) -> Result<bool, StoreError> {
        let signed = self.db().transaction(|tx| {
            let stored = tx.query_one(
                "SELECT kid FROM status_list WHERE profile = $1",
                values![profile.alias()],
            )?;
            // the list another process stored is just as good, if it was
            // signed with this key
            if let Some(stored) = stored
                && stored.get::<String>(0)? == kid
            {
                return Ok(false);
            }
            store_signed_list(tx, profile, sign)?;
            Ok(true)
        })?;
        Ok(signed)
    }
}

/// Signs the status list of `profile` from the revoked credentials, and
/// stores it in place of the one before.
fn store_signed_list(
    tx: &mut dyn Executor,
    profile: Profile,
    sign: &mut SignStatusList<'_>,
) -> Result<(), DbError> {
    let revoked = revoked_positions(tx, profile)?;
    let signed = sign(profile, &revoked)
        .map_err(|e| DbError::new(format!("signing the status list: {e}")))?;
    tx.execute(
        "INSERT INTO status_list (profile, credential, kid) VALUES ($1, $2, $3)
             ON CONFLICT (profile) DO UPDATE
                 SET credential = excluded.credential, kid = excluded.kid",
        values![profile.alias(), signed.credential, signed.kid],
    )?;
    Ok(())
}

/// The positions of the revoked credentials of `profile`, in order.
fn revoked_positions(tx: &mut dyn Executor, profile: Profile) -> Result<Vec<u64>, DbError> {
    let rows = tx.query(
        "SELECT status_index FROM issued_credential
             WHERE profile = $1 AND revoked_at IS NOT NULL ORDER BY status_index",
        values![profile.alias()],
    )?;
    let mut found = Vec::new();
    for row in &rows {
        found.push(from_sql_int(row.get(0)?));
    }
    Ok(found)
}

/// The `issuer_pid` of the oldest delivery due at `now` whose lease, if
/// any, has ended.
fn oldest_free_delivery(tx: &mut dyn Executor, now: u64) -> Result<Option<String>, DbError> {
    let row = tx.query_one(
        "SELECT issuer_pid FROM outbox
             WHERE next_attempt_at <= $1 AND (lease_until IS NULL OR lease_until <= $1)
             ORDER BY next_attempt_at, issuer_pid LIMIT 1",
        values![to_sql_int(now)],
    )?;
    row.map(|row| row.get(0)).transpose()
}

/// The delivery of request `issuer_pid`, if it is in the outbox.
fn delivery(tx: &mut dyn Executor, issuer_pid: &str) -> Result<Option<Delivery>, DbError> {
    let row = tx.query_one(
        &format!(
            "SELECT {DELIVERY_COLUMNS}
             FROM outbox o JOIN credential_request r ON r.issuer_pid = o.issuer_pid
             WHERE o.issuer_pid = $1"
        ),
        values![issuer_pid],
    )?;
    row.as_ref()
        .map(delivery_from)
        .transpose()
        .map_err(|e| DbError::new(e.0))
}

/// A delivery from a row with the [`DELIVERY_COLUMNS`], in their order.
fn delivery_from(row: &Row) -> Result<Delivery, StoreError> {
    let attempts: i64 = row.get(4)?;
    Ok(Delivery {
        issuer_pid: row.get(0)?,
        holder_did: row.get(1)?,
        access_token: row.get(2)?,
        message: row.get(3)?,
        attempts: u32::try_from(attempts).unwrap_or(u32::MAX),
        next_attempt_at: from_sql_int(row.get(5)?),
    })
}

/// Writes one issued credential.
fn insert_issued(tx: &mut dyn Executor, credential: &IssuedCredential) -> Result<(), DbError> {
    tx.execute(
        &format!(
            "INSERT INTO issued_credential ({ISSUED_COLUMNS})
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        ),
        values![
            &credential.id,
            &credential.issuer_pid,
            &credential.holder_did,
            &credential.definition_id,
            &credential.credential_type,
            credential.profile.alias(),
            to_sql_int(credential.status_index),
            to_sql_int(credential.issued_at),
            to_sql_int(credential.expires_at),
            credential.revoked_at.map(to_sql_int)
        ],
    )?;
    Ok(())
}

/// The holder of request `issuer_pid` if the request is still `RECEIVED`;
/// `None` if it is unknown or decided already.
fn holder_if_received(tx: &mut dyn Executor, issuer_pid: &str) -> Result<Option<String>, DbError> {
    let row = tx.query_one(
        "SELECT holder_did FROM credential_request WHERE issuer_pid = $1 AND status = $2",
        values![issuer_pid, RequestStatus::Received.as_str()],
    )?;
    row.map(|row| row.get(0)).transpose()
}

/// The first planned definition of which the holder already has
/// [`IssueRules::max_active_per_holder`] active credentials, if any.
fn first_limit_reached(
    tx: &mut dyn Executor,
    holder_did: &str,
    planned: &[Planned],
    rules: IssueRules,
) -> Result<Option<String>, DbError> {
    for credential in planned {
        let active = active_credentials(tx, holder_did, &credential.definition_id, rules.now)?;
        if active >= rules.max_active_per_holder {
            return Ok(Some(credential.definition_id.clone()));
        }
    }
    Ok(None)
}

/// What [`choose_positions`] found.
enum Positions {
    /// One free position per planned credential, in the same order.
    Chosen(Vec<u64>),
    /// The status list of this profile has no free position.
    ListFull(Profile),
}

/// One free status list position per planned credential.
fn choose_positions(tx: &mut dyn Executor, planned: &[Planned]) -> Result<Positions, DbError> {
    let mut positions: Vec<u64> = Vec::new();
    for credential in planned {
        let taken_here: Vec<u64> = positions_in(planned, &positions, credential.profile);
        match free_position(tx, credential.profile, &taken_here, STATUS_LIST_ENTRIES)? {
            Some(position) => positions.push(position),
            None => return Ok(Positions::ListFull(credential.profile)),
        }
    }
    Ok(Positions::Chosen(positions))
}

/// How many credentials of `definition_id` the holder has that are
/// neither revoked nor expired at `now`.
fn active_credentials(
    tx: &mut dyn Executor,
    holder_did: &str,
    definition_id: &str,
    now: u64,
) -> Result<u64, DbError> {
    let row = tx.query_one(
        "SELECT COUNT(*) FROM issued_credential
             WHERE holder_did = $1 AND definition_id = $2
             AND revoked_at IS NULL AND expires_at > $3",
        values![holder_did, definition_id, to_sql_int(now)],
    )?;
    let count: i64 = match row {
        Some(row) => row.get(0)?,
        None => 0,
    };
    Ok(from_sql_int(count))
}

/// The positions already chosen in this request for credentials of
/// `profile` (two definitions may share a profile, and so a list).
fn positions_in(planned: &[Planned], chosen: &[u64], profile: Profile) -> Vec<u64> {
    let mut taken = Vec::new();
    for (credential, position) in planned.iter().zip(chosen) {
        if credential.profile == profile {
            taken.push(*position);
        }
    }
    taken
}

/// How many random positions are tried before the free ones are counted.
/// Until a list is nearly full, one or two tries find a free position.
const RANDOM_TRIES: usize = 32;

/// A random free position below `entries` in the status list of
/// `profile`, other than `taken_here`; `None` if there is none.
fn free_position(
    tx: &mut dyn Executor,
    profile: Profile,
    taken_here: &[u64],
    entries: u64,
) -> Result<Option<u64>, DbError> {
    for _ in 0..RANDOM_TRIES {
        let candidate = random_below(entries);
        if !taken_here.contains(&candidate) && !is_recorded(tx, profile, candidate)? {
            return Ok(Some(candidate));
        }
    }
    // the list is nearly full (or unlucky): read every taken position and
    // pick one of the free ones at random
    let mut taken: BTreeSet<u64> = taken_here.iter().copied().collect();
    let rows = tx.query(
        "SELECT status_index FROM issued_credential WHERE profile = $1",
        values![profile.alias()],
    )?;
    for row in &rows {
        taken.insert(from_sql_int(row.get(0)?));
    }
    let free = entries.saturating_sub(u64::try_from(taken.len()).unwrap_or(u64::MAX));
    if free == 0 {
        return Ok(None);
    }
    Ok(nth_free(&taken, entries, random_below(free)))
}

/// Whether a credential of `profile` has `position` in its status list.
fn is_recorded(tx: &mut dyn Executor, profile: Profile, position: u64) -> Result<bool, DbError> {
    let found = tx.query_one(
        "SELECT 1 FROM issued_credential WHERE profile = $1 AND status_index = $2",
        values![profile.alias(), to_sql_int(position)],
    )?;
    Ok(found.is_some())
}

/// The `n`th position (from 0) below `entries` that is not in `taken`.
fn nth_free(taken: &BTreeSet<u64>, entries: u64, n: u64) -> Option<u64> {
    let mut skipped = 0;
    for position in 0..entries {
        if taken.contains(&position) {
            continue;
        }
        if skipped == n {
            return Some(position);
        }
        skipped += 1;
    }
    None
}

/// An issued credential from a row with the [`ISSUED_COLUMNS`], in their
/// order.
fn issued_from(row: &Row) -> Result<IssuedCredential, StoreError> {
    let profile: String = row.get(5)?;
    let revoked_at: Option<i64> = row.get(9)?;
    Ok(IssuedCredential {
        id: row.get(0)?,
        issuer_pid: row.get(1)?,
        holder_did: row.get(2)?,
        definition_id: row.get(3)?,
        credential_type: row.get(4)?,
        profile: profile_from_text(&profile)?,
        status_index: from_sql_int(row.get(6)?),
        issued_at: from_sql_int(row.get(7)?),
        expires_at: from_sql_int(row.get(8)?),
        revoked_at: revoked_at.map(from_sql_int),
    })
}

/// A participant from a row with the [`PARTICIPANT_COLUMNS`], in their
/// order.
fn participant_from(row: &Row) -> Result<Participant, StoreError> {
    let claims: String = row.get(1)?;
    Ok(Participant {
        did: row.get(0)?,
        claims: from_json(&claims)?,
        registered_at: from_sql_int(row.get(2)?),
    })
}

fn status_from_text(text: &str) -> Result<RequestStatus, StoreError> {
    RequestStatus::from_name(text)
        .ok_or_else(|| StoreError(format!("unknown request status {text}")))
}

fn profile_from_text(text: &str) -> Result<Profile, DbError> {
    Profile::from_alias(text).ok_or_else(|| DbError::new(format!("unknown profile {text}")))
}

fn to_json(value: &impl serde::Serialize) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|e| StoreError(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError(e.to_string()))
}

/// A number (seconds or a status list position) as the database keeps
/// whole numbers: signed, 64 bits.
fn to_sql_int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A whole number read back; a negative one (never written) is 0.
fn from_sql_int(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_runtime::testing::{databases, schema};
    use std::path::Path;

    const HOLDER: &str = "did:web:holder.example";

    fn request(issuer_pid: &str, holder_pid: &str) -> CredentialRequest {
        CredentialRequest {
            issuer_pid: issuer_pid.to_owned(),
            holder_did: HOLDER.to_owned(),
            holder_pid: holder_pid.to_owned(),
            credential_ids: vec!["membership-vc11".to_owned()],
            access_token: Some("token".to_owned()),
            status: RequestStatus::Received,
            received_at: 100,
        }
    }

    fn credential(id: &str, issuer_pid: &str, index: u64) -> IssuedCredential {
        IssuedCredential {
            id: id.to_owned(),
            issuer_pid: issuer_pid.to_owned(),
            holder_did: HOLDER.to_owned(),
            definition_id: "membership-vc11".to_owned(),
            credential_type: "MembershipCredential".to_owned(),
            profile: Profile::Vc11,
            status_index: index,
            issued_at: 100,
            expires_at: 200,
            revoked_at: None,
        }
    }

    fn planned(definition_id: &str, profile: Profile) -> Planned {
        Planned {
            definition_id: definition_id.to_owned(),
            profile,
        }
    }

    const RULES: IssueRules = IssueRules {
        max_active_per_holder: 10,
        now: 100,
    };

    /// Issues the request with one credential `id` per planned one, and
    /// returns the outcome and the positions given.
    fn issue_as(
        store: &dyn IssuerStore,
        issuer_pid: &str,
        planned: &[Planned],
        rules: IssueRules,
    ) -> (IssueOutcome, Vec<u64>) {
        let mut given = Vec::new();
        let outcome = store
            .issue(issuer_pid, planned, rules, &mut |positions| {
                given = positions.to_vec();
                let mut credentials = Vec::new();
                for (one, position) in planned.iter().zip(positions) {
                    let mut record = credential(&new_id(), issuer_pid, *position);
                    record.definition_id.clone_from(&one.definition_id);
                    record.profile = one.profile;
                    credentials.push(record);
                }
                Ok(SignedRequest {
                    records: credentials,
                    message_json: "{}".to_owned(),
                })
            })
            .unwrap();
        (outcome, given)
    }

    fn new_id() -> String {
        portitor_runtime::system::new_uuid()
    }

    /// A signer of status lists with the key `kid`: the "credential" it
    /// makes names the profile and the revoked positions, so a test can
    /// read what was signed.
    fn signer(kid: &str) -> impl FnMut(Profile, &[u64]) -> Result<SignedStatusList, String> {
        let kid = kid.to_owned();
        move |profile, revoked| {
            Ok(SignedStatusList {
                credential: format!("{}:{revoked:?}", profile.alias()),
                kid: kid.clone(),
            })
        }
    }

    /// The lease of process `holder` until `until`.
    fn lease(holder: &str, until: u64) -> Lease<'_> {
        Lease { holder, until }
    }

    /// Every database the tests run on, each with a store.
    fn stores() -> Vec<DatabaseStore> {
        databases::all(&MIGRATIONS)
            .into_iter()
            .map(|db| DatabaseStore::new(Arc::new(db)))
            .collect()
    }

    /// Run by [`check`]: the outbox of request `i-1`, issued at 100, is
    /// claimed, tried, retried and delivered.
    fn check_outbox(store: &dyn IssuerStore) {
        assert_eq!(store.next_delivery_at().unwrap(), Some(100));
        assert!(store.due_deliveries(99).unwrap().is_empty());
        assert_eq!(
            store.claim_due_delivery(99, lease("p-1", 129)).unwrap(),
            None
        );
        let due = store.due_deliveries(100).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].holder_did, HOLDER);
        assert_eq!(due[0].access_token.as_deref(), Some("token"));
        let claimed = store.claim_due_delivery(100, lease("p-1", 130)).unwrap();
        assert_eq!(claimed.as_ref(), due.first());
        assert!(store.retry_later("i-1", "p-1", 1, 160).unwrap());
        assert!(store.due_deliveries(150).unwrap().is_empty());
        assert_eq!(store.due_deliveries(160).unwrap()[0].attempts, 1);
        let listed = store.issued(Some(HOLDER)).unwrap();
        assert_eq!(listed[0].delivery, DeliveryState::Pending);
        let claimed = store.claim_due_delivery(160, lease("p-1", 190)).unwrap();
        assert_eq!(claimed.unwrap().attempts, 1);
        assert!(
            store
                .end_delivery("i-1", "p-1", DeliveryState::Delivered)
                .unwrap()
        );
        assert_eq!(store.next_delivery_at().unwrap(), None);
    }

    /// Runs every operation; returns the status list position of the one
    /// credential issued and revoked.
    fn check(store: &dyn IssuerStore) -> u64 {
        // participants
        let mut claims = Map::new();
        claims.insert("role".to_owned(), Value::from("provider"));
        let participant = Participant {
            did: HOLDER.to_owned(),
            claims,
            registered_at: 50,
        };
        assert!(store.add_participant(&participant).unwrap());
        assert!(!store.add_participant(&participant).unwrap(), "exists");
        assert_eq!(
            store.participant(HOLDER).unwrap(),
            Some(participant.clone())
        );
        assert_eq!(store.participants().unwrap(), [participant]);

        // a request, its duplicate, and its decision
        assert!(store.add_request(&request("i-1", "h-1")).unwrap());
        assert!(
            !store.add_request(&request("i-2", "h-1")).unwrap(),
            "same holderPid"
        );
        let mut other_holder = request("i-3", "h-1");
        other_holder.holder_did = "did:web:other.example".to_owned();
        assert!(store.add_request(&other_holder).unwrap());
        assert_eq!(store.received_requests().unwrap(), ["i-1", "i-3"]);
        assert!(store.reject("i-3").unwrap());
        assert!(!store.reject("i-3").unwrap(), "decided already");
        let rejected = store.request("i-3").unwrap().unwrap();
        assert_eq!(rejected.status, RequestStatus::Rejected);
        assert_eq!(rejected.access_token, None);

        let vc11 = [planned("membership-vc11", Profile::Vc11)];
        let (outcome, given) = issue_as(store, "i-1", &vc11, RULES);
        assert_eq!(outcome, IssueOutcome::Issued);
        let position = given[0];
        assert!(position < STATUS_LIST_ENTRIES);
        let again = store.issue("i-1", &vc11, RULES, &mut |_| panic!("signed twice"));
        assert_eq!(again.unwrap(), IssueOutcome::NotReceived);
        let id = store.issued(None).unwrap()[0].credential.id.clone();
        assert_eq!(
            store.request("i-1").unwrap().unwrap().status,
            RequestStatus::Issued
        );

        check_outbox(store);
        assert_eq!(store.request("i-1").unwrap().unwrap().access_token, None);

        // listing and revocation
        assert_eq!(store.issued(None).unwrap().len(), 1);
        assert!(
            store
                .issued(Some("did:web:other.example"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.issued(Some(HOLDER)).unwrap()[0].delivery,
            DeliveryState::Delivered
        );
        assert_eq!(
            store.revoke(&id, 150, &mut signer("k-1")).unwrap(),
            Revocation::Revoked(Profile::Vc11)
        );
        let mut not_signed =
            |_: Profile, _: &[u64]| -> Result<SignedStatusList, String> { panic!("signed again") };
        assert_eq!(
            store.revoke(&id, 160, &mut not_signed).unwrap(),
            Revocation::AlreadyRevoked(Profile::Vc11)
        );
        assert_eq!(
            store.revoke("c-x", 160, &mut not_signed).unwrap(),
            Revocation::NotFound
        );
        assert_eq!(store.revoked_indexes(Profile::Vc11).unwrap(), [position]);
        // the list was stored with the revocation
        let list = store.status_list(Profile::Vc11).unwrap().unwrap();
        assert_eq!(list.credential, format!("vc11-sl2021/jwt:[{position}]"));
        assert_eq!(list.kid, "k-1");
        assert_eq!(store.status_list(Profile::Vc20).unwrap(), None);
        assert!(store.revoked_indexes(Profile::Vc20).unwrap().is_empty());
        assert_eq!(
            store.issued(None).unwrap()[0].credential.revoked_at,
            Some(150)
        );

        // deleting a participant keeps what it was issued
        assert!(store.delete_participant(HOLDER).unwrap());
        assert!(!store.delete_participant(HOLDER).unwrap());
        assert_eq!(store.issued(None).unwrap().len(), 1);
        position
    }

    #[test]
    fn on_every_database() {
        for store in stores() {
            check(&store);
        }
    }

    /// The PostgreSQL schema has the tables and columns the `SQLite`
    /// migrations end with.
    #[test]
    fn both_schemas_have_the_same_tables_and_columns() {
        let Some(postgres) = databases::fresh_postgres(&MIGRATIONS) else {
            return;
        };
        let sqlite = Database::in_memory(&MIGRATIONS).unwrap();
        assert_eq!(schema::columns(&postgres), schema::columns(&sqlite));
    }

    /// The file of `path` as the database setting.
    fn file(path: &Path) -> DatabaseSetting {
        DatabaseSetting::SqliteFile(path.to_owned())
    }

    #[test]
    fn in_a_file_that_is_opened_again() {
        let dir = std::env::temp_dir().join(format!(
            "portitor-issuer-store-{}",
            new_id().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("issuer.db");
        let position = check(&DatabaseStore::open(&file(&path)).unwrap());
        let again = DatabaseStore::open(&file(&path)).unwrap();
        assert_eq!(again.revoked_indexes(Profile::Vc11).unwrap(), [position]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_limit_counts_active_credentials_of_one_definition() {
        for store in stores() {
            the_limit_counts_active_credentials_of_one_definition_on(&store);
        }
    }

    fn the_limit_counts_active_credentials_of_one_definition_on(store: &DatabaseStore) {
        let rules = IssueRules {
            max_active_per_holder: 2,
            now: 100,
        };
        let vc11 = [planned("membership-vc11", Profile::Vc11)];
        for (pid, expected) in [
            ("i-1", IssueOutcome::Issued),
            ("i-2", IssueOutcome::Issued),
            (
                "i-3",
                IssueOutcome::LimitReached("membership-vc11".to_owned()),
            ),
        ] {
            store.add_request(&request(pid, pid)).unwrap();
            assert_eq!(issue_as(store, pid, &vc11, rules).0, expected, "{pid}");
        }
        // nothing was written for i-3: it is still RECEIVED
        assert_eq!(store.received_requests().unwrap(), ["i-3"]);
        // another definition has its own count
        let vc20 = [planned("membership-vc20", Profile::Vc20)];
        store.add_request(&request("i-4", "i-4")).unwrap();
        assert_eq!(issue_as(store, "i-4", &vc20, rules).0, IssueOutcome::Issued);
        // a revoked credential, and an expired one, no longer count
        let first = store.issued(None).unwrap();
        let revoked = first
            .iter()
            .find(|record| record.credential.issuer_pid == "i-1")
            .unwrap();
        store
            .revoke(&revoked.credential.id, 100, &mut signer("k-1"))
            .unwrap();
        assert_eq!(issue_as(store, "i-3", &vc11, rules).0, IssueOutcome::Issued);
        store.add_request(&request("i-5", "i-5")).unwrap();
        let later = IssueRules {
            max_active_per_holder: 2,
            now: 200,
        };
        assert_eq!(issue_as(store, "i-5", &vc11, later).0, IssueOutcome::Issued);
    }

    #[test]
    fn positions_are_random_distinct_and_never_wasted() {
        for store in stores() {
            positions_are_random_distinct_and_never_wasted_on(&store);
        }
    }

    fn positions_are_random_distinct_and_never_wasted_on(store: &DatabaseStore) {
        // two definitions in one list get two positions
        let both = [
            planned("membership-a", Profile::Vc11),
            planned("membership-b", Profile::Vc11),
        ];
        store.add_request(&request("i-1", "h-1")).unwrap();
        let (outcome, positions) = issue_as(store, "i-1", &both, RULES);
        assert_eq!(outcome, IssueOutcome::Issued);
        assert_ne!(positions[0], positions[1]);
        // random: forty credentials are not at 0..40
        let rules = IssueRules {
            max_active_per_holder: 100,
            now: 100,
        };
        let mut seen = BTreeSet::new();
        for n in 0..20 {
            let pid = format!("r-{n}");
            store.add_request(&request(&pid, &pid)).unwrap();
            let (_, given) = issue_as(store, &pid, &both, rules);
            seen.extend(given);
        }
        assert_eq!(seen.len(), 40, "never the same position twice");
        assert!(seen.iter().any(|position| *position >= 40));
        // a failed signing writes nothing
        store.add_request(&request("i-2", "h-2")).unwrap();
        let failed = store.issue("i-2", &both, rules, &mut |_| Err("no key".to_owned()));
        assert!(failed.is_err());
        assert_eq!(store.received_requests().unwrap(), ["i-2"]);
        assert_eq!(store.issued(None).unwrap().len(), 42);
    }

    #[test]
    fn a_full_list_gives_no_position() {
        for store in stores() {
            store.add_request(&request("i-1", "h-1")).unwrap();
            store
                .db()
                .transaction(|tx| {
                    for position in [0, 1, 3] {
                        insert_issued(tx, &credential(&new_id(), "i-1", position))?;
                    }
                    // a list of 4 with 0, 1 and 3 taken: only 2 is free,
                    // then none
                    for _ in 0..5 {
                        assert_eq!(free_position(tx, Profile::Vc11, &[], 4)?, Some(2));
                    }
                    assert_eq!(free_position(tx, Profile::Vc11, &[2], 4)?, None);
                    // the other profile's list is empty
                    assert!(free_position(tx, Profile::Vc20, &[], 4)?.is_some());
                    Ok(())
                })
                .unwrap();
        }
    }

    /// A delivery is sent by one process at a time: the one that claimed
    /// it holds it until its lease ends, then another may take it over.
    #[test]
    fn a_delivery_is_held_by_one_process_until_its_lease_ends() {
        for store in stores() {
            store.add_request(&request("i-1", "h-1")).unwrap();
            let vc11 = [planned("membership-vc11", Profile::Vc11)];
            issue_as(&store, "i-1", &vc11, RULES);
            let first = store.claim_due_delivery(100, lease("p-1", 130)).unwrap();
            assert_eq!(first.unwrap().issuer_pid, "i-1");
            // p-2 finds nothing free, and waits for the lease to end
            assert_eq!(
                store.claim_due_delivery(100, lease("p-2", 130)).unwrap(),
                None
            );
            assert_eq!(
                store.claim_due_delivery(129, lease("p-2", 159)).unwrap(),
                None
            );
            assert_eq!(store.next_delivery_at().unwrap(), Some(130));
            // p-1 stopped: at 130 its lease has ended, and p-2 takes over
            let taken = store.claim_due_delivery(130, lease("p-2", 160)).unwrap();
            assert_eq!(taken.unwrap().issuer_pid, "i-1");
            // p-1 comes back late: it holds the delivery no more, so it
            // changes nothing
            assert!(!store.retry_later("i-1", "p-1", 1, 200).unwrap());
            assert!(
                !store
                    .end_delivery("i-1", "p-1", DeliveryState::Failed)
                    .unwrap()
            );
            assert_eq!(store.due_deliveries(130).unwrap()[0].attempts, 0);
            // p-2 records its try; the lease ends with it
            assert!(store.retry_later("i-1", "p-2", 1, 140).unwrap());
            assert_eq!(store.next_delivery_at().unwrap(), Some(140));
            let again = store.claim_due_delivery(140, lease("p-1", 170)).unwrap();
            assert_eq!(again.unwrap().attempts, 1);
            assert!(
                store
                    .end_delivery("i-1", "p-1", DeliveryState::Delivered)
                    .unwrap()
            );
            assert_eq!(store.next_delivery_at().unwrap(), None);
            let listed = store.issued(None).unwrap();
            assert_eq!(listed[0].delivery, DeliveryState::Delivered);
        }
    }

    /// The list stored with a revocation has every revocation; a failed
    /// signing revokes nothing; at start a list is signed only when none
    /// is stored or another key signed it.
    #[test]
    fn a_status_list_is_stored_with_each_revocation() {
        for store in stores() {
            assert!(
                store
                    .sign_status_list_if_needed(Profile::Vc11, "k-1", &mut signer("k-1"))
                    .unwrap()
            );
            let first = store.status_list(Profile::Vc11).unwrap().unwrap();
            assert_eq!(first.credential, "vc11-sl2021/jwt:[]");
            // the same key: the stored list is kept
            assert!(
                !store
                    .sign_status_list_if_needed(Profile::Vc11, "k-1", &mut signer("k-1"))
                    .unwrap()
            );
            store.add_request(&request("i-1", "h-1")).unwrap();
            let both = [
                planned("membership-a", Profile::Vc11),
                planned("membership-b", Profile::Vc11),
            ];
            let (_, positions) = issue_as(&store, "i-1", &both, RULES);
            let ids: Vec<String> = store
                .issued(None)
                .unwrap()
                .into_iter()
                .map(|record| record.credential.id)
                .collect();
            // signing fails: nothing is revoked, the list stays
            let mut failing = |_: Profile, _: &[u64]| -> Result<SignedStatusList, String> {
                Err("no key".to_owned())
            };
            assert!(store.revoke(&ids[0], 150, &mut failing).is_err());
            assert!(store.revoked_indexes(Profile::Vc11).unwrap().is_empty());
            assert_eq!(store.status_list(Profile::Vc11).unwrap().unwrap(), first);
            // two revocations: the list has both
            for id in &ids {
                store.revoke(id, 150, &mut signer("k-1")).unwrap();
            }
            let mut sorted = positions.clone();
            sorted.sort_unstable();
            let list = store.status_list(Profile::Vc11).unwrap().unwrap();
            assert_eq!(list.credential, format!("vc11-sl2021/jwt:{sorted:?}"));
            // a new key at start: signed again, with the revocations
            assert!(
                store
                    .sign_status_list_if_needed(Profile::Vc11, "k-2", &mut signer("k-2"))
                    .unwrap()
            );
            let list = store.status_list(Profile::Vc11).unwrap().unwrap();
            assert_eq!(list.kid, "k-2");
            assert_eq!(list.credential, format!("vc11-sl2021/jwt:{sorted:?}"));
        }
    }

    /// Two processes ("replicas") on one PostgreSQL database.
    #[test]
    fn two_replicas_share_the_outbox_and_the_status_lists() {
        let Some(url) = databases::postgres_url() else {
            return;
        };
        let setting = DatabaseSetting::Postgres(Box::new(databases::fresh_postgres_settings(&url)));
        let a = Arc::new(DatabaseStore::open(&setting).unwrap());
        let b = Arc::new(DatabaseStore::open(&setting).unwrap());
        // both start at once: one list is signed and stored
        let starts: Vec<_> = [Arc::clone(&a), Arc::clone(&b)]
            .into_iter()
            .map(|store| {
                std::thread::spawn(move || {
                    store
                        .sign_status_list_if_needed(Profile::Vc11, "k-1", &mut signer("k-1"))
                        .unwrap()
                })
            })
            .collect();
        let signed: Vec<bool> = starts.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(signed.iter().filter(|s| **s).count(), 1, "{signed:?}");

        // twenty deliveries, claimed by both at once: each by one
        let vc11 = [planned("membership-vc11", Profile::Vc11)];
        let rules = IssueRules {
            max_active_per_holder: 100,
            now: 100,
        };
        for n in 0..20 {
            let pid = format!("i-{n}");
            a.add_request(&request(&pid, &pid)).unwrap();
            issue_as(a.as_ref(), &pid, &vc11, rules);
        }
        let claims: Vec<_> = [("p-a", Arc::clone(&a)), ("p-b", Arc::clone(&b))]
            .into_iter()
            .map(|(holder, store)| {
                std::thread::spawn(move || {
                    let mut claimed = Vec::new();
                    while let Some(delivery) =
                        store.claim_due_delivery(100, lease(holder, 130)).unwrap()
                    {
                        claimed.push(delivery.issuer_pid);
                    }
                    claimed
                })
            })
            .collect();
        let mut all: Vec<String> = claims.into_iter().flat_map(|t| t.join().unwrap()).collect();
        assert_eq!(all.len(), 20, "{all:?}");
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 20, "a delivery claimed twice");

        // a revocation on one is served by the other at once
        let id = a.issued(None).unwrap()[0].credential.id.clone();
        a.revoke(&id, 150, &mut signer("k-1")).unwrap();
        let position = a.issued(None).unwrap()[0].credential.status_index;
        let listed = b.status_list(Profile::Vc11).unwrap().unwrap();
        assert_eq!(listed.credential, format!("vc11-sl2021/jwt:[{position}]"));
        // a token id used on one is refused by the other
        assert!(a.use_token_id("j-1", 200, 100).unwrap());
        assert!(!b.use_token_id("j-1", 200, 100).unwrap());
    }

    #[test]
    fn the_nth_free_position_skips_taken_ones() {
        let taken: BTreeSet<u64> = [0, 2, 3].into_iter().collect();
        assert_eq!(nth_free(&taken, 6, 0), Some(1));
        assert_eq!(nth_free(&taken, 6, 1), Some(4));
        assert_eq!(nth_free(&taken, 6, 2), Some(5));
        assert_eq!(nth_free(&taken, 6, 3), None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod poison_tests;
