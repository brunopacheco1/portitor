// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A download that can be stopped while it streams (ADR 0027, sections 5
//! and 7.2).
//!
//! - [`cut_when_told`] wraps the answer's body: each time the server asks
//!   it for more data, it first looks at the flow's [`CutSignal`]. Once
//!   the flow is cut, the body ends with an error instead of more data, so
//!   the server drops the connection and the consumer sees an incomplete
//!   download, never a complete one.
//! - [`watchdog`] runs next to the download and checks the consumer's
//!   membership again every [`AccessGate::recheck_interval`](crate::access::AccessGate::recheck_interval). When it
//!   no longer holds, it ends the transfer (which cuts the flow); when the
//!   check cannot finish, it cuts the flow's downloads and keeps the
//!   transfer. It stops with the download: the body holds it and aborts
//!   it when dropped.
//! - [`state_watchdog`] runs next to every download and push and reads
//!   the flow's state from the database every [`STATE_CHECK_EVERY`]. Once
//!   the flow is suspended or terminated, or gone, it cuts the flow's
//!   work in this process (ADR 0029, decision 6; TD-52). A suspension or
//!   termination handled by this process cuts at once
//!   ([`RunningFlows::cut`]); one handled by another process that shares
//!   the database is seen here within those 5 seconds. A suspension and
//!   a start again on another process, both within one interval, are not
//!   seen: the flow is `STARTED` again when it is read, and the work goes
//!   on.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Body;
use axum::response::Response;
use hyper::body::{Bytes, Frame, SizeHint};
use tokio::task::JoinHandle;

use portitor_dps::state::DataFlowState;

use crate::access::{Access, AccessGate};
use crate::flows::FlowStore;
use crate::running_flows::{CutSignal, RunningFlows};

/// How often a running download or push reads its flow's state (ADR
/// 0029, decision 6): every 5 seconds.
pub const STATE_CHECK_EVERY: Duration = Duration::from_secs(5);

/// The error type of a body: any error that can cross threads.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A background task that is stopped when this value is dropped.
pub(super) struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts the membership checks of one download of flow `flow_id` (module
/// doc). Keep the value as long as the download runs.
pub(super) fn watchdog(
    gate: Arc<dyn AccessGate>,
    running: Arc<RunningFlows>,
    flow_id: &str,
) -> AbortOnDrop {
    let flow_id = flow_id.to_owned();
    let every = gate.recheck_interval();
    let checks = async move {
        loop {
            tokio::time::sleep(every).await;
            match gate.check(&flow_id).await {
                Access::Granted => {}
                Access::Unchecked(reason) => {
                    tracing::warn!(flow = %flow_id, %reason, "running download cut: the consumer's membership could not be checked now");
                    running.cut(&flow_id);
                    return;
                }
                Access::Revoked(reason) => {
                    // ending the transfer terminates the flow, which cuts
                    // it; cut here too, in case there is no transfer
                    gate.end_transfer(&flow_id, &reason);
                    running.cut(&flow_id);
                    return;
                }
            }
        }
    };
    AbortOnDrop(tokio::spawn(checks))
}

/// Starts the reads of the state of flow `flow_id` while one download or
/// push of it runs (module doc). Keep the value as long as it runs.
pub(super) fn state_watchdog(
    flows: Arc<dyn FlowStore + Send + Sync>,
    running: Arc<RunningFlows>,
    flow_id: &str,
) -> AbortOnDrop {
    let flow_id = flow_id.to_owned();
    let checks = async move {
        loop {
            tokio::time::sleep(STATE_CHECK_EVERY).await;
            let state = match flows.get(&flow_id) {
                Ok(Some(flow)) => Some(flow.state),
                Ok(None) => None,
                Err(error) => {
                    // the database may answer at the next read; a
                    // suspension in this process still cuts at once
                    tracing::warn!(flow = %flow_id, error = %error.0, "the state of a running data flow could not be read");
                    continue;
                }
            };
            if stops_work(state) {
                tracing::info!(flow = %flow_id, ?state, "running download or push cut: its flow was suspended or terminated");
                running.cut(&flow_id);
                return;
            }
        }
    };
    AbortOnDrop(tokio::spawn(checks))
}

/// Whether a flow in `state` (`None`: the flow is gone) stops the work
/// that runs for it: as the suspension and the termination do in the
/// process that handles them. A completion does not, there either.
fn stops_work(state: Option<DataFlowState>) -> bool {
    match state {
        Some(DataFlowState::Suspended | DataFlowState::Terminated) | None => true,
        Some(_) => false,
    }
}

/// `response`, whose body stops with an error once `cut` fires (module
/// doc). The `watchdogs` live as long as the body.
pub(super) fn cut_when_told(
    response: Response,
    mut cut: CutSignal,
    watchdogs: Vec<AbortOnDrop>,
) -> Response {
    let (parts, body) = response.into_parts();
    let cut_body = CutBody {
        inner: body,
        cut: Box::pin(async move { cut.cut().await }),
        ended: false,
        _watchdogs: watchdogs,
    };
    Response::from_parts(parts, Body::new(cut_body))
}

/// A body that ends with an error once its flow is cut.
struct CutBody {
    inner: Body,
    /// Finishes when the flow is cut.
    cut: Pin<Box<dyn Future<Output = ()> + Send>>,
    /// Set once the body has given its last frame or its error.
    ended: bool,
    _watchdogs: Vec<AbortOnDrop>,
}

/// Why a download stopped early; only the server's log sees it.
#[derive(Debug)]
struct Cut;

impl std::fmt::Display for Cut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the data flow was cut: suspended, terminated, or the membership check failed")
    }
}

