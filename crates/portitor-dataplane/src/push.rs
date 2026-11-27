// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The push sender (ADR 0027, section 1): once a push transfer is
//! `STARTED`, the provider's data plane reads the dataset's source and
//! sends it as the body of one `POST` to the consumer's endpoint, with
//! `Authorization: Bearer <the consumer's token>` and the source's
//! `Content-Type`. The body is streamed, never held whole.
//!
//! - A 2xx answer completes the transfer; any other answer, or none,
//!   terminates it with the reason (the status code only: the URL and the
//!   token are never logged or sent back).
//! - The connection must open within [`CONNECT_TIMEOUT`] (a setting of
//!   the push client), and the push must make progress at least every
//!   [`IDLE_TIMEOUT`]: a piece of the body written, or the answer's
//!   headers. Otherwise it fails, which terminates the transfer.
//! - Suspend or terminate cut a push that is running: the data plane cuts
//!   the flow ([`crate::running_flows::RunningFlows::cut`]), the push's
//!   [`crate::running_flows::CutSignal`] fires, the request is dropped,
//!   and the transfer is left as the suspension or termination made it.
//!   When another process that shares the database handled the
//!   suspension or termination, the push reads it from the flow's state
//!   within 5 seconds and is cut the same way (TD-52).
//! - The consumer's membership is checked before the push and again
//!   while it runs, as for a download (ADR 0027, section 7.2): a revoked
//!   membership ends the transfer; a check that cannot finish fails the
//!   push, which terminates the transfer. Any cut that leaves the flow
//!   `STARTED` is such a failure: nothing else would end the transfer.
//! - Each push writes the audit line `data transfer done`
//!   ([`crate::audit`]).
//!
//! The endpoint is an address the partner chose: calling it carries the
//! same risk as calling a callback address (R-17 in
//! `docs/RISKS_AND_TECHNICAL_DEBTS.md`); it must be
//! `https://` unless `PORTITOR_ALLOW_HTTP` is `true`, checked when the
//! transfer request arrives. No trace context is sent to it (ADR 0023).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::http::header;
use http_body_util::BodyExt as _;
pub use portitor_runtime::http::CONNECT_TIMEOUT;
use portitor_runtime::telemetry::http::{ClientCall, Peer};
use tokio::time::Instant;
use tracing::Instrument as _;

use portitor_dps::port::SignalError;
use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::DataAddress;

use crate::access::Access;
use crate::audit::{self, Outcome, Parties};
use crate::plane::HttpDataPlane;
use crate::running_flows::CutSignal;

/// The media type sent when neither the source nor the consumer's address
/// names one.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// The longest a push may go without progress: no piece of the body
/// written, and no answer. The same two minutes as the longest request to
/// the object storage and a bundle download. The consumer's receiving
/// route uses it too, for a body that stops coming.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

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

impl HttpDataPlane {
    /// The push to send now for flow `id`, if it is a push flow that is
    /// STARTED. The caller sends it ([`send`]) and keeps the job until it
    /// is done: its cut signal stops it on a suspension or termination.
    ///
    /// One push per flow: the caller makes sure of it, among all the
    /// processes that share the database, with a lease taken before this
    /// call (the connector's push lease, TD-54). In this process, a flow
    /// whose push still runs gives `None` too.
    ///
    /// # Errors
    ///
    /// The store failed.
    pub fn push_job(&self, id: &str) -> Result<Option<PushJob>, SignalError> {
        // watch first, check second, as a download does
        // (`running_flows` module doc)
        let Some(cut) = self.running.watch_if_idle(id) else {
            // already being sent here
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
    /// section 7.2). A revoked membership also ends the transfer.
    pub(crate) async fn check_membership_before_push(&self, id: &str) -> BeforePush {
        let Some(gate) = &self.gate else {
            // no gate: nothing to check, as in 1.0 (unit tests)
            return BeforePush::Go;
        };
        match gate.check(id).await {
            Access::Granted => BeforePush::Go,
            Access::Unchecked(reason) => {
                tracing::warn!(flow = %id, %reason, "push not sent: the consumer's membership could not be checked now");
                BeforePush::Unchecked
            }
            Access::Revoked(reason) => {
                gate.end_transfer(id, &reason);
                BeforePush::Revoked
            }
        }
    }

    /// Whether flow `id` is still STARTED. A push that was cut while its
    /// flow is still STARTED was not cut by a suspension or a termination
    /// (they change the state first, then cut): see the module doc.
    pub(crate) fn is_started(&self, id: &str) -> bool {
        matches!(self.flows.get(id), Ok(Some(flow)) if flow.state == DataFlowState::Started)
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
}

/// What [`HttpDataPlane::check_membership_before_push`] allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BeforePush {
    /// The consumer is still a member (or there is no gate): send.
    Go,
    /// The membership was revoked: do not send; the transfer is being
    /// ended.
    Revoked,
    /// The check could not finish now: do not send; the push fails.
    Unchecked,
}

/// How a push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// The consumer answered 2xx.
    Done,
    /// It failed; the reason names no URL and no token.
    Failed(String),
    /// Suspend, terminate or a revoked membership cut it: the audit
    /// line's outcome `cut` ([`Outcome::Cut`]).
    Cut,
}

/// How a push ended, and how many bytes of data went out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// How it ended.
    pub sent: Sent,
    /// Bytes of data read from the source and handed to the connection.
    pub bytes: u64,
}

