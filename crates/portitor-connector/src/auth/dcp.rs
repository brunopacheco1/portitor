// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter: the DCP verifier (ADR 0016). For each DSP request it follows
//! DCP v1.0.1 (base protocol section 4.3, presentation protocol section
//! 5):
//!
//! 1. Read `Authorization: Bearer <self-issued ID token>`.
//! 2. Resolve the token's issuer DID, and check the token: signed with a
//!    `capabilityInvocation` key, `aud` is our DID, `iss` = `sub`, `exp`,
//!    `nbf`, `jti` not seen before, valid for at most an hour
//!    ([`portitor_dcp::validate_self_issued`]).
//! 3. Take the access token from its `token` claim, find the caller's
//!    Credential Service in its DID document, and send it a
//!    `PresentationQueryMessage` for the configured scopes, with our own
//!    ID token (from our STS) carrying that access token.
//! 4. Check every presentation (holder is the caller, signed with an
//!    `authentication` key, meant for us, valid now) and every credential
//!    in it (subject is the caller, signed by its issuer with an
//!    `assertionMethod` key, valid now, not revoked or suspended).
//! 5. Require one `MembershipCredential` from a trusted issuer.
//!
//! DCP v1.0.1 section 5.4.3 ("Presentation Validation"), step 8: "If any
//! of the steps fail, the Verifier MUST consider the Verifiable
//! Presentation invalid". So one credential that fails its checks
//! (signature, validity, revocation or suspension, holder binding) makes
//! its presentation invalid, and the caller is refused, even when a valid
//! `MembershipCredential` sits next to it. The failing credential is
//! logged (`credential refused`, with its issuer, types and reason). A
//! holder whose credential was revoked and reissued must delete the
//! revoked one from its identity (DEPLOYING).
//!
//! Gaia-X compliance (ADR 0022). A `gx:ParticipantComplianceCredential` is
//! about a credential, not about the caller, so it gets its own checks
//! ([`DcpVerifier::check_gaiax`]): its issuer is a trusted compliance
//! service (`PORTITOR_GAIAX_COMPLIANCE_ISSUERS`), it is signed with that
//! service's `assertionMethod` key and valid by its `validFrom` and
//! `validUntil`, and the same presentation holds a `gx:LegalPerson`
//! credential that it names by digest (`gx:compliantCredentials`,
//! `digestSRI`), issued and signed by the caller. The holder binding is
//! not applied to these two credentials; it stays for every other one
//! (reported upstream:
//! <https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278>). A
//! compliance credential from a trusted service that fails makes its
//! presentation invalid (step 8 above). A compliance credential from
//! another service, and a legal person credential that no trusted
//! compliance credential names, are skipped before anything is fetched
//! for them, like credentials from untrusted issuers.
//!
//! The verified credentials go with the caller to the handler. When an
//! offer or agreement has credential constraints (ADR 0019), the handler
//! asks for more scopes with [`CallerVerifier::credentials`]: steps 3 and 4
//! again, with the access token the caller sent.
//!
//! Every fetch (DID documents, the Credential Service, status lists) goes
//! through the shared HTTP client: HTTPS only unless allowed for tests,
//! time and size limits, no redirects.
//!
//! A fetch that got no answer, or the answer 408, 429 or 5xx, and a
//! check that took too long, are temporary failures
//! ([`VerifyError::Unavailable`], HTTP 503): the caller may be fine. Any
//! other failure refuses the caller ([`VerifyError::Rejected`], HTTP
//! 401), also a 404 or an invalid document from the caller's or the
//! issuer's server.
//!
//! Status list credentials are cached for a short time, by URL
//! ([`status_list_cache`]; ADR 0016, amended in 0.11). Tokens and
//! presentations are not: they are checked on every request.
//!
//! The caller chooses what is in its presentations, and each credential
//! could make the verifier fetch more (the issuer's DID document, status
//! lists). So credentials from issuers that are not trusted are skipped
//! before anything is fetched for them, the answer may hold at most
//! [`MAX_PRESENTATIONS`] presentations of at most [`MAX_CREDENTIALS`]
//! credentials with at most [`MAX_STATUS_ENTRIES`] status entries each, and
//! the whole check must end within [`DEADLINE`].

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use portitor_dcp::credential::{Credential, Presentation};
use portitor_dcp::gaiax::{
    COMPLIANCE_TYPE, GAIAX_PREFIX, LEGAL_PERSON_TYPE, compliant_credentials, contexts,
    digest_sri_of, matches_scope_type,
};
use portitor_dcp::message::{PresentationQueryMessage, PresentationResponseMessage};
use portitor_dcp::scope::merge_scopes;
use portitor_dcp::status_list::{self, Status, StatusEntry};
use portitor_dcp::{Claims, DidDocument, JtiCache, unverified_claims, validate_self_issued};
use portitor_dsp::policy::HeldCredential;
use portitor_runtime::BoxFuture;
use portitor_runtime::did::DidWebResolver;
use portitor_runtime::http::{HttpError, HttpSender, bearer_token};
use portitor_runtime::system::now_secs;
use serde_json::Value;

