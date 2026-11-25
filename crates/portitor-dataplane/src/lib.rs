// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The built-in HTTP data plane (ADR 0013, "The data"; ADR 0027). It
//! implements the Data Plane Signaling [`DataPlane`] port, serves the data
//! of pull transfers at `GET /data/{flowId}`, and holds what a push
//! transfer needs (the consumer's address) for the push sender
//! (`crate::push`).
//!
//! - Starting a data flow makes a random access token (32 bytes from the
//!   OS generator). Only its SHA-256 hash is stored. The token goes to the
//!   consumer once, inside the transfer's start message.
//! - A request for the data must carry the token as `Authorization: Bearer
//!   ...`. It is served only while the flow is STARTED and the token has
//!   not expired. Unknown flow, wrong token, suspended or ended flow, and
//!   expired token all get the same 401 answer, so a caller learns nothing
//!   about which flows exist.
//! - The data comes from the dataset's source (`portitor:source` in the
//!   datasets file): an upstream URL, fetched with a plain `GET`, or an
//!   object in the connector's object storage (`s3://{bucket}/{key}`, ADR
//!   0027, section 3), read with a signed `GET`. Either is streamed
//!   through.
//!   The control plane names the source in the start message's
//!   `metadata`, under [`SOURCE_METADATA_KEY`] (from 1.2; ADR 0028, step
//!   1), and the flow keeps it until it ends: a source changed later (a
//!   new harvest) reaches only new transfers, and transfers under
//!   agreements already made keep working after the dataset is withdrawn.
//!   The consumer's request headers are not passed on.
//! - A push flow (profile `http-push`) keeps the consumer's data address
//!   instead of a token of ours, until it ends. It has no token, so `/data`
//!   never serves it. Its sender (`crate::push`) watches the flow's
//!   [`RunningFlows`] switch, so suspend and terminate cut it too.
//! - Each served request writes one audit line, `data transfer done`,
//!   with the bytes sent and how it ended ([`crate::audit`]).
//! - From 1.1 (ADR 0027): every request also checks the consumer's
//!   membership again ([`AccessGate`], refused with the same 401),
//!   and a download that runs is cut when the flow is suspended or
//!   terminated ([`RunningFlows`]) or when the membership check, run again
//!   while it streams, fails (`download.rs`).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use http_body_util::Full;
use hyper::body::Bytes;
use portitor_dps::message::{
    DataFlowBaseMessage, DataFlowPrepareMessage, DataFlowResumeMessage, DataFlowStartMessage,
    DataFlowStartedNotificationMessage, DataFlowStatusMessage, DataFlowStatusResponseMessage,
    DataFlowSuspendMessage, DataFlowTerminateMessage, HTTP_PULL, HTTP_PUSH,
};
use portitor_dps::port::{DataPlane, SignalError};
use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::{DataAddress, DataAddressType, EndpointProperty, check_push_address};
use portitor_runtime::http::{
    HttpsClient, StreamingHttpsClient, bearer_token_in, https_client, https_streaming_client,
};
use portitor_runtime::system::{hex, new_uuid, now_secs};
use portitor_runtime::telemetry::http::{ClientCall, Peer};
use tracing::Instrument as _;

use crate::access::{Access, AccessGate};
use crate::audit::{AuditedBody, Direction, Outcome, Parties, transfer_done};
use crate::object_storage::{ObjectStorage, parse_s3_uri};
use crate::running_flows::{CutSignal, RunningFlows};

pub mod access;
pub mod audit;
mod download;
pub mod object_storage;
pub mod push;
pub mod running_flows;
#[cfg(any(test, feature = "test-support"))]
pub mod test_store;

/// The `endpointType` of the data address sent over DSP. It is the value
/// of the DSP 2025-1 examples, the DSP TCK and EDC's DSP tests (ADR 0013);
/// the EDC interop test in 0.5 confirms it.
pub const HTTP_ENDPOINT_TYPE: &str = "https://w3id.org/idsa/v4.1/HTTP";

/// Path of the data endpoint; the flow id follows.
pub const DATA_PATH: &str = "/data";

/// How long the upstream may take to send its answer's headers.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

/// The key, in the `metadata` of a Data Plane Signaling start message,
/// whose value is the dataset's source: an `http://` or `https://` URL, or
/// `s3://{bucket}/{key}`. It is the key operators write in the datasets
/// file. Data Plane Signaling v1.0-RC4 gives `metadata` no fixed shape and
/// names no key for this (maintainer's decision D2, 2026-11-25).
pub const SOURCE_METADATA_KEY: &str = "portitor:source";

