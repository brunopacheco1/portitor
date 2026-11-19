// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! DSP caller authentication with DCP (ADR 0016).
//!
//! Two ports, so tests and the TCK build can swap the adapters:
//!
//! - [`CallerVerifier`]: who is calling, and with which credentials?
//!   Every DSP endpoint (provider endpoints and consumer callbacks; not
//!   `/.well-known/dspace-version`, not the data plane) runs it through
//!   [`require_caller`] before the handler. The adapter
//!   [`dcp::DcpVerifier`] follows DCP v1.0.1: it checks the caller's
//!   self-issued ID token and asks the caller's Credential Service for a
//!   `MembershipCredential`. When an offer or agreement has credential
//!   constraints (ADR 0019), the handler asks it again, with more scopes,
//!   through [`CallerVerifier::credentials`].
//! - [`Prover`]: the `Authorization` header for each DSP request we send.
//!   The adapter [`sts::StsProver`] asks our identity's Secure Token
//!   Service for a fresh token for each request, letting the
//!   counter-party read the configured credentials, and those an offer's
//!   constraints need.
//!
//! [`AuthSettings`] reads the configuration and [`dcp_adapters`] builds
//! both adapters.

use std::ffi::OsString;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use portitor_dcp::gaiax::{COMPLIANCE_SCOPE_TYPE, LEGAL_PERSON_SCOPE_TYPE};
use portitor_dcp::scope::{Scope, VC_ID, merge_scopes};
use portitor_dsp::CatalogError;
use portitor_dsp::negotiation::ContractNegotiationError;
use portitor_dsp::policy::{CallerCredentials, CredentialTypes, HeldCredential, MEMBERSHIP};
use portitor_dsp::transfer::TransferError;
use portitor_runtime::BoxFuture;
use portitor_runtime::api_key::problem;
use portitor_runtime::did::{CACHE_SETTING as DID_CACHE_SETTING, ResolveError};
use portitor_runtime::settings::{bounded_number, comma_list};
use portitor_runtime::telemetry::metrics::{self, Refusal};

pub mod dcp;
pub mod sts;

/// The same as [`BoxFuture`], which the ports below return. Kept as a
/// public name of this crate; new code writes `BoxFuture`.
pub type AuthFuture<'a, T> = BoxFuture<'a, T>;

/// A DSP request's caller, as [`require_caller`] proved it. The handlers
/// take it from the request.
#[derive(Clone, PartialEq, Eq)]
pub struct Caller {
    /// The proven participant id (DID).
    pub id: String,
    /// The caller's credentials that the check verified (trusted issuer,
    /// about the caller, valid now, not revoked): those of the configured
    /// scopes.
    pub credentials: Vec<HeldCredential>,
    /// The access token from the caller's ID token, which lets us query its
    /// Credential Service again (ADR 0019); `None` when there is none.
    access_token: Option<String>,
}

impl Caller {
    /// A caller known by `id` only, with no credentials and no access
    /// token (tests, and the TCK's fixed header).
    #[must_use]
    pub fn new(id: &str) -> Self {
        Self::with_credentials(id, Vec::new())
    }

    /// A caller with verified `credentials`, but no access token to ask
    /// for more.
    #[must_use]
    pub fn with_credentials(id: &str, credentials: Vec<HeldCredential>) -> Self {
        Self {
            id: id.to_owned(),
            credentials,
            access_token: None,
        }
    }

    /// The same caller, with `access_token` for its Credential Service
    /// (for adapters other than [`dcp::DcpVerifier`], and tests).
    #[must_use]
    pub fn with_access_token(self, access_token: &str) -> Self {
        Self {
            access_token: Some(access_token.to_owned()),
            ..self
        }
    }

    /// The access token for the caller's Credential Service, if any.
    #[must_use]
    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }
}

impl std::fmt::Debug for Caller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never the access token
        f.debug_struct("Caller")
            .field("id", &self.id)
            .field("credentials", &self.credentials.len())
            .finish_non_exhaustive()
    }
}

/// Why [`CallerVerifier::verify`] did not accept a caller. The text
/// says why; it goes to the log, never to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The caller is refused: no token or a bad one, a replayed token, an
    /// invalid or revoked credential, an untrusted issuer, or a service
    /// of the caller that answered with a final error (such as 404).
    /// [`require_caller`] answers HTTP 404, or 401 on the catalog routes.
    Rejected(String),
    /// The check could not finish: a service it needs (the caller's DID
    /// document or Credential Service, an issuer's DID document or status
    /// list, our own STS) did not answer in time, or answered 408, 429 or
    /// 5xx. The caller may be fine. [`require_caller`] answers HTTP 503,
    /// so the caller can try again later.
    Unavailable(String),
}

impl VerifyError {
    /// [`VerifyError::Unavailable`] when `temporary`, otherwise
    /// [`VerifyError::Rejected`].
    #[must_use]
    pub fn new(reason: String, temporary: bool) -> Self {
        if temporary {
            Self::Unavailable(reason)
        } else {
            Self::Rejected(reason)
        }
    }

    /// Whether trying again later may help ([`VerifyError::Unavailable`]).
    #[must_use]
    pub fn is_temporary(&self) -> bool {
        match self {
            Self::Rejected(_) => false,
            Self::Unavailable(_) => true,
        }
    }

    /// Why, for the log.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Rejected(reason) | Self::Unavailable(reason) => reason,
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

/// Lets `?` turn a text error into a [`VerifyError::Rejected`]: a check
/// that fails with text refuses the caller. Only the fetches mark a
/// failure as temporary, explicitly.
impl From<String> for VerifyError {
    fn from(reason: String) -> Self {
        Self::Rejected(reason)
    }
}

/// As `From<String>`, for fixed texts.
impl From<&str> for VerifyError {
    fn from(reason: &str) -> Self {
        Self::Rejected(reason.to_owned())
    }
}

/// Lets `?` turn a failed DID resolution into a [`VerifyError`]:
/// temporary when the DID's host did not answer or answered 408, 429 or
/// 5xx ([`ResolveError::is_temporary`]), a rejection otherwise.
impl From<ResolveError> for VerifyError {
    fn from(error: ResolveError) -> Self {
        let temporary = error.is_temporary();
        Self::new(error.reason, temporary)
    }
}

/// Port: who sent a DSP request.
pub trait CallerVerifier: Send + Sync {
    /// Checks `authorization`, the request's `Authorization` header, and
    /// returns the caller, with its verified credentials.
    ///
    /// # Errors
    ///
    /// The caller is refused ([`VerifyError::Rejected`]), or could not be
    /// checked now ([`VerifyError::Unavailable`]).
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Caller, VerifyError>>;

