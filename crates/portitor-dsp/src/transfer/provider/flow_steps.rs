// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The data plane steps of the provider's transfers: run after the
//! transfer's write, from what was written, once (ADR 0028, decision 6;
//! TD-48).
//!
//! Why. A transfer changes by reading it, changing it and writing it only
//! if no other write came first; otherwise the change runs again on the
//! new version ([`process::update`]). A call to the data plane inside that
//! change could run although its write then lost, or run twice.
//!
//! How, the way the outbox handles our DSP messages (ADR 0007):
//!
//! 1. The write that changes a transfer also records the data plane step
//!    the change needs, in the transfer's `flow_step` ([`FlowStep`]). A
//!    write that loses records nothing, so the data plane hears nothing.
//! 2. Right after the write, the same call runs the step that is recorded,
//!    then writes again to record that it ran: `flow_step` is cleared. A
//!    start (or a start again by the provider) gets its start message in
//!    that second write, with the data address the data plane made: it
//!    carries the consumer's token.
//! 3. Our message waits for the step of its change: it is sent only once
//!    `flow_step` is empty (`ready_to_send`). So the data plane is still
//!    told before the consumer (ADR 0013).
//! 4. A step still recorded after its call (the process stopped, or the
//!    recording failed) runs later, once its lease has ended (below); at a
//!    restart such steps run first, then the messages are sent
//!    (`TransferService::pending`). Running a step again is safe: the data plane takes a step to the state the flow is
//!    already in as done, and a start of a flow that exists gives it a new
//!    token (the first one was never sent).
//!
//! One call at a time, through the database. While a step is recorded
//! and has not run, the transfer is leased ([`Lease`]): the write that
//! records the step also takes the lease, in the same compare-and-update
//! (only if no step is recorded, so no lease is held), and the write that
//! records that the step ran empties both. The lease names its holder (an
//! id of the process, [`Environment::process_id`]) and when it ends,
//! [`LEASE_SECONDS`] later. Any other write of the transfer while the
//! lease holds is refused at once with [`Failure::Busy`] (HTTP 503 with
//! `Retry-After`): nothing waits, and no lock is held in the process. So
//! no other write comes between a write and its step, in this process or
//! in another one that shares the database: a step is never replaced
//! before it runs, the steps reach the data plane in the order of their
//! writes (a suspension never after the start again that followed it), a
//! start again makes one token, and a refusal goes to the call whose
//! write recorded the step. Eclipse EDC leases its entities the same way
//! (ADR 0007, note added in 1.2).
//!
//! A lease that has ended, with its step still recorded (its holder
//! stopped, or recording that the step ran failed), is taken over: the
//! next call that changes the transfer takes the lease and runs the step
//! first ([`TransferService::write_then_run_step`]); so do the periodic
//! sweep ([`TransferService::run_steps_left_over`]) and the start
//! ([`TransferService::pending`]). Taking over is itself a
//! compare-and-update, so only one call wins. Running a step twice is
//! safe (see 4). A step that outlives its lease (it never should: the
//! built-in data plane runs in this process, so a step is a function call
//! and a write to the database) may be taken over while it runs; the
//! write that records it then loses, and the call that took it over
//! records it.
//!
//! The data plane is called while no lock is held, so a remote data plane
//! (ADR 0028, decision 7, signaling over HTTP) changes nothing here but
//! the time a step takes, which must stay well under the lease.
//!
//! When the data plane refuses a step ([`IfRefused`]):
//!
//! - a step of the provider's own decision (the decider: start, suspend,
//!   complete, terminate), or one taken over after its lease ended, ends
//!   the transfer: our message becomes a termination that gives the
//!   reason, and the data flow is terminated, best effort (as before 1.2);
//! - a step of a consumer's message, of a message that could not be
//!   delivered (`abort`), of the provider's own termination
//!   (`terminate_own`) or of the end of a push takes its write back: the
//!   transfer is written again as it was before, and the caller gets
//!   [`Failure::DataPlane`] (as before 1.2, when nothing was written).
//!
//! The state change of a write that recorded a step is reported once the
//! step has run, from [`Transfer::state_before_step`], which the write
//! keeps in the database with the step: whichever process runs the step
//! can report it. A write taken back is not reported, as 1.1, which wrote
//! nothing, reported none (the connector's `AnyTransferStore`).
//!
//! When the step ran but recording that it ran failed (the store failed),
//! the recording is tried once more. If that fails too, the step stays
//! recorded with its lease, our message waits, and the caller gets
//! [`Failure::Store`] although its write landed. Once the lease has ended
//! the step runs again (which is safe, see 4), taken over as above. A
//! take-back that was due is not made then: if the data plane refuses at
//! that later run, the transfer ends instead.

