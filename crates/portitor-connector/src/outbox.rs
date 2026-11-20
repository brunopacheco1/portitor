// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Sending saved DSP messages (the outbox, ADR 0007). Four adapters send
//! messages: negotiation and transfer, each as provider and as consumer.
//! Each one implements [`Outbox`], which says how its domain service
//! records what became of a message; [`deliver`] holds the rules they
//! share:
//!
//! 1. Send the message with a fresh token for the counter-party (ADR
//!    0016), retrying as [`Retry`] says, but only when a retry may help
//!    ([`worth_retrying`]: no answer, 408, 429 or 5xx).
//! 2. Acknowledged: record it, and send whatever follows.
//! 3. Refused or unanswered: ask the counter-party for its state. If it
//!    already shows where our message leads, the message had arrived
//!    before (a lost answer, or a resend after a restart): record it.
//! 4. Otherwise, a consumer's first message means the process never
//!    started on the other side; a termination may stay unanswered (DSP
//!    allows it); anything else ends the process.
//!
//! A panic (a bug) while a message is sent or recorded does not lose the
//! message: the same message is sent again after a pause
//! ([`send_until_counter_party_acts`]). That is safe for the reason a
//! resend after a restart is: a message is recorded only while it is
//! still the process's pending message (ADR 0007).

use std::sync::Arc;
use std::time::Duration;

use portitor_dsp::guard::{Direction, MessageGuard};
use portitor_dsp::policy::CredentialTypes;
use portitor_runtime::panics::{Panicked, catch_panic, next_pause};
use portitor_runtime::telemetry::metrics;
use serde_json::Value;
use tracing::Instrument as _;

use crate::auth::Prover;
use crate::http_client::{Sender, answered_status, asks_to_try_later};
use crate::ports::{Ports, Retry};

/// What every adapter needs to send: the client, the source of our
/// tokens, the dataspace's message rules, the retry policy, and the
/// credential types policies may use (for the scopes of our tokens, and
/// of the credentials we ask a caller for; ADR 0019).
pub(crate) struct Link {
    pub(crate) sender: Arc<dyn Sender>,
    pub(crate) prover: Arc<dyn Prover>,
    pub(crate) guard: Arc<dyn MessageGuard + Send + Sync>,
    pub(crate) retry: Retry,
    pub(crate) credential_types: CredentialTypes,
}

impl Link {
    pub(crate) fn new(ports: &Ports) -> Self {
        Self {
            sender: Arc::clone(&ports.sender),
            prover: Arc::clone(&ports.prover),
            guard: Arc::clone(&ports.guard),
            retry: ports.retry,
            credential_types: ports.credential_types.clone(),
        }
    }
}

/// One saved message, as [`deliver`] needs it.
pub(crate) struct Message<'a> {
    /// Our process id, for the log.
    pub(crate) pid: &'a str,
    /// Where to POST it.
    pub(crate) url: &'a str,
    /// The message.
    pub(crate) body: &'a Value,
    /// Where the counter-party reports its state (`GET`); empty when
    /// unknown (a consumer's request: no provider pid yet).
    pub(crate) status_url: &'a str,
    /// The counter-party's participant id: the audience of our token.
    pub(crate) audience: &'a str,
    /// A consumer's first message (a request): if it never arrives, the
    /// process never started on the other side.
    pub(crate) first: bool,
    /// A termination, whose missing answer DSP lets us ignore.
    pub(crate) terminating: bool,
}

impl<'a> Message<'a> {
    /// A message for the participant `audience` that is neither a first
    /// message nor a termination.
    pub(crate) fn new(
        pid: &'a str,
        url: &'a str,
        body: &'a Value,
        status_url: &'a str,
        audience: &'a str,
    ) -> Self {
        Self {
            pid,
            url,
            body,
            status_url,
            audience,
            first: false,
            terminating: false,
        }
    }

    /// Marks a consumer's first message, when `first`.
    pub(crate) fn first(self, first: bool) -> Self {
        Self { first, ..self }
    }

    /// Marks a termination, when `terminating`.
    pub(crate) fn terminating(self, terminating: bool) -> Self {
        Self {
            terminating,
            ..self
        }
    }
}

