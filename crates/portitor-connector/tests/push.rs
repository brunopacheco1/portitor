// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Push transfers (ADR 0027) between two Portitor connectors in process:
//! the consumer asks for `HttpData-PUSH` with its own data address, the
//! provider's data plane sends the dataset to it, and the consumer's
//! receiving route stores it in a fake S3-compatible store
//! (`common/fake_s3.rs`).
//!
//! DSP messages go through the fake network (`common/network.rs`); the
//! push itself is real HTTP: the provider sends it to the consumer's
//! receiving route on a local port, or to a fake endpoint. A test hop in
//! front of the network (`Hop`) records the consumer's messages and can
//! change the data address in them, to aim the push elsewhere or to
//! send a wrong one.
//!
//! Also the pull of a dataset whose source is in the provider's object
//! storage (`s3://{bucket}/{key}`, ADR 0027 section 3).
#![cfg(feature = "data-plane")]
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::IntoResponse;
use hyper::body::Bytes;
use portitor_connector::http_client::Sender;
use portitor_connector::management::ApiKey;
use portitor_connector::{Config, Negotiator};
use portitor_dsp::negotiation::NegotiationState;
use portitor_dsp::transfer::TransferState;
use portitor_dsp::transfer::consumer::ConsumerTransferDecision;
use portitor_runtime::BoxFuture;
use portitor_runtime::database::DatabaseSetting;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tower::ServiceExt as _;

mod common;
use common::fake_s3::{self, FakeS3, PUSH_BUCKET};
use common::network::Network;
use common::{
    CONSUMER as CONSUMER_DID, PROVIDER as PROVIDER_DID, Revocable, authenticating, bearer,
};

const PROVIDER: &str = "https://provider.example";
const CONSUMER: &str = "https://consumer.example";
const DSP: &str = "/dsp/2025-1";
const DATA: &str = "day,celsius\nmon,12\n";
const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";

/// A real HTTP server holding the dataset's data, as `text/csv`.
async fn upstream() -> String {
    let app = Router::new().route(
        "/weather.csv",
        axum::routing::get(|| async { ([("content-type", "text/csv")], DATA) }),
    );
    let url = serve(app).await;
    format!("{url}/weather.csv")
}

/// A source that sends the first bytes of its answer, then nothing more,
/// and never ends: a push of it runs until it is cut.
async fn endless_upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/endless.csv", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request).await;
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/csv\r\n\
                            transfer-encoding: chunked\r\n\r\n4\r\nday\n\r\n";
                socket.write_all(head.as_bytes()).await.unwrap();
                // keep the connection open, and send nothing more
                tokio::time::sleep(Duration::from_secs(3_600)).await;
            });
        }
    });
    url
}

