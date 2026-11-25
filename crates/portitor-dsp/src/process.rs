// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The shared core of the four process services (TD-42): negotiation and
//! transfer, each as provider ([`negotiation::provider`],
//! [`transfer::provider`]) and as consumer ([`negotiation::consumer`],
//! [`transfer::consumer`]).
//!
//! The four run the same steps. For a message from the counter-party:
//!
//! 1. read the process record from the store ([`update`], through the
//!    service's [`Records`]);
//! 2. check the message (the service's own rules), then apply its
//!    transition with [`receive`]. A message that fits only the state our
//!    pending message leads to proves that message arrived, so both are
//!    applied (ADR 0007);
//! 3. ask the service's decider for our next message and keep it as
//!    `pending` (the outbox), in the same write as the new state;
//! 4. write the record only if nobody wrote it since it was read (a
//!    compare-and-update on its `version`); otherwise start again at 1, a
//!    few times ([`WRITE_ATTEMPTS`]).
//!
//! For our own pending message, the adapter sends it and reports back:
//! once the counter-party acknowledges it, the service applies it with
//! [`apply_sent`] (`delivered`); if it cannot be delivered, the service
//! ends the process (`abort`), but only while the message is still
//! pending ([`is_pending`], [`can_abort`]).
//!
//! What differs stays in each service: its record and message types, its
//! checks, its decider and the messages it builds. The states and
//! transitions come from [`ProcessState`]: `next_state` in
//! `negotiation.rs` and `transfer.rs`.
//!
//! [`negotiation::provider`]: crate::negotiation::provider
//! [`negotiation::consumer`]: crate::negotiation::consumer
//! [`transfer::provider`]: crate::transfer::provider
//! [`transfer::consumer`]: crate::transfer::consumer

use crate::{ProcessState, Role, TransitionError};

/// How many times a service writes a process record when other writes
/// keep coming first (compare-and-update on its version, ADR 0007).
pub(crate) const WRITE_ATTEMPTS: usize = 3;

/// A store failure (I/O, corruption); the request cannot be served. The
/// stores of all four services report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "negotiation store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

/// Values the services need from outside the domain.
pub trait Environment {
    /// A new, unique identifier (for example `urn:uuid:...`).
    fn new_id(&self) -> String;
    /// The current time as XSD dateTime in UTC.
    fn now(&self) -> String;
}

/// The transition type of record `R`'s process.
pub(crate) type TransitionOf<R> = <<R as Record>::State as ProcessState>::Transition;

/// Why a message cannot move record `R`'s process.
pub(crate) type TransitionErrorOf<R> =
    TransitionError<<R as Record>::State, <<R as Record>::State as ProcessState>::Transition>;

/// Our message to the counter-party, as the core sees it: which transition
/// it causes once the counter-party has it, and whether it is the
/// consumer's first request.
pub(crate) trait Message<T> {
    /// The transition this message causes once the counter-party has it.
    fn transition_caused(&self) -> T;

    /// Whether this is the consumer's first request. It created the state
    /// already, so its acknowledgement changes no state. A provider never
    /// sends one.
    fn is_first_request(&self) -> bool {
        false
    }
}

/// One process as one party keeps it: `Negotiation`, `ConsumerNegotiation`,
/// `Transfer` or `ConsumerTransfer`. Each has a state, our message on its
/// way (`pending`), and a `version` that changes on every write. The
/// methods only read and write those fields, so the core can work on any
/// of the four.
pub(crate) trait Record: Clone {
    /// `NegotiationState` or `TransferState`.
    type State: ProcessState;
    /// Our message to the counter-party (the module's `Outgoing`).
    type Outgoing: Clone + PartialEq + Message<<Self::State as ProcessState>::Transition>;
    /// The party that keeps the record. It sends `Outgoing`; every
    /// message it receives comes from the other party.
    const PARTY: Role;

    /// The record's state, our message on its way (if any) and the version
    /// it was read with. The read-only methods below are built on it.
    fn parts(&self) -> (Self::State, Option<&Self::Outgoing>, u64);
    /// The same three fields, to change them.
    fn parts_mut(&mut self) -> (&mut Self::State, &mut Option<Self::Outgoing>, &mut u64);

