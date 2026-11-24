// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The audit line of a data transfer (ADR 0027, section 6): one INFO log
//! line `data transfer done` for each finished pull, push sent and push
//! received, in the JSON log the connector already writes (ADR 0023). No
//! token, URL, endpoint or object storage key goes into it.

/// How a data transfer ended, as the `outcome` field says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// All the data was sent or received.
    Done,
    /// It failed: the other side, the source or the store did not take
    /// or give the data.
    Failed,
    /// Suspend or terminate cut it while it ran.
    Cancelled,
}

impl Outcome {
    /// The value of the `outcome` field: `done`, `failed` or `cancelled`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What one audit line says. `flow` is the transfer's process id on this
/// side (the data flow id on the provider, the consumer pid on the
/// consumer), `counter_party` the other participant's DID.
#[derive(Debug, Clone, Copy)]
pub struct TransferDone<'a> {
    /// The process id on this side.
    pub flow: &'a str,
    /// The agreement the transfer is made under.
    pub agreement: &'a str,
    /// The other participant's DID.
    pub counter_party: &'a str,
    /// How many bytes of data moved.
    pub bytes: u64,
    /// How it ended.
    pub outcome: Outcome,
}

/// Writes the audit line for `done`.
pub fn transfer_done(done: &TransferDone<'_>) {
    tracing::info!(
        flow = done.flow,
        agreement = done.agreement,
        counterParty = done.counter_party,
        bytes = done.bytes,
        outcome = done.outcome.as_str(),
        "data transfer done"
    );
}
