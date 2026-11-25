// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapters for the provider side of the transfer process: the HTTPS
//! binding (DSP 2025-1, "Transfer Process HTTPS Binding"), the data plane
//! until one is configured, and the sending of the provider's messages
//! (ADR 0013). A push transfer's data is sent once the transfer is
//! `STARTED` (`begin_push`, ADR 0027).
//!
//! From 1.1 (ADR 0027, section 7) also: the provider's own view and
//! termination of its transfers, for the management API; the consumer's
//! memberships, kept with each new transfer for the data plane; and the
//! termination of a transfer whose consumer is no longer a member
//! ([`EndsThroughProvider`]).

use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use portitor_dsp::policy::{CallerCredentials, credential_scopes};
use portitor_dsp::transfer::provider::{
    DataFlows, Failure, Handled, Outgoing, Transfer, TransferService,
};
use portitor_dsp::transfer::{
    DataAddress, TransferCompletionMessage, TransferError, TransferProcess, TransferRequestMessage,
    TransferStartMessage, TransferState, TransferSuspensionMessage, TransferTerminationMessage,
    Transition,
};
use portitor_runtime::telemetry::metrics;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::auth::{AnyVerifier, Caller, credentials_for};
use crate::flow_membership::EndsTransfers;
use crate::flow_membership::MembershipStore as _;
use crate::harvest::DatasetCopy;
use crate::http_client::outbound_url_allowed;
use crate::outbox::{self, Link, Message, Outbox};
use crate::ports::{
    AnyAgreements, AnyDataFlows, AnyFlowStore, AnyTransferDecider, AnyTransferStore,
};
use crate::system::SystemEnvironment;
use portitor_dataplane::{HttpDataPlane, PushJob};

/// The data plane until one is configured (0.4 PR 4): it supports no
/// format, so every transfer request is refused with a clear reason.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDataFlows;

impl DataFlows for NoDataFlows {
    fn supports(&self, _: &str, _: &str) -> bool {
        false
    }
    fn start(&self, _: &Transfer) -> Result<Option<DataAddress>, String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn suspend(&self, _: &str) -> Result<(), String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn restart(&self, _: &Transfer) -> Result<Option<DataAddress>, String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn resume(&self, _: &str, _: Option<&DataAddress>) -> Result<(), String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn complete(&self, _: &str) -> Result<(), String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn terminate(&self, _: &str) -> Result<(), String> {
        // nothing was started, so there is nothing to end
        Ok(())
    }
}

const NO_DATA_PLANE: &str = "this connector has no data plane";

type Service = TransferService<
    AnyTransferStore,
    AnyAgreements,
    AnyDataFlows,
    AnyTransferDecider,
    DatasetCopy,
    SystemEnvironment,
>;

/// The provider side of the transfer process, as used by the HTTP
/// handlers.
pub(crate) struct TransferProvider {
    service: Service,
    link: Link,
    /// Asks a caller for more credentials when an agreement needs them.
    verifier: AnyVerifier,
    /// Where the consumer's memberships are kept for the data plane (ADR
    /// 0027); `None` without the built-in data plane.
    flow_store: Option<AnyFlowStore>,
    allow_http: bool,
    /// The built-in data plane, which sends the data of push transfers.
    plane: Option<Arc<HttpDataPlane>>,
    /// This provider itself, for the pushes it starts in the background
    /// from a method that only has `&self` (`Weak`: a pointer that does
    /// not keep the provider alive by itself).
    me: Weak<TransferProvider>,
}

impl TransferProvider {
    /// The provider, shared. `plane`: the built-in data plane, if any.
    pub(crate) fn new_shared(
        ports: &crate::Ports,
        catalog: DatasetCopy,
        allow_http: bool,
        plane: Option<Arc<HttpDataPlane>>,
    ) -> Arc<Self> {
        let transfer = ports.transfers.clone();
        // `new_cyclic` hands over the `Weak` pointer to the `Arc` it makes
        Arc::new_cyclic(|me| Self {
            service: TransferService::new(
                transfer.store,
                transfer.agreements,
                transfer.flows,
                transfer.decider,
                catalog,
                SystemEnvironment,
            ),
            link: Link::new(ports),
            verifier: Arc::clone(&ports.verifier),
            flow_store: ports.flow_store.clone(),
            allow_http,
            plane,
            me: me.clone(),
        })
    }

