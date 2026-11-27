// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Gaia-X onboarding (ADR 0021): the identity gets a Gaia-X compliance
//! credential for its participant, and keeps it valid.
//!
//! One run:
//!
//! 1. Read the participant file (`PORTITOR_GAIAX_PARTICIPANT_FILE`) and
//!    the registration number credential it names (a VC-JWT a GXDCH
//!    notary issued). Both are read at each run, so a new file needs no
//!    restart.
//! 2. Sign the `gx:LegalPerson` and `gx:Issuer` credentials with the
//!    identity key, valid for 90 days, with ids under
//!    `{public base}/credentials/gaiax/` ([`portitor_dcp::gaiax_sign`]).
//! 3. Wrap the three in a `vp+jwt` and send it to the compliance service
//!    ([`GaiaxCompliance`]), asking for the id
//!    `{public base}/credentials/gaiax/compliance`.
//! 4. Check the answer: a VC-JWT of type
//!    `gx:ParticipantComplianceCredential` with that id, whose
//!    `digestSRI`s are those of the three credentials sent
//!    ([`portitor_dcp::gaiax_sign::check_answer`]), signed by a key in the
//!    `assertionMethod` of the service's `did:web` document, and valid now.
//! 5. Store the four credentials like any other credential, replacing
//!    those of the last run, so the presentation API returns them. A run
//!    that fails stores nothing: the credentials of the last good run
//!    stay until they expire.
//!
//! When ([`run_schedule`]): at start if there is no valid compliance
//! credential, then once two thirds of its validity have passed. A
//! failed run is tried again after 1, 2, 4 ... seconds, at most one hour
//! apart: the issuer's outbox retry policy
//! ([`portitor_runtime::retry::retry_delay`]), but never given up, since
//! nothing else would renew the credential. After a successful run the
//! next one waits at least a minute. The management API starts a run now
//! ([`start_run`]) and lists the last runs. Runs never overlap, and a
//! scheduled run that waited for the operator's does not follow it at
//! once.
//!
//! Several processes may share one database (ADR 0029, TD-56): a run
//! needs the lease [`RUN_LEASE`] in the database, renewed while it runs
//! and ended with it, so runs never overlap among the processes either.
//! A process that finds it held does not run. The runs still `RUNNING`
//! when a process takes the lease were left by a process that stopped
//! (its lease ended): they are marked failed then, not at start, where
//! they may be another process's run. A run whose lease is lost (its
//! process stalled, or the database did not answer, until the lease
//! ended) stops, stores nothing and is recorded failed: the process that
//! took the lease over runs instead.
//!
//! The public base is where the participant's `did:web` DID document is,
//! without `did.json` ([`public_base`]): `https://provider.example` for
//! `did:web:provider.example`, `https://host.example/participants/a` for
//! `did:web:host.example:participants:a`. The identity serves its
//! certificate chain and these credentials below it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use portitor_dcp::credential::{Credential, Profile};
use portitor_dcp::gaiax::{GAIAX_CONTEXT_PREFIX, digest_sri};
use portitor_dcp::gaiax_sign::{
    Period, REGISTRATION_NUMBER_MEMBER, check_answer, legal_person, presentation, sign_credential,
    sign_presentation, terms_and_conditions,
};
use portitor_runtime::BoxFuture;
use portitor_runtime::api_key::problem;
use portitor_runtime::http::HttpSender;
use portitor_runtime::leases::{Claim, HeldLease, LeaseLost, LeaseStore, Leases};
use portitor_runtime::retry::retry_delay;
use portitor_runtime::system::now_secs;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, Notify, OwnedMutexGuard};

use crate::store::{GAIAX_HOLDER_PID_PREFIX, GaiaxRun, RunOutcome, StoreError, StoredCredential};
use crate::{Identity, LEEWAY};

/// The name (last path segment) of the legal person credential.
pub const LEGAL_PERSON: &str = "legal-person";
/// The name of the terms and conditions (`gx:Issuer`) credential.
pub const TERMS_AND_CONDITIONS: &str = "terms-and-conditions";
/// The name under which the registration number credential is served.
pub const REGISTRATION_NUMBER: &str = "registration-number";
/// The name of the compliance credential (the `vcid` asked for).
pub const COMPLIANCE: &str = "compliance";

/// The path under the public base where the Gaia-X credentials are.
pub const CREDENTIALS_PATH: &str = "/credentials/gaiax";