/// One data flow as the data plane keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// The data flow id (the transfer's provider process id).
    pub id: String,
    /// The current state.
    pub state: DataFlowState,
    /// The dataset whose data is served.
    pub dataset_id: String,
    /// Where the data comes from: the dataset's source as the start
    /// message named it ([`SOURCE_METADATA_KEY`]). `None` only for a flow
    /// started before 1.2 whose dataset had no source in the harvest's
    /// read copy when the database was migrated: such a flow is never
    /// served, as 1.1 did not serve it either.
    pub source: Option<String>,
    /// The agreement the transfer is made under, as the start message
    /// named it (`agreementId`). Empty only for a flow started before 1.2
    /// whose transfer could not be found when the database was migrated.
    pub agreement_id: String,
    /// The consumer's participant id, as the start message named it
    /// (`counterPartyId`). Empty in the same case as `agreement_id`.
    pub counter_party_id: String,
    /// SHA-256 of the access token, hex. Empty for a push flow, which
    /// has no token of ours: no hash matches it.
    pub token_hash: String,
    /// When the token stops working, in seconds since 1970.
    pub expires_at: u64,
    /// A push flow: where to send the data, the consumer's data address
    /// with its token (ADR 0027). `None` for a pull flow, and once the
    /// flow has ended. Never logged.
    pub push_address: Option<DataAddress>,
}

impl Flow {
    /// The agreement and the consumer of this flow, for the audit line.
    #[must_use]
    pub fn parties(&self) -> Parties {
        Parties {
            agreement: self.agreement_id.clone(),
            counter_party: self.counter_party_id.clone(),
        }
    }
}

/// A flow store failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStoreError(pub String);

/// Port: where the data plane keeps its flows.
pub trait FlowStore {
    /// One flow by id.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError>;

    /// Adds a new flow. Returns `false` if the id is taken.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn insert(&self, flow: &Flow) -> Result<bool, FlowStoreError>;

    /// Changes the state of a flow from `from` to `to`, atomically.
    /// Returns `false` if the flow was not in state `from`.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError>;

    /// Replaces the token of a flow that is in `state`: the old token stops
    /// working. Returns `false` if the flow was not in `state`.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError>;

    /// Replaces the consumer's address of a push flow that is in `state`
    /// (a start again with a new token). Returns `false` if the flow was
    /// not in `state`. A flow that moves to a final state through
    /// [`FlowStore::set_state`] loses its address there.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError>;

    /// The ids of the push flows that are STARTED: after a restart, the
    /// pushes to send again.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError>;
}

/// SHA-256 of `token`, hex. Comparing two such hashes with `==` is safe:
/// what a timing difference could reveal is a prefix of the hash, and no
/// one can work back from a hash prefix to a token.
#[must_use]
pub fn hash(token: &str) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref())
}

/// A new random access token: 32 bytes from the OS generator, base64url.
/// # Errors
///
/// The OS has no randomness to give.
pub fn new_token() -> Result<String, SignalError> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SignalError::Failed("no randomness for the token".to_owned()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The built-in HTTP data plane.
#[derive(Clone)]
pub struct HttpDataPlane {
    flows: Arc<dyn FlowStore + Send + Sync>,
    public_url: String,
    token_seconds: u64,
    client: HttpsClient,
    /// The object storage of `s3://` sources, if the connector has one.
    storage: Option<Arc<ObjectStorage>>,
    /// The client of pushes: to an address a partner chose, so plain
    /// `http://` only with `PORTITOR_ALLOW_HTTP`.
    push_client: StreamingHttpsClient,
    /// The downloads that run now, by flow: cut on suspend and terminate.
    running: Arc<RunningFlows>,
    /// Checks the consumer's membership again; `None` checks the token
    /// only, as 1.0 did (unit tests).
    gate: Option<Arc<dyn AccessGate>>,
}

impl std::fmt::Debug for HttpDataPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDataPlane")
            .field("public_url", &self.public_url)
            .field("token_seconds", &self.token_seconds)
            .finish_non_exhaustive()
    }
}

/// A request for data that may be served.
struct Authorized {
    /// Where its data comes from.
    source: String,
    /// The agreement and the consumer, for the audit line.
    parties: Parties,
}

/// Why a request for data is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No token, wrong token, unknown flow, flow not STARTED, or token
    /// expired: HTTP 401, without saying which.
    Unauthorized,
    /// The flow store failed: HTTP 500.
    Store,
}

impl HttpDataPlane {
    /// A data plane that keeps its flows in `flows` and serves their data
    /// at `{public_url}/data/{flowId}`, with tokens valid for
    /// `token_seconds`. Each flow's source comes with its start message;
    /// upstream URLs are set by the operator, so plain `http://` is
    /// allowed for them.
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn new(
        flows: Arc<dyn FlowStore + Send + Sync>,
        public_url: &str,
        token_seconds: u64,
    ) -> Result<Self, String> {
        let (client, _) = https_client(true)?;
        Ok(Self {
            flows,
            public_url: public_url.trim_end_matches('/').to_owned(),
            token_seconds,
            client,
            storage: None,
            push_client: https_streaming_client(false)?,
            running: Arc::new(RunningFlows::new()),
            gate: None,
        })
    }

    /// The same data plane, reading `s3://` sources from `storage`.
    #[must_use]
    pub fn with_object_storage(mut self, storage: Option<Arc<ObjectStorage>>) -> Self {
        self.storage = storage;
        self
    }

    /// The same data plane, pushing to plain `http://` endpoints too
    /// when `allow_http` (`PORTITOR_ALLOW_HTTP`, development only).
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn with_push_over_http(mut self, allow_http: bool) -> Result<Self, String> {
        self.push_client = https_streaming_client(allow_http)?;
        Ok(self)
    }

