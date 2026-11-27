// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The DCP verifier against a small dataspace on a local port: a holder
//! (the caller) with its Credential Service, an issuer with a status list,
//! a third party, and our own identity, all `did:web` DIDs served over
//! plain HTTP.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse as _;
use portitor_dcp::credential::{VC1_CONTEXT, VC2_CONTEXT};
use portitor_dcp::{Claims, Relationship, Service, SigningKey, VerificationMethod};
use portitor_runtime::system::xsd_date_time;
use serde_json::json;

use super::*;

/// The used token ids in an in-memory store of the connector, as in
/// production (TD-09).
fn used_token_ids() -> Arc<dyn UsedTokenStore> {
    Arc::new(crate::store::ConnectorStore::in_memory().unwrap())
}

/// One participant: its DID and key, and its DID document as served.
struct Party {
    did: String,
    key: SigningKey,
}

impl Party {
    fn new(base: &str, name: &str) -> Self {
        let host = base.trim_start_matches("http://").replace(':', "%3A");
        Self {
            did: format!("did:web:{host}:{name}"),
            key: SigningKey::generate().unwrap(),
        }
    }

    fn kid(&self) -> String {
        format!("{}#key-1", self.did)
    }

    /// Its DID document: one key for every purpose, unless
    /// `capability_invocation` is false; the Credential Service at `base`.
    fn document(&self, base: &str, capability_invocation: bool) -> Value {
        let vm = VerificationMethod {
            id: self.kid(),
            kind: "JsonWebKey2020".into(),
            controller: self.did.clone(),
            public_key_jwk: self.key.public_jwk(),
        };
        let all = vec![Relationship::Reference(self.kid())];
        let doc = DidDocument {
            id: self.did.clone(),
            verification_method: vec![vm],
            capability_invocation: if capability_invocation {
                all.clone()
            } else {
                Vec::new()
            },
            authentication: all.clone(),
            assertion_method: all,
            service: vec![Service {
                id: format!("{}#cs", self.did),
                kind: "CredentialService".into(),
                service_endpoint: Value::from(base),
            }],
        };
        serde_json::to_value(doc).unwrap()
    }

    fn sign(&self, payload: &Value) -> String {
        self.key.sign_json(&self.kid(), "JWT", payload).unwrap()
    }

    /// A self-issued ID token for `audience`, with `access` as its `token`
    /// claim.
    fn id_token(&self, audience: &str, access: Option<&str>, jti: &str) -> String {
        let now = now_secs();
        let claims = Claims {
            iss: self.did.clone(),
            sub: self.did.clone(),
            aud: audience.to_owned(),
            jti: jti.to_owned(),
            exp: now + 300,
            iat: now,
            nbf: None,
            token: access.map(str::to_owned),
        };
        self.key.sign(&self.kid(), &claims).unwrap()
    }
}

/// The TEST ONLY RSA key (PKCS#1 DER) the fake Gaia-X compliance
/// service signs with (PS256, as the real one).
const TEST_RSA_KEY: &[u8] =
    include_bytes!("../../../../portitor-dcp/tests/fixtures/test-only-rsa-2048.der");

/// A Gaia-X JSON-LD context (the one of the 0.8 spike).
const GAIAX_CONTEXT: &str = "https://w3id.org/gaia-x/development#";

/// A fake Gaia-X compliance service: its DID document has the shape of
/// the Gaia-X Lab's (`JsonWebKey`, an RSA `publicKeyJwk` with `kid`,
/// `alg` and `x5u`), and it signs as the service does (0.8 spike,
/// finding 4): PS256, `iat` and `exp` in milliseconds in the header.
struct ComplianceService {
    did: String,
    key: ring::signature::RsaKeyPair,
}

impl ComplianceService {
    fn new(base: &str) -> Self {
        let host = base.trim_start_matches("http://").replace(':', "%3A");
        Self {
            did: format!("did:web:{host}:compliance"),
            key: ring::signature::RsaKeyPair::from_der(TEST_RSA_KEY).unwrap(),
        }
    }

    fn kid(&self) -> String {
        format!("{}#X509-JWK", self.did)
    }

    fn document(&self) -> Value {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
        use ring::signature::{KeyPair as _, RsaPublicKeyComponents};
        let public = RsaPublicKeyComponents::<Vec<u8>>::from(self.key.public_key());
        json!({
            "@context": ["https://www.w3.org/ns/did/v1", "https://w3id.org/security/jwk/v1"],
            "id": self.did,
            "verificationMethod": [{
                "id": self.kid(), "type": "JsonWebKey", "controller": self.did,
                "publicKeyJwk": {"kty": "RSA", "n": B64.encode(&public.n), "e": B64.encode(&public.e),
                    "kid": "X509-JWK", "alg": "PS256", "x5u": "https://compliance.test/cert.pem"}
            }],
            "assertionMethod": [self.kid()]
        })
    }

    fn sign(&self, payload: &Value) -> String {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
        let milliseconds = now_secs() * 1000;
        let header = json!({"alg": "PS256", "typ": "vc+jwt", "cty": "vc", "iss": self.did,
            "kid": self.kid(), "iat": milliseconds, "exp": milliseconds + 90 * 86_400_000});
        let input = format!(
            "{}.{}",
            B64.encode(serde_json::to_vec(&header).unwrap()),
            B64.encode(serde_json::to_vec(payload).unwrap())
        );
        let mut signature = vec![0; self.key.public().modulus_len()];
        self.key
            .sign(
                &ring::signature::RSA_PSS_SHA256,
                &ring::rand::SystemRandom::new(),
                input.as_bytes(),
                &mut signature,
            )
            .unwrap();
        format!("{input}.{}", B64.encode(&signature))
    }
}

/// What the local server serves.
#[derive(Default)]
struct World {
    /// DID documents by path, for example `holder/did.json`.
    documents: BTreeMap<String, Value>,
    /// The Credential Service's answer.
    answer: Value,
    /// How long the Credential Service takes to answer.
    delay: Duration,
    /// Status list credentials by name, for example `1` for
    /// `/status/1` (a JWT, or anything for tests). Others answer 404.
    status_lists: BTreeMap<String, String>,
    /// The `Authorization` headers the Credential Service got.
    queries: Vec<String>,
    /// The scopes of each query the Credential Service got.
    scopes: Vec<Vec<String>>,
    /// Every DID document and status list asked for, by path.
    fetched: Vec<String>,
}

type Shared = Arc<Mutex<World>>;