use super::{Agreements, CatalogDatasets};
use super::{
    DataAddress, DataFlows, Environment, Failure, FlowStep, Lease, Lookup, Pending, Role,
    StoreError, Transfer, TransferDecider, TransferError, TransferService, TransferStore,
    Transition, Write, next_state, process, ready_to_send, start_message, termination,
};
use crate::process::Records;

/// How long a lease on a transfer holds, in seconds (module doc). Long
/// enough for any step of the built-in data plane, which runs in this
/// process (milliseconds); short enough that a step left by a process
/// that stopped runs again soon. Not a setting: settings are frozen
/// (`docs/STABILITY.md`), and nothing has needed another value.
pub const LEASE_SECONDS: u64 = 30;

/// The reason a write is refused with while the transfer is leased.
const BUSY: &str = "the transfer is being changed; retry";

/// What happens when the data plane refuses the step that a write
/// recorded (module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IfRefused {
    /// The transfer ends: our message becomes a termination, and the data
    /// flow is terminated (best effort).
    EndTheTransfer,
    /// The write is taken back: the transfer is written again as it was
    /// read before the write, and the caller gets [`Failure::DataPlane`].
    TakeBack,
}

/// A transfer through one write and the data plane step it recorded
/// ([`TransferService::write_then_run_step`]).
pub(super) struct Stepped {
    /// As it was read, before the write.
    pub(super) before: Transfer,
    /// As the write left it, before its step ran.
    pub(super) written: Transfer,
    /// As it is once its step ran and that was recorded.
    pub(super) current: Transfer,
}

