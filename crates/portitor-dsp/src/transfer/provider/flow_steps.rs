// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The data plane steps of the provider's transfers: told after the
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
//! 2. Right after the write, the same call runs the step that is recorded
//!    ([`TransferService::after_write`]), then writes again to record that
//!    it ran: `flow_step` is cleared. A start (or a start again by the
//!    provider) gets its start message in that second write, with the
//!    data address the data plane made: it carries the consumer's token.
//! 3. Our message waits for the step of its change: it is sent only once
//!    `flow_step` is empty (`ready_to_send`). So the data plane is still
//!    told before the consumer (ADR 0013).
//! 4. After a restart, the steps still recorded run first, then the
//!    messages are sent (`TransferService::pending`). Running a step again
//!    is safe: the data plane takes a step to the state the flow is
//!    already in as done, and a start of a flow that exists gives it a new
//!    token (the first one was never sent).
//!
//! A step that a newer write replaced before it ran is not run: the call
//! that made the newer write runs the newer step.
//!
//! When the data plane refuses a step ([`IfRefused`]):
//!
//! - a step of the provider's own decision (the decider: start, suspend,
//!   complete, terminate), or one found after a restart, ends the
//!   transfer: our message becomes a termination that gives the reason,
//!   and the data flow is terminated, best effort (as before 1.2);
//! - a step of a consumer's message, of a message that could not be
//!   delivered (`abort`), of the provider's own termination
//!   (`terminate_own`) or of the end of a push takes its write back: the
//!   transfer is written again as it was before, and the caller gets
//!   [`Failure::DataPlane`] (as before 1.2, when nothing was written). If
//!   another write came in between, the write is not taken back: the
//!   transfer ends as in the first case, and the caller still gets the
//!   failure.
//!
//! One step at a time. Two calls may change the same transfer at once,
//! and each runs the step it finds recorded. If both ran at the same
//! time, an older step could reach the data plane after a newer one (a
//! suspension after the start again that followed it), or a start again
//! could make two tokens. So the steps run under one lock
//! (`one_step_at_a_time`), held from reading the step to recording that it
//! ran. The lock guards no data and is never held while waiting for a
//! partner: the built-in data plane runs in this process, so a step is a
//! function call and a write to the local database. The connector runs as
//! one process on one `SQLite` file, so a lock in the process is enough.

use std::cell::Cell;
use std::sync::PoisonError;

use super::{
    Agreements, CatalogDatasets, DataAddress, DataFlows, Environment, Failure, FlowStep, Lookup,
    Role, Transfer, TransferDecider, TransferService, TransferStore, Transition, Write, next_state,
    process, start_message, termination,
};
use crate::process::Records;

/// What happens when the data plane refuses the step that a write
/// recorded (module doc).
pub(super) enum IfRefused<'a> {
    /// The transfer ends: our message becomes a termination, and the data
    /// flow is terminated (best effort).
    EndTheTransfer,
    /// The write is taken back: the transfer is written again as `before`,
    /// and the caller gets [`Failure::DataPlane`].
    TakeBack {
        /// The transfer as it was read before the write.
        before: &'a Transfer,
    },
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
    /// Runs the data plane step recorded on `written`, a transfer just
    /// written, if it has one, and records that it ran (module doc).
    /// Returns the transfer as it is then.
    ///
    /// # Errors
    ///
    /// [`Failure::DataPlane`] when the data plane refused a step whose
    /// write is taken back ([`IfRefused::TakeBack`]); a store failure; or
    /// the transfer kept changing.
    pub(super) fn after_write(
        &self,
        written: Transfer,
        if_refused: &IfRefused<'_>,
    ) -> Result<Transfer, Failure> {
        if written.flow_step.is_none() {
            return Ok(written);
        }
        // One step at a time (module doc). A panic while the lock was held
        // marks it "poisoned"; it guards no data, so it is taken anyway.
        let _held = self
            .one_step_at_a_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let lookup = Lookup::own(&written.provider_pid);
        // read again under the lock: another call may have run the step
        // since, or recorded a newer one
        let transfer = self.read(&lookup)?;
        let Some(step) = transfer.flow_step else {
            return Ok(transfer);
        };
        let told = self.tell(step, &transfer);
        // The change below may run more than once (when another write
        // comes first), so it notes in this cell whether its last run
        // ended the transfer, instead of returning it.
        let ended = Cell::new(false);
        let recorded = process::update(self, &lookup, |now| {
            if now.flow_step != Some(step) {
                // a newer step was recorded while this one ran: the call
                // that recorded it runs it
                return Ok(Write::Skip);
            }
            now.flow_step = None;
            ended.set(false);
            match (&told, if_refused) {
                (Ok(address), _) => add_start_message(now, step, address.clone()),
                // nothing was written since our write: take it back
                (Err(_), IfRefused::TakeBack { before }) if now.version == written.version => {
                    take_back(now, before);
                }
                (Err(reason), _) => {
                    end_after_refusal(now, reason);
                    ended.set(true);
                }
            }
            Ok(Write::Save)
        })?;
        let Some(now) = recorded else {
            return self.read(&lookup);
        };
        let Err(reason) = told else {
            return Ok(now);
        };
        if ended.get() {
            // end whatever the data plane still serves (best effort)
            let _ = self.flows.terminate(&now.provider_pid);
        }
        match if_refused {
            IfRefused::EndTheTransfer => Ok(now),
            IfRefused::TakeBack { .. } => Err(Failure::DataPlane(reason)),
        }
    }

    /// Tells the data plane `step` for `transfer`. Returns the data
    /// address a start gives (for a pull transfer), else `None`.
    fn tell(&self, step: FlowStep, transfer: &Transfer) -> Result<Option<DataAddress>, String> {
        let pid = transfer.provider_pid.as_str();
        let done = match step {
            FlowStep::Start => return self.flows.start(transfer),
            FlowStep::Restart => return self.flows.restart(transfer),
            FlowStep::Resume => self.flows.resume(pid, None),
            // the consumer's new address, saved with the transfer
            FlowStep::ResumeWithAddress => self.flows.resume(pid, transfer.data_address.as_ref()),
            FlowStep::Suspend => self.flows.suspend(pid),
            FlowStep::Complete => self.flows.complete(pid),
            FlowStep::Terminate => self.flows.terminate(pid),
        };
        match done {
            Ok(()) => Ok(None),
            Err(reason) => Err(reason),
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
