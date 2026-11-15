// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What a participant signs for Gaia-X onboarding (ADR 0021), and the
//! check of the compliance service's answer. Pure: no I/O, no clock.
//!
//! The shapes follow the 0.8 spike (`docs/spikes/0.8-gaia-x-compliance.md`),
//! which a Gaia-X compliance service v2.16.0 accepted, and Gaia-X's own
//! ES256 example (`STANDARD_COMPLIANCE_VP_JWT_EXAMPLE` in gx-compliance
//! `src/examples/signed-examples.ts`):
//!
//! - each credential is VC Data Model 2.0 as a VC-JWT: JOSE header
//!   `alg: ES256`, `typ: vc+jwt`, `cty: vc`, `iss` (the signer's DID) and
//!   `kid` (its verification method); payload `@context`, `id`, `type`,
//!   `issuer`, `validFrom`, `validUntil`, `credentialSubject` (ICAM 25.11,
//!   "Gaia-X credentials");
//! - the presentation is a `vp+jwt` (`cty: vp`) whose payload holds each
//!   credential as an `EnvelopedVerifiableCredential` whose `id` is
//!   `data:application/vc+jwt,<JWT>` (VC Data Model 2.0, "Enveloped
//!   Verifiable Credentials").
//!
//! The answer is a `gx:ParticipantComplianceCredential` that lists, for
//! each credential it checked, its `id` and `digestSRI`
//! ([`crate::gaiax::compliant_credentials`]). [`check_answer`] makes sure
//! it is about exactly the credentials sent; the caller checks its
//! signature with the service's DID document ([`Credential::verify`]).

use serde_json::{Map, Value, json};

use crate::credential::{Credential, Profile, VC2_CONTEXT};
use crate::gaiax::{COMPLIANCE_TYPE, ISSUER_TYPE, LEGAL_PERSON_TYPE, compliant_credentials};
use crate::{Error, Jws, Signer};

/// The JWS `typ` of a Gaia-X credential.
pub const VC_JWT_TYPE: &str = "vc+jwt";
/// The JWS `typ` of a Gaia-X presentation.
pub const VP_JWT_TYPE: &str = "vp+jwt";

/// How long the credentials and the presentation are valid: 90 days, the
/// longest a compliance credential is valid (ADR 0021).
pub const VALIDITY_SECONDS: u64 = 90 * 24 * 60 * 60;

/// The member that names the terms and conditions hash in a `gx:Issuer`
/// credential subject, written without the `gx:` prefix as in Gaia-X's
/// example and the spike (the Gaia-X context's `@vocab` expands it).
pub const TERMS_AND_CONDITIONS_MEMBER: &str = "gaiaxTermsAndConditions";

/// The legal person's link to its registration number credential.
pub const REGISTRATION_NUMBER_MEMBER: &str = "gx:registrationNumber";

/// When the credentials and the presentation are valid, in seconds since
/// 1970.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    /// `validFrom`.
    pub valid_from: u64,
    /// `validUntil`.
    pub valid_until: u64,
}

impl Period {
    /// From `now` for [`VALIDITY_SECONDS`].
    #[must_use]
    pub fn starting(now: u64) -> Self {
        Self {
            valid_from: now,
            valid_until: now.saturating_add(VALIDITY_SECONDS),
        }
    }
}

/// The `credentialSubject` id of a credential with `id`: `<id>#cs`, as in
/// the spike and the compliance service's own answer.
#[must_use]
pub fn subject_id(id: &str) -> String {
    format!("{id}#cs")
}

/// The `gx:LegalPerson` credential with `id`, issued by `issuer`. `claims`
/// are the participant's own (name, addresses, ...), without `id` and
/// without `gx:registrationNumber`: this function writes both, the
/// subject id as [`subject_id`] and the registration number as a link to
/// `registration_subject`, the subject of the notary's credential.
/// `contexts` come after the VC 2.0 context (the Gaia-X context, and any
/// prefix definitions the claims need).
#[must_use]
pub fn legal_person(
    id: &str,
    issuer: &str,
    contexts: &[Value],
    claims: &Map<String, Value>,
    registration_subject: &str,
    period: Period,
) -> Map<String, Value> {
    let mut subject = Map::new();
    subject.insert("id".to_owned(), Value::from(subject_id(id)));
    for (name, value) in claims {
        subject.insert(name.clone(), value.clone());
    }
    subject.insert(
        REGISTRATION_NUMBER_MEMBER.to_owned(),
        json!({ "id": registration_subject }),
    );
    credential(id, issuer, contexts, LEGAL_PERSON_TYPE, subject, period)
}