    /// The caller's verified credentials for `scopes` as well as the
    /// configured ones (ADR 0019): the credentials of `caller` when they
    /// cover `scopes` already, otherwise a new query of the caller's
    /// Credential Service with the access token it sent, checked like the
    /// first.
    ///
    /// # Errors
    ///
    /// The query failed or a credential did not pass the checks. The text
    /// goes to the log.
    fn credentials<'a>(
        &'a self,
        caller: &'a Caller,
        scopes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>>;
}

/// Port: how we authenticate ourselves on a DSP request.
pub trait Prover: Send + Sync {
    /// The `Authorization` header value for one request to the participant
    /// `audience` (its DID), for example `Bearer <token>`. The token lets
    /// `audience` read the configured scopes from our Credential Service,
    /// and `scopes` as well (the scopes an offer's credential constraints
    /// need, ADR 0019; usually none).
    ///
    /// # Errors
    ///
    /// No token could be had; the request is not sent.
    fn authorization<'a>(
        &'a self,
        audience: &'a str,
        scopes: &'a [String],
    ) -> BoxFuture<'a, Result<String, String>>;
}

/// What the domain evaluates a policy against for `caller` (ADR 0019):
/// the configured `types`, and the caller's verified credentials for
/// `scopes`, the scopes that policy needs ([`portitor_dsp::policy::credential_scopes`]).
/// Without scopes, the credentials of the check on the HTTP edge. A query
/// that fails gives no credentials, so every credential constraint fails
/// (closed); the reason goes to the log.
pub async fn credentials_for(
    verifier: &dyn CallerVerifier,
    types: &CredentialTypes,
    caller: &Caller,
    scopes: &[String],
) -> CallerCredentials {
    let held = if scopes.is_empty() {
        caller.credentials.clone()
    } else {
        match verifier.credentials(caller, scopes).await {
            Ok(held) => held,
            Err(reason) => {
                tracing::warn!(caller = %caller.id, %reason, "the caller's credentials for a policy could not be read");
                Vec::new()
            }
        }
    };
    CallerCredentials {
        understood: types.clone(),
        held,
    }
}

/// Adapter: refuses every caller. What [`crate::Ports::new`] starts with,
/// so a connector without a configured verifier is closed, not open.
#[derive(Debug, Clone, Copy, Default)]
pub struct RefuseAll;

impl CallerVerifier for RefuseAll {
    fn verify<'a>(&'a self, _: Option<&'a str>) -> BoxFuture<'a, Result<Caller, VerifyError>> {
        Box::pin(async { Err(VerifyError::from("no caller verifier configured")) })
    }

    fn credentials<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
        Box::pin(async { Err("no caller verifier configured".to_owned()) })
    }
}

/// Adapter: has no tokens, so nothing is sent. What
/// [`crate::Ports::new`] starts with.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTokens;

impl Prover for NoTokens {
    fn authorization<'a>(
        &'a self,
        _: &'a str,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async { Err("no token source configured".to_owned()) })
    }
}

/// The verifier chosen at start.
pub type AnyVerifier = Arc<dyn CallerVerifier>;

/// Middleware for every DSP route: runs the verifier, and either passes
/// the request on with its [`Caller`], or answers:
///
/// - when the caller is refused ([`VerifyError::Rejected`]): HTTP 404 on
///   the negotiation and transfer routes, HTTP 401 on the catalog routes
///   ([`refusal_status`]), with a DSP error that says no more than
///   "unauthorized";
/// - HTTP 503 with a problem body when the check could not finish
///   because a service it needs is down ([`VerifyError::Unavailable`]).
///   A partner's outbox tries a 503 again, but takes a 401 or a 404 as
///   final (ADR 0007), so a short outage at its issuer does not end a
///   negotiation.
///
/// The reason goes to the log only (WARN), never the token.
pub async fn require_caller(
    State(verifier): State<AnyVerifier>,
    mut request: Request,
    next: Next,
) -> Response {
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match verifier.verify(authorization).await {
        Ok(caller) => {
            request.extensions_mut().insert(caller);
            next.run(request).await
        }
        Err(error) if error.is_temporary() => {
            let path = request.uri().path();
            let reason = error.reason();
            tracing::warn!(%path, %reason, "DSP request not checked: a service the check needs is unavailable; answered 503");
            check_unavailable()
        }
        Err(error) => {
            let path = request.uri().path();
            let reason = error.reason();
            let status = refusal_status(path).as_u16();
            // the caller sees 404 on most routes, which looks like a
            // missing process: this line is where an operator finds why
            tracing::warn!(%path, %reason, status, "DSP request refused: caller not authenticated");
            metrics::dsp_request_refused(Refusal::Unauthenticated);
            unauthorized(path)
        }
    }
}

/// HTTP 503 with a problem body: the caller could not be checked now.
/// No reason: it would tell the caller which service failed.
fn check_unavailable() -> Response {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "the caller cannot be checked now; try again later",
    )
}

/// Which of the three DSP protocols the endpoint at `path` belongs to.
/// Each has its own error type and its own rule for a refused caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    Catalog,
    Negotiation,
    Transfer,
}

/// The protocol of the DSP endpoint at `path`, read from the first path
/// segment after the DSP prefix (or the callback prefix, for the
/// consumer's endpoints). Only that segment counts: later ones are ids,
/// the caller's text, and may hold any word. Anything else is taken as
/// the catalog, whose answer tells the least.
fn protocol_of(path: &str) -> Protocol {
    // the callback prefix first: it starts with the DSP prefix
    let below_prefix = path
        .strip_prefix(crate::CALLBACK_PATH)
        .or_else(|| path.strip_prefix(crate::DSP_PATH))
        .unwrap_or(path);
    let first_segment = below_prefix
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    match first_segment {
        "negotiations" => Protocol::Negotiation,
        "transfers" => Protocol::Transfer,
        _ => Protocol::Catalog,
    }
}

