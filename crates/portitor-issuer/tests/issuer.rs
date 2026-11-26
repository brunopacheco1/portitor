// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The issuer end to end, with fake neighbours: DID documents come from
//! memory, and what the issuer sends to holders is recorded.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use portitor_dcp::credential::Credential;
use portitor_dcp::issuance::STATUS_LIST_ENTRIES;
use portitor_dcp::message::RequestStatus;
use portitor_dcp::status_list::{Status, check, list_in};
use portitor_dcp::x509::p256_public_key_pem;
use portitor_dcp::{
    Claims, DidDocument, JtiCache, Jws, Relationship, Service, SigningKey, VerificationMethod,
    validate_self_issued,
};
use portitor_issuer::config::{Config, DatabaseSetting};
use portitor_issuer::management::openapi_json;
use portitor_issuer::store::{
    CredentialRequest, DatabaseStore, Delivery, DeliveryState, IssueOutcome, IssueRules,
    IssuedWithDelivery, IssuerStore, Lease, Participant, Planned, Revocation, SignCredentials,
    SignStatusList, SignedStatusList, StoreError,
};
use portitor_issuer::{Issuer, Ports, definitions, delivery, issue};
use portitor_runtime::BoxFuture;
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::Directory;
use portitor_runtime::http::Sender;
use portitor_runtime::system::{new_uuid, now_secs};
use portitor_runtime::testing::management_calls::{
    Call, statuses_other_than, with_a_wrong_body, with_odd_paths_and_queries, without_the_key,
};
use portitor_runtime::testing::{TempDir, databases};
use portitor_runtime::used_tokens::UsedTokenStore;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const ISSUER: &str = "did:web:issuer.example";
const HOLDER: &str = "did:web:holder.example";
const STRANGER: &str = "did:web:stranger.example";
const SERVICE: &str = "https://issuer.example/issuer";
const API_KEY: &str = "test-only-api-key-0123456789abcdef0123456789ab";

const DEFINITIONS: &str = r#"[
  {"id": "membership-vc11", "credentialType": "MembershipCredential",
   "profile": "vc11-sl2021/jwt", "validityDays": 365,
   "credentialSchema": "https://example.com/membership-credential-schema.json",
   "claims": ["role"]},
  {"id": "membership-vc20", "credentialType": "MembershipCredential",
   "profile": "vc20-bssl/jwt", "validityDays": 30,
   "credentialSchema": "https://example.com/membership-credential-schema.json",
   "claims": ["role", "level"]}
]"#;

/// The DCP v1.0.1 JSON Schemas the issuer's own answers are checked
/// against, by the URL the schemas use to refer to each other. They are
/// the copies kept for `crates/portitor-dcp/tests/spec.rs`.
const DCP_SCHEMAS: &[(&str, &str)] = &[
    (
        "https://w3id.org/dspace-dcp/v1.0/common/context-schema.json",
        include_str!("../../portitor-dcp/tests/schemas/context-schema.json"),
    ),
    (
        "https://w3id.org/dspace-dcp/v1.0/issuance/credential-object-schema.json",
        include_str!("../../portitor-dcp/tests/schemas/credential-object-schema.json"),
    ),
    (
        "https://w3id.org/dspace-dcp/v1.0/issuance/issuer-metadata-schema.json",
        include_str!("../../portitor-dcp/tests/schemas/issuer-metadata-schema.json"),
    ),
    (
        "https://w3id.org/dspace-dcp/v1.0/issuance/credential-offer-message-schema.json",
        include_str!("../../portitor-dcp/tests/schemas/credential-offer-message-schema.json"),
    ),
];

/// Serves [`DCP_SCHEMAS`], so the tests need no network. The Presentation
/// Exchange schema `issuancePolicy` points to is not part of the DCP
/// release; any value passes in its place.
struct LocalSchemas;

impl jsonschema::Retrieve for LocalSchemas {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let uri = uri.as_str();
        if uri.starts_with("https://identity.foundation/presentation-exchange/") {
            return Ok(json!(true));
        }
        let found = DCP_SCHEMAS.iter().find(|(id, _)| *id == uri);
        let (_, raw) = found.ok_or("unknown schema")?;
        Ok(serde_json::from_str(raw)?)
    }
}

/// What the DCP schema `file` reports for `message`; empty when it passes.
fn dcp_schema_errors(file: &str, message: &Value) -> Vec<String> {
    let (_, raw) = DCP_SCHEMAS
        .iter()
        .find(|(id, _)| id.ends_with(file))
        .unwrap();
    let schema: Value = serde_json::from_str(raw).unwrap();
    let validator = jsonschema::options()
        .with_retriever(LocalSchemas)
        .build(&schema)
        .unwrap();
    let mut errors = Vec::new();
    for error in validator.iter_errors(message) {
        errors.push(format!("{} at {}", error, error.instance_path()));
    }
    errors
}

/// Another participant: a DID, a key, and its DID document with a
/// credential service.
struct Party {
    did: &'static str,
    key: SigningKey,
    document: DidDocument,
}

impl Party {
    fn new(did: &'static str) -> Self {
        let key = SigningKey::generate().unwrap();
        let kid = format!("{did}#key-1");
        let only = vec![Relationship::Reference(kid.clone())];
        let document = DidDocument {
            id: did.to_owned(),
            verification_method: vec![VerificationMethod {
                id: kid,
                kind: "JsonWebKey2020".to_owned(),
                controller: did.to_owned(),
                public_key_jwk: key.public_jwk(),
            }],
            capability_invocation: only.clone(),
            authentication: only.clone(),
            assertion_method: only,
            service: vec![Service {
                id: format!("{did}#cs"),
                kind: "CredentialService".to_owned(),
                service_endpoint: Value::from(format!(
                    "https://{}/cs/",
                    did.trim_start_matches("did:web:")
                )),
            }],
        };
        Self { did, key, document }
    }

    /// A self-issued ID token for the issuer, carrying `token`.
    fn token(&self, token: Option<&str>) -> String {
        let mut claims = self.claims();
        claims.token = token.map(str::to_owned);
        self.sign(&claims)
    }

    /// The claims of a valid token for the issuer, to change in a test.
    fn claims(&self) -> Claims {
        let now = now_secs();
        Claims {
            iss: self.did.to_owned(),
            sub: self.did.to_owned(),
            aud: ISSUER.to_owned(),
            jti: new_uuid(),
            exp: now + 300,
            iat: now,
            nbf: None,
            token: None,
        }
    }

    fn sign(&self, claims: &Claims) -> String {
        self.key
            .sign(&format!("{}#key-1", self.did), claims)
            .unwrap()
    }
}

/// DID documents from memory; records which DIDs were looked up.
#[derive(Default)]
struct FakeDirectory {
    documents: Mutex<HashMap<String, Arc<DidDocument>>>,
    looked_up: Mutex<Vec<String>>,
}

impl FakeDirectory {
    fn add(&self, document: DidDocument) {
        self.documents
            .lock()
            .unwrap()
            .insert(document.id.clone(), Arc::new(document));
    }

    fn remove(&self, did: &str) {
        self.documents.lock().unwrap().remove(did);
    }
}

impl Directory for FakeDirectory {
    fn did_document<'a>(&'a self, did: &'a str) -> BoxFuture<'a, Result<Arc<DidDocument>, String>> {
        self.looked_up.lock().unwrap().push(did.to_owned());
        let found = self.documents.lock().unwrap().get(did).cloned();
        Box::pin(async move { found.ok_or_else(|| format!("no document for {did}")) })
    }

    /// The issuer reads no JSON documents.
    fn json<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { Err(format!("not served: {url}")) })
    }
}

/// One message the issuer sent.
#[derive(Debug, Clone)]
struct Sent {
    url: String,
    body: Value,
    authorization: String,
}

/// Records what is sent, and counts every try; while `failure` is set,
/// each try fails with that error (as `HttpSender` words it) and nothing
/// is recorded.
#[derive(Default)]
struct FakeSender {
    sent: Mutex<Vec<Sent>>,
    failure: Mutex<Option<String>>,
    tries: Mutex<u32>,
}

impl FakeSender {
    fn fail_with(&self, error: Option<&str>) {
        *self.failure.lock().unwrap() = error.map(str::to_owned);
    }