impl<S, A, F, D, C, E> TransferService<S, A, F, D, C, E>
where
    S: TransferStore,
    A: Agreements,
    F: DataFlows,
    D: TransferDecider,
    C: CatalogDatasets,
    E: Environment,
{
    /// Changes the transfer `lookup` names with `change` and writes it
    /// ([`process::update_keeping_before`]), taking the lease when the
    /// write records a data plane step; then runs that step and records
    /// that it ran (module doc). A step left recorded with a lease that
    /// has ended runs first. `if_refused`: what a refusal of the data
    /// plane does. Returns `None` when `change` said [`Write::Skip`].
    ///
    /// # Errors
    ///
    /// [`Failure::Busy`] while another call holds the transfer's lease;
    /// the failure of `change`; [`Failure::DataPlane`] when the data plane
    /// refused a step that is taken back ([`IfRefused::TakeBack`]); a
    /// store failure; or the transfer kept changing.
    pub(super) fn write_then_run_step(
        &self,
        lookup: &Lookup<'_>,
        if_refused: IfRefused,
        change: impl Fn(&mut Transfer) -> Result<Write, Failure>,
    ) -> Result<Option<Stepped>, Failure> {
        self.run_step_left_over(lookup)?;
        // made once, so every try of the write below takes the same lease
        let lease = self.new_lease();
        let updated = process::update_keeping_before(self, lookup, |transfer| {
            if transfer.flow_step.is_some() {
                // another call recorded a step since the check above, and
                // holds the lease; this write is refused, not queued
                return Err(busy(transfer));
            }
            let state_before = transfer.state;
            let write = change(transfer)?;
            if transfer.flow_step.is_some() {
                // in the same write as the step: the lease, and the state
                // to report the change from once the step has run
                transfer.state_before_step = Some(state_before);
                transfer.lease = Some(lease.clone());
            }
            Ok(write)
        })?;
        let Some(updated) = updated else {
            return Ok(None);
        };
        let take_back_to = match if_refused {
            IfRefused::EndTheTransfer => None,
            IfRefused::TakeBack => Some(&updated.before),
        };
        let current = self.run_recorded_step(&updated.written, take_back_to)?;
        Ok(Some(Stepped {
            before: updated.before,
            written: updated.written,
            current,
        }))
    }

    /// Adds the new `transfer`, with the lease if it records a step (a
    /// start), then runs that step. A refusal ends the transfer. Returns
    /// the transfer once its step ran, or `None` when it was not added:
    /// its consumer already has a transfer with its `consumer_pid`.
    ///
    /// # Errors
    ///
    /// A store failure.
    pub(super) fn insert_then_run_step(
        &self,
        transfer: &Transfer,
    ) -> Result<Option<Transfer>, Failure> {
        let mut new = transfer.clone();
        if new.flow_step.is_some() {
            // a new transfer: no state before, the event says "new"
            new.state_before_step = None;
            new.lease = Some(self.new_lease());
        }
        let inserted = self.store.insert(&new).map_err(Failure::Store)?;
        if !inserted {
            return Ok(None);
        }
        let current = self.run_recorded_step(&new, None)?;
        Ok(Some(current))
    }

    /// The periodic sweep (module doc): runs every data plane step left
    /// recorded whose lease has ended, and returns the messages that wait
    /// for no step any more after that. A step whose lease still holds is
    /// left to its holder. The connector calls it every few seconds, so a
    /// step left by a process that stopped, or whose recording failed,
    /// runs soon after its lease ends even if no call comes for its
    /// transfer.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    pub fn run_steps_left_over(&self) -> Result<Pending, Failure> {
        let transfers = self.store.recorded_steps().map_err(Failure::Store)?;
        Ok(self.take_up(transfers))
    }

    /// Runs the recorded step of each of `transfers` whose lease has
    /// ended, and collects the messages that may be sent then
    /// ([`TransferService::pending`], [`TransferService::run_steps_left_over`]).
    pub(super) fn take_up(&self, transfers: Vec<Transfer>) -> Pending {
        let mut found = Pending::default();
        for transfer in transfers {
            let current = match self.take_over_and_run(&transfer) {
                Ok(current) => {
                    if transfer.flow_step.is_some() {
                        found.steps_run.push(transfer.provider_pid.clone());
                    }
                    current
                }
                // another call holds the lease: its message follows its
                // step, sent by that call or by a later sweep
                Err(Failure::Busy(_)) => continue,
                Err(failure) => {
                    let pid = transfer.provider_pid.clone();
                    found.not_taken_up.push((pid, failure));
                    continue;
                }
            };
            if let Some(message) = ready_to_send(&current) {
                found.messages.push(message);
            }
        }
        found
    }

    /// Before a call changes the transfer `lookup` names: runs the step an
    /// earlier call left recorded, if its lease has ended (module doc).
    /// It runs before the next write, so the steps reach the data plane in
    /// the order of their writes. A refusal ends the transfer.
    ///
    /// # Errors
    ///
    /// [`Failure::Busy`] while the lease holds; a store failure.
    fn run_step_left_over(&self, lookup: &Lookup<'_>) -> Result<(), Failure> {
        let transfer = self.read(lookup)?;
        self.take_over_and_run(&transfer)?;
        Ok(())
    }

    /// Takes the lease of `found`, as read, whose step is left recorded,
    /// if its lease has ended, and runs that step (module doc). Returns
    /// the transfer as it is then; `found` itself when it records no step.
    ///
    /// # Errors
    ///
    /// [`Failure::Busy`] while the lease holds, or when another call took
    /// it first or changed the transfer; a store failure.
    fn take_over_and_run(&self, found: &Transfer) -> Result<Transfer, Failure> {
        if found.flow_step.is_none() {
            return Ok(found.clone());
        }
        if self.lease_holds(found) {
            return Err(busy(found));
        }
        let mut taken = found.clone();
        taken.lease = Some(self.new_lease());
        // compare-and-update: of two calls that find the same ended
        // lease, only one takes it over
        let expected = found.version;
        taken.version = expected.wrapping_add(1);
        if !self.replace_if(&taken, expected)? {
            return Err(busy(found));
        }
        self.run_recorded_step(&taken, None)
    }

    /// Whether the lease on `transfer` still holds: it has one, and its
    /// end has not come. A step recorded without a lease (written before
    /// leases existed) counts as ended.
    fn lease_holds(&self, transfer: &Transfer) -> bool {
        match &transfer.lease {
            Some(lease) => lease.until > self.env.now_seconds(),
            None => false,
        }
    }

    /// A lease for this process, from now for [`LEASE_SECONDS`].
    fn new_lease(&self) -> Lease {
        Lease {
            holder: self.env.process_id().to_owned(),
            until: self.env.now_seconds().saturating_add(LEASE_SECONDS),
        }
    }

    /// Runs the data plane step recorded on `written`, a transfer just
    /// written with its lease, if it has one, and records that it ran
    /// (module doc). `take_back_to`: the transfer as it was before that
    /// write, when a refusal takes the write back; `None` when a refusal
    /// ends the transfer. Returns the transfer as it is then.
    ///
    /// # Errors
    ///
    /// [`Failure::DataPlane`] when the data plane refused a step whose
    /// write is taken back; [`Failure::Store`] when the step ran but could
    /// not be recorded; [`Failure::Busy`] when the step outlived its lease
    /// and another call took it over.
    fn run_recorded_step(
        &self,
        written: &Transfer,
        take_back_to: Option<&Transfer>,
    ) -> Result<Transfer, Failure> {
        let Some(step) = written.flow_step else {
            return Ok(written.clone());
        };
        let answer = self.run_on_data_plane(step, written);
        let mut recorded = self.record_that_step_ran(written, step, &answer, take_back_to);
        if recorded.is_err() {
            // the step ran: try once more to record it, so that it is not
            // run again and our message does not wait (module doc)
            recorded = self.record_that_step_ran(written, step, &answer, take_back_to);
        }
        let (current, ended) = match recorded {
            Ok(Some(done)) => done,
            // the lease ended while the step ran, and another call took
            // the step over: that call records it
            Ok(None) => return Err(busy(written)),
            Err(failure) => {
                return Err(Failure::Store(StoreError(format!(
                    "the data plane step {} of transfer {} ran, but recording that it ran \
                     failed: {failure:?}; it stays recorded and runs again once its lease \
                     has ended, at the next change of the transfer, the next sweep or the \
                     next restart",
                    step.name(),
                    written.provider_pid
                ))));
            }
        };
        let Err(reason) = answer else {
            return Ok(current);
        };
        if ended {
            // end whatever the data plane still serves (best effort)
            let _ = self.flows.terminate(&current.provider_pid);
        }
        match take_back_to {
            None => Ok(current),
            Some(_) => Err(Failure::DataPlane(reason)),
        }
    }

    /// Writes that `step`, recorded on `written`, ran and gave `answer`:
    /// empties the step and the lease, and adds the start message after a
    /// start, takes the write back to `take_back_to` after a refusal, or
    /// ends the transfer after a refusal that cannot be taken back.
    /// Returns the transfer as written and whether it was ended that way,
    /// or `None` when the write lost: another call took the step over.
    fn record_that_step_ran(
        &self,
        written: &Transfer,
        step: FlowStep,
        answer: &Result<Option<DataAddress>, String>,
        take_back_to: Option<&Transfer>,
    ) -> Result<Option<(Transfer, bool)>, Failure> {
        let mut recorded = written.clone();
        recorded.flow_step = None;
        recorded.state_before_step = None;
        recorded.lease = None;
        let mut ended = false;
        match answer {
            Ok(address) => add_start_message(&mut recorded, step, address.clone()),
            Err(reason) => {
                if let Some(before) = take_back_to {
                    take_back(&mut recorded, before);
                } else {
                    end_after_refusal(&mut recorded, reason);
                    ended = true;
                }
            }
        }
        self.before_write(&mut recorded);
        // compare-and-update on the version `written` has: under the lease
        // no other call wrote since, unless the lease ended and another
        // call took the step over
        let expected = written.version;
        recorded.version = expected.wrapping_add(1);
        if self.replace_if(&recorded, expected)? {
            Ok(Some((recorded, ended)))
        } else {
            Ok(None)
        }
    }

    /// Runs `step` for `transfer` on the data plane. Returns the data
    /// address a start gives (for a pull transfer), else `None`.
    fn run_on_data_plane(
        &self,
        step: FlowStep,
        transfer: &Transfer,
    ) -> Result<Option<DataAddress>, String> {
        let pid = transfer.provider_pid.as_str();
        match step {
            FlowStep::Start => self.flows.start(transfer),
            FlowStep::Restart => self.flows.restart(transfer),
            FlowStep::Resume => self.flows.resume(pid, None).map(|()| None),
            // the consumer's new address, saved with the transfer
            FlowStep::ResumeWithAddress => self
                .flows
                .resume(pid, transfer.data_address.as_ref())
                .map(|()| None),
            FlowStep::Suspend => self.flows.suspend(pid).map(|()| None),
            FlowStep::Complete => self.flows.complete(pid).map(|()| None),
            FlowStep::Terminate => self.flows.terminate(pid).map(|()| None),
        }
    }
}

