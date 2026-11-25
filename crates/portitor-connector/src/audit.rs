// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The audit line of a data transfer (ADR 0027, section 6; ADR 0025,
//! "Audit"; TD-31, first half): each finished pull, push sent and push
//! received writes one JSON log line at INFO, with the message
//! `data transfer done` and the target [`AUDIT_EVENTS`]:
//!
//! - `flow`: the transfer's DSP `providerPid`, on both sides, so the
//!   provider's line and the consumer's line of one push can be matched;
//! - `agreement`: the agreement id;
//! - `counterParty`: the other participant's DID;
//! - `direction`: `sent` (a pull served, a push sent) or `received` (a
//!   push received) ([`Direction`]);
//! - `bytes`: the bytes of data actually sent (or received);
//! - `outcome`: `complete`, `cut` or `failed` ([`Outcome`]).
//!
//! The message, the target and these fields are stable in 1.x
//! (`docs/STABILITY.md`); a later release may add fields.
//!
//! It never holds the token or the data source's URL. There is no audit
//! store: the operator ships and keeps the log (ADR 0023).
//!
//! Two ways to write it:
//!
//! - [`transfer_done`] writes the line, for code that knows the outcome
//!   and the byte count itself (a push, for example).
//! - [`AuditedBody`] wraps a streamed HTTP body, counts the bytes that
//!   pass through it and writes the line once, when the stream ends, fails
//!   or is dropped early.

use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::body::{Body, Bytes, Frame, SizeHint};

/// The target of the audit lines, so that they can be filtered
/// (`RUST_LOG`) and found apart from the other log lines.
pub const AUDIT_EVENTS: &str = "portitor::audit";

/// The message of the audit line.
pub const TRANSFER_DONE: &str = "data transfer done";

/// How a data transfer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// All the data was sent (or received).
    Complete,
    /// The transfer stopped before the end without an error of the data:
    /// the other party closed the connection, or access was taken away
    /// (suspend, terminate) while the data was flowing.
    Cut,
    /// The data source or the connection failed in the middle, or before
    /// any data was sent (for example the source did not answer).
    Failed,
}

impl Outcome {
    /// The value of the `outcome` field: `complete`, `cut` or `failed`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Complete => "complete",
            Outcome::Cut => "cut",
            Outcome::Failed => "failed",
        }
    }
}

/// Which way the data went, seen from the connector that writes the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// This connector sent the data: a pull it served, a push it sent.
    Sent,
    /// This connector received the data: a push into its bucket.
    Received,
}

impl Direction {
    /// The value of the `direction` field: `sent` or `received`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Direction::Sent => "sent",
            Direction::Received => "received",
        }
    }
}

/// Writes the audit line of one finished data transfer: flow `flow` (the
/// transfer's `providerPid`), under agreement `agreement`, with the
/// participant `counter_party` (its DID), `bytes` of data going
/// `direction`, ended with `outcome`. Call it exactly once per transfer of
/// data (one `/data` response, one push sent, one push received).
pub fn transfer_done(
    flow: &str,
    agreement: &str,
    counter_party: &str,
    direction: Direction,
    bytes: u64,
    outcome: Outcome,
) {
    tracing::event!(
        target: AUDIT_EVENTS,
        tracing::Level::INFO,
        flow,
        agreement,
        counterParty = counter_party,
        direction = direction.name(),
        bytes,
        outcome = outcome.name(),
        "{TRANSFER_DONE}"
    );
}

/// Who a flow's data goes to: what the audit line names besides the flow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parties {
    /// The agreement id; empty when it could not be found.
    pub agreement: String,
    /// The other participant's DID; empty when it could not be found.
    pub counter_party: String,
}

/// A streamed HTTP body that counts the bytes passing through it and
/// writes the audit line once:
///
/// - `complete` when the inner body has ended;
/// - `failed` when the inner body gives an error (the data source broke
///   off);
/// - `cut` when it is dropped before the end: the receiver closed the
///   connection, or something outside it stopped the stream.
///
/// Code that cuts a running transfer (suspend, terminate) should wrap
/// this body, not be wrapped by it: dropping it then reports `cut`. An
/// error coming from inside it reports `failed`.
pub struct AuditedBody<B: Body> {
    inner: B,
    flow: String,
    parties: Parties,
    bytes: u64,
    /// Set once the line is written, so it is written only once.
    written: bool,
}

impl<B: Body> AuditedBody<B> {
    /// Wraps `inner`, the data of flow `flow` sent to `parties`.
    pub fn new(inner: B, flow: &str, parties: Parties) -> Self {
        Self {
            inner,
            flow: flow.to_owned(),
            parties,
            bytes: 0,
            written: false,
        }
    }

    /// Writes the audit line with `outcome`, unless it was written.
    fn finish(&mut self, outcome: Outcome) {
        if self.written {
            return;
        }
        self.written = true;
        transfer_done(
            &self.flow,
            &self.parties.agreement,
            &self.parties.counter_party,
            Direction::Sent,
            self.bytes,
            outcome,
        );
    }
}