async fn did_document(
    State(world): State<Shared>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    let mut world = world.lock().unwrap();
    world.fetched.push(format!("/{name}/did.json"));
    match world.documents.get(&name) {
        Some(doc) => axum::Json(doc.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn presentation_query(
    State(world): State<Shared>,
    headers: HeaderMap,
    axum::Json(query): axum::Json<Value>,
) -> axum::response::Response {
    let (answer, delay) = {
        let mut world = world.lock().unwrap();
        let scopes = query["scope"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|scope| scope.as_str().map(str::to_owned))
            .collect();
        world.scopes.push(scopes);
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        world.queries.push(auth);
        (world.answer.clone(), world.delay)
    };
    tokio::time::sleep(delay).await;
    axum::Json(answer).into_response()
}

async fn status_list(
    State(world): State<Shared>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    let mut world = world.lock().unwrap();
    world.fetched.push(format!("/status/{name}"));
    match world.status_lists.get(&name) {
        Some(list) => list.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Our STS: signs as `me`.
struct LocalSts(Party);

impl SecureTokenService for LocalSts {
    fn token<'a>(&'a self, request: TokenRequest<'a>) -> BoxFuture<'a, Result<String, String>> {
        let jti = portitor_runtime::system::new_uuid();
        let token = self
            .0
            .id_token(request.audience, request.access_token, &jti);
        Box::pin(async move { Ok(token) })
    }
}

/// The dataspace: parties, the server, and our verifier.
struct Dataspace {
    base: String,
    world: Shared,
    holder: Party,
    issuer: Party,
    third: Party,
    compliance: ComplianceService,
    me_did: String,
    verifier: DcpVerifier,
}

async fn dataspace() -> Dataspace {
    dataspace_caching_did_documents(300).await
}

/// [`dataspace`], with a verifier that reuses a DID document for
/// `did_cache_seconds` (`PORTITOR_DID_CACHE_SECONDS`).
async fn dataspace_caching_did_documents(did_cache_seconds: u64) -> Dataspace {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let world: Shared = Arc::default();
    let app = axum::Router::new()
        .route("/{name}/did.json", axum::routing::get(did_document))
        .route(
            "/presentations/query",
            axum::routing::post(presentation_query),
        )
        .route("/status/{name}", axum::routing::get(status_list))
        .with_state(Arc::clone(&world));
    tokio::spawn(async move { axum::serve(listener, app).await });
    let (holder, issuer, third, me) = (
        Party::new(&base, "holder"),
        Party::new(&base, "issuer"),
        Party::new(&base, "third"),
        Party::new(&base, "me"),
    );
    let compliance = ComplianceService::new(&base);
    {
        let mut w = world.lock().unwrap();
        w.documents
            .insert("compliance".into(), compliance.document());
        for (name, party) in [
            ("holder", &holder),
            ("issuer", &issuer),
            ("third", &third),
            ("me", &me),
        ] {
            w.documents.insert(name.into(), party.document(&base, true));
        }
    }
    let settings = AuthSettings {
        participant_id: me.did.clone(),
        sts_url: String::new(),
        sts_allow_http: false,
        sts_client_id: String::new(),
        sts_client_secret: String::new(),
        trusted_issuers: vec![issuer.did.clone()],
        issuer_types: portitor_dcp::trust::IssuerTypes::default(),
        gaiax_compliance_issuers: vec![compliance.did.clone()],
        scopes: vec![super::super::DEFAULT_SCOPE.to_owned()],
        allow_http: true,
        status_list_cache_seconds: 0,
        did_cache_seconds,
    };
    let me_did = me.did.clone();
    let mut verifier =
        DcpVerifier::new(&settings, Arc::new(LocalSts(me)), used_token_ids()).unwrap();
    verifier.start_tries = 1;
    Dataspace {
        base,
        world,
        holder,
        issuer,
        third,
        compliance,
        me_did,
        verifier,
    }
}

impl Dataspace {
    /// A VC 1.1 credential JWT (the DCP TCK's shape) by `issuer`, about
    /// `subject`, with `extra` fields in the `vc` claim.
    fn credential(issuer: &Party, kind: &str, subject: &str, extra: &Value) -> String {
        let now = now_secs();
        let mut vc = json!({
            "@context": [VC1_CONTEXT],
            "id": "urn:uuid:c-1",
            "type": ["VerifiableCredential", kind],
            "issuer": issuer.did,
            "issuanceDate": "2026-01-01T00:00:00Z",
            "credentialSubject": {"id": subject}
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            vc[key] = value.clone();
        }
        issuer.sign(
            &json!({"iss": issuer.did, "sub": subject, "jti": "j", "nbf": now - 10,
            "exp": now + 300, "vc": vc}),
        )
    }

    /// A VC 2.0 credential JWT (`vc+jwt`) by `issuer`, about `subject`,
    /// valid from yesterday for a year unless `extra` says otherwise.
    fn credential20(issuer: &Party, kind: &str, subject: &str, extra: &Value) -> String {
        let mut vc = json!({
            "@context": [VC2_CONTEXT],
            "id": "urn:uuid:c-2",
            "type": ["VerifiableCredential", kind],
            "issuer": {"id": issuer.did},
            "validFrom": xsd_date_time(now_secs() - 86_400),
            "validUntil": xsd_date_time(now_secs() + 365 * 86_400),
            "credentialSubject": {"id": subject}
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            vc[key] = value.clone();
        }
        issuer.key.sign_json(&issuer.kid(), "vc+jwt", &vc).unwrap()
    }

    /// A VC 2.0 presentation JWT (`vp+jwt`) by `holder` for us, with the
    /// `credentials` enveloped (`EnvelopedVerifiableCredential`).
    fn presentation20(&self, holder: &Party, credentials: &[String]) -> String {
        let now = now_secs();
        let entries: Vec<Value> = credentials
            .iter()
            .map(|jwt| {
                json!({"@context": [VC2_CONTEXT], "type": "EnvelopedVerifiableCredential",
                    "id": format!("data:application/vc+jwt,{jwt}")})
            })
            .collect();
        holder
            .key
            .sign_json(
                &holder.kid(),
                "vp+jwt",
                &json!({"@context": [VC2_CONTEXT], "type": "VerifiablePresentation",
                    "holder": holder.did, "iss": holder.did, "aud": self.me_did,
                    "iat": now, "exp": now + 300, "verifiableCredential": entries}),
            )
            .unwrap()
    }

    /// `presentation` in an `EnvelopedVerifiablePresentation` signed by
    /// `by`, as EDC Identity Hub sends VC 2.0 presentations.
    fn envelope(&self, by: &Party, presentation: &str) -> String {
        let now = now_secs();
        by.key
            .sign_json(
                &by.kid(),
                "vp+jwt",
                &json!({"@context": [VC2_CONTEXT], "type": "EnvelopedVerifiablePresentation",
                    "id": format!("data:application/vp+jwt,{presentation}"),
                    "iss": by.did, "aud": self.me_did, "iat": now, "exp": now + 300}),
            )
            .unwrap()
    }

    /// A VC 1.1 presentation JWT by `holder` for us, with `credentials`.
    fn presentation(&self, holder: &Party, credentials: &[String]) -> String {
        let now = now_secs();
        holder.sign(
            &json!({"iss": holder.did, "sub": holder.did, "aud": self.me_did,
            "nbf": now - 10, "exp": now + 300,
            "vp": {"@context": [VC1_CONTEXT], "type": "VerifiablePresentation",
                "verifiableCredential": credentials}}),
        )
    }

    /// The Credential Service answers with `presentations`.
    fn answer(&self, presentations: &[String]) {
        self.world.lock().unwrap().answer = json!({
            "@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
            "type": "PresentationResponseMessage",
            "presentation": presentations
        });
    }

    /// The usual answer: one presentation with a trusted membership
    /// credential, optionally with `extra` credential fields.
    fn answer_member(&self, extra: &Value) {
        let credential = Self::credential(&self.issuer, MEMBERSHIP, &self.holder.did, extra);
        self.answer(&[self.presentation(&self.holder, &[credential])]);
    }

    /// A revocation list by `issuer` at `/status/1`, with the `revoked`
    /// bits set.
    fn publish_status_list(&self, issuer: &Party, revoked: &[usize]) {
        self.publish_list("1", issuer, "revocation", revoked, &json!({}));
    }

    /// A Bitstring Status List credential by `issuer` at `/status/<name>`
    /// for `purpose`, with the `set` bits set and `extra` fields.
    fn publish_list(
        &self,
        name: &str,
        issuer: &Party,
        purpose: &str,
        set: &[usize],
        extra: &Value,
    ) {
        let mut vc = json!({
            "@context": [VC2_CONTEXT],
            "type": ["VerifiableCredential", "BitstringStatusListCredential"],
            "issuer": issuer.did,
            "credentialSubject": {"id": "urn:uuid:list", "type": "BitstringStatusList",
                "statusPurpose": purpose, "encodedList": encoded_bits(set)}
        });
        for (key, value) in extra.as_object().into_iter().flatten() {
            vc[key] = value.clone();
        }
        self.serve_list(name, issuer.sign(&vc));
    }

    /// Serves `content` at `/status/<name>`.
    fn serve_list(&self, name: &str, content: String) {
        self.world
            .lock()
            .unwrap()
            .status_lists
            .insert(name.to_owned(), content);
    }

    /// A revocation entry at `index` of `/status/1`.
    fn status_entry(&self, index: &str) -> Value {
        json!({"credentialStatus": self.entry("1", "BitstringStatusListEntry", "revocation", index)})
    }

    /// One `credentialStatus` entry of `kind` for `purpose`, pointing to
    /// `index` of `/status/<list>`.
    fn entry(&self, list: &str, kind: &str, purpose: &str, index: &str) -> Value {
        json!({"id": format!("urn:uuid:s-{list}"), "type": kind,
            "statusPurpose": purpose, "statusListIndex": index,
            "statusListCredential": format!("{}/status/{list}", self.base)})
    }

    /// Clears the record of fetched documents and lists.
    fn forget_fetches(&self) {
        self.world.lock().unwrap().fetched.clear();
    }

    /// The documents and lists fetched since [`Dataspace::forget_fetches`].
    fn fetched(&self) -> Vec<String> {
        self.world.lock().unwrap().fetched.clone()
    }

    /// The caller's `Authorization` header: the holder's token for us.
    fn header(&self) -> String {
        format!(
            "Bearer {}",
            self.holder.id_token(
                &self.me_did,
                Some("access-1"),
                &portitor_runtime::system::new_uuid()
            )
        )
    }

    /// The check of `header`; the error as text, for the reason.
    async fn verify(&self, header: &str) -> Result<Caller, String> {
        self.verifier
            .verify(Some(header))
            .await
            .map_err(|error| error.to_string())
    }

    /// The error a fresh header of the holder gets, with its kind.
    async fn verify_error(&self) -> VerifyError {
        self.verifier
            .verify(Some(&self.header()))
            .await
            .unwrap_err()
    }

    /// The reason a fresh header of the holder is refused.
    async fn refusal(&self) -> String {
        self.verify(&self.header()).await.unwrap_err()
    }
}

/// A 2,048-byte bitstring with the `set` bits set, as an `encodedList`.
fn encoded_bits(set: &[usize]) -> String {
    let mut bits = vec![0u8; 2048];
    for &index in set {
        let index = u64::try_from(index).unwrap();
        portitor_dcp::status_list::set_bit(&mut bits, index).unwrap();
    }
    portitor_dcp::status_list::encode_list(&bits)
}

#[tokio::test]
async fn a_member_is_let_in_and_its_credential_service_gets_our_token() {
    let ds = dataspace().await;
    ds.answer_member(&json!({}));
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.id, ds.holder.did);
    assert_eq!(caller.access_token(), Some("access-1"));
    assert_eq!(caller.credentials.len(), 1);
    assert!(caller.credentials[0].types.iter().any(|t| t == MEMBERSHIP));
    // our query carried our token for the holder, with its access token
    let query = ds.world.lock().unwrap().queries.pop().unwrap();
    let token = query.strip_prefix("Bearer ").unwrap();
    let claims = unverified_claims(token).unwrap();
    assert_eq!(
        (claims.iss.as_str(), claims.aud.as_str()),
        (ds.me_did.as_str(), ds.holder.did.as_str())
    );
    assert_eq!(claims.token.as_deref(), Some("access-1"));
    // our own identity check passes too
    ds.verifier.check_own_identity().await.unwrap();
}

#[tokio::test]
async fn tokens_are_checked() {
    let ds = dataspace().await;
    ds.answer_member(&json!({}));
    let header = ds.header();
    let token = header.strip_prefix("Bearer ").unwrap();
    let now = now_secs();
    let claims = |iss: &str, sub: &str, aud: &str, exp: u64, access: Option<&str>| Claims {
        iss: iss.to_owned(),
        sub: sub.to_owned(),
        aud: aud.to_owned(),
        jti: portitor_runtime::system::new_uuid(),
        exp,
        iat: now,
        nbf: None,
        token: access.map(str::to_owned),
    };
    let holder = &ds.holder;
    let me = ds.me_did.as_str();
    let signed = |by: &Party, claims: Claims| {
        format!("Bearer {}", by.key.sign(&holder.kid(), &claims).unwrap())
    };
    let mut from_the_future = claims(&holder.did, &holder.did, me, now + 600, Some("a"));
    from_the_future.iat = now + 300;
    // each header, and the reason it is refused for
    let refused = [
        (token.to_owned(), "not a Bearer token"),
        ("Bearer ".to_owned(), "not a Bearer token"),
        ("Basic abc".to_owned(), "not a Bearer token"),
        (
            signed(
                holder,
                claims(&holder.did, &ds.third.did, me, now + 300, Some("a")),
            ),
            "iss and sub differ",
        ),
        (
            signed(
                holder,
                claims(
                    &holder.did,
                    &holder.did,
                    &ds.third.did,
                    now + 300,
                    Some("a"),
                ),
            ),
            "aud is not the verifier",
        ),
        (
            signed(
                holder,
                claims(&holder.did, &holder.did, me, now - 3_600, Some("a")),
            ),
            "expired",
        ),
        (
            signed(
                holder,
                claims(&holder.did, &holder.did, me, now + 7_200, Some("a")),
            ),
            "valid for too long",
        ),
        (signed(holder, from_the_future), "issued in the future"),
        (
            signed(
                holder,
                claims(&holder.did, &holder.did, me, now + 300, None),
            ),
            "no access token",
        ),
        (
            signed(
                &ds.third,
                claims(&holder.did, &holder.did, me, now + 300, Some("a")),
            ),
            "bad signature",
        ),
    ];
    let err = ds.verifier.verify(None).await.unwrap_err();
    assert!(err.reason().contains("no Authorization header"), "{err}");
    assert!(!err.is_temporary());
    for (header, reason) in refused {
        let err = ds.verify(&header).await.unwrap_err();
        assert!(err.contains(reason), "{reason}: {err}");
    }
    // a token is good once
    assert!(ds.verify(&header).await.is_ok());
    assert!(
        ds.verify(&header)
            .await
            .unwrap_err()
            .contains("jti replayed")
    );
}

#[tokio::test]
async fn a_key_without_capability_invocation_is_refused() {
    let ds = dataspace().await;
    ds.answer_member(&json!({}));
    ds.world
        .lock()
        .unwrap()
        .documents
        .insert("holder".into(), ds.holder.document(&ds.base, false));
    let err = ds.verify(&ds.header()).await.unwrap_err();
    assert!(err.contains("capabilityInvocation"), "{err}");
}

#[tokio::test]
async fn presentations_and_credentials_are_checked() {
    let ds = dataspace().await;
    let header = || ds.header();
    let member = |issuer: &Party, subject: &str| {
        Dataspace::credential(issuer, MEMBERSHIP, subject, &json!({}))
    };
    let holder = ds.holder.did.as_str();
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("empty presentation array", vec![]),
        ("no credentials", vec![ds.presentation(&ds.holder, &[])]),
        (
            "another credential type",
            vec![ds.presentation(
                &ds.holder,
                &[Dataspace::credential(
                    &ds.issuer,
                    "SensitiveDataCredential",
                    holder,
                    &json!({}),
                )],
            )],
        ),
        (
            "untrusted issuer",
            vec![ds.presentation(&ds.holder, &[member(&ds.third, holder)])],
        ),
        (
            "subject is someone else",
            vec![ds.presentation(&ds.holder, &[member(&ds.issuer, &ds.third.did)])],
        ),
        (
            "holder is someone else",
            vec![ds.presentation(&ds.third, &[member(&ds.issuer, holder)])],
        ),
        (
            "expired",
            vec![ds.presentation(
                &ds.holder,
                &[Dataspace::credential(
                    &ds.issuer,
                    MEMBERSHIP,
                    holder,
                    &json!({"expirationDate": "2026-01-02T00:00:00Z"}),
                )],
            )],
        ),
        (
            "not yet valid",
            vec![ds.presentation(
                &ds.holder,
                &[Dataspace::credential(
                    &ds.issuer,
                    MEMBERSHIP,
                    holder,
                    &json!({"issuanceDate": "2999-01-01T00:00:00Z"}),
                )],
            )],
        ),
        (
            "a credential signed by another key",
            vec![ds.presentation(
                &ds.holder,
                &[{
                    let forged = Party {
                        did: ds.issuer.did.clone(),
                        key: SigningKey::generate().unwrap(),
                    };
                    member(&forged, holder)
                }],
            )],
        ),
    ];
    // the reason each case is refused for
    let expected = |what: &str| match what {
        "empty presentation array" => "is empty",
        "subject is someone else" => "not about the caller",
        "holder is someone else" => "holder is not the caller",
        "expired" => "expired",
        "not yet valid" => "not yet valid",
        "a credential signed by another key" => "bad signature",
        _ => "no MembershipCredential from a trusted issuer",
    };
    for (what, presentations) in cases {
        ds.answer(&presentations);
        let err = ds.verify(&header()).await.unwrap_err();
        assert!(err.contains(expected(what)), "{what}: {err}");
    }
    // a presentation that is not a JWT
    ds.world.lock().unwrap().answer = json!({"@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
        "type": "PresentationResponseMessage", "presentation": [{"a": 1}]});
    let err = ds.verify(&header()).await.unwrap_err();
    assert!(err.contains("a presentation is not a JWT"), "{err}");
    // not a presentation response
    ds.world.lock().unwrap().answer = json!({"presentation": []});
    let err = ds.verify(&header()).await.unwrap_err();
    assert!(err.starts_with("presentation response:"), "{err}");
    // more credentials than asked for are fine
    ds.answer(&[ds.presentation(
        &ds.holder,
        &[
            member(&ds.issuer, holder),
            Dataspace::credential(&ds.issuer, "SensitiveDataCredential", holder, &json!({})),
        ],
    )]);
    assert!(ds.verify(&header()).await.is_ok());
}

/// A revoked credential, and a server of the caller or its issuer that
/// answers 404, refuse the caller (HTTP 404, or 401 on the catalog
/// routes). A server that does not
/// answer is a temporary failure (HTTP 503): the caller may be fine.
#[tokio::test]
async fn unreachable_services_are_temporary_and_refusals_are_not() {
    let ds = dataspace().await;
    // nobody listens on port 9 (discard)
    let down = "http://127.0.0.1:9";

    // revoked: a refusal
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[5]);
    let revoked = ds.verify_error().await;
    assert!(!revoked.is_temporary(), "{revoked}");
    assert!(revoked.reason().contains("revoked"), "{revoked}");

    // the status list's host does not answer: temporary
    let mut entry = ds.entry("1", "BitstringStatusListEntry", "revocation", "5");
    entry["statusListCredential"] = Value::from(format!("{down}/status/1"));
    ds.answer_member(&json!({"credentialStatus": entry}));
    let unreachable = ds.verify_error().await;
    assert!(unreachable.is_temporary(), "{unreachable}");

    // the status list is not there (404): a refusal
    ds.answer_member(
        &json!({"credentialStatus": ds.entry("1", "BitstringStatusListEntry", "revocation", "9")}),
    );
    ds.world.lock().unwrap().status_lists.clear();
    let missing = ds.verify_error().await;
    assert!(!missing.is_temporary(), "{missing}");

    // the holder's DID document is cached now, so the last two cases
    // use callers not seen before
    let token_of = |caller: &Party| {
        let jti = portitor_runtime::system::new_uuid();
        format!("Bearer {}", caller.id_token(&ds.me_did, Some("a"), &jti))
    };

    // the caller's Credential Service does not answer: temporary
    let moved = Party::new(&ds.base, "moved");
    ds.world
        .lock()
        .unwrap()
        .documents
        .insert("moved".into(), moved.document(down, true));
    let no_service = ds.verifier.verify(Some(&token_of(&moved))).await;
    let no_service = no_service.unwrap_err();
    assert!(no_service.is_temporary(), "{no_service}");

    // the caller's DID document is not there (404): a refusal
    let unknown = Party::new(&ds.base, "unknown");
    let no_document = ds.verifier.verify(Some(&token_of(&unknown))).await;
    let no_document = no_document.unwrap_err();
    assert!(!no_document.is_temporary(), "{no_document}");
}

#[tokio::test]
async fn revoked_credentials_and_bad_status_lists_are_refused() {
    let ds = dataspace().await;
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[3]);
    assert!(ds.verify(&ds.header()).await.is_ok(), "bit 5 is clear");
    ds.publish_status_list(&ds.issuer, &[5]);
    let err = ds.refusal().await;
    assert!(err.contains("revoked or suspended (revocation)"), "{err}");

    // each list, and the reason it is refused for
    let unsigned = json!({"credentialSubject": {}}).to_string();
    let too_large = "a".repeat(MAX_STATUS_LIST + 1);
    for (list, reason) in [
        (None, "has another issuer"),
        (Some(unsigned), "malformed token"),
        (Some(too_large), "answer not read"),
    ] {
        match list {
            None => ds.publish_status_list(&ds.third, &[]),
            Some(text) => ds.serve_list("1", text),
        }
        let err = ds.refusal().await;
        assert!(err.contains(reason), "{reason}: {err}");
    }
    // an expired list, signed by the issuer
    ds.publish_list(
        "1",
        &ds.issuer,
        "revocation",
        &[],
        &json!({"validUntil": "2026-01-01T00:00:00Z"}),
    );
    let err = ds.refusal().await;
    assert!(
        err.contains("status list") && err.contains("expired"),
        "{err}"
    );
    // no list at the URL
    ds.world.lock().unwrap().status_lists.clear();
    let err = ds.refusal().await;
    assert!(err.contains("HTTP 404"), "{err}");
    // an index past the end of the list
    ds.answer_member(&ds.status_entry("999999"));
    ds.publish_status_list(&ds.issuer, &[]);
    let err = ds.refusal().await;
    assert!(err.contains("out of range"), "{err}");
}

#[tokio::test]
async fn a_revoked_credential_next_to_a_valid_one_refuses_the_caller() {
    // the authority revoked the first membership credential (bit 5) and
    // issued a new one (bit 6); the holder still presents both. DCP
    // v1.0.1, 5.4.3, step 8: the presentation is invalid
    let ds = dataspace().await;
    let holder = ds.holder.did.as_str();
    let member_at = |index: &str| {
        Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &ds.status_entry(index))
    };
    let revoked_sensitive = Dataspace::credential(
        &ds.issuer,
        "SensitiveDataCredential",
        holder,
        &ds.status_entry("7"),
    );
    ds.publish_status_list(&ds.issuer, &[5, 7]);
    ds.answer(&[ds.presentation(
        &ds.holder,
        &[member_at("5"), member_at("6"), revoked_sensitive],
    )]);
    let err = ds.refusal().await;
    assert!(
        err.contains("presentation is invalid")
            && err.contains("revoked or suspended (revocation)"),
        "{err}"
    );
    // the same with only the revoked sensitive data credential next to
    // the valid membership credential
    let revoked_sensitive = Dataspace::credential(
        &ds.issuer,
        "SensitiveDataCredential",
        holder,
        &ds.status_entry("7"),
    );
    ds.answer(&[ds.presentation(&ds.holder, &[member_at("6"), revoked_sensitive])]);
    let err = ds.refusal().await;
    assert!(
        err.contains("a SensitiveDataCredential credential"),
        "{err}"
    );

    // the holder deletes the revoked credentials from its identity
    // (`DELETE /v1/credentials/{id}`): admitted on the new one
    ds.answer(&[ds.presentation(&ds.holder, &[member_at("6")])]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 1);
    assert!(caller.credentials[0].has_type(MEMBERSHIP));

    // a bad presentation refuses the caller too, valid credentials or
    // not: signed by someone else, or holding a credential about another
    // participant
    let forged_holder = Party {
        did: ds.holder.did.clone(),
        key: SigningKey::generate().unwrap(),
    };
    ds.answer(&[ds.presentation(&forged_holder, &[member_at("6")])]);
    let err = ds.refusal().await;
    assert!(err.contains("presentation:"), "{err}");
    let about_third = Dataspace::credential(&ds.issuer, MEMBERSHIP, &ds.third.did, &json!({}));
    ds.answer(&[ds.presentation(&ds.holder, &[member_at("6"), about_third])]);
    let err = ds.refusal().await;
    assert!(err.contains("not about the caller"), "{err}");
}

#[tokio::test]
async fn revocation_and_suspension_lists_are_both_checked() {
    let ds = dataspace().await;
    let entries = json!({"credentialStatus": [
        ds.entry("1", "BitstringStatusListEntry", "revocation", "5"),
        ds.entry("2", "BitstringStatusListEntry", "suspension", "7"),
    ]});
    ds.answer_member(&entries);
    ds.publish_status_list(&ds.issuer, &[]);
    ds.publish_list("2", &ds.issuer, "suspension", &[], &json!({}));
    assert!(ds.verify(&ds.header()).await.is_ok());
    ds.publish_list("2", &ds.issuer, "suspension", &[7], &json!({}));
    let err = ds.refusal().await;
    assert!(err.contains("revoked or suspended (suspension)"), "{err}");
}

#[tokio::test]
async fn a_vc11_status_list_2021_is_read() {
    let ds = dataspace().await;
    ds.answer_member(&json!({"credentialStatus":
        ds.entry("1", "StatusList2021Entry", "revocation", "5")}));
    let now = now_secs();
    // VC 1.1 shape: the credential in the `vc` claim, the list in standard
    // base64 without the multibase prefix
    let list = |set: &[usize]| {
        let multibase = encoded_bits(set);
        let raw = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &multibase[1..],
        )
        .unwrap();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, raw);
        ds.issuer.sign(&json!({"iss": ds.issuer.did, "sub": "urn:uuid:list",
            "nbf": now - 10, "exp": now + 300, "vc": {
                "@context": [VC1_CONTEXT], "type": ["VerifiableCredential", "StatusList2021Credential"],
                "issuer": ds.issuer.did, "issuanceDate": "2026-01-01T00:00:00Z",
                "credentialSubject": {"id": "urn:uuid:list", "type": "StatusList2021",
                    "statusPurpose": "revocation", "encodedList": encoded}}}))
    };
    ds.serve_list("1", list(&[4]));
    assert!(ds.verify(&ds.header()).await.is_ok(), "bit 5 is clear");
    ds.serve_list("1", list(&[5]));
    let err = ds.refusal().await;
    assert!(err.contains("revoked or suspended (revocation)"), "{err}");
}