/// The refusal of a write while `transfer` is leased (module doc).
fn busy(transfer: &Transfer) -> Failure {
    Failure::Busy(TransferError::new(
        &transfer.provider_pid,
        &transfer.consumer_pid,
        "503",
        BUSY,
    ))
}

/// After a start (or a start again by the provider) ran: the start
/// message, with the `address` the data plane gave, becomes the message
/// to send. Nothing for other steps, or if the transfer can no longer be
/// started.
fn add_start_message(transfer: &mut Transfer, step: FlowStep, address: Option<DataAddress>) {
    let starts = matches!(step, FlowStep::Start | FlowStep::Restart);
    if starts && next_state(Some(transfer.state), Transition::Start, Role::Provider).is_ok() {
        transfer.pending = Some(start_message(transfer, address));
    }
}

/// Makes `transfer` what it was `before` the write, except its version,
/// which the write that takes it back sets. `before` records no step and
/// holds no lease: a write is made only then.
fn take_back(transfer: &mut Transfer, before: &Transfer) {
    let version = transfer.version;
    *transfer = before.clone();
    transfer.version = version;
}

/// The data plane refused a step for `reason`: unless the transfer has
/// ended, our message becomes a termination that says so.
fn end_after_refusal(transfer: &mut Transfer, reason: &str) {
    if !transfer.state.is_terminal() {
        let why = format!("the data plane refused: {reason}");
        transfer.pending = Some(termination(transfer, &why));
    }
}
