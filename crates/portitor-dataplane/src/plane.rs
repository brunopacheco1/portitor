// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! [`HttpDataPlane`]: what the built-in data plane holds (its flow store,
//! its HTTP clients, the object storage, the downloads and pushes that
//! run, the membership gate) and how the connector builds it. What it
//! does is in the other modules: `signaling.rs` (the Data Plane
//! Signaling port), `serving.rs` (`GET /data/{flowId}`), `sources.rs`
//! (reading a source) and `push.rs` (sending a push).
//!
//! Rust lets a type's methods be split over several `impl HttpDataPlane`
//! blocks, in different files of the same crate. This file holds the one
//! that builds the data plane; each of those four files holds one more
//! with the methods of its task, and `signaling.rs` also holds
//! `impl DataPlane for HttpDataPlane`.

use std::sync::Arc;

use portitor_dps::port::SignalError;
use portitor_runtime::http::{
    HttpsClient, StreamingHttpsClient, https_client, https_streaming_client,
};

use crate::access::AccessGate;
use crate::download::{self, AbortOnDrop};
use crate::flows::{Flow, FlowStore, FlowStoreError};
use crate::object_storage::ObjectStorage;
use crate::running_flows::RunningFlows;

/// The built-in HTTP data plane.
#[derive(Clone)]
pub struct HttpDataPlane {
    /// Where the flows are kept.
    pub(crate) flows: Arc<dyn FlowStore + Send + Sync>,
    /// The connector's public URL, without a final `/`: the data
    /// addresses of pull flows start with it.
    pub(crate) public_url: String,
    /// How long a pull flow's token works, in seconds.
    pub(crate) token_seconds: u64,
    /// The client of upstream URLs (`sources.rs`).
    pub(crate) source_client: HttpsClient,
    /// The object storage of `s3://` sources, if the connector has one.
    pub(crate) storage: Option<Arc<ObjectStorage>>,
    /// The client of pushes: to an address a partner chose, so plain
    /// `http://` only with `PORTITOR_ALLOW_HTTP`.
    pub(crate) push_client: StreamingHttpsClient,
    /// The downloads that run now, by flow: cut on suspend and terminate.
    pub(crate) running: Arc<RunningFlows>,
    /// Checks the consumer's membership again; `None` checks the token
    /// only, as 1.0 did (unit tests).
    pub(crate) gate: Option<Arc<dyn AccessGate>>,
}

impl std::fmt::Debug for HttpDataPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDataPlane")
            .field("public_url", &self.public_url)
            .field("token_seconds", &self.token_seconds)
            .finish_non_exhaustive()
    }
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
        let (source_client, _) = https_client(true)?;
        Ok(Self {
            flows,
            public_url: public_url.trim_end_matches('/').to_owned(),
            token_seconds,
            source_client,
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

    /// The checks that run next to a download or push of flow `id`
    /// (`download.rs`); they stop when the values are dropped: the reads
    /// of the flow's state (TD-52), and the membership checks when there
    /// is a gate.
    pub(crate) fn watchdogs(&self, id: &str) -> Vec<AbortOnDrop> {
        let mut watchdogs = vec![download::state_watchdog(
            Arc::clone(&self.flows),
            self.running_flows(),
            id,
        )];
        // no gate: nothing to check again, as in 1.0 (unit tests)
        if let Some(gate) = &self.gate {
            watchdogs.push(download::membership_watchdog(
                Arc::clone(gate),
                self.running_flows(),
                id,
            ));
        }
        watchdogs
    }

    /// A flow store failure as the answer to a signal.
    #[allow(clippy::needless_pass_by_value)] // used as map_err(Self::store_failed)
    pub(crate) fn store_failed(e: FlowStoreError) -> SignalError {
        SignalError::Failed(format!("flow store: {}", e.0))
    }

    /// Flow `id`; unknown is an error.
    pub(crate) fn flow(&self, id: &str) -> Result<Flow, SignalError> {
        self.flows
            .get(id)
            .map_err(Self::store_failed)?
            .ok_or_else(|| SignalError::UnknownDataFlow(id.to_owned()))
    }
}
