// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider's own view of its transfers and its own termination, for
//! the operator (the management API, ADR 0027 section 7.1) and for the
//! data plane when a consumer's membership no longer holds (section 7.2).
//!
//! A termination by the provider follows ADR 0014, "The provider's own
//! steps": the data plane is told first, so access ends at once, then the
//! termination is kept as the pending message and the outbox sends it.
//! One difference from the consumer's own steps: a message of ours still
//! on its way does not stop it. The termination replaces that message
//! (as when a message cannot be delivered, `abort`), because ending
//! access must not wait for a partner that may not answer. The outbox
//! ignores a report about the replaced message: it is no longer pending.

use crate::process::{self, Lookup, Records, Write};

use super::{
    Agreements, CatalogDatasets, DataFlows, Environment, Failure, Outgoing, Transfer,
    TransferDecider, TransferService, TransferStore, Transition, bad, termination,
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
    /// termination of ours was already on its way.
    ///
    /// # Errors
    ///
    /// Unknown transfer, a transfer that has ended (HTTP 400), data plane
    /// or store failure.
    pub fn terminate_own(
        &self,
        provider_pid: &str,
        reason: &str,
    ) -> Result<(Transfer, Option<Outgoing>), Failure> {
        let saved = process::update(self, &Lookup::own(provider_pid), |transfer| {
            if transfer.state.is_terminal() {
                return Err(bad(
                    provider_pid,
                    &transfer.consumer_pid,
                    "the transfer has ended",
                ));
            }
            // the data plane first: access ends now, whatever follows
            self.flows
                .terminate(provider_pid)
                .map_err(Failure::DataPlane)?;
            let ending = transfer
                .pending
                .as_ref()
                .is_some_and(|sent| sent.transition == Transition::Terminate);
            if ending {
                return Ok(Write::Skip);
            }
            transfer.pending = Some(termination(transfer, reason));
            Ok(Write::Save)
        })?;
        match saved {
            Some(transfer) => {
                let out = transfer.pending.clone();
                Ok((transfer, out))
            }
            // our termination was on its way already
            None => Ok((self.transfer(provider_pid)?, None)),
        }
    }
}
