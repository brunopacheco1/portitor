// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider can always end access (ADR 0027, sections 5 and 7), and a
//! partner without membership can still end a transfer it shares with us
//! (TD-47; ADR 0016, amended in 1.1).
//!
//! A connector as provider with the built-in data plane and a local
//! upstream, the management API, and the test verifier whose membership
//! a test can revoke ([`common::Revocable`]). The consumer is played by
//! the test: its messages are posted to the router, ours are captured by
//! a fake sender, which acknowledges each one.
#![cfg(feature = "data-plane")]
#![allow(clippy::unwrap_used, missing_docs)]

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use hyper::body::{Bytes, Frame};
use portitor_connector::harvest::FixedSource;
use portitor_connector::http_client::Sender;
use portitor_connector::management::ApiKey;
use portitor_connector::ports::Shared;
use portitor_connector::store::ConnectorStore;
use portitor_connector::{Config, Ports};
use portitor_dsp::negotiation::Agreement;
use portitor_dsp::negotiation::provider::Negotiation;
use portitor_dsp::transfer::TransferState;
use portitor_dsp::transfer::provider::{Agreements, FlowStep, Lease, StoreError, TransferStore};
use portitor_runtime::BoxFuture;
use portitor_runtime::database::DatabaseSetting;
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::{CONSUMER, PROVIDER, Revocable, from};

const BASE: &str = "/dsp/2025-1/transfers";
const CALLBACK: &str = "https://consumer.example/cb";
const CONSUMER_PID: &str = "urn:uuid:consumer-1";
const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";
const STRANGER: &str = "did:web:stranger.example";

/// Records every message we send and acknowledges it.
#[derive(Default)]
struct FakeSender(Mutex<Vec<(String, Value)>>);

impl Sender for FakeSender {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.0.lock().unwrap().push((url.to_owned(), body.clone()));
        Box::pin(async { Ok(Value::Null) })
    }
}