/// The media type of a VC-JWT (VC JOSE COSE, section 6.1.1).
pub const VC_JWT_MEDIA_TYPE: &str = "application/vc+jwt";
/// The media type of a VP-JWT, how the presentation is sent.
pub const VP_JWT_MEDIA_TYPE: &str = "application/vp+jwt";

/// The path of the compliance service's endpoint, after its base URL
/// (gx-compliance v2.16.0, `POST /api/credential-offers/standard-compliance`).
pub const STANDARD_COMPLIANCE_PATH: &str = "/api/credential-offers/standard-compliance";

/// How long the compliance service may take to answer: it resolves the
/// DIDs, certificates and contexts itself before it answers.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// The largest answer read from the compliance service.
const MAX_ANSWER: usize = 256 * 1024;

/// The longest the schedule sleeps before it looks at the store again.
const IDLE: Duration = Duration::from_secs(3_600);

/// How many runs the management API lists.
pub const RUNS_LISTED: usize = 50;

/// Port: a Gaia-X compliance service. The HTTP adapter is
/// [`HttpGaiaxCompliance`]; an `OpenID4VCI` one can follow when the
/// Gaia-X Compliance Dispatcher ships it (ADR 0021).
pub trait GaiaxCompliance: Send + Sync {
    /// Sends `presentation` (a `vp+jwt`) and asks for a compliance
    /// credential with the id `vcid`. Returns, later, the answer as text
    /// (a VC-JWT, not yet checked), or why there is none.
    fn request_compliance<'a>(
        &'a self,
        vcid: &'a str,
        presentation: &'a str,
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// Adapter: the compliance service's REST API over the shared HTTP client
/// (HTTPS only unless `PORTITOR_ALLOW_HTTP`, size limit, no
/// redirects): `POST {base}/api/credential-offers/standard-compliance?vcid=<id>`
/// with `Content-Type: application/vp+jwt`.
#[derive(Debug)]
pub struct HttpGaiaxCompliance {
    http: HttpSender,
    base_url: String,
}

impl HttpGaiaxCompliance {
    /// The service at `base_url` (`PORTITOR_GAIAX_COMPLIANCE_URL`).
    ///
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn new(base_url: &str, allow_http: bool) -> Result<Self, String> {
        Ok(Self {
            http: HttpSender::new(allow_http)?,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }
}

impl GaiaxCompliance for HttpGaiaxCompliance {
    fn request_compliance<'a>(
        &'a self,
        vcid: &'a str,
        presentation: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let url = format!(
                "{}{STANDARD_COMPLIANCE_PATH}?vcid={}",
                self.base_url,
                query_encode(vcid)
            );
            self.http
                .post_text(
                    &url,
                    VP_JWT_MEDIA_TYPE,
                    presentation.to_owned(),
                    MAX_ANSWER,
                    ANSWER_TIMEOUT,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// Percent-encodes a query value: every byte but the unreserved
/// characters of RFC 3986, section 2.3.
fn query_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push_str(&portitor_runtime::system::hex(&[byte]).to_uppercase());
        }
    }
    out
}

/// The identity's Gaia-X state: the service, the participant file, and
/// what keeps runs from overlapping.
pub struct Onboarding {
    compliance: Arc<dyn GaiaxCompliance>,
    /// The only DID whose answer is taken (`PORTITOR_GAIAX_COMPLIANCE_DID`).
    compliance_did: String,
    participant_file: PathBuf,
    /// Held while a run of this process is on; the lease in the database
    /// ([`RUN_LEASE`]) keeps the other processes out.
    running: Arc<Mutex<()>>,
    /// The leases of this process.
    leases: Leases,
    /// Wakes [`run_schedule`] after a run the operator started.
    ran: Notify,
}

impl std::fmt::Debug for Onboarding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Onboarding")
            .field("participant_file", &self.participant_file)
            .finish_non_exhaustive()
    }
}

impl Onboarding {
    /// Reads the participant file once, so a wrong one stops the start.
    ///
    /// # Errors
    /// The participant file or the registration number credential cannot
    /// be read or is invalid.
    /// `leases`: where the lease of a run is kept, the identity's
    /// database.
    pub fn new(
        compliance: Arc<dyn GaiaxCompliance>,
        compliance_did: &str,
        participant_file: &Path,
        leases: Arc<dyn LeaseStore>,
    ) -> Result<Self, String> {
        let participant = read_participant(participant_file)?;
        tracing::info!(
            registration_number = %participant.registration_subject,
            "Gaia-X participant file read"
        );
        Ok(Self {
            compliance,
            compliance_did: compliance_did.to_owned(),
            participant_file: participant_file.to_owned(),
            running: Arc::new(Mutex::new(())),
            leases: Leases::new(leases, portitor_runtime::system::process_id()),
            ran: Notify::new(),
        })
    }
}