use super::sts::{SecureTokenService, TokenRequest};
use super::{AuthSettings, Caller, CallerVerifier, VerifyError, with_linked_scopes};

pub mod status_list_cache;

use status_list_cache::StatusListCache;

/// Where the verifier reads the current time, in seconds since 1970:
/// [`now_secs`], except in tests, which move time forward.
type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Allowed clock difference between participants, in seconds.
const LEEWAY: u64 = 60;

/// The largest status list credential read, in bytes. A list of the
/// minimum size (16 KiB uncompressed) is a few kilobytes even when many
/// bits are set.
const MAX_STATUS_LIST: usize = 2 * 1024 * 1024;

/// The credential type every caller must hold (ADR 0016).
pub const MEMBERSHIP: &str = portitor_dsp::policy::MEMBERSHIP;

/// The most presentations read from one Credential Service answer. A
/// Credential Service sends one per profile (DCP "Homogeneity
/// requirement"), so two would do.
pub const MAX_PRESENTATIONS: usize = 8;

/// The most credentials read from one presentation.
pub const MAX_CREDENTIALS: usize = 16;

/// The most status entries read from one credential (one revocation and
/// one suspension list is usual).
pub const MAX_STATUS_ENTRIES: usize = 4;

/// The longest one caller check may take, all fetches included.
pub const DEADLINE: Duration = Duration::from_secs(20);

/// How often [`DcpVerifier::check_own_identity`] tries to read our DID
/// document: the identity component may still be starting.
const START_TRIES: u32 = 15;

/// How long [`DcpVerifier::check_own_identity`] waits between two tries:
/// with [`START_TRIES`], about 30 seconds in all.
const START_WAIT: Duration = Duration::from_secs(2);

/// The DCP verifier.
pub struct DcpVerifier {
    participant_id: String,
    resolver: DidWebResolver,
    http: HttpSender,
    sts: Arc<dyn SecureTokenService>,
    trusted_issuers: Vec<String>,
    /// The Gaia-X compliance services we trust (ADR 0022).
    gaiax_compliance_issuers: Vec<String>,
    scopes: Vec<String>,
    /// Token ids already used (DCP 4.3.3, step 6). In memory: one replica
    /// per store (ADR 0016).
    used_token_ids: Mutex<JtiCache>,
    /// Verified status list credentials, by URL
    /// (`PORTITOR_STATUS_LIST_CACHE_SECONDS`).
    status_lists: StatusListCache,
    /// The current time.
    clock: Clock,
    start_tries: u32,
    deadline: Duration,
}

impl std::fmt::Debug for DcpVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DcpVerifier")
            .field("participant_id", &self.participant_id)
            .field("trusted_issuers", &self.trusted_issuers)
            .field("gaiax_compliance_issuers", &self.gaiax_compliance_issuers)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

/// The caller's token, checked.
struct CheckedToken {
    /// The caller's DID document.
    doc: Arc<DidDocument>,
    /// The access token for the caller's Credential Service.
    access_token: String,
}

impl DcpVerifier {
    /// A verifier for `settings`, getting our own tokens from `sts`.
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn new(settings: &AuthSettings, sts: Arc<dyn SecureTokenService>) -> Result<Self, String> {
        // how long a DID document is reused (PORTITOR_DID_CACHE_SECONDS)
        let did_cache = Duration::from_secs(settings.did_cache_seconds);
        Ok(Self {
            participant_id: settings.participant_id.clone(),
            resolver: DidWebResolver::new(settings.allow_http, did_cache)?,
            http: HttpSender::new(settings.allow_http)?,
            sts,
            trusted_issuers: settings.trusted_issuers.clone(),
            gaiax_compliance_issuers: settings.gaiax_compliance_issuers.clone(),
            // a Gaia-X compliance scope brings its legal person scope, as
            // on the prover's side (ADR 0022)
            scopes: with_linked_scopes(&settings.scopes),
            used_token_ids: Mutex::new(JtiCache::default()),
            status_lists: StatusListCache::new(settings.status_list_cache_seconds),
            clock: Arc::new(now_secs),
            start_tries: START_TRIES,
            deadline: DEADLINE,
        })
    }

    /// Checks at start that `PORTITOR_PARTICIPANT_ID` is our identity's
    /// DID (ADR 0016): its DID document resolves, lists a Credential
    /// Service, and a token from our STS is signed by a key in it with
    /// `iss` = `sub` = that DID.
    ///
    /// # Errors
    ///
    /// The DID document cannot be read, has no Credential Service, or the
    /// STS signs for another identity.
    pub async fn check_own_identity(&self) -> Result<(), String> {
        let doc = self.resolve_own_document().await?;
        self.own_identity_agrees(&doc).await?;
        tracing::info!(did = %self.participant_id, "own identity checked: DID document and STS agree");
        Ok(())
    }