    /// The client pushes are sent with.
    pub(crate) fn push_client(&self) -> &StreamingHttpsClient {
        &self.push_client
    }

    /// The same data plane, checking every request for data, and every
    /// running download, with `gate` (ADR 0027, section 7.2).
    #[must_use]
    pub fn checking_membership_with(self, gate: Arc<dyn AccessGate>) -> Self {
        Self {
            gate: Some(gate),
            ..self
        }
    }

    /// What runs for each flow now. A push uses the same switches, so a
    /// suspend or terminate cuts it too.
    #[must_use]
    pub fn running_flows(&self) -> Arc<RunningFlows> {
        Arc::clone(&self.running)
    }

    /// Moves flow `id` away from STARTED (to `to`) and cuts every download
    /// of it that runs (ADR 0027, section 5). Cut also when the move is
    /// refused: stopping a download is always safe.
    fn stop(&self, id: &str, to: DataFlowState) -> Result<(), SignalError> {
        let moved = self.move_to(id, to);
        let cut = self.running.cut(id);
        if cut > 0 {
            tracing::info!(flow = %id, downloads = cut, state = ?to, "running downloads cut");
        }
        moved
    }

    #[allow(clippy::needless_pass_by_value)] // used as map_err(Self::store_failed)
    fn store_failed(e: FlowStoreError) -> SignalError {
        SignalError::Failed(format!("flow store: {}", e.0))
    }

    fn flow(&self, id: &str) -> Result<Flow, SignalError> {
        self.flows
            .get(id)
            .map_err(Self::store_failed)?
            .ok_or_else(|| SignalError::UnknownDataFlow(id.to_owned()))
    }

    /// Moves flow `id` to `to`, if the state machine allows it.
    fn move_to(&self, id: &str, to: DataFlowState) -> Result<(), SignalError> {
        let flow = self.flow(id)?;
        flow.state.go_to(to)?;
        let moved = self
            .flows
            .set_state(id, flow.state, to)
            .map_err(Self::store_failed)?;
        if moved {
            Ok(())
        } else {
            Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )))
        }
    }

    /// The status message for flow `id` in `state`.
    fn status_message(
        id: &str,
        state: DataFlowState,
        address: Option<DataAddress>,
    ) -> DataFlowStatusMessage {
        DataFlowStatusMessage {
            message_id: new_uuid(),
            data_flow_id: id.to_owned(),
            state,
            data_address: address,
            error: None,
        }
    }

    /// Where the consumer fetches flow `id`'s data, with `token`.
    fn address(&self, id: &str, token: &str) -> DataAddress {
        DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_ENDPOINT_TYPE.to_owned(),
            endpoint: Some(format!("{}{DATA_PATH}/{id}", self.public_url)),
            endpoint_properties: vec![
                EndpointProperty::new("authorization", token),
                EndpointProperty::new("authType", "bearer"),
            ],
        }
    }

    /// Gives flow `id` (SUSPENDED or STARTED) a new token with a full
    /// lifetime, and returns its new address; the old token stops working.
    /// Not part of Data Plane Signaling: the provider needs it to start a
    /// pull transfer again, because only the token's hash is kept (ADR
    /// 0014).
    ///
    /// A flow that started before 1.1 gets no new token; the control
    /// plane checks that before it asks (ADR 0027, section 7).
    ///
    /// # Errors
    ///
    /// Unknown or ended flow, or the store failed.
    pub fn new_address(&self, id: &str) -> Result<DataAddress, SignalError> {
        let flow = self.flow(id)?;
        if flow.push_address.is_some() {
            return Err(SignalError::Invalid(format!(
                "data flow {id} is a push; it has no token of ours"
            )));
        }
        if !matches!(
            flow.state,
            DataFlowState::Suspended | DataFlowState::Started
        ) {
            return Err(SignalError::Invalid(format!(
                "data flow {id} is {:?}; it has no access to renew",
                flow.state
            )));
        }
        let token = new_token()?;
        let expires_at = now_secs().saturating_add(self.token_seconds);
        let renewed = self
            .flows
            .set_token(id, flow.state, &hash(&token), expires_at)
            .map_err(Self::store_failed)?;
        if !renewed {
            return Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )));
        }
        Ok(self.address(id, &token))
    }

    /// Checks a request for flow `id` with bearer `token`, and returns the
    /// upstream URL to fetch.
    ///
    /// # Errors
    ///
    /// See [`Denied`].
    pub fn authorize(&self, id: &str, token: &str, now: u64) -> Result<String, Denied> {
        let allowed = self.authorized(id, token, now)?;
        Ok(allowed.source)
    }

    /// [`HttpDataPlane::authorize`], with who the data goes to.
    fn authorized(&self, id: &str, token: &str, now: u64) -> Result<Authorized, Denied> {
        let flow = self
            .flows
            .get(id)
            .map_err(|_| Denied::Store)?
            .ok_or(Denied::Unauthorized)?;
        let allowed = flow.state == DataFlowState::Started
            && flow.push_address.is_none()
            && now < flow.expires_at
            && hash(token) == flow.token_hash;
        if !allowed {
            return Err(Denied::Unauthorized);
        }
        let parties = flow.parties();
        // a flow of 1.1 whose source could not be filled in has none
        // (`Flow::source`): it is refused like any other
        let Some(source) = flow.source else {
            return Err(Denied::Unauthorized);
        };
        Ok(Authorized { source, parties })
    }
}

