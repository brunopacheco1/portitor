// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapters for the provider side of contract negotiation: the HTTPS
//! binding (DSP 2025-1, "Contract Negotiation HTTPS Binding").

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use portitor_dsp::Offer;
use portitor_dsp::negotiation::provider::{Failure, Outgoing, ProviderService};
use portitor_dsp::negotiation::{
    ContractAgreementVerificationMessage, ContractNegotiation, ContractNegotiationError,
    ContractNegotiationEventMessage, ContractNegotiationTerminationMessage, ContractRequestMessage,
    NegotiationEvent, Transition,
};
use portitor_dsp::policy::{CallerCredentials, credential_scopes};
use portitor_runtime::telemetry::metrics;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::{AnyVerifier, Caller, credentials_for};
use crate::harvest::StoredCatalog;
use crate::http_client::outbound_url_allowed;
use crate::inbound::read_message;
use crate::outbox::{self, Link, Message, Outbox};
use crate::ports::{AnyDecider, AnyStore, Ports};
use crate::system::SystemEnvironment;

type Service = ProviderService<AnyStore, AnyDecider, StoredCatalog, SystemEnvironment>;

/// The provider side of negotiation, as used by the HTTP handlers.
pub(crate) struct Provider {
    service: Service,
    link: Link,
    /// Asks a caller for more credentials when an offer needs them.
    verifier: AnyVerifier,
    allow_http: bool,
}

impl Provider {
    pub(crate) fn new(
        ports: Ports,
        catalog: StoredCatalog,
        participant_id: String,
        allow_http: bool,
    ) -> Self {
        Self {
            link: Link::new(&ports),
            verifier: Arc::clone(&ports.verifier),
            service: ProviderService::new(
                ports.store,
                ports.decider,
                catalog,
                SystemEnvironment,
                participant_id,
            ),
            allow_http,
        }
    }

    /// Checks the message's shape with the guard, then reads it.
    fn read<T: DeserializeOwned>(
        &self,
        body: crate::inbound::Body,
        message_type: &str,
        provider_pid: &str,
    ) -> Result<T, Box<ContractNegotiationError>> {
        read_message(
            &*self.link.guard,
            body,
            message_type,
            "consumerPid",
            |consumer_pid, reason| invalid(provider_pid, consumer_pid, reason),
        )
    }

    /// The caller's verified credentials for the credential constraints of
    /// `offer` (ADR 0019).
    async fn credentials(&self, caller: &Caller, offer: &Offer) -> CallerCredentials {
        let types = &self.link.credential_types;
        let scopes = credential_scopes(offer, types);
        credentials_for(&*self.verifier, types, caller, &scopes).await
    }

    /// The caller's verified credentials for a contract request with
    /// `offer`: those of the offer's credential constraints and of its
    /// dataset's access policy, in one presentation query (ADR 0019,
    /// amendment in 1.4).
    ///
    /// When the catalog cannot be read, only the offer's scopes are asked
    /// for: `service.request` reads the catalog again and answers the
    /// store failure itself.
    async fn request_credentials(&self, caller: &Caller, offer: &Offer) -> CallerCredentials {
        let types = &self.link.credential_types;
        let scopes = match self.service.request_scopes(offer, types) {
            Ok(scopes) => scopes,
            Err(_) => credential_scopes(offer, types),
        };
        credentials_for(&*self.verifier, types, caller, &scopes).await
    }

    /// Sends every message saved as pending, for example after a
    /// restart. Returns how many.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub(crate) fn resume(self: &Arc<Self>) -> Result<usize, Box<Failure>> {
        let pending = self.service.pending().map_err(Box::new)?;
        Ok(outbox::resume(self, pending))
    }
}

impl Outbox for Provider {
    type Out = Outgoing;
    type Failure = Failure;

    fn link(&self) -> &Link {
        &self.link
    }

    fn kind(&self) -> metrics::Outbox {
        metrics::Outbox::Negotiation(metrics::Role::Provider)
    }

    fn message(out: &Outgoing) -> Message<'_> {
        Message::new(
            &out.provider_pid,
            &out.url,
            &out.body,
            &out.status_url,
            &out.counter_party,
        )
        .terminating(out.transition == Transition::Terminate)
    }

    fn reached(out: &Outgoing, state: Value) -> bool {
        serde_json::from_value::<ContractNegotiation>(state)
            .is_ok_and(|negotiation| negotiation.state == out.to)
    }

    fn delivered(&self, out: &Outgoing, _: Option<&Value>) -> Result<Option<Outgoing>, Failure> {
        self.service.delivered(out)
    }

    fn abort(&self, out: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        self.service.abort(out, "the consumer could not be reached")
    }

    /// A finalization event that could not be delivered, in a negotiation
    /// that a transfer request from the consumer settled as `FINALIZED`
    /// (ADR 0007, amended in 0.13; TD-44).
    fn settled_without_delivery(&self, out: &Outgoing) -> bool {
        match self.service.finalization_settled(out) {
            Ok(settled) => settled,
            Err(failure) => {
                // not known: counted as given up, as before
                tracing::warn!(pid = %out.provider_pid, ?failure, "negotiation not read");
                false
            }
        }
    }
}