    /// Starts sending the data of transfer `provider_pid` in the
    /// background if it is a push transfer that is `STARTED` and not being
    /// sent already; does nothing otherwise.
    fn begin_push(&self, provider_pid: &str) {
        self.begin_push_once_started(provider_pid, None);
    }

    /// [`Self::begin_push`], but with `consumer` (its state URL and its
    /// id) the push first waits until the consumer has applied our start
    /// ([`until_consumer_started`]), or until the flow is cut. A push that
    /// cannot begin ends the transfer: otherwise it would stay `STARTED`
    /// with nothing sending its data.
    fn begin_push_once_started(&self, provider_pid: &str, consumer: Option<(String, String)>) {
        let (Some(plane), Some(me)) = (&self.plane, self.me.upgrade()) else {
            return;
        };
        match plane.push_job(provider_pid) {
            Ok(Some(mut job)) => {
                let plane = Arc::clone(plane);
                tokio::spawn(async move {
                    if let Some((status_url, audience)) = consumer {
                        // a cut while waiting: the push sees it and stops
                        tokio::select! {
                            () = until_consumer_started(&me.link, &status_url, &audience) => {}
                            () = job.cut.cut() => {}
                        }
                    }
                    run_push(me, plane, job).await;
                });
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(flow = %provider_pid, %error, "push: cannot start; the transfer is terminated");
                let pid = provider_pid.to_owned();
                tokio::spawn(async move {
                    me.push_finished(&pid, Some("the push could not start"))
                        .await;
                });
            }
        }
    }

    /// The push of `provider_pid` ended: complete or terminate the
    /// transfer, and send that message.
    async fn push_finished(self: Arc<Self>, provider_pid: &str, failure: Option<&str>) {
        match self.service.push_finished(provider_pid, failure) {
            Ok(Some(out)) => outbox::deliver(self, out).await,
            Ok(None) => {}
            Err(failure) => {
                tracing::error!(flow = %provider_pid, ?failure, "push: the end of the transfer could not be saved");
            }
        }
    }

    /// The transfer `provider_pid`, for the management API.
    ///
    /// # Errors
    ///
    /// Unknown transfer, or store failure.
    pub(crate) fn transfer(&self, provider_pid: &str) -> Result<Transfer, Box<Failure>> {
        self.service.transfer(provider_pid).map_err(Box::new)
    }

    /// One page of transfers, for the management API.
    ///
    /// # Errors
    ///
    /// Store failure.
    pub(crate) fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Transfer>, Box<Failure>> {
        self.service.list(after, limit).map_err(Box::new)
    }

    /// The provider ends transfer `provider_pid` for `reason`: the data
    /// flow is terminated at once, which cuts its running downloads, and
    /// the termination is sent in the background (ADR 0014, "The
    /// provider's own steps"). Returns the transfer as saved.
    ///
    /// # Errors
    ///
    /// Unknown or ended transfer, data plane or store failure.
    pub(crate) fn terminate_own(
        self: &Arc<Self>,
        provider_pid: &str,
        reason: &str,
    ) -> Result<Transfer, Box<Failure>> {
        let (transfer, out) = self
            .service
            .terminate_own(provider_pid, reason)
            .map_err(Box::new)?;
        if let Some(out) = out {
            tokio::spawn(outbox::deliver(Arc::clone(self), out));
        }
        Ok(transfer)
    }

    /// Keeps the `MembershipCredential`s of `caller`, just verified, with
    /// the data flow of transfer `provider_pid`, so the data plane can
    /// check them again (ADR 0027, section 7.2). Called before the start
    /// message is sent, so before the consumer has a token. A failure is
    /// logged: the flow then has nothing kept and is refused (fail
    /// closed).
    fn keep_memberships(&self, provider_pid: &str, caller: &Caller) {
        let Some(store) = &self.flow_store else {
            return;
        };
        if let Err(error) = store.keep_memberships(provider_pid, &caller.memberships) {
            tracing::error!(transfer = %provider_pid, error = %error.0, "the consumer's memberships could not be kept; its requests for data will be refused");
        }
    }