/// The participant file as written (ADR 0021, "Onboarding"). Only Gaia-X
/// terms, copied from the Gaia-X ontology the service expects.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParticipantFile {
    /// The Gaia-X JSON-LD context IRI, or a list of contexts.
    context: Contexts,
    /// The `credentialSubject` claims of the legal person credential.
    legal_person: Map<String, Value>,
    /// The `gaiaxTermsAndConditions` hash.
    terms_and_conditions: String,
    /// The registration number VC-JWT file, absolute or next to the
    /// participant file.
    registration_number_credential: PathBuf,
}

/// One context IRI (the ADR's form), or a list (the IRI and prefix
/// definitions, such as `{"schema": "https://schema.org/"}`, that the
/// legal person claims need).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Contexts {
    One(String),
    Many(Vec<Value>),
}

/// The participant file, checked, with the registration number
/// credential it names.
#[derive(Debug, Clone)]
pub struct Participant {
    /// The contexts after the VC 2.0 context.
    pub contexts: Vec<Value>,
    /// The legal person claims.
    pub legal_person: Map<String, Value>,
    /// The terms and conditions hash.
    pub terms_and_conditions: String,
    /// The registration number credential.
    pub registration: Credential,
    /// Its subject's id, which the legal person links to.
    pub registration_subject: String,
}

/// Reads and checks the participant file and the registration number
/// credential it names.
///
/// # Errors
/// A file cannot be read, is not in the format, or the credential is not
/// a VC 2.0 JWT with one subject id.
pub fn read_participant(file: &Path) -> Result<Participant, String> {
    let name = file.display();
    let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {name}: {e}"))?;
    let parsed: ParticipantFile =
        serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
    let contexts = match parsed.context {
        Contexts::One(iri) => vec![Value::from(iri)],
        Contexts::Many(list) => list,
    };
    let names_gaiax = contexts
        .iter()
        .filter_map(Value::as_str)
        .any(|iri| iri.starts_with(GAIAX_CONTEXT_PREFIX));
    if !names_gaiax {
        return Err(format!(
            "{name}: context has no Gaia-X context ({GAIAX_CONTEXT_PREFIX}...)"
        ));
    }
    for filled in ["id", REGISTRATION_NUMBER_MEMBER] {
        if parsed.legal_person.contains_key(filled) {
            return Err(format!(
                "{name}: legalPerson must not hold {filled}; the identity writes it"
            ));
        }
    }
    if parsed.terms_and_conditions.trim().is_empty() {
        return Err(format!("{name}: termsAndConditions is empty"));
    }
    let registration_file = beside(file, &parsed.registration_number_credential);
    let registration = read_registration(&registration_file)?;
    let registration_subject = match registration.subject_ids().collect::<Vec<_>>().as_slice() {
        [only] => (*only).to_owned(),
        _ => {
            return Err(format!(
                "{}: the registration number credential must have one subject with an id",
                registration_file.display()
            ));
        }
    };
    Ok(Participant {
        contexts,
        legal_person: parsed.legal_person,
        terms_and_conditions: parsed.terms_and_conditions.trim().to_owned(),
        registration,
        registration_subject,
    })
}

/// `path` as it is if absolute, otherwise in the folder of `file`.
fn beside(file: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    match file.parent() {
        Some(folder) => folder.join(path),
        None => path.to_owned(),
    }
}

/// The registration number credential: a VC 2.0 JWT.
fn read_registration(file: &Path) -> Result<Credential, String> {
    let name = file.display();
    let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {name}: {e}"))?;
    let credential = Credential::parse(text.trim()).map_err(|e| format!("{name}: {e}"))?;
    if credential.profile != Profile::Vc20 {
        return Err(format!("{name}: not a VC 2.0 credential"));
    }
    if credential.id.is_none() {
        return Err(format!("{name}: the credential has no id"));
    }
    Ok(credential)
}

/// The public base URL of a `did:web` DID: the URL of its DID document
/// without `did.json`, and without `/.well-known` for a DID without a
/// path. `https://provider.example` for `did:web:provider.example`,
/// `https://host.example/participants/a` for
/// `did:web:host.example:participants:a`. The path stays so that two
/// participants on one host get different credential ids, `vcid` and
/// `x5u` (`did:web`, "Read (Resolve)").
///
/// # Errors
/// `did` is not a valid `did:web` DID.
pub fn public_base(did: &str) -> Result<String, String> {
    let url = portitor_dcp::did_web_url(did).map_err(|e| e.to_string())?;
    let after_scheme = url.trim_start_matches("https://");
    let host = after_scheme.split('/').next().unwrap_or_default();
    Ok(format!("https://{host}{}", public_path(did)?))
}