/// One adapter's side of the outbox: how its domain service records what
/// became of a message.
pub(crate) trait Outbox: Send + Sync + 'static {
    /// A saved message.
    type Out: Send + Sync + 'static;
    /// Why the store could not record it.
    type Failure: std::fmt::Debug;

    /// How to send.
    fn link(&self) -> &Link;

    /// Which outbox this is, for the metrics: the process and our role.
    fn kind(&self) -> metrics::Outbox;

    /// The scopes, beyond the configured ones, that the counter-party may
    /// read with the token sent with the message: those an offer's
    /// credential constraints need (ADR 0019). None by default, so the
    /// default does not look at the message (`_out`).
    fn scopes(&self, _out: &Self::Out) -> Vec<String> {
        Vec::new()
    }

    /// The parts of `out` that sending needs.
    fn message(out: &Self::Out) -> Message<'_>;

    /// Whether `state`, the counter-party's answer at the status URL, shows
    /// the state that `out` leads to.
    fn reached(out: &Self::Out, state: Value) -> bool;

    /// `out` arrived (`answer`: the acknowledgement, if there was one).
    /// Returns the next message, if any.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn delivered(
        &self,
        out: &Self::Out,
        answer: Option<&Value>,
    ) -> Result<Option<Self::Out>, Self::Failure>;

    /// The message, a first message, never arrived: end the process here.
    /// Only a consumer sends first messages, so by default there is
    /// nothing to do (and the default does not look at `_out`).
    ///
    /// # Errors
    ///
    /// The store failed.
    fn not_started(&self, _out: &Self::Out) -> Result<(), Self::Failure> {
        Ok(())
    }

    /// The counter-party cannot be reached: end the process. Returns the
    /// termination to send, if any.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn abort(&self, out: &Self::Out) -> Result<Option<Self::Out>, Self::Failure>;
}

/// Sends every message in `pending` in the background. Returns how many.
pub(crate) fn resume<O: Outbox>(outbox: &Arc<O>, pending: Vec<O::Out>) -> usize {
    let count = pending.len();
    for out in pending {
        // a message saved before the stop is sent again
        metrics::outbox_resent(outbox.kind());
        tokio::spawn(deliver(Arc::clone(outbox), out));
    }
    count
}

/// Sends `out` and whatever follows from it, until the counter-party has
/// to act or the process ends (the rules are in the module doc).
///
/// The returned task runs in a span made now, a child of the current
/// span (the request that saved the message, if any), so its outbound
/// calls and state changes join that request's trace even though the
/// task runs later, on its own (`tokio::spawn`).
///
/// Why a plain `fn` that returns `impl Future`, not an `async fn`: the
/// body of an `async fn` runs only when the task is first polled, after
/// `tokio::spawn`, when the request's span is no longer the current one.
/// This body runs at the call, so the span is made in the request; only
/// the task it returns runs later.
pub(crate) fn deliver<O: Outbox>(outbox: Arc<O>, out: O::Out) -> impl Future<Output = ()> {
    let kind = outbox.kind();
    // `role_name()` is `None` for an outbox without a DSP role: the span
    // then has no `portitor.dsp.role` field
    let span = tracing::info_span!(
        "dsp.outbox.deliver",
        portitor.dsp.process = kind.process_name(),
        portitor.dsp.role = kind.role_name(),
        portitor.dsp.pid = O::message(&out).pid,
    );
    send_until_counter_party_acts(outbox, out).instrument(span)
}