    /// The check of [`DcpVerifier::check_own_identity`] once, without
    /// waiting between tries and without the log line: for the readiness
    /// check (ADR 0023), which runs it again and again.
    ///
    /// # Errors
    ///
    /// As [`DcpVerifier::check_own_identity`].
    pub async fn check_own_identity_once(&self) -> Result<(), String> {
        // a verifier made with `for_readiness` tries once, without waiting
        let doc = self.resolve_own_document().await?;
        self.own_identity_agrees(&doc).await
    }

    /// Our DID document lists a Credential Service, and a token from our
    /// STS is signed by a key in it with `iss` = `sub` = our DID.
    async fn own_identity_agrees(&self, doc: &DidDocument) -> Result<(), String> {
        let own = self.participant_id.as_str();
        if doc.credential_service().is_none() {
            return Err(format!(
                "the DID document of {own} lists no CredentialService"
            ));
        }
        let request = TokenRequest {
            audience: own,
            scopes: &[],
            access_token: None,
        };
        let token = self.sts.token(request).await?;
        // a cache of its own: this token is never presented anywhere
        let mut unused = JtiCache::default();
        validate_self_issued(&token, doc, own, self.now(), LEEWAY, &mut unused).map_err(|e| {
            format!("PORTITOR_PARTICIPANT_ID is {own}, but the STS token is not signed as it: {e}")
        })?;
        Ok(())
    }

    /// A verifier for the readiness check only: it reads our DID document
    /// afresh each time (no cache), so a document that is no longer
    /// served is noticed; tries once, without waiting; and its fetches
    /// make no spans or measures (the check runs every few seconds, on
    /// its own). `sts` should be unobserved too
    /// ([`super::sts::StsClient::unobserved`]).
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn for_readiness(
        settings: &AuthSettings,
        sts: Arc<dyn SecureTokenService>,
    ) -> Result<Self, String> {
        let mut verifier = Self::new(settings, sts)?;
        verifier.resolver = DidWebResolver::new(settings.allow_http, Duration::ZERO)?.unobserved();
        verifier.http = verifier.http.unobserved();
        verifier.start_tries = 1;
        Ok(verifier)
    }

    /// The current time, in seconds since 1970.
    fn now(&self) -> u64 {
        (self.clock)()
    }

    async fn resolve_own_document(&self) -> Result<Arc<DidDocument>, String> {
        let mut last = String::new();
        for attempt in 1..=self.start_tries.max(1) {
            match self.resolver.resolve(&self.participant_id).await {
                Ok(doc) => return Ok(doc),
                Err(e) => last = e.reason,
            }
            if attempt < self.start_tries {
                tracing::info!(did = %self.participant_id, error = %last, "own DID document not readable yet");
                tokio::time::sleep(START_WAIT).await;
            }
        }
        Err(format!(
            "cannot read the DID document of {}: {last}",
            self.participant_id
        ))
    }

    /// The whole check, cut off after [`DEADLINE`].
    async fn check(&self, authorization: Option<&str>) -> Result<Caller, VerifyError> {
        self.within_deadline("checking the caller", self.check_steps(authorization))
            .await
    }

    /// `work`, cut off after [`DEADLINE`]. Running out of time is a
    /// temporary failure: the services the check fetches from were slow.
    async fn within_deadline<T>(
        &self,
        what: &str,
        work: impl Future<Output = Result<T, VerifyError>>,
    ) -> Result<T, VerifyError> {
        match tokio::time::timeout(self.deadline, work).await {
            Ok(result) => result,
            Err(_) => Err(VerifyError::Unavailable(format!(
                "{what} took longer than {} s",
                self.deadline.as_secs_f32()
            ))),
        }
    }

    /// Steps 1 to 5 of the module doc.
    async fn check_steps(&self, authorization: Option<&str>) -> Result<Caller, VerifyError> {
        let token = bearer(authorization)?;
        let checked = self.check_token(token).await?;
        let caller = checked.doc.id.clone();
        let presentations = self.query(&checked, &self.scopes).await?;
        let credentials = self
            .check_presentations(&caller, &checked.doc, &presentations)
            .await?;
        let is_member = credentials
            .iter()
            .any(|credential| credential.has_type(MEMBERSHIP));
        if !is_member {
            let reason = format!("no {MEMBERSHIP} from a trusted issuer");
            return Err(VerifyError::Rejected(reason));
        }
        Ok(Caller {
            id: caller,
            credentials,
            access_token: Some(checked.access_token),
        })
    }