#[tokio::test]
async fn vc20_and_enveloped_presentations_are_checked() {
    let ds = dataspace().await;
    let holder = ds.holder.did.as_str();
    let member20 = |extra: &Value| Dataspace::credential20(&ds.issuer, MEMBERSHIP, holder, extra);
    let revocable = ds.entry("1", "BitstringStatusListEntry", "revocation", "5");
    let suspendable = ds.entry("2", "BitstringStatusListEntry", "suspension", "5");
    ds.publish_status_list(&ds.issuer, &[5]);
    ds.publish_list("2", &ds.issuer, "suspension", &[5], &json!({}));
    // the same checks for a plain VC 2.0 presentation and an enveloped one
    let wrap = |enveloped: bool, presentation: String| {
        if enveloped {
            ds.envelope(&ds.holder, &presentation)
        } else {
            presentation
        }
    };
    for enveloped in [false, true] {
        let present = |credential: String| {
            ds.answer(&[wrap(
                enveloped,
                ds.presentation20(&ds.holder, &[credential]),
            )]);
        };
        present(member20(&json!({})));
        let caller = ds.verify(&ds.header()).await;
        assert_eq!(
            caller.map(|caller| caller.id),
            Ok(holder.to_owned()),
            "enveloped: {enveloped}"
        );

        let past = xsd_date_time(now_secs() - 3_600);
        let future = xsd_date_time(now_secs() + 3_600);
        for (credential, reason) in [
            (
                member20(&json!({"credentialStatus": revocable})),
                "revoked or suspended (revocation)",
            ),
            (
                member20(&json!({"credentialStatus": suspendable})),
                "revoked or suspended (suspension)",
            ),
            (member20(&json!({"validUntil": past})), "expired"),
            (member20(&json!({"validFrom": future})), "not yet valid"),
            (
                Dataspace::credential20(&ds.third, MEMBERSHIP, holder, &json!({})),
                "no MembershipCredential from a trusted issuer",
            ),
        ] {
            present(credential);
            let err = ds.refusal().await;
            assert!(
                err.contains(reason),
                "enveloped {enveloped}, {reason}: {err}"
            );
        }
    }
    // the inner presentation by a third party, the envelope by the holder
    let inner = ds.presentation20(&ds.third, &[member20(&json!({}))]);
    ds.answer(&[ds.envelope(&ds.holder, &inner)]);
    let err = ds.refusal().await;
    assert!(err.contains("holder is not the caller"), "{err}");
}