impl Pushed {
    /// The `outcome` of the audit line.
    #[must_use]
    pub fn outcome(&self) -> Outcome {
        match self.sent {
            Sent::Done => Outcome::Complete,
            Sent::Failed(_) => Outcome::Failed,
            Sent::Cut => Outcome::Cut,
        }
    }
}

/// Sends `job` with `plane`'s push client, until it is done or its flow is
/// cut.
pub async fn send(plane: &HttpDataPlane, job: &mut PushJob) -> Pushed {
    send_unless(plane, job, std::future::pending()).await
}

/// [`send`], but it also stops, as [`Sent::Cut`], when `stop` finishes:
/// the connector's push lease was lost, and another process sends the
/// data now (TD-54). The flow is left as it is then: that process
/// completes or terminates the transfer.
pub async fn send_unless(
    plane: &HttpDataPlane,
    job: &mut PushJob,
    stop: impl Future<Output = ()>,
) -> Pushed {
    let (mut pushed, stopped) = send_until_cut(plane, job, IDLE_TIMEOUT, stop).await;
    if pushed.sent == Sent::Cut && !stopped && plane.is_started(&job.flow_id) {
        // cut, but the flow is still STARTED: no suspension or termination
        // cut it (they change the state first), but a membership check
        // that could not finish. Nothing would send the data now, so the
        // push failed, and the transfer ends.
        let reason = "the push was cut: the consumer's membership could not be checked";
        pushed.sent = Sent::Failed(reason.to_owned());
    }
    pushed
}

/// [`send`], with `idle` as the longest time without progress. A cut
/// gives [`Sent::Cut`]; so does `stop`, and then the second value is
/// true.
async fn send_until_cut(
    plane: &HttpDataPlane,
    job: &mut PushJob,
    idle: Duration,
    stop: impl Future<Output = ()>,
) -> (Pushed, bool) {
    let not_sent = |sent| Pushed { sent, bytes: 0 };
    if job.cut.is_cut() {
        // cut before it began, while it waited for the consumer
        return (not_sent(Sent::Cut), false);
    }
    match plane.check_membership_before_push(&job.flow_id).await {
        BeforePush::Go => {}
        // the transfer is being ended
        BeforePush::Revoked => return (not_sent(Sent::Cut), false),
        BeforePush::Unchecked => {
            let reason = "the consumer's membership could not be checked";
            return (not_sent(Sent::Failed(reason.to_owned())), false);
        }
    }
    // reads the flow's state (TD-52) and checks the membership again
    // while the push runs
    let _watchdogs = plane.watchdogs(&job.flow_id);
    let counted = Arc::new(AtomicU64::new(0));
    // borrows of the job's fields: the post reads three of them while the
    // cut signal is watched (it needs `&mut`). The job's parties are for
    // the audit line, which the caller writes once the push is over.
    let flow_id = &job.flow_id;
    let source = &job.source;
    let address = &job.address;
    let cut = &mut job.cut;
    // what ended it; the post is dropped unless it finished
    let ended = tokio::select! {
        result = post(plane, flow_id, source, address, Arc::clone(&counted), idle) => Ended::Posted(result),
        () = cut.cut() => Ended::Cut,
        () = stop => Ended::Stopped,
    };
    let sent = match &ended {
        Ended::Posted(Ok(())) => Sent::Done,
        Ended::Posted(Err(reason)) => Sent::Failed(reason.clone()),
        Ended::Cut | Ended::Stopped => Sent::Cut,
    };
    let pushed = Pushed {
        sent,
        bytes: counted.load(Ordering::Relaxed),
    };
    (pushed, matches!(ended, Ended::Stopped))
}