/// The path of [`public_base`] on the identity's own server: empty for a
/// DID without a path, `/participants/a` for
/// `did:web:host.example:participants:a`. The identity serves its DID
/// document below the same path ([`portitor_runtime::did_document::path`]).
///
/// # Errors
/// `did` is not a valid `did:web` DID.
pub fn public_path(did: &str) -> Result<String, String> {
    let document_path = portitor_runtime::did_document::path(did)?;
    let folder = document_path
        .strip_suffix("/did.json")
        .unwrap_or(&document_path);
    let folder = folder.strip_suffix("/.well-known").unwrap_or(folder);
    Ok(folder.to_owned())
}

/// The URL (and id) of the Gaia-X credential `name`.
#[must_use]
pub fn credential_url(public_base: &str, name: &str) -> String {
    format!("{public_base}{CREDENTIALS_PATH}/{name}")
}

/// Why the operator could not start a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// Gaia-X is off (`PORTITOR_GAIAX_COMPLIANCE_URL` not set).
    Off,
    /// A run is on.
    Busy,
    /// The store failed.
    Store(StoreError),
}

/// Starts a run now, in the background, for the management API: returns
/// the run as recorded at its start.
///
/// # Errors
/// Gaia-X is off, a run is on, or the store fails.
pub fn start_run(identity: &Arc<Identity>) -> Result<GaiaxRun, StartError> {
    let Some(onboarding) = &identity.gaiax else {
        return Err(StartError::Off);
    };
    let Ok(guard) = Arc::clone(&onboarding.running).try_lock_owned() else {
        return Err(StartError::Busy);
    };
    let lease = match claim_run(identity, onboarding) {
        Claim::Taken(lease) => lease,
        // another process runs one
        Claim::HeldByOther => return Err(StartError::Busy),
        Claim::Failed => {
            let error = StoreError("cannot take the run's lease".to_owned());
            return Err(StartError::Store(error));
        }
    };
    let lost = lease.lost();
    let lease = RunLease(Some(lease));
    let started_at = now_secs();
    let id = identity
        .store
        .start_gaiax_run(started_at)
        .map_err(StartError::Store)?;
    let background = Arc::clone(identity);
    // a panic is logged and counted; it drops `guard` and ends the lease,
    // so the next run can start, and this run stays `running` in the
    // store until that run marks it failed
    let run = async move {
        finish_run(&background, id, guard, lost).await;
        drop(lease);
        if let Some(onboarding) = &background.gaiax {
            onboarding.ran.notify_one();
        }
    };
    tokio::spawn(portitor_runtime::panics::report_panic("gaiax_run", run));
    Ok(GaiaxRun {
        id,
        started_at,
        finished_at: None,
        outcome: RunOutcome::Running,
        error: None,
    })
}

/// Runs now, after a run that is on, and returns how it ended. `None`
/// when Gaia-X is off or the store fails.
pub async fn run_now(identity: &Identity) -> Option<GaiaxRun> {
    let onboarding = identity.gaiax.as_ref()?;
    let guard = Arc::clone(&onboarding.running).lock_owned().await;
    run_locked(identity, guard).await
}

/// Runs for [`run_schedule`], which saw `seen` as the last run when it
/// decided a run is due. If another run ended while this one waited for
/// the lock (one the operator started), that run counts as the
/// schedule's and none starts, so two runs never follow each other at
/// once. Returns the run that counts; `None` when Gaia-X is off or the
/// store fails.
async fn run_unless_overtaken(identity: &Identity, seen: Option<i64>) -> Option<GaiaxRun> {
    let onboarding = identity.gaiax.as_ref()?;
    let guard = Arc::clone(&onboarding.running).lock_owned().await;
    // under the lock, the last run has ended
    if let Some(latest) = latest_run(identity)
        && Some(latest.id) != seen
    {
        return Some(latest);
    }
    run_locked(identity, guard).await
}

/// The last run recorded, if any.
fn latest_run(identity: &Identity) -> Option<GaiaxRun> {
    match identity.store.gaiax_runs(1) {
        Ok(runs) => runs.into_iter().next(),
        Err(error) => {
            tracing::error!(%error, "identity store failed");
            None
        }
    }
}

