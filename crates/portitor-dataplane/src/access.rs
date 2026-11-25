// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Port: may the consumer of a data flow still get its data (ADR 0027,
//! section 7.2; ADR 0028, decision 5)?
//!
//! The control plane owns the answer: it kept the consumer's credentials
//! when the transfer was asked for, and it checks them again. The data
//! plane only asks, on every request for data and every
//! [`AccessGate::recheck_interval`] while a download or a push runs. Data
//! Plane Signaling has no message for this question, so while the data
//! plane runs inside the connector it is a function call. How a data plane
//! in its own process asks is decided with step 2 of ADR 0028.

use std::time::Duration;

use portitor_runtime::BoxFuture;

/// The answer of [`AccessGate::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// The consumer is still a member: serve.
    Granted,
    /// The check could not finish now, for this reason: refuse, keep the
    /// transfer.
    Unchecked(String),
    /// The consumer is no longer a member, for this reason: refuse and end
    /// the transfer.
    Revoked(String),
}

/// Port: the control plane's check of a flow's consumer (module doc).
pub trait AccessGate: Send + Sync {
    /// Whether the consumer of flow `flow_id` may still get its data.
    /// A boxed task, so the data plane can hold any gate as
    /// `Arc<dyn AccessGate>` (RUST-IDIOMS, "`BoxFuture`").
    fn check<'a>(&'a self, flow_id: &'a str) -> BoxFuture<'a, Access>;

    /// Ends the transfer of flow `flow_id`, whose consumer is no longer a
    /// member, for `reason`.
    fn end_transfer(&self, flow_id: &str, reason: &str);

    /// How often a running download or push asks again.
    fn recheck_interval(&self) -> Duration;
}