    /// Steps 3 and 4 again, for the configured scopes and `extra` (ADR
    /// 0019), with the access token `caller` sent.
    async fn more_credentials(
        &self,
        caller: &Caller,
        extra: &[String],
    ) -> Result<Vec<HeldCredential>, VerifyError> {
        let access_token = caller
            .access_token
            .clone()
            .ok_or("the caller sent no access token")?;
        let doc = self.resolver.resolve(&caller.id).await?;
        let checked = CheckedToken { doc, access_token };
        let scopes = with_linked_scopes(&merge_scopes(&self.scopes, extra));
        let presentations = self.query(&checked, &scopes).await?;
        self.check_presentations(&caller.id, &checked.doc, &presentations)
            .await
    }

    /// Step 2: the caller's self-issued ID token.
    async fn check_token(&self, token: &str) -> Result<CheckedToken, VerifyError> {
        let unverified = unverified_claims(token).map_err(|e| format!("ID token: {e}"))?;
        let now = self.now();
        let doc = self.resolver.resolve(&unverified.iss).await?;
        let claims = self
            .validate_and_remember(token, &doc, now)
            .map_err(|e| format!("ID token: {e}"))?;
        let access_token = claims
            .token
            .filter(|t| !t.is_empty())
            .ok_or("the ID token has no access token (token claim)")?;
        Ok(CheckedToken { doc, access_token })
    }

    /// Checks the token against the caller's DID document and records its
    /// id as used, under the lock of the used ids.
    fn validate_and_remember(
        &self,
        token: &str,
        doc: &DidDocument,
        now: u64,
    ) -> Result<Claims, portitor_dcp::Error> {
        // a panic while holding the lock leaves at most one more token id
        // in the list, so the list is still safe to use
        let mut used = self
            .used_token_ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        validate_self_issued(token, doc, &self.participant_id, now, LEEWAY, &mut used)
    }

    /// Step 3: asks the caller's Credential Service for presentations of
    /// `scopes`. Returns them as JWTs.
    async fn query(
        &self,
        checked: &CheckedToken,
        scopes: &[String],
    ) -> Result<Vec<String>, VerifyError> {
        let service = checked
            .doc
            .credential_service()
            .ok_or("the caller's DID document lists no CredentialService")?;
        // DCP 5.4.1: the query endpoint is below the service's base URL
        let url = format!("{}/presentations/query", service.trim_end_matches('/'));
        let request = TokenRequest {
            audience: &checked.doc.id,
            scopes: &[],
            access_token: Some(&checked.access_token),
        };
        // our own STS failing is never the caller's fault
        let our_token = self
            .sts
            .token(request)
            .await
            .map_err(|e| VerifyError::Unavailable(format!("our STS: {e}")))?;
        let query = PresentationQueryMessage::for_scopes(scopes.to_vec());
        let body = serde_json::to_value(&query).map_err(|e| e.to_string())?;
        let answer = self
            .http
            .post_json_authorized(&url, &body, &format!("Bearer {our_token}"))
            .await
            .map_err(|e| fetch_failed(&format!("presentation query to {url}"), &e))?;
        let jwts = presentation_jwts(answer)?;
        Ok(jwts)
    }

    /// Step 4. Only credentials from trusted issuers are checked (and
    /// returned); the others are skipped. A presentation that fails its
    /// checks, or holds a credential that fails its own, refuses the
    /// caller (DCP 5.4.3, step 8; module doc).
    async fn check_presentations(
        &self,
        caller: &str,
        holder: &DidDocument,
        jwts: &[String],
    ) -> Result<Vec<HeldCredential>, VerifyError> {
        let now = self.now();
        let mut verified = Vec::new();
        for jwt in jwts {
            let presentation = self.check_presentation(caller, holder, jwt, now)?;
            // Gaia-X credentials first: they have their own rules (ADR 0022)
            let gaiax = self
                .check_gaiax(caller, holder, &presentation.credentials, now)
                .await?;
            verified.extend(gaiax);
            let others = self
                .check_dataspace_credentials(caller, &presentation.credentials, now)
                .await?;
            verified.extend(others);
        }
        Ok(verified)
    }

    /// One presentation: its holder is the caller, it is signed with the
    /// caller's `authentication` key, meant for us and valid now, and it
    /// holds at most [`MAX_CREDENTIALS`] credentials.
    fn check_presentation(
        &self,
        caller: &str,
        holder: &DidDocument,
        jwt: &str,
        now: u64,
    ) -> Result<Presentation, String> {
        let presentation = Presentation::parse(jwt).map_err(|e| format!("presentation: {e}"))?;
        if presentation.holder != caller {
            return Err("a presentation's holder is not the caller".to_owned());
        }
        presentation
            .verify(holder, &self.participant_id, now, LEEWAY)
            .map_err(|e| format!("presentation: {e}"))?;
        if presentation.credentials.len() > MAX_CREDENTIALS {
            return Err(format!(
                "a presentation holds more than {MAX_CREDENTIALS} credentials"
            ));
        }
        Ok(presentation)
    }

