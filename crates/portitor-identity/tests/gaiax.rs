// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Gaia-X onboarding (ADR 0021) against a fake compliance service: an
//! axum server in this process, reached over HTTP through the real
//! adapter. It checks the presentation as the spike's service did (the
//! `vp+jwt` shape, our `kid`, our signature) and answers a compliance
//! credential signed by its own test key, with the digests of what it
//! received, or a wrong one when told to.
#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use base64::Engine as _;
use common::*;
use portitor_dcp::credential::{Credential, Presentation};
use portitor_dcp::gaiax::digest_sri_of;
use portitor_dcp::{DidDocument, Jws, Purpose, SigningKey};
use portitor_identity::config::{Config, GaiaxConfig};
use portitor_identity::gaiax::{HttpGaiaxCompliance, renewal_due, run_now};
use portitor_identity::store::{RunOutcome, SqliteStore};
use portitor_runtime::system::{new_uuid, now_secs, xsd_date_time};
use serde_json::{Map, Value, json};
use tower::ServiceExt as _;

const KEY: &str = include_str!("fixtures/test-only-identity-key.pem");
const CHAIN: &str = include_str!("fixtures/test-only-identity-chain.pem");
const OTHER_CERT: &str = include_str!("fixtures/test-only-other-cert.pem");
const COMPLIANCE: &str = "did:web:compliance.example";
const NOTARY: &str = "did:web:notary.example";
const GX: &str = "https://w3id.org/gaia-x/development#";
const BASE: &str = "https://holder.example/credentials/gaiax";

/// The fake compliance service.
struct FakeService {
    party: Party,
    /// The DID document of the holder, to check the presentation.
    holder: Mutex<Option<DidDocument>>,
    /// Answer with a wrong digest.
    wrong_digest: AtomicBool,
    /// The `vcid` of each request.
    asked: Mutex<Vec<String>>,
}

#[derive(serde::Deserialize)]
struct Asked {
    vcid: String,
}

