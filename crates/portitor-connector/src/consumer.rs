// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapters for the consumer side of contract negotiation: the callback
//! endpoints a provider calls (DSP 2025-1, "Contract Negotiation HTTPS
//! Binding", consumer callback paths) and the loop that sends the
//! consumer's messages.
//!
//! The callback endpoints live under `{DSP_PATH}/callback`, so they never
//! clash with the provider endpoints of the same connector. Their base URL
//! is the `callbackAddress` the consumer sends with its requests.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use portitor_dsp::Offer;
use portitor_dsp::negotiation::consumer::{ConsumerNegotiation, ConsumerService, Outgoing};
use portitor_dsp::negotiation::provider::Failure;
use portitor_dsp::negotiation::{
    ContractAgreementMessage, ContractNegotiation, ContractNegotiationError,
    ContractNegotiationEventMessage, ContractNegotiationTerminationMessage, ContractOfferMessage,
    Transition,
};
use portitor_dsp::policy::credential_scopes;
use portitor_runtime::telemetry::metrics;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::Caller;
use crate::http_client::outbound_url_allowed;
use crate::inbound::read_message;
use crate::negotiation::{bad_request, failed};
use crate::outbox::{self, Link, Message, Outbox};
use crate::ports::{AnyConsumerDecider, AnyConsumerStore, Ports};
use crate::system::SystemEnvironment;

/// The acknowledgement for the provider and, possibly, our next message.
type Answer = (ContractNegotiation, Option<Outgoing>);

type Service = ConsumerService<AnyConsumerStore, AnyConsumerDecider, SystemEnvironment>;

/// The consumer side of negotiation, as used by the HTTP handlers.
pub(crate) struct Consumer {
    service: Service,
    link: Link,
    allow_http: bool,
}

impl Consumer {
    pub(crate) fn new(
        ports: &Ports,
        callback_address: String,
        participant_id: String,
        allow_http: bool,
    ) -> Self {
        Self {
            service: ConsumerService::new(
                ports.consumer_store.clone(),
                ports.consumer_decider.clone(),
                SystemEnvironment,
                callback_address,
                participant_id,
            ),
            link: Link::to_providers(ports),
            allow_http,
        }
    }

    /// Starts a negotiation for `offer` with the provider whose DSP
    /// endpoint is `provider_address`, and sends the request in the
    /// background.
    ///
    /// # Errors
    ///
    /// The address is not an allowed URL, the offer has no target, or the
    /// store failed.
    pub(crate) fn start(
        self: &Arc<Self>,
        provider_address: &str,
        provider_id: &str,
        offer: Offer,
    ) -> Result<ConsumerNegotiation, Box<Failure>> {
        if !outbound_url_allowed(provider_address, self.allow_http) {
            return Err(Box::new(Failure::BadRequest(
                ContractNegotiationError::new(
                    "",
                    "",
                    "400",
                    "the provider address must be an https URL",
                ),
            )));
        }
        let (negotiation, out) = self
            .service
            .start(provider_address, provider_id, offer)
            .map_err(Box::new)?;
        tokio::spawn(outbox::deliver(Arc::clone(self), out));
        Ok(negotiation)
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

    pub(crate) fn negotiation(
        &self,
        consumer_pid: &str,
    ) -> Result<ConsumerNegotiation, Box<Failure>> {
        self.service.negotiation(consumer_pid).map_err(Box::new)
    }

    pub(crate) fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerNegotiation>, Box<Failure>> {
        self.service.list(after, limit).map_err(Box::new)
    }

    /// Refuses the request unless `caller` is the provider of negotiation
    /// `consumer_pid` (ADR 0016); returns the refusal to send, if any.
    fn refusal(&self, consumer_pid: &str, caller: &str) -> Option<Response> {
        self.service
            .from_provider(consumer_pid, caller)
            .err()
            .map(failed)
    }

    /// Checks the message's shape with the guard, then reads it.
    fn read<T: DeserializeOwned>(
        &self,
        body: crate::inbound::Body,
        message_type: &str,
        consumer_pid: &str,
    ) -> Result<T, Box<ContractNegotiationError>> {
        read_message(
            &*self.link.guard,
            body,
            message_type,
            "providerPid",
            |provider_pid, reason| {
                ContractNegotiationError::new(provider_pid, consumer_pid, "400", reason)
            },
        )
    }

    /// Answers the provider and starts sending our next message, if any.
    /// A provider message that arrives before our acknowledged message is
    /// recorded needs no waiting: the domain takes it as proof that ours
    /// arrived (ADR 0007).
    fn answer(self: &Arc<Self>, handled: Result<Answer, Failure>) -> Response {
        match handled {
            Ok((ack, next)) => {
                if let Some(out) = next {
                    tokio::spawn(outbox::deliver(Arc::clone(self), out));
                }
                Json(ack).into_response()
            }
            Err(failure) => failed(failure),
        }
    }
}

impl Outbox for Consumer {
    type Out = Outgoing;
    type Failure = Failure;