/// Records a run's start and runs it; `running` is the held lock. When
/// another process runs one (it holds [`RUN_LEASE`]), runs nothing and
/// returns that run.
async fn run_locked(identity: &Identity, running: OwnedMutexGuard<()>) -> Option<GaiaxRun> {
    let onboarding = identity.gaiax.as_ref()?;
    let lease = match claim_run(identity, onboarding) {
        Claim::Taken(lease) => lease,
        Claim::HeldByOther => {
            tracing::info!("Gaia-X compliance run skipped: another process runs one");
            return latest_run(identity);
        }
        // logged; the schedule tries again
        Claim::Failed => return None,
    };
    let lost = lease.lost();
    // ends the lease when this function returns, also on a panic
    let _lease = RunLease(Some(lease));
    let id = match identity.store.start_gaiax_run(now_secs()) {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, "identity store failed");
            return None;
        }
    };
    finish_run(identity, id, running, lost).await
}

/// The lease of a run, ended when the value is dropped: when the run
/// ends, and also when it panics, since this process knows that the run
/// is over then. (A process that stops does not end it: it ends on its
/// own.)
struct RunLease(Option<HeldLease>);

impl Drop for RunLease {
    fn drop(&mut self) {
        if let Some(lease) = self.0.take() {
            lease.release();
        }
    }
}

/// The kind and the id of the lease of a Gaia-X compliance run in the
/// database (TD-56, `portitor_runtime::leases`).
pub const RUN_LEASE: (&str, &str) = ("job", "gaiax_compliance");

/// Takes [`RUN_LEASE`] for this process. Once taken, no other process
/// runs: the runs still `RUNNING` were left by a process that stopped,
/// or by one whose lease was lost, which stopped its run before the lease
/// ended (module doc). They are marked failed.
fn claim_run(identity: &Identity, onboarding: &Onboarding) -> Claim {
    let (kind, id) = RUN_LEASE;
    let claim = onboarding.leases.claim(kind, id);
    if !matches!(claim, Claim::Taken(_)) {
        return claim;
    }
    match identity.store.abandon_gaiax_runs(now_secs()) {
        Ok(0) => {}
        Ok(count) => {
            tracing::warn!(
                count,
                "Gaia-X runs left by a process that stopped marked failed"
            );
        }
        Err(error) => tracing::error!(%error, "identity store failed"),
    }
    claim
}

/// Runs, records how it ended, and lets the next run start. `lost`
/// fires when the run's lease is lost: the run then stops (module doc).
async fn finish_run(
    identity: &Identity,
    id: i64,
    _running: OwnedMutexGuard<()>,
    lost: LeaseLost,
) -> Option<GaiaxRun> {
    let outcome = onboard(identity, now_secs(), &lost).await;
    let finished_at = now_secs();
    let (result, error) = match &outcome {
        Ok(until) => {
            tracing::info!(run = id, valid_until = ?until, "Gaia-X compliance credential stored");
            (RunOutcome::Succeeded, None)
        }
        Err(error) => {
            tracing::error!(run = id, %error, "Gaia-X compliance run failed; the stored credentials stay");
            (RunOutcome::Failed, Some(error.as_str()))
        }
    };
    if let Err(error) = identity
        .store
        .finish_gaiax_run(id, finished_at, result, error)
    {
        tracing::error!(%error, "identity store failed");
    }
    match identity.store.gaiax_runs(RUNS_LISTED) {
        Ok(runs) => runs.into_iter().find(|run| run.id == id),
        Err(_) => None,
    }
}

/// One run at `now` (module doc, steps 1 to 5). Returns when the
/// compliance credential expires. Stops, storing nothing, once `lost`
/// fires.
async fn onboard(identity: &Identity, now: u64, lost: &LeaseLost) -> Result<Option<i64>, String> {
    const LOST: &str = "the run's lease was lost; another process runs instead";
    let onboarding = identity.gaiax.as_ref().ok_or("Gaia-X is off")?;
    let participant = read_participant(&onboarding.participant_file)?;
    let signed = build_and_sign(identity, participant, Period::starting(now))?;
    let answer = tokio::select! {
        answer = send_and_check(identity, onboarding, &signed, now) => answer?,
        () = lost.clone().wait() => return Err(LOST.to_owned()),
    };
    // lost while the answer came: the other process's run stores
    if lost.is_lost() {
        return Err(LOST.to_owned());
    }
    store_credentials(identity, &signed, &answer)?;
    Ok(answer.validity.until)
}