#[tokio::test]
async fn nothing_is_fetched_for_an_untrusted_issuer() {
    let ds = dataspace().await;
    let holder = ds.holder.did.as_str();
    let mut entry = ds.status_entry("5");
    entry["credentialStatus"]["statusListCredential"] = json!(format!("{}/status/9", ds.base));
    let untrusted = Dataspace::credential(&ds.third, MEMBERSHIP, holder, &entry);
    let trusted = Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &json!({}));
    ds.answer(&[ds.presentation(&ds.holder, &[untrusted, trusted])]);
    ds.forget_fetches();
    assert!(
        ds.verify(&ds.header()).await.is_ok(),
        "the trusted one counts"
    );
    let fetched = ds.fetched();
    assert!(
        !fetched.contains(&"/third/did.json".to_owned()),
        "{fetched:?}"
    );
    assert!(!fetched.contains(&"/status/9".to_owned()), "{fetched:?}");
    assert!(
        fetched.contains(&"/issuer/did.json".to_owned()),
        "{fetched:?}"
    );
}

/// R-48: `third` is trusted, but only for data permits
/// (`PORTITOR_TRUSTED_ISSUER_TYPES`), as EDC's `supportedtypes`.
fn limit_third_to_data_permits(ds: &mut Dataspace) {
    let trusted = vec![ds.issuer.did.clone(), ds.third.did.clone()];
    let limit = vec![format!("{}=DataPermitCredential", ds.third.did)];
    ds.verifier.trusted_issuers.clone_from(&trusted);
    ds.verifier.issuer_types = IssuerTypes::parse(&limit, &trusted).unwrap();
}

#[tokio::test]
async fn a_credential_from_an_issuer_not_trusted_for_its_type_is_skipped_unfetched() {
    let mut ds = dataspace().await;
    limit_third_to_data_permits(&mut ds);
    let holder = ds.holder.did.as_str();
    let mut entry = ds.status_entry("5");
    entry["credentialStatus"]["statusListCredential"] = json!(format!("{}/status/9", ds.base));
    let thirds_membership = Dataspace::credential(&ds.third, MEMBERSHIP, holder, &entry);
    let trusted = Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &json!({}));
    ds.answer(&[ds.presentation(&ds.holder, &[thirds_membership.clone(), trusted])]);
    ds.forget_fetches();
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 1, "only the issuer's membership");
    assert_eq!(caller.memberships.len(), 1);
    assert_eq!(caller.memberships[0].issuer, ds.issuer.did);
    let fetched = ds.fetched();
    assert!(
        !fetched.contains(&"/third/did.json".to_owned()),
        "{fetched:?}"
    );
    assert!(!fetched.contains(&"/status/9".to_owned()), "{fetched:?}");

    // alone, the third party's membership makes no member
    ds.answer(&[ds.presentation(&ds.holder, &[thirds_membership])]);
    assert!(ds.verify(&ds.header()).await.is_err());
}