/// The `gx:Issuer` credential with `id`: the participant accepts the
/// Gaia-X terms and conditions whose hash is `terms_hash`.
#[must_use]
pub fn terms_and_conditions(
    id: &str,
    issuer: &str,
    contexts: &[Value],
    terms_hash: &str,
    period: Period,
) -> Map<String, Value> {
    let mut subject = Map::new();
    subject.insert("id".to_owned(), Value::from(subject_id(id)));
    subject.insert(
        TERMS_AND_CONDITIONS_MEMBER.to_owned(),
        Value::from(terms_hash),
    );
    credential(id, issuer, contexts, ISSUER_TYPE, subject, period)
}

/// A VC 2.0 credential of one Gaia-X type.
fn credential(
    id: &str,
    issuer: &str,
    contexts: &[Value],
    kind: &str,
    subject: Map<String, Value>,
    period: Period,
) -> Map<String, Value> {
    let mut all_contexts = vec![Value::from(VC2_CONTEXT)];
    all_contexts.extend(contexts.iter().cloned());
    let mut credential = Map::new();
    credential.insert("@context".to_owned(), Value::Array(all_contexts));
    credential.insert("id".to_owned(), Value::from(id));
    credential.insert("type".to_owned(), json!(["VerifiableCredential", kind]));
    credential.insert("issuer".to_owned(), Value::from(issuer));
    write_period(&mut credential, period);
    credential.insert("credentialSubject".to_owned(), Value::Object(subject));
    credential
}

/// The presentation of `credentials` (VC-JWTs), by `holder`.
#[must_use]
pub fn presentation(holder: &str, credentials: &[&str], period: Period) -> Map<String, Value> {
    let mut envelopes = Vec::new();
    for jwt in credentials {
        envelopes.push(envelope(jwt));
    }
    let mut presentation = Map::new();
    presentation.insert("@context".to_owned(), json!([VC2_CONTEXT]));
    presentation.insert("type".to_owned(), Value::from("VerifiablePresentation"));
    presentation.insert("verifiableCredential".to_owned(), Value::Array(envelopes));
    presentation.insert("issuer".to_owned(), Value::from(holder));
    write_period(&mut presentation, period);
    presentation
}

/// One credential inside the presentation, written as in the spike: the
/// `@context` is one string here, not a list.
fn envelope(jwt: &str) -> Value {
    json!({
        "@context": VC2_CONTEXT,
        "id": format!("data:application/vc+jwt,{jwt}"),
        "type": "EnvelopedVerifiableCredential",
    })
}

fn write_period(object: &mut Map<String, Value>, period: Period) {
    object.insert(
        "validFrom".to_owned(),
        Value::from(portitor_time::xsd_date_time(period.valid_from)),
    );
    object.insert(
        "validUntil".to_owned(),
        Value::from(portitor_time::xsd_date_time(period.valid_until)),
    );
}

/// Signs a credential: `typ: vc+jwt`, `cty: vc`, `iss`, `kid`.
///
/// # Errors
/// Signing fails.
pub fn sign_credential(
    signer: Signer<'_>,
    credential: &Map<String, Value>,
) -> Result<String, Error> {
    signer
        .key
        .sign_with_headers(&headers(signer, VC_JWT_TYPE, "vc"), credential)
}

/// Signs a presentation: `typ: vp+jwt`, `cty: vp`, `iss`, `kid`.
///
/// # Errors
/// Signing fails.
pub fn sign_presentation(
    signer: Signer<'_>,
    presentation: &Map<String, Value>,
) -> Result<String, Error> {
    signer
        .key
        .sign_with_headers(&headers(signer, VP_JWT_TYPE, "vp"), presentation)
}

/// The JOSE header members besides `alg`. `kid` must be the verification
/// method's full id: the compliance service compares them exactly (spike,
/// finding 2).
fn headers(signer: Signer<'_>, typ: &str, cty: &str) -> Map<String, Value> {
    let mut headers = Map::new();
    headers.insert("typ".to_owned(), Value::from(typ));
    headers.insert("cty".to_owned(), Value::from(cty));
    headers.insert("iss".to_owned(), Value::from(signer.did));
    headers.insert("kid".to_owned(), Value::from(signer.kid));
    headers
}

/// Why the compliance service's answer is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    /// Not a VC 2.0 credential JWT of type `vc+jwt`.
    #[error("the answer is not a VC-JWT")]
    NotVcJwt,
    /// Not a `gx:ParticipantComplianceCredential`.
    #[error("the answer is not a gx:ParticipantComplianceCredential")]
    NotCompliance,
    /// Its `id` is not the one asked for (`vcid`).
    #[error("the answer's id is {0:?}, not the id asked for")]
    OtherId(Option<String>),
    /// A credential sent is not listed in `gx:compliantCredentials`.
    #[error("the answer does not list {0}")]
    Missing(String),
    /// A listed credential was not sent, or its `digestSRI` is not the
    /// digest of the credential sent.
    #[error("the answer's digestSRI for {0} is not the digest of what was sent")]
    WrongDigest(String),
}