/// The HTTP status for a caller that is refused at `path`:
///
/// - negotiation and transfer routes, also the consumer's callbacks: 404.
///   DSP 2025-1, "Contract Negotiation HTTPS Binding" and "Transfer
///   Process HTTPS Binding", "Unauthorized Access": "If the client is not
///   authorized, the Consumer or Provider MUST return an HTTP 404 (Not
///   Found) response." A caller so cannot tell whether a process exists.
/// - catalog routes: 401. The "Catalog HTTPS Binding" only asks for "an
///   appropriate HTTP code", and 401 says more.
///
/// Eclipse EDC 1.0.0-RC7 does the same (`DspRequestHandlerImpl`,
/// `unauthorizedStatus`). ADR 0016, amended in 0.12.
fn refusal_status(path: &str) -> StatusCode {
    match protocol_of(path) {
        Protocol::Catalog => StatusCode::UNAUTHORIZED,
        Protocol::Negotiation | Protocol::Transfer => StatusCode::NOT_FOUND,
    }
}

/// The answer to a refused caller: the status [`refusal_status`] gives,
/// with the DSP error type of the endpoint at `path` (DSP 2025-1:
/// `CatalogError`, `ContractNegotiationError`, `TransferError`). The
/// error's `code` is the HTTP status. No process ids: the caller is
/// unknown.
fn unauthorized(path: &str) -> Response {
    const REASON: &str = "unauthorized";
    let status = refusal_status(path);
    let code = status.as_str();
    match protocol_of(path) {
        Protocol::Catalog => (status, Json(CatalogError::new(code, REASON))).into_response(),
        Protocol::Negotiation => {
            let error = ContractNegotiationError::new("", "", code, REASON);
            (status, Json(error)).into_response()
        }
        Protocol::Transfer => {
            (status, Json(TransferError::new("", "", code, REASON))).into_response()
        }
    }
}

/// The scope asked for when `PORTITOR_DCP_SCOPES` is not set: the DCP
/// alias for "credentials of type `MembershipCredential`", the DCP TCK's
/// default too.
pub const DEFAULT_SCOPE: &str = portitor_dsp::policy::DEFAULT_MEMBERSHIP_SCOPE;

/// The DCP alias for "credentials of this type", followed by `:`.
const VC_TYPE_PREFIX: &str = "org.eclipse.dspace.dcp.vc.type:";

/// Reads `PORTITOR_POLICY_CREDENTIALS` (ADR 0019): the credential types
/// that policies may use, comma-separated, each `Type` or `Type=scope`. A
/// type alone gets the scope `org.eclipse.dspace.dcp.vc.type:<Type>:read`
/// (`:read` for EDC, reported upstream: <https://github.com/eclipse-edc/IdentityHub/issues/1278>).
/// `MembershipCredential` is always there; unless listed, its scope is
/// the `MembershipCredential` scope of `PORTITOR_DCP_SCOPES` (default
/// [`DEFAULT_SCOPE`]), which every query asks for anyway.
///
/// # Errors
///
/// An entry with an empty or odd type (it must not hold `.`, `:`, `/` or
/// spaces), a type listed twice, or a scope that is not a DCP scope by
/// that same type.
pub fn policy_credentials(
    var: impl Fn(&str) -> Option<OsString>,
) -> Result<CredentialTypes, String> {
    let text = |name: &str| var(name).map(|v| v.to_string_lossy().trim().to_owned());
    let dcp_scopes = text("PORTITOR_DCP_SCOPES").map(|scopes| comma_list(&scopes));
    let membership_scope = dcp_scopes
        .unwrap_or_default()
        .into_iter()
        .find(|scope| scope_type(scope).as_deref() == Some(MEMBERSHIP))
        .unwrap_or_else(|| DEFAULT_SCOPE.to_owned());
    let mut types = CredentialTypes::new(&membership_scope);
    let mut listed: Vec<String> = Vec::new();
    for entry in comma_list(&text("PORTITOR_POLICY_CREDENTIALS").unwrap_or_default()) {
        let (credential_type, scope) = parse_policy_entry(&entry)?;
        let credential_type = credential_type.as_str();
        if listed.iter().any(|t| t == credential_type) {
            return Err(format!(
                "PORTITOR_POLICY_CREDENTIALS lists {credential_type} twice"
            ));
        }
        if scope_type(&scope).as_deref() != Some(credential_type) {
            return Err(format!(
                "PORTITOR_POLICY_CREDENTIALS: the scope {scope:?} is not a DCP scope for credentials of type {credential_type}"
            ));
        }
        types.add(credential_type, &scope);
        listed.push(credential_type.to_owned());
    }
    Ok(types)
}

/// One entry of `PORTITOR_POLICY_CREDENTIALS`: `Type` or `Type=scope`,
/// read as the type and its scope (for `Type` alone,
/// `org.eclipse.dspace.dcp.vc.type:<Type>:read`; see <https://github.com/eclipse-edc/IdentityHub/issues/1278>). Whether the scope fits
/// the type is checked by the caller.
///
/// # Errors
/// The type is empty or has a character a type name cannot have.
fn parse_policy_entry(entry: &str) -> Result<(String, String), String> {
    let (credential_type, scope) = match entry.split_once('=') {
        Some((credential_type, scope)) => (credential_type.trim(), scope.trim().to_owned()),
        None => (entry, format!("{VC_TYPE_PREFIX}{entry}:read")),
    };
    if credential_type.is_empty() || credential_type.contains(not_in_type_name) {
        return Err(format!(
            "PORTITOR_POLICY_CREDENTIALS: {entry:?} is not Type or Type=scope"
        ));
    }
    Ok((credential_type.to_owned(), scope))
}

/// Whether `c` cannot be part of a credential type name here: `.` and
/// `/` would make the left operand `<Type>.<claim>` ambiguous, `:` would
/// break the scope, and spaces are a typo.
fn not_in_type_name(c: char) -> bool {
    c == '.' || c == ':' || c == '/' || c.is_whitespace()
}