/// Serves `app` on a local port; returns its base URL.
async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn config(participant: &str, public_url: &str, allow_http: bool) -> Config {
    Config {
        participant_id: participant.to_owned(),
        public_url: public_url.to_owned(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http,
        database: DatabaseSetting::SqliteFile("unused.db".into()),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// A push request as a fake endpoint got it.
#[derive(Debug, Clone)]
struct Received {
    authorization: Option<String>,
    content_type: Option<String>,
    /// The body, or `None` when it broke off or is still coming.
    body: Option<String>,
    /// Whether the body ended (whole or broken off).
    ended: bool,
}

impl Received {
    /// Cut by the sender: the body broke off.
    fn cut(&self) -> bool {
        self.ended && self.body.is_none()
    }
}

/// A fake push endpoint: records each request and answers `status`.
#[derive(Clone)]
struct FakeEndpoint {
    status: StatusCode,
    received: Arc<Mutex<Vec<Received>>>,
}

async fn take(
    State(fake): State<FakeEndpoint>,
    headers: HeaderMap,
    body: Body,
) -> impl IntoResponse {
    let text = |name| {
        headers
            .get(name)
            .map(|v: &header::HeaderValue| v.to_str().unwrap().to_owned())
    };
    let mut received = Received {
        authorization: text(header::AUTHORIZATION),
        content_type: text(header::CONTENT_TYPE),
        body: None,
        ended: false,
    };
    // recorded at once, so a test sees a push that is still running
    let index = {
        let mut all = fake.received.lock().unwrap();
        all.push(received.clone());
        all.len() - 1
    };
    received.body = to_bytes(body, 1 << 20)
        .await
        .ok()
        .map(|b| String::from_utf8(b.to_vec()).unwrap());
    received.ended = true;
    fake.received.lock().unwrap()[index] = received;
    fake.status
}

/// A fake endpoint answering `status`; returns its push URL and its
/// record.
async fn fake_endpoint(status: StatusCode) -> (String, Arc<Mutex<Vec<Received>>>) {
    let fake = FakeEndpoint {
        status,
        received: Arc::default(),
    };
    let received = Arc::clone(&fake.received);
    let app = Router::new()
        .route("/in", axum::routing::post(take))
        .with_state(fake);
    (format!("{}/in", serve(app).await), received)
}

/// What the hop does to the data address of the consumer's messages.
type Change = Box<dyn Fn(&mut Value) + Send + Sync>;

/// The consumer's hop in front of the network: records each message it
/// sends (before any change), changes the `dataAddress` of its transfer
/// request and start messages with `change`, and records the answers.
struct Hop {
    network: Arc<Network>,
    change: Mutex<Change>,
    sent: Mutex<Vec<Value>>,
    answers: Mutex<Vec<Result<Value, String>>>,
}

impl Hop {
    fn new(network: &Arc<Network>) -> Arc<Self> {
        Arc::new(Self {
            network: Arc::clone(network),
            change: Mutex::new(Box::new(|_| {})),
            sent: Mutex::default(),
            answers: Mutex::default(),
        })
    }

    fn change(&self, change: impl Fn(&mut Value) + Send + Sync + 'static) {
        *self.change.lock().unwrap() = Box::new(change);
    }

    /// The `nth` message (from 0) with a data address that the consumer
    /// sent, once it is sent (they go in the background).
    async fn with_address(&self, nth: usize) -> Value {
        let read = || {
            let sent = self.sent.lock().unwrap();
            sent.iter()
                .filter(|m| m.get("dataAddress").is_some())
                .nth(nth)
                .cloned()
        };
        until(read, Option::is_some).await.unwrap()
    }

    /// The bearer token of the `nth` data address the consumer sent.
    async fn token(&self, nth: usize) -> String {
        token_of(&self.with_address(nth).await["dataAddress"])
    }
}

fn token_of(address: &Value) -> String {
    address["endpointProperties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "authorization")
        .unwrap()["value"]
        .as_str()
        .unwrap()
        .to_owned()
}

impl Sender for Hop {
    fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("a DSP message without a token".to_owned()) })
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(body.clone());
            let mut body = body.clone();
            if let Some(address) = body.get_mut("dataAddress") {
                (self.change.lock().unwrap())(address);
            }
            let answer = self
                .network
                .post_authorized(url, &body, authorization)
                .await;
            self.answers.lock().unwrap().push(answer.clone());
            answer
        })
    }

    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.network.get_authorized(url, authorization)
    }
}

/// Two connectors, both with object storage in `s3`.
struct Parties {
    network: Arc<Network>,
    provider: Router,
    /// The provider's management API, with [`KEY`].
    management: Router,
    consumer: Negotiator,
    hop: Arc<Hop>,
    s3: Arc<FakeS3>,
    /// The consumer's receiving route on a local port: `http://...`.
    receiving: String,
}

/// The provider offers `weather` for pull and push, from `source`; it
/// takes `http://` endpoints when `allow_http`.
async fn parties(source: String, allow_http: bool) -> Parties {
    parties_lagging(source, allow_http, 0).await
}

/// [`parties`], with a consumer that applies the provider's start only
/// after it has been asked for its state `lag` times ([`Lagging`]).
async fn parties_lagging(source: String, allow_http: bool, lag: u32) -> Parties {
    parties_with(source, allow_http, lag, None).await
}

