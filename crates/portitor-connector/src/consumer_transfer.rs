// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapters for the consumer side of the transfer process (ADR 0014): the
//! callback endpoints a provider calls (DSP 2025-1, "Transfer Process HTTPS
//! Binding", consumer callback paths) and the loop that sends the
//! consumer's messages.
//!
//! The callback endpoints live under `{DSP_PATH}/callback/transfers`, next
//! to the negotiation callbacks.
//!
//! A push transfer (1.1, ADR 0027) asks the provider to send the data to
//! us: its request carries our own data address and token, made by
//! [`crate::receiving`], whose route then stores the data.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use portitor_dsp::policy::credential_scopes;
use portitor_dsp::process::StoreError;
use portitor_dsp::transfer::consumer::{
    ConsumerTransfer, ConsumerTransferDecision, ConsumerTransferService, Handled, HeldAgreements,
    Outgoing, Request,
};
use portitor_dsp::transfer::provider::Failure;
use portitor_dsp::transfer::{
    TransferCompletionMessage, TransferError, TransferStartMessage, TransferSuspensionMessage,
    TransferTerminationMessage, Transition, is_push_format,
};
use portitor_runtime::telemetry::metrics;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::Caller;
use crate::http_client::outbound_url_allowed;
use crate::inbound::{Body, read_message};
use crate::outbox::{self, Link, Message, Outbox};
use crate::ports::{
    AnyConsumerTransferDecider, AnyConsumerTransferStore, AnyHeldAgreements, Ports,
};
use crate::receiving::PushAddresses;
use crate::system::SystemEnvironment;
use crate::transfer::{bad_request, failed, log_termination_without_membership, reached_state};

type Service = ConsumerTransferService<
    AnyConsumerTransferStore,
    AnyConsumerTransferDecider,
    SystemEnvironment,
>;

/// The consumer side of the transfer process, as used by the HTTP
/// handlers and the management API.
pub(crate) struct TransferConsumer {
    service: Service,
    held: AnyHeldAgreements,
    link: Link,
    allow_http: bool,
    /// Our push data addresses (ADR 0027): `None` without object storage
    /// and a push bucket, and then push transfers are refused.
    push: Option<PushAddresses>,
}

impl TransferConsumer {
    pub(crate) fn new(
        ports: &Ports,
        callback_address: String,
        allow_http: bool,
        push: Option<PushAddresses>,
    ) -> Self {
        let transfer_ports = ports.consumer_transfers.clone();
        Self {
            service: ConsumerTransferService::new(
                transfer_ports.store,
                transfer_ports.decider,
                SystemEnvironment,
                callback_address,
            ),
            held: transfer_ports.held,
            link: Link::new(ports),
            allow_http,
            push,
        }
    }

    /// Requests a transfer under `agreement_id`, one of our agreements, in
    /// `format`, from the provider that granted it. The request is sent in
    /// the background.
    ///
    /// # Errors
    ///
    /// We hold no such agreement, the format is empty, or the store failed.
    pub(crate) fn request(
        self: &Arc<Self>,
        agreement_id: &str,
        format: &str,
    ) -> Result<ConsumerTransfer, Box<Failure>> {
        if is_push_format(format) {
            self.push_addresses()?;
        }
        let provider = self
            .held
            .provider_of(agreement_id)
            .map_err(|e| Box::new(Failure::Store(e)))?
            .ok_or_else(|| Box::new(refused("no finalized negotiation holds this agreement")))?;
        self.start(&provider.address, &provider.id, agreement_id, format)
    }