    fn tries(&self) -> u32 {
        *self.tries.lock().unwrap()
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl Sender for FakeSender {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.post_authorized(url, body, "")
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        *self.tries.lock().unwrap() += 1;
        let failure = self.failure.lock().unwrap().clone();
        if failure.is_none() {
            self.sent.lock().unwrap().push(Sent {
                url: url.to_owned(),
                body: body.clone(),
                authorization: authorization.to_owned(),
            });
        }
        Box::pin(async move {
            match failure {
                Some(error) => Err(error),
                None => Ok(Value::Null),
            }
        })
    }
}

struct World {
    issuer: Arc<Issuer>,
    app: Router,
    management: Router,
    store: Arc<DatabaseStore>,
    directory: Arc<FakeDirectory>,
    sender: Arc<FakeSender>,
    holder: Party,
    stranger: Party,
}

/// How a [`World`] is built.
struct Setup {
    store: DatabaseStore,
    key: SigningKey,
    max_active_per_holder: u64,
    /// `PORTITOR_ISSUER_BUNDLE_DIR`.
    bundle_dir: Option<PathBuf>,
    /// `PORTITOR_ISSUER_RETIRED_KEY_FILES`.
    retired_key_files: Vec<PathBuf>,
}

impl Setup {
    fn in_memory() -> Self {
        Self {
            store: DatabaseStore::in_memory().unwrap(),
            key: SigningKey::generate().unwrap(),
            max_active_per_holder: 10,
            bundle_dir: None,
            retired_key_files: Vec::new(),
        }
    }

    /// The issuer on a file, with `key`: started again on the same file
    /// and key, it is the same issuer after a restart.
    fn on_file(path: &Path, key: &SigningKey) -> Self {
        Self::on(&DatabaseSetting::SqliteFile(path.to_owned()), key)
    }

    /// The issuer on the database of `setting`, with `key`: two of them
    /// on one PostgreSQL database are two replicas of one issuer.
    fn on(setting: &DatabaseSetting, key: &SigningKey) -> Self {
        Self {
            store: DatabaseStore::open(setting).unwrap(),
            key: SigningKey::from_pkcs8(key.pkcs8()).unwrap(),
            max_active_per_holder: 10,
            bundle_dir: None,
            retired_key_files: Vec::new(),
        }
    }
}

impl World {
    fn new() -> Self {
        Self::with(Setup::in_memory())
    }

    fn with(setup: Setup) -> Self {
        Self::with_store_port(setup, |store| store)
    }

    /// As [`World::with`], but the issuer reaches the store through
    /// `port(store)`: a test can put a store that misbehaves in between.
    fn with_store_port(
        setup: Setup,
        port: impl FnOnce(Arc<DatabaseStore>) -> Arc<dyn IssuerStore>,
    ) -> Self {
        let directory = Arc::new(FakeDirectory::default());
        let holder = Party::new(HOLDER);
        let stranger = Party::new(STRANGER);
        for party in [&holder, &stranger] {
            directory.add(party.document.clone());
        }
        let sender = Arc::new(FakeSender::default());
        let config = Config {
            participant_id: ISSUER.to_owned(),
            key_file: "unused".into(),
            retired_key_files: setup.retired_key_files,
            credential_definitions_file: "unused".into(),
            issuer_service_url: SERVICE.to_owned(),
            bind: "unused".to_owned(),
            database: portitor_runtime::database::DatabaseSetting::SqliteFile("unused".into()),
            management: portitor_issuer::config::ManagementConfig::health_only(
                portitor_issuer::config::DEFAULT_MANAGEMENT_BIND,
            ),
            allow_http: false,
            max_active_per_holder: setup.max_active_per_holder,
            bundle_key_file: None,
            bundle_dir: setup.bundle_dir,
        };
        let store = Arc::new(setup.store);
        let ports = Ports {
            store: port(Arc::clone(&store)),
            directory: Arc::clone(&directory) as _,
            sender: Arc::clone(&sender) as _,
        };
        let issuer = Arc::new(
            Issuer::new(
                &config,
                setup.key,
                definitions::parse(DEFINITIONS).unwrap(),
                ports,
            )
            .unwrap(),
        );
        let app = portitor_issuer::router(Arc::clone(&issuer));
        let management =
            portitor_issuer::management::router(Arc::clone(&issuer), ApiKey::new(API_KEY).unwrap());
        Self {
            issuer,
            app,
            management,
            store,
            directory,
            sender,
            holder,
            stranger,
        }
    }