impl HttpDataPlane {
    /// `start` for a push flow: the message carries the consumer's data
    /// address, which the flow keeps; the answer carries none.
    fn start_push(
        &self,
        message: &DataFlowStartMessage,
    ) -> Result<DataFlowStatusMessage, SignalError> {
        let request = &message.flow;
        let Some(address) = &message.data_address else {
            return Err(SignalError::Invalid(
                "a push start carries the consumer's data address".to_owned(),
            ));
        };
        check_push_address(address).map_err(SignalError::Invalid)?;
        let source = self.source_in(request)?;
        let flow = Flow {
            id: request.data_flow_id.clone(),
            state: DataFlowState::Started,
            dataset_id: request.dataset_id.clone(),
            source: Some(source),
            agreement_id: request.agreement_id.clone(),
            counter_party_id: request.counter_party_id.clone(),
            token_hash: String::new(),
            expires_at: 0,
            push_address: Some(address.clone()),
        };
        if !self.flows.insert(&flow).map_err(Self::store_failed)? {
            return Err(SignalError::DataFlowExists(flow.id));
        }
        Ok(Self::status_message(&flow.id, flow.state, None))
    }

    /// The dataset's source that the start message `request` names in its
    /// `metadata` ([`SOURCE_METADATA_KEY`]), if it is one this data plane
    /// can read: an `http(s)` URL, or an `s3://` object when the
    /// connector has object storage.
    fn source_in(&self, request: &DataFlowBaseMessage) -> Result<String, SignalError> {
        let dataset_id = &request.dataset_id;
        let named = match &request.metadata {
            Some(metadata) => metadata.get(SOURCE_METADATA_KEY),
            None => None,
        };
        let Some(named) = named else {
            return Err(SignalError::Invalid(format!(
                "dataset {dataset_id} has no source"
            )));
        };
        // the same forms the harvest accepts in the datasets file
        let source = match named.as_str() {
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => url,
            Some(uri) if parse_s3_uri(uri).is_some() => uri,
            _ => {
                return Err(SignalError::Invalid(format!(
                    "dataset {dataset_id}: {SOURCE_METADATA_KEY} must be an http:// or https:// URL, or s3://{{bucket}}/{{key}}"
                )));
            }
        };
        if !self.can_read(source) {
            return Err(SignalError::Invalid(format!(
                "dataset {dataset_id} is in object storage, which this connector does not have"
            )));
        }
        Ok(source.to_owned())
    }

    /// Replaces the consumer's address of push flow `id`, SUSPENDED.
    fn replace_push_address(&self, id: &str, address: &DataAddress) -> Result<(), SignalError> {
        let flow = self.flow(id)?;
        if flow.push_address.is_none() {
            return Err(SignalError::Invalid(
                "a pull resume carries no data address".to_owned(),
            ));
        }
        check_push_address(address).map_err(SignalError::Invalid)?;
        let replaced = self
            .flows
            .set_push_address(id, flow.state, address)
            .map_err(Self::store_failed)?;
        if replaced {
            Ok(())
        } else {
            Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )))
        }
    }
}

/// A push to send: what [`HttpDataPlane::push_job`] hands to the sender.
#[derive(Debug)]
pub struct PushJob {
    /// The data flow (the transfer's provider pid).
    pub flow_id: String,
    /// The dataset's source.
    pub source: String,
    /// The consumer's data address. Never logged.
    pub address: DataAddress,
    /// The agreement and the consumer, for the audit line.
    pub parties: Parties,
    /// Held while the push runs; fires on suspend or terminate.
    pub cut: CutSignal,
}

/// A source's answer: its media type and its bytes as they come.
pub struct SourceData {
    /// The source's `Content-Type`, if it gave one.
    pub content_type: Option<header::HeaderValue>,
    /// The bytes.
    pub body: Body,
}