    /// Requests a transfer from the provider at `provider_address`, without
    /// checking the agreement (the TCK's consumer tests use agreements
    /// they never negotiated). The request is sent in the background.
    ///
    /// # Errors
    ///
    /// The address is not an allowed URL, an id is empty, a push without
    /// object storage, or the store failed.
    pub(crate) fn start(
        self: &Arc<Self>,
        provider_address: &str,
        provider_id: &str,
        agreement_id: &str,
        format: &str,
    ) -> Result<ConsumerTransfer, Box<Failure>> {
        if !outbound_url_allowed(provider_address, self.allow_http) {
            return Err(Box::new(refused(
                "the provider address must be an https URL",
            )));
        }
        let (transfer, out) = if is_push_format(format) {
            let request = Request {
                provider_address,
                provider_id,
                agreement_id,
                format,
            };
            self.start_push(&request)?
        } else {
            self.service
                .start(provider_address, provider_id, agreement_id, format)
                .map_err(Box::new)?
        };
        tokio::spawn(outbox::deliver(Arc::clone(self), out));
        Ok(transfer)
    }

    /// The request of a push transfer: with our data address and a new
    /// token, whose hash is kept before the request is sent.
    fn start_push(
        &self,
        request: &Request<'_>,
    ) -> Result<(ConsumerTransfer, Outgoing), Box<Failure>> {
        let push = self.push_addresses()?;
        let token =
            PushAddresses::new_token().map_err(|e| Box::new(Failure::Store(StoreError(e))))?;
        let (transfer, out) = self
            .service
            .start_push(request, &|consumer_pid| push.address(consumer_pid, &token))
            .map_err(Box::new)?;
        push.keep(&transfer.consumer_pid, &token)
            .map_err(|e| Box::new(Failure::Store(e)))?;
        Ok((transfer, out))
    }

    /// Our push addresses, or the refusal of a push transfer.
    fn push_addresses(&self) -> Result<&PushAddresses, Box<Failure>> {
        self.push.as_ref().ok_or_else(|| {
            Box::new(refused(
                "push transfers need object storage settings and PORTITOR_PUSH_BUCKET",
            ))
        })
    }

    /// The object the data of push transfer `consumer_pid` went into, once
    /// some arrived.
    pub(crate) fn object_key(&self, consumer_pid: &str) -> Option<String> {
        self.push.as_ref()?.object_key(consumer_pid)
    }

    /// Our own step on a transfer (suspend, start again, complete or
    /// terminate). The message is sent in the background. Returns the
    /// transfer with that message pending.
    ///
    /// # Errors
    ///
    /// Unknown transfer, a step its state does not allow, a message still
    /// on its way, or the store failed.
    pub(crate) fn act(
        self: &Arc<Self>,
        consumer_pid: &str,
        decision: ConsumerTransferDecision,
    ) -> Result<ConsumerTransfer, Box<Failure>> {
        let push_again = decision == ConsumerTransferDecision::Start
            && is_push_format(&self.transfer(consumer_pid)?.format);
        let out = if push_again {
            // a new token, which replaces the one the provider has
            let push = self.push_addresses()?;
            let token =
                PushAddresses::new_token().map_err(|e| Box::new(Failure::Store(StoreError(e))))?;
            let address = push.address(consumer_pid, &token);
            let out = self
                .service
                .act_with_address(consumer_pid, decision, Some(&address))
                .map_err(Box::new)?;
            push.keep(consumer_pid, &token)
                .map_err(|e| Box::new(Failure::Store(e)))?;
            out
        } else {
            self.service.act(consumer_pid, decision).map_err(Box::new)?
        };
        tokio::spawn(outbox::deliver(Arc::clone(self), out));
        self.transfer(consumer_pid)
    }

    pub(crate) fn transfer(&self, consumer_pid: &str) -> Result<ConsumerTransfer, Box<Failure>> {
        self.service.transfer(consumer_pid).map_err(Box::new)
    }

    pub(crate) fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerTransfer>, Box<Failure>> {
        self.service.list(after, limit).map_err(Box::new)
    }

