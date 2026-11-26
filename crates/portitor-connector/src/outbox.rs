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
//! Recording that a message arrived may find the provider's transfer
//! leased by another call that changes it (TD-48): the recording is tried
//! again after a short wait ([`retry_while_busy`]), not the sending.
//!
//! A panic (a bug) while a message is sent or recorded does not lose the
//! message: the same message is sent again after a pause
//! ([`send_until_counter_party_acts`]). That is safe for the reason a
//! resend after a restart is: a message is recorded only while it is
//! still the process's pending message (ADR 0007).
//!
//! Several processes may share one database (ADR 0029, TD-53). Before it
//! sends a saved message, a process takes a lease on that message in the
//! database (`portitor_runtime::leases`: kind [`lease_kind`], id
//! [`lease_id`]), renews it while it sends, and ends it once what became
//! of the message is recorded. A message leased by another process, or
//! by another task of this one, is left to the holder. The lease is on
//! the message, not on its process: a counter-party's message that
//! arrives while ours is still on its way may save our next message
//! (ADR 0007), which then goes out at once.
//!
//! Every process runs a loop ([`resume`], called every
//! [`crate::OUTBOX_LOOP_EVERY`]) that takes up the pending messages that
//! nobody leases: one whose recording failed (its lease is left to end,
//! so it is sent again about 30 seconds later, not at once and over and
//! over), and those of a process that stopped, once their leases have
//! ended. As Eclipse EDC's state machines do with `nextNotLeased`.

use std::sync::Arc;
use std::time::Duration;

use portitor_dsp::guard::{Direction, MessageGuard};
use portitor_dsp::policy::CredentialTypes;
use portitor_dsp::transfer::provider::LEASE_SECONDS;
use portitor_runtime::leases::HeldLease;
use portitor_runtime::panics::{catch_panic, next_pause};
use portitor_runtime::telemetry::metrics;
use serde_json::Value;
use tracing::Instrument as _;

use crate::auth::Prover;
use crate::http_client::{Sender, answered_status, asks_to_try_later};
use crate::ports::{Ports, Retry};

/// What every adapter needs to send: the client, the source of our
/// tokens, the dataspace's message rules, the retry policy, the
/// credential types policies may use (for the scopes of our tokens, and
/// of the credentials we ask a caller for; ADR 0019), and the leases in
/// the database (TD-53).
pub(crate) struct Link {
    pub(crate) sender: Arc<dyn Sender>,
    pub(crate) prover: Arc<dyn Prover>,
    pub(crate) guard: Arc<dyn MessageGuard + Send + Sync>,
    pub(crate) retry: Retry,
    pub(crate) credential_types: CredentialTypes,
    pub(crate) leases: portitor_runtime::leases::Leases,
}

