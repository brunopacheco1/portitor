// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The consumer's membership, checked again for every use of a data flow
//! (ADR 0027, section 7.2).
//!
//! When the consumer asks for a transfer, the DSP caller check has just
//! verified its `MembershipCredential`s ([`crate::auth::Membership`]).
//! The provider keeps them with the transfer's data flow
//! ([`FlowStore::keep_memberships`], keyed by the provider pid, which is
//! also the flow id). [`MembershipGate::check`] checks them again, without
//! the consumer, through the caller verifier
//! ([`crate::auth::CallerVerifier::check_membership`]): the issuer is
//! still trusted, the credential has not expired, and no status bit is
//! set (with the status list cache, `PORTITOR_STATUS_LIST_CACHE_SECONDS`).
//!
//! The data plane asks the gate on every `/data` request and, while a
//! download runs, every [`MembershipGate::recheck_interval`]. A push
//! (ADR 0027, section 1) asks it before it starts and while it sends.
//!
//! What the answer means:
//!
//! - [`Access::Granted`] means: serve.
//! - [`Access::Unchecked`] means: the check could not finish (a status list or
//!   the issuer's DID document did not answer). Refuse now, and cut what
//!   runs, but keep the transfer: the next request may pass.
//! - [`Access::Revoked`] means: the membership no longer holds. Refuse, and end
//!   the transfer ([`MembershipGate::end_transfer`]): the provider sends
//!   the DSP termination (its own step, ADR 0014), and the data plane cuts
//!   every running download.
//!
//! Flows of 1.0 (started before this file had memberships) have none
//! kept: the migration marks them [`StoredMembership::StartedBefore1_1`].
//! They are served as in 1.0, until their token expires (at most
//! `PORTITOR_DATA_TOKEN_SECONDS`), and logged once. A flow of 1.1 with
//! nothing kept (the write failed, or the caller verifier gave no
//! membership) is refused: fail closed.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use crate::auth::{AnyVerifier, Membership, VerifyError};
use crate::data_plane::{FlowStore, FlowStoreError};
use crate::ports::AnyFlowStore;

/// What the flow store keeps of a flow's consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredMembership {
    /// Nothing: a flow of 1.1 whose memberships were not kept. Refused.
    Missing,
    /// A flow that started before 1.1: served until its token expires.
    StartedBefore1_1,
    /// The consumer's memberships when it asked for the transfer.
    Kept(Vec<Membership>),
}

/// The answer of [`MembershipGate::check`].
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

/// Ends a transfer on the provider's side: the DSP termination is sent
/// and the data flow terminated. The provider's transfer service
/// implements it; the gate calls it when a membership no longer holds.
pub trait EndsTransfers: Send + Sync {
    /// Ends transfer `provider_pid` for `reason`. Failures are logged.
    fn end_transfer(&self, provider_pid: &str, reason: &str);
}

/// Checks the memberships kept with a data flow (module doc).
pub struct MembershipGate {
    flows: AnyFlowStore,
    verifier: AnyVerifier,
    /// Set once the provider's transfer service exists (it is built after
    /// the data plane).
    ender: OnceLock<Arc<dyn EndsTransfers>>,
    /// The flows of 1.0 already logged, so each is logged once.
    logged_before_1_1: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for MembershipGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MembershipGate").finish_non_exhaustive()
    }
}

impl MembershipGate {
    /// A gate that reads the memberships from `flows` and checks them with
    /// `verifier`.
    #[must_use]
    pub fn new(flows: AnyFlowStore, verifier: AnyVerifier) -> Self {
        Self {
            flows,
            verifier,
            ender: OnceLock::new(),
            logged_before_1_1: Mutex::new(BTreeSet::new()),
        }
    }

    /// Who ends a transfer whose consumer is no longer a member. Set once;
    /// a second call changes nothing.
    pub fn ends_transfers_with(&self, ender: Arc<dyn EndsTransfers>) {
        let _ = self.ender.set(ender);
    }

    /// How often a running download or push checks again.
    #[must_use]
    pub fn recheck_interval(&self) -> Duration {
        self.verifier.membership_recheck_interval()
    }

    /// Whether the consumer of flow `flow_id` may still get its data
    /// (module doc). Every membership kept must pass: the caller check
    /// refuses a presentation with one failing credential too (ADR 0016).
    pub async fn check(&self, flow_id: &str) -> Access {
        let memberships = match self.flows.memberships(flow_id) {
            Ok(StoredMembership::Kept(memberships)) => memberships,
            Ok(StoredMembership::StartedBefore1_1) => {
                self.log_before_1_1(flow_id);
                return Access::Granted;
            }
            Ok(StoredMembership::Missing) => {
                return Access::Revoked("no membership was kept for this data flow".to_owned());
            }
            Err(FlowStoreError(error)) => {
                return Access::Unchecked(format!("flow store: {error}"));
            }
        };
        if memberships.is_empty() {
            return Access::Revoked("no membership was kept for this data flow".to_owned());
        }
        for membership in &memberships {
            match self.verifier.check_membership(membership).await {
                Ok(()) => {}
                Err(VerifyError::Unavailable(reason)) => return Access::Unchecked(reason),
                Err(error) => return Access::Revoked(error.reason().to_owned()),
            }
        }
        Access::Granted
    }

