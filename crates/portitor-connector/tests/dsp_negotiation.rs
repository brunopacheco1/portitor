// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider side of contract negotiation over HTTP, with the outgoing
//! messages captured by a fake sender.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use portitor_connector::http_client::Sender;
use portitor_connector::ports::{Retry, Shared};
use portitor_connector::{Config, Ports};
use portitor_dsp::guard::{Direction, MessageGuard, Rejection};
use portitor_dsp::negotiation::provider::{Decider, Decision, Negotiation};
use portitor_runtime::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tower::ServiceExt as _;

mod common;
use common::{Audiences, CONSUMER, PROVIDER, from, recording};

const BASE: &str = "/dsp/2025-1/negotiations";
const CALLBACK: &str = "https://consumer.example/cb";

/// Records every POST; answers with `Ok` unless `fail` is set (no answer
/// at all) or `refuses_with` is (the consumer's answer, such as
/// `HTTP 404 Not Found`). Holds POSTs whose URL ends with `hold` until
/// `release`. Answers a GET of the consumer's state with `reported`, if
/// set.
#[derive(Default)]
struct FakeSender {
    sent: Mutex<Vec<(String, Value)>>,
    fail: bool,
    refuses_with: Option<&'static str>,
    hold: Option<&'static str>,
    release: Notify,
    reported: Option<&'static str>,
}

impl Sender for FakeSender {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.sent
            .lock()
            .unwrap()
            .push((url.to_owned(), body.clone()));
        Box::pin(async move {
            if self.hold.is_some_and(|h| url.ends_with(h)) {
                self.release.notified().await;
            }
            if let Some(refusal) = self.refuses_with {
                Err(refusal.to_owned())
            } else if self.fail {
                Err("unreachable".to_owned())
            } else {
                Ok(Value::Null)
            }
        })
    }

    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            assert_eq!(url, format!("{CALLBACK}/negotiations/urn:uuid:consumer-1"));
            let state = self.reported.ok_or("no answer")?;
            Ok(json!({
                "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
                "@type": "ContractNegotiation",
                "providerPid": "ignored",
                "consumerPid": "urn:uuid:consumer-1",
                "state": state
            }))
        })
    }
}

impl FakeSender {
    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
    fn get(&self, i: usize) -> (String, Value) {
        self.sent.lock().unwrap()[i].clone()
    }
}

struct WaitDecider;

impl Decider for WaitDecider {
    fn decide(&self, _: &Negotiation) -> Decision {
        Decision::Wait
    }
}

/// Refuses one message type in one direction.
struct Refuse(Direction, &'static str);

impl MessageGuard for Refuse {
    fn check(&self, d: Direction, t: &str, _: &Value) -> Result<(), Rejection> {
        if d == self.0 && t == self.1 {
            Err(Rejection {
                reasons: vec![format!("{t} is not allowed here")],
            })
        } else {
            Ok(())
        }
    }
}

fn config(allow_http: bool) -> Config {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");
    Config {
        participant_id: "did:web:provider.example".into(),
        public_url: "https://provider.example".into(),
        datasets_file: Some(path.into()),
        harvest_interval_seconds: 0,
        allow_http: allow_http,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

fn ports(sender: &Arc<FakeSender>) -> Ports {
    recorded_ports(sender).0
}

/// Like [`ports`], and the audience of each token the provider sends.
fn recorded_ports(sender: &Arc<FakeSender>) -> (Ports, Audiences) {
    let (mut p, audiences) = recording(Ports::new(Arc::clone(sender) as Arc<dyn Sender>), PROVIDER);
    // the default build agrees to catalog offers; the tck build scripts
    // only ACN* datasets, so CAT0101 behaves the same in both
    p.retry = Retry {
        attempts: 2,
        first_delay: Duration::from_millis(1),
    };
    (p, audiences)
}

fn app(ports: Ports) -> Router {
    portitor_connector::router(&config(false), ports).unwrap()
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
    post_as(uri, body, CONSUMER)
}

/// A POST of `body` to `uri` by the participant `did`.
fn post_as(uri: &str, body: &Value, did: &str) -> Request<Body> {
    from(Request::post(uri), did)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    from(Request::get(uri), CONSUMER)
        .body(Body::empty())
        .unwrap()
}

fn request(callback: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "ContractRequestMessage",
        "consumerPid": "urn:uuid:consumer-1",
        "offer": {
            "@id": "CAT0101:offer:use",
            "@type": "Offer",
            "target": "CAT0101",
            "permission": [{"action": "use"}]
        },
        "callbackAddress": callback
    })
}

fn ids(pid: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "providerPid": pid,
        "consumerPid": "urn:uuid:consumer-1"
    })
}