pub(crate) fn invalid(
    provider_pid: &str,
    consumer_pid: &str,
    reason: &str,
) -> ContractNegotiationError {
    ContractNegotiationError::new(provider_pid, consumer_pid, "400", reason)
}

pub(crate) fn bad_request(e: ContractNegotiationError) -> Response {
    (StatusCode::BAD_REQUEST, Json(e)).into_response()
}

/// Turns the service's answer into the HTTP response and starts sending
/// whatever the provider sends next.
fn finish(
    provider: &Arc<Provider>,
    handled: Result<(ContractNegotiation, Option<Outgoing>), Failure>,
    ok: StatusCode,
) -> Response {
    match handled {
        Ok((ack, next)) => {
            if let Some(out) = next {
                tokio::spawn(outbox::deliver(Arc::clone(provider), out));
            }
            (ok, Json(ack)).into_response()
        }
        Err(failure) => failed(failure),
    }
}

pub(crate) fn failed(failure: Failure) -> Response {
    match failure {
        Failure::BadRequest(e) => (StatusCode::BAD_REQUEST, Json(e)).into_response(),
        Failure::NotFound(e) => (StatusCode::NOT_FOUND, Json(e)).into_response(),
        Failure::Store(e) => {
            tracing::error!(error = %e, "negotiation store failed");
            let e = ContractNegotiationError::new("", "", "500", "internal error");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(e)).into_response()
        }
    }
}

/// `GET /negotiations/{providerPid}`.
pub(crate) async fn get(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
) -> Response {
    match provider.service.get(&pid, &caller.id) {
        Ok(ack) => Json(ack).into_response(),
        Err(f) => failed(f),
    }
}

/// `POST /negotiations/request`.
pub(crate) async fn request_new(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    body: crate::inbound::Body,
) -> Response {
    let msg: ContractRequestMessage = match provider.read(body, "ContractRequestMessage", "") {
        Ok(m) => m,
        Err(e) => return bad_request(*e),
    };
    let callback = msg.callback_address.as_deref().unwrap_or_default();
    if !callback.is_empty() && !outbound_url_allowed(callback, provider.allow_http) {
        return bad_request(invalid(
            "",
            &msg.consumer_pid,
            "callbackAddress must be an https URL",
        ));
    }
    let credentials = provider.request_credentials(&caller, &msg.offer).await;
    let handled = provider
        .service
        .request(None, msg, &caller.id, &credentials);
    finish(&provider, handled, StatusCode::CREATED)
}

/// `POST /negotiations/{providerPid}/request`.
pub(crate) async fn request_counter(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    let msg: ContractRequestMessage = match provider.read(body, "ContractRequestMessage", &pid) {
        Ok(m) => m,
        Err(e) => return bad_request(*e),
    };
    let credentials = provider.request_credentials(&caller, &msg.offer).await;
    let handled = provider
        .service
        .request(Some(&pid), msg, &caller.id, &credentials);
    finish(&provider, handled, StatusCode::OK)
}

/// `POST /negotiations/{providerPid}/events`.
pub(crate) async fn event(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    let msg: ContractNegotiationEventMessage =
        match provider.read(body, "ContractNegotiationEventMessage", &pid) {
            Ok(m) => m,
            Err(e) => return bad_request(*e),
        };
    let credentials = credentials_for_event(&provider, &caller, &pid, msg.event_type).await;
    let handled = provider.service.event(&pid, &msg, &caller.id, &credentials);
    finish(&provider, handled, StatusCode::OK)
}

/// The caller's credentials that an event on negotiation `pid` needs.
/// Only an acceptance may lead to an agreement, so only then are the
/// credentials of the offer's constraints asked for (ADR 0019).
///
/// When the offer cannot be read (an unknown negotiation, another
/// consumer's, or a store failure), the answer is "no credentials":
/// `service.event` then reads the negotiation itself and answers that
/// error, and with no credentials a credential constraint can only fail
/// (fail closed).
async fn credentials_for_event(
    provider: &Provider,
    caller: &Caller,
    pid: &str,
    event: NegotiationEvent,
) -> CallerCredentials {
    match event {
        NegotiationEvent::Accepted => {}
        NegotiationEvent::Finalized => return CallerCredentials::default(),
    }
    match provider.service.offer_under_negotiation(pid, &caller.id) {
        Ok(offer) => provider.credentials(caller, &offer).await,
        Err(_) => CallerCredentials::default(),
    }
}

