// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapters for the provider side of the transfer process: the HTTPS
//! binding (DSP 2025-1, "Transfer Process HTTPS Binding"), in-memory
//! transfers and agreements, and the sending of the provider's messages
//! (ADR 0013).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use portitor_dsp::negotiation::Agreement;
use portitor_dsp::negotiation::provider::{Negotiation, NegotiationStore};
use portitor_dsp::policy::{CallerCredentials, credential_scopes};
use portitor_dsp::transfer::provider::{
    Agreements, DataFlows, Failure, Handled, Outgoing, StoreError, Transfer, TransferService,
    TransferStore,
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
use crate::harvest::DatasetCopy;
use crate::http_client::outbound_url_allowed;
use crate::negotiation::InMemoryNegotiations;
use crate::outbox::{self, Link, Message, Outbox};
use crate::ports::{AnyAgreements, AnyDataFlows, AnyTransferDecider, AnyTransferStore};
use crate::store::finalized_agreement;
use crate::system::SystemEnvironment;

/// Adapter: transfers kept in memory (lost on restart). The `SQLite`
/// adapter replaces it for real use.
#[derive(Debug, Clone, Default)]
pub struct InMemoryTransfers(Arc<Mutex<BTreeMap<String, Transfer>>>);

impl InMemoryTransfers {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Transfer>> {
        // every write is a single insert, so a panic cannot leave half a
        // transfer behind
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The transfer in `map` that the consumer `counter_party` asked for with
/// its process id `consumer_pid`, if any.
fn same_request<'a>(
    map: &'a BTreeMap<String, Transfer>,
    counter_party: &str,
    consumer_pid: &str,
) -> Option<&'a Transfer> {
    map.values().find(|transfer| {
        transfer.counter_party == counter_party && transfer.consumer_pid == consumer_pid
    })
}

impl TransferStore for InMemoryTransfers {
    fn get(&self, provider_pid: &str) -> Result<Option<Transfer>, StoreError> {
        Ok(self.map().get(provider_pid).cloned())
    }

    fn requested(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, StoreError> {
        Ok(same_request(&self.map(), counter_party, consumer_pid).cloned())
    }

    fn insert(&self, transfer: &Transfer) -> Result<bool, StoreError> {
        // one lock for the lookup and the insert, so two copies of a
        // request cannot both be added
        let mut map = self.map();
        if map.contains_key(&transfer.provider_pid) {
            return Err(StoreError(format!("{} exists", transfer.provider_pid)));
        }
        if same_request(&map, &transfer.counter_party, &transfer.consumer_pid).is_some() {
            return Ok(false);
        }
        map.insert(transfer.provider_pid.clone(), transfer.clone());
        Ok(true)
    }

    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, StoreError> {
        let mut map = self.map();
        let current = map.get(&transfer.provider_pid).map(|old| old.version);
        if current != Some(expected) {
            return Ok(false);
        }
        map.insert(transfer.provider_pid.clone(), transfer.clone());
        Ok(true)
    }

    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        let map = self.map();
        Ok(map
            .values()
            .filter(|transfer| transfer.pending.is_some())
            .cloned()
            .collect())
    }
}

impl Agreements for InMemoryNegotiations {
    fn finalized(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        let map = self.map();
        Ok(map
            .values()
            .find_map(|negotiation| finalized_agreement(negotiation, agreement_id)))
    }

    fn negotiation_of(&self, agreement_id: &str) -> Result<Option<Negotiation>, StoreError> {
        let map = self.map();
        let made_it = |negotiation: &&Negotiation| {
            negotiation
                .agreement
                .as_ref()
                .is_some_and(|agreement| agreement.id == agreement_id)
        };
        Ok(map.values().find(made_it).cloned())
    }

    fn replace_negotiation_if(
        &self,
        negotiation: &Negotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        NegotiationStore::replace_if(self, negotiation, expected)
    }
}

/// The data plane until one is configured (0.4 PR 4): it supports no
/// format, so every transfer request is refused with a clear reason.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDataFlows;

impl DataFlows for NoDataFlows {
    fn supports(&self, _: &str, _: &str) -> bool {
        false
    }
    fn start(&self, _: &Transfer) -> Result<DataAddress, String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn suspend(&self, _: &str) -> Result<(), String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn restart(&self, _: &str) -> Result<DataAddress, String> {
        Err(NO_DATA_PLANE.to_owned())
    }
    fn resume(&self, _: &str) -> Result<(), String> {
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
    allow_http: bool,
}

impl TransferProvider {
    pub(crate) fn new(ports: &crate::Ports, catalog: DatasetCopy, allow_http: bool) -> Self {
        let transfer = ports.transfers.clone();
        Self {
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
            allow_http,
        }
    }

    /// The caller's verified credentials for the credential constraints of
    /// its agreement `agreement_id`, if it holds that agreement (ADR 0019).
    async fn credentials(&self, caller: &Caller, agreement_id: &str) -> CallerCredentials {
        let types = &self.link.credential_types;
        let scopes = match self.service.agreement(agreement_id, &caller.id) {
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
        Ok(outbox::resume(self, pending))
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
        self.service.delivered(out)
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
    // the agreement is evaluated again with fresh credentials (ADR 0019)
    let credentials = provider.credentials(&caller, &msg.agreement_id).await;
    let handled = provider.service.request(&msg, &caller.id, &credentials);
    if let Err(failure) = &handled
        && failure.is_no_finalized_agreement()
    {
        log_no_finalized_agreement();
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
    let handled = provider.service.start(&pid, &msg, &caller.id);
    provider.finish(handled, StatusCode::OK)
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
    provider.finish(handled, StatusCode::OK)
}