fn with(mut base: Value, extra: &Value) -> Value {
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

/// Waits until `sender` has sent `n` messages.
async fn sent(sender: &FakeSender, n: usize) {
    for _ in 0..500 {
        if sender.count() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("expected {n} messages, got {}", sender.count());
}

/// Waits until the negotiation reaches `state`.
async fn state(app: &Router, pid: &str, state: &str) {
    let mut last = Value::Null;
    for _ in 0..500 {
        let (_, body) = call(app, get(&format!("{BASE}/{pid}"))).await;
        if body["state"] == state {
            return;
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("expected {state}, got {last}");
}

async fn start(app: &Router) -> String {
    let (s, ack) = call(app, post(&format!("{BASE}/request"), &request(CALLBACK))).await;
    assert_eq!(s, StatusCode::CREATED, "{ack}");
    assert_eq!(ack["@type"], "ContractNegotiation");
    assert_eq!(ack["state"], "REQUESTED");
    assert_eq!(ack["consumerPid"], "urn:uuid:consumer-1");
    ack["providerPid"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn request_agree_verify_finalize() {
    let sender = Arc::new(FakeSender::default());
    let (ports, audiences) = recorded_ports(&sender);
    let app = app(ports);
    let pid = start(&app).await;
    assert!(pid.starts_with("urn:uuid:"));

    sent(&sender, 1).await;
    let (url, agreement) = sender.get(0);
    assert_eq!(
        url,
        format!("{CALLBACK}/negotiations/urn:uuid:consumer-1/agreement")
    );
    assert_eq!(agreement["@type"], "ContractAgreementMessage");
    assert_eq!(agreement["agreement"]["target"], "CAT0101");
    assert_eq!(
        agreement["agreement"]["assigner"],
        "did:web:provider.example"
    );
    state(&app, &pid, "AGREED").await;

    let verify = with(
        ids(&pid),
        &json!({"@type": "ContractAgreementVerificationMessage"}),
    );
    let (s, ack) = call(
        &app,
        post(&format!("{BASE}/{pid}/agreement/verification"), &verify),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{ack}");
    assert_eq!(ack["state"], "VERIFIED");

    sent(&sender, 2).await;
    let (url, event) = sender.get(1);
    assert_eq!(
        url,
        format!("{CALLBACK}/negotiations/urn:uuid:consumer-1/events")
    );
    assert_eq!(event["eventType"], "FINALIZED");
    state(&app, &pid, "FINALIZED").await;
    // both messages went with a token for the consumer
    assert_eq!(*audiences.lock().unwrap(), [CONSUMER, CONSUMER]);
}

#[tokio::test]
async fn consumer_terminates() {
    let sender = Arc::new(FakeSender::default());
    let mut p = ports(&sender);
    p.decider = Shared(Arc::new(WaitDecider));
    let app = app(p);
    let pid = start(&app).await;
    let msg = with(
        ids(&pid),
        &json!({"@type": "ContractNegotiationTerminationMessage", "code": "1", "reason": ["no longer needed"]}),
    );
    let (s, ack) = call(&app, post(&format!("{BASE}/{pid}/termination"), &msg)).await;
    assert_eq!(s, StatusCode::OK, "{ack}");
    assert_eq!(ack["state"], "TERMINATED");
    // a second termination is an invalid transition
    let (s, err) = call(&app, post(&format!("{BASE}/{pid}/termination"), &msg)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(err["@type"], "ContractNegotiationError");
    assert_eq!(sender.count(), 0);
}

#[tokio::test]
async fn invalid_transition_is_400() {
    let sender = Arc::new(FakeSender::default());
    let mut p = ports(&sender);
    p.decider = Shared(Arc::new(WaitDecider));
    let app = app(p);
    let pid = start(&app).await;
    let verify = with(
        ids(&pid),
        &json!({"@type": "ContractAgreementVerificationMessage"}),
    );
    let (s, err) = call(
        &app,
        post(&format!("{BASE}/{pid}/agreement/verification"), &verify),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(err["@type"], "ContractNegotiationError");
    assert_eq!(err["providerPid"], pid.as_str());
    let accept = with(
        ids(&pid),
        &json!({"@type": "ContractNegotiationEventMessage", "eventType": "ACCEPTED"}),
    );
    let (s, _) = call(&app, post(&format!("{BASE}/{pid}/events"), &accept)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn counter_offer_on_an_existing_negotiation() {
    let sender = Arc::new(FakeSender::default());
    let mut p = ports(&sender);
    p.decider = Shared(Arc::new(WaitDecider));
    let app = app(p);
    let pid = start(&app).await;
    // REQUESTED -> REQUESTED is not a transition
    let mut again = request(CALLBACK);
    again["providerPid"] = json!(pid);
    let (s, _) = call(&app, post(&format!("{BASE}/{pid}/request"), &again)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // the path and the body must name the same negotiation
    again["providerPid"] = json!("urn:uuid:other");
    let (s, err) = call(&app, post(&format!("{BASE}/{pid}/request"), &again)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(err["reason"][0].as_str().unwrap().contains("path"));
    // and the consumer pid must be the one the negotiation started with
    let mut other_consumer_pid = request(CALLBACK);
    other_consumer_pid["providerPid"] = json!(pid);
    other_consumer_pid["consumerPid"] = json!("urn:uuid:consumer-2");
    let uri = format!("{BASE}/{pid}/request");
    let (s, err) = call(&app, post(&uri, &other_consumer_pid)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(err["reason"][0], "process ids do not match");
}

#[tokio::test]
async fn unknown_negotiation_is_404() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    let (s, err) = call(&app, get(&format!("{BASE}/urn:uuid:nope"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(err["@type"], "ContractNegotiationError");
    let verify = with(
        ids("urn:uuid:nope"),
        &json!({"@type": "ContractAgreementVerificationMessage"}),
    );
    let (s, _) = call(
        &app,
        post(
            &format!("{BASE}/urn:uuid:nope/agreement/verification"),
            &verify,
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_and_unknown_offers_are_400() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    let req = from(Request::post(format!("{BASE}/request")), CONSUMER)
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let (s, err) = call(&app, req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(err["@type"], "ContractNegotiationError");

    let mut msg = request(CALLBACK);
    msg["offer"]["@id"] = json!("CAT0101:offer:made-up");
    let (s, _) = call(&app, post(&format!("{BASE}/request"), &msg)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let mut msg = request(CALLBACK);
    msg.as_object_mut().unwrap().remove("consumerPid");
    let (s, _) = call(&app, post(&format!("{BASE}/request"), &msg)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(sender.count(), 0);
}

#[tokio::test]
async fn http_callbacks_need_the_flag() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    for bad in [
        "http://consumer.example/cb",
        "ftp://x",
        "https://",
        "https://u@h/",
    ] {
        let (s, err) = call(&app, post(&format!("{BASE}/request"), &request(bad))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
        assert!(err["reason"][0].as_str().unwrap().contains("https"));
    }
    let allowed = portitor_connector::router(&config(true), ports(&sender)).unwrap();
    let (s, _) = call(
        &allowed,
        post(
            &format!("{BASE}/request"),
            &request("http://127.0.0.1:9/cb"),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn unreachable_consumer_ends_in_termination() {
    let sender = Arc::new(FakeSender {
        fail: true,
        ..FakeSender::default()
    });
    let app = app(ports(&sender));
    let pid = start(&app).await;
    // two tries of the agreement, then two of the termination; the
    // termination's failure is ignored (DSP allows it)
    sent(&sender, 4).await;
    assert_eq!(sender.get(1).1["@type"], "ContractAgreementMessage");
    assert_eq!(
        sender.get(2).1["@type"],
        "ContractNegotiationTerminationMessage"
    );
    state(&app, &pid, "TERMINATED").await;
}

/// A consumer that answers 404 will not take the message: since 0.12 it
/// is also the answer of a partner that refuses our credentials (ADR
/// 0016). The agreement is sent once, not tried again, and the
/// negotiation ends (ADR 0007). The TCK build tries a 404 again
/// (`outbox.rs`, `tck_retries`).
#[tokio::test]
async fn a_consumer_that_answers_404_is_not_sent_the_message_again() {
    let sender = Arc::new(FakeSender {
        refuses_with: Some("HTTP 404 Not Found"),
        ..FakeSender::default()
    });
    let app = app(ports(&sender));
    let pid = start(&app).await;
    state(&app, &pid, "TERMINATED").await;

    // one try of the agreement and one of the termination; two each with `tck`
    let tries_each = if cfg!(feature = "tck") { 2 } else { 1 };
    sent(&sender, 2 * tries_each).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sender.count(), 2 * tries_each);
    assert_eq!(sender.get(0).1["@type"], "ContractAgreementMessage");
    assert_eq!(
        sender.get(2 * tries_each - 1).1["@type"],
        "ContractNegotiationTerminationMessage"
    );
}

/// The consumer's verification arrives while its acknowledgement of our
/// agreement is still on its way: it proves the agreement arrived, so it
/// is accepted at once (ADR 0007).
#[tokio::test]
async fn a_reply_racing_our_acknowledgement_proves_delivery() {
    let sender = Arc::new(FakeSender {
        hold: Some("/agreement"),
        ..FakeSender::default()
    });
    let app = app(ports(&sender));
    let pid = start(&app).await;
    sent(&sender, 1).await; // the agreement is held
    let verify = with(
        ids(&pid),
        &json!({"@type": "ContractAgreementVerificationMessage"}),
    );
    let (s, ack) = call(
        &app,
        post(&format!("{BASE}/{pid}/agreement/verification"), &verify),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{ack}");
    assert_eq!(ack["state"], "VERIFIED");
    sent(&sender, 2).await;
    assert_eq!(sender.get(1).1["eventType"], "FINALIZED");
    state(&app, &pid, "FINALIZED").await;
    // the late acknowledgement of the agreement changes nothing
    sender.release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(20)).await;
    state(&app, &pid, "FINALIZED").await;
    assert_eq!(sender.count(), 2);
}

/// The consumer took the agreement but its answer was lost: its state says
/// so, and the negotiation goes on instead of being terminated.
#[tokio::test]
async fn a_lost_answer_is_checked_with_the_consumer() {
    let sender = Arc::new(FakeSender {
        fail: true,
        reported: Some("AGREED"),
        ..FakeSender::default()
    });
    let app = app(ports(&sender));
    let pid = start(&app).await;
    state(&app, &pid, "AGREED").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        sender.count(),
        2,
        "two tries of the agreement, no termination"
    );
}

#[tokio::test]
async fn guard_checks_inbound_messages() {
    let sender = Arc::new(FakeSender::default());
    let mut p = ports(&sender);
    p.guard = Arc::new(Refuse(Direction::Inbound, "ContractRequestMessage"));
    let app = app(p);
    let (s, err) = call(&app, post(&format!("{BASE}/request"), &request(CALLBACK))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(err["consumerPid"], "urn:uuid:consumer-1");
    assert!(err["reason"][0].as_str().unwrap().contains("not allowed"));
}

#[tokio::test]
async fn guard_checks_outbound_messages() {
    let sender = Arc::new(FakeSender::default());
    let mut p = ports(&sender);
    p.guard = Arc::new(Refuse(Direction::Outbound, "ContractAgreementMessage"));
    let app = app(p);
    let pid = start(&app).await;
    // the agreement is never sent; the provider terminates instead
    sent(&sender, 1).await;
    assert_eq!(
        sender.get(0).1["@type"],
        "ContractNegotiationTerminationMessage"
    );
    state(&app, &pid, "TERMINATED").await;
}

#[tokio::test]
async fn only_the_consumer_that_started_a_negotiation_sees_it() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    let pid = start(&app).await;
    let uri = format!("{BASE}/{pid}");
    let anonymous = Request::get(&uri).body(Body::empty()).unwrap();
    let (s, err) = call(&app, anonymous).await;
    // DSP 2025-1, "Contract Negotiation HTTPS Binding", "Unauthorized
    // Access": a client that is not authorized gets 404
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(err["@type"], "ContractNegotiationError");
    assert_eq!(err["code"], "404");
    let stranger = from(Request::get(&uri), "did:web:stranger.example");
    let (s, _) = call(&app, stranger.body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, body) = call(&app, get(&uri)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
}

/// A stranger's messages about someone else's negotiation are answered
/// 404 and change nothing.
#[tokio::test]
async fn a_stranger_cannot_change_a_negotiation() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    let pid = start(&app).await;
    state(&app, &pid, "AGREED").await;
    let messages = [
        (
            "request",
            with(request(CALLBACK), &json!({"providerPid": pid})),
        ),
        (
            "events",
            with(
                ids(&pid),
                &json!({"@type": "ContractNegotiationEventMessage", "eventType": "ACCEPTED"}),
            ),
        ),
        (
            "agreement/verification",
            with(
                ids(&pid),
                &json!({"@type": "ContractAgreementVerificationMessage"}),
            ),
        ),
        (
            "termination",
            with(
                ids(&pid),
                &json!({"@type": "ContractNegotiationTerminationMessage", "code": "1"}),
            ),
        ),
    ];
    for (path, body) in messages {
        let uri = format!("{BASE}/{pid}/{path}");
        let (s, err) = call(&app, post_as(&uri, &body, "did:web:stranger.example")).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}: {err}");
        assert_eq!(err["@type"], "ContractNegotiationError", "{path}");
        state(&app, &pid, "AGREED").await;
    }
}

/// Policies are read strictly (ADR 0009, amendment of 2026-11-18): a
/// contract request whose offer has a term Portitor does not read gets
/// HTTP 400 with a `ContractNegotiationError` that names the field (DSP
/// 2025-1, Contract Negotiation HTTPS binding, "Error responses"), and
/// nothing changes: no negotiation, no message sent.
#[tokio::test]
async fn a_request_with_an_unknown_policy_term_is_refused_by_name() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));

    // a duty inside the permission: a real ODRL term, not enforceable here
    let mut with_duty = request(CALLBACK);
    with_duty["offer"]["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    // a field ODRL defines on a constraint
    let mut with_unit = request(CALLBACK);
    with_unit["offer"]["permission"][0]["constraint"] = json!([{"leftOperand": "dateTime",
        "operator": "lteq", "rightOperand": "2030-12-31", "unit": "http://example.com/unit"}]);
    // a field of the offer itself
    let mut with_uid = request(CALLBACK);
    with_uid["offer"]["uid"] = json!("urn:offer:1");

    for (message, field) in [(with_duty, "duty"), (with_unit, "unit"), (with_uid, "uid")] {
        let (status, error) = call(&app, post(&format!("{BASE}/request"), &message)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {error}");
        assert_eq!(error["@type"], "ContractNegotiationError");
        assert_eq!(error["consumerPid"], "urn:uuid:consumer-1");
        assert_eq!(error["code"], "400");
        let reason = error["reason"][0].to_string();
        assert!(
            reason.contains(&format!("unknown field `{field}`")),
            "{field}: {reason}"
        );
    }
    assert_eq!(sender.count(), 0);

    // the same on a negotiation that exists: its state and offer stay
    let mut waiting = ports(&sender);
    waiting.decider = Shared(Arc::new(WaitDecider));
    let app = portitor_connector::router(&config(false), waiting).unwrap();
    let pid = start(&app).await;
    let mut counter = with(request(CALLBACK), &json!({"providerPid": pid}));
    counter["offer"]["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    let (status, error) = call(&app, post(&format!("{BASE}/{pid}/request"), &counter)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["providerPid"], pid);
    assert!(
        error["reason"][0]
            .to_string()
            .contains("unknown field `duty`")
    );
    state(&app, &pid, "REQUESTED").await;
    assert_eq!(sender.count(), 0);
}

/// An offer may name its parties (Eclipse EDC sends `assigner`, the DSP
/// TCK both). They are read and checked: another provider or another
/// consumer is refused with HTTP 400, and the right ones are accepted.
#[tokio::test]
async fn the_parties_an_offer_names_must_be_the_provider_and_the_caller() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));

    let mut other_provider = request(CALLBACK);
    other_provider["offer"]["assigner"] = json!("did:web:someone-else.example");
    let (status, error) = call(&app, post(&format!("{BASE}/request"), &other_provider)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(
        error["reason"][0],
        "the offer's assigner is not the provider"
    );

    let mut other_consumer = request(CALLBACK);
    other_consumer["offer"]["assignee"] = json!("did:web:someone-else.example");
    let (status, error) = call(&app, post(&format!("{BASE}/request"), &other_consumer)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(
        error["reason"][0],
        "the offer's assignee is not the consumer"
    );
    assert_eq!(sender.count(), 0);

    // as Eclipse EDC sends it: empty rule lists, the assigner and the target
    let mut named = request(CALLBACK);
    named["offer"]["assigner"] = json!(PROVIDER);
    named["offer"]["assignee"] = json!(CONSUMER);
    named["offer"]["prohibition"] = json!([]);
    named["offer"]["obligation"] = json!([]);
    let (status, ack) = call(&app, post(&format!("{BASE}/request"), &named)).await;
    assert_eq!(status, StatusCode::CREATED, "{ack}");
    sent(&sender, 1).await;
    let (url, agreement) = sender.get(0);
    assert!(url.ends_with("/agreement"), "{url}");
    assert_eq!(agreement["agreement"]["assigner"], PROVIDER);
    assert_eq!(agreement["agreement"]["assignee"], CONSUMER);
}

/// A counter-request that names another `consumerPid` is refused (TD-36)
/// and changes nothing: the negotiation keeps its consumer pid and its
/// state, and nothing is sent.
#[tokio::test]
async fn a_counter_request_with_another_consumer_pid_changes_nothing() {
    let sender = Arc::new(FakeSender::default());
    let mut waiting = ports(&sender);
    waiting.decider = Shared(Arc::new(WaitDecider));
    let app = app(waiting);
    let pid = start(&app).await;

    let other_pid = json!({"providerPid": pid, "consumerPid": "urn:uuid:consumer-2"});
    let counter = with(request(CALLBACK), &other_pid);
    let (status, error) = call(&app, post(&format!("{BASE}/{pid}/request"), &counter)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["@type"], "ContractNegotiationError");
    assert_eq!(error["code"], "400");
    assert_eq!(error["reason"][0], "process ids do not match");

    let (status, negotiation) = call(&app, get(&format!("{BASE}/{pid}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(negotiation["consumerPid"], "urn:uuid:consumer-1");
    assert_eq!(negotiation["state"], "REQUESTED");
    assert_eq!(sender.count(), 0);
}

/// A message without a token changes nothing: the negotiation it names
/// keeps its state (DSP 2025-1, "Unauthorized Access": 404).
#[tokio::test]
async fn a_message_without_a_token_does_not_change_a_negotiation() {
    let sender = Arc::new(FakeSender::default());
    let mut waiting = ports(&sender);
    waiting.decider = Shared(Arc::new(WaitDecider));
    let app = app(waiting);
    let pid = start(&app).await;

    let terminate = with(
        ids(&pid),
        &json!({"@type": "ContractNegotiationTerminationMessage"}),
    );
    let anonymous = Request::post(format!("{BASE}/{pid}/termination"))
        .header("content-type", "application/json")
        .body(Body::from(terminate.to_string()))
        .unwrap();
    let (status, error) = call(&app, anonymous).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["reason"][0], "unauthorized");
    // the answer is the one for a negotiation that does not exist
    assert_eq!(error["providerPid"], "");
    state(&app, &pid, "REQUESTED").await;
    assert_eq!(sender.count(), 0);
}

/// The body of a contract request as text, with `offer` in place of the
/// offer: only text can hold one key twice.
fn request_text(offer: &str) -> String {
    format!(
        r#"{{"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "ContractRequestMessage", "consumerPid": "urn:uuid:consumer-1",
            "callbackAddress": "{CALLBACK}", "offer": {offer}}}"#
    )
}

/// A policy field given twice would have one of its two values dropped
/// unseen, and the two parties may not keep the same one (ADR 0009,
/// amendment of 2026-11-18: no term is dropped). The request is refused
/// and no negotiation starts.
#[tokio::test]
async fn a_request_with_a_policy_field_given_twice_is_refused() {
    let sender = Arc::new(FakeSender::default());
    let app = app(ports(&sender));
    let permission_twice = r#"{"@id": "CAT0101:offer:use", "@type": "Offer", "target": "CAT0101",
        "permission": [{"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2020-12-31"}]}],
        "permission": [{"action": "use"}]}"#;
    let request = from(Request::post(format!("{BASE}/request")), CONSUMER)
        .header("content-type", "application/json")
        .body(Body::from(request_text(permission_twice)))
        .unwrap();
    let (status, answer) = call(&app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(answer["@type"], "ContractNegotiationError");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sender.count(), 0);
}

/// A port to the consumer with a bug: sending panics while `.1` is set.
/// Otherwise it is the sender in `.0`.
struct PanicsWhileSet(Arc<FakeSender>, std::sync::atomic::AtomicBool);

impl Sender for PanicsWhileSet {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        let panic_now = self.1.load(std::sync::atomic::Ordering::SeqCst);
        assert!(!panic_now, "a bug in the sender");
        self.0.post(url, body)
    }

    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        // the consumer's state, asked through the trait (`FakeSender` has
        // a `get` of its own for the messages it recorded)
        Sender::get(self.0.as_ref(), url)
    }
}

/// A panic while a message is sent (TD-41) ends only that delivery
/// (`outbox.rs`, `deliver`): the request was answered, the server goes
/// on, the negotiation keeps its state, and the message stays saved. It
/// is sent at the next start, as after any stop (ADR 0007).
#[tokio::test]
async fn a_send_that_panics_leaves_the_message_saved_for_the_next_start() {
    use portitor_connector::store::SqliteNegotiations;
    use portitor_dsp::negotiation::provider::NegotiationStore as _;

    let sender = Arc::new(FakeSender::default());
    let with_a_bug = Arc::new(PanicsWhileSet(
        Arc::clone(&sender),
        std::sync::atomic::AtomicBool::new(true),
    ));
    let file = Arc::new(SqliteNegotiations::in_memory().unwrap());
    let (mut p, _) = recording(Ports::new(with_a_bug.clone() as Arc<dyn Sender>), PROVIDER);
    p.use_sqlite(&file);
    let (app, connector) = portitor_connector::build(&config(false), p).unwrap();

    let pid = start(&app).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    state(&app, &pid, "REQUESTED").await;
    assert_eq!(sender.count(), 0, "nothing went out");
    let saved = file.pending().unwrap();
    assert_eq!(saved.len(), 1, "the agreement is still to be sent");
    assert_eq!(saved[0].provider_pid, pid);

    // the next start, with the bug gone
    with_a_bug
        .1
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(connector.resume().unwrap(), 1);
    state(&app, &pid, "AGREED").await;
    assert_eq!(sender.count(), 1);
    assert_eq!(sender.get(0).1["@type"], "ContractAgreementMessage");
}