#[tokio::test]
async fn a_limited_issuer_is_trusted_for_its_type_and_others_for_every_type() {
    let mut ds = dataspace().await;
    limit_third_to_data_permits(&mut ds);
    let holder = ds.holder.did.as_str();
    let membership = Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &json!({}));
    let thirds_permit =
        Dataspace::credential(&ds.third, "DataPermitCredential", holder, &json!({}));
    // the issuer the setting does not name keeps every type
    let issuers_permit =
        Dataspace::credential(&ds.issuer, "DataPermitCredential", holder, &json!({}));
    ds.answer(&[ds.presentation(&ds.holder, &[membership, thirds_permit, issuers_permit])]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    let mut permits = 0;
    for credential in &caller.credentials {
        if credential.has_type("DataPermitCredential") {
            permits += 1;
        }
    }
    assert_eq!(permits, 2, "both data permits are held");
}

#[tokio::test]
async fn a_membership_is_no_longer_trusted_once_its_issuer_is_limited() {
    let mut ds = dataspace().await;
    ds.answer_member(&json!({}));
    let caller = ds.verify(&ds.header()).await.unwrap();
    let membership = caller.memberships[0].clone();
    assert!(
        ds.verifier
            .check_membership_again(&membership)
            .await
            .is_ok()
    );
    // the issuer is now trusted for data permits only
    let trusted = vec![ds.issuer.did.clone()];
    let limit = vec![format!("{}=DataPermitCredential", ds.issuer.did)];
    ds.verifier.issuer_types = IssuerTypes::parse(&limit, &trusted).unwrap();
    let error = ds
        .verifier
        .check_membership_again(&membership)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no longer trusted"), "{error}");
}

#[tokio::test]
async fn answers_are_bounded() {
    let ds = dataspace().await;
    let holder = ds.holder.did.as_str();
    let member = Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &json!({}));
    let one = ds.presentation(&ds.holder, std::slice::from_ref(&member));

    ds.answer(&vec![one.clone(); MAX_PRESENTATIONS]);
    assert!(ds.verify(&ds.header()).await.is_ok(), "at the limit");
    ds.answer(&vec![one; MAX_PRESENTATIONS + 1]);
    let err = ds.refusal().await;
    assert!(err.contains("more than 8 presentations"), "{err}");

    ds.answer(&[ds.presentation(&ds.holder, &vec![member.clone(); MAX_CREDENTIALS])]);
    assert!(ds.verify(&ds.header()).await.is_ok(), "at the limit");
    ds.answer(&[ds.presentation(&ds.holder, &vec![member; MAX_CREDENTIALS + 1])]);
    let err = ds.refusal().await;
    assert!(err.contains("more than 16 credentials"), "{err}");

    ds.publish_status_list(&ds.issuer, &[]);
    let entries = |count: usize| {
        let entry = ds.entry("1", "BitstringStatusListEntry", "revocation", "5");
        json!({"credentialStatus": vec![entry; count]})
    };
    ds.answer_member(&entries(MAX_STATUS_ENTRIES));
    assert!(ds.verify(&ds.header()).await.is_ok(), "at the limit");
    ds.answer_member(&entries(MAX_STATUS_ENTRIES + 1));
    let err = ds.refusal().await;
    assert!(err.contains("more than 4 status entries"), "{err}");
}

#[tokio::test]
async fn a_slow_check_is_cut_off() {
    let mut ds = dataspace().await;
    ds.answer_member(&json!({}));
    ds.verifier.deadline = Duration::from_millis(200);
    ds.world.lock().unwrap().delay = Duration::from_secs(2);
    let err = ds.refusal().await;
    assert!(err.contains("took longer than 0.2 s"), "{err}");
}

#[tokio::test]
async fn our_identity_must_match_the_sts() {
    let ds = dataspace().await;
    // the STS signs as someone else
    let settings = AuthSettings {
        participant_id: ds.me_did.clone(),
        sts_url: String::new(),
        sts_allow_http: false,
        sts_client_id: String::new(),
        sts_client_secret: String::new(),
        trusted_issuers: Vec::new(),
        issuer_types: portitor_dcp::trust::IssuerTypes::default(),
        gaiax_compliance_issuers: Vec::new(),
        scopes: Vec::new(),
        allow_http: true,
        status_list_cache_seconds: 0,
        did_cache_seconds: 300,
    };
    let other = Party::new(&ds.base, "third");
    let mut verifier =
        DcpVerifier::new(&settings, Arc::new(LocalSts(other)), used_token_ids()).unwrap();
    verifier.start_tries = 1;
    assert!(
        verifier
            .check_own_identity()
            .await
            .unwrap_err()
            .contains("not signed as it")
    );
    // our DID document cannot be read
    let mut lost = settings.clone();
    lost.participant_id = format!("{}x", ds.me_did);
    let mut verifier = DcpVerifier::new(
        &lost,
        Arc::new(LocalSts(Party::new(&ds.base, "mex"))),
        used_token_ids(),
    )
    .unwrap();
    verifier.start_tries = 1;
    assert!(
        verifier
            .check_own_identity()
            .await
            .unwrap_err()
            .contains("cannot read")
    );
    // no Credential Service in it
    let mut doc = ds.world.lock().unwrap().documents["me"].clone();
    doc["service"] = json!([]);
    ds.world.lock().unwrap().documents.insert("me".into(), doc);
    let fresh = DcpVerifier::new(
        &settings,
        Arc::new(LocalSts(Party::new(&ds.base, "me"))),
        used_token_ids(),
    )
    .unwrap();
    assert!(
        fresh
            .check_own_identity()
            .await
            .unwrap_err()
            .contains("CredentialService")
    );
    assert!(format!("{fresh:?}").contains("DcpVerifier"));
}

#[test]
fn reads_bearer_headers() {
    assert_eq!(bearer(Some("Bearer abc")), Ok("abc"));
    assert_eq!(bearer(Some("bearer  abc ")), Ok("abc"));
    for bad in [
        None,
        Some(""),
        Some("abc"),
        Some("Bearer"),
        Some("Bearer  "),
        Some("Basic abc"),
    ] {
        assert!(bearer(bad).is_err(), "{bad:?}");
    }
}

/// ADR 0019: more scopes mean a second query with the caller's access
/// token, checked like the first; scopes already asked for need none.
#[tokio::test]
async fn more_credentials_are_asked_for_with_the_callers_access_token() {
    let ds = dataspace().await;
    let holder = ds.holder.did.as_str();
    let member = Dataspace::credential(&ds.issuer, MEMBERSHIP, holder, &json!({}));
    let processor = Dataspace::credential(
        &ds.issuer,
        "DataProcessorCredential",
        holder,
        &json!({"credentialSubject": {"id": holder, "processing": "eu"}}),
    );
    ds.answer(&[ds.presentation(&ds.holder, std::slice::from_ref(&member))]);
    let caller = ds.verify(&ds.header()).await.unwrap();

    // the configured scope only: the credentials of the first check
    let configured = vec![super::super::DEFAULT_SCOPE.to_owned()];
    let again = ds.verifier.credentials(&caller, &configured).await.unwrap();
    assert_eq!(again, caller.credentials);
    assert_eq!(ds.world.lock().unwrap().scopes.len(), 1);

    // one more scope: a new query for both, with the same access token
    let extra = vec!["org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read".to_owned()];
    ds.answer(&[ds.presentation(&ds.holder, &[member.clone(), processor])]);
    let more = ds.verifier.credentials(&caller, &extra).await.unwrap();
    assert_eq!(more.len(), 2);
    assert_eq!(more[1].subjects[0]["processing"], "eu");
    let (scopes, query) = {
        let mut world = ds.world.lock().unwrap();
        (world.scopes.pop().unwrap(), world.queries.pop().unwrap())
    };
    assert_eq!(scopes, [configured[0].clone(), extra[0].clone()]);
    let claims = unverified_claims(query.strip_prefix("Bearer ").unwrap()).unwrap();
    assert_eq!(claims.token.as_deref(), Some("access-1"));

    // a revoked credential in the second answer makes its presentation
    // invalid (DCP 5.4.3, step 8): no credentials, so constraints fail
    ds.publish_status_list(&ds.issuer, &[3]);
    let revoked = Dataspace::credential(
        &ds.issuer,
        "DataProcessorCredential",
        holder,
        &ds.status_entry("3"),
    );
    ds.answer(&[ds.presentation(&ds.holder, &[member, revoked])]);
    let err = ds.verifier.credentials(&caller, &extra).await.unwrap_err();
    assert!(err.contains("revoked"), "{err}");

    // a caller without an access token cannot be asked again
    let anonymous = Caller::new(holder);
    let err = ds
        .verifier
        .credentials(&anonymous, &extra)
        .await
        .unwrap_err();
    assert!(err.contains("no access token"), "{err}");
}

/// A `gx:LegalPerson` credential (VC 2.0) by `issuer`, as in the 0.8
/// spike, with `name` as its legal name. Returns the JWT and its
/// `digestSRI`.
fn legal_person(issuer: &Party, name: &str) -> (String, String) {
    legal_person_with(issuer, name, &json!({}))
}

/// [`legal_person`] with `extra` fields.
fn legal_person_with(issuer: &Party, name: &str, extra: &Value) -> (String, String) {
    let vc = legal_person_payload(&issuer.did, name, extra);
    let digest = portitor_dcp::gaiax::digest_sri(vc.as_object().unwrap());
    let jwt = issuer.key.sign_json(&issuer.kid(), "vc+jwt", &vc).unwrap();
    (jwt, digest)
}