/// What one run signs and sends.
struct SignedCredentials {
    /// The `gx:LegalPerson` credential, as read back from its JWT.
    legal_person: Credential,
    /// The `gx:Issuer` (terms and conditions) credential, read back.
    terms: Credential,
    /// The notary's registration number credential, as in the file.
    registration: Credential,
    /// The presentation of the three, signed.
    presentation: String,
    /// The id and `digestSRI` of each of the three: what the answer must
    /// list.
    sent: Vec<(String, String)>,
}

/// Step 2: builds the legal person and terms and conditions credentials
/// for `period`, signs them and the presentation of the three, and notes
/// the digest of each.
fn build_and_sign(
    identity: &Identity,
    participant: Participant,
    period: Period,
) -> Result<SignedCredentials, String> {
    let did = identity.participant_id.as_str();
    let base = identity.public_base.as_str();
    let signer = identity.signer();
    let legal_id = credential_url(base, LEGAL_PERSON);
    let legal = legal_person(
        &legal_id,
        did,
        &participant.contexts,
        &participant.legal_person,
        &participant.registration_subject,
        period,
    );
    let terms_id = credential_url(base, TERMS_AND_CONDITIONS);
    let terms = terms_and_conditions(
        &terms_id,
        did,
        &participant.contexts,
        &participant.terms_and_conditions,
        period,
    );
    let legal_jwt = sign_credential(signer, &legal).map_err(|e| e.to_string())?;
    let terms_jwt = sign_credential(signer, &terms).map_err(|e| e.to_string())?;
    let registration = participant.registration;
    let vp = presentation(did, &[&legal_jwt, &terms_jwt, registration.jwt()], period);
    let presentation = sign_presentation(signer, &vp).map_err(|e| e.to_string())?;
    // the digest of what is sent, not of what is read back: the service
    // hashes the payload it receives
    let registration_payload = registration.payload().map_err(|e| e.to_string())?;
    let registration_id = registration.id.clone().unwrap_or_default();
    let sent = vec![
        (legal_id, digest_sri(&legal)),
        (terms_id, digest_sri(&terms)),
        (registration_id, digest_sri(&registration_payload)),
    ];
    Ok(SignedCredentials {
        legal_person: Credential::parse(&legal_jwt).map_err(|e| e.to_string())?,
        terms: Credential::parse(&terms_jwt).map_err(|e| e.to_string())?,
        registration,
        presentation,
        sent,
    })
}

/// Steps 3 and 4: sends the presentation, asking for the compliance
/// credential's id, and checks the answer: its type, id and digests, its
/// issuer (the configured service's DID), and its signature by a key of
/// that DID's `did:web` document.
async fn send_and_check(
    identity: &Identity,
    onboarding: &Onboarding,
    signed: &SignedCredentials,
    now: u64,
) -> Result<Credential, String> {
    let vcid = credential_url(&identity.public_base, COMPLIANCE);
    let answer_text = onboarding
        .compliance
        .request_compliance(&vcid, &signed.presentation)
        .await
        .map_err(|e| format!("compliance service: {e}"))?;
    let answer = Credential::parse(answer_text.trim())
        .map_err(|e| format!("compliance service answer: {e}"))?;
    check_answer(&answer, &vcid, &signed.sent).map_err(|e| e.to_string())?;
    // only the configured service: checked before its DID is resolved, so
    // an answer never makes the identity fetch a DID document it names
    if answer.issuer != onboarding.compliance_did {
        return Err(format!(
            "compliance service answer: issued by {}, not by {} (PORTITOR_GAIAX_COMPLIANCE_DID)",
            answer.issuer, onboarding.compliance_did
        ));
    }
    let service = identity
        .directory
        .did_document(&answer.issuer)
        .await
        .map_err(|e| format!("cannot resolve {}: {e}", answer.issuer))?;
    answer
        .verify(&service, now, LEEWAY)
        .map_err(|e| format!("compliance service answer: {e}"))?;
    Ok(answer)
}

/// Step 5: stores the four credentials, replacing those of the last run.
fn store_credentials(
    identity: &Identity,
    signed: &SignedCredentials,
    answer: &Credential,
) -> Result<(), String> {
    let stored = [
        stored(&signed.legal_person, LEGAL_PERSON),
        stored(&signed.terms, TERMS_AND_CONDITIONS),
        stored(&signed.registration, REGISTRATION_NUMBER),
        stored(answer, COMPLIANCE),
    ];
    identity
        .store
        .replace_gaiax_credentials(&stored)
        .map_err(|e| e.to_string())
}

