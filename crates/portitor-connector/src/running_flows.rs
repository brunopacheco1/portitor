// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Data that is moving right now, per data flow, so that it can be cut
//! (ADR 0027, sections 1 and 5): suspend or terminate stops a push that
//! is running, and a running download of a pull transfer.
//!
//! One small mechanism for both:
//!
//! - whatever moves data for a flow (a push, a download) calls
//!   [`RunningFlows::register`] first and keeps the [`FlowGuard`] it gets
//!   until it is done. Dropping the guard says "done".
//! - whoever ends access (the data plane on suspend or terminate) calls
//!   [`RunningFlows::cancel`] with the flow id. Every guard of that flow
//!   is told, and [`FlowGuard::cancelled`] returns; the mover stops.
//!
//! A guard taken after a cancel is new and not cancelled: cancelling
//! cuts what runs now, it does not block a later start again. Whether a
//! new transfer of data may start at all is the data plane's state
//! check, not this registry's.
//!
//! Built on a `tokio::sync::watch` channel per flow (one sender here,
//! one receiver in each guard), so no new crate is needed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

/// One flow's entry: the channel that tells its guards to stop, and how
/// many guards are out.
struct Entry {
    stop: watch::Sender<bool>,
    guards: usize,
    /// Which round of the flow this is: a new entry after a cancel gets
    /// a new number, so a guard of the old round cannot count it out.
    round: u64,
}

/// The registry of running data movements, shared (cheap to clone).
#[derive(Clone, Default)]
pub struct RunningFlows {
    flows: Arc<Mutex<Flows>>,
}

/// The entries, and the number of the next round.
#[derive(Default)]
struct Flows {
    entries: HashMap<String, Entry>,
    next_round: u64,
}

impl Flows {
    /// A new entry for `flow_id` with no guard yet; replaces none.
    fn new_entry(&mut self) -> Entry {
        self.next_round = self.next_round.wrapping_add(1);
        Entry {
            stop: watch::channel(false).0,
            guards: 0,
            round: self.next_round,
        }
    }
}

impl std::fmt::Debug for RunningFlows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningFlows")
            .field("flows", &self.lock().entries.len())
            .finish()
    }
}

impl RunningFlows {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The map, even after a panic in another thread: every change below
    /// is one step, so the map is never left half-changed.
    fn lock(&self) -> MutexGuard<'_, Flows> {
        self.flows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records that data of flow `flow_id` starts moving. Keep the guard
    /// while it moves.
    #[must_use]
    pub fn register(&self, flow_id: &str) -> FlowGuard {
        let mut flows = self.lock();
        let mut entry = match flows.entries.remove(flow_id) {
            Some(entry) => entry,
            None => flows.new_entry(),
        };
        entry.guards += 1;
        let guard = self.guard(flow_id, &entry);
        flows.entries.insert(flow_id.to_owned(), entry);
        guard
    }

    /// Like [`RunningFlows::register`], but only when nothing of flow
    /// `flow_id` is moving now; `None` otherwise. Checked and recorded in
    /// one step, so two callers cannot both start (one push per flow).
    #[must_use]
    pub fn register_if_idle(&self, flow_id: &str) -> Option<FlowGuard> {
        let mut flows = self.lock();
        if flows.entries.contains_key(flow_id) {
            return None;
        }
        let mut entry = flows.new_entry();
        entry.guards = 1;
        let guard = self.guard(flow_id, &entry);
        flows.entries.insert(flow_id.to_owned(), entry);
        Some(guard)
    }

    /// Tells every guard of flow `flow_id` to stop. Returns how many were
    /// running. A flow with nothing running is not an error (0).
    #[allow(clippy::must_use_candidate)] // callers may ignore the count
    pub fn cancel(&self, flow_id: &str) -> usize {
        let Some(entry) = self.lock().entries.remove(flow_id) else {
            return 0;
        };
        // the receivers keep the value after the sender is dropped
        let _ = entry.stop.send_replace(true);
        entry.guards
    }

