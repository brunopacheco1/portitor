// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The binary's wiring of its senders (R-17): `ports_for_partners`, as
//! `main.rs` calls it, with the block on internal addresses on and no
//! exceptions. Messages to consumers, at the callback address each one
//! chose, are refused on an internal address; messages and catalog reads
//! to providers, at the address our operator gave, are sent; the data
//! plane still reads our own data sources. And which sender each kind
//! of message goes through.
#![cfg(feature = "data-plane")]
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use portitor_connector::harvest::FixedSource;
use portitor_connector::http_client::{AnswerLimits, HttpError, RawAnswer, Sender};
use portitor_connector::store::ConnectorStore;
use portitor_connector::{Config, Negotiator, Ports, ports_for_partners};
use portitor_dsp::negotiation::NegotiationState;
use portitor_dsp::transfer::TransferState;
use portitor_dsp::transfer::consumer::{ConsumerTransfer, ConsumerTransferDecision};
use portitor_runtime::BoxFuture;
use portitor_runtime::database::DatabaseSetting;
use portitor_runtime::internal_addresses::{BLOCK_SETTING, InternalAddresses};
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::network::Network;
use common::{CONSUMER as CONSUMER_DID, PROVIDER as PROVIDER_DID, authenticating};

const PROVIDER: &str = "https://provider.example";
const CONSUMER: &str = "https://consumer.example";
const DSP: &str = "/dsp/2025-1";

/// The block on internal addresses on, with no exceptions.
fn blocked() -> InternalAddresses {
    InternalAddresses {
        block: true,
        ..InternalAddresses::default()
    }
}