/// The credential type a DCP "by type" scope asks for; `None` for any
/// other scope.
fn scope_type(scope: &str) -> Option<String> {
    match Scope::parse(scope, &[]) {
        Ok(Scope::ByType {
            credential_type, ..
        }) => Some(credential_type),
        _ => None,
    }
}

/// The scopes `scopes`, and after them, for each "by type" scope of
/// `ParticipantComplianceCredential`, the scope of `LegalPerson` with the
/// same alias and access part (ADR 0022). A verifier can only accept a
/// Gaia-X compliance credential together with the legal person credential
/// it names, so both sides ask for both: the consumer when it asks its
/// STS for a token, the provider when it queries the consumer's
/// Credential Service. Each scope is listed once.
///
/// For example `org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read`
/// adds `org.eclipse.dspace.dcp.vc.type:LegalPerson:read`.
#[must_use]
pub fn with_linked_scopes(scopes: &[String]) -> Vec<String> {
    let mut linked = Vec::new();
    for scope in scopes {
        if let Some(legal_person) = legal_person_scope(scope) {
            linked.push(legal_person);
        }
    }
    merge_scopes(scopes, &linked)
}

/// The `LegalPerson` scope that goes with `scope`, when `scope` asks for
/// credentials of type `ParticipantComplianceCredential` (any alias, so
/// EDC's own aliases work too); `None` for any other scope.
fn legal_person_scope(scope: &str) -> Option<String> {
    // a scope is alias:type, or alias:type:access (see portitor_dcp::scope)
    let (alias, discriminator) = scope.split_once(':')?;
    if alias == VC_ID {
        // one credential by its id, not a type
        return None;
    }
    let (credential_type, access) = match discriminator.split_once(':') {
        Some((credential_type, access)) => (credential_type, Some(access)),
        None => (discriminator, None),
    };
    if credential_type != COMPLIANCE_SCOPE_TYPE {
        return None;
    }
    match access {
        Some(access) => Some(format!("{alias}:{LEGAL_PERSON_SCOPE_TYPE}:{access}")),
        None => Some(format!("{alias}:{LEGAL_PERSON_SCOPE_TYPE}")),
    }
}

/// The connector's DCP settings (ADR 0016).
#[derive(Clone)]
pub struct AuthSettings {
    /// Our participant id (`PORTITOR_PARTICIPANT_ID`): the audience of
    /// incoming tokens and the issuer of ours.
    pub participant_id: String,
    /// Our STS token endpoint (`PORTITOR_STS_URL`).
    pub sts_url: String,
    /// Allow a plain-HTTP `sts_url`, and only that URL
    /// (`PORTITOR_STS_ALLOW_HTTP`): the STS on an internal network, next
    /// to the connector.
    pub sts_allow_http: bool,
    /// Our STS client id (`PORTITOR_STS_CLIENT_ID`, default the
    /// participant id).
    pub sts_client_id: String,
    /// Our STS client secret, read from `PORTITOR_STS_CLIENT_SECRET_FILE`.
    pub sts_client_secret: String,
    /// DIDs trusted to issue `MembershipCredential`
    /// (`PORTITOR_TRUSTED_ISSUERS`, comma-separated).
    pub trusted_issuers: Vec<String>,
    /// DIDs of the Gaia-X compliance services trusted to issue
    /// `gx:ParticipantComplianceCredential`
    /// (`PORTITOR_GAIAX_COMPLIANCE_ISSUERS`, comma-separated; ADR 0022).
    /// Empty: no compliance credential is accepted.
    pub gaiax_compliance_issuers: Vec<String>,
    /// Scopes asked for, on both sides (`PORTITOR_DCP_SCOPES`,
    /// comma-separated).
    pub scopes: Vec<String>,
    /// Allow plain-HTTP DIDs and URLs (`PORTITOR_ALLOW_HTTP_CALLBACKS`;
    /// tests and the TCKs only).
    pub allow_http: bool,
    /// How long a verified status list credential is kept, in seconds
    /// (`PORTITOR_STATUS_LIST_CACHE_SECONDS`, default 300, at most 3600;
    /// 0: fetched on every check). See [`dcp::status_list_cache`].
    pub status_list_cache_seconds: u64,
    /// How long a resolved DID document is reused, in seconds
    /// (`PORTITOR_DID_CACHE_SECONDS`, default 300, at most 3600; 0:
    /// fetched on every use). While a document is kept, a key removed
    /// from it is still accepted (R-41). See [`portitor_runtime::did`].
    pub did_cache_seconds: u64,
}

impl std::fmt::Debug for AuthSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never the secret
        f.debug_struct("AuthSettings")
            .field("participant_id", &self.participant_id)
            .field("sts_url", &self.sts_url)
            .field("sts_allow_http", &self.sts_allow_http)
            .field("sts_client_id", &self.sts_client_id)
            .field("trusted_issuers", &self.trusted_issuers)
            .field("gaiax_compliance_issuers", &self.gaiax_compliance_issuers)
            .field("scopes", &self.scopes)
            .field("allow_http", &self.allow_http)
            .field("status_list_cache_seconds", &self.status_list_cache_seconds)
            .field("did_cache_seconds", &self.did_cache_seconds)
            .finish_non_exhaustive()
    }
}

impl AuthSettings {
    /// Reads the settings from the environment. `None` when
    /// `PORTITOR_STS_URL` is not set, which only the `tck` build accepts
    /// (see `tck.rs`).
    ///
    /// # Errors
    ///
    /// See [`AuthSettings::from_vars`].
    pub fn from_env(participant_id: &str, allow_http: bool) -> Result<Option<Self>, String> {
        Self::from_vars(participant_id, allow_http, |name| std::env::var_os(name))
    }