/// The payload of a [`legal_person`] credential by `issuer`.
fn legal_person_payload(issuer: &str, name: &str, extra: &Value) -> Value {
    let mut vc = json!({
        "@context": [VC2_CONTEXT, GAIAX_CONTEXT,
            {"vcard": "http://www.w3.org/2006/vcard/ns#", "schema": "https://schema.org/"}],
        "id": "https://participant.test/credentials/legal-person.json",
        "type": ["VerifiableCredential", "gx:LegalPerson"],
        "issuer": issuer,
        "validFrom": xsd_date_time(now_secs() - 86_400),
        "validUntil": xsd_date_time(now_secs() + 90 * 86_400),
        "credentialSubject": {
            "id": "https://participant.test/credentials/legal-person.json#cs",
            "schema:name": name,
            "gx:legalAddress": {"type": "gx:Address", "gx:countryCode": "LU"}
        }
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        vc[key] = value.clone();
    }
    vc
}

/// The payload of a `gx:ParticipantComplianceCredential` by `issuer`
/// naming `digests` (0.8 spike, finding 4), with `extra` fields.
fn compliance_payload(issuer: &str, digests: &[&str], extra: &Value) -> Value {
    let id = "https://compliance.test/credentials/c-1";
    let mut named = Vec::new();
    for digest in digests {
        named.push(
            json!({"id": "https://participant.test/credentials/legal-person.json",
            "gx:credentialType": "gx:LegalPerson", "digestSRI": digest}),
        );
    }
    let mut vc = json!({
        "@context": [VC2_CONTEXT, GAIAX_CONTEXT],
        "id": id,
        "type": ["VerifiableCredential", "gx:ParticipantComplianceCredential"],
        "issuer": issuer,
        "validFrom": xsd_date_time(now_secs() - 3_600),
        "validUntil": xsd_date_time(now_secs() + 90 * 86_400),
        "credentialSubject": {
            "id": format!("{id}#cs"),
            "gx:labelLevel": "SC",
            "gx:engineVersion": "2.16.0",
            "gx:rulesVersion": "CD4.0.0",
            "gx:compliantCredentials": named
        }
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        vc[key] = value.clone();
    }
    vc
}

impl Dataspace {
    /// A compliance credential by our trusted compliance service, naming
    /// `digests`.
    fn compliance_credential(&self, digests: &[&str], extra: &Value) -> String {
        self.compliance
            .sign(&compliance_payload(&self.compliance.did, digests, extra))
    }

    /// A trusted membership credential of the holder (VC 2.0).
    fn member20(&self) -> String {
        Dataspace::credential20(&self.issuer, MEMBERSHIP, &self.holder.did, &json!({}))
    }

    /// The Credential Service answers with one VC 2.0 presentation by the
    /// holder holding `credentials`.
    fn answer20(&self, credentials: &[String]) {
        self.answer(&[self.presentation20(&self.holder, credentials)]);
    }
}

/// The policy types of a provider that may use the compliance
/// credential, and its scope.
fn compliance_types() -> portitor_dsp::policy::CredentialTypes {
    let mut types = portitor_dsp::policy::CredentialTypes::default();
    types.add(
        "ParticipantComplianceCredential",
        "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read",
    );
    types
}

/// An offer whose permission needs `gx:labelLevel` `label`.
fn label_offer(label: &str) -> portitor_dsp::Offer {
    serde_json::from_value(json!({"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [{
            "leftOperand": "ParticipantComplianceCredential.gx:labelLevel",
            "operator": "eq", "rightOperand": label}]}]}))
    .unwrap()
}

/// ADR 0022: a compliance credential from a trusted service, with the
/// legal person credential it names, is accepted without holder binding,
/// and a policy on its label holds.
#[tokio::test]
async fn a_gaiax_compliant_caller_is_accepted_and_its_label_is_read() {
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    let compliance = ds.compliance_credential(&[&digest], &json!({}));
    ds.answer20(&[ds.member20(), person, compliance]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 3);
    let compliance = &caller.credentials[0];
    assert!(compliance.has_type("gx:ParticipantComplianceCredential"));
    assert!(compliance.has_type("ParticipantComplianceCredential"));
    assert!(caller.credentials[1].has_type("LegalPerson"));
    // the policy evaluator reads the label (ADR 0019 left operand)
    let held = portitor_dsp::policy::CallerCredentials {
        understood: compliance_types(),
        held: caller.credentials.clone(),
    };
    let now = xsd_date_time(now_secs());
    assert_eq!(
        portitor_dsp::policy::permissions_hold(&label_offer("SC"), &now, &held),
        Ok(())
    );
    assert!(portitor_dsp::policy::permissions_hold(&label_offer("L1"), &now, &held).is_err());
}

#[tokio::test]
async fn a_compliance_credential_that_fails_refuses_the_caller() {
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    // the legal person credential changed after the compliance check
    let (tampered, _) = legal_person(&ds.holder, "Someone Else S.A.");
    let named = ds.compliance_credential(&[&digest], &json!({}));
    // a legal person credential with the listed digest, but issued by
    // someone else than the caller
    let (by_third, third_digest) = legal_person(&ds.third, "Portitor Test S.A.");
    let names_third = ds.compliance_credential(&[&third_digest], &json!({}));
    // signed by someone else than the service it names as issuer
    let forged = ds.holder.key.sign_json(
        &ds.compliance.kid(),
        "vc+jwt",
        &compliance_payload(&ds.compliance.did, &[&digest], &json!({})),
    );
    let past = json!({"validUntil": xsd_date_time(now_secs() - 3_600)});
    let expired = ds.compliance_credential(&[&digest], &past);
    // a compliance credential without an end never expires
    let mut endless = compliance_payload(&ds.compliance.did, &[&digest], &json!({}));
    endless.as_object_mut().unwrap().remove("validUntil");
    let endless = ds.compliance.sign(&endless);
    let cases = [
        (vec![tampered, named.clone()], "names no gx:LegalPerson"),
        (vec![named.clone()], "names no gx:LegalPerson"),
        (vec![by_third, names_third], "is not the caller"),
        (vec![person.clone(), forged.unwrap()], "credential from"),
        (vec![person.clone(), expired], "expired"),
        (vec![person, endless], "no validUntil"),
    ];
    for (gaiax, reason) in cases {
        let mut credentials = vec![ds.member20()];
        credentials.extend(gaiax);
        ds.answer20(&credentials);
        ds.forget_fetches();
        let err = ds.refusal().await;
        assert!(
            err.contains("gx:ParticipantComplianceCredential") || err.contains("gx:LegalPerson"),
            "{reason}: {err}"
        );
        assert!(err.contains(reason), "{reason}: {err}");
    }
}

/// More ADR 0022 refusals, each naming the credential that fails: the
/// compliance credential not yet valid, the legal person credential
/// expired or signed by a key that is not the caller's, and a second
/// compliance credential that names no credential of the presentation.
#[tokio::test]
async fn gaiax_refusals_name_the_failing_credential() {
    const COMPLIANCE: &str = "a gx:ParticipantComplianceCredential credential fails";
    const PERSON: &str = "a gx:LegalPerson credential fails";
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    let named = ds.compliance_credential(&[&digest], &json!({}));
    let future = json!({"validFrom": xsd_date_time(now_secs() + 3_600)});
    let not_yet_valid = ds.compliance_credential(&[&digest], &future);
    let past = json!({"validUntil": xsd_date_time(now_secs() - 3_600)});
    let (expired_person, expired_digest) =
        legal_person_with(&ds.holder, "Portitor Test S.A.", &past);
    let names_expired = ds.compliance_credential(&[&expired_digest], &json!({}));
    // issued as the caller, signed with the third party's key, which is
    // not in the caller's DID document
    let payload = legal_person_payload(&ds.holder.did, "Portitor Test S.A.", &json!({}));
    let foreign_digest = portitor_dcp::gaiax::digest_sri(payload.as_object().unwrap());
    let foreign_key = ds
        .third
        .key
        .sign_json(&ds.third.kid(), "vc+jwt", &payload)
        .unwrap();
    let names_foreign = ds.compliance_credential(&[&foreign_digest], &json!({}));
    let names_other = ds.compliance_credential(&["sha256-00"], &json!({}));
    let cases = [
        (
            vec![person.clone(), not_yet_valid],
            COMPLIANCE,
            "not yet valid",
        ),
        (vec![expired_person, names_expired], PERSON, "expired"),
        (vec![foreign_key, names_foreign], PERSON, "kid not found"),
        (
            vec![person, named, names_other],
            COMPLIANCE,
            "names no gx:LegalPerson",
        ),
    ];
    for (gaiax, failing, reason) in cases {
        let mut credentials = vec![ds.member20()];
        credentials.extend(gaiax);
        ds.answer20(&credentials);
        let err = ds.refusal().await;
        assert!(err.contains(failing), "{reason}: {err}");
        assert!(err.contains(reason), "{reason}: {err}");
    }
}

/// ADR 0022: a compliance credential from a service we do not trust is
/// skipped, like a credential from an untrusted issuer, and the legal
/// person credential it names with it. A member of another clearing house
/// is still let in for offers that need only a membership.
#[tokio::test]
async fn a_compliance_credential_from_an_untrusted_service_is_skipped() {
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    let untrusted = ds
        .third
        .key
        .sign_json(
            &ds.third.kid(),
            "vc+jwt",
            &compliance_payload(&ds.third.did, &[&digest], &json!({})),
        )
        .unwrap();
    ds.answer20(&[ds.member20(), person, untrusted]);
    ds.forget_fetches();
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 1);
    assert!(caller.credentials[0].has_type(MEMBERSHIP));
    // nothing is fetched for an untrusted compliance service
    assert!(!ds.fetched().contains(&"/third/did.json".to_owned()));
}