impl HttpDataPlane {
    /// The push to send now for flow `id`, if it is a push flow that is
    /// STARTED and none of its pushes runs already. The caller sends it
    /// (`crate::push`) and keeps the job until it is done: its cut signal
    /// tells the data plane a push of this flow runs.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn push_job(&self, id: &str) -> Result<Option<PushJob>, SignalError> {
        // watch first, check second, as a download does
        // (`running_flows` module doc)
        let Some(cut) = self.running.watch_if_idle(id) else {
            // already being sent
            return Ok(None);
        };
        let flow = self.flow(id)?;
        let parties = flow.parties();
        let Some(address) = flow.push_address else {
            return Ok(None);
        };
        if flow.state != DataFlowState::Started {
            return Ok(None);
        }
        // a flow of 1.1 whose source could not be filled in has none
        // (`Flow::source`)
        let Some(source) = flow.source else {
            return Err(SignalError::Invalid(format!(
                "dataset {} has no source",
                flow.dataset_id
            )));
        };
        Ok(Some(PushJob {
            flow_id: flow.id,
            source,
            address,
            parties,
            cut,
        }))
    }

    /// The membership check before a push of flow `id` (ADR 0027,
    /// section 7.2): `Err(Cut)` when the membership was revoked (the
    /// transfer is then ended), `Err(Failed)` when it could not be checked.
    pub(crate) async fn push_membership(&self, id: &str) -> Result<(), Outcome> {
        let Some(gate) = &self.gate else {
            return Ok(());
        };
        match gate.check(id).await {
            Access::Granted => Ok(()),
            Access::Unchecked(reason) => {
                tracing::warn!(flow = %id, %reason, "push not sent: the consumer's membership could not be checked now");
                Err(Outcome::Failed)
            }
            Access::Revoked(reason) => {
                gate.end_transfer(id, &reason);
                Err(Outcome::Cut)
            }
        }
    }

    /// Whether flow `id` is still STARTED. A push that was cut while its
    /// flow is still STARTED was not cut by a suspension or a termination
    /// (they change the state first, then cut): see `crate::push`.
    pub(crate) fn is_started(&self, id: &str) -> bool {
        matches!(self.flows.get(id), Ok(Some(flow)) if flow.state == DataFlowState::Started)
    }

    /// The membership checks of a running push of flow `id`, as for a
    /// download; they stop when the value is dropped.
    pub(crate) fn push_watchdog(&self, id: &str) -> Option<impl Send + use<>> {
        self.gate
            .as_ref()
            .map(|gate| download::watchdog(Arc::clone(gate), self.running_flows(), id))
    }

    /// The push flows that are STARTED (after a restart, the pushes to
    /// send again).
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn started_pushes(&self) -> Result<Vec<String>, SignalError> {
        self.flows.started_pushes().map_err(Self::store_failed)
    }

    /// Whether this data plane can read `source`: an `http(s)` URL, or an
    /// `s3://` object when the connector has object storage.
    fn can_read(&self, source: &str) -> bool {
        parse_s3_uri(source).is_none() || self.storage.is_some()
    }

    /// Opens `source`, an upstream URL or an `s3://` object, and returns
    /// its media type and its bytes as a stream (2xx answers only).
    ///
    /// # Errors
    ///
    /// The source did not answer, or not with 2xx; the text names no URL.
    pub async fn open_source(&self, source: &str) -> Result<SourceData, String> {
        match parse_s3_uri(source) {
            Some(object) => {
                let Some(storage) = &self.storage else {
                    return Err(
                        "the source is in object storage, which is not configured".to_owned()
                    );
                };
                let found = storage
                    .get(&object.bucket, &object.key)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(SourceData {
                    content_type: found.content_type,
                    body: Body::new(found.body),
                })
            }
            None => fetch(&self.client, source).await,
        }
    }
}

impl DataPlane for HttpDataPlane {
    fn prepare(&self, _: &DataFlowPrepareMessage) -> Result<DataFlowStatusMessage, SignalError> {
        Err(SignalError::Invalid(
            "prepare is for push transfers and consumer data planes".to_owned(),
        ))
    }

    fn start(&self, message: &DataFlowStartMessage) -> Result<DataFlowStatusMessage, SignalError> {
        let request = &message.flow;
        if request.profile == HTTP_PUSH {
            return self.start_push(message);
        }
        if request.profile != HTTP_PULL {
            return Err(SignalError::Invalid(format!(
                "unsupported profile {}",
                request.profile
            )));
        }
        if message.data_address.is_some() {
            // the spec: a data address on start for a pull "MUST result in a 4xx"
            return Err(SignalError::Invalid(
                "a pull start carries no data address".to_owned(),
            ));
        }
        let source = self.source_in(request)?;
        let token = new_token()?;
        let flow = Flow {
            id: request.data_flow_id.clone(),
            state: DataFlowState::Started,
            dataset_id: request.dataset_id.clone(),
            source: Some(source),
            agreement_id: request.agreement_id.clone(),
            counter_party_id: request.counter_party_id.clone(),
            token_hash: hash(&token),
            expires_at: now_secs().saturating_add(self.token_seconds),
            push_address: None,
        };
        if !self.flows.insert(&flow).map_err(Self::store_failed)? {
            return Err(SignalError::DataFlowExists(flow.id));
        }
        let address = self.address(&flow.id, &token);
        Ok(Self::status_message(&flow.id, flow.state, Some(address)))
    }

    fn started(&self, _: &str, _: &DataFlowStartedNotificationMessage) -> Result<(), SignalError> {
        Err(SignalError::Invalid(
            "started is for consumer data planes".to_owned(),
        ))
    }

    fn suspend(&self, id: &str, _: &DataFlowSuspendMessage) -> Result<(), SignalError> {
        self.stop(id, DataFlowState::Suspended)
    }

    fn resume(
        &self,
        id: &str,
        message: &DataFlowResumeMessage,
    ) -> Result<DataFlowStatusMessage, SignalError> {
        if let Some(address) = &message.data_address {
            // a push started again with a new token (ADR 0027)
            self.replace_push_address(id, address)?;
        }
        // the same token works again (ADR 0013), so no new address
        self.move_to(id, DataFlowState::Started)?;
        Ok(Self::status_message(id, DataFlowState::Started, None))
    }