    /// Sends every message saved as pending, for example after a restart.
    /// Returns how many.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub(crate) fn resume(self: &Arc<Self>) -> Result<usize, Box<Failure>> {
        let pending = self.service.pending().map_err(Box::new)?;
        Ok(outbox::resume(self, pending))
    }

    /// Refuses the request unless `caller` is the provider of transfer
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
        body: Body,
        message_type: &str,
        consumer_pid: &str,
    ) -> Result<T, Box<TransferError>> {
        read_message(
            &*self.link.guard,
            body,
            message_type,
            "providerPid",
            |ppid, reason| TransferError::new(ppid, consumer_pid, "400", reason),
        )
    }

    /// Answers the provider and starts sending our next message, if any.
    fn answer(self: &Arc<Self>, handled: Handled) -> Response {
        match handled {
            Ok((ack, next)) => {
                if let Some(out) = next {
                    tokio::spawn(outbox::deliver(Arc::clone(self), out));
                }
                (StatusCode::OK, Json(ack)).into_response()
            }
            Err(failure) => failed(failure),
        }
    }
}

impl Outbox for TransferConsumer {
    type Out = Outgoing;
    type Failure = Failure;

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
        reached_state(state, out.to)
    }

    /// A transfer request makes the provider evaluate the agreement again
    /// with our credentials: our token lets it read those its constraints
    /// need (ADR 0019). An agreement we do not hold (the TCK's) needs none.
    fn scopes(&self, out: &Outgoing) -> Vec<String> {
        if !out.initial {
            return Vec::new();
        }
        let Ok(transfer) = self.service.transfer(&out.consumer_pid) else {
            return Vec::new();
        };
        match self.held.agreement(&transfer.agreement_id) {
            Ok(Some(agreement)) => credential_scopes(&agreement, &self.link.credential_types),
            Ok(None) | Err(_) => Vec::new(),
        }
    }

    fn link(&self) -> &Link {
        &self.link
    }

    fn kind(&self) -> metrics::Outbox {
        metrics::Outbox::Transfer(metrics::Role::Consumer)
    }

    fn delivered(&self, out: &Outgoing, ack: Option<&Value>) -> Result<Option<Outgoing>, Failure> {
        self.service.delivered(out, ack)
    }

    fn abort(&self, out: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        self.service.abort(out, "the provider could not be reached")
    }

    fn not_started(&self, out: &Outgoing) -> Result<(), Failure> {
        self.service.not_started(&out.consumer_pid)
    }
}

fn refused(reason: &str) -> Failure {
    Failure::BadRequest(TransferError::new("", "", "400", reason))
}

type Consumer = State<Arc<TransferConsumer>>;

/// `GET <callback>/transfers/{consumerPid}`.
pub(crate) async fn get(
    State(consumer): Consumer,
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

/// `POST <callback>/transfers/{consumerPid}/start`.
pub(crate) async fn start(
    State(consumer): Consumer,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: TransferStartMessage = match consumer.read(body, "TransferStartMessage", &cpid) {
        Ok(m) => m,
        Err(refused) => return bad_request(*refused),
    };
    consumer.answer(consumer.service.started(&cpid, &msg))
}

/// `POST <callback>/transfers/{consumerPid}/suspension`.
pub(crate) async fn suspension(
    State(consumer): Consumer,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: TransferSuspensionMessage =
        match consumer.read(body, "TransferSuspensionMessage", &cpid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    consumer.answer(
        consumer
            .service
            .provider_step(&cpid, ids, Transition::Suspend),
    )
}

/// `POST <callback>/transfers/{consumerPid}/completion`.
pub(crate) async fn completion(
    State(consumer): Consumer,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: TransferCompletionMessage =
        match consumer.read(body, "TransferCompletionMessage", &cpid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    consumer.answer(
        consumer
            .service
            .provider_step(&cpid, ids, Transition::Complete),
    )
}

/// `POST <callback>/transfers/{consumerPid}/termination`.
pub(crate) async fn termination(
    State(consumer): Consumer,
    Extension(caller): Extension<Caller>,
    Path(cpid): Path<String>,
    body: Body,
) -> Response {
    if let Some(refused) = consumer.refusal(&cpid, &caller.id) {
        return refused;
    }
    let msg: TransferTerminationMessage =
        match consumer.read(body, "TransferTerminationMessage", &cpid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    let handled = consumer
        .service
        .provider_step(&cpid, ids, Transition::Terminate);
    if handled.is_ok() {
        log_termination_without_membership(&caller, &cpid);
    }
    consumer.answer(handled)
}