    /// Like [`AuthSettings::from_env`], with the variables looked up by
    /// `var`.
    ///
    /// # Errors
    ///
    /// The STS URL is plain HTTP without `PORTITOR_STS_ALLOW_HTTP`, the
    /// secret file cannot be read or is empty, no trusted issuer is set,
    /// a Gaia-X compliance service is not a `did:web` DID, a scope is
    /// empty, not a DCP scope, or asks for every credential (type
    /// `VerifiableCredential`), or `PORTITOR_STATUS_LIST_CACHE_SECONDS` is
    /// not a number from 0 to 3600.
    pub fn from_vars(
        participant_id: &str,
        allow_http: bool,
        var: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Option<Self>, String> {
        let text = |name: &str| var(name).map(|v| v.to_string_lossy().trim().to_owned());
        let Some(sts_url) = text("PORTITOR_STS_URL").filter(|url| !url.is_empty()) else {
            return Ok(None);
        };
        let sts_allow_http =
            text("PORTITOR_STS_ALLOW_HTTP").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let plain_http = sts_url
            .get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"));
        if plain_http && !sts_allow_http && !allow_http {
            return Err(format!(
                "PORTITOR_STS_URL {sts_url} is plain HTTP: set PORTITOR_STS_ALLOW_HTTP=true if the STS is on an internal network"
            ));
        }
        let secret_file = text("PORTITOR_STS_CLIENT_SECRET_FILE")
            .ok_or("PORTITOR_STS_CLIENT_SECRET_FILE is required with PORTITOR_STS_URL")?;
        let sts_client_secret = std::fs::read_to_string(&secret_file)
            .map_err(|e| format!("cannot read PORTITOR_STS_CLIENT_SECRET_FILE {secret_file}: {e}"))?
            .trim()
            .to_owned();
        if sts_client_secret.is_empty() {
            return Err(format!("the STS client secret file {secret_file} is empty"));
        }
        let trusted_issuers = comma_list(
            text("PORTITOR_TRUSTED_ISSUERS")
                .as_deref()
                .unwrap_or_default(),
        );
        if trusted_issuers.is_empty() {
            return Err("PORTITOR_TRUSTED_ISSUERS is required: the DIDs trusted to issue MembershipCredential".to_owned());
        }
        let gaiax_compliance_issuers = comma_list(
            text("PORTITOR_GAIAX_COMPLIANCE_ISSUERS")
                .as_deref()
                .unwrap_or_default(),
        );
        for did in &gaiax_compliance_issuers {
            // resolved like any did:web DID (with a path, as the Gaia-X
            // Lab's did:web:compliance.lab.gaia-x.eu:v2)
            portitor_dcp::did_web_url(did)
                .map_err(|e| format!("PORTITOR_GAIAX_COMPLIANCE_ISSUERS: {did}: {e}"))?;
        }
        let scopes = match text("PORTITOR_DCP_SCOPES") {
            Some(scopes) => comma_list(&scopes),
            None => vec![DEFAULT_SCOPE.to_owned()],
        };
        if scopes.is_empty() {
            return Err("PORTITOR_DCP_SCOPES is set but empty".to_owned());
        }
        for scope in &scopes {
            // the DCP aliases only: they are what other Credential Services
            // must understand
            Scope::parse(scope, &[]).map_err(|e| format!("PORTITOR_DCP_SCOPES: {e}"))?;
        }
        let status_list_cache_seconds =
            status_list_cache_seconds(text("PORTITOR_STATUS_LIST_CACHE_SECONDS").as_deref())?;
        let did_cache_seconds =
            portitor_runtime::did::cache_seconds(text(DID_CACHE_SETTING).as_deref())?;
        Ok(Some(Self {
            participant_id: participant_id.to_owned(),
            sts_url,
            sts_allow_http,
            sts_client_id: text("PORTITOR_STS_CLIENT_ID")
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| participant_id.to_owned()),
            sts_client_secret,
            trusted_issuers,
            gaiax_compliance_issuers,
            scopes,
            allow_http,
            status_list_cache_seconds,
            did_cache_seconds,
        }))
    }

    /// A warning for the start log when the policy types and these
    /// settings do not fit: as provider, a policy on
    /// `ParticipantComplianceCredential` never holds without a trusted
    /// Gaia-X compliance service (ADR 0022). Only a warning: a consumer
    /// lists the type too, to grant the scopes, and needs no compliance
    /// service.
    #[must_use]
    pub fn policy_types_warning(&self, types: &CredentialTypes) -> Option<String> {
        if types.scope(COMPLIANCE_SCOPE_TYPE).is_some() && self.gaiax_compliance_issuers.is_empty()
        {
            return Some(format!(
                "PORTITOR_POLICY_CREDENTIALS lists {COMPLIANCE_SCOPE_TYPE} but PORTITOR_GAIAX_COMPLIANCE_ISSUERS is empty: as provider, offers that require Gaia-X compliance admit no one"
            ));
        }
        None
    }
}

/// Reads `PORTITOR_STATUS_LIST_CACHE_SECONDS` (`value`): whole seconds
/// from 0 to [`dcp::status_list_cache::MAX_SECONDS`];
/// [`dcp::status_list_cache::DEFAULT_SECONDS`] when it is not set or
/// empty ([`bounded_number`]).
fn status_list_cache_seconds(value: Option<&str>) -> Result<u64, String> {
    use dcp::status_list_cache::{DEFAULT_SECONDS, MAX_SECONDS};
    let name = "PORTITOR_STATUS_LIST_CACHE_SECONDS";
    bounded_number(name, value, 0..=MAX_SECONDS, DEFAULT_SECONDS)
}