/// A legal person credential that no trusted compliance credential names
/// is skipped, like one from an untrusted issuer; every other credential
/// still needs the holder binding.
#[tokio::test]
async fn other_credentials_keep_the_holder_binding() {
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    ds.answer20(&[ds.member20(), person.clone()]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 1, "the legal person is skipped");
    // a membership about someone else, next to a valid Gaia-X pair
    let compliance = ds.compliance_credential(&[&digest], &json!({}));
    let not_ours = Dataspace::credential20(&ds.issuer, MEMBERSHIP, &ds.third.did, &json!({}));
    ds.answer20(&[ds.member20(), not_ours, person, compliance]);
    let err = ds.refusal().await;
    assert!(err.contains("not about the caller"), "{err}");
}

/// ADR 0022: the status of the caller's own legal person credential is
/// not fetched; the caller controls it.
#[tokio::test]
async fn the_status_of_the_callers_legal_person_is_not_fetched() {
    let ds = dataspace().await;
    ds.publish_status_list(&ds.holder, &[7]);
    let revoked = ds.status_entry("7");
    let (person, digest) = legal_person_with(&ds.holder, "Portitor Test S.A.", &revoked);
    let compliance = ds.compliance_credential(&[&digest], &json!({}));
    ds.answer20(&[ds.member20(), person, compliance]);
    ds.forget_fetches();
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 3);
    assert!(!ds.fetched().iter().any(|path| path.starts_with("/status/")));
}

/// ADR 0022: a compliance credential counts only as a compliance
/// credential. Typed `MembershipCredential` too, it neither lets the
/// caller in nor meets a membership constraint.
#[tokio::test]
async fn a_compliance_credential_is_held_only_as_one() {
    let ds = dataspace().await;
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    let mut payload = compliance_payload(&ds.compliance.did, &[&digest], &json!({}));
    payload["type"] = json!([
        "VerifiableCredential",
        "gx:ParticipantComplianceCredential",
        MEMBERSHIP
    ]);
    payload["credentialSubject"]["role"] = json!("provider");
    let also_member = ds.compliance.sign(&payload);
    // without a real membership, the caller is not let in
    ds.answer20(&[person.clone(), also_member.clone()]);
    let err = ds.refusal().await;
    assert!(err.contains("no MembershipCredential"), "{err}");
    // with one, the compliance credential's role is not read as a member's
    ds.answer20(&[ds.member20(), person, also_member]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(
        caller.credentials[0].types,
        [
            "gx:ParticipantComplianceCredential",
            "ParticipantComplianceCredential"
        ]
    );
    assert_eq!(
        caller.credentials[1].types,
        ["gx:LegalPerson", "LegalPerson"]
    );
    let offer: portitor_dsp::Offer = serde_json::from_value(json!({"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [{
            "leftOperand": "MembershipCredential.role", "operator": "eq",
            "rightOperand": "provider"}]}]}))
    .unwrap();
    let held = portitor_dsp::policy::CallerCredentials {
        understood: compliance_types(),
        held: caller.credentials.clone(),
    };
    let now = xsd_date_time(now_secs());
    assert!(portitor_dsp::policy::permissions_hold(&offer, &now, &held).is_err());
}

/// A `gx:LegalPerson` from a trusted dataspace issuer is a dataspace
/// credential, checked as before 0.8: accepted when it is about the
/// caller, with no compliance credential, and refused when it is not.
#[tokio::test]
async fn a_trusted_issuers_legal_person_is_a_dataspace_credential() {
    let ds = dataspace().await;
    let about_caller =
        Dataspace::credential20(&ds.issuer, "gx:LegalPerson", &ds.holder.did, &json!({}));
    ds.answer20(&[ds.member20(), about_caller]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    assert_eq!(caller.credentials.len(), 2);
    assert!(caller.credentials[1].has_type("gx:LegalPerson"));
    let about_third =
        Dataspace::credential20(&ds.issuer, "gx:LegalPerson", &ds.third.did, &json!({}));
    ds.answer20(&[ds.member20(), about_third]);
    let err = ds.refusal().await;
    assert!(err.contains("not about the caller"), "{err}");
}

/// ADR 0022: the consumer's token and the provider's query carry the same
/// scopes for a compliance policy, the legal person scope included.
#[tokio::test]
async fn consumer_and_provider_derive_the_same_scopes() {
    /// An STS that keeps the scopes it is asked to grant.
    struct RecordingSts(Mutex<Vec<Vec<String>>>);
    impl SecureTokenService for RecordingSts {
        fn token<'a>(&'a self, request: TokenRequest<'a>) -> BoxFuture<'a, Result<String, String>> {
            self.0.lock().unwrap().push(request.scopes.to_vec());
            Box::pin(async { Ok("token".to_owned()) })
        }
    }

    let ds = dataspace().await;
    let offer = label_offer("SC");
    let needed = portitor_dsp::policy::credential_scopes(&offer, &compliance_types());
    // the consumer asks its STS for a token
    let sts = Arc::new(RecordingSts(Mutex::new(Vec::new())));
    let configured = vec![super::super::DEFAULT_SCOPE.to_owned()];
    let prover = super::super::sts::StsProver::new(
        Arc::clone(&sts) as Arc<dyn SecureTokenService>,
        configured,
    );
    let audience = ds.me_did.clone();
    super::super::Prover::authorization(&prover, &audience, &needed)
        .await
        .unwrap();
    let granted = sts.0.lock().unwrap().pop().unwrap();
    // the provider asks the consumer's Credential Service
    let (person, digest) = legal_person(&ds.holder, "Portitor Test S.A.");
    let compliance = ds.compliance_credential(&[&digest], &json!({}));
    ds.answer20(&[ds.member20()]);
    let caller = ds.verify(&ds.header()).await.unwrap();
    ds.answer20(&[ds.member20(), person, compliance]);
    let held = ds.verifier.credentials(&caller, &needed).await.unwrap();
    assert_eq!(held.len(), 3);
    let queried = ds.world.lock().unwrap().scopes.pop().unwrap();
    assert_eq!(granted, queried);
    assert_eq!(
        queried,
        [
            super::super::DEFAULT_SCOPE,
            "org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read",
            "org.eclipse.dspace.dcp.vc.type:LegalPerson:read",
        ]
    );
}

/// Our STS for the readiness check: signs as `party`, counts the tokens
/// it gives, and fails while `down` is set.
struct CountingSts {
    party: Party,
    tokens: std::sync::atomic::AtomicUsize,
    down: std::sync::atomic::AtomicBool,
}

impl SecureTokenService for CountingSts {
    fn token<'a>(&'a self, request: TokenRequest<'a>) -> BoxFuture<'a, Result<String, String>> {
        use std::sync::atomic::Ordering;
        let answer = if self.down.load(Ordering::SeqCst) {
            Err("STS: connection refused".to_owned())
        } else {
            self.tokens.fetch_add(1, Ordering::SeqCst);
            let jti = portitor_runtime::system::new_uuid();
            Ok(self
                .party
                .id_token(request.audience, request.access_token, &jti))
        };
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn readiness_checks_our_identity_in_the_background() {
    use crate::health::OwnIdentityCheck;
    use portitor_runtime::health::Check as _;
    use std::sync::atomic::Ordering;
    let ds = dataspace().await;
    // a participant of our own, with its DID document served
    let mine = Party::new(&ds.base, "mine");
    let doc = mine.document(&ds.base, true);
    ds.world
        .lock()
        .unwrap()
        .documents
        .insert("mine".into(), doc);
    let settings = AuthSettings {
        participant_id: mine.did.clone(),
        sts_url: String::new(),
        sts_allow_http: false,
        sts_client_id: String::new(),
        sts_client_secret: String::new(),
        trusted_issuers: Vec::new(),
        issuer_types: portitor_dcp::trust::IssuerTypes::default(),
        gaiax_compliance_issuers: Vec::new(),
        scopes: Vec::new(),
        allow_http: true,
        status_list_cache_seconds: 0,
        did_cache_seconds: 300,
    };
    let sts = Arc::new(CountingSts {
        party: mine,
        tokens: std::sync::atomic::AtomicUsize::new(0),
        down: std::sync::atomic::AtomicBool::new(false),
    });
    let check = OwnIdentityCheck::with_sts(&settings, Arc::clone(&sts) as _).unwrap();
    // not ready before the first run
    assert!(check.check().await.is_err());
    check.refresh().await;
    assert_eq!(check.check().await, Ok(()));
    // probes only read the result: one token in all
    assert_eq!(check.check().await, Ok(()));
    assert_eq!(check.check().await, Ok(()));
    assert_eq!(sts.tokens.load(Ordering::SeqCst), 1);

    // our STS is down: a check that never passed fails, and says what
    // failed without any secret
    sts.down.store(true, Ordering::SeqCst);
    let fresh = OwnIdentityCheck::with_sts(&settings, Arc::clone(&sts) as _).unwrap();
    fresh.refresh().await;
    let err = fresh.check().await.unwrap_err();
    assert!(err.contains("STS"), "{err}");
    // and so does the next probe, without asking again
    assert_eq!(fresh.check().await.unwrap_err(), err);

    // our DID document is no longer served
    sts.down.store(false, Ordering::SeqCst);
    ds.world.lock().unwrap().documents.remove("mine");
    let fresh = OwnIdentityCheck::with_sts(&settings, Arc::clone(&sts) as _).unwrap();
    fresh.refresh().await;
    let err = fresh.check().await.unwrap_err();
    assert!(err.contains("cannot read the DID document"), "{err}");
}

/// How far ahead of the system clock a test has moved the verifier's
/// clock, in seconds.
type Offset = Arc<std::sync::atomic::AtomicU64>;

impl Dataspace {
    /// The verifier keeps status lists for `seconds`. Returns the offset
    /// of its clock, which the test moves forward ([`move_clock`]).
    fn cache_status_lists(&mut self, seconds: u64) -> Offset {
        let offset: Offset = Arc::default();
        let ahead = Arc::clone(&offset);
        self.verifier.status_lists = StatusListCache::new(seconds);
        // A fixed start, so a tick of the real clock during the test does
        // not move the verifier's time past a cache boundary.
        let start = now_secs();
        self.verifier.clock =
            Arc::new(move || start + ahead.load(std::sync::atomic::Ordering::SeqCst));
        offset
    }

    /// How many status lists were fetched since
    /// [`Dataspace::forget_fetches`].
    fn status_fetches(&self) -> usize {
        let mut count = 0;
        for path in self.fetched() {
            if path.starts_with("/status/") {
                count += 1;
            }
        }
        count
    }
}

/// Moves the verifier's clock to `seconds` after the start of the test.
fn move_clock(offset: &Offset, seconds: u64) {
    offset.store(seconds, std::sync::atomic::Ordering::SeqCst);
}

#[tokio::test]
async fn at_zero_every_check_fetches_the_status_list() {
    let mut ds = dataspace().await;
    let _offset = ds.cache_status_lists(0);
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[]);
    for _ in 0..3 {
        assert!(ds.verify(&ds.header()).await.is_ok());
    }
    assert_eq!(ds.status_fetches(), 3);
}

#[tokio::test]
async fn at_300_seconds_a_status_list_is_fetched_again_only_when_stale() {
    let mut ds = dataspace().await;
    let offset = ds.cache_status_lists(300);
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[]);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 1);
    // within 300 s: from the cache
    move_clock(&offset, 299);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 1);
    // after 300 s: fetched again, and kept again
    move_clock(&offset, 300);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 2);
    move_clock(&offset, 301);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 2);
}

