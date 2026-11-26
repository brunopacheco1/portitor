// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Everything that is running for a data flow right now, and the switch
//! that stops it all at once (ADR 0027, section 5: "suspend or terminate
//! cuts every download of that flow that is already running").
//!
//! A piece of work that moves a flow's data (a `/data` download, or a
//! push) asks [`RunningFlows::watch`] for a [`CutSignal`] before
//! it checks that the flow may still be served, and stops as soon as
//! [`CutSignal::cut`] finishes. [`RunningFlows::cut`] stops every piece
//! of work of one flow. It knows nothing of downloads or pushes: it only
//! keeps one switch per flow.
//!
//! The order matters, and closes the gap between "checked" and "cut":
//! the data plane changes the flow's state first and cuts second, and a
//! download watches first and checks the state second. So a download
//! that started its check before the change is cut, and one that started
//! after it sees the new state and is refused.
//!
//! How it works: each flow with running work has one
//! `tokio::sync::watch` sender, and each piece of work holds a receiver.
//! The sender never sends; cutting removes it from the map and drops it,
//! and every receiver wakes up because its sender is gone. The next
//! piece of work for that flow gets a new sender.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

/// The switches of the flows that have work running.
#[derive(Debug, Default)]
pub struct RunningFlows {
    switches: Mutex<BTreeMap<String, watch::Sender<()>>>,
}

/// Held by one piece of work of one flow: tells it when to stop.
#[derive(Debug)]
pub struct CutSignal(watch::Receiver<()>);

impl CutSignal {
    /// Finishes when the work must stop: its flow was cut. Waits forever
    /// otherwise.
    pub async fn cut(&mut self) {
        // the sender never sends, so this ends only when it is dropped,
        // which is what `RunningFlows::cut` does
        while self.0.changed().await.is_ok() {}
    }

    /// Whether the flow was cut already.
    #[must_use]
    pub fn is_cut(&self) -> bool {
        self.0.has_changed().is_err()
    }
}

impl RunningFlows {
    /// No flow has work running.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn switches(&self) -> MutexGuard<'_, BTreeMap<String, watch::Sender<()>>> {
        // the map stays usable after a panic elsewhere: at worst it keeps
        // a switch nobody watches, which the next `watch` removes
        self.switches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A signal for one more piece of work of flow `flow_id`. Call it
    /// before checking that the flow may be served (module doc).
    pub fn watch(&self, flow_id: &str) -> CutSignal {
        let mut switches = self.switches();
        // the switches of flows whose work has all ended are no longer
        // needed: drop them, so the map holds only running flows
        switches.retain(|_, switch| switch.receiver_count() > 0);
        let switch = switches
            .entry(flow_id.to_owned())
            .or_insert_with(|| watch::channel(()).0);
        CutSignal(switch.subscribe())
    }

    /// Stops every piece of work of flow `flow_id` that is running now.
    /// Work started later is not affected. Returns how many were told.
    pub fn cut(&self, flow_id: &str) -> usize {
        let removed = self.switches().remove(flow_id);
        match removed {
            // dropping the sender wakes every receiver
            Some(switch) => switch.receiver_count(),
            None => 0,
        }
    }

    /// How many pieces of work of flow `flow_id` are running.
    #[must_use]
    pub fn running(&self, flow_id: &str) -> usize {
        self.switches()
            .get(flow_id)
            .map_or(0, watch::Sender::receiver_count)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_cut_stops_all_work_of_that_flow_only() {
        let running = RunningFlows::new();
        let mut first = running.watch("f1");
        let mut second = running.watch("f1");
        let other = running.watch("f2");
        assert_eq!(running.running("f1"), 2);
        assert!(!first.is_cut());

        assert_eq!(running.cut("f1"), 2);
        tokio::time::timeout(Duration::from_secs(1), first.cut())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), second.cut())
            .await
            .unwrap();
        assert!(second.is_cut());
        assert!(!other.is_cut(), "another flow goes on");
        assert_eq!(running.cut("f1"), 0, "nothing left to cut");
    }

    #[tokio::test]
    async fn work_started_after_a_cut_runs() {
        let running = RunningFlows::new();
        let old = running.watch("f1");
        running.cut("f1");
        let mut new = running.watch("f1");
        assert!(old.is_cut());
        assert!(!new.is_cut());
        let waited = tokio::time::timeout(Duration::from_millis(50), new.cut()).await;
        assert!(waited.is_err(), "not cut");
    }

    #[test]
    fn ended_work_leaves_no_switch() {
        let running = RunningFlows::new();
        drop(running.watch("f1"));
        let _f2 = running.watch("f2");
        assert_eq!(running.running("f1"), 0);
        assert_eq!(running.switches().len(), 1, "only f2 is kept");
    }
}
