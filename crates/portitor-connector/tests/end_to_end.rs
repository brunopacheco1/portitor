// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Two Portitor connectors talking DSP 2025-1 to each other, in process:
//! the consumer negotiates a contract for a dataset, requests a pull
//! transfer under the agreement, fetches the data from the provider's
//! built-in data plane with the token it was given, and completes the
//! transfer. Messages go through a fake network that hands each request to
//! the right connector's router; only the upstream data source is a real
//! local HTTP server.
#![cfg(feature = "data-plane")]
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use portitor_connector::harvest::{DatasetSource, Snapshot, SourceError};
use portitor_connector::http_client::Sender;
use portitor_connector::ports::Retry;
use portitor_connector::{Config, Negotiator, Ports};
use portitor_dsp::negotiation::NegotiationState;
use portitor_dsp::transfer::TransferState;
use portitor_dsp::transfer::consumer::{ConsumerTransfer, ConsumerTransferDecision};
use portitor_runtime::BoxFuture;
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::network::Network;
use common::{CONSUMER as CONSUMER_DID, PROVIDER as PROVIDER_DID, authenticating, bearer};

const PROVIDER: &str = "https://provider.example";
const CONSUMER: &str = "https://consumer.example";
const DSP: &str = "/dsp/2025-1";

/// A real HTTP server holding the dataset's data.
async fn upstream() -> String {
    let app = Router::new().route(
        "/weather.csv",
        axum::routing::get(|| async { ([("content-type", "text/csv")], "day,celsius\nmon,12\n") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/weather.csv", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn config(participant: &str, public_url: &str) -> Config {
    Config {
        participant_id: participant.to_owned(),
        public_url: public_url.to_owned(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// A datasets source the test can change while the provider runs.
#[derive(Default)]
struct Switch(Mutex<Vec<Value>>);

impl Switch {
    fn set(&self, entries: Vec<Value>) {
        *self.0.lock().unwrap() = entries;
    }
}

impl DatasetSource for Switch {
    fn name(&self) -> String {
        "switch".to_owned()
    }
    fn snapshot(&self) -> Result<Snapshot, SourceError> {
        let entries = self.0.lock().unwrap().clone();
        Ok(Snapshot {
            sha256: serde_json::to_string(&entries).unwrap(),
            entries,
        })
    }
}

/// The provider's handle and its datasets source.
struct Provider {
    app: Router,
    negotiator: Negotiator,
    source: Arc<Switch>,
}

/// The provider, offering dataset `weather`, and the consumer, both on
/// `network`. Returns the provider and the consumer's handle.
async fn connectors(network: &Arc<Network>) -> (Provider, Negotiator) {
    connectors_where_the_provider_sends_through(network, Arc::clone(network) as _).await
}

/// Like [`connectors`], but what the provider sends goes through
/// `provider_sender` first (a test's own hop in front of `network`).
async fn connectors_where_the_provider_sends_through(
    network: &Arc<Network>,
    provider_sender: Arc<dyn Sender>,
) -> (Provider, Negotiator) {
    let provider = config(PROVIDER_DID, PROVIDER);
    let source = Arc::new(Switch::default());
    source.set(vec![json!({
        "@id": "weather",
        "@type": "Dataset",
        "hasPolicy": [{"@id": "weather:offer", "@type": "Offer", "permission": [{"action": "use"}]}],
        "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}],
        "portitor:source": upstream().await
    })]);
    let sender = Arc::clone(network) as Arc<dyn Sender>;
    let mut provider_ports = authenticating(Ports::new(provider_sender), PROVIDER_DID);
    // a second try at once, so that a test with a lost answer is quick
    provider_ports.retry = Retry {
        attempts: 2,
        first_delay: Duration::from_millis(1),
    };
    provider_ports.harvest.source = Some(Arc::clone(&source) as _);
    let (provider_app, provider_negotiator) =
        portitor_connector::build(&provider, provider_ports).unwrap();
    let consumer = config(CONSUMER_DID, CONSUMER);
    let consumer_ports = authenticating(Ports::new(sender), CONSUMER_DID);
    let (consumer_app, negotiator) = portitor_connector::build(&consumer, consumer_ports).unwrap();
    network.join(PROVIDER, provider_app.clone());
    network.join(CONSUMER, consumer_app);
    let provider = Provider {
        app: provider_app,
        negotiator: provider_negotiator,
        source,
    };
    (provider, negotiator)
}

/// Polls `read` until `done` says yes, for up to two seconds.
async fn until<T: std::fmt::Debug>(read: impl Fn() -> T, done: impl Fn(&T) -> bool) -> T {
    for _ in 0..200 {
        let value = read();
        if done(&value) {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("still {:?}", read());
}

/// `GET` of the data address with its token: the status and the body.
async fn fetch(provider: &Router, t: &ConsumerTransfer) -> (StatusCode, String) {
    let address = t.data_address.as_ref().unwrap();
    let endpoint = address.endpoint.as_deref().unwrap();
    let path = endpoint.strip_prefix(PROVIDER).unwrap();
    let token = address.property("authorization").unwrap();
    assert_eq!(address.property("authType"), Some("bearer"));
    let req = Request::get(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = provider.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn negotiate_transfer_fetch_and_complete() {
    let network = Arc::new(Network::default());
    let (provider, consumer) = connectors(&network).await;
    let provider = provider.app;

    // the offer, as the provider's catalog shows it
    let catalog = format!("{PROVIDER}{DSP}/catalog/datasets/weather");
    let dataset = network
        .get_authorized(&catalog, &bearer(CONSUMER_DID))
        .await
        .unwrap();
    let mut offer: portitor_dsp::Offer =
        serde_json::from_value(dataset["hasPolicy"][0].clone()).unwrap();
    offer.target = Some("weather".to_owned());

    let n = consumer
        .start(&format!("{PROVIDER}{DSP}"), PROVIDER_DID, offer)
        .unwrap();
    let n = until(
        || consumer.get(&n.consumer_pid).unwrap(),
        |n| n.state == NegotiationState::Finalized,
    )
    .await;
    let agreement_id = n.agreement.unwrap().id;

    // no transfer under an agreement we do not hold
    assert!(
        consumer
            .request_transfer("urn:uuid:other", "HttpData-PULL")
            .is_err()
    );

    let t = consumer
        .request_transfer(&agreement_id, "HttpData-PULL")
        .unwrap();
    assert_eq!(t.state, TransferState::Requested);
    let t = until(
        || consumer.transfer(&t.consumer_pid).unwrap(),
        |t| t.state == TransferState::Started,
    )
    .await;
    assert_eq!(
        fetch(&provider, &t).await,
        (StatusCode::OK, "day,celsius\nmon,12\n".to_owned())
    );

    // done with the data: complete; the token stops working and is dropped
    consumer
        .act_on_transfer(&t.consumer_pid, ConsumerTransferDecision::Complete)
        .unwrap();
    let done = until(
        || consumer.transfer(&t.consumer_pid).unwrap(),
        |t| t.state == TransferState::Completed,
    )
    .await;
    assert_eq!(done.data_address, None);
    assert_eq!(fetch(&provider, &t).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(consumer.transfers(None, 10).unwrap(), vec![done]);
}

/// Starts a negotiation for `weather`, as the provider's catalog offers
/// it; returns the consumer's process id.
async fn start_negotiation(network: &Network, consumer: &Negotiator) -> String {
    let catalog = format!("{PROVIDER}{DSP}/catalog/datasets/weather");
    let dataset = network
        .get_authorized(&catalog, &bearer(CONSUMER_DID))
        .await
        .unwrap();
    let mut offer: portitor_dsp::Offer =
        serde_json::from_value(dataset["hasPolicy"][0].clone()).unwrap();
    offer.target = Some("weather".to_owned());
    let n = consumer
        .start(&format!("{PROVIDER}{DSP}"), PROVIDER_DID, offer)
        .unwrap();
    n.consumer_pid
}

/// Agrees a contract for `weather`; returns the agreement id.
async fn agree(network: &Network, consumer: &Negotiator) -> String {
    let consumer_pid = start_negotiation(network, consumer).await;
    let n = until(
        || consumer.get(&consumer_pid).unwrap(),
        |n| n.state == NegotiationState::Finalized,
    )
    .await;
    n.agreement.unwrap().id
}

/// ADR 0020: a dataset that leaves the source is withdrawn. It leaves the
/// catalog and new contract requests for it are refused, but a transfer
/// under an agreement already made still works, data included.
#[tokio::test]
async fn a_withdrawn_dataset_keeps_its_transfers() {
    let network = Arc::new(Network::default());
    let (provider, consumer) = connectors(&network).await;
    let agreement_id = agree(&network, &consumer).await;
    let offer: portitor_dsp::Offer = serde_json::from_value(json!({
        "@id": "weather:offer", "@type": "Offer", "target": "weather",
        "permission": [{"action": "use"}]
    }))
    .unwrap();

    // the source drops the dataset; the next scheduled run withdraws it
    provider.source.set(Vec::new());
    let schedule = provider
        .negotiator
        .schedule_harvest(Duration::from_millis(20))
        .unwrap();
    until(
        || {
            provider.negotiator.harvest_jobs(None, 1).unwrap()[0]
                .record
                .counts
                .withdrawn
        },
        |withdrawn| *withdrawn == 1,
    )
    .await;
    schedule.abort();

    let catalog = format!("{PROVIDER}{DSP}/catalog/datasets/weather");
    let gone = network
        .get_authorized(&catalog, &bearer(CONSUMER_DID))
        .await;
    assert!(gone.is_err(), "{gone:?}");
    // a new contract request for it is refused
    let request = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "ContractRequestMessage",
        "consumerPid": "urn:uuid:late",
        "offer": offer,
        "callbackAddress": format!("{CONSUMER}{DSP}/callback")
    });
    let req = Request::post(format!("{DSP}/negotiations/request"))
        .header(header::AUTHORIZATION, bearer(CONSUMER_DID))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(request.to_string()))
        .unwrap();
    let res = provider.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let error = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let error = String::from_utf8(error.to_vec()).unwrap();
    assert!(error.contains("unknown offer"), "{error}");

    let t = consumer
        .request_transfer(&agreement_id, "HttpData-PULL")
        .unwrap();
    let t = until(
        || consumer.transfer(&t.consumer_pid).unwrap(),
        |t| t.state == TransferState::Started,
    )
    .await;
    assert_eq!(
        fetch(&provider.app, &t).await,
        (StatusCode::OK, "day,celsius\nmon,12\n".to_owned())
    );
}

// ---- ADR 0007, amended in 0.13 (TD-44): the consumer asks for a
// ---- transfer while the provider still waits for the answer to its
// ---- finalization event. Here with both connectors and the real
// ---- outbox; the hop below decides what becomes of that event.

/// What becomes of the provider's finalization event on its way to the
/// consumer, and of the consumer's answer on its way back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EventFate {
    /// The event arrives. The answer is held back until the test lets it
    /// go, and then arrives.
    AnswerLate,
    /// The event arrives. The answer is held back and then lost, so the
    /// provider tries again; the second try goes through.
    AnswerLost,
    /// No try arrives: the consumer never sees the event.
    NeverArrives,
}

/// A hop between the provider and the network that holds the first
/// finalization event (a POST to `.../events`), as [`EventFate`] says.
/// Everything else goes straight to the network.
struct HeldEvent {
    network: Arc<Network>,
    fate: EventFate,
    /// How many times the provider sent the event.
    tries: AtomicUsize,
    /// Whether the first try is being held now.
    holding: AtomicBool,
    /// The test adds a permit to let the held try go on.
    go_on: tokio::sync::Semaphore,
}

impl HeldEvent {
    fn new(network: &Arc<Network>, fate: EventFate) -> Arc<Self> {
        Arc::new(Self {
            network: Arc::clone(network),
            fate,
            tries: AtomicUsize::new(0),
            holding: AtomicBool::new(false),
            go_on: tokio::sync::Semaphore::new(0),
        })
    }

    fn tries(&self) -> usize {
        self.tries.load(Ordering::SeqCst)
    }

    /// One try of the finalization event.
    async fn event(&self, url: &str, body: &Value, authorization: &str) -> Result<Value, String> {
        let lost = || Err("no answer".to_owned());
        let earlier_tries = self.tries.fetch_add(1, Ordering::SeqCst);
        let answer = if self.fate == EventFate::NeverArrives {
            lost()
        } else {
            let sent = self.network.post_authorized(url, body, authorization);
            sent.await
        };
        if earlier_tries > 0 {
            return answer;
        }
        // the first try: hold it here until the test says go on
        self.holding.store(true, Ordering::SeqCst);
        self.go_on.acquire().await.unwrap().forget();
        match self.fate {
            EventFate::AnswerLate => answer,
            EventFate::AnswerLost | EventFate::NeverArrives => lost(),
        }
    }
}

impl Sender for HeldEvent {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.network.post(url, body)
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        if url.ends_with("/events") {
            Box::pin(self.event(url, body, authorization))
        } else {
            self.network.post_authorized(url, body, authorization)
        }
    }

    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.network.get_authorized(url, authorization)
    }
}

/// Both connectors, with a negotiation taken to the moment of TD-44: the
/// provider sent its finalization event and waits for the answer.
struct RightAfterTheAgreement {
    hop: Arc<HeldEvent>,
    provider: Router,
    consumer: Negotiator,
    /// The negotiation's ids: the consumer's and the provider's.
    consumer_pid: String,
    provider_pid: String,
    agreement_id: String,
}

impl RightAfterTheAgreement {
    async fn with(fate: EventFate) -> Self {
        let network = Arc::new(Network::default());
        let hop = HeldEvent::new(&network, fate);
        let sender = Arc::clone(&hop) as Arc<dyn Sender>;
        let (provider, consumer) =
            connectors_where_the_provider_sends_through(&network, sender).await;
        let consumer_pid = start_negotiation(&network, &consumer).await;
        until(|| hop.holding.load(Ordering::SeqCst), |holding| *holding).await;
        let n = consumer.get(&consumer_pid).unwrap();
        let moment = Self {
            hop,
            provider: provider.app,
            consumer,
            consumer_pid,
            provider_pid: n.provider_pid.unwrap(),
            agreement_id: n.agreement.unwrap().id,
        };
        assert_eq!(moment.provider_state().await, "VERIFIED");
        moment
    }

    /// The negotiation's state as the provider reports it (DSP 2025-1,
    /// `GET /negotiations/{providerPid}`).
    async fn provider_state(&self) -> String {
        let uri = format!("{DSP}/negotiations/{}", self.provider_pid);
        let (status, negotiation) = self.ask_provider(Request::get(uri), Body::empty()).await;
        assert_eq!(status, StatusCode::OK, "{negotiation}");
        negotiation["state"].as_str().unwrap().to_owned()
    }

    fn consumer_state(&self) -> NegotiationState {
        self.consumer.get(&self.consumer_pid).unwrap().state
    }

    /// Sends `request` to the provider as the consumer.
    async fn ask_provider(
        &self,
        request: axum::http::request::Builder,
        body: Body,
    ) -> (StatusCode, Value) {
        let request = request
            .header(header::AUTHORIZATION, bearer(CONSUMER_DID))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap();
        let response = self.provider.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// The consumer asks for a transfer and waits until it has started.
    async fn started_transfer(&self) -> ConsumerTransfer {
        let t = self
            .consumer
            .request_transfer(&self.agreement_id, "HttpData-PULL")
            .unwrap();
        until(
            || self.consumer.transfer(&t.consumer_pid).unwrap(),
            |t| t.state == TransferState::Started,
        )
        .await
    }

    /// Lets the held event go on, waits until the provider has sent the
    /// event `tries` times in all, and then a moment more for whatever
    /// the provider does with the outcome.
    async fn let_the_event_go_on(&self, tries: usize) {
        self.hop.go_on.add_permits(1);
        until(|| self.hop.tries(), |sent| *sent == tries).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(self.hop.tries(), tries, "sent again");
    }
}

/// The race of TD-44 as it happens: the consumer has the event and asks
/// for a transfer before the provider has its answer. The transfer
/// starts, the provider's negotiation is `FINALIZED` at once, and the
/// answer that arrives later changes nothing.
#[tokio::test]
async fn a_transfer_asked_for_before_the_finalization_is_acknowledged_starts() {
    let moment = RightAfterTheAgreement::with(EventFate::AnswerLate).await;
    assert_eq!(moment.consumer_state(), NegotiationState::Finalized);

    let t = moment.started_transfer().await;
    assert_eq!(moment.provider_state().await, "FINALIZED");
    assert_eq!(moment.hop.tries(), 1);
    assert_eq!(fetch(&moment.provider, &t).await.0, StatusCode::OK);

    moment.let_the_event_go_on(1).await;
    assert_eq!(moment.provider_state().await, "FINALIZED");
    assert_eq!(moment.consumer_state(), NegotiationState::Finalized);
    assert_eq!(fetch(&moment.provider, &t).await.0, StatusCode::OK);
}

/// The answer to the finalization event is lost after the transfer
/// request settled the negotiation. The outbox does not know and sends
/// the event a second time: the consumer, long `FINALIZED`, gets it
/// twice. Nothing changes on either side, and the transfer goes on.
#[tokio::test]
async fn a_finalization_event_sent_again_after_it_was_settled_changes_nothing() {
    let moment = RightAfterTheAgreement::with(EventFate::AnswerLost).await;
    let t = moment.started_transfer().await;
    assert_eq!(moment.provider_state().await, "FINALIZED");

    moment.let_the_event_go_on(2).await;
    assert_eq!(moment.provider_state().await, "FINALIZED");
    assert_eq!(moment.consumer_state(), NegotiationState::Finalized);
    let still = moment.consumer.transfer(&t.consumer_pid).unwrap();
    assert_eq!(still.state, TransferState::Started);
    assert_eq!(fetch(&moment.provider, &t).await.0, StatusCode::OK);
}

/// "The cost, accepted" of ADR 0007's amendment: a consumer that asks
/// for a transfer after its verification, without having received the
/// finalization event, is served too. When the event then never arrives,
/// the provider does not end the negotiation it has settled: it stays
/// `FINALIZED` there and `VERIFIED` at the consumer, and no termination
/// is sent.
#[tokio::test]
async fn a_finalization_that_never_arrives_after_it_was_settled_ends_nothing() {
    let moment = RightAfterTheAgreement::with(EventFate::NeverArrives).await;
    assert_eq!(moment.consumer_state(), NegotiationState::Verified);
    // Portitor as consumer follows DSP and asks only after FINALIZED
    let own = moment
        .consumer
        .request_transfer(&moment.agreement_id, "HttpData-PULL");
    assert!(own.is_err(), "{own:?}");

    // a consumer that does not wait: the request as a DSP message
    let request = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "TransferRequestMessage",
        "consumerPid": "urn:uuid:too-early",
        "agreementId": moment.agreement_id,
        "format": "HttpData-PULL",
        "callbackAddress": format!("{CONSUMER}{DSP}/callback")
    });
    let uri = format!("{DSP}/transfers/request");
    let (status, answer) = moment
        .ask_provider(Request::post(uri), Body::from(request.to_string()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(moment.provider_state().await, "FINALIZED");

    // both tries of the event fail; the provider then asks the consumer
    // for its state, which is not FINALIZED
    moment.let_the_event_go_on(2).await;
    assert_eq!(moment.provider_state().await, "FINALIZED");
    assert_eq!(moment.consumer_state(), NegotiationState::Verified);
    // TD-44: the event was not delivered, but the negotiation was
    // settled without it, so the provider's outbox did not count it in
    // `portitor.outbox.message.given_up`
    assert_eq!(given_up_by_the_provider_negotiation(), 0);
}

/// How many messages the provider's negotiation outbox counted as given
/// up (`portitor.outbox.message.given_up`) in this test so far.
fn given_up_by_the_provider_negotiation() -> usize {
    use portitor_runtime::telemetry::metrics::{Outbox, Role};
    portitor_runtime::telemetry::testing::given_up_on_this_thread()
        .iter()
        .filter(|outbox| **outbox == Outbox::Negotiation(Role::Provider))
        .count()
}

/// The other side of TD-44: the same event that never arrives, with no
/// transfer request to settle the negotiation. The provider gives the
/// event up, counts it once, and ends the negotiation.
#[tokio::test]
async fn a_finalization_that_never_arrives_and_was_not_settled_is_counted_as_given_up() {
    let moment = RightAfterTheAgreement::with(EventFate::NeverArrives).await;
    assert_eq!(moment.consumer_state(), NegotiationState::Verified);
    assert_eq!(given_up_by_the_provider_negotiation(), 0);

    moment.let_the_event_go_on(2).await;
    assert_eq!(moment.provider_state().await, "TERMINATED");
    assert_eq!(given_up_by_the_provider_negotiation(), 1);
}
