// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider side of the transfer process over HTTP (ADR 0013), with a
//! fixed agreement and the outgoing messages captured by a fake sender.
//! Most tests use a fake data plane; the last ones use the built-in one
//! and fetch data from a local upstream.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use portitor_connector::auth::{Caller, CallerVerifier, VerifyError};
use portitor_connector::harvest::FixedSource;
use portitor_connector::http_client::Sender;
use portitor_connector::ports::Retry;
use portitor_connector::ports::Shared;
use portitor_connector::store::SqliteNegotiations;
use portitor_connector::{Config, Ports};
use portitor_dsp::negotiation::provider::{Negotiation, Outgoing};
use portitor_dsp::negotiation::{Agreement, NegotiationState, Transition};
use portitor_dsp::policy::HeldCredential;
use portitor_dsp::transfer::provider::{Agreements, DataFlows, StoreError, Transfer};
use portitor_dsp::transfer::{DataAddress, DataAddressType, EndpointProperty};
use portitor_runtime::BoxFuture;
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::{Audiences, CONSUMER, PROVIDER, authenticating, from, recording};

const BASE: &str = "/dsp/2025-1/transfers";
const CALLBACK: &str = "https://consumer.example/cb";
const CONSUMER_PID: &str = "urn:uuid:consumer-1";

/// Records every POST and answers `Ok`, unless `fail` is set (no answer
/// at all) or `refuses_with` is (the consumer's answer, such as
/// `HTTP 404 Not Found`).
#[derive(Default)]
struct FakeSender {
    sent: Mutex<Vec<(String, Value)>>,
    fail: bool,
    refuses_with: Option<&'static str>,
}

impl Sender for FakeSender {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.sent
            .lock()
            .unwrap()
            .push((url.to_owned(), body.clone()));
        Box::pin(async move {
            if let Some(refusal) = self.refuses_with {
                Err(refusal.to_owned())
            } else if self.fail {
                Err("unreachable".to_owned())
            } else {
                Ok(Value::Null)
            }
        })
    }
}

impl FakeSender {
    fn sent(&self) -> Vec<(String, Value)> {
        self.sent.lock().unwrap().clone()
    }
}

/// One finalized agreement, `a1`, for dataset `CAT0101`, held by the
/// consumer.
struct OneAgreement;