/// [`parties_lagging`]; the provider checks the consumer's membership
/// with `verifier` when one is given.
async fn parties_with(
    source: String,
    allow_http: bool,
    lag: u32,
    verifier: Option<Revocable>,
) -> Parties {
    let network = Arc::new(Network::default());
    let lagging = Arc::new(Lagging {
        network: Arc::clone(&network),
        lag: AtomicU32::new(lag),
    });
    let mut provider_ports =
        authenticating(common::ports(lagging as Arc<dyn Sender>), PROVIDER_DID);
    if let Some(verifier) = verifier {
        provider_ports.verifier = Arc::new(verifier);
    }
    provider_ports.harvest.source = Some(Arc::new(Datasets(json!([{
        "@id": "weather",
        "@type": "Dataset",
        "hasPolicy": [{"@id": "weather:offer", "@type": "Offer", "permission": [{"action": "use"}]}],
        "distribution": [
            {"@type": "Distribution", "format": "HttpData-PULL"},
            {"@type": "Distribution", "format": "HttpData-PUSH"}
        ],
        "portitor:source": source
    }]))));
    let (s3_url, s3) = fake_s3::start().await;
    provider_ports.object_storage = Some(Arc::new(fake_s3::client(&s3_url)));
    let (provider_app, provider) =
        portitor_connector::build(&config(PROVIDER_DID, PROVIDER, allow_http), provider_ports)
            .unwrap();
    let hop = Hop::new(&network);
    let mut consumer_ports = authenticating(common::ports(Arc::clone(&hop) as _), CONSUMER_DID);
    consumer_ports.object_storage = Some(Arc::new(fake_s3::client(&s3_url)));
    let (consumer_app, consumer) =
        portitor_connector::build(&config(CONSUMER_DID, CONSUMER, false), consumer_ports).unwrap();
    let receiving = serve(consumer.receiving_routes()).await;
    network.join(PROVIDER, provider_app.clone());
    network.join(CONSUMER, consumer_app);
    let management = portitor_connector::management::router(provider, ApiKey::new(KEY).unwrap());
    Parties {
        network,
        provider: provider_app,
        management,
        consumer,
        hop,
        s3,
        receiving,
    }
}

/// The provider's view of a consumer that acknowledges a start before it
/// applies it, as Eclipse EDC does: while `lag` is above zero, each state
/// read answers `REQUESTED` (and lowers `lag`), and a completion is
/// refused with 400.
struct Lagging {
    network: Arc<Network>,
    lag: AtomicU32,
}

impl Lagging {
    fn lagging(&self) -> bool {
        self.lag.load(Ordering::SeqCst) > 0
    }
}

impl Sender for Lagging {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.network.post(url, body)
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        if body["@type"] == "TransferCompletionMessage" && self.lagging() {
            return Box::pin(async { Err("HTTP 400 Bad Request".to_owned()) });
        }
        self.network.post_authorized(url, body, authorization)
    }

    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let mut state = self.network.get_authorized(url, authorization).await?;
            if self.lagging() {
                self.lag.fetch_sub(1, Ordering::SeqCst);
                state["state"] = json!("REQUESTED");
            }
            Ok(state)
        })
    }
}

/// A fixed datasets source.
struct Datasets(Value);

impl portitor_connector::harvest::DatasetSource for Datasets {
    fn name(&self) -> String {
        "fixed".to_owned()
    }
    fn snapshot(
        &self,
    ) -> Result<portitor_connector::harvest::Snapshot, portitor_connector::harvest::SourceError>
    {
        Ok(portitor_connector::harvest::Snapshot {
            sha256: self.0.to_string(),
            entries: self.0.as_array().unwrap().clone(),
        })
    }
}

impl Parties {
    /// Agrees a contract for `weather`; returns the agreement id.
    async fn agree(&self) -> String {
        let catalog = format!("{PROVIDER}{DSP}/catalog/datasets/weather");
        let dataset = self
            .network
            .get_authorized(&catalog, &bearer(CONSUMER_DID))
            .await
            .unwrap();
        let mut offer: portitor_dsp::Offer =
            serde_json::from_value(dataset["hasPolicy"][0].clone()).unwrap();
        offer.target = Some("weather".to_owned());
        let n = self
            .consumer
            .start(&format!("{PROVIDER}{DSP}"), PROVIDER_DID, offer)
            .unwrap();
        let n = until(
            || self.consumer.get(&n.consumer_pid).unwrap(),
            |n| n.state == NegotiationState::Finalized,
        )
        .await;
        n.agreement.unwrap().id
    }

