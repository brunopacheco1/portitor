// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider's own view of its transfers and its own termination, for
//! the operator (the management API, ADR 0027 section 7.1) and for the
//! data plane when a consumer's membership no longer holds (section 7.2).
//!
//! A termination by the provider follows ADR 0014, "The provider's own
//! steps": access ends at once, then the outbox sends the termination.
//! The termination is saved as the pending message, with the data flow's
//! termination as the step to run; the step runs right after that write,
//! before the call returns, and the message waits for it (`flow_steps`).
//! One difference from the consumer's own steps: a message of ours still
//! on its way does not stop it. The termination replaces that message
//! (as when a message cannot be delivered, `abort`), because ending
//! access must not wait for a partner that may not answer. The outbox
//! ignores a report about the replaced message: it is no longer pending.

use crate::process::{Lookup, Records, Write};

use super::{
    Agreements, CatalogDatasets, DataFlows, Environment, Failure, FlowStep, IfRefused, Outgoing,
    Transfer, TransferDecider, TransferService, TransferStore, Transition, bad, ready_to_send,
    termination,
};

/// The most transfers one page of [`TransferService::list`] holds.
pub const MAX_PAGE: usize = 100;

impl<S, A, F, D, C, E> TransferService<S, A, F, D, C, E>
where
    S: TransferStore,
    A: Agreements,
    F: DataFlows,
    D: TransferDecider,
    C: CatalogDatasets,
    E: Environment,
{
    /// The transfer `provider_pid`, as stored, whoever its consumer.
    ///
    /// # Errors
    ///
    /// Unknown transfer, or store failure.
    pub fn transfer(&self, provider_pid: &str) -> Result<Transfer, Failure> {
        self.read(&Lookup::own(provider_pid))
    }

    /// One page of transfers in provider pid order: up to `limit` (1 to
    /// [`MAX_PAGE`]) after `after`.
    ///
    /// # Errors
    ///
    /// Store failure.
    pub fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, Failure> {
        self.store
            .list(after, limit.clamp(1, MAX_PAGE))
            .map_err(Failure::Store)
    }

    /// The provider ends transfer `provider_pid` for `reason`: the data
    /// flow is terminated (which stops every running download), and the
    /// termination becomes the message to send (module doc). Returns the
    /// transfer as saved, and the termination to send; `None` when a
    /// termination of ours was already on its way. The data flow is
    /// terminated then too: ending access must not depend on what came
    /// before.
    ///
    /// # Errors
    ///
    /// Unknown transfer, a transfer that has ended (HTTP 400), data plane
    /// or store failure. When the data plane refuses, the transfer stays
    /// as it was.
    pub fn terminate_own(
        &self,
        provider_pid: &str,
        reason: &str,
    ) -> Result<(Transfer, Option<Outgoing>), Failure> {
        let lookup = Lookup::own(provider_pid);
        let stepped = self.write_then_run_step(&lookup, IfRefused::TakeBack, |transfer| {
            if transfer.state.is_terminal() {
                return Err(bad(
                    provider_pid,
                    &transfer.consumer_pid,
                    "the transfer has ended",
                ));
            }
            // access ends right after this write, whatever follows
            transfer.flow_step = Some(FlowStep::Terminate);
            if !termination_on_its_way(transfer) {
                transfer.pending = Some(termination(transfer, reason));
            }
            Ok(Write::Save)
        })?;
        // the change above never skips; `None` is handled anyway
        let Some(stepped) = stepped else {
            return Ok((self.transfer(provider_pid)?, None));
        };
        // our termination was on its way already: it is not sent again
        let out = if termination_on_its_way(&stepped.before) {
            None
        } else {
            ready_to_send(&stepped.current)
        };
        Ok((stepped.written, out))
    }
}

/// Whether our termination of `transfer` is on its way already.
fn termination_on_its_way(transfer: &Transfer) -> bool {
    match &transfer.pending {
        Some(sent) => sent.transition == Transition::Terminate,
        None => false,
    }
}