/// A Gaia-X credential as the store keeps it; its `holder_pid` says which
/// one it is (`gaiax:<name>`).
fn stored(credential: &Credential, name: &str) -> StoredCredential {
    StoredCredential {
        id: credential.id.clone().unwrap_or_default(),
        issuer: credential.issuer.clone(),
        types: credential.types.clone(),
        profile: credential.profile,
        expires_at: credential.validity.until,
        holder_pid: format!("{GAIAX_HOLDER_PID_PREFIX}{name}"),
        jwt: credential.jwt().to_owned(),
        status: None,
    }
}

/// When the next run is due, seconds since 1970: two thirds into the
/// stored compliance credential's validity, or now (0) when there is
/// none, it has expired, or the store cannot be read.
#[must_use]
pub fn renewal_due(identity: &Identity, now: u64) -> u64 {
    let pid = format!("{GAIAX_HOLDER_PID_PREFIX}{COMPLIANCE}");
    let held = match identity.store.credentials() {
        Ok(credentials) => credentials.into_iter().find(|c| c.holder_pid == pid),
        Err(error) => {
            tracing::error!(%error, "identity store failed");
            None
        }
    };
    let Some(held) = held else {
        return 0;
    };
    let Ok(credential) = Credential::parse(&held.jwt) else {
        return 0;
    };
    renewal_time(credential.validity.from, credential.validity.until, now)
}

/// Two thirds into the period `from` to `until`; without a start, a 90
/// day period is assumed; without an end, there is nothing to renew for
/// another 90 days. 0 (now) when it has ended.
fn renewal_time(from: Option<i64>, until: Option<i64>, now: u64) -> u64 {
    let full = i64::try_from(portitor_dcp::gaiax_sign::VALIDITY_SECONDS).unwrap_or(i64::MAX);
    let now_signed = i64::try_from(now).unwrap_or(i64::MAX);
    let Some(until) = until else {
        return now.saturating_add(portitor_dcp::gaiax_sign::VALIDITY_SECONDS);
    };
    if until <= now_signed {
        return 0;
    }
    let from = from.unwrap_or(until.saturating_sub(full));
    let due = from.saturating_add(until.saturating_sub(from).saturating_mul(2) / 3);
    u64::try_from(due).unwrap_or(0)
}

/// The shortest time, in seconds, from the end of a successful run to the
/// next scheduled one. Without it, a credential that is due for renewal
/// at once (a service that gives a very short validity) or a store that
/// cannot be read back would make the schedule run again and again.
const MIN_RUN_INTERVAL: u64 = 60;