#[tokio::test]
async fn a_status_list_that_ends_sooner_is_fetched_again_at_its_end() {
    let mut ds = dataspace().await;
    let offset = ds.cache_status_lists(300);
    ds.answer_member(&ds.status_entry("5"));
    let ends = json!({"validUntil": xsd_date_time(now_secs() + 100)});
    ds.publish_list("1", &ds.issuer, "revocation", &[], &ends);
    assert!(ds.verify(&ds.header()).await.is_ok());
    move_clock(&offset, 99);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 1);
    // at its end: fetched again (still accepted, within the leeway)
    move_clock(&offset, 100);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 2);
}

#[tokio::test]
async fn a_status_list_that_fails_is_not_cached() {
    let mut ds = dataspace().await;
    let _offset = ds.cache_status_lists(300);
    ds.answer_member(&ds.status_entry("5"));
    // not found
    let err = ds.refusal().await;
    assert!(err.contains("HTTP 404"), "{err}");
    // signed by another issuer
    ds.publish_status_list(&ds.third, &[]);
    let err = ds.refusal().await;
    assert!(err.contains("has another issuer"), "{err}");
    // each failure was fetched, and the good list is fetched at once
    ds.publish_status_list(&ds.issuer, &[]);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 3);
    // and then kept
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.status_fetches(), 3);
}

#[tokio::test]
async fn a_revocation_is_seen_once_the_cached_list_is_stale() {
    let mut ds = dataspace().await;
    let offset = ds.cache_status_lists(300);
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[]);
    assert!(ds.verify(&ds.header()).await.is_ok());
    // the issuer revokes the credential: not seen while the list is cached
    ds.publish_status_list(&ds.issuer, &[5]);
    move_clock(&offset, 299);
    assert!(ds.verify(&ds.header()).await.is_ok());
    move_clock(&offset, 300);
    let err = ds.refusal().await;
    assert!(err.contains("revoked or suspended (revocation)"), "{err}");
}

#[tokio::test]
async fn a_stale_list_that_cannot_be_fetched_again_refuses_the_caller() {
    let mut ds = dataspace().await;
    let offset = ds.cache_status_lists(300);
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[]);
    assert!(ds.verify(&ds.header()).await.is_ok());
    // the list is gone: the cached copy serves while it is fresh
    ds.world.lock().unwrap().status_lists.clear();
    move_clock(&offset, 299);
    assert!(ds.verify(&ds.header()).await.is_ok());
    // once stale, the failed fetch refuses the caller: a stale copy is
    // never used instead (fail closed)
    move_clock(&offset, 300);
    let err = ds.refusal().await;
    assert!(err.contains("HTTP 404"), "{err}");
}

#[tokio::test]
async fn a_cached_status_list_serves_only_its_own_issuer() {
    let mut ds = dataspace().await;
    let _offset = ds.cache_status_lists(300);
    ds.verifier.trusted_issuers.push(ds.third.did.clone());
    ds.answer_member(&ds.status_entry("5"));
    ds.publish_status_list(&ds.issuer, &[]);
    assert!(ds.verify(&ds.header()).await.is_ok());
    // another trusted issuer's credential names the same, cached, URL
    let credential =
        Dataspace::credential(&ds.third, MEMBERSHIP, &ds.holder.did, &ds.status_entry("5"));
    ds.answer(&[ds.presentation(&ds.holder, &[credential])]);
    let err = ds.refusal().await;
    assert!(err.contains("has another issuer"), "{err}");
    assert_eq!(ds.status_fetches(), 1);
}

impl Dataspace {
    /// How many times the DID document of the party `name` was fetched
    /// since [`Dataspace::forget_fetches`].
    fn did_fetches(&self, name: &str) -> usize {
        let path = format!("/{name}/did.json");
        self.fetched().iter().filter(|got| **got == path).count()
    }

    /// The holder changes its key: its DID document now lists a new key
    /// only, under the same key id.
    fn replace_the_holders_key(&self) {
        let with_a_new_key = Party::new(&self.base, "holder").document(&self.base, true);
        let mut world = self.world.lock().unwrap();
        world.documents.insert("holder".into(), with_a_new_key);
    }
}

/// `PORTITOR_DID_CACHE_SECONDS=0` reaches the verifier: every check
/// fetches the caller's and the issuer's DID document again.
#[tokio::test]
async fn at_zero_every_check_fetches_the_did_documents() {
    let ds = dataspace_caching_did_documents(0).await;
    ds.answer_member(&json!({}));
    assert!(ds.verify(&ds.header()).await.is_ok());
    let (holder_first, issuer_first) = (ds.did_fetches("holder"), ds.did_fetches("issuer"));
    assert!(holder_first >= 1 && issuer_first >= 1);
    assert!(ds.verify(&ds.header()).await.is_ok());
    assert_eq!(ds.did_fetches("holder"), 2 * holder_first);
    assert_eq!(ds.did_fetches("issuer"), 2 * issuer_first);
}

/// The default, 300 seconds: the documents are fetched once and reused
/// by the checks that follow.
#[tokio::test]
async fn at_300_seconds_the_did_documents_are_fetched_once() {
    let ds = dataspace_caching_did_documents(300).await;
    ds.answer_member(&json!({}));
    for _ in 0..3 {
        assert!(ds.verify(&ds.header()).await.is_ok());
    }
    assert_eq!(ds.did_fetches("holder"), 1);
    assert_eq!(ds.did_fetches("issuer"), 1);
}

/// What the setting trades (R-41): while a DID document is kept, a key
/// its owner removed from it is still accepted. Without a cache the old
/// key is refused at the next check.
#[tokio::test]
async fn a_removed_key_is_refused_at_once_only_without_a_did_cache() {
    for (did_cache_seconds, old_key_still_accepted) in [(0, false), (300, true)] {
        let ds = dataspace_caching_did_documents(did_cache_seconds).await;
        ds.answer_member(&json!({}));
        assert!(ds.verify(&ds.header()).await.is_ok());
        ds.replace_the_holders_key();
        // the header is signed with the key that is no longer listed
        let accepted = ds.verify(&ds.header()).await.is_ok();
        assert_eq!(
            accepted, old_key_still_accepted,
            "{did_cache_seconds} seconds"
        );
    }
}

/// DCP v1.0.1, section 5.4.3, step 4: the DID of the credential's
/// verification method must be the credential's issuer. The trusted
/// issuer's DID document lists a key of another DID (`third`) under
/// `assertionMethod`; a credential in the issuer's name, signed with that
/// key, refuses the caller. It is a refusal, not a temporary failure.
#[tokio::test]
async fn a_credential_signed_with_another_dids_key_refuses_the_caller() {
    let ds = dataspace().await;
    {
        let mut world = ds.world.lock().unwrap();
        let thirds_method = world.documents["third"]["verificationMethod"][0].clone();
        let issuers = world.documents.get_mut("issuer").unwrap();
        issuers["assertionMethod"]
            .as_array_mut()
            .unwrap()
            .push(json!(ds.third.kid()));
        issuers["verificationMethod"]
            .as_array_mut()
            .unwrap()
            .push(thirds_method);
    }
    // a membership credential in the issuer's name, under the third
    // party's key id and signature
    let now = now_secs();
    let in_the_issuers_name = json!({
        "iss": ds.issuer.did, "sub": ds.holder.did, "jti": "j",
        "nbf": now - 10, "exp": now + 300,
        "vc": {
            "@context": [VC1_CONTEXT],
            "id": "urn:uuid:c-1",
            "type": ["VerifiableCredential", MEMBERSHIP],
            "issuer": ds.issuer.did,
            "issuanceDate": "2026-01-01T00:00:00Z",
            "credentialSubject": {"id": ds.holder.did}
        }
    });
    let forged = ds.third.sign(&in_the_issuers_name);
    ds.answer(&[ds.presentation(&ds.holder, &[forged])]);
    let error = ds.verify_error().await;
    assert!(matches!(error, VerifyError::NotAMember { .. }), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("the signing key belongs to another DID than the issuer"),
        "{error}"
    );

    // the same document, and a credential the issuer signed itself
    ds.answer_member(&json!({}));
    assert!(ds.verify(&ds.header()).await.is_ok());
}
