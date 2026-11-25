// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The end of a push transfer (ADR 0027, section 1): the data plane has
//! sent the data, or failed to, and the provider completes or terminates
//! the transfer.

use super::{
    Agreements, CatalogDatasets, DataFlows, Environment, Failure, FlowStep, IfRefused, Lookup,
    Outgoing, TransferDecider, TransferService, TransferState, TransferStore, Write, completion,
    process, ready_to_send, termination,
};

impl<S, A, F, D, C, E> TransferService<S, A, F, D, C, E>
where
    S: TransferStore,
    A: Agreements,
    F: DataFlows,
    D: TransferDecider,
    C: CatalogDatasets,
    E: Environment,
{
    /// The data plane finished sending the data of push transfer
    /// `provider_pid` (ADR 0027, section 1): `failure` is `None` when the
    /// consumer's endpoint answered 2xx, else the reason (a status code,
    /// never the URL or the token). Ends the data flow and the transfer:
    /// completed on success, terminated with the reason otherwise. Returns
    /// the completion or termination to send.
    ///
    /// Changes nothing when the transfer is no longer `STARTED` (it was
    /// suspended or ended meanwhile) or a message of ours is still on its
    /// way.
    ///
    /// The data flow is ended right after the transfer's write, before
    /// the message is sent (`flow_steps`).
    ///
    /// # Errors
    ///
    /// Unknown transfer, data plane or store failure. When the data plane
    /// refuses, the transfer stays as it was.
    pub fn push_finished(
        &self,
        provider_pid: &str,
        failure: Option<&str>,
    ) -> Result<Option<Outgoing>, Failure> {
        let lookup = Lookup::own(provider_pid);
        let saved = process::update_keeping_before(self, &lookup, |transfer| {
            if transfer.state != TransferState::Started || transfer.pending.is_some() {
                return Ok(Write::Skip);
            }
            let (message, step) = match failure {
                None => (completion(transfer), FlowStep::Complete),
                Some(reason) => (
                    termination(transfer, &format!("the push failed: {reason}")),
                    FlowStep::Terminate,
                ),
            };
            transfer.pending = Some(message);
            transfer.flow_step = Some(step);
            Ok(Write::Save)
        })?;
        let Some((before, written)) = saved else {
            return Ok(None);
        };
        let if_refused = IfRefused::TakeBack { before: &before };
        let now = self.after_write(written.clone(), &if_refused)?;
        Ok(ready_to_send(&now))
    }
}