/// Keeps the compliance credential valid, until the process stops: runs
/// when [`renewal_due`] says so, and after a failed run again as the
/// retry policy says (see the module documentation).
pub async fn run_schedule(identity: Arc<Identity>) {
    let Some(onboarding) = &identity.gaiax else {
        return;
    };
    let mut schedule = Schedule::default();
    loop {
        let now = now_secs();
        let seen = latest_run(&identity).map(|run| run.id);
        let due = schedule.next_run(renewal_due(&identity, now), now);
        if due <= now {
            let run = run_unless_overtaken(&identity, seen).await;
            schedule.record(run.as_ref(), now_secs());
            continue;
        }
        let wait = Duration::from_secs(due - now).min(IDLE);
        tokio::select! {
            () = onboarding.ran.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// What [`run_schedule`] remembers between runs.
#[derive(Debug, Default)]
struct Schedule {
    /// Failed runs in a row. While there is no valid credential, the
    /// renewal time is "now"; so after a failure the next run follows the
    /// retry policy instead, or a failing service would be asked again at
    /// once, in a loop.
    failed: u32,
    /// When the last failed run ended: the retry delay counts from there.
    last_failure: u64,
    /// When the last successful run ended (see [`MIN_RUN_INTERVAL`]).
    last_success: u64,
}

impl Schedule {
    /// When the next run is due, given the `renewal` time of the stored
    /// credential ([`renewal_due`]).
    fn next_run(&mut self, renewal: u64, now: u64) -> u64 {
        // a valid credential (from any run, also one the operator started)
        // ends the retries
        if renewal > now {
            self.failed = 0;
        }
        if self.failed > 0 {
            return self.last_failure.saturating_add(retry_delay(self.failed));
        }
        renewal.max(self.last_success.saturating_add(MIN_RUN_INTERVAL))
    }

    /// Counts a run that ended at `ended`; `None` (the store failed)
    /// counts as a failure.
    fn record(&mut self, run: Option<&GaiaxRun>, ended: u64) {
        if run.is_some_and(|run| run.outcome == RunOutcome::Succeeded) {
            self.failed = 0;
            self.last_success = ended;
        } else {
            self.failed = self.failed.saturating_add(1);
            self.last_failure = ended;
        }
    }
}

/// `GET /credentials/gaiax/{name}`: the legal person, terms and
/// conditions and registration number credentials of the last good run,
/// as VC-JWT, so anyone can recompute their digests (ADR 0021).
pub(crate) async fn serve_credential(
    State(identity): State<Arc<Identity>>,
    UrlPath(name): UrlPath<String>,
) -> Response {
    let public = [LEGAL_PERSON, TERMS_AND_CONDITIONS, REGISTRATION_NUMBER];
    if !public.contains(&name.as_str()) {
        return problem(StatusCode::NOT_FOUND, "no such Gaia-X credential");
    }
    let pid = format!("{GAIAX_HOLDER_PID_PREFIX}{name}");
    match identity.store.credentials() {
        Ok(credentials) => match credentials.into_iter().find(|c| c.holder_pid == pid) {
            Some(credential) => {
                ([(header::CONTENT_TYPE, VC_JWT_MEDIA_TYPE)], credential.jwt).into_response()
            }
            None => problem(StatusCode::NOT_FOUND, "no such Gaia-X credential"),
        },
        Err(error) => portitor_runtime::api_key::internal_error("identity store", &error),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn public_base_is_where_the_did_document_is() {
        assert_eq!(
            public_base("did:web:provider.example").unwrap(),
            "https://provider.example"
        );
        assert_eq!(public_path("did:web:provider.example").unwrap(), "");
        // two participants on one host do not share credential ids
        assert_eq!(
            public_base("did:web:localhost%3A8090:participants:a").unwrap(),
            "https://localhost:8090/participants/a"
        );
        assert_eq!(
            public_base("did:web:localhost%3A8090:participants:b").unwrap(),
            "https://localhost:8090/participants/b"
        );
        assert_eq!(
            public_path("did:web:localhost%3A8090:participants:a").unwrap(),
            "/participants/a"
        );
        assert!(public_base("did:key:z6Mk").is_err());
        assert_eq!(
            credential_url("https://provider.example", LEGAL_PERSON),
            "https://provider.example/credentials/gaiax/legal-person"
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(
            query_encode("https://p.example/credentials/gaiax/compliance"),
            "https%3A%2F%2Fp.example%2Fcredentials%2Fgaiax%2Fcompliance"
        );
    }

    fn run(outcome: RunOutcome) -> GaiaxRun {
        GaiaxRun {
            id: 1,
            started_at: 0,
            finished_at: Some(0),
            outcome,
            error: None,
        }
    }

    #[test]
    fn the_schedule_waits_between_runs() {
        let mut schedule = Schedule::default();
        assert_eq!(schedule.next_run(0, 1_000), 60, "no run yet: due now");
        // a success whose credential is due again at once: a minute later
        schedule.record(Some(&run(RunOutcome::Succeeded)), 1_000);
        assert_eq!(schedule.next_run(0, 1_000), 1_060);
        assert_eq!(schedule.next_run(5_000, 1_000), 5_000);
        // failures follow the retry policy: 1, 2, 4 ... seconds
        schedule.record(Some(&run(RunOutcome::Failed)), 2_000);
        assert_eq!(schedule.next_run(0, 2_000), 2_001);
        schedule.record(None, 2_001);
        assert_eq!(schedule.next_run(0, 2_001), 2_003);
        // a valid credential, from any run, ends the retries
        assert_eq!(schedule.next_run(9_000, 2_001), 9_000);
        assert_eq!(schedule.failed, 0);
    }

    #[test]
    fn renewal_is_two_thirds_into_the_validity() {
        assert_eq!(renewal_time(Some(0), Some(90), 10), 60);
        assert_eq!(renewal_time(Some(0), Some(90), 90), 0, "ended");
        assert_eq!(renewal_time(Some(0), Some(90), 100), 0, "ended");
        let full = portitor_dcp::gaiax_sign::VALIDITY_SECONDS;
        let until = i64::try_from(full * 2).unwrap();
        assert_eq!(renewal_time(None, Some(until), 10), full + full * 2 / 3);
        assert_eq!(renewal_time(Some(0), None, 10), 10 + full);
    }
}