    /// Ends the transfer of flow `flow_id`, whose consumer is no longer a
    /// member, and logs why (WARN). Without a transfer service (tests),
    /// only the log line.
    pub fn end_transfer(&self, flow_id: &str, reason: &str) {
        tracing::warn!(
            flow = %flow_id,
            %reason,
            "data flow ended: the consumer's membership no longer holds"
        );
        if let Some(ender) = self.ender.get() {
            ender.end_transfer(flow_id, reason);
        }
    }

    /// One INFO line per flow of 1.0 and process: it is served without a
    /// membership check.
    fn log_before_1_1(&self, flow_id: &str) {
        let first_time = self
            .logged_before_1_1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(flow_id.to_owned());
        if first_time {
            tracing::info!(
                flow = %flow_id,
                "data flow started before 1.1: no membership kept; served until its token expires"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use portitor_dsp::policy::HeldCredential;
    use portitor_runtime::BoxFuture;

    use super::*;
    use crate::auth::{Caller, CallerVerifier};
    use crate::ports::Shared;
    use crate::test_stores::InMemoryFlows;

    /// Takes a membership from `did:web:issuer` while `revoked` is false;
    /// from `did:web:down` the check cannot finish.
    #[derive(Default)]
    struct Issuer {
        revoked: AtomicBool,
    }

    impl CallerVerifier for Issuer {
        fn verify<'a>(&'a self, _: Option<&'a str>) -> BoxFuture<'a, Result<Caller, VerifyError>> {
            Box::pin(async { Err(VerifyError::from("not used")) })
        }
        fn credentials<'a>(
            &'a self,
            _: &'a Caller,
            _: &'a [String],
        ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn check_membership<'a>(
            &'a self,
            membership: &'a Membership,
        ) -> BoxFuture<'a, Result<(), VerifyError>> {
            let answer = match membership.issuer.as_str() {
                "did:web:down" => Err(VerifyError::Unavailable("no answer".to_owned())),
                _ if self.revoked.load(Ordering::SeqCst) => {
                    Err(VerifyError::from("a credential is revoked"))
                }
                _ => Ok(()),
            };
            Box::pin(async move { answer })
        }
    }

    fn membership(issuer: &str) -> Membership {
        Membership {
            issuer: issuer.to_owned(),
            valid_until: None,
            status: Vec::new(),
        }
    }

    fn gate() -> (MembershipGate, InMemoryFlows, Arc<Issuer>) {
        let flows = InMemoryFlows::default();
        let issuer = Arc::new(Issuer::default());
        let store: AnyFlowStore = Shared(Arc::new(flows.clone()) as _);
        let gate = MembershipGate::new(store, Arc::clone(&issuer) as _);
        (gate, flows, issuer)
    }

    #[tokio::test]
    async fn a_kept_membership_is_checked_again() {
        let (gate, flows, issuer) = gate();
        flows
            .keep_memberships("f1", &[membership("did:web:issuer")])
            .unwrap();
        assert_eq!(gate.check("f1").await, Access::Granted);
        issuer.revoked.store(true, Ordering::SeqCst);
        assert!(matches!(gate.check("f1").await, Access::Revoked(r) if r.contains("revoked")));
    }

    #[tokio::test]
    async fn every_kept_membership_must_pass() {
        let (gate, flows, _) = gate();
        let both = [membership("did:web:issuer"), membership("did:web:down")];
        flows.keep_memberships("f1", &both).unwrap();
        assert!(matches!(gate.check("f1").await, Access::Unchecked(_)));
    }

    #[tokio::test]
    async fn a_flow_without_memberships_is_refused_and_one_of_1_0_is_served() {
        let (gate, flows, _) = gate();
        assert!(matches!(gate.check("unknown").await, Access::Revoked(_)));
        flows.keep_memberships("empty", &[]).unwrap();
        assert!(matches!(gate.check("empty").await, Access::Revoked(_)));
        flows.started_before_1_1("old");
        assert_eq!(gate.check("old").await, Access::Granted);
        assert_eq!(gate.check("old").await, Access::Granted);
        assert_eq!(gate.logged_before_1_1.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_transfer_is_ended_through_the_ender() {
        #[derive(Default)]
        struct Ender(Mutex<Vec<String>>);
        impl EndsTransfers for Ender {
            fn end_transfer(&self, pid: &str, reason: &str) {
                self.0.lock().unwrap().push(format!("{pid}: {reason}"));
            }
        }
        let (gate, _, _) = gate();
        gate.end_transfer("f1", "nobody to tell yet");
        let ender = Arc::new(Ender::default());
        gate.ends_transfers_with(Arc::clone(&ender) as _);
        gate.end_transfer("f1", "revoked");
        assert_eq!(*ender.0.lock().unwrap(), ["f1: revoked"]);
    }
}