impl<B> Body for AuditedBody<B>
where
    B: Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, B::Error>>> {
        // `AuditedBody` is `Unpin` because `B` is, so it can be used as a
        // plain `&mut`
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let length = u64::try_from(data.len()).unwrap_or(u64::MAX);
                    this.bytes = this.bytes.saturating_add(length);
                }
                // the server may stop asking once the body says it has
                // ended (a known length was reached)
                if this.inner.is_end_stream() {
                    this.finish(Outcome::Complete);
                }
            }
            Poll::Ready(Some(Err(_))) => this.finish(Outcome::Failed),
            Poll::Ready(None) => this.finish(Outcome::Complete),
            Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<B: Body> Drop for AuditedBody<B> {
    fn drop(&mut self) {
        // dropped before the line was written: the stream was stopped
        // from outside, unless the body had in fact ended (an empty body
        // the server never needed to read)
        let outcome = if self.inner.is_end_stream() {
            Outcome::Complete
        } else {
            Outcome::Cut
        };
        self.finish(outcome);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use http_body_util::BodyExt as _;

    use super::*;

    /// The JSON log lines written while a test runs.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Log {
        fn lines(&self) -> Vec<serde_json::Value> {
            let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
            text.lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    /// Runs `run` on a runtime of its own, and returns the log lines it
    /// wrote.
    fn logged(run: impl std::future::Future<Output = ()>) -> Vec<serde_json::Value> {
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            runtime.block_on(run);
        });
        log.lines()
    }

    /// A body of `chunks`, then an error if `fails`.
    struct Chunks {
        chunks: VecDeque<&'static str>,
        fails: bool,
    }

    impl Body for Chunks {
        type Data = Bytes;
        type Error = String;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, String>>> {
            let next = match self.chunks.pop_front() {
                Some(chunk) => Some(Ok(Frame::data(Bytes::from_static(chunk.as_bytes())))),
                None if self.fails => Some(Err("the source broke off".to_owned())),
                None => None,
            };
            Poll::Ready(next)
        }
    }

    fn parties() -> Parties {
        Parties {
            agreement: "urn:uuid:agreement-1".to_owned(),
            counter_party: "did:web:consumer.example".to_owned(),
        }
    }

    fn audited(chunks: &[&'static str], fails: bool) -> AuditedBody<Chunks> {
        let body = Chunks {
            chunks: chunks.iter().copied().collect(),
            fails,
        };
        AuditedBody::new(body, "urn:uuid:flow-1", parties())
    }

    /// The one audit line among `lines`, as (bytes, outcome).
    fn the_line(lines: &[serde_json::Value]) -> (u64, String) {
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert_eq!(line["level"], "INFO");
        assert_eq!(line["target"], AUDIT_EVENTS);
        let fields = &line["fields"];
        assert_eq!(fields["message"], TRANSFER_DONE);
        assert_eq!(fields["flow"], "urn:uuid:flow-1");
        assert_eq!(fields["agreement"], "urn:uuid:agreement-1");
        assert_eq!(fields["counterParty"], "did:web:consumer.example");
        let bytes = fields["bytes"].as_u64().unwrap();
        (bytes, fields["outcome"].as_str().unwrap().to_owned())
    }

    #[test]
    fn a_body_read_to_its_end_is_complete_with_every_byte_counted() {
        let lines = logged(async {
            let body = audited(&["day,celsius\n", "mon,12\n"], false);
            let all = body.collect().await.unwrap().to_bytes();
            assert_eq!(all.len(), 19);
        });
        assert_eq!(the_line(&lines), (19, "complete".to_owned()));
    }

    #[test]
    fn a_body_dropped_before_its_end_is_cut() {
        let lines = logged(async {
            let mut body = audited(&["day,celsius\n", "mon,12\n"], false);
            body.frame().await.unwrap().unwrap();
            drop(body);
        });
        assert_eq!(the_line(&lines), (12, "cut".to_owned()));
    }

    #[test]
    fn a_body_whose_source_breaks_off_failed() {
        let lines = logged(async {
            let mut body = audited(&["day,celsius\n"], true);
            body.frame().await.unwrap().unwrap();
            assert!(body.frame().await.unwrap().is_err());
            drop(body);
        });
        assert_eq!(the_line(&lines), (12, "failed".to_owned()));
    }

    /// A body of known length says it has ended with its last frame; the
    /// server then stops reading. That is complete, not cut.
    #[test]
    fn a_body_that_says_it_has_ended_is_complete() {
        let lines = logged(async {
            let full = http_body_util::Full::new(Bytes::from_static(b"hello"));
            let mut body = AuditedBody::new(full, "urn:uuid:flow-1", parties());
            body.frame().await.unwrap().unwrap();
            assert!(body.is_end_stream());
            drop(body);
        });
        assert_eq!(the_line(&lines), (5, "complete".to_owned()));
    }

    #[test]
    fn the_outcomes_have_the_documented_names() {
        let names = [Outcome::Complete, Outcome::Cut, Outcome::Failed].map(Outcome::name);
        assert_eq!(names, ["complete", "cut", "failed"]);
    }
}