    /// Sends the push to the consumer's receiving route on its local
    /// port (the address the consumer gives names its public URL).
    fn aim_at_the_receiving_route(&self) {
        let local = format!("{}/push/", self.receiving);
        self.hop.change(move |address| {
            let endpoint = address["endpoint"].as_str().unwrap();
            let path = endpoint.strip_prefix(&format!("{CONSUMER}/push/")).unwrap();
            address["endpoint"] = json!(format!("{local}{path}"));
        });
    }

    /// Sends the push to `endpoint`.
    fn aim_at(&self, endpoint: &str) {
        let endpoint = endpoint.to_owned();
        self.hop
            .change(move |address| address["endpoint"] = json!(endpoint));
    }

    fn state(&self, consumer_pid: &str) -> TransferState {
        self.consumer.transfer(consumer_pid).unwrap().state
    }

    async fn until_state(&self, consumer_pid: &str, state: TransferState) {
        until(|| self.state(consumer_pid), |s| *s == state).await;
    }

    /// `POST` of `body` to the consumer's receiving route for
    /// `consumer_pid`, with `token` if any.
    async fn push_to_consumer(&self, consumer_pid: &str, token: Option<&str>) -> StatusCode {
        let mut request = Request::post(format!("/push/{consumer_pid}"));
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = request.body(Body::from("pushed")).unwrap();
        let answer = self
            .consumer
            .receiving_routes()
            .oneshot(request)
            .await
            .unwrap();
        answer.status()
    }
}