/// What ended [`send_until_cut`]'s post.
enum Ended {
    /// It finished: the consumer answered, or it failed.
    Posted(Result<(), String>),
    /// The flow was cut.
    Cut,
    /// The caller's `stop` finished.
    Stopped,
}

/// When a push last made progress.
#[derive(Clone)]
struct LastProgress(Arc<Mutex<Instant>>);

impl LastProgress {
    /// Progress now.
    fn now() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }

    /// Notes progress now.
    fn note(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Finishes once there has been no progress for `limit`.
    async fn none_for(&self, limit: Duration) {
        loop {
            let last = *self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let deadline = last + limit;
            if Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep_until(deadline).await;
        }
    }
}

/// Reads the source and `POST`s it to the consumer's endpoint; `counted`
/// grows with each chunk of data sent. Fails after `idle` without
/// progress (module doc).
async fn post(
    plane: &HttpDataPlane,
    flow_id: &str,
    source: &str,
    address: &DataAddress,
    counted: Arc<AtomicU64>,
    idle: Duration,
) -> Result<(), String> {
    let data = match plane.open_source(source).await {
        Ok(data) => data,
        Err(error) => {
            // the source is private: the log names the flow, not the URL
            tracing::warn!(flow = %flow_id, %error, "push: the source could not be read");
            return Err("the source could not be read".to_owned());
        }
    };
    let Some(endpoint) = address.endpoint.as_deref() else {
        return Err("the dataAddress has no endpoint".to_owned());
    };
    let content_type = match (&data.content_type, address.property("contentType")) {
        (Some(value), _) => value.clone(),
        (None, Some(asked)) => {
            header::HeaderValue::from_str(asked).map_err(|_| "contentType is not a header value")?
        }
        (None, None) => header::HeaderValue::from_static(DEFAULT_CONTENT_TYPE),
    };
    let progress = LastProgress::now();
    let written = progress.clone();
    let body = data.body.map_frame(move |frame| {
        if let Some(bytes) = frame.data_ref() {
            counted.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        // the connection asks for the next piece once it took this one
        written.note();
        frame
    });
    let mut builder = hyper::Request::post(endpoint).header(header::CONTENT_TYPE, content_type);
    if let Some(token) = address.property("authorization") {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let mut request = builder
        .body(Body::new(body))
        .map_err(|_| "the dataAddress cannot be used for a request".to_owned())?;
    // another participant's endpoint: no trace context (ADR 0023)
    let mut call = ClientCall::start(&mut request, Peer::Other);
    let span = call.span().clone();
    let sending = plane.push_client().request(request).instrument(span);
    let answer = tokio::select! {
        answer = sending => answer,
        () = progress.none_for(idle) => {
            call.end();
            return Err(format!(
                "the consumer's endpoint took no data and gave no answer for {} s",
                idle.as_secs()
            ));
        }
    };
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            call.end();
            // an internal address the block refused (R-17): say which
            // setting allows it; any other failure keeps its short text
            if let Some(refused) = portitor_runtime::http::connect::refusal_in(&error) {
                return Err(format!("the consumer's endpoint is refused: {refused}"));
            }
            return Err("the data could not be sent to the consumer's endpoint".to_owned());
        }
    };
    let status = answer.status();
    call.answered(status.as_u16());
    call.end();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("the consumer answered HTTP {}", status.as_u16()))
    }
}