    /// The current state.
    fn state(&self) -> Self::State {
        self.parts().0
    }
    /// Sets the current state.
    fn set_state(&mut self, state: Self::State) {
        *self.parts_mut().0 = state;
    }
    /// Our message on its way, if any.
    fn pending(&self) -> Option<&Self::Outgoing> {
        self.parts().1
    }
    /// Sets (or clears) our message on its way.
    fn set_pending(&mut self, pending: Option<Self::Outgoing>) {
        *self.parts_mut().1 = pending;
    }
    /// The version the record was read with.
    fn version(&self) -> u64 {
        self.parts().2
    }
    /// Sets the version to write.
    fn set_version(&mut self, version: u64) {
        *self.parts_mut().2 = version;
    }

    /// Applies our message `sent`, which the counter-party has (it
    /// acknowledged it, or its next message proved it). Most records keep
    /// the default, [`apply_sent`]; the provider's negotiation also keeps
    /// the agreement or offer it sent.
    ///
    /// # Errors
    ///
    /// The state machine does not allow `sent` in the current state.
    fn apply_sent(&mut self, sent: &Self::Outgoing) -> Result<(), TransitionErrorOf<Self>> {
        apply_sent(self, sent)
    }
}

/// Applies our message `sent` to `record`, the part every record shares:
/// the state moves on (not for the first request, which created it), and
/// nothing is pending any more.
///
/// # Errors
///
/// The state machine does not allow `sent` in the current state.
pub(crate) fn apply_sent<R: Record>(
    record: &mut R,
    sent: &R::Outgoing,
) -> Result<(), TransitionErrorOf<R>> {
    if !sent.is_first_request() {
        let state = R::State::next_state(Some(record.state()), sent.transition_caused(), R::PARTY)?;
        record.set_state(state);
    }
    record.set_pending(None);
    Ok(())
}

/// Why [`receive`] refused a message of the counter-party. Each service
/// turns it into its own `Failure`.
#[derive(Debug)]
pub(crate) enum Refused<R: Record> {
    /// Our termination is on its way; only a termination is taken now.
    Ending,
    /// The state machine does not allow the message.
    NotAllowed(TransitionErrorOf<R>),
}

/// Applies the counter-party's message, which causes `transition`, to
/// `record`.
///
/// - Any message of the provider proves that our first request arrived:
///   it is no longer pending.
/// - While our termination is on its way, only a termination is taken.
/// - If the message does not fit the current state but fits the state our
///   pending message leads to, the counter-party has received that
///   message (it could not have sent this one otherwise), so both are
///   applied (proof of delivery, ADR 0007).
///
/// # Errors
///
/// The message is refused; `record` may have changed then, and must not
/// be written.
pub(crate) fn receive<R: Record>(
    record: &mut R,
    transition: TransitionOf<R>,
) -> Result<(), Refused<R>> {
    let terminate = R::State::TERMINATE;
    let (first_request, ending) = match record.pending() {
        Some(sent) => (
            sent.is_first_request(),
            sent.transition_caused() == terminate,
        ),
        None => (false, false),
    };
    if first_request {
        record.set_pending(None);
    } else if ending && transition != terminate {
        return Err(Refused::Ending);
    }
    let sender = other_party(R::PARTY);
    let error = match R::State::next_state(Some(record.state()), transition, sender) {
        Ok(state) => {
            record.set_state(state);
            return Ok(());
        }
        Err(error) => error,
    };
    // our pending message may have arrived without its acknowledgement
    // being recorded yet: if the counter-party's message fits the state
    // that message leads to, it is proof of delivery (ADR 0007), and both
    // are applied together
    if let Some(sent) = record.pending().cloned() {
        let mut proved = record.clone();
        if proved.apply_sent(&sent).is_ok()
            && let Ok(state) = R::State::next_state(Some(proved.state()), transition, sender)
        {
            proved.set_state(state);
            *record = proved;
            return Ok(());
        }
    }
    Err(Refused::NotAllowed(error))
}

/// The other party of a process.
fn other_party(party: Role) -> Role {
    match party {
        Role::Provider => Role::Consumer,
        Role::Consumer => Role::Provider,
    }
}

/// Whether `sent` is still the message on its way for `record`. When it
/// is not, the counter-party's next message already proved it arrived, or
/// the process moved on, and a report about `sent` changes nothing.
pub(crate) fn is_pending<R: Record>(record: &R, sent: &R::Outgoing) -> bool {
    record.pending() == Some(sent)
}