/// A local HTTP server on 127.0.0.1 that answers `{}` to any POST, and
/// counts the requests it got.
async fn counting_server() -> (String, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&count);
    let app = Router::new().fallback(move || {
        let counted = Arc::clone(&counted);
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            axum::Json(json!({}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, count)
}

/// The ports the binary builds, with plain http allowed (the test
/// servers are plain http) and the block on.
fn binary_ports() -> Ports {
    let store = Arc::new(ConnectorStore::in_memory().unwrap());
    ports_for_partners(&store, true, &blocked()).unwrap()
}

#[tokio::test]
async fn a_message_to_a_consumer_callback_on_an_internal_address_is_refused() {
    let ports = binary_ports();
    let (base, count) = counting_server().await;
    let callback = format!("{base}{DSP}/callback/negotiations/urn:uuid:c/agreement");
    let refused = ports
        .sender
        .post_authorized(&callback, &json!({}), "Bearer t")
        .await
        .unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_message_to_a_provider_on_an_internal_address_is_sent() {
    let ports = binary_ports();
    let (base, count) = counting_server().await;
    let request = format!("{base}{DSP}/negotiations/request");
    ports
        .provider_sender
        .post_authorized(&request, &json!({}), "Bearer t")
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_block_is_set_for_the_clients_the_connector_builds_itself() {
    // the data plane's push client is built from it (`data_plane.rs`)
    assert_eq!(binary_ports().internal_addresses, blocked());
}

/// A real HTTP server on 127.0.0.1 holding the dataset's data.
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
        database: DatabaseSetting::SqliteFile("unused.db".into()),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// The dataset `weather`, whose data is at `source`.
fn weather(source: &str) -> Value {
    json!({
        "@id": "weather",
        "@type": "Dataset",
        "hasPolicy": [{"@id": "weather:offer", "@type": "Offer", "permission": [{"action": "use"}]}],
        "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}],
        "portitor:source": source
    })
}

/// Builds the provider with `provider_ports` and the consumer with
/// `consumer_ports`, both on `network`. Returns the provider's router
/// and the consumer's handle.
async fn connect(
    network: &Arc<Network>,
    mut provider_ports: Ports,
    consumer_ports: Ports,
) -> (Router, Negotiator) {
    provider_ports.harvest.source =
        Some(Arc::new(FixedSource::new(vec![weather(&upstream().await)])));
    let provider_ports = authenticating(provider_ports, PROVIDER_DID);
    let (provider_app, _provider) =
        portitor_connector::build(&config(PROVIDER_DID, PROVIDER), provider_ports).unwrap();
    let consumer_ports = authenticating(consumer_ports, CONSUMER_DID);
    let (consumer_app, consumer) =
        portitor_connector::build(&config(CONSUMER_DID, CONSUMER), consumer_ports).unwrap();
    network.join(PROVIDER, provider_app.clone());
    network.join(CONSUMER, consumer_app);
    (provider_app, consumer)
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

/// Reads the catalog, negotiates for `weather` and starts a pull
/// transfer under the agreement: the transfer once started.
async fn negotiate_and_transfer(consumer: &Negotiator) -> ConsumerTransfer {
    let answer = consumer
        .read_catalog(&format!("{PROVIDER}{DSP}"), PROVIDER_DID, &[])
        .await
        .unwrap();
    let catalog: Value = serde_json::from_slice(&answer.body).unwrap();
    let mut offer: portitor_dsp::Offer =
        serde_json::from_value(catalog["dataset"][0]["hasPolicy"][0].clone()).unwrap();
    offer.target = Some("weather".to_owned());
    let started = consumer
        .start(&format!("{PROVIDER}{DSP}"), PROVIDER_DID, offer)
        .unwrap();
    let negotiation = until(
        || consumer.get(&started.consumer_pid).unwrap(),
        |n| n.state == NegotiationState::Finalized,
    )
    .await;
    let agreement_id = negotiation.agreement.unwrap().id;
    let requested = consumer
        .request_transfer(&agreement_id, "HttpData-PULL")
        .unwrap();
    until(
        || consumer.transfer(&requested.consumer_pid).unwrap(),
        |t| t.state == TransferState::Started,
    )
    .await
}

/// `GET` of the data address with its token: the status and the body.
async fn fetch(provider: &Router, transfer: &ConsumerTransfer) -> (StatusCode, String) {
    let address = transfer.data_address.as_ref().unwrap();
    let endpoint = address.endpoint.as_deref().unwrap();
    let path = endpoint.strip_prefix(PROVIDER).unwrap();
    let token = address.property("authorization").unwrap();
    let request = Request::get(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let answer = provider.clone().oneshot(request).await.unwrap();
    let status = answer.status();
    let bytes = to_bytes(answer.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// The block applies to addresses partners choose, not to our own data
/// sources: the provider built as the binary builds it, with the block
/// on, still reads a dataset's source on 127.0.0.1 for a pull transfer.
#[tokio::test]
async fn the_data_source_of_a_dataset_on_an_internal_address_is_still_read() {
    let network = Arc::new(Network::default());
    let store = Arc::new(ConnectorStore::in_memory().unwrap());
    let mut provider_ports = ports_for_partners(&store, false, &blocked()).unwrap();
    // the consumer's callback is on the in-process network, not on HTTP
    provider_ports.sender = Arc::clone(&network) as _;
    let consumer_ports = common::ports(Arc::clone(&network) as _);
    let (provider, consumer) = connect(&network, provider_ports, consumer_ports).await;
    let transfer = negotiate_and_transfer(&consumer).await;
    assert_eq!(
        fetch(&provider, &transfer).await,
        (StatusCode::OK, "day,celsius\nmon,12\n".to_owned())
    );
}

/// A sender that notes the URL of each call, then passes it on to the
/// network.
struct Recording {
    network: Arc<Network>,
    urls: Mutex<Vec<String>>,
}

impl Recording {
    fn new(network: &Arc<Network>) -> Arc<Self> {
        Arc::new(Self {
            network: Arc::clone(network),
            urls: Mutex::new(Vec::new()),
        })
    }

    fn note(&self, url: &str) {
        self.urls.lock().unwrap().push(url.to_owned());
    }

    fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

impl Sender for Recording {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        self.note(url);
        self.network.post(url, body)
    }

    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        self.note(url);
        self.network.get(url)
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.note(url);
        self.network.post_authorized(url, body, authorization)
    }

    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.note(url);
        self.network.get_authorized(url, authorization)
    }

    fn post_authorized_within<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
        limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        self.note(url);
        self.network
            .post_authorized_within(url, body, authorization, limits)
    }

    fn get_authorized_within<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
        limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        self.note(url);
        self.network
            .get_authorized_within(url, authorization, limits)
    }
}

/// Whether every one of `urls` starts with `base`, and there is one.
fn all_to(urls: &[String], base: &str) -> bool {
    !urls.is_empty() && urls.iter().all(|url| url.starts_with(base))
}

/// R-17: the consumer's negotiation and transfer messages and its
/// catalog reads go only through `provider_sender`; the provider's
/// messages to a consumer's callback only through `sender`.
#[tokio::test]
async fn consumer_and_provider_messages_use_their_own_senders() {
    let network = Arc::new(Network::default());
    let provider_to_consumers = Recording::new(&network);
    let provider_to_providers = Recording::new(&network);
    let consumer_to_consumers = Recording::new(&network);
    let consumer_to_providers = Recording::new(&network);
    let mut provider_ports = common::ports(Arc::clone(&provider_to_consumers) as _);
    provider_ports.provider_sender = Arc::clone(&provider_to_providers) as _;
    let mut consumer_ports = common::ports(Arc::clone(&consumer_to_consumers) as _);
    consumer_ports.provider_sender = Arc::clone(&consumer_to_providers) as _;
    let (provider, consumer) = connect(&network, provider_ports, consumer_ports).await;

    let transfer = negotiate_and_transfer(&consumer).await;
    assert_eq!(fetch(&provider, &transfer).await.0, StatusCode::OK);
    consumer
        .act_on_transfer(&transfer.consumer_pid, ConsumerTransferDecision::Complete)
        .unwrap();
    until(
        || consumer.transfer(&transfer.consumer_pid).unwrap(),
        |t| t.state == TransferState::Completed,
    )
    .await;

    let to_providers = consumer_to_providers.urls();
    assert!(all_to(&to_providers, PROVIDER), "{to_providers:?}");
    for kind in [
        "/catalog/request",
        "/negotiations/request",
        "/transfers/request",
        "/completion",
    ] {
        assert!(
            to_providers.iter().any(|url| url.contains(kind)),
            "{kind}: {to_providers:?}"
        );
    }
    let to_consumers = provider_to_consumers.urls();
    assert!(all_to(&to_consumers, CONSUMER), "{to_consumers:?}");
    for kind in ["/agreement", "/start"] {
        assert!(
            to_consumers.iter().any(|url| url.contains(kind)),
            "{kind}: {to_consumers:?}"
        );
    }
    assert_eq!(consumer_to_consumers.urls(), Vec::<String>::new());
    assert_eq!(provider_to_providers.urls(), Vec::<String>::new());
}