/// The work of [`deliver`]: sends `out`, then each message that follows
/// from the answer, until it is the counter-party's turn or the process
/// ends.
///
/// A panic while one message is sent or recorded (a bug) is logged and
/// counted (`portitor.task.panicked`, task `outbox_delivery`), and the
/// same message is sent again after a pause: 5 seconds, twice as long
/// after each further panic on this message, at most 5 minutes
/// ([`next_pause`], as for a supervised task). Messages sent before it in
/// this task are recorded already and are not sent again. A message
/// that panics every time is tried for as long as the process runs.
///
/// Why sending again is safe (ADR 0007): the counter-party answers a
/// message it already took with an error, its state then shows that the
/// message arrived, and `delivered`, `abort` and `not_started` change
/// nothing when the message is no longer the pending one (it was
/// recorded before the panic, or the counter-party's next message
/// proved it arrived). It is what a restart does with a pending message.
async fn send_until_counter_party_acts<O: Outbox>(outbox: Arc<O>, mut out: O::Out) {
    // the pause before the current try of `out`; none before the first
    let mut pause: Option<Duration> = None;
    loop {
        let started = tokio::time::Instant::now();
        // `out` is only lent (`&out`) to the step, so after a panic this
        // loop still has it and can send it again
        match catch_panic(send_and_record(outbox.as_ref(), &out)).await {
            Ok(Some(next)) => {
                out = next;
                pause = None;
            }
            Ok(None) => return,
            Err(Panicked) => {
                let wait = next_pause(pause, started.elapsed());
                tracing::error!(
                    task = "outbox_delivery",
                    pid = O::message(&out).pid,
                    again_in_seconds = wait.as_secs(),
                    "a background task panicked; its message is sent again"
                );
                metrics::task_panicked("outbox_delivery");
                metrics::outbox_resent(outbox.kind());
                tokio::time::sleep(wait).await;
                pause = Some(wait);
            }
        }
    }
}

/// One step of [`send_until_counter_party_acts`]: sends `out` and records
/// what became of it. Returns the message that follows, or `None` when
/// it is the counter-party's turn or the process ended.
async fn send_and_record<O: Outbox>(outbox: &O, out: &O::Out) -> Option<O::Out> {
    let msg = O::message(out);
    let (pid, first) = (msg.pid.to_owned(), msg.first);
    let scopes = outbox.scopes(out);
    let reached = |state| O::reached(out, state);
    let recorded = match attempt(outbox.link(), outbox.kind(), &msg, &scopes, reached).await {
        Outcome::Acknowledged(answer) => outbox.delivered(out, Some(&answer)),
        Outcome::ArrivedBefore | Outcome::TerminationUnanswered => outbox.delivered(out, None),
        Outcome::NotStarted => outbox.not_started(out).map(|()| None),
        Outcome::Unreachable => outbox.abort(out),
    };
    match recorded {
        Ok(next) => next,
        Err(failure) => {
            tracing::warn!(%pid, ?failure, "outgoing message not recorded");
            if first {
                // without the counter-party's pid the process cannot go on
                let _ = outbox.not_started(out);
            }
            None
        }
    }
}

/// What became of a message [`attempt`] sent.
enum Outcome {
    /// The counter-party acknowledged it, with this answer.
    Acknowledged(Value),
    /// Refused or unanswered, but it had arrived before.
    ArrivedBefore,
    /// A first message could not be delivered.
    NotStarted,
    /// A termination was not acknowledged.
    TerminationUnanswered,
    /// The counter-party could not be reached.
    Unreachable,
}

/// Sends `msg`, with a token that also grants `scopes`, and says what
/// became of it. `reached` reads the counter-party's state; it is used
/// only when the send failed.
async fn attempt(
    link: &Link,
    kind: metrics::Outbox,
    msg: &Message<'_>,
    scopes: &[String],
    reached: impl FnOnce(Value) -> bool,
) -> Outcome {
    let failure = match send(link, kind, msg, scopes).await {
        Ok(answer) => return Outcome::Acknowledged(answer),
        Err(failure) => failure,
    };
    // a message the rules refused was not sent now, but an earlier copy
    // (before a restart with other rules) may have arrived: ask as well
    let outcome = if arrived(link, msg.status_url, msg.audience, reached).await {
        Outcome::ArrivedBefore
    } else if msg.first {
        Outcome::NotStarted
    } else if msg.terminating {
        Outcome::TerminationUnanswered
    } else {
        Outcome::Unreachable
    };
    log_unanswered(msg.pid, &outcome, &failure);
    // the message did not arrive and will not be sent again
    if !matches!(outcome, Outcome::ArrivedBefore) {
        metrics::outbox_given_up(kind);
    }
    outcome
}