impl std::error::Error for Cut {}

impl hyper::body::Body for CutBody {
    type Data = Bytes;
    type Error = BoxError;

    /// The server asks for the next piece of data: the cut is looked at
    /// first, so no more data goes out once it fired.
    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        // every field is `Unpin`, so the pin can be undone
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        if this.cut.as_mut().poll(context).is_ready() {
            this.ended = true;
            return Poll::Ready(Some(Err(Box::new(Cut))));
        }
        match Pin::new(&mut this.inner).poll_frame(context) {
            Poll::Ready(Some(Ok(frame))) => Poll::Ready(Some(Ok(frame))),
            Poll::Ready(Some(Err(error))) => {
                this.ended = true;
                Poll::Ready(Some(Err(Box::new(error))))
            }
            Poll::Ready(None) => {
                this.ended = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.ended || self.inner.is_end_stream()
    }

    /// The upstream's size, so a known length is sent as before.
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use http_body_util::BodyExt as _;

    use super::*;

    /// A body that gives one piece, then waits forever.
    struct Endless(bool);

    impl hyper::body::Body for Endless {
        type Data = Bytes;
        type Error = BoxError;

        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
            let this = self.get_mut();
            if this.0 {
                return Poll::Pending;
            }
            this.0 = true;
            Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"first")))))
        }
    }

    fn endless() -> Response {
        Response::new(Body::new(Endless(false)))
    }

    #[tokio::test]
    async fn a_cut_ends_the_body_with_an_error() {
        let running = RunningFlows::new();
        let cut = running.watch("f1");
        let response = cut_when_told(endless(), cut, Vec::new());
        let mut body = response.into_body();
        let first = body.frame().await.unwrap().unwrap();
        assert_eq!(first.into_data().unwrap(), "first");
        running.cut("f1");
        let next = tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap();
        assert!(next.unwrap().is_err(), "an error, not the end");
        assert!(body.frame().await.is_none());
    }

    /// A flow in the store, STARTED, and its running work.
    fn started_flow() -> (Arc<crate::test_store::InMemoryFlows>, Arc<RunningFlows>) {
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        let flow = crate::Flow {
            id: "f1".to_owned(),
            state: DataFlowState::Started,
            dataset_id: "d1".to_owned(),
            source: Some("https://upstream.example/d1".to_owned()),
            agreement_id: "a1".to_owned(),
            counter_party_id: "did:web:c.example".to_owned(),
            token_hash: String::new(),
            expires_at: u64::MAX,
            push_address: None,
        };
        assert!(flows.insert(&flow).unwrap());
        (flows, Arc::new(RunningFlows::new()))
    }

    /// TD-52: another process suspends the flow (it writes the state in
    /// the shared database and cuts only its own work). The watchdog of
    /// the download here reads the state within 5 seconds and cuts it.
    /// A paused clock: the reads come at once, and the test reads when.
    #[tokio::test(start_paused = true)]
    async fn a_suspension_written_by_another_process_cuts_within_5_seconds() {
        let (flows, running) = started_flow();
        let mut cut = running.watch("f1");
        let _watchdog = state_watchdog(Arc::clone(&flows) as _, Arc::clone(&running), "f1");
        // still STARTED: the reads change nothing
        tokio::time::sleep(STATE_CHECK_EVERY * 3).await;
        assert!(!cut.is_cut());
        let begun = tokio::time::Instant::now();
        let changed = flows.set_state("f1", DataFlowState::Started, DataFlowState::Suspended);
        assert!(changed.unwrap());
        cut.cut().await;
        assert!(begun.elapsed() <= STATE_CHECK_EVERY);
    }

    /// A completion does not cut, as in the process that handles it.
    #[tokio::test(start_paused = true)]
    async fn a_completion_does_not_cut() {
        let (flows, running) = started_flow();
        let cut = running.watch("f1");
        let _watchdog = state_watchdog(Arc::clone(&flows) as _, Arc::clone(&running), "f1");
        let changed = flows.set_state("f1", DataFlowState::Started, DataFlowState::Completed);
        assert!(changed.unwrap());
        tokio::time::sleep(STATE_CHECK_EVERY * 3).await;
        assert!(!cut.is_cut());
        assert!(stops_work(None), "a flow that is gone stops");
        assert!(stops_work(Some(DataFlowState::Terminated)));
    }
}