/// Writes the audit line of a push sent for transfer `flow` to
/// `parties` ([`PushJob::parties`]).
pub fn audit(flow: &str, parties: &Parties, pushed: &Pushed) {
    audit::transfer_done(
        flow,
        &parties.agreement_id,
        &parties.counter_party_id,
        audit::Direction::Sent,
        pushed.bytes,
        pushed.outcome(),
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dsp::transfer::{DataAddressType, HTTP_PUSH_ENDPOINT_TYPE};
    use portitor_runtime::internal_addresses::InternalAddresses;

    use super::*;

    /// A local server that takes connections and never answers; its URL.
    async fn silent_endpoint() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/in", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut open = Vec::new();
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                open.push(socket);
            }
        });
        url
    }

    /// A local server with a small dataset; its URL.
    async fn source() -> String {
        let app =
            axum::Router::new().route("/d1", axum::routing::get(|| async { "day,celsius\n" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/d1", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    /// M2 of the 1.1 review: an endpoint that takes the connection and
    /// then neither reads nor answers must not hold the push forever.
    #[tokio::test]
    async fn a_push_without_progress_fails() {
        let source = source().await;
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        let plane = HttpDataPlane::new(flows, "https://provider.example", 60)
            .unwrap()
            .with_push_client(true, &InternalAddresses::default())
            .unwrap();
        let address = DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some(silent_endpoint().await),
            endpoint_properties: Vec::new(),
        };
        let counted = Arc::new(AtomicU64::new(0));
        let idle = Duration::from_millis(200);
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            post(&plane, "f1", &source, &address, counted, idle),
        )
        .await
        .unwrap();
        let reason = sent.unwrap_err();
        assert!(reason.contains("no answer"), "{reason}");
    }

    /// R-17: with the block on internal addresses, a push to an endpoint
    /// on one is refused before any data goes out, with the setting that
    /// would allow it. The source, which the operator chose, is on one
    /// too and is still read.
    #[tokio::test]
    async fn a_push_to_an_internal_address_is_refused_with_the_block_on() {
        use portitor_runtime::internal_addresses::{ALLOWED_SETTING, BLOCK_SETTING};
        let source = source().await;
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        let blocking = InternalAddresses::from_vars(|name: &str| {
            (name == BLOCK_SETTING).then(|| std::ffi::OsString::from("true"))
        })
        .unwrap();
        let plane = HttpDataPlane::new(flows, "https://provider.example", 60)
            .unwrap()
            .with_push_client(true, &blocking)
            .unwrap();
        let address = DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some(silent_endpoint().await),
            endpoint_properties: Vec::new(),
        };
        let counted = Arc::new(AtomicU64::new(0));
        let idle = Duration::from_millis(200);
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            post(&plane, "f1", &source, &address, Arc::clone(&counted), idle),
        )
        .await
        .unwrap();
        let reason = sent.unwrap_err();
        assert!(reason.contains(ALLOWED_SETTING), "{reason}");
        assert_eq!(counted.load(Ordering::Relaxed), 0, "no data went out");
    }

    /// A push whose connector lease is lost stops as cut, and leaves its
    /// flow STARTED for the process that took the push over: not failed,
    /// which would end the transfer. While it runs, this process starts
    /// no second push of the flow.
    #[tokio::test]
    async fn a_stopped_push_is_cut_and_leaves_the_flow() {
        use crate::flows::{Flow, FlowStore as _};

        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        let flow = Flow {
            id: "f1".to_owned(),
            state: DataFlowState::Started,
            dataset_id: "d1".to_owned(),
            source: Some(source().await),
            agreement_id: String::new(),
            counter_party_id: String::new(),
            token_hash: String::new(),
            expires_at: u64::MAX / 2,
            push_address: Some(DataAddress {
                kind: DataAddressType,
                endpoint_type: HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
                endpoint: Some(silent_endpoint().await),
                endpoint_properties: Vec::new(),
            }),
        };
        flows.insert(&flow).unwrap();
        let plane = HttpDataPlane::new(flows, "https://provider.example", 60)
            .unwrap()
            .with_push_client(true, &InternalAddresses::default())
            .unwrap();
        let mut job = plane.push_job("f1").unwrap().unwrap();
        assert!(plane.push_job("f1").unwrap().is_none(), "one push here");
        let stop = tokio::time::sleep(Duration::from_millis(100));
        let pushed =
            tokio::time::timeout(Duration::from_secs(5), send_unless(&plane, &mut job, stop))
                .await
                .unwrap();
        assert_eq!(pushed.sent, Sent::Cut);
        assert!(plane.is_started("f1"));
        drop(job);
        assert!(plane.push_job("f1").unwrap().is_some(), "it ended");
    }
}
