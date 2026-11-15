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
//! nothing else would renew the credential. The management API starts a
//! run now ([`start_run`]) and lists the last runs. Runs never overlap.
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
/// (HTTPS only unless `PORTITOR_ALLOW_HTTP_CALLBACKS`, size limit, no
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
    participant_file: PathBuf,
    /// Held while a run is on.
    running: Arc<Mutex<()>>,
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
    pub fn new(
        compliance: Arc<dyn GaiaxCompliance>,
        participant_file: &Path,
    ) -> Result<Self, String> {
        let participant = read_participant(participant_file)?;
        tracing::info!(
            registration_number = %participant.registration_subject,
            "Gaia-X participant file read"
        );
        Ok(Self {
            compliance,
            participant_file: participant_file.to_owned(),
            running: Arc::new(Mutex::new(())),
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
    let started_at = now_secs();
    let id = identity
        .store
        .start_gaiax_run(started_at)
        .map_err(StartError::Store)?;
    let background = Arc::clone(identity);
    tokio::spawn(async move {
        finish_run(&background, id, guard).await;
        if let Some(onboarding) = &background.gaiax {
            onboarding.ran.notify_one();
        }
    });
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
    let id = match identity.store.start_gaiax_run(now_secs()) {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, "identity store failed");
            return None;
        }
    };
    finish_run(identity, id, guard).await
}

/// Runs, records how it ended, and lets the next run start.
async fn finish_run(
    identity: &Identity,
    id: i64,
    _running: OwnedMutexGuard<()>,
) -> Option<GaiaxRun> {
    let outcome = onboard(identity, now_secs()).await;
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

/// One run at `now`. Returns when the compliance credential expires.
async fn onboard(identity: &Identity, now: u64) -> Result<Option<i64>, String> {
    let onboarding = identity.gaiax.as_ref().ok_or("Gaia-X is off")?;
    let participant = read_participant(&onboarding.participant_file)?;
    let did = identity.participant_id.as_str();
    let base = identity.public_base.as_str();
    let period = Period::starting(now);
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
    let registration_jwt = participant.registration.jwt();
    let vp = presentation(did, &[&legal_jwt, &terms_jwt, registration_jwt], period);
    let vp_jwt = sign_presentation(signer, &vp).map_err(|e| e.to_string())?;

    // what the answer must list: each credential's id and digest
    let registration_payload = participant
        .registration
        .payload()
        .map_err(|e| e.to_string())?;
    let registration_id = participant.registration.id.clone().unwrap_or_default();
    let sent = vec![
        (legal_id.clone(), digest_sri(&legal)),
        (terms_id.clone(), digest_sri(&terms)),
        (registration_id, digest_sri(&registration_payload)),
    ];

    let vcid = credential_url(base, COMPLIANCE);
    let answer_text = onboarding
        .compliance
        .request_compliance(&vcid, &vp_jwt)
        .await
        .map_err(|e| format!("compliance service: {e}"))?;
    let answer = Credential::parse(answer_text.trim())
        .map_err(|e| format!("compliance service answer: {e}"))?;
    check_answer(&answer, &vcid, &sent).map_err(|e| e.to_string())?;
    let service = identity
        .directory
        .did_document(&answer.issuer)
        .await
        .map_err(|e| format!("cannot resolve {}: {e}", answer.issuer))?;
    answer
        .verify(&service, now, LEEWAY)
        .map_err(|e| format!("compliance service answer: {e}"))?;

    let legal_read = Credential::parse(&legal_jwt).map_err(|e| e.to_string())?;
    let terms_read = Credential::parse(&terms_jwt).map_err(|e| e.to_string())?;
    let stored = [
        stored(&legal_read, LEGAL_PERSON),
        stored(&terms_read, TERMS_AND_CONDITIONS),
        stored(&participant.registration, REGISTRATION_NUMBER),
        stored(&answer, COMPLIANCE),
    ];
    identity
        .store
        .replace_gaiax_credentials(&stored)
        .map_err(|e| e.to_string())?;
    Ok(answer.validity.until)
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

/// Keeps the compliance credential valid, until the process stops: runs
/// when [`renewal_due`] says so, and after a failed run again as the
/// retry policy says (see the module documentation).
pub async fn run_schedule(identity: Arc<Identity>) {
    let Some(onboarding) = &identity.gaiax else {
        return;
    };
    match identity.store.abandon_gaiax_runs(now_secs()) {
        Ok(0) => {}
        Ok(count) => tracing::warn!(count, "Gaia-X runs stopped by a restart marked failed"),
        Err(error) => tracing::error!(%error, "identity store failed"),
    }
    let mut failed: u32 = 0;
    let mut last_failure: u64 = 0;
    loop {
        let now = now_secs();
        let renewal = renewal_due(&identity, now);
        // a valid credential (from any run, also one the operator started)
        // ends the retries
        if renewal > now {
            failed = 0;
        }
        let due = if failed == 0 {
            renewal
        } else {
            last_failure.saturating_add(retry_delay(failed))
        };
        if due <= now {
            let run = run_now(&identity).await;
            if run.is_some_and(|run| run.outcome == RunOutcome::Succeeded) {
                failed = 0;
            } else {
                failed = failed.saturating_add(1);
                last_failure = now_secs();
            }
            continue;
        }
        let wait = Duration::from_secs(due - now).min(IDLE);
        tokio::select! {
            () = onboarding.ran.notified() => {}
            () = tokio::time::sleep(wait) => {}
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