/// One log line for a message that got no answer: what became of it
/// (`outcome`), and why it got none (`failure`: refused by the
/// dataspace's message rules, or not answered).
fn log_unanswered(pid: &str, outcome: &Outcome, failure: &SendFailure) {
    use SendFailure::{Failed, Rejected};
    match (outcome, failure) {
        (Outcome::ArrivedBefore, Rejected(error) | Failed(error)) => {
            tracing::info!(%pid, %error, "message had arrived before");
        }
        (Outcome::NotStarted, Rejected(error)) => tracing::warn!(
            %pid,
            %error,
            "message breaks the dataspace's message rules; not sent, process not started"
        ),
        (Outcome::NotStarted, Failed(error)) => {
            tracing::warn!(%pid, %error, "counter-party not reachable; process not started");
        }
        (Outcome::TerminationUnanswered, Rejected(error)) => tracing::warn!(
            %pid,
            %error,
            "termination breaks the dataspace's message rules; not sent"
        ),
        (Outcome::TerminationUnanswered, Failed(error)) => {
            tracing::warn!(%pid, %error, "termination not acknowledged");
        }
        (Outcome::Unreachable, Rejected(error)) => tracing::warn!(
            %pid,
            %error,
            "message breaks the dataspace's message rules; not sent, terminating"
        ),
        (Outcome::Unreachable, Failed(error)) => {
            tracing::warn!(%pid, %error, "counter-party not reachable; terminating");
        }
        // answered: nothing to say here
        (Outcome::Acknowledged(_), _) => {}
    }
}

/// Why [`send`] got no answer.
enum SendFailure {
    /// The dataspace's message rules (the guard) refused the message: it
    /// was not sent.
    Rejected(String),
    /// It was sent, or could not be, without an answer: no token, the
    /// counter-party down or refusing.
    Failed(String),
}

/// Whether the counter-party `audience`'s state at `status_url` passes
/// `reached`, that is, a message that leads there has arrived. `false`
/// when it cannot tell.
pub(crate) async fn arrived(
    link: &Link,
    status_url: &str,
    audience: &str,
    reached: impl FnOnce(Value) -> bool,
) -> bool {
    if status_url.is_empty() {
        return false;
    }
    let authorization = match link.prover.authorization(audience, &[]).await {
        Ok(authorization) => authorization,
        Err(error) => {
            tracing::warn!(%error, "no token to read the counter-party's state");
            return false;
        }
    };
    link.sender
        .get_authorized(status_url, &authorization)
        .await
        .is_ok_and(reached)
}