    fn link(&self) -> &Link {
        &self.link
    }

    fn kind(&self) -> metrics::Outbox {
        metrics::Outbox::Negotiation(metrics::Role::Consumer)
    }

    /// A request or an acceptance may lead the provider to agree, after it
    /// asks for the credentials the offer's constraints need: our token
    /// lets it read them (ADR 0019).
    fn scopes(&self, out: &Outgoing) -> Vec<String> {
        if !matches!(out.transition, Transition::Request | Transition::Accept) {
            return Vec::new();
        }
        match self.service.negotiation(&out.consumer_pid) {
            Ok(negotiation) => credential_scopes(&negotiation.offer, &self.link.credential_types),
            // the message is still sent; the provider decides without them
            Err(_) => Vec::new(),
        }
    }

    fn message(out: &Outgoing) -> Message<'_> {
        Message::new(
            &out.consumer_pid,
            &out.url,
            &out.body,
            &out.status_url,
            &out.counter_party,
        )
        .first(out.initial)
        .terminating(out.transition == Transition::Terminate)
    }

    fn reached(out: &Outgoing, state: Value) -> bool {
        serde_json::from_value::<ContractNegotiation>(state).is_ok_and(|negotiation| {
            negotiation.state == out.to
                && outbox::same_process(
                    &out.body,
                    &negotiation.provider_pid,
                    &negotiation.consumer_pid,
                )
        })
    }

    fn delivered(
        &self,
        out: &Outgoing,
        answer: Option<&Value>,
    ) -> Result<Option<Outgoing>, Failure> {
        self.service.delivered(out, answer)
    }

    fn not_started(&self, out: &Outgoing) -> Result<(), Failure> {
        self.service.not_started(&out.consumer_pid)
    }

    fn abort(&self, out: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        self.service.abort(out, "the provider could not be reached")
    }
}

/// `GET <callback>/negotiations/{consumerPid}`.
pub(crate) async fn get(
    State(consumer): State<Arc<Consumer>>,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    match consumer.service.get(&cpid) {
        Ok(ack) => Json(ack).into_response(),
        Err(f) => failed(f),
    }
}

/// `POST <callback>/negotiations/{consumerPid}/offers`.
pub(crate) async fn offer(
    State(consumer): State<Arc<Consumer>>,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: ContractOfferMessage = match consumer.read(body, "ContractOfferMessage", &cpid) {
        Ok(m) => m,
        Err(e) => return bad_request(*e),
    };
    consumer.answer(consumer.service.offer(&cpid, &msg))
}

/// `POST <callback>/negotiations/{consumerPid}/agreement`.
pub(crate) async fn agreement(
    State(consumer): State<Arc<Consumer>>,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: ContractAgreementMessage = match consumer.read(body, "ContractAgreementMessage", &cpid)
    {
        Ok(m) => m,
        Err(e) => return bad_request(*e),
    };
    consumer.answer(consumer.service.agreement(&cpid, &msg))
}

/// `POST <callback>/negotiations/{consumerPid}/events`.
pub(crate) async fn event(
    State(consumer): State<Arc<Consumer>>,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: ContractNegotiationEventMessage =
        match consumer.read(body, "ContractNegotiationEventMessage", &cpid) {
            Ok(m) => m,
            Err(e) => return bad_request(*e),
        };
    consumer.answer(consumer.service.event(&cpid, &msg))
}

/// `POST <callback>/negotiations/{consumerPid}/termination`.
pub(crate) async fn terminate(
    State(consumer): State<Arc<Consumer>>,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: crate::inbound::Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: ContractNegotiationTerminationMessage =
        match consumer.read(body, "ContractNegotiationTerminationMessage", &cpid) {
            Ok(m) => m,
            Err(e) => return bad_request(*e),
        };
    consumer.answer(consumer.service.terminate(&cpid, &msg))
}
