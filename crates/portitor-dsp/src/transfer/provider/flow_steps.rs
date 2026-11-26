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
//! 4. After a restart, the steps still recorded run first, then the
//!    messages are sent (`TransferService::pending`). Running a step again
//!    is safe: the data plane takes a step to the state the flow is
//!    already in as done, and a start of a flow that exists gives it a new
//!    token (the first one was never sent).
//!
//! One call at a time. Every write of a provider's transfer takes one
//! lock (`one_step_at_a_time`) before it reads the transfer, and holds it
//! until the step it recorded has run and that is recorded
//! ([`TransferService::write_then_run_step`]). So no other write comes
//! between a write and its step: a step is never replaced before it runs,
//! the steps reach the data plane in the order of their writes (a
//! suspension never after the start again that followed it), a start
//! again makes one token, and a refusal goes to the call whose write
//! recorded the step. The lock guards no data and is never held while
//! waiting for a partner: the built-in data plane runs in this process,
//! so a step is a function call and a write to the local database. The
//! connector runs as one process on one `SQLite` file, so a lock in the
//! process is enough. With a remote data plane (ADR 0028, decision 7,
//! signaling over HTTP) this lock would be held across network calls, so
//! [`DataFlows`] would have to become async there, with a lock that may be
//! held across an `.await`.
//!
//! When the data plane refuses a step ([`IfRefused`]):
//!
//! - a step of the provider's own decision (the decider: start, suspend,
//!   complete, terminate), or one found after a restart or left recorded
//!   by an earlier call, ends the transfer: our message becomes a
//!   termination that gives the reason, and the data flow is terminated,
//!   best effort (as before 1.2);
//! - a step of a consumer's message, of a message that could not be
//!   delivered (`abort`), of the provider's own termination
//!   (`terminate_own`) or of the end of a push takes its write back: the
//!   transfer is written again as it was before, and the caller gets
//!   [`Failure::DataPlane`] (as before 1.2, when nothing was written).
//!   The connector reports neither of the two writes as a state change
//!   (its `AnyTransferStore`), as 1.1, which wrote nothing, reported none.
//!
//! When the step ran but recording that it ran failed (the store failed),
//! the recording is tried once more. If that fails too, the step stays
//! recorded, our message waits, and the caller gets [`Failure::Store`]
//! although its write landed. The step runs again (which is safe, see 4)
//! before the next write of that transfer, or at the next restart:
//! nothing runs it sooner, as the outbox has no periodic loop (ADR 0007).
//! A take-back that was due is not made then: if the data plane refuses
//! at that later run, the transfer ends instead.

use std::cell::Cell;
use std::sync::{MutexGuard, PoisonError};

use super::{
    Agreements, CatalogDatasets, DataAddress, DataFlows, Environment, Failure, FlowStep, Lookup,
    Role, StoreError, Transfer, TransferDecider, TransferService, TransferStore, Transition, Write,
    next_state, process, start_message, termination,
};
use crate::process::Records;

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

