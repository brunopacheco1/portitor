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

use portitor_dsp::transfer::DataAddress;

use crate::audit::{self, Outcome};
use crate::{HttpDataPlane, PushJob};

/// The media type sent when neither the source nor the consumer's address
/// names one.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// The longest a push may go without progress: no piece of the body
/// written, and no answer. The same two minutes as the longest request to
/// the object storage and a bundle download. The consumer's receiving
/// route uses it too, for a body that stops coming.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How a push ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// The consumer answered 2xx.
    Done,
    /// It failed; the reason names no URL and no token.
    Failed(String),
    /// Suspend or terminate cut it.
    Cancelled,
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
            Sent::Cancelled => Outcome::Cut,
        }
    }
}

/// Sends `job` with `plane`'s client, until it is done or its flow is
/// cut.
pub async fn send(plane: &HttpDataPlane, job: &mut PushJob) -> Pushed {
    let mut pushed = send_until_cut(plane, job, IDLE_TIMEOUT).await;
    if pushed.sent == Sent::Cancelled && plane.is_started(&job.flow_id) {
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
/// gives [`Sent::Cancelled`].
async fn send_until_cut(plane: &HttpDataPlane, job: &mut PushJob, idle: Duration) -> Pushed {
    if job.cut.is_cut() {
        // cut before it began, while it waited for the consumer
        return Pushed {
            sent: Sent::Cancelled,
            bytes: 0,
        };
    }
    if let Err(outcome) = plane.push_membership(&job.flow_id).await {
        let sent = if outcome == Outcome::Cut {
            // the membership was revoked: the transfer is being ended
            Sent::Cancelled
        } else {
            Sent::Failed("the consumer's membership could not be checked".to_owned())
        };
        return Pushed { sent, bytes: 0 };
    }
    // checks the membership again while the push runs
    let _watchdog = plane.push_watchdog(&job.flow_id);
    let counted = Arc::new(AtomicU64::new(0));
    let PushJob {
        flow_id,
        source,
        address,
        cut,
    } = job;
    let sent = tokio::select! {
        result = post(plane, flow_id, source, address, Arc::clone(&counted), idle) => match result {
            Ok(()) => Sent::Done,
            Err(reason) => Sent::Failed(reason),
        },
        () = cut.cut() => Sent::Cancelled,
    };
    Pushed {
        sent,
        bytes: counted.load(Ordering::Relaxed),
    }
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
    let Ok(answer) = answer else {
        call.end();
        return Err("the data could not be sent to the consumer's endpoint".to_owned());
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

/// Writes the audit line of a push sent for transfer `flow` under
/// `agreement` to `counter_party`.
pub fn audit(flow: &str, agreement: &str, counter_party: &str, pushed: &Pushed) {
    audit::transfer_done(
        flow,
        agreement,
        counter_party,
        audit::Direction::Sent,
        pushed.bytes,
        pushed.outcome(),
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeMap;

    use portitor_dsp::transfer::{DataAddressType, HTTP_PUSH_ENDPOINT_TYPE};

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
        let sources = BTreeMap::from([("d1".to_owned(), source.clone())]);
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        let plane = HttpDataPlane::new(flows, Arc::new(sources), "https://provider.example", 60)
            .unwrap()
            .with_push_over_http(true)
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
}