    /// Whether `credential` gets the Gaia-X checks of ADR 0022
    /// ([`DcpVerifier::check_gaiax`]) instead of the dataspace ones: a
    /// compliance or legal person credential whose issuer is not a
    /// trusted dataspace issuer (`PORTITOR_TRUSTED_ISSUERS`). A trusted
    /// issuer's `gx:LegalPerson` is about the caller like any other
    /// dataspace credential, so it is checked as before 0.8, holder
    /// binding included.
    fn takes_gaiax_checks(&self, credential: &Credential) -> bool {
        let gaiax_type = is_gaiax_compliance(credential) || is_legal_person(credential);
        gaiax_type && !self.trusted_issuers.contains(&credential.issuer)
    }

    /// The credentials of one presentation that do not take the Gaia-X
    /// checks ([`DcpVerifier::takes_gaiax_checks`]): from a trusted
    /// issuer, about the caller, signed, valid now, not revoked or
    /// suspended. Credentials from other issuers are skipped.
    async fn check_dataspace_credentials(
        &self,
        caller: &str,
        credentials: &[Credential],
        now: u64,
    ) -> Result<Vec<HeldCredential>, VerifyError> {
        let mut verified = Vec::new();
        for credential in credentials {
            if self.takes_gaiax_checks(credential) {
                continue;
            }
            // skipped before its issuer's DID document or status list
            // is fetched: an untrusted issuer could name any URL there
            if !self.trusted_issuers.contains(&credential.issuer) {
                tracing::debug!(issuer = %credential.issuer, "credential from an untrusted issuer skipped");
                continue;
            }
            // holder binding (DCP 5.4.3) and the fetch limit
            if !credential.is_only_about(caller) {
                return Err(VerifyError::from(
                    "a credential is not about the caller (credentialSubject.id)",
                ));
            }
            check_status_count(credential)?;
            match self.check_credential(credential, now).await {
                Ok(()) => {}
                // its issuer's server is down: not the credential's fault
                Err(error) if error.is_temporary() => return Err(error),
                Err(error) => return Err(refused(caller, credential, error.reason())),
            }
            verified.push(held_credential(credential));
        }
        Ok(verified)
    }

    /// The Gaia-X credentials of one presentation (ADR 0022, module doc):
    /// each `gx:ParticipantComplianceCredential` must come from a trusted
    /// compliance service and name, by digest, a `gx:LegalPerson`
    /// credential of the same presentation that the caller issued and
    /// signed. Returns both kinds, checked: the compliance credentials
    /// first. Legal person credentials that no compliance credential
    /// names are skipped, without a fetch.
    async fn check_gaiax(
        &self,
        caller: &str,
        holder: &DidDocument,
        credentials: &[Credential],
        now: u64,
    ) -> Result<Vec<HeldCredential>, VerifyError> {
        let mut compliance = self
            .check_compliance_credentials(caller, credentials, now)
            .await?;
        let mut verified = Vec::new();
        for checked in &compliance {
            verified.push(held_as(checked.credential, COMPLIANCE_TYPE));
        }
        let persons =
            self.check_named_legal_persons(caller, holder, credentials, &mut compliance, now)?;
        verified.extend(persons);
        // a compliance credential vouches for a legal person credential;
        // without it in the same presentation it says nothing about the
        // caller (ADR 0022, step 4)
        for checked in &compliance {
            if !checked.linked {
                let reason = format!(
                    "it names no {LEGAL_PERSON_TYPE} credential of its presentation (digestSRI)"
                );
                return Err(refused(caller, checked.credential, &reason));
            }
        }
        Ok(verified)
    }