fn payload_of(jwt: &str) -> Map<String, Value> {
    let middle = jwt.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(middle)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn header_of(jwt: &str) -> Value {
    let first = jwt.split('.').next().unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(first)
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// `POST /api/credential-offers/standard-compliance?vcid=...`
async fn standard_compliance(
    State(service): State<Arc<FakeService>>,
    Query(asked): Query<Asked>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    service.asked.lock().unwrap().push(asked.vcid.clone());
    if headers.get(header::CONTENT_TYPE).unwrap() != "application/vp+jwt" {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, String::new());
    }
    // the presentation: our shape, signed by the holder's key named by kid
    let holder = service.holder.lock().unwrap().clone().unwrap();
    let vp_header = header_of(&body);
    assert_eq!(vp_header["typ"], "vp+jwt");
    assert_eq!(vp_header["cty"], "vp");
    assert_eq!(vp_header["iss"], HOLDER);
    let jws = Jws::parse(&body).unwrap();
    let key = holder.key_for(jws.kid(), Purpose::AssertionMethod).unwrap();
    assert_eq!(Some(key.id.as_str()), jws.kid(), "kid is the full id");
    jws.verify(&key.public_key_jwk).unwrap();
    let vp = payload_of(&body);
    assert_eq!(vp["type"], "VerifiablePresentation");
    assert_eq!(vp["issuer"], HOLDER);
    let mut listed = Vec::new();
    for envelope in vp["verifiableCredential"].as_array().unwrap() {
        assert_eq!(envelope["type"], "EnvelopedVerifiableCredential");
        let jwt = envelope["id"]
            .as_str()
            .unwrap()
            .strip_prefix("data:application/vc+jwt,")
            .unwrap();
        let credential = Credential::parse(jwt).unwrap();
        let mut digest = digest_sri_of(&credential).unwrap();
        if service.wrong_digest.load(Ordering::SeqCst) {
            digest = format!("sha256-{}", "0".repeat(64));
        }
        listed.push(json!({
            "type": "gx:CompliantCredential",
            "id": credential.id,
            "gx:credentialType": credential.types[1],
            "digestSRI": digest,
        }));
    }
    let now = now_secs();
    let answer = json!({
        "@context": ["https://www.w3.org/ns/credentials/v2", GX],
        "id": asked.vcid,
        "type": ["VerifiableCredential", "gx:ParticipantComplianceCredential"],
        "issuer": COMPLIANCE,
        "validFrom": xsd_date_time(now),
        "validUntil": xsd_date_time(now + 90 * 24 * 3600),
        "credentialSubject": {
            "id": format!("{}#cs", asked.vcid),
            "gx:labelLevel": "SC",
            "gx:compliantCredentials": listed
        }
    });
    let mut headers = Map::new();
    headers.insert("typ".to_owned(), Value::from("vc+jwt"));
    headers.insert("cty".to_owned(), Value::from("vc"));
    headers.insert("iss".to_owned(), Value::from(COMPLIANCE));
    headers.insert("kid".to_owned(), Value::from(service.party.kid()));
    let jwt = service
        .party
        .key
        .sign_with_headers(&headers, &answer)
        .unwrap();
    (StatusCode::CREATED, jwt)
}

/// A temporary folder with the chain, the participant file and the
/// registration number credential.
struct Files {
    dir: PathBuf,
}

impl Files {
    fn new(notary: &Party) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "portitor-identity-gaiax-{}",
            new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chain.pem"), CHAIN).unwrap();
        let registration_id = format!("{BASE}/registration-number");
        let registration = json!({
            "@context": ["https://www.w3.org/ns/credentials/v2", GX],
            "id": registration_id,
            "type": ["VerifiableCredential", "gx:VatID"],
            "issuer": NOTARY,
            "validFrom": "2026-11-15T00:00:00Z",
            "credentialSubject": {
                "id": format!("{registration_id}#cs"),
                "type": "gx:VatID",
                "gx:vatID": "BE0762747721",
                "gx:countryCode": "BE"
            }
        });
        let jwt = notary
            .key
            .sign_json(&notary.kid(), "vc+jwt", &registration)
            .unwrap();
        std::fs::write(dir.join("registration-number.jwt"), format!("{jwt}\n")).unwrap();
        let address = json!({"type": "gx:Address", "gx:countryCode": "BE",
            "vcard:street-address": "1 Rue Test", "vcard:locality": "Bruxelles",
            "vcard:postal-code": "1000"});
        let participant = json!({
            "context": [GX, {"schema": "https://schema.org/"}],
            "legalPerson": {
                "schema:name": "Holder S.A.",
                "gx:legalAddress": address,
                "gx:headquartersAddress": address
            },
            "termsAndConditions": "4bd7554097444c960292b4726c2efa1373485e8a5565d94d41195214c5e0ceb3",
            "registrationNumberCredential": "registration-number.jwt"
        });
        std::fs::write(dir.join("participant.json"), participant.to_string()).unwrap();
        Self { dir }
    }

    fn config(&self, compliance_url: &str) -> Config {
        Config {
            certificate_chain_file: Some(self.dir.join("chain.pem")),
            gaiax: Some(GaiaxConfig {
                compliance_url: compliance_url.to_owned(),
                participant_file: self.dir.join("participant.json"),
            }),
            allow_http: true,
            ..config()
        }
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// An identity with Gaia-X on, the fake service, and their files.
struct Setup {
    world: World,
    service: Arc<FakeService>,
    _files: Files,
}

async fn setup() -> Setup {
    let service = Arc::new(FakeService {
        party: Party::new(COMPLIANCE),
        holder: Mutex::new(None),
        wrong_digest: AtomicBool::new(false),
        asked: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route(
            "/v2/api/credential-offers/standard-compliance",
            post(standard_compliance),
        )
        .with_state(Arc::clone(&service));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v2", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let notary = Party::new(NOTARY);
    let files = Files::new(&notary);
    let adapter = HttpGaiaxCompliance::new(&url, true).unwrap();
    let world = World::try_with(
        &files.config(&url),
        Arc::new(SqliteStore::in_memory().unwrap()),
        SigningKey::from_pem(KEY).unwrap(),
        Some(Arc::new(adapter)),
    )
    .unwrap();
    world.directory.add(&service.party);
    *service.holder.lock().unwrap() = Some(did_document(&world).await);
    Setup {
        world,
        service,
        _files: files,
    }
}

async fn get(app: &Router, path: &str) -> (StatusCode, HeaderMap, String) {
    let response = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn did_document(world: &World) -> DidDocument {
    let (_, _, text) = get(&world.app, "/.well-known/did.json").await;
    serde_json::from_str(&text).unwrap()
}

/// The Gaia-X credentials held, by `holderPid`'s name part.
fn held_gaiax(world: &World) -> Vec<(String, String)> {
    let mut held: Vec<(String, String)> = world
        .identity
        .store()
        .credentials()
        .unwrap()
        .into_iter()
        .filter_map(|c| {
            c.holder_pid
                .strip_prefix("gaiax:")
                .map(|name| (name.to_owned(), c.id.clone()))
        })
        .collect();
    held.sort();
    held
}

#[tokio::test]
async fn onboarding_stores_and_serves_the_credentials() {
    let setup = setup().await;
    let world = &setup.world;

    // the DID document's JWK names its algorithm and its chain
    let (_, _, text) = get(&world.app, "/.well-known/did.json").await;
    let document: Value = serde_json::from_str(&text).unwrap();
    let jwk = &document["verificationMethod"][0]["publicKeyJwk"];
    assert_eq!(jwk["alg"], "ES256");
    assert_eq!(
        jwk["x5u"],
        "https://holder.example/.well-known/x509CertificateChain.pem"
    );
    let (status, headers, chain) = get(&world.app, "/.well-known/x509CertificateChain.pem").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(chain, CHAIN);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/pem-certificate-chain"
    );

    // before a run, nothing is served
    let (status, _, _) = get(&world.app, "/credentials/gaiax/legal-person").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(renewal_due(&world.identity, now_secs()), 0, "due now");

    let run = run_now(&world.identity).await.unwrap();
    assert_eq!(run.outcome, RunOutcome::Succeeded, "{:?}", run.error);
    assert_eq!(
        *setup.service.asked.lock().unwrap(),
        [format!("{BASE}/compliance")]
    );
    assert_eq!(
        held_gaiax(world),
        [
            ("compliance".to_owned(), format!("{BASE}/compliance")),
            ("legal-person".to_owned(), format!("{BASE}/legal-person")),
            (
                "registration-number".to_owned(),
                format!("{BASE}/registration-number")
            ),
            (
                "terms-and-conditions".to_owned(),
                format!("{BASE}/terms-and-conditions")
            ),
        ]
    );
    // renewal two thirds into the 90 days
    let due = renewal_due(&world.identity, now_secs());
    let sixty_days = 60 * 24 * 3600;
    assert!(due.abs_diff(now_secs() + sixty_days) < 10, "{due}");

    // the self-signed credentials are public, as VC-JWT
    let (status, headers, legal) = get(&world.app, "/credentials/gaiax/legal-person").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/vc+jwt");
    let legal_payload = payload_of(&legal);
    assert_eq!(
        legal_payload["credentialSubject"]["id"],
        format!("{BASE}/legal-person#cs")
    );
    assert_eq!(
        legal_payload["credentialSubject"]["gx:registrationNumber"],
        json!({"id": format!("{BASE}/registration-number#cs")})
    );
    assert_eq!(
        legal_payload["@context"],
        json!(["https://www.w3.org/ns/credentials/v2", GX, {"schema": "https://schema.org/"}])
    );
    assert_eq!(
        header_of(&legal),
        json!({"alg": "ES256", "typ": "vc+jwt", "cty": "vc", "iss": HOLDER, "kid": world.holder_kid()})
    );
    for name in ["terms-and-conditions", "registration-number"] {
        let (status, _, _) = get(&world.app, &format!("/credentials/gaiax/{name}")).await;
        assert_eq!(status, StatusCode::OK, "{name}");
    }
    let (status, _, _) = get(&world.app, "/credentials/gaiax/compliance").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only the three inputs");

    // the presentation API finds them by scope type without the prefix
    for (scope_type, wanted) in [
        (
            "ParticipantComplianceCredential",
            "gx:ParticipantComplianceCredential",
        ),
        ("LegalPerson", "gx:LegalPerson"),
    ] {
        let scope = format!("org.eclipse.dspace.dcp.vc.type:{scope_type}");
        let (status, answer) = world.query(&scope, &scope_query(&[&scope])).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        let presentations = answer["presentation"].as_array().unwrap();
        assert_eq!(presentations.len(), 1, "{answer}");
        let presentation = Presentation::parse(presentations[0].as_str().unwrap()).unwrap();
        let types: Vec<&str> = presentation
            .credentials
            .iter()
            .flat_map(|c| c.types.iter().map(String::as_str))
            .filter(|t| *t != "VerifiableCredential")
            .collect();
        assert_eq!(types, [wanted]);
    }
}

#[tokio::test]
async fn a_wrong_digest_is_refused_and_nothing_is_stored() {
    let setup = setup().await;
    let world = &setup.world;
    setup.service.wrong_digest.store(true, Ordering::SeqCst);
    let run = run_now(&world.identity).await.unwrap();
    assert_eq!(run.outcome, RunOutcome::Failed);
    let error = run.error.unwrap();
    assert!(error.contains("digestSRI"), "{error}");
    assert!(held_gaiax(world).is_empty());

    // after a good run, a bad one keeps the credentials of the good one
    setup.service.wrong_digest.store(false, Ordering::SeqCst);
    let good = run_now(&world.identity).await.unwrap();
    assert_eq!(good.outcome, RunOutcome::Succeeded, "{:?}", good.error);
    let (_, _, before) = get(&world.app, "/credentials/gaiax/legal-person").await;
    setup.service.wrong_digest.store(true, Ordering::SeqCst);
    let bad = run_now(&world.identity).await.unwrap();
    assert_eq!(bad.outcome, RunOutcome::Failed);
    let (_, _, after) = get(&world.app, "/credentials/gaiax/legal-person").await;
    assert_eq!(before, after);
    assert_eq!(held_gaiax(world).len(), 4);
}

#[tokio::test]
async fn the_management_api_starts_and_lists_runs() {
    let setup = setup().await;
    let world = &setup.world;
    let authed = |request: axum::http::request::Builder| {
        request
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
            .body(Body::empty())
            .unwrap()
    };
    let (status, started) = call(
        &world.management,
        authed(Request::post("/v1/gaiax/compliance")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["outcome"], "RUNNING");
    let id = started["id"].clone();
    // wait for the run in the background
    let mut runs = Value::Null;
    for _ in 0..200 {
        let (status, listed) = call(
            &world.management,
            authed(Request::get("/v1/gaiax/compliance")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        runs = listed;
        if runs[0]["outcome"] != "RUNNING" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(runs[0]["id"], id);
    assert_eq!(runs[0]["outcome"], "SUCCEEDED", "{runs}");
    assert!(runs[0]["finished"].is_string());
    assert!(runs[0].get("error").is_none());
    let (_, credentials) = call(&world.management, authed(Request::get("/v1/credentials"))).await;
    let compliance = credentials
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["types"][1] == "gx:ParticipantComplianceCredential")
        .unwrap();
    assert_eq!(compliance["issuer"], COMPLIANCE);
}

#[tokio::test]
async fn gaiax_off_has_no_runs() {
    let world = World::new();
    let (status, body) = call(
        &world.management,
        Request::post("/v1/gaiax/compliance")
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // the DID document names the algorithm, without x5u
    let (_, _, text) = get(&world.app, "/.well-known/did.json").await;
    let document: Value = serde_json::from_str(&text).unwrap();
    let jwk = &document["verificationMethod"][0]["publicKeyJwk"];
    assert_eq!(jwk["alg"], "ES256");
    assert!(jwk.get("x5u").is_none());
    let (status, _, _) = get(&world.app, "/.well-known/x509CertificateChain.pem").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_chain_for_another_key_stops_the_start() {
    let notary = Party::new(NOTARY);
    let files = Files::new(&notary);
    std::fs::write(files.dir.join("chain.pem"), OTHER_CERT).unwrap();
    let adapter = HttpGaiaxCompliance::new("http://127.0.0.1:1", true).unwrap();
    let refused = World::try_with(
        &files.config("http://127.0.0.1:1"),
        Arc::new(SqliteStore::in_memory().unwrap()),
        SigningKey::from_pem(KEY).unwrap(),
        Some(Arc::new(adapter)),
    )
    .err()
    .unwrap();
    assert!(
        refused.contains("PORTITOR_IDENTITY_CERTIFICATE_CHAIN"),
        "{refused}"
    );
    assert!(refused.contains("another key"), "{refused}");
}

#[tokio::test]
async fn a_bad_participant_file_stops_the_start() {
    let notary = Party::new(NOTARY);
    let files = Files::new(&notary);
    let start = || {
        let adapter = HttpGaiaxCompliance::new("http://127.0.0.1:1", true).unwrap();
        World::try_with(
            &files.config("http://127.0.0.1:1"),
            Arc::new(SqliteStore::in_memory().unwrap()),
            SigningKey::from_pem(KEY).unwrap(),
            Some(Arc::new(adapter)),
        )
        .err()
    };
    let participant = files.dir.join("participant.json");
    let good: Value =
        serde_json::from_str(&std::fs::read_to_string(&participant).unwrap()).unwrap();
    for (change, expected) in [
        (json!({"legalPerson": {"id": "x"}}), "must not hold id"),
        (
            json!({"context": "https://example.org/other"}),
            "no Gaia-X context",
        ),
        (
            json!({"registrationNumberCredential": "missing.jwt"}),
            "cannot read",
        ),
        (json!({"extra": 1}), "unknown field"),
    ] {
        let mut file = good.clone();
        for (name, value) in change.as_object().unwrap() {
            file[name] = value.clone();
        }
        std::fs::write(&participant, file.to_string()).unwrap();
        let refused = start().unwrap();
        assert!(refused.contains(expected), "{refused}");
    }
}