impl Agreements for OneAgreement {
    fn finalized(&self, id: &str) -> Result<Option<Agreement>, StoreError> {
        let agreement = json!({
            "@id": "a1", "@type": "Agreement", "target": "CAT0101",
            "assigner": "did:web:provider.example", "assignee": CONSUMER,
            "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
        });
        Ok((id == "a1").then(|| serde_json::from_value(agreement).unwrap()))
    }
    // the fixed agreement belongs to no stored negotiation
    fn negotiation_of(&self, _: &str) -> Result<Option<Negotiation>, StoreError> {
        Ok(None)
    }
    fn replace_negotiation_if(&self, _: &Negotiation, _: u64) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// A data plane that serves `HttpData-PULL` and records its calls.
#[derive(Default)]
struct FakeFlows(Mutex<Vec<String>>);

impl FakeFlows {
    fn record(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }
    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl DataFlows for FakeFlows {
    fn supports(&self, _: &str, format: &str) -> bool {
        format == "HttpData-PULL"
    }
    fn start(&self, t: &Transfer) -> Result<DataAddress, String> {
        self.record(format!("start {}", t.dataset_id));
        Ok(DataAddress {
            kind: DataAddressType,
            endpoint_type: "https://w3id.org/idsa/v4.1/HTTP".to_owned(),
            endpoint: Some("https://provider.example/data".to_owned()),
            endpoint_properties: vec![
                EndpointProperty::new("authorization", "t0ken"),
                EndpointProperty::new("authType", "bearer"),
            ],
        })
    }
    fn restart(&self, _: &str) -> Result<DataAddress, String> {
        self.record("restart".to_owned());
        Err("the fake provider never starts a transfer again".to_owned())
    }
    fn suspend(&self, _: &str) -> Result<(), String> {
        self.record("suspend".to_owned());
        Ok(())
    }
    fn resume(&self, _: &str) -> Result<(), String> {
        self.record("resume".to_owned());
        Ok(())
    }
    fn complete(&self, _: &str) -> Result<(), String> {
        self.record("complete".to_owned());
        Ok(())
    }
    fn terminate(&self, _: &str) -> Result<(), String> {
        self.record("terminate".to_owned());
        Ok(())
    }
}

fn config() -> Config {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");
    Config {
        participant_id: "did:web:provider.example".into(),
        public_url: "https://provider.example".into(),
        datasets_file: Some(path.into()),
        harvest_interval_seconds: 0,
        allow_http: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

fn ports(sender: &Arc<FakeSender>, flows: &Arc<FakeFlows>) -> Ports {
    recorded_ports(sender, flows).0
}

/// Like [`ports`], and the audience of each token the provider sends.
fn recorded_ports(sender: &Arc<FakeSender>, flows: &Arc<FakeFlows>) -> (Ports, Audiences) {
    let (mut p, audiences) = recording(Ports::new(Arc::clone(sender) as Arc<dyn Sender>), PROVIDER);
    p.retry = Retry {
        attempts: 2,
        first_delay: Duration::from_millis(1),
    };
    p.transfers.agreements = Shared(Arc::new(OneAgreement));
    p.transfers.flows = Shared(Arc::clone(flows) as _);
    // without a flow store the built-in data plane does not start, so it
    // does not replace the fake one
    p.flow_store = None;
    (p, audiences)
}

/// A POST of `body` to `uri` by the participant `did`.
fn post_as(uri: &str, body: &Value, did: &str) -> Request<Body> {
    from(Request::post(uri), did)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn post(uri: &str, body: &Value) -> Request<Body> {
    from(Request::post(uri), CONSUMER)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn request(agreement_id: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "TransferRequestMessage",
        "consumerPid": CONSUMER_PID,
        "agreementId": agreement_id,
        "format": "HttpData-PULL",
        "callbackAddress": CALLBACK
    })
}

/// A message from the consumer about transfer `pid`.
fn message(kind: &str, pid: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": kind,
        "providerPid": pid,
        "consumerPid": CONSUMER_PID
    })
}

/// Waits until the sender has sent `n` messages (they go out in the
/// background).
async fn sent(sender: &FakeSender, n: usize) -> Vec<(String, Value)> {
    for _ in 0..200 {
        if sender.sent().len() >= n {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    sender.sent()
}

async fn state(app: &Router, pid: &str) -> String {
    let req = from(Request::get(format!("{BASE}/{pid}")), CONSUMER)
        .body(Body::empty())
        .unwrap();
    let (status, body) = call(app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["state"].as_str().unwrap().to_owned()
}

/// Waits until the transfer reaches `expected` (acknowledgements are
/// recorded in the background).
async fn wait_for(app: &Router, pid: &str, expected: &str) {
    for _ in 0..200 {
        if state(app, pid).await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{pid} never reached {expected}");
}

#[tokio::test]
async fn request_starts_and_the_consumer_suspends_restarts_and_completes() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let (ports, audiences) = recorded_ports(&sender, &flows);
    let app = portitor_connector::router(&config(), ports).unwrap();

    let (status, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{ack}");
    assert_eq!(ack["@type"], "TransferProcess");
    assert_eq!(ack["state"], "REQUESTED");
    let pid = ack["providerPid"].as_str().unwrap().to_owned();

    // the provider sends its start message with the data address
    let out = sent(&sender, 1).await;
    assert_eq!(
        out[0].0,
        format!("{CALLBACK}/transfers/{CONSUMER_PID}/start")
    );
    assert_eq!(out[0].1["@type"], "TransferStartMessage");
    assert_eq!(
        out[0].1["dataAddress"]["endpoint"],
        "https://provider.example/data"
    );
    wait_for(&app, &pid, "STARTED").await;
    assert_eq!(
        *audiences.lock().unwrap(),
        [CONSUMER],
        "the start went with a token for the consumer"
    );

    let suspend = message("TransferSuspensionMessage", &pid);
    let (status, ack) = call(&app, post(&format!("{BASE}/{pid}/suspension"), &suspend)).await;
    assert_eq!(
        (status, ack["state"].as_str()),
        (StatusCode::OK, Some("SUSPENDED"))
    );
    let restart = message("TransferStartMessage", &pid);
    let (status, ack) = call(&app, post(&format!("{BASE}/{pid}/start"), &restart)).await;
    assert_eq!(
        (status, ack["state"].as_str()),
        (StatusCode::OK, Some("STARTED"))
    );
    let complete = message("TransferCompletionMessage", &pid);
    let (status, ack) = call(&app, post(&format!("{BASE}/{pid}/completion"), &complete)).await;
    assert_eq!(
        (status, ack["state"].as_str()),
        (StatusCode::OK, Some("COMPLETED"))
    );

    assert_eq!(
        flows.calls(),
        ["start CAT0101", "suspend", "resume", "complete"]
    );
    // nothing more was sent: a restart needs no new address
    assert_eq!(sender.sent().len(), 1);

    // a completed transfer cannot be terminated
    let terminate = message("TransferTerminationMessage", &pid);
    let (status, error) = call(&app, post(&format!("{BASE}/{pid}/termination"), &terminate)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["@type"], "TransferError");
}

/// DSP 2025-1, "Transfer Request Message": "Providers SHOULD implement
/// idempotent behavior for Transfer Request Messages based on the value of
/// `consumerPid`" (TD-34).
#[tokio::test]
async fn a_repeated_request_gets_the_same_transfer_and_one_token() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let uri = format!("{BASE}/request");

    let (status, first) = call(&app, post(&uri, &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let pid = first["providerPid"].as_str().unwrap().to_owned();
    sent(&sender, 1).await;
    wait_for(&app, &pid, "STARTED").await;

    // the consumer sends its request again, for example after a timeout
    let (status, again) = call(&app, post(&uri, &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["@type"], "TransferProcess");
    assert_eq!(again["providerPid"], pid.as_str());
    assert_eq!(again["consumerPid"], CONSUMER_PID);
    assert_eq!(again["state"], "STARTED", "the transfer as it is now");

    // give a second start message the time to go out, if there were one
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sender.sent().len(), 1, "one start message, so one token");
    assert_eq!(flows.calls(), ["start CAT0101"], "one data flow");

    // the same consumer pid with another callback address is not the same
    // request: refused, and nothing is started
    let mut other = request("a1");
    other["callbackAddress"] = json!("https://elsewhere.example/cb");
    let (status, error) = call(&app, post(&uri, &other)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["@type"], "TransferError");

    // another participant with the same consumer pid learns nothing of
    // the transfer: it is refused as for any agreement that is not its own
    let stranger = post_as(&uri, &request("a1"), "did:web:stranger.example");
    let (status, error) = call(&app, stranger).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert!(!error.to_string().contains(&pid), "{error}");
    assert_eq!(flows.calls(), ["start CAT0101"]);
}

/// A repeated request that is refused because it differs leaves the
/// first transfer as it was: still STARTED, its data flow running, and
/// nothing more sent to the consumer.
#[tokio::test]
async fn a_differing_repeated_request_leaves_the_first_transfer_as_it_was() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let uri = format!("{BASE}/request");
    let (_, first) = call(&app, post(&uri, &request("a1"))).await;
    let pid = first["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;

    let mut other_callback = request("a1");
    other_callback["callbackAddress"] = json!("https://elsewhere.example/cb");
    let (status, error) = call(&app, post(&uri, &other_callback)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["code"], "400");
    assert_eq!(error["consumerPid"], CONSUMER_PID);
    assert_eq!(
        error["reason"][0],
        "consumerPid is already used by another transfer request of this consumer"
    );
    // the refusal does not name the transfer that exists
    assert!(!error.to_string().contains(&pid), "{error}");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state(&app, &pid).await, "STARTED");
    assert_eq!(flows.calls(), ["start CAT0101"]);
    let out = sender.sent();
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].0.starts_with(CALLBACK), "sent to the first address");
}

/// A `consumerPid` stays taken once its transfer has ended: the request
/// sent again gets the ended transfer, and no second transfer starts
/// under it (DSP 2025-1, "Transfer Request Message": idempotent on
/// `consumerPid`; ADR 0013, amended in 0.12).
#[tokio::test]
async fn a_repeated_request_after_the_transfer_ended_starts_nothing() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let uri = format!("{BASE}/request");
    let (_, first) = call(&app, post(&uri, &request("a1"))).await;
    let pid = first["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;
    let complete = message("TransferCompletionMessage", &pid);
    let (status, _) = call(&app, post(&format!("{BASE}/{pid}/completion"), &complete)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, again) = call(&app, post(&uri, &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["providerPid"], pid.as_str());
    assert_eq!(again["state"], "COMPLETED");

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(flows.calls(), ["start CAT0101", "complete"]);
    assert_eq!(sender.sent().len(), 1, "no second start message");
}

/// The repeated request with the `SQLite` store, as in production: the
/// lookup and the insert are SQL there (`store/transfers.rs`).
#[tokio::test]
async fn a_repeated_request_is_one_transfer_in_the_sqlite_store() {
    use portitor_dsp::transfer::provider::TransferStore as _;

    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let file = Arc::new(SqliteNegotiations::in_memory().unwrap());
    let mut p = ports(&sender, &flows);
    p.use_sqlite(&file);
    // the agreement still comes from the test, not from a stored negotiation
    p.transfers.agreements = Shared(Arc::new(OneAgreement));
    let app = portitor_connector::router(&config(), p).unwrap();
    let uri = format!("{BASE}/request");

    let (status, first) = call(&app, post(&uri, &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let pid = first["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;
    let (status, again) = call(&app, post(&uri, &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["providerPid"], pid.as_str());

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sender.sent().len(), 1, "one start message, so one token");
    assert_eq!(flows.calls(), ["start CAT0101"]);
    let stored = file.requested(CONSUMER, CONSUMER_PID).unwrap().unwrap();
    assert_eq!(stored.provider_pid, pid);
    assert!(file.pending().unwrap().is_empty(), "nothing left to send");
}

/// Every DSP route of the connector: method, path and DSP error type.
/// The process ids do not exist; a caller that is refused never gets as
/// far as the lookup.
const DSP_ROUTES: &[(&str, &str, &str)] = &[
    ("POST", "/dsp/2025-1/catalog/request", "CatalogError"),
    (
        "GET",
        "/dsp/2025-1/catalog/datasets/CAT0101",
        "CatalogError",
    ),
    (
        "POST",
        "/dsp/2025-1/negotiations/request",
        "ContractNegotiationError",
    ),
    (
        "GET",
        "/dsp/2025-1/negotiations/p1",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/negotiations/p1/request",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/negotiations/p1/events",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/negotiations/p1/agreement/verification",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/negotiations/p1/termination",
        "ContractNegotiationError",
    ),
    ("POST", "/dsp/2025-1/transfers/request", "TransferError"),
    ("GET", "/dsp/2025-1/transfers/p1", "TransferError"),
    ("POST", "/dsp/2025-1/transfers/p1/start", "TransferError"),
    (
        "POST",
        "/dsp/2025-1/transfers/p1/suspension",
        "TransferError",
    ),
    (
        "POST",
        "/dsp/2025-1/transfers/p1/completion",
        "TransferError",
    ),
    (
        "POST",
        "/dsp/2025-1/transfers/p1/termination",
        "TransferError",
    ),
    (
        "GET",
        "/dsp/2025-1/callback/negotiations/c1",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/negotiations/c1/offers",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/negotiations/c1/agreement",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/negotiations/c1/events",
        "ContractNegotiationError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/negotiations/c1/termination",
        "ContractNegotiationError",
    ),
    ("GET", "/dsp/2025-1/callback/transfers/c1", "TransferError"),
    (
        "POST",
        "/dsp/2025-1/callback/transfers/c1/start",
        "TransferError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/transfers/c1/suspension",
        "TransferError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/transfers/c1/completion",
        "TransferError",
    ),
    (
        "POST",
        "/dsp/2025-1/callback/transfers/c1/termination",
        "TransferError",
    ),
];

/// A request to a route of [`DSP_ROUTES`] with an empty JSON body; with
/// `caller`, sent by that participant, without, with no `Authorization`
/// header.
fn route_request(method: &str, path: &str, caller: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(did) = caller {
        request = from(request, did);
    }
    request
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap()
}

/// DSP 2025-1, "Unauthorized Access" of the negotiation and the transfer
/// HTTPS bindings: "If the client is not authorized, the Consumer or
/// Provider MUST return an HTTP 404 (Not Found) response." The catalog
/// binding asks for "an appropriate HTTP code": 401 (ADR 0016, amended
/// in 0.12). Checked on every route of the real router, the consumer's
/// callbacks included.
#[tokio::test]
async fn every_dsp_route_refuses_a_caller_without_a_token() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    for (method, path, error_type) in DSP_ROUTES {
        let expected = if *error_type == "CatalogError" {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::NOT_FOUND
        };
        let (status, error) = call(&app, route_request(method, path, None)).await;
        assert_eq!(status, expected, "{method} {path}: {error}");
        assert_eq!(error["@type"], *error_type, "{method} {path}");
        assert_eq!(error["code"], expected.as_str(), "{method} {path}");
        assert_eq!(error["reason"], json!(["unauthorized"]), "{method} {path}");
    }
}

/// A verifier whose check cannot finish: a service it needs is down.
struct Down;

impl CallerVerifier for Down {
    fn verify<'a>(&'a self, _: Option<&'a str>) -> BoxFuture<'a, Result<Caller, VerifyError>> {
        let down = VerifyError::Unavailable("the issuer's DID document: no answer".to_owned());
        Box::pin(async move { Err(down) })
    }

    fn credentials<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Unchanged in 0.12: while a service the caller's check needs is down,
/// every DSP route answers 503 (ADR 0016, amended in 0.11), never the 404
/// or 401 of a refusal, so the partner's outbox tries again (ADR 0007).
#[tokio::test]
async fn every_dsp_route_answers_503_while_the_callers_check_cannot_finish() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let mut p = ports(&sender, &flows);
    p.verifier = Arc::new(Down);
    let app = portitor_connector::router(&config(), p).unwrap();
    for (method, path, _) in DSP_ROUTES {
        let request = route_request(method, path, Some(CONSUMER));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{method} {path}"
        );
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json",
            "{method} {path}"
        );
    }
}

/// A message without a token changes nothing: the transfer it names
/// stays STARTED and its data flow goes on.
#[tokio::test]
async fn a_message_without_a_token_does_not_change_a_transfer() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let (_, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    let pid = ack["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;

    for (path, kind) in [
        ("suspension", "TransferSuspensionMessage"),
        ("completion", "TransferCompletionMessage"),
        ("termination", "TransferTerminationMessage"),
    ] {
        let anonymous = Request::post(format!("{BASE}/{pid}/{path}"))
            .header("content-type", "application/json")
            .body(Body::from(message(kind, &pid).to_string()))
            .unwrap();
        let (status, error) = call(&app, anonymous).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {error}");
        // the answer is the one for a transfer that does not exist
        assert_eq!(error["providerPid"], "", "{path}");
        assert_eq!(state(&app, &pid).await, "STARTED", "{path}");
    }
    assert_eq!(flows.calls(), ["start CAT0101"]);
}

/// A consumer that answers 404 will not take the message: since 0.12 it
/// is also the answer of a partner that refuses our credentials (ADR
/// 0016). The start message is sent once, not tried again, and the
/// transfer ends with its data flow (ADR 0007). The TCK build tries a
/// 404 again (`outbox.rs`, `tck_retries`).
#[tokio::test]
async fn a_consumer_that_answers_404_is_not_sent_the_message_again() {
    let sender = Arc::new(FakeSender {
        refuses_with: Some("HTTP 404 Not Found"),
        ..FakeSender::default()
    });
    let flows = Arc::new(FakeFlows::default());
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let (_, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    let pid = ack["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "TERMINATED").await;
    assert_eq!(flows.calls(), ["start CAT0101", "terminate"]);

    // one try of the start and one of the termination; two each with `tck`
    let tries_each = if cfg!(feature = "tck") { 2 } else { 1 };
    let out = sent(&sender, 2 * tries_each).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sender.sent().len(), 2 * tries_each, "{out:?}");
    assert!(out[0].0.ends_with("/start"), "{out:?}");
    assert!(
        out[2 * tries_each - 1].0.ends_with("/termination"),
        "{out:?}"
    );
}

#[tokio::test]
async fn requests_without_a_held_agreement_are_refused() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let (status, error) = call(&app, post(&format!("{BASE}/request"), &request("unknown"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["@type"], "TransferError");
    assert_eq!(error["consumerPid"], CONSUMER_PID);

    let mut push = request("a1");
    push["dataAddress"] =
        json!({"@type": "DataAddress", "endpointType": "https://w3id.org/idsa/v4.1/HTTP"});
    let (status, _) = call(&app, post(&format!("{BASE}/request"), &push)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut http_callback = request("a1");
    http_callback["callbackAddress"] = json!("http://consumer.example/cb");
    let (status, _) = call(&app, post(&format!("{BASE}/request"), &http_callback)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(&app, post(&format!("{BASE}/request"), &json!({"x": 1}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(flows.calls().is_empty());
    assert!(sender.sent().is_empty());
}

#[tokio::test]
async fn only_the_assignee_may_use_an_agreement() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let uri = format!("{BASE}/request");
    let body = Body::from(request("a1").to_string());
    let stranger = from(Request::post(&uri), "did:web:stranger.example")
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let (status, error) = call(&app, stranger).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    let anonymous = Request::post(&uri)
        .header("content-type", "application/json")
        .body(Body::from(request("a1").to_string()))
        .unwrap();
    let (status, error) = call(&app, anonymous).await;
    // DSP 2025-1, "Transfer Process HTTPS Binding", "Unauthorized Access":
    // a client that is not authorized gets 404
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        (error["@type"].as_str(), error["code"].as_str()),
        (Some("TransferError"), Some("404"))
    );
    assert!(flows.calls().is_empty());
}

#[tokio::test]
async fn an_unknown_transfer_is_not_found() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let req = from(Request::get(format!("{BASE}/urn:uuid:nope")), CONSUMER)
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&app, req).await.0, StatusCode::NOT_FOUND);
    let suspend = message("TransferSuspensionMessage", "urn:uuid:nope");
    let (status, _) = call(
        &app,
        post(&format!("{BASE}/urn:uuid:nope/suspension"), &suspend),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unreachable_consumer_ends_the_transfer_and_its_data_flow() {
    let sender = Arc::new(FakeSender {
        fail: true,
        ..FakeSender::default()
    });
    let flows = Arc::new(FakeFlows::default());
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let (_, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    let pid = ack["providerPid"].as_str().unwrap().to_owned();
    // two tries of the start, then two of the termination
    let out = sent(&sender, 4).await;
    assert!(out[3].0.ends_with("/termination"), "{out:?}");
    wait_for(&app, &pid, "TERMINATED").await;
    assert_eq!(flows.calls(), ["start CAT0101", "terminate"]);
}

#[tokio::test]
async fn without_a_data_plane_every_request_is_refused() {
    let sender = Arc::new(FakeSender::default());
    let mut p = authenticating(Ports::new(Arc::clone(&sender) as Arc<dyn Sender>), PROVIDER);
    p.transfers.agreements = Shared(Arc::new(OneAgreement));
    p.flow_store = None;
    let app = portitor_connector::router(&config(), p).unwrap();
    let (status, error) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["reason"].to_string().contains("cannot transfer"),
        "{error}"
    );
}

/// With `SQLite`, the agreements come from the negotiation table: a
/// request under an agreement that is not finalized there is refused.
#[tokio::test]
async fn sqlite_ports_use_the_stored_agreements() {
    let sender = Arc::new(FakeSender::default());
    let flows = Arc::new(FakeFlows::default());
    let mut p = ports(&sender, &flows);
    p.use_sqlite(&Arc::new(SqliteNegotiations::in_memory().unwrap()));
    let app = portitor_connector::router(&config(), p).unwrap();
    let (status, _) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- ADR 0007, amended in 0.13 (TD-44): a transfer request that arrives
// ---- before the acknowledgement of the finalization event is recorded

/// The provider's negotiation `urn:uuid:n1` with the consumer, in the
/// state the race of TD-44 finds it: `VERIFIED`, its agreement `a-new`
/// for dataset `CAT0101` sent, the finalization event on its way.
fn finalization_on_its_way() -> Negotiation {
    let status_url = format!("{CALLBACK}/negotiations/urn:uuid:c1");
    Negotiation {
        provider_pid: "urn:uuid:n1".to_owned(),
        consumer_pid: "urn:uuid:c1".to_owned(),
        state: NegotiationState::Verified,
        callback_address: CALLBACK.to_owned(),
        counter_party: CONSUMER.to_owned(),
        offer: serde_json::from_value(json!({
            "@id": "o", "@type": "Offer", "target": "CAT0101",
            "permission": [{"action": "use"}]
        }))
        .unwrap(),
        agreement: Some(
            serde_json::from_value(json!({
                "@id": "a-new", "@type": "Agreement", "target": "CAT0101",
                "assigner": "did:web:provider.example", "assignee": CONSUMER,
                "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
            }))
            .unwrap(),
        ),
        pending: Some(Outgoing {
            provider_pid: "urn:uuid:n1".to_owned(),
            url: format!("{status_url}/events"),
            body: json!({"@type": "ContractNegotiationEventMessage", "eventType": "FINALIZED"}),
            transition: Transition::Finalize,
            to: NegotiationState::Finalized,
            status_url,
            counter_party: CONSUMER.to_owned(),
        }),
        version: 4,
    }
}

/// The connector with the `SQLite` store, as in production, holding
/// `negotiation`.
fn app_with_stored(negotiation: &Negotiation) -> (Router, Arc<SqliteNegotiations>, Arc<FakeFlows>) {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let file = Arc::new(SqliteNegotiations::in_memory().unwrap());
    portitor_dsp::negotiation::provider::NegotiationStore::insert(&*file, negotiation).unwrap();
    let mut p = ports(&sender, &flows);
    p.use_sqlite(&file);
    let app = portitor_connector::router(&config(), p).unwrap();
    (app, file, flows)
}

fn stored(file: &SqliteNegotiations) -> Negotiation {
    portitor_dsp::negotiation::provider::NegotiationStore::get(file, "urn:uuid:n1")
        .unwrap()
        .unwrap()
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
        let log = Self::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        (log, tracing::subscriber::set_default(subscriber))
    }
    fn lines(&self) -> Vec<Value> {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn a_request_while_the_finalization_is_on_its_way_is_accepted_and_settles_it() {
    let (app, file, flows) = app_with_stored(&finalization_on_its_way());
    let (log, _guard) = Log::start();

    let uri = format!("{BASE}/request");
    let (status, ack) = call(&app, post(&uri, &request("a-new"))).await;
    assert_eq!(status, StatusCode::CREATED, "{ack}");
    assert_eq!(flows.calls(), ["start CAT0101"]);

    let settled = stored(&file);
    assert_eq!(settled.state, NegotiationState::Finalized);
    assert_eq!(settled.pending, None);
    assert_eq!(settled.version, 5);
    assert!(
        portitor_dsp::negotiation::provider::NegotiationStore::pending(&*file)
            .unwrap()
            .is_empty(),
        "the event is not sent again after a restart"
    );

    // the state change is reported like every other one (ADR 0023)
    let changes: Vec<Value> = log
        .lines()
        .into_iter()
        .filter(|line| line["fields"]["portitor.dsp.process"] == "negotiation")
        .collect();
    assert_eq!(changes.len(), 1, "{changes:?}");
    let fields = &changes[0]["fields"];
    assert_eq!(fields["portitor.dsp.provider_pid"], "urn:uuid:n1");
    assert_eq!(fields["portitor.dsp.previous_state"], "VERIFIED");
    assert_eq!(fields["portitor.dsp.state"], "FINALIZED");
}

/// ADR 0013: only the agreement's consumer; and a negotiation in any
/// other state is refused as before. Each refusal is one INFO line that
/// names neither the agreement nor a participant.
#[tokio::test]
async fn a_request_that_proves_nothing_is_refused_logged_and_settles_nothing() {
    let uri = format!("{BASE}/request");
    let mut agreed_only = finalization_on_its_way();
    agreed_only.state = NegotiationState::Agreed;
    agreed_only.pending = None;
    let mut terminated = agreed_only.clone();
    terminated.state = NegotiationState::Terminated;
    let cases = [
        (finalization_on_its_way(), "did:web:stranger.example"),
        (agreed_only, CONSUMER),
        (terminated, CONSUMER),
    ];
    for (negotiation, caller) in cases {
        let (app, file, flows) = app_with_stored(&negotiation);
        let (log, _guard) = Log::start();
        let (status, error) = call(&app, post_as(&uri, &request("a-new"), caller)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        assert_eq!(stored(&file), negotiation, "nothing is written");
        assert!(flows.calls().is_empty());

        let lines = log.lines();
        let refusals: Vec<&Value> = lines
            .iter()
            .filter(|line| line["fields"]["message"] == "transfer request is refused; answered 400")
            .collect();
        assert_eq!(refusals.len(), 1, "{lines:?}");
        assert_eq!(refusals[0]["level"], "INFO");
        let text = refusals[0].to_string();
        assert!(text.contains("no finalized agreement"), "{text}");
        for private in ["a-new", caller, "urn:uuid:n1", CONSUMER_PID] {
            assert!(!text.contains(private), "{text}");
        }
    }
}

/// Sixteen transfer requests at once under one agreement whose
/// finalization is on its way, on several threads: all are accepted, and
/// the negotiation is written once (version 4 to 5). The requests that
/// lose the version check read the negotiation again and find it
/// FINALIZED (compare-and-update, ADR 0007: no lock, no wait).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_at_once_while_the_finalization_is_on_its_way_settle_it_once() {
    let (app, file, _flows) = app_with_stored(&finalization_on_its_way());
    let uri = format!("{BASE}/request");
    let mut running = Vec::new();
    for number in 0..16 {
        let mut body = request("a-new");
        body["consumerPid"] = json!(format!("urn:uuid:consumer-{number}"));
        let (app, uri) = (app.clone(), uri.clone());
        running.push(tokio::spawn(
            async move { call(&app, post(&uri, &body)).await },
        ));
    }
    for task in running {
        let (status, answer) = task.await.unwrap();
        assert_eq!(status, StatusCode::CREATED, "{answer}");
    }
    let settled = stored(&file);
    assert_eq!(settled.state, NegotiationState::Finalized);
    assert_eq!(settled.pending, None);
    assert_eq!(settled.version, 5, "written once");
}

/// `finalization_on_its_way()` moved to `state`, with `pending` on its
/// way instead of the finalization event.
fn negotiation_in(state: NegotiationState, pending: Option<Transition>) -> Negotiation {
    let mut negotiation = finalization_on_its_way();
    negotiation.state = state;
    negotiation.pending = match (negotiation.pending, pending) {
        (Some(message), Some(transition)) => Some(Outgoing {
            transition,
            to: match transition {
                Transition::Finalize => NegotiationState::Finalized,
                Transition::Terminate => NegotiationState::Terminated,
                _ => state,
            },
            ..message
        }),
        _ => None,
    };
    negotiation
}

/// The table of ADR 0007 (amended in 0.13) through HTTP and the `SQLite`
/// store, for every state of the negotiation and every message that can
/// be on its way when the transfer request arrives: it is served only
/// under a `FINALIZED` negotiation, or a `VERIFIED` one whose
/// finalization event is on its way. Every other request gets 400 and
/// the stored negotiation stays as it was.
#[tokio::test]
async fn a_request_is_served_only_when_finalized_or_when_the_finalization_is_on_its_way() {
    use NegotiationState::{Accepted, Agreed, Finalized, Offered, Requested, Terminated, Verified};
    use Transition::{Agree, Finalize, Offer, Terminate};
    let uri = format!("{BASE}/request");
    let states = [
        Requested, Offered, Accepted, Agreed, Verified, Finalized, Terminated,
    ];
    let on_its_way = [
        None,
        Some(Offer),
        Some(Agree),
        Some(Finalize),
        Some(Terminate),
    ];
    for state in states {
        for pending in on_its_way {
            let served = state == Finalized || (state == Verified && pending == Some(Finalize));
            let negotiation = negotiation_in(state, pending);
            let (app, file, flows) = app_with_stored(&negotiation);
            let (status, answer) = call(&app, post(&uri, &request("a-new"))).await;
            let case = format!("{state:?} with {pending:?} on its way: {answer}");
            if served {
                assert_eq!(status, StatusCode::CREATED, "{case}");
                assert_eq!(stored(&file).state, Finalized, "{case}");
                assert_eq!(flows.calls(), ["start CAT0101"], "{case}");
            } else {
                assert_eq!(status, StatusCode::BAD_REQUEST, "{case}");
                assert_eq!(stored(&file), negotiation, "{case}");
                assert!(flows.calls().is_empty(), "{case}");
            }
        }
    }
}

/// ADR 0007, amended in 0.13: "The negotiation is settled when the
/// request names the agreement and the caller is its consumer, also if
/// the request is then refused for another reason. The proof is that the
/// consumer asked, not that the transfer is granted."
#[tokio::test]
async fn a_request_refused_for_another_reason_still_settles_the_finalization() {
    let uri = format!("{BASE}/request");
    let mut other_format = request("a-new");
    other_format["format"] = json!("HttpData-PUSH");
    // a push transfer, which this connector does not do (ADR 0013)
    let mut push = request("a-new");
    push["dataAddress"] = json!({
        "@type": "DataAddress",
        "endpointType": "https://w3id.org/idsa/v4.1/HTTP",
        "endpoint": "https://consumer.example/in"
    });
    for (name, body) in [("another format", other_format), ("push", push)] {
        let (app, file, flows) = app_with_stored(&finalization_on_its_way());
        let (status, error) = call(&app, post(&uri, &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {error}");
        assert!(flows.calls().is_empty(), "{name}");
        let settled = stored(&file);
        assert_eq!(settled.state, NegotiationState::Finalized, "{name}");
        assert_eq!(settled.pending, None, "{name}");
        assert_eq!(settled.version, 5, "{name}");
        // and the next, good request is served without another write
        let (status, ack) = call(&app, post(&uri, &request("a-new"))).await;
        assert_eq!(status, StatusCode::CREATED, "{name}: {ack}");
        assert_eq!(stored(&file), settled, "{name}");
    }
}

/// What the code does today, written down: a request that is refused
/// before the agreement is looked up settles nothing. These are requests
/// that are no valid `TransferRequestMessage`, have no caller, or name a
/// callback address the connector may not call.
#[tokio::test]
async fn a_request_refused_before_the_agreement_is_looked_up_settles_nothing() {
    let uri = format!("{BASE}/request");
    let mut no_format = request("a-new");
    no_format.as_object_mut().unwrap().remove("format");
    let mut http_callback = request("a-new");
    http_callback["callbackAddress"] = json!("http://consumer.example/cb");
    let json_body = |body: &Value| Body::from(body.to_string());
    let cases = [
        (
            "not a valid message",
            post(&uri, &no_format),
            StatusCode::BAD_REQUEST,
        ),
        (
            "an http callback",
            post(&uri, &http_callback),
            StatusCode::BAD_REQUEST,
        ),
        (
            "no caller",
            Request::post(&uri)
                .header("content-type", "application/json")
                .body(json_body(&request("a-new")))
                .unwrap(),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (name, req, expected) in cases {
        let (app, file, flows) = app_with_stored(&finalization_on_its_way());
        let (status, error) = call(&app, req).await;
        assert_eq!(status, expected, "{name}: {error}");
        assert_eq!(stored(&file), finalization_on_its_way(), "{name}");
        assert!(flows.calls().is_empty(), "{name}");
    }
}

/// The connector on the `SQLite` file `path`. With `after_a_start`,
/// what the file holds as pending is sent again, as `main` does it.
/// Returns the router, what it sends, and how many messages were sent
/// again.
fn connector_on(path: &std::path::Path, after_a_start: bool) -> (Router, Arc<FakeSender>, usize) {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let mut p = ports(&sender, &flows);
    p.use_sqlite(&Arc::new(SqliteNegotiations::open(path).unwrap()));
    let (app, negotiator) = portitor_connector::build(&config(), p).unwrap();
    let sent_again = if after_a_start {
        negotiator.resume().unwrap()
    } else {
        0
    };
    (app, sender, sent_again)
}

/// A restart between the transfer request that settled the finalization
/// and the acknowledgement that never came: the file says `FINALIZED`,
/// the finalization event is not sent again ("It is not sent again after
/// a restart", ADR 0007), the transfer is still there and the agreement
/// serves another one.
#[tokio::test]
async fn after_a_restart_a_settled_finalization_is_not_sent_again() {
    use portitor_dsp::negotiation::provider::NegotiationStore as _;

    let folder = portitor_runtime::testing::TempDir::new("td-44");
    let path = folder.path().join("portitor.db");
    let file = SqliteNegotiations::open(&path).unwrap();
    file.insert(&finalization_on_its_way()).unwrap();
    let uri = format!("{BASE}/request");

    // the event is on its way (nothing answers it); the request arrives
    let (app, _sender, _) = connector_on(&path, false);
    let (status, first) = call(&app, post(&uri, &request("a-new"))).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let pid = first["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;
    drop(app);

    // the restart
    let (app, sender, sent_again) = connector_on(&path, true);
    assert_eq!(sent_again, 0, "nothing was pending");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(sender.sent().is_empty(), "{:?}", sender.sent());
    let settled = file.get("urn:uuid:n1").unwrap().unwrap();
    assert_eq!(settled.state, NegotiationState::Finalized);
    assert_eq!(settled.pending, None);
    assert_eq!(settled.version, 5);
    assert_eq!(state(&app, &pid).await, "STARTED");
    let mut second = request("a-new");
    second["consumerPid"] = json!("urn:uuid:consumer-2");
    let (status, answer) = call(&app, post(&uri, &second)).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(file.get("urn:uuid:n1").unwrap().unwrap(), settled);
}

/// The other order: the restart comes first and finds the finalization
/// event still pending. It is sent again; a transfer request that
/// arrives before the consumer answers it is served all the same.
#[tokio::test]
async fn after_a_restart_with_the_finalization_still_on_its_way_a_request_is_served() {
    use portitor_dsp::negotiation::provider::NegotiationStore as _;

    let folder = portitor_runtime::testing::TempDir::new("td-44");
    let path = folder.path().join("portitor.db");
    let file = SqliteNegotiations::open(&path).unwrap();
    file.insert(&finalization_on_its_way()).unwrap();

    let (app, sender, sent_again) = connector_on(&path, true);
    assert_eq!(sent_again, 1, "the finalization event");
    let (status, answer) = call(&app, post(&format!("{BASE}/request"), &request("a-new"))).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    // the event's acknowledgement and the transfer's start message
    let all = sent(&sender, 2).await;
    let events = all.iter().filter(|(url, _)| url.ends_with("/events"));
    assert_eq!(events.count(), 1, "{all:?}");
    let settled = file.get("urn:uuid:n1").unwrap().unwrap();
    assert_eq!(settled.state, NegotiationState::Finalized);
    assert_eq!(settled.pending, None);
    assert_eq!(
        settled.version, 5,
        "written once: by the request or by the acknowledgement"
    );
}

/// Serves `hello` as CSV at `/file` on a free local port; returns its URL.
#[cfg(feature = "data-plane")]
async fn upstream() -> String {
    let app = Router::new().route(
        "/file",
        axum::routing::get(|| async { ([("content-type", "text/csv")], "hello") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// The built-in data plane, with `source` as the data of dataset `CAT0101`.
fn built_in(sender: &Arc<FakeSender>, source: Option<&str>) -> Router {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");
    let mut entries: Vec<Value> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    // the TCK's file names a source for CAT0101: replace it, or drop it
    for entry in entries.iter_mut().filter(|entry| entry["@id"] == "CAT0101") {
        let fields = entry.as_object_mut().unwrap();
        fields.remove("portitor:source");
        if let Some(url) = source {
            fields.insert("portitor:source".to_owned(), json!(url));
        }
    }
    let mut p = authenticating(Ports::new(Arc::clone(sender) as Arc<dyn Sender>), PROVIDER);
    p.transfers.agreements = Shared(Arc::new(OneAgreement));
    p.harvest.source = Some(Arc::new(FixedSource::new(entries)));
    portitor_connector::router(&config(), p).unwrap()
}

/// `GET` of a data address path with `token`: the status and the body.
#[cfg(feature = "data-plane")]
async fn fetch(app: &Router, path: &str, token: &str) -> (StatusCode, String) {
    let req = Request::get(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[cfg(feature = "data-plane")] // the built-in data plane is behind it
#[tokio::test]
async fn the_built_in_data_plane_serves_the_data_while_the_transfer_is_started() {
    let sender = Arc::new(FakeSender::default());
    let url = upstream().await;
    let app = built_in(&sender, Some(&url));
    let (status, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{ack}");
    let pid = ack["providerPid"].as_str().unwrap().to_owned();

    // the start message carries the data address and its token
    let address = sent(&sender, 1).await[0].1["dataAddress"].clone();
    assert_eq!(address["endpointType"], "https://w3id.org/idsa/v4.1/HTTP");
    let endpoint = address["endpoint"].as_str().unwrap();
    let path = endpoint.strip_prefix("https://provider.example").unwrap();
    assert!(path.starts_with("/data/"), "{endpoint}");
    let property = |name: &str| {
        address["endpointProperties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .map(|p| p["value"].as_str().unwrap().to_owned())
            .unwrap()
    };
    assert_eq!(property("authType"), "bearer");
    let token = property("authorization");
    wait_for(&app, &pid, "STARTED").await;

    assert_eq!(
        fetch(&app, path, &token).await,
        (StatusCode::OK, "hello".to_owned())
    );
    assert_eq!(fetch(&app, path, "wrong").await.0, StatusCode::UNAUTHORIZED);

    // suspended: refused; started again: served with the same token
    let suspend = message("TransferSuspensionMessage", &pid);
    call(&app, post(&format!("{BASE}/{pid}/suspension"), &suspend)).await;
    assert_eq!(fetch(&app, path, &token).await.0, StatusCode::UNAUTHORIZED);
    let restart = message("TransferStartMessage", &pid);
    call(&app, post(&format!("{BASE}/{pid}/start"), &restart)).await;
    assert_eq!(fetch(&app, path, &token).await.0, StatusCode::OK);

    // completed: refused for good
    let complete = message("TransferCompletionMessage", &pid);
    call(&app, post(&format!("{BASE}/{pid}/completion"), &complete)).await;
    assert_eq!(fetch(&app, path, &token).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_dataset_without_a_source_cannot_be_transferred() {
    let sender = Arc::new(FakeSender::default());
    let app = built_in(&sender, None);
    let (status, error) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["reason"].to_string().contains("cannot transfer"),
        "{error}"
    );
    assert!(sender.sent().is_empty());
}

/// A stranger's messages about someone else's transfer are answered 404
/// and change nothing, not even the data flow.
#[tokio::test]
async fn a_stranger_cannot_change_a_transfer() {
    let (sender, flows) = (
        Arc::new(FakeSender::default()),
        Arc::new(FakeFlows::default()),
    );
    let app = portitor_connector::router(&config(), ports(&sender, &flows)).unwrap();
    let (status, ack) = call(&app, post(&format!("{BASE}/request"), &request("a1"))).await;
    assert_eq!(status, StatusCode::CREATED, "{ack}");
    let pid = ack["providerPid"].as_str().unwrap().to_owned();
    wait_for(&app, &pid, "STARTED").await;
    for (path, kind) in [
        ("start", "TransferStartMessage"),
        ("suspension", "TransferSuspensionMessage"),
        ("completion", "TransferCompletionMessage"),
        ("termination", "TransferTerminationMessage"),
    ] {
        let uri = format!("{BASE}/{pid}/{path}");
        let request = post_as(&uri, &message(kind, &pid), "did:web:stranger.example");
        let (status, error) = call(&app, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {error}");
        assert_eq!(error["@type"], "TransferError", "{path}");
        assert_eq!(state(&app, &pid).await, "STARTED", "{path}");
    }
    assert_eq!(flows.calls(), ["start CAT0101"]);
}