/// Polls `read` until `done` says yes, for up to five seconds.
async fn until<T: std::fmt::Debug>(read: impl Fn() -> T, done: impl Fn(&T) -> bool) -> T {
    for _ in 0..500 {
        let value = read();
        if done(&value) {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("still {:?}", read());
}

#[tokio::test]
async fn a_push_lands_in_the_consumers_bucket_and_completes_the_transfer() {
    let p = parties(upstream().await, true).await;
    let agreement = p.agree().await;
    p.aim_at_the_receiving_route();
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    // the address the consumer sent: its receiving route, a bearer token
    let request = p.hop.with_address(0).await;
    assert_eq!(request["@type"], "TransferRequestMessage");
    let address = &request["dataAddress"];
    assert_eq!(
        address["endpoint"],
        format!("{CONSUMER}/push/{}", t.consumer_pid)
    );
    assert_eq!(
        address["endpointType"],
        "https://w3id.org/dspace-sig/profile/http-push"
    );
    let token = token_of(address);
    assert_eq!(token.len(), 43, "32 bytes, base64url without padding");

    p.until_state(&t.consumer_pid, TransferState::Completed)
        .await;
    let stored = p.s3.object(PUSH_BUCKET, &t.consumer_pid).unwrap();
    assert_eq!(stored.bytes, DATA.as_bytes());
    assert_eq!(stored.content_type.as_deref(), Some("text/csv"));
    assert_eq!(
        p.consumer.object_key(&t.consumer_pid),
        Some(t.consumer_pid.clone())
    );
    // a push transfer shows no data address of anyone's
    assert_eq!(
        p.consumer.transfer(&t.consumer_pid).unwrap().data_address,
        None
    );
    // the transfer has ended: its token no longer works
    assert_eq!(
        p.push_to_consumer(&t.consumer_pid, Some(&token)).await,
        StatusCode::UNAUTHORIZED
    );
}

/// The provider's audit line of a push names the agreement and the
/// consumer that the data flow kept from its start message (ADR 0028,
/// step 1).
#[tokio::test]
async fn a_sent_push_writes_its_agreement_and_consumer_in_the_audit_line() {
    let p = parties(upstream().await, true).await;
    let (log, _guard) = Log::start();
    let agreement = p.agree().await;
    p.aim_at_the_receiving_route();
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    p.until_state(&t.consumer_pid, TransferState::Completed)
        .await;
    let sent: Vec<Value> = log
        .with_message("data transfer done")
        .into_iter()
        .filter(|line| line["fields"]["direction"] == "sent")
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let fields = &sent[0]["fields"];
    assert_eq!(fields["agreement"], agreement.as_str());
    assert_eq!(fields["counterParty"], CONSUMER_DID);
    assert_eq!(fields["outcome"], "complete");
    assert_eq!(fields["bytes"], DATA.len());
}

#[tokio::test]
async fn the_receiving_route_takes_only_the_token_of_a_running_transfer() {
    // a push that never ends, to a fake endpoint: the transfer stays
    // STARTED
    let p = parties(endless_upstream().await, true).await;
    let agreement = p.agree().await;
    let (endpoint, _) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    let token = p.hop.token(0).await;
    let cpid = t.consumer_pid.as_str();
    p.until_state(cpid, TransferState::Started).await;

    // every refusal is the same 401
    assert_eq!(
        p.push_to_consumer(cpid, None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        p.push_to_consumer(cpid, Some("wrong")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        p.push_to_consumer("urn:uuid:unknown", Some(&token)).await,
        StatusCode::UNAUTHORIZED
    );
    assert!(p.s3.object(PUSH_BUCKET, cpid).is_none());

    // the right token, while the transfer runs: stored, and a second
    // push replaces the object
    assert_eq!(
        p.push_to_consumer(cpid, Some(&token)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        p.push_to_consumer(cpid, Some(&token)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(p.s3.object(PUSH_BUCKET, cpid).unwrap().bytes, b"pushed");
    assert_eq!(p.consumer.object_key(cpid), Some(cpid.to_owned()));
}

#[tokio::test]
async fn a_push_waits_until_the_consumer_has_applied_the_start() {
    // without the wait, the completion would come while the consumer
    // still says REQUESTED, be refused, and end the transfer
    let p = parties_lagging(upstream().await, true, 3).await;
    let agreement = p.agree().await;
    p.aim_at_the_receiving_route();
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    p.until_state(&t.consumer_pid, TransferState::Completed)
        .await;
    let stored = p.s3.object(PUSH_BUCKET, &t.consumer_pid).unwrap();
    assert_eq!(stored.bytes, DATA.as_bytes());
}

#[tokio::test]
async fn a_push_the_endpoint_refuses_terminates_the_transfer() {
    let p = parties(upstream().await, true).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::INTERNAL_SERVER_ERROR).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    let token = p.hop.token(0).await;
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
    let received = received.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].authorization.as_deref(),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(received[0].content_type.as_deref(), Some("text/csv"));
    assert_eq!(received[0].body.as_deref(), Some(DATA));
}

#[tokio::test]
async fn an_http_endpoint_is_refused_without_allow_http() {
    let p = parties(upstream().await, false).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
    let answers = p.hop.answers.lock().unwrap().clone();
    let refusal = answers.last().unwrap().as_ref().unwrap_err();
    assert!(refusal.starts_with("400"), "{refusal}");
    assert!(refusal.contains("must be an https URL"), "{refusal}");
    assert!(received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_wrong_endpoint_type_or_a_missing_endpoint_is_refused() {
    for change in [
        Box::new(|a: &mut Value| a["endpointType"] = json!("https://w3id.org/idsa/v4.1/HTTP"))
            as Change,
        Box::new(|a: &mut Value| {
            a.as_object_mut().unwrap().remove("endpoint");
        }),
    ] {
        let p = parties(upstream().await, true).await;
        let agreement = p.agree().await;
        p.hop.change(change);
        let t = p
            .consumer
            .request_transfer(&agreement, "HttpData-PUSH")
            .unwrap();
        p.until_state(&t.consumer_pid, TransferState::Terminated)
            .await;
        let answers = p.hop.answers.lock().unwrap().clone();
        let refusal = answers.last().unwrap().as_ref().unwrap_err();
        assert!(refusal.starts_with("400"), "{refusal}");
        assert!(p.s3.object(PUSH_BUCKET, &t.consumer_pid).is_none());
    }
}

#[tokio::test]
async fn a_suspension_cuts_a_running_push_and_a_start_again_sends_it_again() {
    let p = parties(endless_upstream().await, true).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    let cpid = t.consumer_pid.as_str();
    let first_token = p.hop.token(0).await;
    p.until_state(cpid, TransferState::Started).await;
    until(|| received.lock().unwrap().len(), |n| *n == 1).await;

    p.consumer
        .act_on_transfer(cpid, ConsumerTransferDecision::Suspend)
        .unwrap();
    p.until_state(cpid, TransferState::Suspended).await;
    // the push was cut: its body broke off
    until(|| received.lock().unwrap()[0].cut(), |cut| *cut).await;
    // a suspended transfer's token does not work on the receiving route
    assert_eq!(
        p.push_to_consumer(cpid, Some(&first_token)).await,
        StatusCode::UNAUTHORIZED
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(p.state(cpid), TransferState::Suspended, "nothing more");

    // start again: a new token in a new address, and the push again
    p.consumer
        .act_on_transfer(cpid, ConsumerTransferDecision::Start)
        .unwrap();
    p.until_state(cpid, TransferState::Started).await;
    let second_token = p.hop.token(1).await;
    assert_ne!(second_token, first_token);
    until(|| received.lock().unwrap().len(), |n| *n == 2).await;
    assert_eq!(
        received.lock().unwrap()[1].authorization.as_deref(),
        Some(format!("Bearer {second_token}").as_str())
    );
    // the old token is gone; the new one works while the transfer runs
    assert_eq!(
        p.push_to_consumer(cpid, Some(&first_token)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        p.push_to_consumer(cpid, Some(&second_token)).await,
        StatusCode::NO_CONTENT
    );

    // terminate: the running push is cut too
    p.consumer
        .act_on_transfer(cpid, ConsumerTransferDecision::Terminate)
        .unwrap();
    p.until_state(cpid, TransferState::Terminated).await;
    until(|| received.lock().unwrap()[1].cut(), |cut| *cut).await;
}

#[tokio::test]
async fn a_push_needs_object_storage_and_a_push_bucket() {
    let network = Arc::new(Network::default());
    let ports = authenticating(
        common::ports(Arc::clone(&network) as Arc<dyn Sender>),
        CONSUMER_DID,
    );
    let (_, consumer) =
        portitor_connector::build(&config(CONSUMER_DID, CONSUMER, false), ports).unwrap();
    assert!(
        !consumer.receiving_routes().has_routes(),
        "no receiving route"
    );
    let refused = consumer
        .request_transfer("urn:uuid:agreement", "HttpData-PUSH")
        .unwrap_err();
    assert!(
        format!("{refused:?}").contains("PORTITOR_PUSH_BUCKET"),
        "{refused:?}"
    );
}

/// `GET` of a pull transfer's data address with its token.
async fn fetch(p: &Parties, consumer_pid: &str) -> (StatusCode, Option<String>, String) {
    let address = p
        .consumer
        .transfer(consumer_pid)
        .unwrap()
        .data_address
        .unwrap();
    let path = address
        .endpoint
        .as_deref()
        .unwrap()
        .strip_prefix(PROVIDER)
        .unwrap()
        .to_owned();
    let token = address.property("authorization").unwrap();
    let request = Request::get(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let answer = p.provider.clone().oneshot(request).await.unwrap();
    let status = answer.status();
    let content_type = answer
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(answer.into_body(), 1 << 20).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn a_pull_reads_the_providers_object_storage() {
    let p = parties("s3://datasets/weather.csv".to_owned(), false).await;
    p.s3.put("datasets", "weather.csv", Some("text/csv"), DATA.as_bytes());
    let agreement = p.agree().await;
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PULL")
        .unwrap();
    p.until_state(&t.consumer_pid, TransferState::Started).await;
    assert_eq!(
        fetch(&p, &t.consumer_pid).await,
        (StatusCode::OK, Some("text/csv".to_owned()), DATA.to_owned())
    );
}

#[tokio::test]
async fn a_missing_object_is_a_bad_gateway_as_for_an_http_source() {
    let p = parties("s3://datasets/missing.csv".to_owned(), false).await;
    let agreement = p.agree().await;
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PULL")
        .unwrap();
    p.until_state(&t.consumer_pid, TransferState::Started).await;
    assert_eq!(fetch(&p, &t.consumer_pid).await.0, StatusCode::BAD_GATEWAY);
}

/// The provider pid of the consumer's transfer `consumer_pid`.
fn provider_pid(p: &Parties, consumer_pid: &str) -> String {
    p.consumer
        .transfer(consumer_pid)
        .unwrap()
        .provider_pid
        .unwrap()
}

/// A provider that checks the membership with [`Revocable`], every 50 ms
/// while a push runs; and the verifier's switches: revoked, unreachable.
async fn parties_checking(source: String, lag: u32) -> (Parties, Arc<AtomicBool>, Arc<AtomicBool>) {
    let (verifier, revoked) = Revocable::new(Duration::from_millis(50));
    let unreachable = Arc::clone(&verifier.unreachable);
    let p = parties_with(source, true, lag, Some(verifier)).await;
    (p, revoked, unreachable)
}

/// ADR 0027, section 7.2: a membership revoked after the transfer was
/// accepted, before the push goes, ends the transfer; nothing is sent.
#[tokio::test]
async fn a_membership_revoked_before_the_push_ends_the_transfer() {
    // the consumer applies the start late, so the push waits a moment
    let (p, revoked, _) = parties_checking(upstream().await, 3).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let answered = p.hop.answers.lock().unwrap().len();
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    // the provider has accepted the request
    let answers = until(
        || p.hop.answers.lock().unwrap().clone(),
        |a| a.len() > answered,
    )
    .await;
    assert!(answers[answered].is_ok(), "{answers:?}");
    revoked.store(true, Ordering::SeqCst);
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
    assert!(received.lock().unwrap().is_empty(), "nothing was pushed");
}

/// ADR 0027, section 7.2: a membership revoked while the push runs cuts
/// it, and the transfer ends.
#[tokio::test]
async fn a_membership_revoked_during_a_push_ends_the_transfer() {
    let (p, revoked, _) = parties_checking(endless_upstream().await, 0).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    until(|| received.lock().unwrap().len(), |n| *n == 1).await;
    revoked.store(true, Ordering::SeqCst);
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
    until(|| received.lock().unwrap()[0].cut(), |cut| *cut).await;
}

/// A membership check that cannot finish while the push runs cuts the
/// push. The transfer must not stay `STARTED` with nothing sending: it
/// is terminated, as for any failed push.
#[tokio::test]
async fn a_membership_check_that_cannot_finish_ends_a_running_push() {
    let (p, _, unreachable) = parties_checking(endless_upstream().await, 0).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    until(|| received.lock().unwrap().len(), |n| *n == 1).await;
    unreachable.store(true, Ordering::SeqCst);
    until(|| received.lock().unwrap()[0].cut(), |cut| *cut).await;
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
    let pid = provider_pid(&p, &t.consumer_pid);
    let view = p.provided_transfer(&pid).await;
    assert_eq!(view["state"], "TERMINATED", "{view}");
}

/// ADR 0027, section 7.1: the provider's own termination cuts a running
/// push at once.
#[tokio::test]
async fn the_providers_termination_cuts_a_running_push() {
    let p = parties(endless_upstream().await, true).await;
    let agreement = p.agree().await;
    let (endpoint, received) = fake_endpoint(StatusCode::OK).await;
    p.aim_at(&endpoint);
    let t = p
        .consumer
        .request_transfer(&agreement, "HttpData-PUSH")
        .unwrap();
    until(|| received.lock().unwrap().len(), |n| *n == 1).await;
    let pid = provider_pid(&p, &t.consumer_pid);
    let request = Request::post(format!("/v1/provided-transfers/{pid}/termination"))
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .body(Body::empty())
        .unwrap();
    let answer = p.management.clone().oneshot(request).await.unwrap();
    assert_eq!(answer.status(), StatusCode::ACCEPTED);
    until(|| received.lock().unwrap()[0].cut(), |cut| *cut).await;
    p.until_state(&t.consumer_pid, TransferState::Terminated)
        .await;
}

/// A request body that gives what the test sends into its channel, and
/// ends when the channel is closed.
struct Streamed(tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>);

impl hyper::body::Body for Streamed {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, std::io::Error>>> {
        self.get_mut()
            .0
            .poll_recv(context)
            .map(|chunk| chunk.map(|chunk| chunk.map(hyper::body::Frame::data)))
    }
}

/// What the test sends into a [`Streamed`] body.
type Chunks = tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>;

impl Parties {
    /// The provider's view of its transfer `pid`.
    async fn provided_transfer(&self, pid: &str) -> Value {
        let request = Request::get(format!("/v1/provided-transfers/{pid}"))
            .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
            .body(Body::empty())
            .unwrap();
        let answer = self.management.clone().oneshot(request).await.unwrap();
        let bytes = to_bytes(answer.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// A push transfer, as consumer, that is `STARTED`; the provider's
    /// own push runs to a fake endpoint, forever. Returns the consumer pid
    /// and the token of the receiving route.
    async fn receiving_transfer(&self) -> (String, String) {
        let agreement = self.agree().await;
        let (endpoint, _) = fake_endpoint(StatusCode::OK).await;
        self.aim_at(&endpoint);
        let t = self
            .consumer
            .request_transfer(&agreement, "HttpData-PUSH")
            .unwrap();
        let token = self.hop.token(0).await;
        self.until_state(&t.consumer_pid, TransferState::Started)
            .await;
        (t.consumer_pid, token)
    }

    /// Starts a push to the consumer's receiving route for
    /// `consumer_pid` with `token`. Its body is what the test sends into
    /// the returned channel; the answer comes once the body has ended.
    fn push_streamed(
        &self,
        consumer_pid: &str,
        token: &str,
    ) -> (Chunks, tokio::task::JoinHandle<StatusCode>) {
        let (chunks, receiver) = tokio::sync::mpsc::channel(4);
        let request = Request::post(format!("/push/{consumer_pid}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::new(Streamed(receiver)))
            .unwrap();
        let routes = self.consumer.receiving_routes();
        let answer = tokio::spawn(async move { routes.oneshot(request).await.unwrap().status() });
        (chunks, answer)
    }
}

/// A push whose body breaks off after its first part has gone up: the
/// multipart upload is aborted, nothing is stored, the sender gets 400,
/// and the audit line says `cut`.
#[tokio::test]
async fn a_push_body_that_breaks_off_aborts_the_upload() {
    let p = parties(endless_upstream().await, true).await;
    let (cpid, token) = p.receiving_transfer().await;
    let (log, _guard) = Log::start();
    let (chunks, answer) = p.push_streamed(&cpid, &token);
    // more than one part, so the upload is a multipart one
    let part = vec![7u8; portitor_dataplane::object_storage::PART_BYTES + 10];
    chunks.send(Ok(Bytes::from(part))).await.unwrap();
    until(|| p.s3.open_uploads(), |n| *n == 1).await;
    chunks
        .send(Err(std::io::Error::other("the sender stopped")))
        .await
        .unwrap();
    assert_eq!(answer.await.unwrap(), StatusCode::BAD_REQUEST);
    until(|| p.s3.open_uploads(), |n| *n == 0).await;
    assert!(p.s3.object(PUSH_BUCKET, &cpid).is_none());
    let audit = log.with_message("data transfer done");
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["fields"]["outcome"], "cut");
    assert_eq!(audit[0]["fields"]["direction"], "received");
    // the provider's process id, as in the provider's line
    let provider_pid = p.consumer.transfer(&cpid).unwrap().provider_pid.unwrap();
    assert_eq!(audit[0]["fields"]["flow"], provider_pid.as_str());
}

/// One upload at a time per transfer: a second one while the first runs
/// gets 409 and changes nothing; once the first has ended, the next one
/// is taken.
#[tokio::test]
async fn a_second_push_while_one_runs_is_refused_with_409() {
    let p = parties(endless_upstream().await, true).await;
    let (cpid, token) = p.receiving_transfer().await;
    let (chunks, first) = p.push_streamed(&cpid, &token);
    chunks.send(Ok(Bytes::from_static(b"first"))).await.unwrap();
    // the first request is in the handler, reading its body
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        p.push_to_consumer(&cpid, Some(&token)).await,
        StatusCode::CONFLICT
    );
    drop(chunks);
    assert_eq!(first.await.unwrap(), StatusCode::NO_CONTENT);
    assert_eq!(p.s3.object(PUSH_BUCKET, &cpid).unwrap().bytes, b"first");
    assert_eq!(
        p.push_to_consumer(&cpid, Some(&token)).await,
        StatusCode::NO_CONTENT
    );
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