    /// The caller's verified credentials for the credential constraints of
    /// its agreement `agreement_id`, if it holds that agreement (ADR 0019).
    async fn credentials(&self, caller: &Caller, agreement_id: &str) -> CallerCredentials {
        let types = &self.link.credential_types;
        // not only a read: an agreement whose finalization event is on
        // its way to this caller is recorded as FINALIZED here (ADR 0007,
        // amended in 0.13), before the request itself is checked
        let held = self
            .service
            .agreement_settling_finalization(agreement_id, &caller.id);
        let scopes = match held {
            Ok(Some(agreement)) => credential_scopes(&agreement, types),
            // the request is refused anyway
            Ok(None) | Err(_) => Vec::new(),
        };
        credentials_for(&*self.verifier, types, caller, &scopes).await
    }

    /// Checks the message's shape with the guard, then reads it.
    fn read<T: DeserializeOwned>(
        &self,
        body: Body,
        message_type: &str,
        provider_pid: &str,
    ) -> Result<T, Box<TransferError>> {
        crate::inbound::read_message(
            &*self.link.guard,
            body,
            message_type,
            "consumerPid",
            |cpid, reason| TransferError::new(provider_pid, cpid, "400", reason),
        )
    }

    /// Sends every message saved as pending, for example after a restart.
    /// Returns how many.
    pub(crate) fn resume(self: &Arc<Self>) -> Result<usize, Box<Failure>> {
        let pending = self.service.pending().map_err(Box::new)?;
        self.resume_pushes();
        Ok(outbox::resume(self, pending))
    }

    /// Sends again, from the start, the pushes a restart cut: transfers
    /// that are `STARTED`, with nothing of ours on its way. A push whose
    /// start message is still on its way begins once it is delivered.
    fn resume_pushes(&self) {
        let Some(plane) = &self.plane else {
            return;
        };
        let started = match plane.started_pushes() {
            Ok(started) => started,
            Err(error) => {
                tracing::error!(%error, "push: the started pushes cannot be read");
                return;
            }
        };
        for pid in started {
            let ready = self
                .service
                .transfer(&pid)
                .ok()
                .is_some_and(|t| t.state == TransferState::Started && t.pending.is_none());
            if ready {
                self.begin_push(&pid);
            }
        }
    }

    /// Turns the service's answer into the HTTP response and starts
    /// sending whatever the provider sends next.
    fn finish(self: &Arc<Self>, handled: Handled, ok: StatusCode) -> Response {
        match handled {
            Ok((ack, next)) => {
                if let Some(out) = next {
                    tokio::spawn(outbox::deliver(Arc::clone(self), out));
                }
                (ok, Json(ack)).into_response()
            }
            Err(failure) => failed(failure),
        }
    }
}

impl Outbox for TransferProvider {
    type Out = Outgoing;
    type Failure = Failure;

    fn link(&self) -> &Link {
        &self.link
    }

    fn kind(&self) -> metrics::Outbox {
        metrics::Outbox::Transfer(metrics::Role::Provider)
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
        reached_state(state, out.to)
    }

    fn delivered(&self, out: &Outgoing, _: Option<&Value>) -> Result<Option<Outgoing>, Failure> {
        let next = self.service.delivered(out)?;
        if out.transition == Transition::Start {
            // the consumer has our start: a push transfer's data goes as
            // soon as the consumer has applied it
            let consumer = (out.status_url.clone(), out.counter_party.clone());
            self.begin_push_once_started(&out.provider_pid, Some(consumer));
        }
        Ok(next)
    }

    fn abort(&self, out: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        self.service.abort(out, "the consumer could not be reached")
    }
}

/// Whether `state`, a counter-party's answer to a transfer state query,
/// is `to`.
pub(crate) fn reached_state(state: Value, to: TransferState) -> bool {
    serde_json::from_value::<TransferProcess>(state).is_ok_and(|transfer| transfer.state == to)
}

pub(crate) fn bad_request(e: TransferError) -> Response {
    (StatusCode::BAD_REQUEST, Json(e)).into_response()
}

pub(crate) fn failed(failure: Failure) -> Response {
    let internal = |what: &str, error: &str| {
        tracing::error!(%error, "{what} failed");
        let e = TransferError::new("", "", "500", "internal error");
        (StatusCode::INTERNAL_SERVER_ERROR, Json(e)).into_response()
    };
    match failure {
        Failure::BadRequest(e) => bad_request(e),
        Failure::NotFound(e) => (StatusCode::NOT_FOUND, Json(e)).into_response(),
        Failure::Store(e) => internal("transfer store", &e.to_string()),
        Failure::DataPlane(e) => internal("data plane", &e),
    }
}

