// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The state changes of negotiations and transfers, as log lines, span
//! events and metrics (ADR 0023). The stores' [`Shared`](crate::ports::Shared)
//! wrappers call these functions on every write, so every change is seen
//! once, whichever service or adapter made it. Transfers as provider go
//! through [`AnyTransferStore`](crate::ports::AnyTransferStore), which
//! reports a change once the data plane step its write recorded has run.
//!
//! Each change is one event with the target
//! [`STATE_EVENTS`](portitor_runtime::telemetry::STATE_EVENTS): a JSON
//! log line, and, with export on, an event of the current span. It holds
//! the DSP process ids, the old and the new state, and nothing else:
//! both sides of a process log the same ids, which is how their logs are
//! matched across participants. A process that reaches a final state also
//! counts in `portitor.negotiation.ended` or `portitor.transfer.ended`.

use portitor_dsp::negotiation::NegotiationState;
use portitor_dsp::transfer::TransferState;
use portitor_runtime::telemetry::{STATE_EVENTS, metrics};

pub(crate) use portitor_runtime::telemetry::metrics::Role;

/// A negotiation was written in `after`; it was in `before` (`None` for
/// a new one). Nothing happens when the state did not change.
pub(crate) fn negotiation_saved(
    role: Role,
    provider_pid: &str,
    consumer_pid: &str,
    before: Option<NegotiationState>,
    after: NegotiationState,
) {
    if before == Some(after) {
        return;
    }
    tracing::event!(
        target: STATE_EVENTS,
        tracing::Level::INFO,
        portitor.dsp.process = "negotiation",
        portitor.dsp.role = role.name(),
        portitor.dsp.provider_pid = provider_pid,
        portitor.dsp.consumer_pid = consumer_pid,
        portitor.dsp.previous_state = before.map(negotiation_state_name),
        portitor.dsp.state = negotiation_state_name(after),
        "negotiation state changed"
    );
    let was_terminal = before.is_some_and(NegotiationState::is_terminal);
    if after.is_terminal() && !was_terminal {
        metrics::negotiation_ended(role, negotiation_state_name(after));
    }
}

/// A transfer was written in `after`; it was in `before` (`None` for a
/// new one). Nothing happens when the state did not change.
pub(crate) fn transfer_saved(
    role: Role,
    provider_pid: &str,
    consumer_pid: &str,
    before: Option<TransferState>,
    after: TransferState,
) {
    if before == Some(after) {
        return;
    }
    tracing::event!(
        target: STATE_EVENTS,
        tracing::Level::INFO,
        portitor.dsp.process = "transfer",
        portitor.dsp.role = role.name(),
        portitor.dsp.provider_pid = provider_pid,
        portitor.dsp.consumer_pid = consumer_pid,
        portitor.dsp.previous_state = before.map(transfer_state_name),
        portitor.dsp.state = transfer_state_name(after),
        "transfer state changed"
    );
    let was_terminal = before.is_some_and(TransferState::is_terminal);
    if after.is_terminal() && !was_terminal {
        metrics::transfer_ended(role, transfer_state_name(after));
    }
}

/// The state as DSP 2025-1 writes it (without the `dspace:` prefix).
fn negotiation_state_name(state: NegotiationState) -> &'static str {
    match state {
        NegotiationState::Requested => "REQUESTED",
        NegotiationState::Offered => "OFFERED",
        NegotiationState::Accepted => "ACCEPTED",
        NegotiationState::Agreed => "AGREED",
        NegotiationState::Verified => "VERIFIED",
        NegotiationState::Finalized => "FINALIZED",
        NegotiationState::Terminated => "TERMINATED",
    }
}

/// The state as DSP 2025-1 writes it (without the `dspace:` prefix).
fn transfer_state_name(state: TransferState) -> &'static str {
    match state {
        TransferState::Requested => "REQUESTED",
        TransferState::Started => "STARTED",
        TransferState::Suspended => "SUSPENDED",
        TransferState::Completed => "COMPLETED",
        TransferState::Terminated => "TERMINATED",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// One JSON log line per change, with both process ids; none when
    /// the state stays the same (a write that only clears the outbox).
    #[test]
    fn a_change_is_one_event_with_the_process_ids() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
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
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let agreed = Some(NegotiationState::Agreed);
            negotiation_saved(
                Role::Provider,
                "p-1",
                "c-1",
                agreed,
                NegotiationState::Agreed,
            );
            let verified = NegotiationState::Verified;
            negotiation_saved(Role::Provider, "p-1", "c-1", agreed, verified);
            transfer_saved(Role::Consumer, "", "c-2", None, TransferState::Requested);
        });
        let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0]["target"], STATE_EVENTS);
        let fields = &lines[0]["fields"];
        assert_eq!(fields["portitor.dsp.provider_pid"], "p-1");
        assert_eq!(fields["portitor.dsp.consumer_pid"], "c-1");
        assert_eq!(fields["portitor.dsp.previous_state"], "AGREED");
        assert_eq!(fields["portitor.dsp.state"], "VERIFIED");
        assert_eq!(lines[1]["fields"]["portitor.dsp.process"], "transfer");
        assert_eq!(lines[1]["fields"]["portitor.dsp.state"], "REQUESTED");
    }

    /// The names are the ones serde writes for the DSP messages.
    #[test]
    fn state_names_are_the_dsp_names() {
        for state in [
            NegotiationState::Requested,
            NegotiationState::Offered,
            NegotiationState::Accepted,
            NegotiationState::Agreed,
            NegotiationState::Verified,
            NegotiationState::Finalized,
            NegotiationState::Terminated,
        ] {
            let json = serde_json::to_value(state).unwrap();
            assert_eq!(json, negotiation_state_name(state));
        }
        for state in [
            TransferState::Requested,
            TransferState::Started,
            TransferState::Suspended,
            TransferState::Completed,
            TransferState::Terminated,
        ] {
            let json = serde_json::to_value(state).unwrap();
            assert_eq!(json, transfer_state_name(state));
        }
    }
}