/// Proof that the caller holds `one_step_at_a_time`: the functions that
/// run a step take it, so they cannot be called without the lock.
type Turn<'a> = MutexGuard<'a, ()>;

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
    /// ([`process::update_keeping_before`]), then runs the data plane step
    /// that write recorded and records that it ran, all under one lock
    /// (module doc). A step that an earlier call left recorded runs first.
    /// `if_refused`: what a refusal of the data plane does. Returns `None`
    /// when `change` said [`Write::Skip`].
    ///
    /// # Errors
    ///
    /// The failure of `change`; [`Failure::DataPlane`] when the data plane
    /// refused a step that is taken back ([`IfRefused::TakeBack`]); a
    /// store failure; or the transfer kept changing.
    pub(super) fn write_then_run_step(
        &self,
        lookup: &Lookup<'_>,
        if_refused: IfRefused,
        change: impl Fn(&mut Transfer) -> Result<Write, Failure>,
    ) -> Result<Option<Stepped>, Failure> {
        let turn = self.take_turn();
        self.run_step_left_recorded(&turn, lookup)?;
        let Some(updated) = process::update_keeping_before(self, lookup, change)? else {
            return Ok(None);
        };
        let take_back_to = match if_refused {
            IfRefused::EndTheTransfer => None,
            IfRefused::TakeBack => Some(&updated.before),
        };
        let current = self.run_recorded_step(&turn, &updated.written, take_back_to)?;
        Ok(Some(Stepped {
            before: updated.before,
            written: updated.written,
            current,
        }))
    }

    /// Adds the new `transfer`, then runs the data plane step it records
    /// (a start), under the lock. A refusal ends the transfer. Returns the
    /// transfer once its step ran, or `None` when it was not added: its
    /// consumer already has a transfer with its `consumer_pid`.
    ///
    /// # Errors
    ///
    /// A store failure.
    pub(super) fn insert_then_run_step(
        &self,
        transfer: &Transfer,
    ) -> Result<Option<Transfer>, Failure> {
        let turn = self.take_turn();
        let inserted = self.store.insert(transfer).map_err(Failure::Store)?;
        if !inserted {
            return Ok(None);
        }
        let current = self.run_recorded_step(&turn, transfer, None)?;
        Ok(Some(current))
    }

    /// Runs the step recorded on `transfer`, found after a restart, under
    /// the lock. A refusal ends the transfer. Returns the transfer as it is
    /// then.
    ///
    /// # Errors
    ///
    /// A store failure, or the transfer kept changing.
    pub(super) fn run_step_found_after_restart(
        &self,
        transfer: &Transfer,
    ) -> Result<Transfer, Failure> {
        let turn = self.take_turn();
        self.run_recorded_step(&turn, transfer, None)
    }

    /// Waits for `one_step_at_a_time` (module doc). A panic while the lock
    /// was held marks it "poisoned"; it guards no data, so it is taken
    /// anyway.
    fn take_turn(&self) -> Turn<'_> {
        self.one_step_at_a_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs the step an earlier call left recorded on the transfer
    /// `lookup` names, if there is one: that call's recording failed
    /// (module doc). It runs before the next write, so the steps reach the
    /// data plane in the order of their writes. A refusal ends the
    /// transfer.
    fn run_step_left_recorded(&self, turn: &Turn<'_>, lookup: &Lookup<'_>) -> Result<(), Failure> {
        let transfer = self.read(lookup)?;
        if transfer.flow_step.is_some() {
            self.run_recorded_step(turn, &transfer, None)?;
        }
        Ok(())
    }

    /// Runs the data plane step recorded on `written`, a transfer just
    /// written, if it has one, and records that it ran (module doc).
    /// `take_back_to`: the transfer as it was before that write, when a
    /// refusal takes the write back; `None` when a refusal ends the
    /// transfer. Returns the transfer as it is then.
    ///
    /// # Errors
    ///
    /// [`Failure::DataPlane`] when the data plane refused a step whose
    /// write is taken back; [`Failure::Store`] when the step ran but could
    /// not be recorded; a store failure.
    fn run_recorded_step(
        &self,
        _turn: &Turn<'_>,
        written: &Transfer,
        take_back_to: Option<&Transfer>,
    ) -> Result<Transfer, Failure> {
        if written.flow_step.is_none() {
            return Ok(written.clone());
        }
        let lookup = Lookup::own(&written.provider_pid);
        // read again: after a restart, `written` was read before the lock
        // was taken
        let transfer = self.read(&lookup)?;
        let Some(step) = transfer.flow_step else {
            return Ok(transfer);
        };
        let answer = self.run_on_data_plane(step, &transfer);
        // The change in `record_that_step_ran` may run more than once
        // (when a write comes first), so it notes in this cell whether its
        // last run ended the transfer, instead of returning it.
        let ended = Cell::new(false);
        let mut recorded =
            self.record_that_step_ran(&lookup, step, &answer, written, take_back_to, &ended);
        if recorded.is_err() {
            // the step ran: try once more to record it, so that it is not
            // run again and our message does not wait (module doc)
            recorded =
                self.record_that_step_ran(&lookup, step, &answer, written, take_back_to, &ended);
        }
        let current = match recorded {
            Ok(Some(current)) => current,
            // the step changed in between, which the lock should prevent:
            // the transfer as it is now
            Ok(None) => return self.read(&lookup),
            Err(failure) => {
                return Err(Failure::Store(StoreError(format!(
                    "the data plane step {} of transfer {} ran, but recording that it ran \
                     failed: {failure:?}; it stays recorded and runs again before the next \
                     write of the transfer or at the next restart",
                    step.name(),
                    written.provider_pid
                ))));
            }
        };
        let Err(reason) = answer else {
            return Ok(current);
        };
        if ended.get() {
            // end whatever the data plane still serves (best effort)
            let _ = self.flows.terminate(&current.provider_pid);
        }
        match take_back_to {
            None => Ok(current),
            Some(_) => Err(Failure::DataPlane(reason)),
        }
    }

    /// Writes that `step` ran and gave `answer`: clears `flow_step`, and
    /// adds the start message after a start, takes the write of `written`
    /// back to `take_back_to` after a refusal, or ends the transfer after
    /// a refusal that cannot be taken back (then sets `ended`). Returns
    /// the transfer as written, or `None` if `flow_step` no longer holds
    /// `step`.
    fn record_that_step_ran(
        &self,
        lookup: &Lookup<'_>,
        step: FlowStep,
        answer: &Result<Option<DataAddress>, String>,
        written: &Transfer,
        take_back_to: Option<&Transfer>,
        ended: &Cell<bool>,
    ) -> Result<Option<Transfer>, Failure> {
        process::update(self, lookup, |current| {
            if current.flow_step != Some(step) {
                // every write holds the lock, so no newer step can have
                // been recorded; if one were, it is not ours to clear
                return Ok(Write::Skip);
            }
            current.flow_step = None;
            // an earlier run of this closure lost its write and may have
            // set it
            ended.set(false);
            match answer {
                Ok(address) => add_start_message(current, step, address.clone()),
                Err(reason) => {
                    // under the lock nothing is written in between; were
                    // something written, it would not be undone
                    let nothing_written_since = current.version == written.version;
                    match take_back_to {
                        Some(before) if nothing_written_since => take_back(current, before),
                        _ => {
                            end_after_refusal(current, reason);
                            ended.set(true);
                        }
                    }
                }
            }
            Ok(Write::Save)
        })
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
/// which the write that takes it back sets.
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