    /// The compliance credentials of one presentation from trusted
    /// compliance services, each checked
    /// ([`DcpVerifier::check_compliance_credential`]), with the digests
    /// they name. None is linked yet. Those from other issuers are
    /// skipped.
    async fn check_compliance_credentials<'a>(
        &self,
        caller: &str,
        credentials: &'a [Credential],
        now: u64,
    ) -> Result<Vec<ComplianceCheck<'a>>, VerifyError> {
        let mut checked = Vec::new();
        for credential in credentials {
            if !is_gaiax_compliance(credential) || !self.takes_gaiax_checks(credential) {
                continue;
            }
            // Skipped, not refused, like any credential from an untrusted
            // issuer, and before its issuer's DID document is fetched. A
            // participant of another clearing house can still use offers
            // that need only a membership.
            if !self.gaiax_compliance_issuers.contains(&credential.issuer) {
                tracing::debug!(issuer = %credential.issuer, "gx:ParticipantComplianceCredential from an untrusted compliance service skipped");
                continue;
            }
            match self.check_compliance_credential(credential, now).await {
                Ok(()) => {}
                // the service's server is down: not the credential's fault
                Err(error) if error.is_temporary() => return Err(error),
                Err(error) => return Err(refused(caller, credential, error.reason())),
            }
            checked.push(ComplianceCheck::new(credential));
        }
        Ok(checked)
    }

    /// The legal person credentials of one presentation that a checked
    /// compliance credential names, each checked
    /// ([`check_legal_person`]), without a fetch. Marks the compliance
    /// credentials that name one as linked. The others are skipped.
    fn check_named_legal_persons(
        &self,
        caller: &str,
        holder: &DidDocument,
        credentials: &[Credential],
        compliance: &mut [ComplianceCheck<'_>],
        now: u64,
    ) -> Result<Vec<HeldCredential>, VerifyError> {
        let mut verified = Vec::new();
        for credential in credentials {
            if !is_legal_person(credential) || !self.takes_gaiax_checks(credential) {
                continue;
            }
            let digest = digest_sri_of(credential).map_err(|e| format!("credential: {e}"))?;
            if !link(compliance, &digest) {
                // like a credential from an untrusted issuer
                tracing::debug!(issuer = %credential.issuer, "gx:LegalPerson credential that no trusted compliance credential names skipped");
                continue;
            }
            if let Err(reason) = check_legal_person(caller, holder, credential, now) {
                return Err(refused(caller, credential, &reason));
            }
            verified.push(held_as(credential, LEGAL_PERSON_TYPE));
        }
        Ok(verified)
    }

    /// One Gaia-X compliance credential from a trusted compliance service
    /// (ADR 0022, steps 1 to 3): signed with that service's
    /// `assertionMethod` key; valid now by `validFrom` and `validUntil`
    /// only (the service writes its JWT times in milliseconds), and
    /// `validUntil` is required; its
    /// status, if it has one, as for any credential. The caller is not its
    /// subject, so there is no holder binding.
    async fn check_compliance_credential(
        &self,
        credential: &Credential,
        now: u64,
    ) -> Result<(), VerifyError> {
        // The JWT's `exp` is not read (milliseconds), so `validUntil` is
        // the only end. Without one the credential would never expire,
        // which a compliance service must not issue (ADR 0022, step 2).
        if credential.stated_validity().until.is_none() {
            return Err(VerifyError::from(
                "it has no validUntil (expirationDate in VC 1.1)",
            ));
        }
        check_status_count(credential)?;
        let issuer = self.resolver.resolve(&credential.issuer).await?;
        credential
            .verify_stated_dates(&issuer, now, LEEWAY)
            .map_err(|e| format!("credential from {}: {e}", credential.issuer))?;
        for entry in &credential.status {
            self.check_status(credential, &issuer, entry, now).await?;
        }
        Ok(())
    }

    /// One credential about the caller (checked before): signed by its
    /// issuer, valid now, not revoked or suspended. The error says why
    /// not.
    async fn check_credential(&self, credential: &Credential, now: u64) -> Result<(), VerifyError> {
        let issuer = self.resolver.resolve(&credential.issuer).await?;
        credential
            .verify(&issuer, now, LEEWAY)
            .map_err(|e| format!("credential from {}: {e}", credential.issuer))?;
        for entry in &credential.status {
            self.check_status(credential, &issuer, entry, now).await?;
        }
        Ok(())
    }

    /// One status entry: takes the status list credential, which must be
    /// signed by the same issuer and valid now
    /// ([`DcpVerifier::status_list_credential`]), and looks up the bit. A
    /// list that cannot be read or checked refuses the credential, unless
    /// its server did not answer or answered 408, 429 or 5xx (a temporary
    /// failure).
    async fn check_status(
        &self,
        credential: &Credential,
        issuer: &DidDocument,
        entry: &StatusEntry,
        now: u64,
    ) -> Result<(), VerifyError> {
        let url = &entry.status_list_credential;
        let list_credential = self
            .status_list_credential(credential, issuer, url, now)
            .await?;
        let list = status_list::list_in(&list_credential, entry)
            .map_err(|e| format!("status list {url}: {e}"))?;
        match status_list::check(entry, &list).map_err(|e| format!("status list {url}: {e}"))? {
            Status::Clear => Ok(()),
            Status::Set => Err(VerifyError::Rejected(format!(
                "a credential is revoked or suspended ({})",
                entry.status_purpose
            ))),
        }
    }

    /// The status list credential at `url` for `credential`, signed by
    /// the same issuer (`issuer` is its DID document) and valid at `now`:
    /// from the cache, or fetched, checked and then kept in the cache.
    async fn status_list_credential(
        &self,
        credential: &Credential,
        issuer: &DidDocument,
        url: &str,
        now: u64,
    ) -> Result<Arc<Credential>, VerifyError> {
        let another_issuer =
            || VerifyError::Rejected(format!("status list {url} has another issuer"));
        if let Some(cached) = self.status_lists.get(url, now) {
            // it was checked against its own issuer when it was fetched;
            // another issuer's credential may name the same URL
            if cached.issuer != credential.issuer {
                return Err(another_issuer());
            }
            return Ok(cached);
        }
        let text = self
            .http
            .get_text(url, "application/vc+jwt", MAX_STATUS_LIST)
            .await
            .map_err(|e| fetch_failed(&format!("status list {url}"), &e))?;
        let list_credential =
            Credential::parse(text.trim()).map_err(|e| format!("status list {url}: {e}"))?;
        if list_credential.issuer != credential.issuer {
            return Err(another_issuer());
        }
        list_credential
            .verify(issuer, now, LEEWAY)
            .map_err(|e| format!("status list {url}: {e}"))?;
        // only now, fetched and verified, may it be kept
        let list_credential = Arc::new(list_credential);
        self.status_lists
            .keep(url, Arc::clone(&list_credential), now);
        Ok(list_credential)
    }
}