/// `POST /negotiations/{providerPid}/agreement/verification`.
pub(crate) async fn verify(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    let msg: ContractAgreementVerificationMessage =
        match provider.read(body, "ContractAgreementVerificationMessage", &pid) {
            Ok(m) => m,
            Err(e) => return bad_request(*e),
        };
    let handled = provider.service.verify(&pid, &msg, &caller.id);
    finish(&provider, handled, StatusCode::OK)
}

/// `POST /negotiations/{providerPid}/termination`.
pub(crate) async fn terminate(
    State(provider): State<Arc<Provider>>,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    let msg: ContractNegotiationTerminationMessage =
        match provider.read(body, "ContractNegotiationTerminationMessage", &pid) {
            Ok(m) => m,
            Err(e) => return bad_request(*e),
        };
    let handled = provider.service.terminate(&pid, &msg, &caller.id);
    finish(&provider, handled, StatusCode::OK)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_dsp::Offer;
    use portitor_dsp::negotiation::NegotiationState;

    use crate::auth::Prover;
    use crate::http_client::{HttpSender, Sender};
    use crate::outbox::Link;
    use crate::ports::Shared;
    use crate::test_stores::InMemoryNegotiations;
    use portitor_dsp::negotiation::provider::{Negotiation, NegotiationStore, StoreError};
    use portitor_runtime::BoxFuture;

    fn negotiation(state: NegotiationState) -> Negotiation {
        Negotiation {
            provider_pid: "p".into(),
            consumer_pid: "c".into(),
            state,
            callback_address: "https://c.example".into(),
            counter_party: "did:web:c.example".into(),
            offer: serde_json::from_value::<Offer>(
                serde_json::json!({"@id": "o", "@type": "Offer", "target": "d"}),
            )
            .unwrap(),
            agreement: None,
            pending: None,
            version: 0,
        }
    }

    /// The finalization event of the negotiation `p`, as the provider
    /// saves it before sending it.
    fn finalization_event() -> Outgoing {
        Outgoing {
            provider_pid: "p".into(),
            url: "https://c.example/negotiations/c/events".into(),
            body: serde_json::json!({}),
            transition: Transition::Finalize,
            to: NegotiationState::Finalized,
            status_url: "https://c.example/negotiations/c".into(),
            counter_party: "did:web:c.example".into(),
        }
    }

    /// The provider as the connector builds it, on the negotiation store
    /// `store`. It never sends in these tests.
    fn provider_on(store: AnyStore) -> Provider {
        let mut ports = Ports::in_memory(Arc::new(PostOnly));
        ports.store = store;
        let participant_id = "did:web:p.example".to_owned();
        Provider::new(ports, StoredCatalog::empty(), participant_id, false)
    }

    /// ADR 0007, amended in 0.13 (TD-44), on the real provider and an
    /// in-memory store: the outbox is told "settled without delivery"
    /// only for the finalization event of a negotiation that is
    /// `FINALIZED`, which is what a transfer request from the consumer
    /// makes of a `VERIFIED` one (`finalization_proved`). This answer
    /// keeps the event out of `portitor.outbox.message.given_up`.
    #[test]
    fn the_provider_outbox_says_settled_only_for_a_finalization_a_transfer_request_proved() {
        use portitor_dsp::negotiation::provider::finalization_proved;

        let store: AnyStore = Shared(Arc::new(InMemoryNegotiations::default()));
        let provider = provider_on(store.clone());
        let event = finalization_event();
        // the agreement message of the same negotiation, sent earlier
        let mut agreement = event.clone();
        agreement.transition = Transition::Agree;
        agreement.to = NegotiationState::Agreed;

        // an unknown negotiation
        assert!(!provider.settled_without_delivery(&event));

        // VERIFIED, the event on its way: not settled
        let mut verified = negotiation(NegotiationState::Verified);
        verified.pending = Some(event.clone());
        store.insert(&verified).unwrap();
        assert!(!provider.settled_without_delivery(&event));

        // what the transfer provider writes for a transfer request
        let mut finalized = finalization_proved(&verified).unwrap();
        finalized.version = verified.version + 1;
        assert!(store.replace_if(&finalized, verified.version).unwrap());
        assert_eq!(finalized.state, NegotiationState::Finalized);
        assert!(provider.settled_without_delivery(&event));
        // only the finalization event is asked about
        assert!(!provider.settled_without_delivery(&agreement));

        // a negotiation the outbox ended instead is not settled
        let mut terminated = finalized.clone();
        terminated.state = NegotiationState::Terminated;
        terminated.version = finalized.version + 1;
        assert!(store.replace_if(&terminated, finalized.version).unwrap());
        assert!(!provider.settled_without_delivery(&event));
    }

    /// A negotiation store that cannot be read.
    struct BrokenStore;

    impl NegotiationStore for BrokenStore {
        fn get(&self, _: &str) -> Result<Option<Negotiation>, StoreError> {
            Err(StoreError("disk I/O error".to_owned()))
        }
        fn insert(&self, _: &Negotiation) -> Result<(), StoreError> {
            Err(StoreError("disk I/O error".to_owned()))
        }
        fn replace_if(&self, _: &Negotiation, _: u64) -> Result<bool, StoreError> {
            Err(StoreError("disk I/O error".to_owned()))
        }
        fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
            Err(StoreError("disk I/O error".to_owned()))
        }
    }

    /// "Not known" is not "settled": when the negotiation cannot be
    /// read, the event stays one that the outbox counts as given up.
    #[test]
    fn a_store_failure_is_not_read_as_settled() {
        let provider = provider_on(Shared(Arc::new(BrokenStore)));
        assert!(!provider.settled_without_delivery(&finalization_event()));
    }

    #[test]
    fn debug_hides_internals() {
        let ports = Ports::in_memory(Arc::new(HttpSender::new(true).unwrap()));
        let shown = format!("{ports:?} {:?} {:?}", ports.store, ports.decider);
        assert!(shown.contains("Ports") && shown.contains("Shared"));
        assert!(format!("{:?}", HttpSender::new(true).unwrap()).contains("allow_http: true"));
    }

    /// Sends, but cannot read a counter-party's state.
    struct PostOnly;

    impl Sender for PostOnly {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Ok(Value::Null) })
        }
    }

    /// A prover with a fixed token.
    struct FixedToken;

    impl Prover for FixedToken {
        fn authorization<'a>(
            &'a self,
            _: &'a str,
            _: &'a [String],
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async { Ok("Bearer t".to_owned()) })
        }
    }

    /// Whether `sender`, with tokens from `prover`, reads state `to` at
    /// `url`.
    async fn read_state(
        sender: Arc<dyn Sender>,
        prover: Arc<dyn Prover>,
        url: &str,
        to: NegotiationState,
    ) -> bool {
        let mut ports = Ports::in_memory(sender);
        ports.prover = prover;
        let reached = |state: Value| {
            serde_json::from_value::<ContractNegotiation>(state).is_ok_and(|n| n.state == to)
        };
        outbox::arrived(&Link::new(&ports), url, "did:web:c.example", reached).await
    }

    /// Whether `sender` reads state `to` at `url`.
    async fn arrived(sender: Arc<dyn Sender>, url: &str, to: NegotiationState) -> bool {
        read_state(sender, Arc::new(FixedToken), url, to).await
    }

    #[tokio::test]
    async fn arrived_reads_the_counter_party_state() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route(
                "/negotiations/c",
                axum::routing::get(|| async {
                    Json(ContractNegotiation::new("p", "c", NegotiationState::Agreed))
                }),
            )
            .route(
                "/negotiations/x",
                axum::routing::get(|| async { Json(serde_json::json!({"state": 1})) }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let sender: Arc<dyn Sender> = Arc::new(HttpSender::new(true).unwrap());
        let url = format!("http://{addr}/negotiations/c");
        let http = || Arc::clone(&sender);
        assert!(arrived(http(), &url, NegotiationState::Agreed).await);
        assert!(!arrived(http(), &url, NegotiationState::Verified).await);
        let garbage = format!("http://{addr}/negotiations/x");
        assert!(!arrived(http(), &garbage, NegotiationState::Agreed).await);
        let missing = format!("http://{addr}/negotiations/none");
        assert!(!arrived(http(), &missing, NegotiationState::Agreed).await);
        assert!(!arrived(http(), "", NegotiationState::Agreed).await);
        // a sender that cannot read never claims delivery
        assert!(!arrived(Arc::new(PostOnly), &url, NegotiationState::Agreed).await);
        // nor does one without a token
        let no_tokens = Arc::new(crate::auth::NoTokens);
        assert!(!read_state(http(), no_tokens, &url, NegotiationState::Agreed).await);
    }
}