type Provider = State<Arc<TransferProvider>>;
pub(crate) use crate::inbound::Body;

/// `GET /transfers/{providerPid}`.
pub(crate) async fn get(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
) -> Response {
    match provider.service.get(&pid, &caller.id) {
        Ok(ack) => Json(ack).into_response(),
        Err(f) => failed(f),
    }
}

/// `POST /transfers/request`.
pub(crate) async fn request(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    body: Body,
) -> Response {
    let msg: TransferRequestMessage = match provider.read(body, "TransferRequestMessage", "") {
        Ok(m) => m,
        Err(refused) => return bad_request(*refused),
    };
    if !outbound_url_allowed(&msg.callback_address, provider.allow_http) {
        let reason = "callbackAddress must be an https URL";
        return bad_request(TransferError::new("", &msg.consumer_pid, "400", reason));
    }
    if !push_endpoint_allowed(msg.data_address.as_ref(), provider.allow_http) {
        let reason = "the dataAddress endpoint must be an https URL";
        return bad_request(TransferError::new("", &msg.consumer_pid, "400", reason));
    }
    // the agreement is evaluated again with fresh credentials (ADR 0019)
    let credentials = provider.credentials(&caller, &msg.agreement_id).await;
    let handled = provider.service.request(&msg, &caller.id, &credentials);
    match &handled {
        Ok((transfer, _)) => provider.keep_memberships(&transfer.provider_pid, &caller),
        Err(failure) if failure.is_no_finalized_agreement() => log_no_finalized_agreement(),
        Err(_) => {}
    }
    provider.finish(handled, StatusCode::CREATED)
}

/// One INFO line for a transfer request refused because it names no
/// finalized agreement of the caller, so that the operator can see why a
/// partner's transfer ended (TD-44). The line names neither the agreement
/// nor the caller: the id in the request is the caller's claim and may be
/// another participant's agreement.
fn log_no_finalized_agreement() {
    tracing::info!(
        message_type = "TransferRequestMessage",
        reason = "no finalized agreement with this id for the caller",
        "transfer request is refused; answered 400"
    );
}

/// `POST /transfers/{providerPid}/start`.
pub(crate) async fn start(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: Body,
) -> Response {
    let msg: TransferStartMessage = match provider.read(body, "TransferStartMessage", &pid) {
        Ok(m) => m,
        Err(refused) => return bad_request(*refused),
    };
    if !push_endpoint_allowed(msg.data_address.as_ref(), provider.allow_http) {
        let reason = "the dataAddress endpoint must be an https URL";
        return bad_request(TransferError::new(&pid, &msg.consumer_pid, "400", reason));
    }
    let handled = provider.service.start(&pid, &msg, &caller.id);
    let started = matches!(&handled, Ok((ack, _)) if ack.state == TransferState::Started);
    let response = provider.finish(handled, StatusCode::OK);
    if started {
        // started again: a push transfer sends its data again (ADR 0027)
        provider.begin_push(&pid);
    }
    response
}

/// Whether the endpoint of a push `address`, chosen by the consumer, may
/// be called: `https://`, or `http://` with `PORTITOR_ALLOW_HTTP` (ADR
/// 0027, section 1; the HTTP profile says a receiving party MUST reject a
/// non-HTTPS endpoint). No address, nothing to check.
fn push_endpoint_allowed(address: Option<&DataAddress>, allow_http: bool) -> bool {
    match address.and_then(|a| a.endpoint.as_deref()) {
        Some(endpoint) => outbound_url_allowed(endpoint, allow_http),
        None => true,
    }
}

/// The longest a push waits for the consumer to apply our start.
const START_WAIT: Duration = Duration::from_secs(30);

/// Waits until the consumer at `status_url` no longer says the transfer is
/// `REQUESTED`, for at most [`START_WAIT`]. A consumer may acknowledge our
/// start and apply it later: Eclipse EDC answers 200, moves to `STARTED`
/// only once it has signalled its data plane, and until then refuses a
/// completion with 400, which would end the transfer. Any other state, no
/// answer, or the time limit ends the wait: the push goes ahead.
async fn until_consumer_started(link: &Link, status_url: &str, audience: &str) {
    let still_requested = || {
        outbox::arrived(link, status_url, audience, |state| {
            reached_state(state, TransferState::Requested)
        })
    };
    let mut delay = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < START_WAIT && still_requested().await {
        tokio::time::sleep(delay).await;
        waited += delay;
        delay = delay.saturating_mul(2).min(Duration::from_secs(2));
    }
}