/// Checks the content of the compliance service's answer: a VC-JWT
/// (`typ: vc+jwt`, VC 2.0) of type `gx:ParticipantComplianceCredential`,
/// with the `id` asked for (`vcid`), whose `gx:compliantCredentials`
/// list each credential of `sent` (as `(id, digestSRI)`, the digest from
/// [`crate::gaiax::digest_sri`]) with the same digest, and nothing else
/// (ADR 0021; ADR 0022 ties the holder to its compliance credential with
/// these digests).
///
/// The signature and the dates are not checked here: see
/// [`Credential::verify`].
///
/// # Errors
/// Any of the above does not hold.
pub fn check_answer(
    answer: &Credential,
    vcid: &str,
    sent: &[(String, String)],
) -> Result<(), AnswerError> {
    let typ_is_vc_jwt = Jws::parse(answer.jwt())
        .ok()
        .is_some_and(|jws| jws.typ() == Some(VC_JWT_TYPE));
    if answer.profile != Profile::Vc20 || !typ_is_vc_jwt {
        return Err(AnswerError::NotVcJwt);
    }
    if !answer.has_type(COMPLIANCE_TYPE) {
        return Err(AnswerError::NotCompliance);
    }
    if answer.id.as_deref() != Some(vcid) {
        return Err(AnswerError::OtherId(answer.id.clone()));
    }
    let listed = compliant_credentials(answer);
    // every entry must be one we sent, with our digest
    for entry in &listed {
        let ours = sent
            .iter()
            .any(|(id, digest)| *id == entry.id && *digest == entry.digest_sri);
        if !ours {
            return Err(AnswerError::WrongDigest(entry.id.clone()));
        }
    }
    // and every credential we sent must be listed
    for (id, _) in sent {
        if !listed.iter().any(|entry| entry.id == *id) {
            return Err(AnswerError::Missing(id.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::gaiax::{digest_sri, digest_sri_of};
    use crate::{DidDocument, Relationship, SigningKey, VerificationMethod};

    const DID: &str = "did:web:participant.test";
    const BASE: &str = "https://participant.test/credentials/gaiax";
    const GX: &str = "https://w3id.org/gaia-x/development#";
    const NOW: u64 = 1_794_738_545;

    fn signer_parts() -> (SigningKey, String) {
        (SigningKey::generate().unwrap(), format!("{DID}#key-0"))
    }

    fn header_of(jwt: &str) -> Value {
        use base64::Engine as _;
        let first = jwt.split('.').next().unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(first)
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn credentials_have_the_spike_shape() {
        let claims = json!({"schema:name": "Portitor Test S.A."});
        let Value::Object(claims) = claims else {
            unreachable!()
        };
        let contexts = [Value::from(GX), json!({"schema": "https://schema.org/"})];
        let id = format!("{BASE}/legal-person");
        let period = Period::starting(NOW);
        let legal = legal_person(
            &id,
            DID,
            &contexts,
            &claims,
            "https://notary.test/vat#cs",
            period,
        );
        assert_eq!(
            Value::Object(legal.clone()),
            json!({
                "@context": [VC2_CONTEXT, GX, {"schema": "https://schema.org/"}],
                "id": id,
                "type": ["VerifiableCredential", "gx:LegalPerson"],
                "issuer": DID,
                "validFrom": portitor_time::xsd_date_time(NOW),
                "validUntil": portitor_time::xsd_date_time(NOW + VALIDITY_SECONDS),
                "credentialSubject": {
                    "id": format!("{id}#cs"),
                    "schema:name": "Portitor Test S.A.",
                    "gx:registrationNumber": {"id": "https://notary.test/vat#cs"}
                }
            })
        );
        let terms = terms_and_conditions(
            &format!("{BASE}/terms-and-conditions"),
            DID,
            &contexts[..1],
            "4bd7",
            period,
        );
        assert_eq!(
            terms["credentialSubject"]["gaiaxTermsAndConditions"],
            "4bd7"
        );
        assert_eq!(terms["type"], json!(["VerifiableCredential", "gx:Issuer"]));

        let (key, kid) = signer_parts();
        let signer = Signer {
            key: &key,
            kid: &kid,
            did: DID,
        };
        let jwt = sign_credential(signer, &legal).unwrap();
        assert_eq!(
            header_of(&jwt),
            json!({"alg": "ES256", "typ": "vc+jwt", "cty": "vc", "iss": DID, "kid": kid})
        );
        // it reads back as a VC 2.0 credential, signed by our key
        let read = Credential::parse(&jwt).unwrap();
        assert_eq!(read.profile, Profile::Vc20);
        assert_eq!(read.payload().unwrap(), legal);
        let document = DidDocument {
            id: DID.to_owned(),
            verification_method: vec![VerificationMethod {
                id: kid.clone(),
                kind: "JsonWebKey2020".to_owned(),
                controller: DID.to_owned(),
                public_key_jwk: key.public_jwk(),
            }],
            assertion_method: vec![Relationship::Reference(kid.clone())],
            ..DidDocument::default()
        };
        read.verify(&document, NOW, 0).unwrap();

        let vp = sign_presentation(signer, &presentation(DID, &[&jwt], period)).unwrap();
        assert_eq!(
            header_of(&vp),
            json!({"alg": "ES256", "typ": "vp+jwt", "cty": "vp", "iss": DID, "kid": kid})
        );
        let vp_payload: Value = {
            use base64::Engine as _;
            let middle = vp.split('.').nth(1).unwrap();
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(middle)
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        };
        assert_eq!(
            vp_payload,
            json!({
                "@context": [VC2_CONTEXT],
                "type": "VerifiablePresentation",
                "verifiableCredential": [{
                    "@context": VC2_CONTEXT,
                    "id": format!("data:application/vc+jwt,{jwt}"),
                    "type": "EnvelopedVerifiableCredential"
                }],
                "issuer": DID,
                "validFrom": portitor_time::xsd_date_time(NOW),
                "validUntil": portitor_time::xsd_date_time(NOW + VALIDITY_SECONDS)
            })
        );
    }

    /// A compliance credential `id` listing `entries` (id, digest).
    fn answer(typ: &str, kind: &str, id: &str, entries: &[(&str, &str)]) -> Credential {
        let (key, _) = signer_parts();
        let listed: Vec<Value> = entries
            .iter()
            .map(|(id, digest)| {
                json!({"type": "gx:CompliantCredential", "id": id,
                       "gx:credentialType": "gx:LegalPerson", "digestSRI": digest})
            })
            .collect();
        let payload = json!({
            "@context": [VC2_CONTEXT, GX],
            "id": id,
            "type": ["VerifiableCredential", kind],
            "issuer": "did:web:compliance.test",
            "validFrom": "2026-11-15T09:00:00Z",
            "validUntil": "2027-02-13T09:00:00Z",
            "credentialSubject": {"id": format!("{id}#cs"), "gx:compliantCredentials": listed}
        });
        let mut headers = Map::new();
        headers.insert("typ".to_owned(), Value::from(typ));
        Credential::parse(&key.sign_with_headers(&headers, &payload).unwrap()).unwrap()
    }

    #[test]
    fn the_answer_must_list_exactly_what_was_sent() {
        let vcid = format!("{BASE}/compliance");
        let mut credential = Map::new();
        credential.insert("id".to_owned(), Value::from("a"));
        let digest = digest_sri(&credential);
        let sent = vec![
            ("a".to_owned(), digest.clone()),
            ("b".to_owned(), "sha256-b".to_owned()),
        ];
        let good = answer(
            "vc+jwt",
            COMPLIANCE_TYPE,
            &vcid,
            &[("a", &digest), ("b", "sha256-b")],
        );
        assert_eq!(check_answer(&good, &vcid, &sent), Ok(()));
        assert!(digest_sri_of(&good).unwrap().starts_with("sha256-"));

        let refused = |answer: Credential| check_answer(&answer, &vcid, &sent).unwrap_err();
        assert_eq!(
            refused(answer(
                "vc+jwt",
                COMPLIANCE_TYPE,
                &vcid,
                &[("a", "sha256-0"), ("b", "sha256-b")]
            )),
            AnswerError::WrongDigest("a".to_owned())
        );
        assert_eq!(
            refused(answer("vc+jwt", COMPLIANCE_TYPE, &vcid, &[("a", &digest)])),
            AnswerError::Missing("b".to_owned())
        );
        assert_eq!(
            refused(answer(
                "vc+jwt",
                COMPLIANCE_TYPE,
                &vcid,
                &[("a", &digest), ("b", "sha256-b"), ("c", "sha256-c")]
            )),
            AnswerError::WrongDigest("c".to_owned())
        );
        assert_eq!(
            refused(answer("vc+jwt", "gx:LegalPerson", &vcid, &[])),
            AnswerError::NotCompliance
        );
        assert_eq!(
            refused(answer("vc+jwt", COMPLIANCE_TYPE, "urn:other", &[])),
            AnswerError::OtherId(Some("urn:other".to_owned()))
        );
        assert_eq!(
            refused(answer("JWT", COMPLIANCE_TYPE, &vcid, &[])),
            AnswerError::NotVcJwt
        );
    }
}