/// Whether the process may still be ended because `sent` could not be
/// delivered: it has not ended, and `sent` is still pending.
pub(crate) fn can_abort<R: Record>(record: &R, sent: &R::Outgoing) -> bool {
    !record.state().is_terminal() && is_pending(record, sent)
}

/// Our messages still to send, one per record that has one (for example
/// after a restart).
pub(crate) fn pending_messages<R: Record>(records: Vec<R>) -> Vec<R::Outgoing> {
    let mut messages = Vec::new();
    for record in records {
        if let Some(message) = record.pending() {
            messages.push(message.clone());
        }
    }
    messages
}

/// What a change made by [`update`] asks it to write.
pub(crate) enum Write {
    /// Nothing changed; write nothing.
    Skip,
    /// Write the changed record.
    Save,
}

/// Which record [`update`] changes, and on whose behalf.
pub(crate) struct Lookup<'a> {
    /// Our process id: the record's key.
    pub(crate) pid: &'a str,
    /// The counter-party's process id as its message names it, for the
    /// error answer; empty when there is none.
    pub(crate) named_pid: &'a str,
    /// The participant whose message this is. Only its own process is
    /// found: another's looks like a missing one (DSP: "if the client is
    /// not authorized ... MUST return 404"). `None` for our own steps.
    pub(crate) caller: Option<&'a str>,
}

impl<'a> Lookup<'a> {
    /// The record `pid`, for one of our own steps (an acknowledgement, a
    /// message that could not be delivered, a management call).
    pub(crate) fn own(pid: &'a str) -> Self {
        Self {
            pid,
            named_pid: "",
            caller: None,
        }
    }

    /// The record `pid`, for a message of `caller` that names the
    /// counter-party's process `named_pid`.
    pub(crate) fn from_caller(pid: &'a str, named_pid: &'a str, caller: &'a str) -> Self {
        Self {
            pid,
            named_pid,
            caller: Some(caller),
        }
    }
}

/// What [`update`] needs from a service: how it reads and writes its
/// records, and the answers it gives when that fails. Each of the four
/// services implements it on itself, over its store port.
pub(crate) trait Records {
    /// The service's record.
    type Record: Record;
    /// The service's failure (its module's `Failure`).
    type Failure;

    /// The record `lookup` names, as stored now.
    ///
    /// # Errors
    ///
    /// "Not found" for an unknown record, or one of another participant
    /// than `lookup.caller`; or the store failed.
    fn read(&self, lookup: &Lookup<'_>) -> Result<Self::Record, Self::Failure>;

    /// Writes `record` only if the stored version is still `expected`.
    /// Returns `false` when another write came first.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn replace_if(&self, record: &Self::Record, expected: u64) -> Result<bool, Self::Failure>;

    /// The answer when other writes kept coming first.
    fn keeps_changing(&self, lookup: &Lookup<'_>) -> Self::Failure;

    /// A last change before each write. Most services need none; the
    /// consumer's transfer drops its access token once it has ended.
    fn before_write(&self, _record: &mut Self::Record) {}
}

/// Reads a record, changes it with `change` and writes it if its version
/// is unchanged; otherwise reads it again and repeats, a few times.
/// Returns what was written, or `None` when `change` said
/// [`Write::Skip`].
///
/// # Errors
///
/// The failure of `records.read` or of `change`, a store failure, or
/// `records.keeps_changing` after [`WRITE_ATTEMPTS`] tries.
pub(crate) fn update<S: Records>(
    records: &S,
    lookup: &Lookup<'_>,
    change: impl Fn(&mut S::Record) -> Result<Write, S::Failure>,
) -> Result<Option<S::Record>, S::Failure> {
    for _ in 0..WRITE_ATTEMPTS {
        let mut record = records.read(lookup)?;
        match change(&mut record)? {
            Write::Skip => return Ok(None),
            Write::Save => {}
        }
        records.before_write(&mut record);
        // compare-and-update: written only if nobody wrote in between
        let expected = record.version();
        record.set_version(expected.wrapping_add(1));
        if records.replace_if(&record, expected)? {
            return Ok(Some(record));
        }
    }
    Err(records.keeps_changing(lookup))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
