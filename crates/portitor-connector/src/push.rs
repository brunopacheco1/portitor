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
//! - Suspend or terminate cut a push that is running: the data plane cuts
//!   the flow ([`crate::running_flows::RunningFlows::cut`]), the push's
//!   [`crate::running_flows::CutSignal`] fires, the request is dropped,
//!   and the transfer is left as the suspension or termination made it.
//! - The consumer's membership is checked before the push and again
//!   while it runs, as for a download (ADR 0027, section 7.2): a revoked
//!   membership ends the transfer; a check that cannot finish fails the
//!   push, which terminates the transfer.
//! - Each push writes the audit line `data transfer done`
//!   ([`crate::audit`]).
//!
//! The endpoint is an address the partner chose: calling it carries the
//! same risk as calling a callback address (R-17 in
//! `docs/RISKS_AND_TECHNICAL_DEBTS.md`); it must be
//! `https://` unless `PORTITOR_ALLOW_HTTP` is `true`, checked when the
//! transfer request arrives. No trace context is sent to it (ADR 0023).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::header;
use http_body_util::BodyExt as _;
use portitor_runtime::telemetry::http::{ClientCall, Peer};
use tracing::Instrument as _;

use portitor_dsp::transfer::DataAddress;

use crate::audit::{self, Outcome};
use crate::data_plane::{HttpDataPlane, PushJob};

/// The media type sent when neither the source nor the consumer's address
/// names one.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

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
        result = post(plane, flow_id, source, address, Arc::clone(&counted)) => match result {
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

/// Reads the source and `POST`s it to the consumer's endpoint; `counted`
/// grows with each chunk of data sent.
async fn post(
    plane: &HttpDataPlane,
    flow_id: &str,
    source: &str,
    address: &DataAddress,
    counted: Arc<AtomicU64>,
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
    let body = data.body.map_frame(move |frame| {
        if let Some(bytes) = frame.data_ref() {
            counted.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
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
    let Ok(answer) = plane.push_client().request(request).instrument(span).await else {
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
        pushed.bytes,
        pushed.outcome(),
    );
}