    async fn register(&self, did: &str, claims: Value) -> StatusCode {
        let (status, _) = call(
            &self.management,
            authed(Request::post("/v1/participants"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"did": did, "claims": claims}).to_string(),
                ))
                .unwrap(),
        )
        .await;
        status
    }

    /// `POST /credentials` as `party`; returns the status and the
    /// `Location`.
    async fn request(
        &self,
        party: &Party,
        holder_pid: &str,
        ids: &[&str],
        access_token: Option<&str>,
    ) -> (StatusCode, Option<String>) {
        let body = json!({
            "@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
            "type": "CredentialRequestMessage",
            "holderPid": holder_pid,
            "credentials": ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()
        });
        let response = self
            .app
            .clone()
            .oneshot(
                Request::post("/issuer/credentials")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", party.token(access_token)),
                    )
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|value| value.to_str().unwrap().to_owned());
        (response.status(), location)
    }

    /// The request's status as `party` sees it.
    async fn status(&self, party: &Party, location: &str) -> (StatusCode, Value) {
        let path = location.trim_start_matches("https://issuer.example");
        call(
            &self.app,
            Request::get(path)
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", party.token(None)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    /// Waits until the request at `location` is decided, and returns its
    /// status.
    async fn decided(&self, location: &str) -> String {
        for _ in 0..100 {
            let (_, body) = self.status(&self.holder, location).await;
            let status = body["status"].as_str().unwrap().to_owned();
            if status != "RECEIVED" {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the request was not decided");
    }

    async fn listed(&self, query: &str) -> Vec<Value> {
        let (status, body) = call(
            &self.management,
            authed(Request::get(format!("/v1/credentials{query}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        body.as_array().unwrap().clone()
    }

    /// Revokes the credential `id`; returns the answer's status.
    async fn revoke(&self, id: &str) -> StatusCode {
        let (status, _) = call(
            &self.management,
            authed(Request::post(format!("/v1/credentials/{id}/revoke")))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        status
    }

    /// Asks for `ids` as the holder and waits for the decision; returns
    /// it.
    async fn ask(&self, holder_pid: &str, ids: &[&str]) -> String {
        let (status, location) = self.request(&self.holder, holder_pid, ids, None).await;
        assert_eq!(status, StatusCode::CREATED);
        self.decided(&location.unwrap()).await
    }

    /// Where the delivery of the only issued request stands, and how many
    /// tries failed.
    async fn delivery(&self) -> (String, Option<u32>) {
        let listed = self.listed("").await;
        let state = listed[0]["delivery"].as_str().unwrap().to_owned();
        let attempts = self
            .store
            .due_deliveries(u64::MAX)
            .unwrap()
            .first()
            .map(|waiting| waiting.attempts);
        (state, attempts)
    }

    /// The status (`Clear` or `Set`) of the credential `jwt` in its
    /// status list, as served now.
    async fn status_in_list(&self, jwt: &str) -> Status {
        let credential = Credential::parse(jwt).unwrap();
        let entry = credential.status[0].clone();
        let name = entry.status_list_credential.rsplit('/').next().unwrap();
        let list = Credential::parse(&self.status_list(name).await).unwrap();
        list.verify(&self.did_document().await, now_secs(), 5)
            .unwrap();
        check(&entry, &list_in(&list, &entry).unwrap()).unwrap()
    }

    /// The credentials (JWTs) in the delivered message `n`.
    fn delivered(&self, n: usize) -> Vec<String> {
        let sent = self.sender.sent();
        let mut jwts = Vec::new();
        for container in sent[n].body["credentials"].as_array().unwrap() {
            jwts.push(container["payload"].as_str().unwrap().to_owned());
        }
        jwts
    }

    async fn did_document(&self) -> DidDocument {
        let (_, json) = call(
            &self.app,
            Request::get("/.well-known/did.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        serde_json::from_value(json).unwrap()
    }

    async fn status_list(&self, name: &str) -> String {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::get(format!("/issuer/status-lists/{name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/vc+jwt"
        );
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}

fn authed(request: axum::http::request::Builder) -> axum::http::request::Builder {
    request.header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn did_document_and_metadata() {
    let world = World::new();
    let document = world.did_document().await;
    assert_eq!(document.id, ISSUER);
    assert_eq!(document.service_url("IssuerService"), Some(SERVICE));
    let (status, metadata) = call(
        &world.app,
        Request::get("/issuer/metadata")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(metadata["type"], "IssuerMetadata");
    assert_eq!(metadata["issuer"], ISSUER);
    assert_eq!(metadata["credentialsSupported"][1]["id"], "membership-vc20");
}

/// TD-33: what `GET /metadata` answers passes DCP's own
/// `issuer-metadata-schema.json` (DCP v1.0.1, section 6.7), which wants
/// `credentialType`, `credentialSchema`, `offerReason`, `bindingMethods`
/// and `profile` in every credential object.
#[tokio::test]
async fn the_metadata_answer_passes_the_dcp_schema() {
    let world = World::new();
    let (status, metadata) = call(
        &world.app,
        Request::get("/issuer/metadata")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let errors = dcp_schema_errors("issuer-metadata-schema.json", &metadata);
    assert!(errors.is_empty(), "{errors:?}\n{metadata:#}");

    let supported = metadata["credentialsSupported"].as_array().unwrap();
    assert_eq!(supported.len(), 2);
    for object in supported {
        assert_eq!(
            object["credentialSchema"],
            "https://example.com/membership-credential-schema.json"
        );
        assert_eq!(object["offerReason"], "reissue");
        assert_eq!(object["bindingMethods"], json!(["did:web"]));
    }

    // the check is real: without one required property the schema objects
    for property in ["credentialSchema", "offerReason"] {
        let mut incomplete = metadata.clone();
        let first = incomplete["credentialsSupported"][0]
            .as_object_mut()
            .unwrap();
        first.remove(property);
        let errors = dcp_schema_errors("issuer-metadata-schema.json", &incomplete);
        assert!(!errors.is_empty(), "{property} is required");
    }
}

/// Every definitions file in the repository (the demos, the TCK run, the
/// EDC interop test) has the `credentialSchema` 0.12 requires, and the
/// metadata built from it passes the DCP schema.
#[test]
fn the_definitions_files_of_the_repository_are_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in [
        "examples/issuer-credentials.json",
        "tck/issuer-credentials.json",
        "scenarios/edc-interop/config/issuer-credentials.json",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        let definitions = match portitor_issuer::definitions::parse(&text) {
            Ok(definitions) => definitions,
            Err(e) => panic!("{file}: {e}"),
        };
        let metadata = portitor_issuer::definitions::metadata(ISSUER, &definitions);
        let json = serde_json::to_value(&metadata).unwrap();
        let errors = dcp_schema_errors("issuer-metadata-schema.json", &json);
        assert!(errors.is_empty(), "{file}: {errors:?}");
    }
}

#[tokio::test]
async fn a_registered_holder_gets_one_delivery_with_its_access_token() {
    let world = World::new();
    assert_eq!(
        world
            .register(HOLDER, json!({"role": "provider", "secret": 1}))
            .await,
        StatusCode::CREATED
    );
    let (status, location) = world
        .request(
            &world.holder,
            "h-1",
            &["membership-vc11", "membership-vc20"],
            Some("holder-access-token"),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let location = location.unwrap();
    assert!(location.starts_with("https://issuer.example/issuer/requests/"));
    assert_eq!(world.decided(&location).await, "ISSUED");

    delivery::send_due(&world.issuer).await;
    let sent = world.sender.sent();
    assert_eq!(sent.len(), 1, "one message for the whole request");
    assert_eq!(sent[0].url, "https://holder.example/cs/credentials");
    let message = &sent[0].body;
    assert_eq!(message["type"], "CredentialMessage");
    assert_eq!(message["holderPid"], "h-1");
    assert_eq!(message["status"], "ISSUED");
    assert_eq!(message["credentials"].as_array().unwrap().len(), 2);

    // the delivery token: from the issuer to the holder, with the token
    let issuer_document = world.did_document().await;
    let token = sent[0].authorization.strip_prefix("Bearer ").unwrap();
    let claims = validate_self_issued(
        token,
        &issuer_document,
        HOLDER,
        now_secs(),
        5,
        &mut JtiCache::default(),
    )
    .unwrap();
    assert_eq!(claims.token.as_deref(), Some("holder-access-token"));

    // the credentials verify, name the holder and carry the claims
    for container in message["credentials"].as_array().unwrap() {
        let credential = Credential::parse(container["payload"].as_str().unwrap()).unwrap();
        credential.verify(&issuer_document, now_secs(), 5).unwrap();
        assert!(credential.is_only_about(HOLDER));
        assert_eq!(credential.subjects[0]["role"], "provider");
        assert!(credential.subjects[0].get("secret").is_none());
        assert_eq!(container["format"], credential.profile.alias());
    }

    // sent once only, and listed as delivered
    delivery::send_due(&world.issuer).await;
    assert_eq!(world.sender.sent.lock().unwrap().len(), 1);
    let listed = world.listed(&format!("?holder={HOLDER}")).await;
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|c| c["delivery"] == "DELIVERED"));
    assert!(
        world
            .listed("?holder=did:web:nobody.example")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn an_unregistered_caller_gets_401_and_nothing_is_fetched_or_stored() {
    let world = World::new();
    let (status, location) = world
        .request(&world.holder, "h-1", &["membership-vc11"], None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(location, None);
    // a request status, even for an unknown id: the same
    let (status, _) = world
        .status(&world.holder, "/issuer/requests/not-there")
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(world.directory.looked_up.lock().unwrap().is_empty());
    assert!(world.store.received_requests().unwrap().is_empty());

    // once registered, it may ask
    world.register(HOLDER, json!({})).await;
    let (status, _) = world
        .request(&world.holder, "h-1", &["membership-vc11"], None)
        .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn unknown_ids_are_rejected_and_nothing_is_sent() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    assert_eq!(
        world.ask("h-2", &["membership-vc11", "unknown"]).await,
        "REJECTED"
    );
    delivery::send_due(&world.issuer).await;
    assert!(world.sender.sent().is_empty());
    assert!(world.listed("").await.is_empty());
}

#[tokio::test]
async fn a_deleted_participant_gets_401_and_a_waiting_request_is_rejected() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    // taken while registered, decided after the registration is deleted
    let waiting = CredentialRequest {
        issuer_pid: "i-1".to_owned(),
        holder_did: HOLDER.to_owned(),
        holder_pid: "h-1".to_owned(),
        credential_ids: vec!["membership-vc20".to_owned()],
        access_token: None,
        status: RequestStatus::Received,
        received_at: now_secs(),
    };
    assert!(world.store.add_request(&waiting).unwrap());
    assert!(world.store.delete_participant(HOLDER).unwrap());
    issue::decide(&world.issuer, "i-1");
    assert_eq!(
        world.store.request("i-1").unwrap().unwrap().status,
        RequestStatus::Rejected
    );
    let (status, _) = world
        .request(&world.holder, "h-2", &["membership-vc20"], None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(world.listed("").await.is_empty());
}

#[tokio::test]
async fn bad_tokens_are_refused_on_both_routes() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    let (_, location) = world
        .request(&world.holder, "h-1", &["membership-vc11"], None)
        .await;
    let status_path = location
        .unwrap()
        .trim_start_matches("https://issuer.example")
        .to_owned();
    let body = json!({
        "@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
        "type": "CredentialRequestMessage",
        "holderPid": "h-bad",
        "credentials": [{"id": "membership-vc11"}]
    })
    .to_string();
    let holder = &world.holder;
    let mut wrong_audience = holder.claims();
    wrong_audience.aud = "did:web:someone-else.example".to_owned();
    let mut expired = holder.claims();
    expired.iat -= 3_600;
    expired.exp = now_secs() - 600;
    let replayed = holder.sign(&holder.claims());
    for (name, token) in [
        ("wrong aud", holder.sign(&wrong_audience)),
        ("expired", holder.sign(&expired)),
    ] {
        for (method, uri, body) in [
            ("POST", "/issuer/credentials", body.clone()),
            ("GET", status_path.as_str(), String::new()),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap();
            let (status, _) = call(&world.app, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}, {method}");
        }
    }
    // a token is good once, whichever route used it first
    for (method, uri, body, expected) in [
        ("GET", status_path.as_str(), String::new(), StatusCode::OK),
        (
            "GET",
            status_path.as_str(),
            String::new(),
            StatusCode::UNAUTHORIZED,
        ),
        (
            "POST",
            "/issuer/credentials",
            body.clone(),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {replayed}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        let (status, _) = call(&world.app, request).await;
        assert_eq!(status, expected, "replayed jti, {method}");
    }
}

#[tokio::test]
async fn a_repeated_holder_pid_issues_nothing_and_bad_requests_are_refused() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    world.register(STRANGER, json!({})).await;
    let (first, location) = world
        .request(&world.holder, "h-1", &["membership-vc11"], None)
        .await;
    assert_eq!(first, StatusCode::CREATED);
    assert_eq!(world.decided(&location.unwrap()).await, "ISSUED");
    let (again, _) = world
        .request(&world.holder, "h-1", &["membership-vc20"], None)
        .await;
    assert_eq!(again, StatusCode::CONFLICT);
    // another holder may use the same holderPid
    let (other, _) = world
        .request(&world.stranger, "h-1", &["membership-vc11"], None)
        .await;
    assert_eq!(other, StatusCode::CREATED);

    let (empty, _) = world.request(&world.holder, "h-3", &[], None).await;
    assert_eq!(empty, StatusCode::BAD_REQUEST);
    let (no_pid, _) = world
        .request(&world.holder, "", &["membership-vc11"], None)
        .await;
    assert_eq!(no_pid, StatusCode::BAD_REQUEST);

    let unsigned = world
        .app
        .clone()
        .oneshot(
            Request::post("/issuer/credentials")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);

    // the repeated holderPid issued nothing: the holder has its one
    // vc11 credential, nothing of vc20, and one message waits
    let held = world.listed(&format!("?holder={HOLDER}")).await;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0]["definitionId"], "membership-vc11");
    delivery::send_due(&world.issuer).await;
    let to_holder = world
        .sender
        .sent()
        .iter()
        .filter(|sent| sent.url.starts_with("https://holder.example/"))
        .count();
    assert_eq!(to_holder, 1);
}

#[tokio::test]
async fn only_the_requesting_holder_sees_the_status() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    let (_, location) = world
        .request(&world.holder, "h-1", &["membership-vc11"], None)
        .await;
    let location = location.unwrap();
    world.decided(&location).await;
    let (status, body) = world.status(&world.holder, &location).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["type"], "CredentialStatus");
    assert_eq!(body["holderPid"], "h-1");
    // another caller: 401 while it is not registered, 404 once it is
    let (status, _) = world.status(&world.stranger, &location).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    world.register(STRANGER, json!({})).await;
    let (status, _) = world.status(&world.stranger, &location).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = world
        .status(&world.holder, "/issuer/requests/not-there")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_failed_delivery_is_retried_later() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    world.sender.fail_with(Some("HTTP 503 Service Unavailable"));
    assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    let start = now_secs();
    delivery::send_due_at(&world.issuer, start).await;
    assert_eq!(world.sender.tries(), 1);
    assert_eq!(world.delivery().await, ("PENDING".to_owned(), Some(1)));
    world.sender.fail_with(None);
    // the next try waits one second: not due yet
    delivery::send_due_at(&world.issuer, start).await;
    assert_eq!(world.sender.tries(), 1);
    delivery::send_due_at(&world.issuer, start + 1).await;
    assert_eq!(world.sender.tries(), 2);
    assert_eq!(world.sender.sent().len(), 1);
    assert_eq!(world.delivery().await, ("DELIVERED".to_owned(), None));
}

/// Each try takes long here: the clock moves 40 seconds each time it is
/// read. Each claim and each record reads it again, so a lease and the
/// next try count from when they are taken, not from when the round
/// began (where the second delivery's lease would have ended before it
/// was taken).
#[tokio::test]
async fn each_try_reads_the_clock_again() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    world.sender.fail_with(Some("HTTP 503 Service Unavailable"));
    assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    assert_eq!(world.ask("h-2", &["membership-vc20"]).await, "ISSUED");
    let start = now_secs();
    let time = std::sync::atomic::AtomicU64::new(start);
    let clock = || time.fetch_add(40, std::sync::atomic::Ordering::SeqCst);
    // reads: the round at start; claim, record, claim, record 40 s apart
    delivery::send_due_with(&world.issuer, &clock).await;
    assert_eq!(world.sender.tries(), 2);
    let mut next: Vec<u64> = world
        .store
        .due_deliveries(u64::MAX)
        .unwrap()
        .iter()
        .map(|waiting| waiting.next_attempt_at)
        .collect();
    next.sort_unstable();
    // recorded at start + 80 and start + 160, then one second
    assert_eq!(next, [start + 81, start + 161]);
}

#[tokio::test]
async fn a_delivery_is_given_up_after_the_last_try() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    world.sender.fail_with(Some("no answer within 30s"));
    assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    let mut now = now_secs();
    for tried in 1..delivery::MAX_ATTEMPTS {
        delivery::send_due_at(&world.issuer, now).await;
        assert_eq!(world.delivery().await, ("PENDING".to_owned(), Some(tried)));
        now += portitor_runtime::retry::MAX_RETRY_DELAY;
    }
    delivery::send_due_at(&world.issuer, now).await;
    assert_eq!(world.sender.tries(), delivery::MAX_ATTEMPTS);
    assert_eq!(world.delivery().await, ("FAILED".to_owned(), None));
    // the credential stays issued, and nothing more is tried
    delivery::send_due_at(&world.issuer, u64::MAX / 2).await;
    assert_eq!(world.sender.tries(), delivery::MAX_ATTEMPTS);
    assert_eq!(world.listed("").await.len(), 1);
}

#[tokio::test]
async fn a_refusal_by_the_holder_ends_the_delivery_at_once() {
    for (answer, permanent) in [
        ("HTTP 400 Bad Request", true),
        ("HTTP 403 Forbidden", true),
        ("HTTP 404 Not Found", true),
        ("HTTP 401 Unauthorized", false),
        ("HTTP 408 Request Timeout", false),
        ("HTTP 429 Too Many Requests", false),
        ("HTTP 502 Bad Gateway", false),
    ] {
        let world = World::new();
        world.register(HOLDER, json!({})).await;
        world.sender.fail_with(Some(answer));
        assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
        delivery::send_due(&world.issuer).await;
        let expected = if permanent {
            ("FAILED".to_owned(), None)
        } else {
            ("PENDING".to_owned(), Some(1))
        };
        assert_eq!(world.delivery().await, expected, "{answer}");
    }
}

#[tokio::test]
async fn a_holder_without_did_document_or_credential_service_is_retried() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    // the holder's DID no longer resolves
    world.directory.remove(HOLDER);
    let start = now_secs();
    delivery::send_due_at(&world.issuer, start).await;
    assert_eq!(world.sender.tries(), 0, "nothing to send to");
    assert_eq!(world.delivery().await, ("PENDING".to_owned(), Some(1)));
    // it resolves, without a credential service
    let mut without_service = world.holder.document.clone();
    without_service.service.clear();
    world.directory.add(without_service);
    delivery::send_due_at(&world.issuer, start + 1).await;
    assert_eq!(world.sender.tries(), 0);
    assert_eq!(world.delivery().await, ("PENDING".to_owned(), Some(2)));
    // and with one again: delivered
    world.directory.add(world.holder.document.clone());
    delivery::send_due_at(&world.issuer, start + 3).await;
    assert_eq!(world.sender.sent().len(), 1);
    assert_eq!(world.delivery().await, ("DELIVERED".to_owned(), None));
}

#[tokio::test]
async fn revocation_sets_the_bit_and_signs_the_list_again() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    assert_eq!(
        world
            .ask("h-1", &["membership-vc11", "membership-vc20"])
            .await,
        "ISSUED"
    );
    delivery::send_due(&world.issuer).await;
    let jwts = world.delivered(0);
    assert_eq!(jwts.len(), 2);
    let mut lists = Vec::new();
    for jwt in &jwts {
        let credential = Credential::parse(jwt).unwrap();
        lists.push(credential.status[0].status_list_credential.clone());
        assert_eq!(world.status_in_list(jwt).await, Status::Clear);
    }
    assert_eq!(
        lists,
        [
            "https://issuer.example/issuer/status-lists/vc11",
            "https://issuer.example/issuer/status-lists/vc20"
        ]
    );

    for jwt in &jwts {
        let id = Credential::parse(jwt).unwrap().id.unwrap();
        // revoking again changes nothing, and is fine
        for _ in 0..2 {
            assert_eq!(world.revoke(&id).await, StatusCode::NO_CONTENT);
        }
    }
    for jwt in &jwts {
        assert_eq!(world.status_in_list(jwt).await, Status::Set);
    }
    assert!(
        world
            .listed("")
            .await
            .iter()
            .all(|credential| credential["revoked"].is_string())
    );
    assert_eq!(world.revoke("urn:uuid:none").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_holder_may_hold_a_limited_number_of_active_credentials() {
    let mut setup = Setup::in_memory();
    setup.max_active_per_holder = 1;
    let world = World::with(setup);
    world.register(HOLDER, json!({})).await;
    assert_eq!(world.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    assert_eq!(world.ask("h-2", &["membership-vc20"]).await, "REJECTED");
    // another definition is counted on its own
    assert_eq!(world.ask("h-3", &["membership-vc11"]).await, "ISSUED");
    // once revoked, it no longer counts
    let first = world
        .listed("")
        .await
        .into_iter()
        .find(|credential| credential["definitionId"] == "membership-vc20")
        .unwrap();
    let id = first["id"].as_str().unwrap();
    assert_eq!(world.revoke(id).await, StatusCode::NO_CONTENT);
    assert_eq!(world.ask("h-4", &["membership-vc20"]).await, "ISSUED");
}

#[tokio::test]
async fn a_full_status_list_rejects_the_request() {
    let dir = TempDir::new("issuer-test");
    let path = dir.path().join("issuer.db");
    let world = World::with(Setup::on_file(&path, &SigningKey::generate().unwrap()));
    // every position of the vc11 list is taken, by other holders
    let mut conn = rusqlite::Connection::open(&path).unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute(
        "INSERT INTO credential_request (issuer_pid, holder_did, holder_pid, credential_ids,
             status, received_at) VALUES ('old', 'did:web:others.example', 'old', '[]',
             'ISSUED', 0)",
        [],
    )
    .unwrap();
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO issued_credential (id, issuer_pid, holder_did, definition_id,
                     credential_type, profile, status_index, issued_at, expires_at)
                     VALUES (?1, 'old', 'did:web:others.example', 'membership-vc11',
                     'MembershipCredential', 'vc11-sl2021/jwt', ?1, 0, 9999999999)",
            )
            .unwrap();
        for position in 0..STATUS_LIST_ENTRIES {
            insert.execute([i64::try_from(position).unwrap()]).unwrap();
        }
    }
    tx.commit().unwrap();
    drop(conn);

    world.register(HOLDER, json!({})).await;
    assert_eq!(
        world
            .ask("h-1", &["membership-vc20", "membership-vc11"])
            .await,
        "REJECTED"
    );
    // the vc20 list lost no position for the rejected request
    assert!(world.listed(&format!("?holder={HOLDER}")).await.is_empty());
    assert_eq!(world.ask("h-2", &["membership-vc20"]).await, "ISSUED");
}

#[tokio::test]
async fn after_a_restart_waiting_work_is_done_and_revocations_kept() {
    let dir = TempDir::new("issuer-test");
    let path = dir.path().join("issuer.db");
    let key = SigningKey::generate().unwrap();

    // before the restart: one credential delivered and revoked, one
    // issued but not delivered, one request taken but not decided
    let before = World::with(Setup::on_file(&path, &key));
    before.register(HOLDER, json!({})).await;
    assert_eq!(before.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    delivery::send_due(&before.issuer).await;
    let revoked_jwt = before.delivered(0)[0].clone();
    let revoked_id = Credential::parse(&revoked_jwt).unwrap().id.unwrap();
    assert_eq!(before.revoke(&revoked_id).await, StatusCode::NO_CONTENT);
    before
        .sender
        .fail_with(Some("HTTP 503 Service Unavailable"));
    assert_eq!(before.ask("h-2", &["membership-vc11"]).await, "ISSUED");
    delivery::send_due(&before.issuer).await;
    let undecided = CredentialRequest {
        issuer_pid: "i-3".to_owned(),
        holder_did: HOLDER.to_owned(),
        holder_pid: "h-3".to_owned(),
        credential_ids: vec!["membership-vc20".to_owned()],
        access_token: Some("holder-access-token".to_owned()),
        status: RequestStatus::Received,
        received_at: now_secs(),
    };
    assert!(before.store.add_request(&undecided).unwrap());
    drop(before);

    // after: the same file and key
    let after = World::with(Setup::on_file(&path, &key));
    // the list is signed at start with the revoked bit still set
    assert_eq!(after.status_in_list(&revoked_jwt).await, Status::Set);
    // the outbox decides the waiting request and sends both messages;
    // the first try of h-2 failed, so it is due a second later
    let outbox = tokio::spawn(delivery::run_outbox(Arc::clone(&after.issuer)));
    for _ in 0..500 {
        if after.sender.sent().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    outbox.abort();
    let sent = after.sender.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    let mut holder_pids: Vec<&str> = sent
        .iter()
        .map(|message| message.body["holderPid"].as_str().unwrap())
        .collect();
    holder_pids.sort_unstable();
    assert_eq!(holder_pids, ["h-2", "h-3"]);
    assert_eq!(
        after.store.request("i-3").unwrap().unwrap().status,
        RequestStatus::Issued
    );
    let listed = after.listed("").await;
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().all(|c| c["delivery"] == "DELIVERED"));
    // the one revoked before is still revoked, the new ones are not
    assert_eq!(after.status_in_list(&revoked_jwt).await, Status::Set);
    for jwt in after.delivered(0).iter().chain(&after.delivered(1)) {
        assert_eq!(after.status_in_list(jwt).await, Status::Clear);
    }
}

/// Two issuer processes on one PostgreSQL database (TD-51, TD-57): both
/// outboxes run, and the holder gets the message once; a credential
/// revoked on one is shown revoked by the lists of both.
#[tokio::test]
async fn two_replicas_on_postgres_share_the_outbox_and_the_lists() {
    let Some(url) = databases::postgres_url() else {
        return;
    };
    let setting = DatabaseSetting::Postgres(Box::new(databases::fresh_postgres_settings(&url)));
    let key = SigningKey::generate().unwrap();
    let a = World::with(Setup::on(&setting, &key));
    let b = World::with(Setup::on(&setting, &key));
    a.register(HOLDER, json!({})).await;
    assert_eq!(a.ask("h-1", &["membership-vc20"]).await, "ISSUED");
    tokio::join!(delivery::send_due(&a.issuer), delivery::send_due(&b.issuer));
    let (sent_by_a, sent_by_b) = (a.sender.sent().len(), b.sender.sent().len());
    assert_eq!(sent_by_a + sent_by_b, 1, "sent once");
    let sender = if sent_by_a == 1 { &a } else { &b };
    let jwt = sender.delivered(0)[0].clone();
    let id = Credential::parse(&jwt).unwrap().id.unwrap();
    assert_eq!(a.status_in_list(&jwt).await, Status::Clear);
    assert_eq!(b.revoke(&id).await, StatusCode::NO_CONTENT);
    assert_eq!(a.status_in_list(&jwt).await, Status::Set);
    assert_eq!(b.status_in_list(&jwt).await, Status::Set);
    // the delivery is recorded once, as delivered
    assert_eq!(a.listed("").await[0]["delivery"], "DELIVERED");
}

/// A store whose `request` panics for one request, as a bug in deciding
/// that request would; everything else goes to the real store.
struct PanicsOnRequest {
    inner: Arc<DatabaseStore>,
    issuer_pid: &'static str,
}

impl UsedTokenStore for PanicsOnRequest {
    fn use_token_id(&self, token_id: &str, expires_at: u64, now: u64) -> Result<bool, String> {
        self.inner.use_token_id(token_id, expires_at, now)
    }
}

impl IssuerStore for PanicsOnRequest {
    fn request(&self, issuer_pid: &str) -> Result<Option<CredentialRequest>, StoreError> {
        assert_ne!(issuer_pid, self.issuer_pid, "deciding this request panics");
        self.inner.request(issuer_pid)
    }

    fn add_participant(&self, participant: &Participant) -> Result<bool, StoreError> {
        self.inner.add_participant(participant)
    }
    fn participant(&self, did: &str) -> Result<Option<Participant>, StoreError> {
        self.inner.participant(did)
    }
    fn participants(&self) -> Result<Vec<Participant>, StoreError> {
        self.inner.participants()
    }
    fn delete_participant(&self, did: &str) -> Result<bool, StoreError> {
        self.inner.delete_participant(did)
    }
    fn add_request(&self, request: &CredentialRequest) -> Result<bool, StoreError> {
        self.inner.add_request(request)
    }
    fn received_requests(&self) -> Result<Vec<String>, StoreError> {
        self.inner.received_requests()
    }
    fn reject(&self, issuer_pid: &str) -> Result<bool, StoreError> {
        self.inner.reject(issuer_pid)
    }
    fn issue(
        &self,
        issuer_pid: &str,
        planned: &[Planned],
        rules: IssueRules,
        sign: &mut SignCredentials<'_>,
    ) -> Result<IssueOutcome, StoreError> {
        self.inner.issue(issuer_pid, planned, rules, sign)
    }
    fn due_deliveries(&self, now: u64) -> Result<Vec<Delivery>, StoreError> {
        self.inner.due_deliveries(now)
    }
    fn claim_due_delivery(
        &self,
        now: u64,
        lease: Lease<'_>,
    ) -> Result<Option<Delivery>, StoreError> {
        self.inner.claim_due_delivery(now, lease)
    }
    fn next_delivery_at(&self) -> Result<Option<u64>, StoreError> {
        self.inner.next_delivery_at()
    }
    fn retry_later(
        &self,
        issuer_pid: &str,
        holder: &str,
        attempts: u32,
        at: u64,
    ) -> Result<bool, StoreError> {
        self.inner.retry_later(issuer_pid, holder, attempts, at)
    }
    fn end_delivery(
        &self,
        issuer_pid: &str,
        holder: &str,
        state: DeliveryState,
    ) -> Result<bool, StoreError> {
        self.inner.end_delivery(issuer_pid, holder, state)
    }
    fn issued(&self, holder: Option<&str>) -> Result<Vec<IssuedWithDelivery>, StoreError> {
        self.inner.issued(holder)
    }
    fn revoke(
        &self,
        id: &str,
        now: u64,
        sign: &mut SignStatusList<'_>,
    ) -> Result<Revocation, StoreError> {
        self.inner.revoke(id, now, sign)
    }
    fn status_list(
        &self,
        profile: portitor_dcp::credential::Profile,
    ) -> Result<Option<SignedStatusList>, StoreError> {
        self.inner.status_list(profile)
    }
    fn sign_status_list_if_needed(
        &self,
        profile: portitor_dcp::credential::Profile,
        kid: &str,
        sign: &mut SignStatusList<'_>,
    ) -> Result<bool, StoreError> {
        self.inner.sign_status_list_if_needed(profile, kid, sign)
    }
    fn revoked_indexes(
        &self,
        profile: portitor_dcp::credential::Profile,
    ) -> Result<Vec<u64>, StoreError> {
        self.inner.revoked_indexes(profile)
    }
}

/// One request whose decision panics on every round does not block the
/// outbox: the other waiting request is decided and its credentials are
/// sent; the panicking one stays `RECEIVED`.
#[tokio::test]
async fn a_request_whose_decision_panics_does_not_block_the_outbox() {
    let world = World::with_store_port(Setup::in_memory(), |store| {
        Arc::new(PanicsOnRequest {
            inner: store,
            issuer_pid: "i-1",
        })
    });
    world.register(HOLDER, json!({})).await;
    for (issuer_pid, holder_pid) in [("i-1", "h-1"), ("i-2", "h-2")] {
        let waiting = CredentialRequest {
            issuer_pid: issuer_pid.to_owned(),
            holder_did: HOLDER.to_owned(),
            holder_pid: holder_pid.to_owned(),
            credential_ids: vec!["membership-vc20".to_owned()],
            access_token: Some("holder-access-token".to_owned()),
            status: RequestStatus::Received,
            received_at: now_secs(),
        };
        assert!(world.store.add_request(&waiting).unwrap());
    }

    // i-1 comes first and panics; i-2 is still decided and sent
    let outbox = tokio::spawn(delivery::run_outbox(Arc::clone(&world.issuer)));
    for _ in 0..500 {
        if !world.sender.sent().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!outbox.is_finished(), "the outbox ended");
    outbox.abort();
    let sent = world.sender.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].body["holderPid"], "h-2");
    let status = |issuer_pid| world.store.request(issuer_pid).unwrap().unwrap().status;
    assert_eq!(status("i-1"), RequestStatus::Received);
    assert_eq!(status("i-2"), RequestStatus::Issued);
}

/// A key change with the old public key listed as retired
/// (`PORTITOR_ISSUER_RETIRED_KEY_FILES`, TD-05): the credentials and
/// status lists the old key signed keep verifying while it is listed;
/// the new key signs the lists from the restart on; once the old key is
/// removed from the list, what it signed no longer verifies.
#[tokio::test]
async fn a_key_change_with_the_old_key_retired_leaves_no_gap() {
    let dir = TempDir::new("issuer-test");
    let path = dir.path().join("issuer.db");
    let old_key = SigningKey::generate().unwrap();
    let new_key = SigningKey::generate().unwrap();

    // before the change: one credential per profile, and both status
    // lists, signed with the old key
    let before = World::with(Setup::on_file(&path, &old_key));
    let old_kid = before.issuer.kid().to_owned();
    before.register(HOLDER, json!({})).await;
    assert_eq!(
        before
            .ask("h-1", &["membership-vc11", "membership-vc20"])
            .await,
        "ISSUED"
    );
    delivery::send_due(&before.issuer).await;
    let credentials = before.delivered(0);
    assert_eq!(credentials.len(), 2);
    let mut old_signed = credentials.clone();
    for name in ["vc11", "vc20"] {
        old_signed.push(before.status_list(name).await);
    }
    for jwt in &old_signed {
        assert_eq!(Jws::parse(jwt).unwrap().kid(), Some(old_kid.as_str()));
    }
    drop(before);

    // after: the new key signs, the old public key is listed as retired
    let old_public = p256_public_key_pem(&old_key.public_jwk()).unwrap();
    let retired_file = dir.write("old-key.pub.pem", &old_public);
    let after = World::with(Setup {
        retired_key_files: vec![retired_file],
        ..Setup::on_file(&path, &new_key)
    });
    assert_ne!(after.issuer.kid(), old_kid);
    let document = after.did_document().await;
    for jwt in &old_signed {
        let credential = Credential::parse(jwt).unwrap();
        credential.verify(&document, now_secs(), 5).unwrap();
    }
    // the lists are signed again at start, with the new key; a credential
    // of the old key is checked against them, and can still be revoked
    for name in ["vc11", "vc20"] {
        let list = after.status_list(name).await;
        assert_eq!(Jws::parse(&list).unwrap().kid(), Some(after.issuer.kid()));
    }
    let revoked_id = Credential::parse(&credentials[0]).unwrap().id.unwrap();
    assert_eq!(after.revoke(&revoked_id).await, StatusCode::NO_CONTENT);
    assert_eq!(after.status_in_list(&credentials[0]).await, Status::Set);
    assert_eq!(after.status_in_list(&credentials[1]).await, Status::Clear);
    drop(after);

    // the operator removes the old key from the list: what it signed no
    // longer verifies; the lists the new key signed still do
    let removed = World::with(Setup::on_file(&path, &new_key));
    let document = removed.did_document().await;
    assert_eq!(document.verification_method.len(), 1);
    for jwt in &old_signed {
        let credential = Credential::parse(jwt).unwrap();
        assert!(credential.verify(&document, now_secs(), 5).is_err());
    }
    assert_eq!(removed.status_in_list(&credentials[0]).await, Status::Set);
}

#[test]
fn the_issuer_does_not_start_with_its_own_key_as_retired() {
    let dir = TempDir::new("issuer-test");
    let key = SigningKey::generate().unwrap();
    let current_public = p256_public_key_pem(&key.public_jwk()).unwrap();
    let current_file = dir.write("current.pub.pem", &current_public);
    let config = Config {
        participant_id: ISSUER.to_owned(),
        key_file: "unused".into(),
        retired_key_files: vec![current_file],
        credential_definitions_file: "unused".into(),
        issuer_service_url: SERVICE.to_owned(),
        bind: "unused".to_owned(),
        database: portitor_runtime::database::DatabaseSetting::SqliteFile("unused".into()),
        management: portitor_issuer::config::ManagementConfig::health_only(
            portitor_issuer::config::DEFAULT_MANAGEMENT_BIND,
        ),
        allow_http: false,
        max_active_per_holder: 10,
        bundle_key_file: None,
        bundle_dir: None,
    };
    let ports = Ports {
        store: Arc::new(DatabaseStore::in_memory().unwrap()),
        directory: Arc::new(FakeDirectory::default()),
        sender: Arc::new(FakeSender::default()),
    };
    let error = Issuer::new(&config, key, Vec::new(), ports).err().unwrap();
    assert!(
        error.starts_with("PORTITOR_ISSUER_RETIRED_KEY_FILES: "),
        "{error}"
    );
    assert!(error.contains("is the current key"), "{error}");
}

#[tokio::test]
async fn participants_are_managed_with_the_api_key() {
    let world = World::new();
    let (status, _) = call(
        &world.management,
        Request::get("/v1/participants")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &world.management,
        Request::get("/v1/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        world.register(HOLDER, json!({"role": "consumer"})).await,
        StatusCode::CREATED
    );
    assert_eq!(
        world.register(HOLDER, json!({})).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        world.register("did:key:z6Mk", json!({})).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        world.register(STRANGER, json!({"id": "x"})).await,
        StatusCode::BAD_REQUEST
    );
    let (_, list) = call(
        &world.management,
        authed(Request::get("/v1/participants"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(list[0]["did"], HOLDER);
    assert_eq!(list[0]["claims"]["role"], "consumer");

    // a DID with a port, percent-encoded as a whole in the path
    let ported = "did:web:localhost%3A8090";
    assert_eq!(world.register(ported, json!({})).await, StatusCode::CREATED);
    let (status, _) = call(
        &world.management,
        authed(Request::delete(
            "/v1/participants/did%3Aweb%3Alocalhost%253A8090",
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &world.management,
        authed(Request::delete(
            "/v1/participants/did%3Aweb%3Alocalhost%253A8090",
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn offers_go_to_registered_participants_only() {
    let world = World::new();
    let offer = |did: &str, ids: Value| {
        authed(Request::post("/v1/offers"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"participantDid": did, "credentialIds": ids, "offerReason": "reissue"})
                    .to_string(),
            ))
            .unwrap()
    };
    let (status, _) = call(&world.management, offer(HOLDER, json!(["membership-vc20"]))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    world.register(HOLDER, json!({})).await;
    let (status, _) = call(&world.management, offer(HOLDER, json!(["nope"]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&world.management, offer(HOLDER, json!(["membership-vc20"]))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let sent = world.sender.sent();
    assert_eq!(sent[0].url, "https://holder.example/cs/offers");
    assert_eq!(sent[0].body["type"], "CredentialOfferMessage");
    assert_eq!(sent[0].body["issuer"], ISSUER);
    assert_eq!(sent[0].body["credentials"][0]["id"], "membership-vc20");
    assert_eq!(sent[0].body["credentials"][0]["offerReason"], "reissue");
    assert_eq!(sent[0].body["credentials"][0]["profile"], "vc20-bssl/jwt");
    // the offer passes DCP's credential-offer-message-schema.json (6.6.1)
    let errors = dcp_schema_errors("credential-offer-message-schema.json", &sent[0].body);
    assert!(errors.is_empty(), "{errors:?}");

    world.sender.fail_with(Some("HTTP 503 Service Unavailable"));
    let (status, _) = call(&world.management, offer(HOLDER, json!(null))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

/// An offer sent without a reason has no `offerReason`: the `reissue` of
/// the metadata (there because the metadata schema requires it) is not
/// copied into it. The offer still passes its DCP schema.
#[tokio::test]
async fn an_offer_without_a_reason_does_not_take_the_metadata_reason() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    let request = authed(Request::post("/v1/offers"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"participantDid": HOLDER}).to_string()))
        .unwrap();
    let (status, _) = call(&world.management, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let sent = world.sender.sent();
    let offered = sent[0].body["credentials"].as_array().unwrap();
    assert_eq!(offered.len(), 2);
    for object in offered {
        assert!(object.get("offerReason").is_none(), "{object}");
        assert_eq!(
            object["credentialSchema"],
            "https://example.com/membership-credential-schema.json"
        );
    }
    let errors = dcp_schema_errors("credential-offer-message-schema.json", &sent[0].body);
    assert!(errors.is_empty(), "{errors:?}");
}

/// `docs/api/issuer-management-openapi.json` is what the code generates.
/// After changing the management API, write it again with
/// `cargo test -p portitor-issuer --test issuer -- --ignored
/// write_management_description`.
#[test]
fn published_management_description_is_current() {
    let published = include_str!("../../../docs/api/issuer-management-openapi.json");
    let generated = format!("{}\n", openapi_json());
    assert!(
        published == generated,
        "docs/api/issuer-management-openapi.json is out of date; see this test's doc comment"
    );
    let spec: Value = serde_json::from_str(published).unwrap();
    assert!(spec["openapi"].as_str().unwrap().starts_with("3.1"));
}

/// Every operation in docs/api/issuer-openapi.json is routed (the bundle
/// routes with an empty bundle folder). Paths with their own `servers`
/// entry sit under the issuer service URL, which is
/// `https://issuer.example/issuer` here.
#[tokio::test]
async fn openapi_paths_are_served() {
    let raw = include_str!("../../../docs/api/issuer-openapi.json");
    let spec: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(spec["info"]["version"], env!("CARGO_PKG_VERSION"));
    let bundles = std::env::temp_dir().join(format!("portitor-issuer-openapi-{}", new_uuid()));
    std::fs::create_dir_all(&bundles).unwrap();
    let world = World::with(Setup {
        bundle_dir: Some(bundles),
        ..Setup::in_memory()
    });
    let mut count = 0;
    for (path, item) in spec["paths"].as_object().unwrap() {
        let path = path
            .replace("{issuerPid}", "unknown")
            .replace("{name}", "vc11")
            .replace("{version}", "unknown")
            .replace("{path}", "manifest.json");
        let uri = if item.get("servers").is_some() {
            format!("/issuer{path}")
        } else {
            path.clone()
        };
        for method in ["get", "post", "put", "delete"] {
            if item.get(method).is_none() {
                continue;
            }
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&uri)
                .body(Body::empty())
                .unwrap();
            let response = world.app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
            assert!(
                status != StatusCode::NOT_FOUND || !bytes.is_empty(),
                "{method} {uri} is not routed"
            );
            count += 1;
        }
    }
    assert_eq!(count, 8);
}

/// Not a check: writes `docs/api/issuer-management-openapi.json` when run
/// by name (see above).
#[test]
#[ignore = "writes the file"]
fn write_management_description() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/api/issuer-management-openapi.json"
        ),
        format!("{}\n", openapi_json()),
    )
    .unwrap();
}

/// The health checks on the management port (ADR 0023): open, readiness
/// on the issuer's own file, key and status lists, and the management API
/// still behind its key.
#[tokio::test]
async fn health_checks_its_own_dependencies_without_the_key() {
    use portitor_runtime::health::management_app;
    let dir = TempDir::new("issuer-test");
    let path = dir.path().join("issuer.db");
    let world = World::with(Setup::on_file(&path, &SigningKey::generate().unwrap()));
    let checks = Arc::new(portitor_issuer::health::readiness_checks(
        &world.issuer,
        DatabaseSetting::SqliteFile(path.clone()),
        Arc::clone(world.store.database()),
    ));
    let full = management_app(Arc::clone(&checks), Some(world.management.clone()));
    let get = |path: &str| Request::get(path).body(Body::empty()).unwrap();
    let (status, body) = call(&full, get("/api/check/readiness")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"isSystemHealthy": true, "componentResults": [
            {"component": "database", "isHealthy": true, "failure": null},
            {"component": "key", "isHealthy": true, "failure": null},
            {"component": "status-lists", "isHealthy": true, "failure": null}
        ]})
    );
    // the checks need no key; the management API still does
    assert_eq!(
        call(&full, get("/api/check/liveness")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&full, get("/v1/participants")).await.0,
        StatusCode::UNAUTHORIZED
    );
    // without the API: the checks only
    let checks_only = management_app(Arc::clone(&checks), None);
    assert_eq!(
        call(&checks_only, get("/api/check/startup")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&checks_only, get("/v1/participants")).await.0,
        StatusCode::NOT_FOUND
    );
    // the file is gone: not ready, and the answer says which check failed
    std::fs::remove_dir_all(dir.path()).unwrap();
    let (status, body) = call(&checks_only, get("/api/check/health")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["isSystemHealthy"], false);
    assert_eq!(body["componentResults"][0]["component"], "database");
    assert_eq!(body["componentResults"][0]["isHealthy"], false);
    assert_eq!(body["componentResults"][2]["isHealthy"], true);
}

/// `portitor-issuer health x` and `portitor-issuer x` are usage errors: exit code 2, before
/// anything starts.
#[test]
fn unknown_arguments_exit_with_2() {
    for args in [&["health", "x"][..], &["x"][..]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_portitor-issuer"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("takes no arguments"), "{stderr}");
    }
}

/// A TEST ONLY P-256 key, for starting the binary.
const TEST_KEY: &str =
    include_str!("../../portitor-identity/tests/fixtures/test-only-identity-key.pem");

/// Starts the `portitor-issuer` binary with `definitions` as its
/// definitions file and `more_settings` in its environment, and waits
/// until it stops: whether it ended well, and what it logged. Every
/// start here is one that must stop before the issuer listens; a process
/// that is still there after 30 seconds is stopped and fails the test.
fn start_and_wait_for_the_stop(
    definitions: &str,
    more_settings: &[(&str, &str)],
) -> (bool, String) {
    let dir = TempDir::new("issuer-start");
    let key_file = dir.write("test-only-key.pem", TEST_KEY);
    let definitions_file = dir.write("credentials.json", definitions);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_portitor-issuer"))
        // nothing of the test machine's own settings
        .env_clear()
        .env("PORTITOR_PARTICIPANT_ID", ISSUER)
        .env(
            "PORTITOR_ISSUER_SERVICE_URL",
            "https://issuer.example/issuer",
        )
        .env("PORTITOR_ISSUER_KEY_FILE", key_file)
        .env(
            "PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE",
            definitions_file,
        )
        .env("PORTITOR_DB_FILE", dir.path().join("issuer.db"))
        .env("PORTITOR_BIND", "127.0.0.1:0")
        .envs(more_settings.iter().copied())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(30) {
            child.kill().unwrap();
            panic!("the issuer started instead of stopping");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    let mut logged = String::from_utf8_lossy(&output.stdout).into_owned();
    logged.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), logged)
}

/// From 0.12 every definition needs a `credentialSchema` (TD-33; DCP
/// v1.0.1, section 6.7). A definitions file without one, as written for
/// 0.11, or with one that is not an absolute URL as text, stops the
/// issuer at start, and the log names the setting, the entry and the
/// field.
#[test]
fn the_issuer_does_not_start_without_a_valid_credential_schema() {
    let entry = |credential_schema: &str| {
        format!(
            r#"[{{"id": "membership-vc20", "credentialType": "MembershipCredential",
                 "profile": "vc20-bssl/jwt", "validityDays": 365{credential_schema}}}]"#
        )
    };
    let required = "\\\"membership-vc20\\\": credentialSchema is required";
    let not_a_url = "\\\"membership-vc20\\\": credentialSchema must be an absolute http";
    for (credential_schema, reason) in [
        ("", required),
        (r#", "credentialSchema": null"#, required),
        (r#", "credentialSchema": """#, not_a_url),
        (r#", "credentialSchema": "membership.json""#, not_a_url),
        (
            r#", "credentialSchema": ["https://example.com/m.json"]"#,
            "invalid type",
        ),
    ] {
        let (ended_well, logged) = start_and_wait_for_the_stop(&entry(credential_schema), &[]);
        assert!(!ended_well, "{credential_schema}: {logged}");
        assert!(
            logged.contains("PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE"),
            "{credential_schema}: {logged}"
        );
        assert!(logged.contains(reason), "{credential_schema}: {logged}");
    }
}

/// The issuer reads `PORTITOR_DID_CACHE_SECONDS` at start (it resolves
/// the holders' DIDs): a value out of bounds stops it, and the log names
/// the setting.
#[test]
fn the_issuer_does_not_start_with_a_did_cache_time_out_of_bounds() {
    for value in ["3601", "-1", "5m"] {
        let settings = [("PORTITOR_DID_CACHE_SECONDS", value)];
        let (ended_well, logged) = start_and_wait_for_the_stop(DEFINITIONS, &settings);
        assert!(!ended_well, "{value}: {logged}");
        assert!(
            logged.contains("PORTITOR_DID_CACHE_SECONDS has an invalid value"),
            "{value}: {logged}"
        );
    }
}

/// Request size limits (R-18): a body one byte over the limit gets 413,
/// a body at the limit is not refused for its size. A credential request
/// has the fixed limit of small messages, or the public port's limit
/// when that is lower.
#[tokio::test]
async fn request_bodies_over_the_limits_get_413() {
    use portitor_runtime::request_size::testing::assert_size_limit;
    use portitor_runtime::request_size::{
        MANAGEMENT_SETTING, PUBLIC_SETTING, RequestLimits, SMALL_MESSAGE_MAX_BYTES,
    };
    use portitor_runtime::testing::vars;
    // the limits as `main.rs` reads and applies them
    let limits = |public: &str| {
        let settings = vars(&[(PUBLIC_SETTING, public), (MANAGEMENT_SETTING, "100000")]);
        RequestLimits::from_vars(settings).unwrap()
    };
    let world = World::new();
    let credentials =
        || Request::post("/issuer/credentials").header(header::CONTENT_TYPE, "application/json");
    let public = limits("").on_public_port(world.app.clone());
    assert_size_limit(&public, credentials, SMALL_MESSAGE_MAX_BYTES).await;
    let public = limits("1000").on_public_port(world.app.clone());
    assert_size_limit(&public, credentials, 1_000).await;

    let management = limits("").on_management_port(world.management.clone());
    let participants = || {
        authed(Request::post("/v1/participants")).header(header::CONTENT_TYPE, "application/json")
    };
    assert_size_limit(&management, participants, 100_000).await;
}

/// Every route of the management API, with a body it accepts where it
/// reads one.
fn every_management_call() -> Vec<Call> {
    let participant = json!({"did": "did:web:new.example", "claims": {"role": "consumer"}});
    let offer = json!({
        "participantDid": HOLDER,
        "credentialIds": ["membership-vc20"],
        "offerReason": "reissue"
    });
    vec![
        Call::post("/v1/participants", participant),
        Call::without_body("GET", "/v1/participants"),
        Call::without_body("DELETE", "/v1/participants/did:web:nobody.example"),
        Call::post("/v1/offers", offer),
        Call::without_body("GET", "/v1/credentials"),
        Call::without_body("POST", "/v1/credentials/urn:uuid:1/revoke"),
    ]
}

#[tokio::test]
async fn every_management_route_answers_401_without_the_key_and_never_403() {
    let world = World::new();
    let answers = without_the_key(&world.management, &every_management_call(), API_KEY).await;
    assert_eq!(answers.len(), 6 * 6);
    assert_eq!(statuses_other_than(&[401], &answers), "");
}

/// What is not answered 400 today. An `offerReason` is taken whatever
/// it is, empty or 100 000 letters long, and sent to the holder in the
/// offer (204). A `participantDid` that is no DID is looked up and not
/// found (404).
const WRONG_BODIES_NOT_ANSWERED_400: &str = "\
POST /v1/offers, offerReason is empty: 204
POST /v1/offers, offerReason is only spaces: 204
POST /v1/offers, offerReason is very long: 204
POST /v1/offers, participantDid is empty: 404
POST /v1/offers, participantDid is only spaces: 404
POST /v1/offers, participantDid is very long: 404
";

#[tokio::test]
async fn every_management_route_refuses_a_wrong_body_with_400() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    let answers = with_a_wrong_body(&world.management, &every_management_call(), API_KEY).await;
    assert!(answers.len() > 40, "{}", answers.len());
    assert_eq!(
        statuses_other_than(&[400], &answers),
        WRONG_BODIES_NOT_ANSWERED_400
    );
}

#[tokio::test]
async fn odd_management_paths_and_queries_are_never_an_error_of_the_server() {
    let world = World::new();
    world.register(HOLDER, json!({})).await;
    let answers =
        with_odd_paths_and_queries(&world.management, &every_management_call(), API_KEY).await;
    assert_eq!(answers.len(), 6 * 14);
    // 409: the good body of a new participant, sent a second time
    assert_eq!(
        statuses_other_than(&[200, 201, 204, 400, 404, 409], &answers),
        ""
    );
}