impl Link {
    pub(crate) fn new(ports: &Ports) -> Self {
        Self {
            sender: Arc::clone(&ports.sender),
            prover: Arc::clone(&ports.prover),
            guard: Arc::clone(&ports.guard),
            retry: ports.retry,
            credential_types: ports.credential_types.clone(),
            leases: ports.leases.clone(),
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
    /// Why the store could not record it. `Send`: it is kept across the
    /// wait of [`retry_while_busy`], which may go on on another thread.
    type Failure: std::fmt::Debug + Send;

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

    /// Whether the process took `out` as delivered although the outbox
    /// could not deliver it: something else proved that it arrived. Such
    /// a message is not counted as given up ([`count_if_given_up`]).
    ///
    /// Only the negotiation provider has such a case today: a transfer
    /// request that settled a finalization event on its way (ADR 0007,
    /// amended in 0.13; TD-44). So the default is `false`, and it does
    /// not look at the message (`_out`).
    fn settled_without_delivery(&self, _out: &Self::Out) -> bool {
        false
    }

    /// Whether `failure` only says that another call is changing the
    /// process now, so recording again a moment later may work. Only the
    /// provider's transfer is leased (TD-48), so the default is `false`.
    fn is_busy(_failure: &Self::Failure) -> bool {
        false
    }
}

/// The longest [`retry_while_busy`] waits in all: a little more than one
/// lease, so that a lease left by a process that stopped has ended by the
/// last try, and that try takes its step over (`portitor-dsp`,
/// `transfer::provider::flow_steps`).
pub(crate) const BUSY_WAIT_LIMIT: Duration = Duration::from_secs(LEASE_SECONDS + 5);

/// Calls `call` again while it fails with a failure that `is_busy` says
/// is only "busy" (another call holds the transfer's lease, TD-48),
/// waiting in between: 100 ms, then twice as long each time, at most 5
/// seconds, until [`BUSY_WAIT_LIMIT`] has passed. Returns the last answer.
/// For our own calls in the background (the outbox, the end of a push);
/// a partner's request is answered 503 at once instead.
///
/// The wait is `tokio::time::sleep`: it frees the thread for other tasks
/// (a `std::thread::sleep` would block it).
pub(crate) async fn retry_while_busy<T, F>(
    is_busy: impl Fn(&F) -> bool,
    mut call: impl FnMut() -> Result<T, F>,
) -> Result<T, F> {
    let mut wait = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    loop {
        let answer = call();
        let try_again = match &answer {
            Err(failure) => is_busy(failure) && waited < BUSY_WAIT_LIMIT,
            Ok(_) => false,
        };
        if !try_again {
            return answer;
        }
        tokio::time::sleep(wait).await;
        waited += wait;
        wait = wait.saturating_mul(2).min(Duration::from_secs(5));
    }
}

/// The kind of the leases of `kind`'s messages in the database, for
/// example `outbox:transfer:provider` (module doc).
pub(crate) fn lease_kind(kind: metrics::Outbox) -> String {
    match kind.role_name() {
        Some(role) => format!("outbox:{}:{role}", kind.process_name()),
        None => format!("outbox:{}", kind.process_name()),
    }
}

/// The id of the lease on one saved message: our process id, then the
/// SHA-256 of where it goes and what it says. The message saved by a
/// request and the same message read back from the database get the same
/// id, so the loop and the request's own task never both send it.
pub(crate) fn lease_id(msg: &Message<'_>) -> String {
    let what = format!("{}\n{}", msg.url, msg.body);
    let digest = ring::digest::digest(&ring::digest::SHA256, what.as_bytes());
    format!("{} {}", msg.pid, crate::system::hex(digest.as_ref()))
}

/// Sends in the background each message in `pending` whose process
/// nobody leases now: the loop of every process (module doc), and the
/// first round at start. Returns how many it took up. A message it takes
/// up is counted as sent again (`portitor.outbox.message.resent`) once
/// its lease is taken.
pub(crate) fn resume<O: Outbox>(outbox: &Arc<O>, pending: Vec<O::Out>) -> usize {
    let held = outbox.link().leases.held(&lease_kind(outbox.kind()));
    let mut count = 0;
    for out in pending {
        if held.contains(&lease_id(&O::message(&out))) {
            // being sent: by another process, or by a task of this one
            continue;
        }
        count += 1;
        tokio::spawn(deliver_taken_up(Arc::clone(outbox), out, true));
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
///
/// Each message is sent only when this task can take the lease on it
/// (module doc); otherwise its holder sends it.
pub(crate) fn deliver<O: Outbox>(outbox: Arc<O>, out: O::Out) -> impl Future<Output = ()> {
    deliver_taken_up(outbox, out, false)
}

/// [`deliver`]; `taken_up` when the loop found the message ([`resume`]):
/// it is then counted as sent again.
fn deliver_taken_up<O: Outbox>(
    outbox: Arc<O>,
    out: O::Out,
    taken_up: bool,
) -> impl Future<Output = ()> {
    let kind = outbox.kind();
    // `role_name()` is `None` for an outbox without a DSP role: the span
    // then has no `portitor.dsp.role` field
    let span = tracing::info_span!(
        "dsp.outbox.deliver",
        portitor.dsp.process = kind.process_name(),
        portitor.dsp.role = kind.role_name(),
        portitor.dsp.pid = O::message(&out).pid,
    );
    // the first lease is taken now, in the request, so that a message
    // the request saved is never taken up by the loop before this task
    // runs
    let first = claim(outbox.as_ref(), &out);
    send_until_counter_party_acts(outbox, out, first, taken_up).instrument(span)
}

/// The lease on `out` for this process, if nobody holds it.
fn claim<O: Outbox>(outbox: &O, out: &O::Out) -> Option<HeldLease> {
    let msg = O::message(out);
    let lease = outbox
        .link()
        .leases
        .claim(&lease_kind(outbox.kind()), &lease_id(&msg));
    if lease.is_none() {
        tracing::debug!(pid = msg.pid, "message left to the process that leases it");
    }
    lease
}

/// The work of [`deliver`]: sends `out`, under its lease `first`, then
/// each message that follows from the answer, each under its own lease,
/// until it is the counter-party's turn, the process ends, or another
/// process holds the next message. `taken_up`: `out` was found by the
/// loop, and is counted as sent again.
///
/// A lease ends once what became of its message is recorded. When that
/// could not be recorded (the store failed), the lease is left to end on
/// its own: the message is still pending, and the loop sends it again
/// after that pause.
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
async fn send_until_counter_party_acts<O: Outbox>(
    outbox: Arc<O>,
    mut out: O::Out,
    first: Option<HeldLease>,
    taken_up: bool,
) {
    let Some(mut lease) = first else {
        return;
    };
    if taken_up {
        // a message saved before a stop, or left by another process
        metrics::outbox_resent(outbox.kind());
    }
    loop {
        match send_until_recorded(&outbox, &out).await {
            Step::Next(next) => {
                lease.release();
                let Some(next_lease) = claim(outbox.as_ref(), &next) else {
                    return;
                };
                (out, lease) = (next, next_lease);
            }
            Step::Done => {
                lease.release();
                return;
            }
            // dropped, not released: see the doc above
            Step::NotRecorded => return,
        }
    }
}

/// Sends `out` and records what became of it, again after a pause when
/// that panics (the doc of [`send_until_counter_party_acts`]).
async fn send_until_recorded<O: Outbox>(outbox: &Arc<O>, out: &O::Out) -> Step<O::Out> {
    // the pause before the current try of `out`; none before the first
    let mut pause: Option<Duration> = None;
    loop {
        let started = tokio::time::Instant::now();
        // `out` is only lent to the step, so after a panic this loop
        // still has it and can send it again
        if let Ok(step) = catch_panic(send_and_record(outbox.as_ref(), out)).await {
            return step;
        }
        // it panicked
        let wait = next_pause(pause, started.elapsed());
        tracing::error!(
            task = "outbox_delivery",
            pid = O::message(out).pid,
            again_in_seconds = wait.as_secs(),
            "a background task panicked; its message is sent again"
        );
        metrics::task_panicked("outbox_delivery");
        metrics::outbox_resent(outbox.kind());
        tokio::time::sleep(wait).await;
        pause = Some(wait);
    }
}

/// What one step of [`send_until_counter_party_acts`] left.
enum Step<T> {
    /// The message that follows, to send now.
    Next(T),
    /// It is the counter-party's turn, or the process ended.
    Done,
    /// What became of the message could not be recorded: it is still
    /// pending.
    NotRecorded,
}

/// One step of [`send_until_counter_party_acts`]: sends `out` and records
/// what became of it.
async fn send_and_record<O: Outbox>(outbox: &O, out: &O::Out) -> Step<O::Out> {
    let msg = O::message(out);
    let (pid, first) = (msg.pid.to_owned(), msg.first);
    let scopes = outbox.scopes(out);
    let reached = |state| O::reached(out, state);
    let outcome = attempt(outbox.link(), outbox.kind(), &msg, &scopes, reached).await;
    let undelivered = outcome.undelivered();
    let record = || match &outcome {
        Outcome::Acknowledged(answer) => outbox.delivered(out, Some(answer)),
        Outcome::ArrivedBefore | Outcome::TerminationUnanswered => outbox.delivered(out, None),
        Outcome::NotStarted => outbox.not_started(out).map(|()| None),
        Outcome::Unreachable => outbox.abort(out),
    };
    // only the recording is tried again, never the sending
    let recorded = retry_while_busy(O::is_busy, record).await;
    match recorded {
        Ok(next) => {
            if undelivered {
                count_if_given_up(outbox, out);
            }
            match next {
                Some(next) => Step::Next(next),
                None => Step::Done,
            }
        }
        Err(failure) => {
            // Not counted as given up: the outbox has not given it up.
            // The record was not written (in practice: the store failed),
            // so the message is still saved as pending and is sent again
            // once its lease has ended ([`resume`]). It may then arrive, or be
            // settled by a transfer request, or fail again and be
            // counted. Counting it now too would count one message
            // twice, or count one that is delivered in the end. The
            // failure itself is in the log line below.
            tracing::warn!(%pid, ?failure, "outgoing message not recorded");
            if first {
                // without the counter-party's pid the process cannot go on
                let _ = outbox.not_started(out);
            }
            Step::NotRecorded
        }
    }
}

/// Counts `out`, which the outbox could not deliver and will not send
/// again, as given up (`portitor.outbox.message.given_up`), unless the
/// process took it as delivered for another reason
/// ([`Outbox::settled_without_delivery`]).
///
/// Call it only after the outcome was recorded. For the negotiation
/// provider, `abort` and a transfer request that settles the negotiation
/// write the same record with a version check, so once `abort` has
/// returned `Ok` one of the two has won, and the outbox's answer does
/// not change any more.
fn count_if_given_up<O: Outbox>(outbox: &O, out: &O::Out) {
    if outbox.settled_without_delivery(out) {
        tracing::info!(
            pid = O::message(out).pid,
            "message not delivered, but the process was settled without it; not counted as given up"
        );
    } else {
        metrics::outbox_given_up(outbox.kind());
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

impl Outcome {
    /// Whether the message did not arrive and will not be sent again.
    fn undelivered(&self) -> bool {
        match self {
            Outcome::Acknowledged(_) | Outcome::ArrivedBefore => false,
            Outcome::NotStarted | Outcome::TerminationUnanswered | Outcome::Unreachable => true,
        }
    }
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
        let mut ports = Ports::in_memory(sender);
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
        let mut ports = Ports::in_memory(sender);
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

    /// An outbox whose messages cannot be delivered. `settled` is what it
    /// answers when asked whether the process was settled without the
    /// message; `calls` lists what the outbox was asked, in order. With
    /// `store_fails`, recording the outcome fails.
    struct Undeliverable {
        link: Link,
        settled: bool,
        store_fails: bool,
        calls: Mutex<Vec<&'static str>>,
    }

    /// A link to a counter-party that answers nothing, tried once.
    fn link_to_nobody() -> Link {
        let mut ports = Ports::in_memory(Arc::new(Down));
        ports.prover = Arc::new(FixedToken);
        ports.retry = Retry {
            attempts: 1,
            first_delay: Duration::from_millis(1),
        };
        Link::new(&ports)
    }

    impl Undeliverable {
        fn new(settled: bool) -> Self {
            Self {
                link: link_to_nobody(),
                settled,
                store_fails: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Notes the call `name` and answers as the store does.
        fn record(&self, name: &'static str) -> Result<Option<Value>, String> {
            self.calls.lock().unwrap().push(name);
            if self.store_fails {
                Err("the store is busy".to_owned())
            } else {
                Ok(None)
            }
        }
    }

    /// The message of the test outboxes. Its kind is read from the
    /// body's `@type`: a request is a first message, a termination is a
    /// termination, anything else is a message in the middle.
    fn test_message(out: &Value) -> Message<'_> {
        let kind = out["@type"].as_str().unwrap_or_default();
        Message::new("urn:uuid:1", "https://c.example/x", out, "", "did:web:c")
            .first(kind == "ContractRequestMessage")
            .terminating(kind == "ContractNegotiationTerminationMessage")
    }

    impl Outbox for Undeliverable {
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
            test_message(out)
        }

        fn reached(_: &Value, _: Value) -> bool {
            false
        }

        fn delivered(&self, _: &Value, _: Option<&Value>) -> Result<Option<Value>, String> {
            self.record("delivered")
        }

        fn not_started(&self, _: &Value) -> Result<(), String> {
            self.record("not started").map(|_| ())
        }

        fn abort(&self, _: &Value) -> Result<Option<Value>, String> {
            self.record("abort")
        }

        fn settled_without_delivery(&self, _: &Value) -> bool {
            self.calls.lock().unwrap().push("settled?");
            self.settled
        }
    }

    /// The messages counted as given up on this test's thread
    /// (`portitor.outbox.message.given_up`), one entry per message.
    fn counted_as_given_up() -> Vec<metrics::Outbox> {
        portitor_runtime::telemetry::testing::given_up_on_this_thread()
    }

    const PROVIDER_NEGOTIATION: metrics::Outbox =
        metrics::Outbox::Negotiation(metrics::Role::Provider);

    /// ADR 0007, amended in 0.13 (TD-44): a finalization event that could
    /// not be delivered, in a negotiation that a transfer request
    /// settled, is not counted as given up.
    #[tokio::test]
    async fn an_undelivered_message_of_a_settled_process_is_not_counted_as_given_up() {
        let event = serde_json::json!({"@type": "ContractNegotiationEventMessage"});
        let outbox = Undeliverable::new(true);
        assert!(matches!(send_and_record(&outbox, &event).await, Step::Done));
        // the failure is recorded first (`abort`, which changes nothing
        // for a settled negotiation), and the outbox is asked afterwards
        assert_eq!(*outbox.calls.lock().unwrap(), ["abort", "settled?"]);
        assert!(counted_as_given_up().is_empty());
    }

    /// Any other message that could not be delivered is counted, once:
    /// one that the counter-party did not take in the middle of a
    /// process, a first message that started nothing, and a termination
    /// that got no answer.
    #[tokio::test]
    async fn an_undelivered_message_is_counted_as_given_up_whatever_its_kind() {
        for (message_type, recorded_as) in [
            ("ContractNegotiationEventMessage", "abort"),
            ("ContractRequestMessage", "not started"),
            ("ContractNegotiationTerminationMessage", "delivered"),
        ] {
            let before = counted_as_given_up().len();
            let message = serde_json::json!({"@type": message_type});
            let outbox = Undeliverable::new(false);
            assert!(matches!(
                send_and_record(&outbox, &message).await,
                Step::Done
            ));
            assert_eq!(
                *outbox.calls.lock().unwrap(),
                [recorded_as, "settled?"],
                "{message_type}"
            );
            let counted = counted_as_given_up();
            assert_eq!(counted.len(), before + 1, "{message_type}");
            assert_eq!(counted.last(), Some(&PROVIDER_NEGOTIATION));
        }
    }

    /// A message whose outcome the store did not take is not given up:
    /// it is still saved as pending and is sent again after a restart.
    /// It is not counted now, and the outbox is not asked about it.
    #[tokio::test]
    async fn a_message_that_could_not_be_recorded_is_not_counted_as_given_up() {
        let event = serde_json::json!({"@type": "ContractNegotiationEventMessage"});
        let mut outbox = Undeliverable::new(false);
        outbox.store_fails = true;
        let step = send_and_record(&outbox, &event).await;
        assert!(matches!(step, Step::NotRecorded));
        assert_eq!(*outbox.calls.lock().unwrap(), ["abort"]);
        assert!(counted_as_given_up().is_empty());
    }

    /// An outbox that says nothing about settling: every method that has
    /// a default keeps it, as the consumer's and the transfer outboxes
    /// do.
    struct Plain {
        link: Link,
    }

    impl Outbox for Plain {
        /// The message's body.
        type Out = Value;
        type Failure = String;

        fn link(&self) -> &Link {
            &self.link
        }

        fn kind(&self) -> metrics::Outbox {
            metrics::Outbox::Transfer(metrics::Role::Consumer)
        }

        fn message(out: &Value) -> Message<'_> {
            test_message(out)
        }

        fn reached(_: &Value, _: Value) -> bool {
            false
        }

        fn delivered(&self, _: &Value, _: Option<&Value>) -> Result<Option<Value>, String> {
            Ok(None)
        }

        fn abort(&self, _: &Value) -> Result<Option<Value>, String> {
            Ok(None)
        }
    }

    /// Only the negotiation provider has a case of "settled without
    /// delivery": by default an outbox never says so, and its
    /// undelivered message is counted.
    #[tokio::test]
    async fn an_outbox_never_says_settled_by_default() {
        let outbox = Plain {
            link: link_to_nobody(),
        };
        let message = serde_json::json!({"@type": "TransferCompletionMessage"});
        assert!(!outbox.settled_without_delivery(&message));
        assert!(matches!(
            send_and_record(&outbox, &message).await,
            Step::Done
        ));
        assert_eq!(
            counted_as_given_up(),
            [metrics::Outbox::Transfer(metrics::Role::Consumer)]
        );
    }

    /// A message that was delivered is never asked about: there is
    /// nothing to count.
    #[tokio::test]
    async fn a_delivered_message_is_not_asked_about() {
        let mut outbox = Undeliverable::new(false);
        let mut ports = Ports::in_memory(Arc::new(Accepts));
        ports.prover = Arc::new(FixedToken);
        outbox.link = Link::new(&ports);
        let event = serde_json::json!({"@type": "ContractNegotiationEventMessage"});
        assert!(matches!(send_and_record(&outbox, &event).await, Step::Done));
        assert_eq!(*outbox.calls.lock().unwrap(), ["delivered"]);
        assert!(counted_as_given_up().is_empty());
    }

    #[test]
    fn only_a_message_that_did_not_arrive_can_be_given_up() {
        assert!(!Outcome::Acknowledged(Value::Null).undelivered());
        assert!(!Outcome::ArrivedBefore.undelivered());
        assert!(Outcome::NotStarted.undelivered());
        assert!(Outcome::TerminationUnanswered.undelivered());
        assert!(Outcome::Unreachable.undelivered());
    }

    /// A paused clock: the pauses pass at once, and the test reads how
    /// long they were.
    #[tokio::test(start_paused = true)]
    async fn a_message_whose_delivery_panics_is_sent_again_after_a_pause() {
        let mut ports = Ports::in_memory(Arc::new(Accepts));
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

    /// A call refused as busy (another call holds the transfer's lease,
    /// TD-48) is made again after 100 ms, then twice as long each time,
    /// and its first other answer is the one returned.
    #[tokio::test(start_paused = true)]
    async fn a_busy_call_is_made_again_until_it_is_not_busy() {
        let begun = tokio::time::Instant::now();
        let mut calls = 0;
        let answer: Result<&str, &str> = retry_while_busy(
            |failure: &&str| *failure == "busy",
            || {
                calls += 1;
                if calls < 4 { Err("busy") } else { Ok("done") }
            },
        )
        .await;
        assert_eq!(answer, Ok("done"));
        assert_eq!(calls, 4);
        // pauses of 100, 200 and 400 ms
        assert_eq!(begun.elapsed(), Duration::from_millis(700));

        // another failure is returned at once
        let mut calls = 0;
        let answer: Result<(), &str> = retry_while_busy(
            |failure: &&str| *failure == "busy",
            || {
                calls += 1;
                Err("store down")
            },
        )
        .await;
        assert_eq!((answer, calls), (Err("store down"), 1));
    }

    /// A call that stays busy is given up once a little more than one
    /// lease has passed: by then a lease left by a process that stopped
    /// has ended, and the last try took its step over.
    #[tokio::test(start_paused = true)]
    async fn a_call_that_stays_busy_is_given_up_after_one_lease() {
        let begun = tokio::time::Instant::now();
        let answer: Result<(), &str> =
            retry_while_busy(|failure: &&str| *failure == "busy", || Err("busy")).await;
        assert_eq!(answer, Err("busy"));
        assert!(begun.elapsed() >= BUSY_WAIT_LIMIT);
        assert!(begun.elapsed() < BUSY_WAIT_LIMIT + Duration::from_secs(6));
        assert!(BUSY_WAIT_LIMIT > Duration::from_secs(LEASE_SECONDS));
    }
}