    /// How many guards of flow `flow_id` are out now.
    #[must_use]
    pub fn running(&self, flow_id: &str) -> usize {
        self.lock()
            .entries
            .get(flow_id)
            .map_or(0, |entry| entry.guards)
    }

    /// A guard of `entry`, the entry of `flow_id`.
    fn guard(&self, flow_id: &str, entry: &Entry) -> FlowGuard {
        FlowGuard {
            stop: entry.stop.subscribe(),
            flows: self.clone(),
            flow_id: flow_id.to_owned(),
            round: entry.round,
        }
    }

    /// A guard of `flow_id`, round `round`, was dropped: count it out.
    fn release(&self, flow_id: &str, round: u64) {
        let mut flows = self.lock();
        let Some(entry) = flows.entries.get_mut(flow_id) else {
            // cancelled: the entry is gone already
            return;
        };
        // a guard of an earlier, cancelled round must not count out a
        // guard of the entry that replaced it
        if entry.round != round {
            return;
        }
        entry.guards = entry.guards.saturating_sub(1);
        if entry.guards == 0 {
            flows.entries.remove(flow_id);
        }
    }
}

/// Held while data of one flow moves; see the module comment. Dropping
/// it says the movement is over.
pub struct FlowGuard {
    stop: watch::Receiver<bool>,
    flows: RunningFlows,
    flow_id: String,
    round: u64,
}

impl std::fmt::Debug for FlowGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowGuard")
            .field("flow_id", &self.flow_id)
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl FlowGuard {
    /// Whether the flow was cancelled since this guard was taken.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.stop.borrow()
    }

    /// Waits until the flow is cancelled. Use it in a `tokio::select!`
    /// next to the work, so the work is dropped (and stops) on cancel.
    pub async fn cancelled(&self) {
        let mut stop = self.stop.clone();
        // `wait_for` looks at the current value first, so a cancel that
        // came before this call is seen. It fails only when the sender is
        // gone without a cancel, which `RunningFlows` never does while a
        // guard is out; then this waits for ever, as nothing can cancel.
        if stop.wait_for(|stopped| *stopped).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// The flow this guard belongs to.
    #[must_use]
    pub fn flow_id(&self) -> &str {
        &self.flow_id
    }
}

impl Drop for FlowGuard {
    fn drop(&mut self) {
        self.flows.release(&self.flow_id, self.round);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn cancel_reaches_every_guard_of_the_flow_and_only_those() {
        let running = RunningFlows::new();
        let first = running.register("f1");
        let second = running.register("f1");
        let other = running.register("f2");
        assert_eq!(running.running("f1"), 2);
        assert_eq!(running.cancel("f1"), 2);
        assert!(first.is_cancelled() && second.is_cancelled());
        assert!(!other.is_cancelled());
        // returns at once: the cancel came before the wait
        tokio::time::timeout(Duration::from_secs(1), first.cancelled())
            .await
            .unwrap();
        assert_eq!(running.running("f1"), 0);
        assert_eq!(running.cancel("f1"), 0, "nothing runs any more");
    }

    #[tokio::test]
    async fn a_waiting_guard_wakes_up_on_cancel() {
        let running = RunningFlows::new();
        let guard = running.register("f1");
        let canceller = running.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            canceller.cancel("f1");
        });
        tokio::time::timeout(Duration::from_secs(5), guard.cancelled())
            .await
            .unwrap();
    }

    #[test]
    fn dropping_guards_counts_them_out_and_a_new_round_is_not_cancelled() {
        let running = RunningFlows::new();
        let old = running.register("f1");
        running.cancel("f1");
        let new = running.register("f1");
        assert!(!new.is_cancelled(), "a guard after a cancel is new");
        drop(old);
        assert_eq!(running.running("f1"), 1, "the old guard counts nothing");
        drop(new);
        assert_eq!(running.running("f1"), 0);
    }

    #[test]
    fn only_one_guard_when_idle_is_asked_for() {
        let running = RunningFlows::new();
        let push = running.register_if_idle("f1").unwrap();
        assert!(running.register_if_idle("f1").is_none());
        drop(push);
        assert!(running.register_if_idle("f1").is_some());
    }
}