    fn terminate(&self, id: &str, _: &DataFlowTerminateMessage) -> Result<(), SignalError> {
        self.stop(id, DataFlowState::Terminated)
    }

    fn completed(&self, id: &str) -> Result<(), SignalError> {
        self.move_to(id, DataFlowState::Completed)
    }

    fn status(&self, id: &str) -> Result<DataFlowStatusResponseMessage, SignalError> {
        let flow = self.flow(id)?;
        Ok(DataFlowStatusResponseMessage {
            data_flow_id: flow.id,
            state: flow.state,
        })
    }
}

/// The answer to a refused request for data: 401 with a `Bearer`
/// challenge, the same for every reason.
#[must_use]
pub fn unauthorized() -> Response {
    let challenge = [(header::WWW_AUTHENTICATE, "Bearer")];
    (StatusCode::UNAUTHORIZED, challenge).into_response()
}

/// `GET /data/{flowId}`: checks the token and the consumer's
/// membership, then streams the upstream answer (status 2xx only; its
/// `Content-Type` is kept). The stream stops when the flow is suspended
/// or terminated, or when the membership check fails while it runs.
/// Once the answer has ended, was cut or failed, one audit line says so;
/// a request that is refused (401) writes none.
pub async fn serve(
    State(plane): State<Arc<HttpDataPlane>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = bearer_token_in(&headers) else {
        return unauthorized();
    };
    // watch first, check second: a suspend between the two still cuts
    // this download (`running_flows` module doc)
    let cut = plane.running.watch(&id);
    let Authorized { source, parties } = match plane.authorized(&id, token, now_secs()) {
        Ok(allowed) => allowed,
        Err(Denied::Unauthorized) => return unauthorized(),
        Err(Denied::Store) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if let Some(gate) = &plane.gate
        && !member_may_download(gate.as_ref(), &id).await
    {
        return unauthorized();
    }
    match plane.open_source(&source).await {
        Ok(data) => {
            let mut response = Response::new(data.body);
            if let Some(value) = data.content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            // the bytes are counted as they go to the consumer; the cut
            // wraps the count, so a cut download is audited as `cut`
            let response = response.map(|body| Body::new(AuditedBody::new(body, &id, parties)));
            let watchdog = plane
                .gate
                .as_ref()
                .map(|gate| download::watchdog(Arc::clone(gate), plane.running_flows(), &id));
            download::cut_when_told(response, cut, watchdog)
        }
        Err(error) => {
            // the source is private: the log names the flow, not the URL
            tracing::warn!(flow = %id, %error, "upstream fetch failed");
            let (agreement, consumer) = (&parties.agreement, &parties.counter_party);
            transfer_done(
                &id,
                agreement,
                consumer,
                Direction::Sent,
                0,
                Outcome::Failed,
            );
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Whether the consumer of flow `id` is still a member ([`AccessGate`]).
/// When it is not, its transfer is ended too; when the check could not
/// finish, the request is refused and the transfer kept. The reason goes
/// to the log, never to the caller.
async fn member_may_download(gate: &dyn AccessGate, id: &str) -> bool {
    match gate.check(id).await {
        Access::Granted => true,
        Access::Unchecked(reason) => {
            tracing::warn!(flow = %id, %reason, "request for data refused: the consumer's membership could not be checked now");
            false
        }
        Access::Revoked(reason) => {
            gate.end_transfer(id, &reason);
            false
        }
    }
}

/// GETs `url` and returns a 2xx answer's media type and body, streamed.
/// The data source is our participant's own, so the request carries our
/// trace context (ADR 0023, section 4). Its span ends when the answer's
/// headers came; the body is streamed afterwards.
async fn fetch(client: &HttpsClient, url: &str) -> Result<SourceData, String> {
    let mut request = hyper::Request::get(url)
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let mut call = ClientCall::start(&mut request, Peer::Own);
    let sent = client.request(request).instrument(call.span().clone());
    // two results in one: the outer `Ok`/`Err` is the timeout's (did the
    // answer come in time?), the inner one the request's own (did it
    // work?)
    let answer = match tokio::time::timeout(UPSTREAM_TIMEOUT, sent).await {
        Ok(Ok(answer)) => {
            call.answered(answer.status().as_u16());
            call.end();
            answer
        }
        Ok(Err(error)) => {
            call.end();
            return Err(error.to_string());
        }
        Err(_) => {
            call.end_timed_out();
            return Err(format!("no answer within {}s", UPSTREAM_TIMEOUT.as_secs()));
        }
    };
    if !answer.status().is_success() {
        return Err(format!("upstream answered HTTP {}", answer.status()));
    }
    let content_type = answer.headers().get(header::CONTENT_TYPE).cloned();
    Ok(SourceData {
        content_type,
        body: Body::new(answer.into_body()),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dps::message::{AddressMessage, DataFlowBaseMessage, ReasonMessage};

    use super::*;

    fn plane() -> HttpDataPlane {
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        HttpDataPlane::new(flows, "https://provider.example/", 60).unwrap()
    }

    /// The `metadata` of a start message that names `source`.
    fn naming(source: &str) -> serde_json::Map<String, serde_json::Value> {
        let mut metadata = serde_json::Map::new();
        metadata.insert(SOURCE_METADATA_KEY.to_owned(), source.into());
        metadata
    }

    /// A start message for `dataset`, as the control plane sends it: only
    /// dataset `d1` has a source, `http://upstream.example/d1`.
    fn start(dataset: &str, profile: &str) -> DataFlowStartMessage {
        let metadata = if dataset == "d1" {
            Some(naming("http://upstream.example/d1"))
        } else {
            None
        };
        DataFlowStartMessage {
            flow: DataFlowBaseMessage {
                message_id: "m".to_owned(),
                participant_id: "did:web:p".to_owned(),
                counter_party_id: "did:web:c".to_owned(),
                dataspace_context: "default".to_owned(),
                data_flow_id: "f1".to_owned(),
                agreement_id: "a".to_owned(),
                dataset_id: dataset.to_owned(),
                profile: profile.to_owned(),
                claims: serde_json::Map::new(),
                metadata,
                labels: vec![],
            },
            data_address: None,
        }
    }

    fn token_of(status: &DataFlowStatusMessage) -> String {
        let address = status.data_address.as_ref().unwrap();
        address.property("authorization").unwrap().to_owned()
    }

    fn reason() -> ReasonMessage {
        ReasonMessage {
            message_id: "m".to_owned(),
            reason: None,
        }
    }

    #[test]
    fn a_new_address_replaces_the_token() {
        let p = plane();
        let old = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        p.suspend("f1", &reason()).unwrap();
        let address = p.new_address("f1").unwrap();
        let new = address.property("authorization").unwrap().to_owned();
        assert_ne!(new, old);
        let resume = AddressMessage {
            message_id: "m".to_owned(),
            data_address: None,
        };
        p.resume("f1", &resume).unwrap();
        let now = now_secs();
        assert!(p.authorize("f1", &new, now).is_ok());
        assert_eq!(p.authorize("f1", &old, now), Err(Denied::Unauthorized));

        p.completed("f1").unwrap();
        assert!(p.new_address("f1").is_err(), "an ended flow gets no token");
        assert!(p.new_address("unknown").is_err());
    }

    #[test]
    fn start_gives_an_address_with_a_token_that_only_works_while_started() {
        let p = plane();
        let status = p.start(&start("d1", HTTP_PULL)).unwrap();
        assert_eq!(status.state, DataFlowState::Started);
        let address = status.data_address.clone().unwrap();
        assert_eq!(
            address.endpoint.as_deref(),
            Some("https://provider.example/data/f1")
        );
        assert_eq!(address.endpoint_type, HTTP_ENDPOINT_TYPE);
        assert_eq!(address.property("authType"), Some("bearer"));
        let token = token_of(&status);
        assert!(token.len() >= 43, "32 random bytes");

        let now = now_secs();
        assert_eq!(
            p.authorize("f1", &token, now).unwrap(),
            "http://upstream.example/d1"
        );
        // the token is not stored, only its hash
        let stored = p.flows.get("f1").unwrap().unwrap();
        assert_ne!(stored.token_hash, token);
        assert_eq!(p.authorize("f1", "wrong", now), Err(Denied::Unauthorized));
        assert_eq!(p.authorize("f2", &token, now), Err(Denied::Unauthorized));
        assert_eq!(
            p.authorize("f1", &token, now + 61),
            Err(Denied::Unauthorized),
            "expired"
        );

        p.suspend("f1", &reason()).unwrap();
        assert_eq!(p.authorize("f1", &token, now), Err(Denied::Unauthorized));
        let resume = AddressMessage {
            message_id: "m".to_owned(),
            data_address: None,
        };
        let resumed = p.resume("f1", &resume).unwrap();
        assert!(resumed.data_address.is_none(), "same token");
        assert!(p.authorize("f1", &token, now).is_ok());
        p.completed("f1").unwrap();
        assert_eq!(p.authorize("f1", &token, now), Err(Denied::Unauthorized));
        assert_eq!(p.status("f1").unwrap().state, DataFlowState::Completed);
        assert!(matches!(
            p.terminate("f1", &reason()),
            Err(SignalError::NotAllowed(_))
        ));
    }

    #[test]
    fn start_refuses_what_it_cannot_serve() {
        let p = plane();
        let refused = |m: &DataFlowStartMessage| matches!(p.start(m), Err(SignalError::Invalid(_)));
        assert!(refused(&start(
            "d1",
            "https://w3id.org/dspace-sig/profile/http-push"
        )));
        assert!(refused(&start("no-source", HTTP_PULL)));
        let mut with_address = start("d1", HTTP_PULL);
        with_address.data_address = Some(
            serde_json::from_value(serde_json::json!(
                {"@type": "DataAddress", "endpointType": HTTP_ENDPOINT_TYPE}
            ))
            .unwrap(),
        );
        assert!(refused(&with_address));
        p.start(&start("d1", HTTP_PULL)).unwrap();
        assert!(matches!(
            p.start(&start("d1", HTTP_PULL)),
            Err(SignalError::DataFlowExists(_))
        ));
        assert!(matches!(
            p.suspend("nope", &reason()),
            Err(SignalError::UnknownDataFlow(_))
        ));
    }

    /// From 1.2 the source comes with the start message (ADR 0028, step
    /// 1): the flow keeps it and serves from it.
    #[test]
    fn the_flow_serves_the_source_its_start_message_named() {
        let p = plane();
        let mut message = start("d1", HTTP_PULL);
        message.flow.metadata = Some(naming("https://elsewhere.example/the-data"));
        let token = token_of(&p.start(&message).unwrap());
        let stored = p.flows.get("f1").unwrap().unwrap();
        let source = Some("https://elsewhere.example/the-data".to_owned());
        assert_eq!(stored.source, source);
        assert_eq!(p.authorize("f1", &token, now_secs()).ok(), source);
    }

    /// A start whose metadata names no source this data plane can read is
    /// refused, and no flow is kept.
    #[test]
    fn a_start_without_a_readable_source_is_refused() {
        let p = plane();
        let mut no_key = start("d1", HTTP_PULL);
        no_key.flow.metadata = Some(serde_json::Map::new());
        let mut not_text = start("d1", HTTP_PULL);
        not_text.flow.metadata = Some(serde_json::Map::from_iter([(
            SOURCE_METADATA_KEY.to_owned(),
            serde_json::json!(42),
        )]));
        let mut file = start("d1", HTTP_PULL);
        file.flow.metadata = Some(naming("file:///etc/passwd"));
        let mut no_storage = start("d1", HTTP_PULL);
        no_storage.flow.metadata = Some(naming("s3://datasets/d1.csv"));
        let mut push_without = start("d1", HTTP_PUSH);
        push_without.flow.metadata = None;
        push_without.data_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        for message in [no_key, not_text, file, no_storage, push_without] {
            let refused = p.start(&message);
            assert!(
                matches!(refused, Err(SignalError::Invalid(_))),
                "{refused:?}"
            );
        }
        assert_eq!(p.flows.get("f1").unwrap(), None);
    }

    /// A flow kept by 1.1 whose source the migration could not fill in
    /// (`Flow::source`) is never served, and its push never sent.
    #[test]
    fn a_flow_without_a_source_is_not_served() {
        let p = plane();
        let token = new_token().unwrap();
        let mut flow = Flow {
            id: "old".to_owned(),
            state: DataFlowState::Started,
            dataset_id: "gone".to_owned(),
            source: None,
            agreement_id: String::new(),
            counter_party_id: String::new(),
            token_hash: hash(&token),
            expires_at: now_secs() + 60,
            push_address: None,
        };
        p.flows.insert(&flow).unwrap();
        assert_eq!(
            p.authorize("old", &token, now_secs()),
            Err(Denied::Unauthorized)
        );
        flow.id = "old-push".to_owned();
        flow.push_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        p.flows.insert(&flow).unwrap();
        assert!(matches!(
            p.push_job("old-push"),
            Err(SignalError::Invalid(_))
        ));
    }

    /// The flow keeps the agreement and the consumer its start message
    /// names, for the audit line of each pull and push.
    #[test]
    fn the_flow_keeps_the_parties_of_its_start_message() {
        let p = plane();
        let token = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        let allowed = p.authorized("f1", &token, now_secs()).unwrap();
        let parties = Parties {
            agreement: "a".to_owned(),
            counter_party: "did:web:c".to_owned(),
        };
        assert_eq!(allowed.parties, parties);

        let mut push = start("d1", HTTP_PUSH);
        push.flow.data_flow_id = "f2".to_owned();
        push.data_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        p.start(&push).unwrap();
        let job = p.push_job("f2").unwrap().unwrap();
        assert_eq!(job.parties, parties);
        assert_eq!(job.source, "http://upstream.example/d1");
    }

    #[test]
    fn tokens_differ_per_flow() {
        let p = plane();
        let a = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        let mut second = start("d1", HTTP_PULL);
        second.flow.data_flow_id = "f2".to_owned();
        let b = token_of(&p.start(&second).unwrap());
        assert_ne!(a, b);
        assert_eq!(p.authorize("f2", &a, now_secs()), Err(Denied::Unauthorized));
    }

    /// The data source is our own: the fetch carries our trace context
    /// (ADR 0023, section 4).
    #[tokio::test]
    async fn the_fetch_from_our_data_source_carries_trace_context() {
        use portitor_runtime::telemetry::testing::{
            recorded, trace_on_this_thread, traceparent_recorder,
        };
        let traced = trace_on_this_thread();
        let (base, recorder) = traceparent_recorder().await;
        let (client, _) = https_client(true).unwrap();
        let span = tracing::info_span!(target: "portitor_test", "request");
        let response = fetch(&client, &format!("{base}/d1")).instrument(span).await;
        assert!(response.is_ok());
        let got = recorded(&recorder);
        assert_eq!(got.len(), 1);
        assert!(
            got[0].as_deref().is_some_and(|t| t.starts_with("00-")),
            "{got:?}"
        );
        assert!(traced.spans().iter().any(|span| span.name == "GET"));
    }
}