/// A checked Gaia-X compliance credential of a presentation.
struct ComplianceCheck<'a> {
    credential: &'a Credential,
    /// The `digestSRI` of each credential it names
    /// (`gx:compliantCredentials`).
    digests: Vec<String>,
    /// Whether the presentation holds a legal person credential with one
    /// of these digests.
    linked: bool,
}

impl<'a> ComplianceCheck<'a> {
    fn new(credential: &'a Credential) -> Self {
        let mut digests = Vec::new();
        for entry in compliant_credentials(credential) {
            digests.push(entry.digest_sri);
        }
        Self {
            credential,
            digests,
            linked: false,
        }
    }
}

/// Marks each compliance credential that names `digest` as linked.
/// Returns whether any does.
fn link(compliance: &mut [ComplianceCheck<'_>], digest: &str) -> bool {
    let mut named = false;
    for checked in compliance.iter_mut() {
        if checked.digests.iter().any(|listed| listed == digest) {
            checked.linked = true;
            named = true;
        }
    }
    named
}

impl CallerVerifier for DcpVerifier {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Caller, VerifyError>> {
        Box::pin(self.check(authorization))
    }

    fn credentials<'a>(
        &'a self,
        caller: &'a Caller,
        scopes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
        Box::pin(async move {
            // the first query asked for the configured scopes already
            if scopes.iter().all(|scope| self.scopes.contains(scope)) {
                return Ok(caller.credentials.clone());
            }
            let more = self
                .within_deadline(
                    "asking the caller for more credentials",
                    self.more_credentials(caller, scopes),
                )
                .await;
            // this port gives text: any failure means no credentials
            more.map_err(|error| error.to_string())
        })
    }
}

/// Whether `credential` is a Gaia-X compliance credential
/// (`gx:ParticipantComplianceCredential`).
fn is_gaiax_compliance(credential: &Credential) -> bool {
    credential.has_type(COMPLIANCE_TYPE)
}

/// Whether `credential` is a Gaia-X legal person credential
/// (`gx:LegalPerson`).
fn is_legal_person(credential: &Credential) -> bool {
    credential.has_type(LEGAL_PERSON_TYPE)
}

/// One `gx:LegalPerson` credential that a trusted compliance credential
/// names (ADR 0022, step 4): issued by the caller and signed with the
/// caller's `assertionMethod` key (`holder` is the caller's DID
/// document), valid now. Its subject is not the caller's DID, so there is
/// no holder binding.
///
/// Its `credentialStatus` is not fetched. The caller issued it and
/// controls its status lists, so a status says nothing the caller could
/// not change, and fetching would only let the caller choose URLs that we
/// fetch. The compliance credential's own validity bounds it.
fn check_legal_person(
    caller: &str,
    holder: &DidDocument,
    credential: &Credential,
    now: u64,
) -> Result<(), String> {
    if credential.issuer != caller {
        return Err(format!(
            "its issuer {} is not the caller",
            credential.issuer
        ));
    }
    credential
        .verify(holder, now, LEEWAY)
        .map_err(|e| format!("credential from {}: {e}", credential.issuer))
}

/// The fetch limit on status entries: at most [`MAX_STATUS_ENTRIES`].
fn check_status_count(credential: &Credential) -> Result<(), String> {
    if credential.status.len() > MAX_STATUS_ENTRIES {
        return Err(format!(
            "a credential has more than {MAX_STATUS_ENTRIES} status entries"
        ));
    }
    Ok(())
}