/// Checks the message's body with the guard, then POSTs it, retrying as
/// the link's retry policy says while a retry may help
/// ([`worth_retrying`]). Each try gets a new token, which also grants
/// `scopes`: a token is good for one request only. Returns the
/// counter-party's answer.
async fn send(
    link: &Link,
    kind: metrics::Outbox,
    msg: &Message<'_>,
    scopes: &[String],
) -> Result<Value, SendFailure> {
    let (url, body) = (msg.url, msg.body);
    let message_type = body
        .get("@type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    link.guard
        .check(Direction::Outbound, message_type, body)
        .map_err(|rejection| SendFailure::Rejected(rejection.to_string()))?;
    let mut delay = link.retry.first_delay;
    let mut last = String::from("not sent");
    for attempt in 0..link.retry.attempts.max(1) {
        if attempt > 0 {
            tokio::time::sleep(delay).await;
            delay = delay.saturating_mul(2);
            metrics::outbox_resent(kind);
        }
        let authorization = match link.prover.authorization(msg.audience, scopes).await {
            Ok(authorization) => authorization,
            Err(e) => {
                last = format!("no token: {e}");
                continue;
            }
        };
        match link.sender.post_authorized(url, body, &authorization).await {
            Ok(answer) => return Ok(answer),
            Err(e) if worth_retrying(&e) => last = e,
            // the counter-party answered and will not take this message
            Err(e) => return Err(SendFailure::Failed(e)),
        }
    }
    Err(SendFailure::Failed(last))
}

/// Whether a send that failed with `error` is worth another try (ADR
/// 0007, amended in 0.11; TD-30): no answer at all (connection refused,
/// timeout, an answer that could not be read), or the answers 408
/// (Request Timeout), 429 (Too Many Requests) and 5xx. Any other answer,
/// such as 400, 401, 404 or 409, means the counter-party will not take
/// this message, so it is not sent again. A partner's Portitor answers
/// 503 when it could not check our credentials because a service was
/// down (ADR 0016, amended in 0.11), so that case is tried again. When it
/// refuses our credentials it answers 404 on the negotiation and transfer
/// routes (401 before 0.12; Eclipse EDC answers 404 too): final, like a
/// process the partner does not know. The log of the partner says which
/// of the two it was; ours cannot (ADR 0016, amended in 0.12).
///
/// An error without a status is tried again too, even when it came from
/// our side (a URL our client refuses, no CA certificates): the text does
/// not say which side it came from. Sending again is safe: proof of
/// delivery (ADR 0007) makes a message that did arrive count once.
fn worth_retrying(error: &str) -> bool {
    // the `Sender` port's errors are text; `answered_status` reads the
    // status back from the text `HttpError` writes (`HTTP 404 Not Found`)
    match answered_status(error) {
        None => true,
        Some(code) => asks_to_try_later(code) || tck_retries(code),
    }
}

/// The TCK build also tries a 404 again. The DSP TCK v1.0.2 registers the
/// handler for a message only once it has seen the state before it (for
/// example `thenWaitForState(SUSPENDED)` before `expectTerminationMessage`
/// in `TP_C:02-03`), so a message sent right after the previous one gets
/// 404, and the TCK then waits for it with no time limit. Before 0.11 the
/// outbox tried every refusal again, which hid this. Never in release
/// images (R-08).
#[cfg(feature = "tck")]
fn tck_retries(status: u16) -> bool {
    status == 404
}

/// See the TCK build's version: a release build never tries a 404 again.
#[cfg(not(feature = "tck"))]
fn tck_retries(_status: u16) -> bool {
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use portitor_dsp::guard::Rejection;
    use portitor_runtime::BoxFuture;

    use super::*;

    /// Takes every message, answering `null`.
    struct Accepts;

    impl Sender for Accepts {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Ok(Value::Null) })
        }
    }

    /// Answers nothing.
    struct Down;

    impl Sender for Down {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Err("connection refused".to_owned()) })
        }
    }

    /// A prover with a fixed token.
    struct FixedToken;

    impl Prover for FixedToken {
        fn authorization<'a>(
            &'a self,
            _: &'a str,
            _: &'a [String],
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async { Ok("Bearer t".to_owned()) })
        }
    }

    /// Refuses every message.
    struct RefusesAll;

    impl MessageGuard for RefusesAll {
        fn check(&self, _: Direction, _: &str, _: &Value) -> Result<(), Rejection> {
            Err(Rejection {
                reasons: vec!["SHACL: a constraint is needed".to_owned()],
            })
        }
    }

    /// The log, as the subscriber writes it.
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

    /// Sends one message (first, or a later one) through `sender` and the
    /// guard `guard`; returns the outcome's name and the log.
    fn attempt_logged(
        sender: Arc<dyn Sender>,
        guard: Arc<dyn MessageGuard + Send + Sync>,
        first: bool,
    ) -> (&'static str, String) {
        let mut ports = Ports::new(sender);
        ports.prover = Arc::new(FixedToken);
        ports.guard = guard;
        ports.retry = Retry {
            attempts: 1,
            first_delay: std::time::Duration::from_millis(1),
        };
        let link = Link::new(&ports);
        let body = serde_json::json!({"@type": "ContractRequestMessage"});
        let msg = Message::new(
            "urn:uuid:1",
            "https://p.example/dsp",
            &body,
            "",
            "did:web:p.example",
        )
        .first(first);
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let outcome = tracing::subscriber::with_default(subscriber, || {
            let kind = metrics::Outbox::Negotiation(metrics::Role::Consumer);
            runtime.block_on(attempt(&link, kind, &msg, &[], |_| false))
        });
        let name = match outcome {
            Outcome::Acknowledged(_) => "acknowledged",
            Outcome::ArrivedBefore => "arrived before",
            Outcome::NotStarted => "not started",
            Outcome::TerminationUnanswered => "termination unanswered",
            Outcome::Unreachable => "unreachable",
        };
        let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        (name, text)
    }

    #[test]
    fn a_message_the_rules_refuse_is_logged_as_a_rejection() {
        for (first, outcome, line) in [
            (
                false,
                "unreachable",
                "message breaks the dataspace's message rules; not sent, terminating",
            ),
            (
                true,
                "not started",
                "message breaks the dataspace's message rules; not sent, process not started",
            ),
        ] {
            let (name, log) = attempt_logged(Arc::new(Accepts), Arc::new(RefusesAll), first);
            assert_eq!(name, outcome);
            assert!(log.contains(line), "{log}");
            assert!(
                log.contains("message rejected: SHACL: a constraint is needed"),
                "{log}"
            );
            assert!(!log.contains("not reachable"), "{log}");
        }
    }

    #[test]
    fn a_counter_party_that_does_not_answer_is_logged_as_not_reachable() {
        let allow_all: Arc<dyn MessageGuard + Send + Sync> =
            Arc::new(portitor_dsp::guard::AllowAll);
        let (name, log) = attempt_logged(Arc::new(Down), allow_all, false);
        assert_eq!(name, "unreachable");
        assert!(
            log.contains("counter-party not reachable; terminating"),
            "{log}"
        );
        assert!(!log.contains("message rules"), "{log}");
    }

    /// Fails every POST and counts them: the first with `error`, the next
    /// ones with each of `then` in turn, and the last error again once
    /// they run out. Answers every GET (the counter-party's state) with
    /// `null`.
    struct Fails {
        errors: Vec<&'static str>,
        posts: std::sync::atomic::AtomicUsize,
    }

    impl Fails {
        fn new(error: &'static str) -> Arc<Self> {
            Self::then(error, &[])
        }

        fn then(error: &'static str, then: &[&'static str]) -> Arc<Self> {
            let mut errors = vec![error];
            errors.extend_from_slice(then);
            Arc::new(Self {
                errors,
                posts: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn posts(&self) -> usize {
            self.posts.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Sender for Fails {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            let post = self.posts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let error = self.errors[post.min(self.errors.len() - 1)];
            Box::pin(async move { Err(error.to_owned()) })
        }

        fn get<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Ok(Value::Null) })
        }
    }

    /// Sends one message (not a first one) through `sender`, which may
    /// try 3 times, at once. Its state is read at `status_url` (empty:
    /// unknown), and `reached` says whether the state shows it arrived.
    async fn attempt_through(sender: Arc<Fails>, status_url: &str, reached: bool) -> Outcome {
        let mut ports = Ports::new(sender);
        ports.prover = Arc::new(FixedToken);
        ports.guard = Arc::new(portitor_dsp::guard::AllowAll);
        ports.retry = Retry {
            attempts: 3,
            first_delay: std::time::Duration::from_millis(1),
        };
        let link = Link::new(&ports);
        let body = serde_json::json!({"@type": "ContractAgreementMessage"});
        let msg = Message::new(
            "urn:uuid:1",
            "https://c.example/negotiations/1/agreement",
            &body,
            status_url,
            "did:web:c.example",
        );
        let kind = metrics::Outbox::Negotiation(metrics::Role::Provider);
        attempt(&link, kind, &msg, &[], |_| reached).await
    }

    #[tokio::test]
    async fn no_answer_408_429_and_5xx_are_tried_again() {
        for error in [
            "connection refused",
            "no answer within 10 s",
            "HTTP 408 Request Timeout",
            "HTTP 429 Too Many Requests",
            "HTTP 500 Internal Server Error",
            "HTTP 503 Service Unavailable",
        ] {
            let sender = Fails::new(error);
            let outcome = attempt_through(Arc::clone(&sender), "", false).await;
            assert!(matches!(outcome, Outcome::Unreachable), "{error}");
            assert_eq!(sender.posts(), 3, "{error}");
        }
    }

    #[tokio::test]
    async fn any_other_answer_is_not_sent_again() {
        for error in [
            "HTTP 400 Bad Request",
            "HTTP 401 Unauthorized",
            "HTTP 403 Forbidden",
            "HTTP 409 Conflict",
            "HTTP 302 Found",
        ] {
            let sender = Fails::new(error);
            let outcome = attempt_through(Arc::clone(&sender), "", false).await;
            assert!(matches!(outcome, Outcome::Unreachable), "{error}");
            assert_eq!(sender.posts(), 1, "{error}");
        }
    }

    #[tokio::test]
    async fn a_404_is_tried_again_only_in_the_tck_build() {
        let sender = Fails::new("HTTP 404 Not Found");
        let outcome = attempt_through(Arc::clone(&sender), "", false).await;
        assert!(matches!(outcome, Outcome::Unreachable));
        let expected = if cfg!(feature = "tck") { 3 } else { 1 };
        assert_eq!(sender.posts(), expected);
    }

    #[tokio::test]
    async fn the_tries_stop_at_the_first_answer_that_may_not_be_retried() {
        // after a failure that is tried again, a refusal still ends the
        // tries (ROADMAP 0.11, exit)
        let sender = Fails::then("HTTP 503 Service Unavailable", &["HTTP 400 Bad Request"]);
        let outcome = attempt_through(Arc::clone(&sender), "", false).await;
        assert!(matches!(outcome, Outcome::Unreachable));
        assert_eq!(sender.posts(), 2);
    }

    #[tokio::test]
    async fn a_refused_message_still_counts_as_delivered_when_its_state_shows_it() {
        // ADR 0007, proof of delivery: a counter-party that already took
        // the message (a resend after a restart) may refuse it; its state
        // then shows that the message arrived
        let sender = Fails::new("HTTP 409 Conflict");
        let status_url = "https://c.example/negotiations/1";
        let outcome = attempt_through(Arc::clone(&sender), status_url, true).await;
        assert!(matches!(outcome, Outcome::ArrivedBefore));
        assert_eq!(sender.posts(), 1);
    }

    /// An outbox whose first `panics` recordings panic (a bug); it counts
    /// the recordings.
    struct PanicsWhenRecording {
        link: Link,
        panics: usize,
        recordings: std::sync::atomic::AtomicUsize,
    }

    impl Outbox for PanicsWhenRecording {
        /// The message's body.
        type Out = Value;
        type Failure = String;

        fn link(&self) -> &Link {
            &self.link
        }

        fn kind(&self) -> metrics::Outbox {
            metrics::Outbox::Negotiation(metrics::Role::Provider)
        }

        fn message(out: &Value) -> Message<'_> {
            Message::new("urn:uuid:1", "https://c.example/x", out, "", "did:web:c")
        }

        fn reached(_: &Value, _: Value) -> bool {
            false
        }

        fn delivered(&self, _: &Value, _: Option<&Value>) -> Result<Option<Value>, String> {
            let nth = self
                .recordings
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert!(nth >= self.panics, "recording {nth} panics");
            Ok(None)
        }

        fn abort(&self, _: &Value) -> Result<Option<Value>, String> {
            Ok(None)
        }
    }

    /// A paused clock: the pauses pass at once, and the test reads how
    /// long they were.
    #[tokio::test(start_paused = true)]
    async fn a_message_whose_delivery_panics_is_sent_again_after_a_pause() {
        let mut ports = Ports::new(Arc::new(Accepts));
        ports.prover = Arc::new(FixedToken);
        let outbox = Arc::new(PanicsWhenRecording {
            link: Link::new(&ports),
            panics: 2,
            recordings: std::sync::atomic::AtomicUsize::new(0),
        });
        let begun = tokio::time::Instant::now();
        let out = serde_json::json!({"@type": "ContractAgreementMessage"});
        deliver(Arc::clone(&outbox), out).await;
        // sent and recorded three times: two panics, then it worked
        let recordings = outbox.recordings.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(recordings, 3);
        // pauses of 5 and 10 seconds
        assert_eq!(begun.elapsed(), Duration::from_secs(15));
    }
}