/// Sends `job` (a push of `provider`'s), writes the audit line, then
/// completes or terminates the transfer. A push that suspend or terminate
/// cut leaves the transfer as they made it; any other cut is a failure
/// ([`portitor_dataplane::push::send`]).
async fn run_push(provider: Arc<TransferProvider>, plane: Arc<HttpDataPlane>, mut job: PushJob) {
    let pid = job.flow_id.clone();
    let transfer = provider.service.transfer(&pid).ok();
    let pushed = portitor_dataplane::push::send(&plane, &mut job).await;
    // the push is over: a later start again may send again
    drop(job);
    let (agreement, counter_party) = match &transfer {
        Some(t) => (t.agreement_id.as_str(), t.counter_party.as_str()),
        // the transfer could not be read: the audit line names no one
        None => ("", ""),
    };
    portitor_dataplane::push::audit(&pid, agreement, counter_party, &pushed);
    let failure = match &pushed.sent {
        portitor_dataplane::push::Sent::Done => None,
        portitor_dataplane::push::Sent::Failed(reason) => Some(reason.as_str()),
        portitor_dataplane::push::Sent::Cancelled => return,
    };
    provider.push_finished(&pid, failure).await;
}

/// `POST /transfers/{providerPid}/suspension`.
pub(crate) async fn suspension(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: Body,
) -> Response {
    let msg: TransferSuspensionMessage =
        match provider.read(body, "TransferSuspensionMessage", &pid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    let handled = provider.service.suspend(&pid, ids, &caller.id);
    provider.finish(handled, StatusCode::OK)
}

/// `POST /transfers/{providerPid}/completion`.
pub(crate) async fn completion(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: Body,
) -> Response {
    let msg: TransferCompletionMessage =
        match provider.read(body, "TransferCompletionMessage", &pid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    let handled = provider.service.complete(&pid, ids, &caller.id);
    provider.finish(handled, StatusCode::OK)
}

/// `POST /transfers/{providerPid}/termination`.
pub(crate) async fn termination(
    State(provider): Provider,
    Extension(caller): Extension<Caller>,
    Path(pid): Path<String>,
    body: Body,
) -> Response {
    let msg: TransferTerminationMessage =
        match provider.read(body, "TransferTerminationMessage", &pid) {
            Ok(m) => m,
            Err(refused) => return bad_request(*refused),
        };
    let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
    let handled = provider.service.terminate(&pid, ids, &caller.id);
    if handled.is_ok() {
        log_termination_without_membership(&caller, &pid);
    }
    provider.finish(handled, StatusCode::OK)
}

/// One WARN line when a transfer termination was accepted from a caller
/// whose token is valid but whose credentials did not pass (TD-47; ADR
/// 0016, amended in 1.1): on the provider's side and on the consumer's
/// callback. `process` is our process id of the transfer. Nothing for a
/// caller that is a member.
pub(crate) fn log_termination_without_membership(caller: &Caller, process: &str) {
    if let Some(reason) = caller.without_membership() {
        tracing::warn!(
            %process,
            caller = %caller.id,
            %reason,
            "DSP termination accepted from a partner without membership"
        );
    }
}

/// Ends transfers for the data plane's membership gate
/// ([`crate::flow_membership::MembershipGate`]) as the provider's own
/// termination. It holds a weak pointer, because the provider holds the
/// data plane, which holds the gate: a strong one would keep all three
/// alive forever.
pub(crate) struct EndsThroughProvider(pub(crate) Weak<TransferProvider>);

impl EndsTransfers for EndsThroughProvider {
    fn end_transfer(&self, provider_pid: &str, reason: &str) {
        let Some(provider) = self.0.upgrade() else {
            return;
        };
        let why = format!("the consumer's membership no longer holds: {reason}");
        match provider.terminate_own(provider_pid, &why) {
            Ok(_) => {}
            // a second request after the first ended it
            Err(failure) => {
                tracing::info!(transfer = %provider_pid, failure = ?failure, "transfer not ended again");
            }
        }
    }
}