/// The DCP adapters for `settings`: the verifier and the prover, both
/// using our STS. Before it returns, it checks that our identity is the
/// one configured ([`dcp::DcpVerifier::check_own_identity`]).
///
/// # Errors
///
/// The HTTP client cannot be built, or the identity check failed.
pub async fn dcp_adapters(
    settings: &AuthSettings,
) -> Result<(Arc<dyn CallerVerifier>, Arc<dyn Prover>), String> {
    let sts: Arc<dyn sts::SecureTokenService> = Arc::new(sts::StsClient::new(settings)?);
    let verifier = dcp::DcpVerifier::new(settings, Arc::clone(&sts))?;
    verifier.check_own_identity().await?;
    let prover = sts::StsProver::new(sts, settings.scopes.clone());
    Ok((Arc::new(verifier), Arc::new(prover)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_runtime::testing::vars;

    #[test]
    fn reads_the_settings() {
        let dir = std::env::temp_dir().join(format!("portitor-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("secret");
        std::fs::write(&secret, "s3cret\n").unwrap();
        let secret = secret.to_string_lossy().into_owned();
        let me = "did:web:me.example";
        // no STS URL: not configured
        assert!(
            AuthSettings::from_vars(me, false, vars(&[]))
                .unwrap()
                .is_none()
        );
        let s = AuthSettings::from_vars(
            me,
            false,
            vars(&[
                ("PORTITOR_STS_URL", "https://me.example/sts/token"),
                ("PORTITOR_STS_CLIENT_SECRET_FILE", &secret),
                ("PORTITOR_TRUSTED_ISSUERS", " did:web:a , ,did:web:b"),
            ]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(s.sts_client_id, me);
        assert_eq!(s.sts_client_secret, "s3cret");
        assert_eq!(s.trusted_issuers, ["did:web:a", "did:web:b"]);
        assert_eq!(s.scopes, [DEFAULT_SCOPE]);
        assert!(s.gaiax_compliance_issuers.is_empty());
        assert!(!format!("{s:?}").contains("s3cret"));
        let s = AuthSettings::from_vars(
            me,
            true,
            vars(&[
                ("PORTITOR_STS_URL", "https://me.example/sts/token"),
                ("PORTITOR_STS_CLIENT_ID", "client"),
                ("PORTITOR_STS_CLIENT_SECRET_FILE", &secret),
                ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
                (
                    "PORTITOR_DCP_SCOPES",
                    "org.eclipse.dspace.dcp.vc.type:A,org.eclipse.dspace.dcp.vc.id:B",
                ),
            ]),
        )
        .unwrap()
        .unwrap();
        assert_eq!((s.sts_client_id.as_str(), s.scopes.len()), ("client", 2));
        assert!(!s.sts_allow_http);
        for missing in [
            vec![
                ("PORTITOR_STS_URL", "u"),
                ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
            ],
            vec![
                ("PORTITOR_STS_URL", "u"),
                ("PORTITOR_STS_CLIENT_SECRET_FILE", &secret),
            ],
            vec![
                ("PORTITOR_STS_URL", "u"),
                ("PORTITOR_STS_CLIENT_SECRET_FILE", "/nonexistent/secret"),
                ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
            ],
            vec![
                ("PORTITOR_STS_URL", "u"),
                ("PORTITOR_STS_CLIENT_SECRET_FILE", &secret),
                ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
                ("PORTITOR_DCP_SCOPES", " , "),
            ],
        ] {
            assert!(
                AuthSettings::from_vars(me, false, vars(&missing)).is_err(),
                "{missing:?}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A secret file in a fresh directory; the directory is removed by
    /// the caller.
    fn secret_file(name: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("portitor-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("secret");
        std::fs::write(&secret, "s3cret\n").unwrap();
        let path = secret.to_string_lossy().into_owned();
        (dir, path)
    }

    /// `PORTITOR_DID_CACHE_SECONDS`: 300 by default, 0 for no cache, at
    /// most 3600; the same rules as the status list cache.
    #[test]
    fn the_did_cache_setting_is_bounded() {
        let (dir, secret) = secret_file("did-cache");
        let me = "did:web:me.example";
        let base = [
            ("PORTITOR_STS_URL", "https://me.example/sts/token"),
            ("PORTITOR_STS_CLIENT_SECRET_FILE", secret.as_str()),
            ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
        ];
        let read = |value: Option<&str>| {
            let mut with = base.to_vec();
            if let Some(value) = value {
                with.push(("PORTITOR_DID_CACHE_SECONDS", value));
            }
            AuthSettings::from_vars(me, false, vars(&with))
                .map(|settings| settings.unwrap().did_cache_seconds)
        };
        assert_eq!(read(None), Ok(300));
        assert_eq!(read(Some("")), Ok(300));
        assert_eq!(read(Some("0")), Ok(0));
        assert_eq!(read(Some(" 3600 ")), Ok(3600));
        for refused in ["3601", "-1", "5m"] {
            let err = read(Some(refused)).unwrap_err();
            assert!(
                err.starts_with("PORTITOR_DID_CACHE_SECONDS"),
                "{refused}: {err}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_status_list_cache_setting_is_bounded() {
        let (dir, secret) = secret_file("status-cache");
        let me = "did:web:me.example";
        let base = [
            ("PORTITOR_STS_URL", "https://me.example/sts/token"),
            ("PORTITOR_STS_CLIENT_SECRET_FILE", secret.as_str()),
            ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
        ];
        let read = |value: Option<&str>| {
            let mut with = base.to_vec();
            if let Some(value) = value {
                with.push(("PORTITOR_STATUS_LIST_CACHE_SECONDS", value));
            }
            AuthSettings::from_vars(me, false, vars(&with))
                .map(|settings| settings.unwrap().status_list_cache_seconds)
        };
        assert_eq!(read(None), Ok(300));
        // empty is not set, as for every number setting
        assert_eq!(read(Some("")), Ok(300));
        assert_eq!(read(Some("0")), Ok(0));
        assert_eq!(read(Some(" 3600 ")), Ok(3600));
        for refused in ["3601", "-1", "5m"] {
            let err = read(Some(refused)).unwrap_err();
            assert!(
                err.starts_with("PORTITOR_STATUS_LIST_CACHE_SECONDS"),
                "{refused}: {err}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_plain_http_sts_needs_its_flag() {
        let (dir, secret) = secret_file("sts-http");
        let me = "did:web:me.example";
        let internal = [
            ("PORTITOR_STS_URL", "http://identity:8090/sts/token"),
            ("PORTITOR_STS_CLIENT_SECRET_FILE", secret.as_str()),
            ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
        ];
        let err = AuthSettings::from_vars(me, false, vars(&internal)).unwrap_err();
        assert!(err.contains("PORTITOR_STS_ALLOW_HTTP"), "{err}");
        let mut allowed = internal.to_vec();
        allowed.push(("PORTITOR_STS_ALLOW_HTTP", "true"));
        let s = AuthSettings::from_vars(me, false, vars(&allowed))
            .unwrap()
            .unwrap();
        assert!(s.sts_allow_http && !s.allow_http);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scopes_are_checked_at_start() {
        let (dir, secret) = secret_file("scopes");
        let me = "did:web:me.example";
        let base = [
            ("PORTITOR_STS_URL", "https://me.example/sts/token"),
            ("PORTITOR_STS_CLIENT_SECRET_FILE", secret.as_str()),
            ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
        ];
        for (scopes, reason) in [
            ("no-colon", "not of the form alias:discriminator"),
            (
                "org.eclipse.dspace.dcp.vc.type:",
                "not of the form alias:discriminator",
            ),
            ("com.example:MembershipCredential", "is not supported"),
            (
                "org.eclipse.dspace.dcp.vc.type:VerifiableCredential",
                "too broad",
            ),
        ] {
            let mut with = base.to_vec();
            with.push(("PORTITOR_DCP_SCOPES", scopes));
            let err = AuthSettings::from_vars(me, false, vars(&with)).unwrap_err();
            assert!(
                err.starts_with("PORTITOR_DCP_SCOPES") && err.contains(reason),
                "{scopes}: {err}"
            );
        }
        // EDC's scope, with its access part
        let mut edc = base.to_vec();
        edc.push((
            "PORTITOR_DCP_SCOPES",
            "org.eclipse.dspace.dcp.vc.type:MembershipCredential:read",
        ));
        assert!(AuthSettings::from_vars(me, false, vars(&edc)).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn policy_credentials_are_read_and_checked() {
        let types = policy_credentials(vars(&[])).unwrap();
        assert_eq!(types, CredentialTypes::default());
        let types = policy_credentials(vars(&[
            (
                "PORTITOR_POLICY_CREDENTIALS",
                " DataProcessorCredential , Other=org.eclipse.dspace.dcp.vc.type:Other",
            ),
            (
                "PORTITOR_DCP_SCOPES",
                "org.eclipse.dspace.dcp.vc.id:x,org.eclipse.dspace.dcp.vc.type:MembershipCredential:read",
            ),
        ]))
        .unwrap();
        let listed: Vec<(&str, &str)> = types.iter().collect();
        assert_eq!(
            listed,
            [
                (
                    "DataProcessorCredential",
                    "org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read"
                ),
                (
                    "MembershipCredential",
                    "org.eclipse.dspace.dcp.vc.type:MembershipCredential:read"
                ),
                ("Other", "org.eclipse.dspace.dcp.vc.type:Other"),
            ]
        );
        // the membership's scope may be set here too
        let types = policy_credentials(vars(&[(
            "PORTITOR_POLICY_CREDENTIALS",
            "MembershipCredential=org.eclipse.dspace.dcp.vc.type:MembershipCredential:read",
        )]))
        .unwrap();
        assert_eq!(
            types.scope("MembershipCredential"),
            Some("org.eclipse.dspace.dcp.vc.type:MembershipCredential:read")
        );
        for (value, reason) in [
            ("A.b", "not Type or Type=scope"),
            (
                "=org.eclipse.dspace.dcp.vc.type:A",
                "not Type or Type=scope",
            ),
            ("ex:A", "not Type or Type=scope"),
            ("A,A", "twice"),
            ("A=org.eclipse.dspace.dcp.vc.type:B", "not a DCP scope"),
            ("A=org.eclipse.dspace.dcp.vc.id:A", "not a DCP scope"),
            ("A=com.example:A", "not a DCP scope"),
            ("VerifiableCredential", "not a DCP scope"),
        ] {
            let err =
                policy_credentials(vars(&[("PORTITOR_POLICY_CREDENTIALS", value)])).unwrap_err();
            assert!(err.contains(reason), "{value}: {err}");
        }
    }

    #[test]
    fn gaiax_compliance_issuers_are_read_and_checked() {
        let (dir, secret) = secret_file("gaiax");
        let me = "did:web:me.example";
        let base = [
            ("PORTITOR_STS_URL", "https://me.example/sts/token"),
            ("PORTITOR_STS_CLIENT_SECRET_FILE", secret.as_str()),
            ("PORTITOR_TRUSTED_ISSUERS", "did:web:a"),
        ];
        let mut with = base.to_vec();
        with.push((
            "PORTITOR_GAIAX_COMPLIANCE_ISSUERS",
            "did:web:compliance.lab.gaia-x.eu:v2, did:web:gx.example",
        ));
        let s = AuthSettings::from_vars(me, false, vars(&with))
            .unwrap()
            .unwrap();
        assert_eq!(
            s.gaiax_compliance_issuers,
            ["did:web:compliance.lab.gaia-x.eu:v2", "did:web:gx.example"]
        );
        let mut types = CredentialTypes::default();
        assert_eq!(s.policy_types_warning(&types), None);
        types.add(
            COMPLIANCE_SCOPE_TYPE,
            "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read",
        );
        assert_eq!(s.policy_types_warning(&types), None);
        // a compliance policy without a trusted compliance service
        let without = AuthSettings::from_vars(me, false, vars(&base))
            .unwrap()
            .unwrap();
        let err = without.policy_types_warning(&types).unwrap();
        assert!(err.contains("PORTITOR_GAIAX_COMPLIANCE_ISSUERS"), "{err}");
        // not a did:web DID
        let mut bad = base.to_vec();
        bad.push(("PORTITOR_GAIAX_COMPLIANCE_ISSUERS", "did:key:z6Mk"));
        let err = AuthSettings::from_vars(me, false, vars(&bad)).unwrap_err();
        assert!(
            err.starts_with("PORTITOR_GAIAX_COMPLIANCE_ISSUERS"),
            "{err}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_compliance_scope_brings_the_legal_person_scope() {
        let list = |scopes: &[&str]| -> Vec<String> {
            scopes.iter().map(|scope| (*scope).to_owned()).collect()
        };
        let cases = [
            (vec![], vec![]),
            (vec![DEFAULT_SCOPE], vec![DEFAULT_SCOPE]),
            (
                vec![
                    DEFAULT_SCOPE,
                    "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read",
                ],
                vec![
                    DEFAULT_SCOPE,
                    "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read",
                    "org.eclipse.dspace.dcp.vc.type:LegalPerson:read",
                ],
            ),
            // the same alias and access part, EDC's alias too
            (
                vec!["org.eclipse.edc.vc.type:ParticipantComplianceCredential"],
                vec![
                    "org.eclipse.edc.vc.type:ParticipantComplianceCredential",
                    "org.eclipse.edc.vc.type:LegalPerson",
                ],
            ),
            // listed already: once
            (
                vec![
                    "org.eclipse.dspace.dcp.vc.type:LegalPerson",
                    "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential",
                ],
                vec![
                    "org.eclipse.dspace.dcp.vc.type:LegalPerson",
                    "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential",
                ],
            ),
            // a credential id is not a type
            (
                vec!["org.eclipse.dspace.dcp.vc.id:ParticipantComplianceCredential"],
                vec!["org.eclipse.dspace.dcp.vc.id:ParticipantComplianceCredential"],
            ),
        ];
        for (scopes, expected) in cases {
            assert_eq!(
                with_linked_scopes(&list(&scopes)),
                list(&expected),
                "{scopes:?}"
            );
        }
    }

    /// DSP 2025-1, "Unauthorized Access" in the negotiation and transfer
    /// HTTPS bindings: 404. The catalog binding: "an appropriate HTTP
    /// code", here 401 (as Eclipse EDC, `unauthorizedStatus`).
    #[tokio::test]
    async fn refusals_have_the_endpoint_error_type_and_status() {
        let (unauthorized_401, not_found) = (StatusCode::UNAUTHORIZED, StatusCode::NOT_FOUND);
        for (path, kind, status) in [
            (
                "/dsp/2025-1/catalog/request",
                "CatalogError",
                unauthorized_401,
            ),
            (
                "/dsp/2025-1/catalog/datasets/d1",
                "CatalogError",
                unauthorized_401,
            ),
            // a dataset id that looks like another protocol's path
            (
                "/dsp/2025-1/catalog/datasets/negotiations",
                "CatalogError",
                unauthorized_401,
            ),
            (
                "/dsp/2025-1/negotiations/request",
                "ContractNegotiationError",
                not_found,
            ),
            (
                "/dsp/2025-1/callback/negotiations/x/agreement",
                "ContractNegotiationError",
                not_found,
            ),
            ("/dsp/2025-1/transfers/x", "TransferError", not_found),
            // process ids that look like another protocol's path
            (
                "/dsp/2025-1/transfers/negotiations",
                "TransferError",
                not_found,
            ),
            (
                "/dsp/2025-1/negotiations/catalog/events",
                "ContractNegotiationError",
                not_found,
            ),
            (
                "/dsp/2025-1/callback/transfers/x/start",
                "TransferError",
                not_found,
            ),
        ] {
            let response = unauthorized(path);
            assert_eq!(response.status(), status, "{path}");
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["@type"], kind, "{path}");
            assert_eq!(body["code"], status.as_str(), "{path}");
            assert_eq!(body["reason"][0], "unauthorized", "{path}");
        }
    }

    #[tokio::test]
    async fn a_check_that_could_not_finish_answers_503_and_a_refusal_401_or_404() {
        use axum::routing::get;
        use tower::ServiceExt as _;

        /// Fails every check with `error`.
        struct Fails(VerifyError);

        impl CallerVerifier for Fails {
            fn verify<'a>(
                &'a self,
                _: Option<&'a str>,
            ) -> BoxFuture<'a, Result<Caller, VerifyError>> {
                Box::pin(async { Err(self.0.clone()) })
            }

            fn credentials<'a>(
                &'a self,
                _: &'a Caller,
                _: &'a [String],
            ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
                Box::pin(async { Ok(Vec::new()) })
            }
        }

        const CATALOG: &str = "/dsp/2025-1/catalog/request";
        const NEGOTIATION: &str = "/dsp/2025-1/negotiations/p1";
        const TRANSFER: &str = "/dsp/2025-1/transfers/p1";
        let down = VerifyError::Unavailable("status list: no answer within 10s".to_owned());
        let refused = VerifyError::Rejected("a credential is revoked".to_owned());
        for (error, path, expected) in [
            // a check that could not finish: 503 on every route
            (down.clone(), CATALOG, StatusCode::SERVICE_UNAVAILABLE),
            (down.clone(), NEGOTIATION, StatusCode::SERVICE_UNAVAILABLE),
            (down, TRANSFER, StatusCode::SERVICE_UNAVAILABLE),
            (refused.clone(), CATALOG, StatusCode::UNAUTHORIZED),
            (refused.clone(), NEGOTIATION, StatusCode::NOT_FOUND),
            (refused, TRANSFER, StatusCode::NOT_FOUND),
        ] {
            let verifier: AnyVerifier = Arc::new(Fails(error));
            let app = axum::Router::new()
                .route(CATALOG, get(|| async { "ok" }))
                .route(NEGOTIATION, get(|| async { "ok" }))
                .route(TRANSFER, get(|| async { "ok" }))
                .route_layer(axum::middleware::from_fn_with_state(
                    verifier,
                    require_caller,
                ));
            let request = Request::get(path).body(axum::body::Body::empty()).unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), expected, "{path}");
        }
    }

    #[test]
    fn verify_errors_say_whether_to_try_again() {
        let down = VerifyError::new("x".to_owned(), true);
        assert!(down.is_temporary());
        assert_eq!(down, VerifyError::Unavailable("x".to_owned()));
        assert!(!VerifyError::new("x".to_owned(), false).is_temporary());
        // text errors are refusals
        assert!(!VerifyError::from("bad signature").is_temporary());
        let resolve = |temporary| ResolveError {
            reason: "r".to_owned(),
            temporary,
        };
        assert!(VerifyError::from(resolve(true)).is_temporary());
        assert!(!VerifyError::from(resolve(false)).is_temporary());
    }

    #[tokio::test]
    async fn the_defaults_refuse() {
        assert!(RefuseAll.verify(Some("Bearer x")).await.is_err());
        assert!(NoTokens.authorization("did:web:x", &[]).await.is_err());
        let caller = Caller::new("did:web:x");
        assert!(RefuseAll.credentials(&caller, &[]).await.is_err());
    }
}