impl FakeSender {
    /// The messages sent so far, once there are `n` (they go out in the
    /// background).
    async fn sent(&self, n: usize) -> Vec<(String, Value)> {
        for _ in 0..400 {
            if self.0.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.0.lock().unwrap().clone()
    }
}

/// One finalized agreement, `a1`, for dataset `CAT0101`, held by the
/// consumer.
struct OneAgreement;

impl Agreements for OneAgreement {
    fn finalized(&self, id: &str) -> Result<Option<Agreement>, StoreError> {
        let agreement = json!({
            "@id": "a1", "@type": "Agreement", "target": "CAT0101",
            "assigner": PROVIDER, "assignee": CONSUMER,
            "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
        });
        Ok((id == "a1").then(|| serde_json::from_value(agreement).unwrap()))
    }
    fn negotiation_of(&self, _: &str) -> Result<Option<Negotiation>, StoreError> {
        Ok(None)
    }
    fn replace_negotiation_if(&self, _: &Negotiation, _: u64) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// A body that sends a line every 10 ms and never ends: a long download.
struct Drip(tokio::time::Interval);

impl hyper::body::Body for Drip {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.get_mut().0.poll_tick(context) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"row\n"))))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Serves `hello` at `/file` and a download that never ends at `/drip`,
/// on a free local port; returns the base URL.
async fn upstream() -> String {
    let app = Router::new()
        .route(
            "/file",
            axum::routing::get(|| async { ([("content-type", "text/csv")], "hello") }),
        )
        .route(
            "/drip",
            axum::routing::get(|| async {
                let every = tokio::time::interval(Duration::from_millis(10));
                Body::new(Drip(every))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// A provider under test: its DSP and data routes, its management API,
/// what it sent, and the switch that revokes the consumer's membership.
struct Provider {
    app: Router,
    management: Router,
    negotiator: portitor_connector::Negotiator,
    sender: Arc<FakeSender>,
    revoked: Arc<AtomicBool>,
}

fn config(token_seconds: u64) -> Config {
    Config {
        participant_id: PROVIDER.into(),
        public_url: "https://provider.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http: false,
        database: DatabaseSetting::SqliteFile("unused.db".into()),
        data_token_seconds: token_seconds,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// A provider whose dataset `CAT0101` has `source` as its data, whose
/// running downloads check the membership every `recheck`, with tokens
/// valid `token_seconds`, keeping everything in `store`.
fn provider_with(
    source: &str,
    recheck: Duration,
    token_seconds: u64,
    store: &Arc<ConnectorStore>,
) -> Provider {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");
    let mut entries: Vec<Value> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    for entry in entries.iter_mut().filter(|entry| entry["@id"] == "CAT0101") {
        entry["portitor:source"] = json!(source);
    }
    let sender = Arc::new(FakeSender::default());
    let ports = Ports::new(Arc::clone(&sender) as Arc<dyn Sender>, store);
    // the prover names us in our own messages; the verifier is replaced
    let (mut ports, _) = common::recording(ports, PROVIDER);
    let (verifier, revoked) = Revocable::new(recheck);
    ports.verifier = Arc::new(verifier);
    ports.transfers.agreements = Shared(Arc::new(OneAgreement));
    ports.harvest.source = Some(Arc::new(FixedSource::new(entries)));
    let (app, negotiator) = portitor_connector::build(&config(token_seconds), ports).unwrap();
    let management =
        portitor_connector::management::router(negotiator.clone(), ApiKey::new(KEY).unwrap());
    Provider {
        app,
        management,
        negotiator,
        sender,
        revoked,
    }
}

fn provider(source: &str, recheck: Duration) -> Provider {
    let store = Arc::new(ConnectorStore::in_memory().unwrap());
    provider_with(source, recheck, 3_600, &store)
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

/// A DSP message `kind` about transfer `pid`, posted by `did`.
fn message_from(did: &str, kind: &str, pid: &str, path: &str) -> Request<Body> {
    let body = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": kind, "providerPid": pid, "consumerPid": CONSUMER_PID
    });
    from(Request::post(format!("{BASE}/{pid}/{path}")), did)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// A management call, with the key.
fn manage(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .body(Body::empty())
        .unwrap()
}

impl Provider {
    /// The consumer asks for a transfer of `CAT0101`; returns the provider
    /// pid, the data path and the token, once the transfer is STARTED.
    async fn started_transfer(&self) -> (String, String, String) {
        let body = json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "TransferRequestMessage", "consumerPid": CONSUMER_PID,
            "agreementId": "a1", "format": "HttpData-PULL", "callbackAddress": CALLBACK
        });
        let request = from(Request::post(format!("{BASE}/request")), CONSUMER)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let (status, ack) = call(&self.app, request).await;
        assert_eq!(status, StatusCode::CREATED, "{ack}");
        let pid = ack["providerPid"].as_str().unwrap().to_owned();
        let address = self.sender.sent(1).await[0].1["dataAddress"].clone();
        let endpoint = address["endpoint"].as_str().unwrap();
        let path = endpoint
            .strip_prefix("https://provider.example")
            .unwrap()
            .to_owned();
        let token = address["endpointProperties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "authorization")
            .map(|p| p["value"].as_str().unwrap().to_owned())
            .unwrap();
        self.wait_for(&pid, "STARTED").await;
        (pid, path, token)
    }

    /// The transfer as the management API shows it.
    async fn view(&self, pid: &str) -> (StatusCode, Value) {
        let uri = format!("/v1/provided-transfers/{pid}");
        call(&self.management, manage("GET", &uri)).await
    }

    async fn wait_for(&self, pid: &str, state: &str) {
        for _ in 0..400 {
            if self.view(pid).await.1["state"] == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{pid} never reached {state}: {}", self.view(pid).await.1);
    }

    /// Starts a download of `path` with `token`: its status and its body,
    /// still streaming.
    async fn open(&self, path: &str, token: &str) -> (StatusCode, Body) {
        let request = Request::get(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        (response.status(), response.into_body())
    }

    /// A whole download of `path` with `token`: its status and its body.
    async fn fetch(&self, path: &str, token: &str) -> (StatusCode, String) {
        let (status, body) = self.open(path, token).await;
        let bytes = to_bytes(body, 1 << 20).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
    }

    /// Our termination of the transfer, once sent.
    async fn termination_sent(&self) -> Value {
        for _ in 0..400 {
            let sent = self.sender.0.lock().unwrap().clone();
            let found = sent.iter().find(|(url, _)| url.ends_with("/termination"));
            if let Some((url, body)) = found {
                assert_eq!(
                    url,
                    &format!("{CALLBACK}/transfers/{CONSUMER_PID}/termination")
                );
                return body.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("no termination was sent");
    }
}

/// Reads `body` until it fails, for at most `limit`. `true` when it
/// failed (the download was cut), `false` when it ended or went on.
async fn is_cut_within(body: &mut Body, limit: Duration) -> bool {
    let reading = async {
        loop {
            match body.frame().await {
                Some(Ok(_)) => {}
                Some(Err(_)) => return true,
                None => return false,
            }
        }
    };
    tokio::time::timeout(limit, reading).await.unwrap_or(false)
}

/// ADR 0027, section 7.2: the next request after a revocation is refused
/// with the same 401 as any other refusal, and the transfer is ended: the
/// provider sends the DSP termination with the reason.
#[tokio::test]
async fn a_revoked_consumer_is_refused_on_its_next_request_and_its_transfer_ends() {
    let p = provider(
        &format!("{}/file", upstream().await),
        Duration::from_secs(60),
    );
    let (pid, path, token) = p.started_transfer().await;
    assert_eq!(
        p.fetch(&path, &token).await,
        (StatusCode::OK, "hello".to_owned())
    );

    p.revoke();
    let (status, body) = p.fetch(&path, &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, "", "no reason for the caller");
    let termination = p.termination_sent().await;
    assert_eq!(termination["@type"], "TransferTerminationMessage");
    let reason = termination["reason"][0].as_str().unwrap();
    assert!(reason.contains("membership no longer holds"), "{reason}");
    p.wait_for(&pid, "TERMINATED").await;
    assert_eq!(p.fetch(&path, &token).await.0, StatusCode::UNAUTHORIZED);
}

/// A download that runs when the membership is revoked is cut at the next
/// check (here every 50 ms), and the transfer ends.
#[tokio::test]
async fn a_revocation_cuts_a_running_download() {
    let p = provider(
        &format!("{}/drip", upstream().await),
        Duration::from_millis(50),
    );
    let (pid, path, token) = p.started_transfer().await;
    let (status, mut body) = p.open(&path, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !is_cut_within(&mut body, Duration::from_millis(200)).await,
        "a member's download goes on"
    );

    p.revoke();
    assert!(is_cut_within(&mut body, Duration::from_secs(5)).await);
    p.termination_sent().await;
    p.wait_for(&pid, "TERMINATED").await;
}

/// ADR 0027, section 7.1: the provider's operator ends a transfer. Access
/// ends at once: the running download is cut and the next request is
/// refused; the termination goes to the consumer. The view never shows
/// the token or its hash.
#[tokio::test]
async fn the_providers_termination_cuts_a_running_download() {
    let p = provider(
        &format!("{}/drip", upstream().await),
        Duration::from_secs(60),
    );
    let (pid, path, token) = p.started_transfer().await;
    let (status, mut body) = p.open(&path, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.frame().await.unwrap().is_ok());

    let uri = format!("/v1/provided-transfers/{pid}/termination");
    let (status, view) = call(&p.management, manage("POST", &uri)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{view}");
    assert_eq!(view["providerPid"], pid.as_str());
    assert_eq!(view["counterPartyId"], CONSUMER);
    assert!(is_cut_within(&mut body, Duration::from_secs(2)).await);
    assert_eq!(p.open(&path, &token).await.0, StatusCode::UNAUTHORIZED);
    let termination = p.termination_sent().await;
    assert_eq!(termination["reason"][0], "terminated by the provider");
    p.wait_for(&pid, "TERMINATED").await;

    // an ended transfer cannot be ended again; an unknown one is not found
    let (status, _) = call(&p.management, manage("POST", &uri)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let unknown = "/v1/provided-transfers/urn:uuid:unknown/termination";
    let (status, _) = call(&p.management, manage("POST", unknown)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // the list holds it; no token, no hash anywhere
    let (status, page) = call(&p.management, manage("GET", "/v1/provided-transfers")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["transfers"][0]["providerPid"], pid.as_str());
    assert_eq!(page["transfers"][0]["state"], "TERMINATED");
    let hash = portitor_dcp::bundle::sha256_hex(token.as_bytes());
    for shown in [page.to_string(), p.view(&pid).await.1.to_string()] {
        assert!(!shown.contains(&token) && !shown.contains(&hash), "{shown}");
        assert!(!shown.to_lowercase().contains("token"), "{shown}");
    }
}

/// ADR 0027, section 5: a suspension by the consumer cuts its own running
/// download too; a download started again later runs.
#[tokio::test]
async fn a_suspension_cuts_a_running_download() {
    let p = provider(
        &format!("{}/drip", upstream().await),
        Duration::from_secs(60),
    );
    let (pid, path, token) = p.started_transfer().await;
    let (_, mut body) = p.open(&path, &token).await;
    assert!(body.frame().await.unwrap().is_ok());
    let request = message_from(CONSUMER, "TransferSuspensionMessage", &pid, "suspension");
    assert_eq!(call(&p.app, request).await.0, StatusCode::OK);
    assert!(is_cut_within(&mut body, Duration::from_secs(2)).await);

    let request = message_from(CONSUMER, "TransferStartMessage", &pid, "start");
    assert_eq!(call(&p.app, request).await.0, StatusCode::OK);
    let (status, mut body) = p.open(&path, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!is_cut_within(&mut body, Duration::from_millis(200)).await);
}

/// A flow started before 1.1 has no membership kept (the migration marks
/// it). It is served as in 1.0, also after a revocation, until its token
/// expires; then it is refused.
#[tokio::test]
async fn a_flow_from_before_1_1_is_served_until_its_token_expires() {
    let folder = portitor_runtime::testing::TempDir::new("flow-1-0");
    let file = folder.path().join("portitor.db");
    let store = Arc::new(ConnectorStore::open_sqlite_file(&file).unwrap());
    let source = format!("{}/file", upstream().await);
    let p = provider_with(&source, Duration::from_secs(60), 2, &store);
    let (pid, path, token) = p.started_transfer().await;
    // what migration 12 writes for a flow of 1.0
    let raw = rusqlite::Connection::open(&file).unwrap();
    let marked = raw
        .execute(
            "UPDATE flow_membership SET memberships = NULL WHERE flow_id = ?1",
            [&pid],
        )
        .unwrap();
    assert_eq!(marked, 1);

    p.revoke();
    assert_eq!(p.fetch(&path, &token).await.0, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    assert_eq!(p.fetch(&path, &token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(p.view(&pid).await.1["state"], "STARTED", "not ended");
}

/// A flow of 1.1 whose memberships were not kept is refused (fail
/// closed), and ended.
#[tokio::test]
async fn a_flow_without_a_kept_membership_is_refused() {
    let folder = portitor_runtime::testing::TempDir::new("flow-no-membership");
    let file = folder.path().join("portitor.db");
    let store = Arc::new(ConnectorStore::open_sqlite_file(&file).unwrap());
    let source = format!("{}/file", upstream().await);
    let p = provider_with(&source, Duration::from_secs(60), 3_600, &store);
    let (_, path, token) = p.started_transfer().await;
    let raw = rusqlite::Connection::open(&file).unwrap();
    raw.execute("DELETE FROM flow_membership", []).unwrap();
    assert_eq!(p.fetch(&path, &token).await.0, StatusCode::UNAUTHORIZED);
    p.termination_sent().await;
}

/// Log lines collected on the test's thread, as JSON.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    /// Collects until the guard is dropped.
    fn start() -> (Self, tracing::subscriber::DefaultGuard) {
        portitor_runtime::telemetry::testing::keep_a_second_subscriber();
        let log = Self::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        (log, tracing::subscriber::set_default(subscriber))
    }

    /// The log lines with `message`.
    fn with_message(&self, message: &str) -> Vec<Value> {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|line| line["fields"]["message"] == message)
            .collect()
    }
}

const ACCEPTED_WITHOUT_MEMBERSHIP: &str =
    "DSP termination accepted from a partner without membership";

/// TD-47, provider's side: a consumer whose membership was revoked may
/// still end its own transfer; every other message of it is refused
/// (404), and so is a termination of a transfer that is not its own.
#[tokio::test]
async fn a_consumer_without_membership_may_still_end_its_transfer() {
    let p = provider(
        &format!("{}/file", upstream().await),
        Duration::from_secs(60),
    );
    let (pid, _, _) = p.started_transfer().await;
    let (log, _guard) = Log::start();
    p.revoke();

    for (kind, path) in [
        ("TransferSuspensionMessage", "suspension"),
        ("TransferCompletionMessage", "completion"),
        ("TransferStartMessage", "start"),
    ] {
        let (status, _) = call(&p.app, message_from(CONSUMER, kind, &pid, path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let get = from(Request::get(format!("{BASE}/{pid}")), CONSUMER)
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&p.app, get).await.0, StatusCode::NOT_FOUND);
    // another participant without membership: not its transfer
    let by_stranger = message_from(STRANGER, "TransferTerminationMessage", &pid, "termination");
    assert_eq!(call(&p.app, by_stranger).await.0, StatusCode::NOT_FOUND);
    assert!(log.with_message(ACCEPTED_WITHOUT_MEMBERSHIP).is_empty());

    let ending = message_from(CONSUMER, "TransferTerminationMessage", &pid, "termination");
    let (status, ack) = call(&p.app, ending).await;
    assert_eq!(status, StatusCode::OK, "{ack}");
    assert_eq!(ack["state"], "TERMINATED");
    let lines = log.with_message(ACCEPTED_WITHOUT_MEMBERSHIP);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["fields"]["process"], pid.as_str());
    assert_eq!(lines[0]["fields"]["caller"], CONSUMER);
    assert_eq!(p.view(&pid).await.1["state"], "TERMINATED");
}

/// TD-47, consumer's side: the provider of a transfer we asked for, now
/// without membership, may still end it; nothing else.
#[tokio::test]
async fn a_provider_without_membership_may_still_end_our_transfer() {
    use portitor_dsp::transfer::TransferState;
    use portitor_dsp::transfer::consumer::{ConsumerTransfer, ConsumerTransferStore as _};

    let mut ports = common::ports(Arc::new(FakeSender::default()) as Arc<dyn Sender>);
    let (verifier, revoked) = Revocable::new(Duration::from_secs(60));
    ports.verifier = Arc::new(verifier);
    let transfer = ConsumerTransfer {
        consumer_pid: "urn:uuid:ct-1".to_owned(),
        provider_pid: Some("urn:uuid:pt-1".to_owned()),
        state: TransferState::Requested,
        provider_address: "https://provider.example/dsp/2025-1".to_owned(),
        provider_id: PROVIDER.to_owned(),
        agreement_id: "urn:uuid:agreement-1".to_owned(),
        format: "HttpData-PULL".to_owned(),
        data_address: None,
        pending: None,
        version: 0,
    };
    ports.consumer_transfers.store.insert(&transfer).unwrap();
    let store = ports.consumer_transfers.store.clone();
    let mut config = config(3_600);
    config.participant_id = CONSUMER.into();
    config.public_url = "https://consumer.example".into();
    let (app, _) = portitor_connector::build(&config, ports).unwrap();
    let (log, _guard) = Log::start();
    revoked.store(true, Ordering::SeqCst);

    let base = "/dsp/2025-1/callback/transfers/urn:uuid:ct-1";
    let message = |did: &str, kind: &str, path: &str| {
        let body = json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": kind, "providerPid": "urn:uuid:pt-1", "consumerPid": "urn:uuid:ct-1"
        });
        from(Request::post(format!("{base}/{path}")), did)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let start = message(PROVIDER, "TransferStartMessage", "start");
    assert_eq!(call(&app, start).await.0, StatusCode::NOT_FOUND);
    let by_stranger = message(STRANGER, "TransferTerminationMessage", "termination");
    assert_eq!(call(&app, by_stranger).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        store.get("urn:uuid:ct-1").unwrap().unwrap().state,
        TransferState::Requested
    );

    let ending = message(PROVIDER, "TransferTerminationMessage", "termination");
    let (status, ack) = call(&app, ending).await;
    assert_eq!(status, StatusCode::OK, "{ack}");
    assert_eq!(
        store.get("urn:uuid:ct-1").unwrap().unwrap().state,
        TransferState::Terminated
    );
    let lines = log.with_message(ACCEPTED_WITHOUT_MEMBERSHIP);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["fields"]["process"], "urn:uuid:ct-1");
    assert_eq!(lines[0]["fields"]["caller"], PROVIDER);
}

/// Now, in seconds since 1970, as leases count it.
fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Writes to `store`, as another process sharing the database would, that
/// transfer `pid` is suspended by the consumer and its data plane step
/// recorded but not run, with a lease that ends at `lease_until`.
fn left_suspending(store: &ConnectorStore, pid: &str, lease_until: u64) {
    let mut transfer = store.get(pid).unwrap().unwrap();
    let expected = transfer.version;
    transfer.version += 1;
    transfer.state = TransferState::Suspended;
    transfer.flow_step = Some(FlowStep::Suspend);
    transfer.state_before_step = Some(TransferState::Started);
    transfer.lease = Some(Lease {
        holder: "urn:uuid:another-process".to_owned(),
        until: lease_until,
    });
    assert!(store.replace_if(&transfer, expected).unwrap());
}

/// TD-48 (closed in 1.2): while another process holds the transfer's
/// lease (its data plane step runs), a consumer's message is answered 503
/// with the DSP error and `Retry-After`, and so is the provider's own
/// termination on the management API; nothing changes. Once the lease
/// has ended, the sweep runs the step left over: access stops, and the
/// step is recorded as run.
#[tokio::test]
async fn a_leased_transfer_is_answered_503_and_the_sweep_takes_it_over() {
    let store = Arc::new(ConnectorStore::in_memory().unwrap());
    let p = provider_with(
        &format!("{}/file", upstream().await),
        Duration::from_secs(60),
        3_600,
        &store,
    );
    let (pid, path, token) = p.started_transfer().await;
    left_suspending(&store, &pid, now_seconds() + 60);

    let request = message_from(CONSUMER, "TransferTerminationMessage", &pid, "termination");
    let response = p.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["@type"], "TransferError");
    assert_eq!(error["code"], "503");
    assert_eq!(error["providerPid"], pid.as_str());
    assert_eq!(error["reason"][0], "the transfer is being changed; retry");

    let uri = format!("/v1/provided-transfers/{pid}/termination");
    let response = p
        .management
        .clone()
        .oneshot(manage("POST", &uri))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(
        p.view(&pid).await.1["state"],
        "SUSPENDED",
        "nothing written"
    );
    assert_eq!(
        p.fetch(&path, &token).await.0,
        StatusCode::OK,
        "no step ran"
    );

    // the lease ends; the sweep runs the step left over
    left_suspending(&store, &pid, 1);
    let sweep = p
        .negotiator
        .schedule_transfer_sweep(Duration::from_millis(10));
    for _ in 0..400 {
        if store.get(&pid).unwrap().unwrap().flow_step.is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    sweep.abort();
    let saved = store.get(&pid).unwrap().unwrap();
    assert_eq!((saved.flow_step, saved.lease), (None, None));
    assert_ne!(p.fetch(&path, &token).await.0, StatusCode::OK, "suspended");
}