/// A fetch for the check that failed: [`VerifyError::Unavailable`] when
/// trying again later may help (no answer, 408, 429, 5xx;
/// [`HttpError::is_temporary`]), [`VerifyError::Rejected`] otherwise.
fn fetch_failed(what: &str, error: &HttpError) -> VerifyError {
    VerifyError::new(format!("{what}: {error}"), error.is_temporary())
}

/// Logs a credential that failed its checks, and returns why the caller
/// is refused: the whole presentation is invalid (DCP 5.4.3, step 8).
fn refused(caller: &str, credential: &Credential, reason: &str) -> VerifyError {
    tracing::info!(
        %caller,
        issuer = %credential.issuer,
        types = ?credential.types,
        %reason,
        "credential refused"
    );
    VerifyError::Rejected(format!(
        "a {} credential fails its checks, so its presentation is invalid: {reason}",
        credential_types(credential)
    ))
}

/// A checked credential as the policy evaluator reads it (ADR 0019).
fn held_credential(credential: &Credential) -> HeldCredential {
    HeldCredential {
        types: policy_type_names(credential),
        subjects: credential.subjects.clone(),
    }
}

/// A checked Gaia-X credential as the policy evaluator reads it: as
/// `gaiax_type` only (and its scope name), whatever other types it lists.
/// Its signer vouches for that one type: a compliance service does not
/// issue memberships, and the caller signs its own legal person
/// credential, so another type there would be the caller's own claim.
fn held_as(credential: &Credential, gaiax_type: &str) -> HeldCredential {
    let mut types = vec![gaiax_type.to_owned()];
    let credential_contexts = credential_contexts_of(credential);
    if let Some(short) = gaiax_type.strip_prefix(GAIAX_PREFIX)
        && matches_scope_type(&types, &credential_contexts, short)
    {
        types.push(short.to_owned());
    }
    HeldCredential {
        types,
        subjects: credential.subjects.clone(),
    }
}

/// The `@context` entries of a credential, from its JWT's payload,
/// decoded again here; none when the payload cannot be read.
fn credential_contexts_of(credential: &Credential) -> Vec<String> {
    match credential.payload() {
        Ok(payload) => contexts(&payload),
        Err(_) => Vec::new(),
    }
}

/// The type names a policy may use for `credential`: its types, and for a
/// credential with a Gaia-X context each `gx:` type without its prefix,
/// the way a scope names it (ADR 0021, `matches_scope_type`). So
/// `gx:ParticipantComplianceCredential` is also
/// `ParticipantComplianceCredential`, the type in the left operand
/// `ParticipantComplianceCredential.gx:labelLevel` (ADR 0022).
fn policy_type_names(credential: &Credential) -> Vec<String> {
    let mut names = credential.types.clone();
    let credential_contexts = credential_contexts_of(credential);
    for credential_type in &credential.types {
        let Some(short) = credential_type.strip_prefix(GAIAX_PREFIX) else {
            continue;
        };
        let is_gaiax = matches_scope_type(&credential.types, &credential_contexts, short);
        if is_gaiax && !names.iter().any(|name| name == short) {
            names.push(short.to_owned());
        }
    }
    names
}

/// A credential's types for a message, without the base type
/// `VerifiableCredential` (every credential has it).
fn credential_types(credential: &Credential) -> String {
    let mut names = Vec::new();
    for credential_type in &credential.types {
        if credential_type != "VerifiableCredential" {
            names.push(credential_type.as_str());
        }
    }
    if names.is_empty() {
        "VerifiableCredential".to_owned()
    } else {
        names.join("/")
    }
}

/// Step 1: the token in `Authorization: Bearer <token>`. EDC sends the
/// `Bearer ` prefix and refuses a token without it; the DCP TCK v1.2.1
/// expects the same.
fn bearer(authorization: Option<&str>) -> Result<&str, String> {
    let header = authorization.ok_or("no Authorization header")?;
    bearer_token(header).ok_or_else(|| "the Authorization header is not a Bearer token".to_owned())
}

/// The presentations of a Credential Service's answer, which must be a
/// `PresentationResponseMessage` with at least one presentation, each a
/// JWT (both DCP profiles Portitor supports use JWTs).
fn presentation_jwts(answer: Value) -> Result<Vec<String>, String> {
    let response: PresentationResponseMessage =
        serde_json::from_value(answer).map_err(|e| format!("presentation response: {e}"))?;
    if response.presentation.is_empty() {
        return Err("the presentation response is empty".to_owned());
    }
    if response.presentation.len() > MAX_PRESENTATIONS {
        return Err(format!(
            "the presentation response holds more than {MAX_PRESENTATIONS} presentations"
        ));
    }
    let mut jwts = Vec::new();
    for presentation in &response.presentation {
        match presentation.as_str() {
            Some(jwt) => jwts.push(jwt.to_owned()),
            None => return Err("a presentation is not a JWT".to_owned()),
        }
    }
    Ok(jwts)
}

#[cfg(test)]
mod tests;
