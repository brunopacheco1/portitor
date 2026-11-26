// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! In-memory stores for the unit tests (TD-42): negotiations as provider
//! and as consumer, transfers as consumer, the agreements the consumer
//! holds, and the data plane's flows. They are compiled only for tests; the
//! connector itself always keeps these in its database (`store.rs`, an
//! in-memory `SQLite` database in the integration tests).
//!
//! Each store keeps its records in a `BTreeMap` behind a `Mutex`, and
//! does what the port asks in one lock, as the database adapter does in
//! one statement or transaction.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use portitor_dps::state::DataFlowState;
use portitor_dsp::negotiation::consumer::{ConsumerNegotiation, ConsumerStore};
use portitor_dsp::negotiation::provider::{Negotiation, NegotiationStore, StoreError};
use portitor_dsp::negotiation::{Agreement, NegotiationState};
use portitor_dsp::transfer::DataAddress;
use portitor_dsp::transfer::consumer::{
    AgreementProvider, ConsumerTransfer, ConsumerTransferStore, HeldAgreements,
};

use crate::auth::Membership;
use crate::flow_membership::{MembershipStore, StoredMembership};
use crate::http_client::Sender;
use crate::ports::Ports;
use crate::store::ConnectorStore;
use portitor_dataplane::{Flow, FlowStore, FlowStoreError};

impl Ports {
    /// The ports for a unit test: every store in a new in-memory `SQLite`
    /// database, as [`Ports::new`] does.
    #[allow(clippy::unwrap_used)]
    pub(crate) fn in_memory(sender: Arc<dyn Sender>) -> Self {
        Ports::new(sender, &Arc::new(ConnectorStore::in_memory().unwrap()))
    }
}

/// Adapter: negotiations kept in memory (lost on restart). The database
/// adapter (`ConnectorStore`) replaces it for real use.
#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryNegotiations(Arc<Mutex<BTreeMap<String, Negotiation>>>);

impl InMemoryNegotiations {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Negotiation>> {
        // a panic while holding the lock cannot leave a half-written entry:
        // every write is a single insert
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NegotiationStore for InMemoryNegotiations {
    fn get(&self, provider_pid: &str) -> Result<Option<Negotiation>, StoreError> {
        Ok(self.map().get(provider_pid).cloned())
    }

    fn insert(&self, negotiation: &Negotiation) -> Result<(), StoreError> {
        let mut map = self.map();
        if map.contains_key(&negotiation.provider_pid) {
            return Err(StoreError(format!("{} exists", negotiation.provider_pid)));
        }
        map.insert(negotiation.provider_pid.clone(), negotiation.clone());
        Ok(())
    }

    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        let mut map = self.map();
        match map.get(&negotiation.provider_pid) {
            Some(old) if old.version == expected => {
                map.insert(negotiation.provider_pid.clone(), negotiation.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        Ok(self
            .map()
            .values()
            .filter(|negotiation| negotiation.pending.is_some())
            .cloned()
            .collect())
    }
}

/// Adapter: consumer negotiations kept in memory (lost on restart).
#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryConsumerNegotiations(Arc<Mutex<BTreeMap<String, ConsumerNegotiation>>>);

impl InMemoryConsumerNegotiations {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, ConsumerNegotiation>> {
        // every write is a single insert, so a panic cannot leave half an entry
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ConsumerStore for InMemoryConsumerNegotiations {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerNegotiation>, StoreError> {
        Ok(self.map().get(consumer_pid).cloned())
    }

    fn insert(&self, negotiation: &ConsumerNegotiation) -> Result<(), StoreError> {
        let mut map = self.map();
        if map.contains_key(&negotiation.consumer_pid) {
            return Err(StoreError(format!("{} exists", negotiation.consumer_pid)));
        }
        map.insert(negotiation.consumer_pid.clone(), negotiation.clone());
        Ok(())
    }

    fn replace_if(
        &self,
        negotiation: &ConsumerNegotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        let mut map = self.map();
        match map.get(&negotiation.consumer_pid) {
            Some(old) if old.version == expected => {
                map.insert(negotiation.consumer_pid.clone(), negotiation.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        Ok(page(&self.map(), after, limit))
    }

    fn pending(&self) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        Ok(self
            .map()
            .values()
            .filter(|negotiation| negotiation.pending.is_some())
            .cloned()
            .collect())
    }
}

/// Up to `limit` values of `map` whose keys sort after `after`, in key
/// order: one page for the stores in this file.
fn page<V: Clone>(map: &BTreeMap<String, V>, after: Option<&str>, limit: usize) -> Vec<V> {
    let range = match after {
        Some(a) => map.range::<str, _>((std::ops::Bound::Excluded(a), std::ops::Bound::Unbounded)),
        None => map.range::<str, _>(..),
    };
    range.take(limit).map(|(_, v)| v.clone()).collect()
}

/// Adapter: consumer transfers kept in memory (lost on restart). The
/// database adapter replaces it for real use.
#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryConsumerTransfers(Arc<Mutex<BTreeMap<String, ConsumerTransfer>>>);

impl InMemoryConsumerTransfers {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, ConsumerTransfer>> {
        // every write is a single insert, so a panic cannot leave half a
        // transfer behind
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ConsumerTransferStore for InMemoryConsumerTransfers {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerTransfer>, StoreError> {
        Ok(self.map().get(consumer_pid).cloned())
    }

    fn insert(&self, transfer: &ConsumerTransfer) -> Result<(), StoreError> {
        let mut map = self.map();
        if map.contains_key(&transfer.consumer_pid) {
            return Err(StoreError(format!("{} exists", transfer.consumer_pid)));
        }
        map.insert(transfer.consumer_pid.clone(), transfer.clone());
        Ok(())
    }

    fn replace_if(&self, transfer: &ConsumerTransfer, expected: u64) -> Result<bool, StoreError> {
        let mut map = self.map();
        let current = map.get(&transfer.consumer_pid).map(|old| old.version);
        if current != Some(expected) {
            return Ok(false);
        }
        map.insert(transfer.consumer_pid.clone(), transfer.clone());
        Ok(true)
    }

    fn pending(&self) -> Result<Vec<ConsumerTransfer>, StoreError> {
        let map = self.map();
        Ok(map
            .values()
            .filter(|transfer| transfer.pending.is_some())
            .cloned()
            .collect())
    }

    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<ConsumerTransfer>, StoreError> {
        Ok(page(&self.map(), after, limit))
    }
}

impl HeldAgreements for InMemoryConsumerNegotiations {
    fn provider_of(&self, agreement_id: &str) -> Result<Option<AgreementProvider>, StoreError> {
        let map = self.map();
        Ok(map
            .values()
            .find(|negotiation| {
                negotiation.state == NegotiationState::Finalized
                    && negotiation
                        .agreement
                        .as_ref()
                        .is_some_and(|a| a.id == agreement_id)
            })
            .map(|negotiation| AgreementProvider {
                address: negotiation.provider_address.clone(),
                id: negotiation.provider_id.clone(),
            }))
    }

    fn agreement(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        let map = self.map();
        for negotiation in map.values() {
            if negotiation.state != NegotiationState::Finalized {
                continue;
            }
            if let Some(agreement) = &negotiation.agreement
                && agreement.id == agreement_id
            {
                return Ok(Some(agreement.clone()));
            }
        }
        Ok(None)
    }
}

/// Adapter: flows kept in memory (lost on restart; tests).
#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryFlows {
    /// The data plane's own in-memory flows.
    flows: portitor_dataplane::test_store::InMemoryFlows,
    /// What the control plane keeps of each flow's consumer (ADR 0027).
    memberships: Arc<Mutex<BTreeMap<String, StoredMembership>>>,
}

impl InMemoryFlows {
    fn members(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, StoredMembership>> {
        self.memberships
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks flow `id` as one of 1.0, as the migration does.
    pub(crate) fn started_before_1_1(&self, id: &str) {
        self.members()
            .insert(id.to_owned(), StoredMembership::StartedBefore1_1);
    }
}

impl FlowStore for InMemoryFlows {
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError> {
        self.flows.get(id)
    }
    fn insert(&self, flow: &Flow) -> Result<bool, FlowStoreError> {
        self.flows.insert(flow)
    }
    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError> {
        self.flows.set_state(id, from, to)
    }
    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError> {
        self.flows.set_token(id, state, token_hash, expires_at)
    }
    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError> {
        self.flows.set_push_address(id, state, address)
    }
    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError> {
        self.flows.started_pushes()
    }
}

impl MembershipStore for InMemoryFlows {
    fn memberships(&self, id: &str) -> Result<StoredMembership, FlowStoreError> {
        let found = self.members().get(id).cloned();
        Ok(found.unwrap_or(StoredMembership::Missing))
    }

    fn keep_memberships(&self, id: &str, memberships: &[Membership]) -> Result<(), FlowStoreError> {
        let kept = StoredMembership::Kept(memberships.to_vec());
        self.members().entry(id.to_owned()).or_insert(kept);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dsp::Offer;
    use portitor_dsp::negotiation::Transition;
    use portitor_dsp::negotiation::provider::Outgoing;
    use portitor_dsp::transfer::TransferState;

    use super::*;
    use crate::ports::{
        AnyConsumerStore, AnyConsumerTransferStore, AnyHeldAgreements, AnyStore, Shared,
    };

    fn negotiation(state: NegotiationState) -> Negotiation {
        Negotiation {
            provider_pid: "p".into(),
            consumer_pid: "c".into(),
            state,
            callback_address: "https://c.example".into(),
            counter_party: "did:web:c.example".into(),
            offer: serde_json::from_value::<Offer>(
                serde_json::json!({"@id": "o", "@type": "Offer", "target": "d"}),
            )
            .unwrap(),
            agreement: None,
            pending: None,
            version: 0,
        }
    }
    #[test]
    fn in_memory_store_compares_before_replacing() {
        let store: AnyStore = Shared(Arc::new(InMemoryNegotiations::default()));
        let n = negotiation(NegotiationState::Requested);
        assert!(store.get("p").unwrap().is_none());
        store.insert(&n).unwrap();
        assert!(store.insert(&n).is_err());
        let mut offered = negotiation(NegotiationState::Offered);
        offered.version = 1;
        assert!(!store.replace_if(&offered, 1).unwrap());
        assert!(store.replace_if(&offered, 0).unwrap());
        assert_eq!(
            store.get("p").unwrap().unwrap().state,
            NegotiationState::Offered
        );
        // a second write from the same read loses
        assert!(!store.replace_if(&offered, 0).unwrap());
        let mut other = offered.clone();
        other.provider_pid = "q".into();
        assert!(!store.replace_if(&other, 0).unwrap());
        assert!(store.pending().unwrap().is_empty());
        offered.version = 2;
        offered.pending = Some(Outgoing {
            provider_pid: "p".into(),
            url: "https://c.example/negotiations/c/agreement".into(),
            body: serde_json::json!({}),
            transition: Transition::Agree,
            to: NegotiationState::Agreed,
            status_url: "https://c.example/negotiations/c".into(),
            counter_party: "did:web:c.example".into(),
        });
        assert!(store.replace_if(&offered, 1).unwrap());
        assert_eq!(store.pending().unwrap(), vec![offered]);
    }

    fn consumer_negotiation(state: NegotiationState) -> ConsumerNegotiation {
        ConsumerNegotiation {
            consumer_pid: "c".into(),
            provider_pid: None,
            state,
            provider_address: "https://p.example/dsp".into(),
            provider_id: "did:web:p.example".into(),
            offer: serde_json::from_value(
                serde_json::json!({"@id": "o", "@type": "Offer", "target": "d"}),
            )
            .unwrap(),
            agreement: None,
            pending: None,
            version: 0,
        }
    }

    #[test]
    fn consumer_negotiations_compare_before_replacing() {
        let store: AnyConsumerStore = Shared(Arc::new(InMemoryConsumerNegotiations::default()));
        let n = consumer_negotiation(NegotiationState::Requested);
        assert!(store.get("c").unwrap().is_none());
        store.insert(&n).unwrap();
        assert!(store.insert(&n).is_err());
        let mut offered = consumer_negotiation(NegotiationState::Offered);
        offered.version = 1;
        assert!(!store.replace_if(&offered, 1).unwrap());
        assert!(store.replace_if(&offered, 0).unwrap());
        assert!(!store.replace_if(&offered, 0).unwrap());
        assert!(store.pending().unwrap().is_empty());
        let mut other = offered;
        other.consumer_pid = "d".into();
        assert!(!store.replace_if(&other, 1).unwrap());
        store.insert(&other).unwrap();
        let all = store.list(None, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(store.list(Some("c"), 10).unwrap()[0].consumer_pid, "d");
        assert!(store.list(Some("d"), 10).unwrap().is_empty());
        assert_eq!(store.list(None, 1).unwrap().len(), 1);
        assert_eq!(format!("{store:?}"), "Shared");
    }

    fn transfer(pid: &str) -> ConsumerTransfer {
        ConsumerTransfer {
            consumer_pid: pid.to_owned(),
            provider_pid: None,
            state: TransferState::Requested,
            provider_address: "https://p.example/dsp".to_owned(),
            provider_id: "did:web:p.example".to_owned(),
            agreement_id: "a".to_owned(),
            format: "HttpData-PULL".to_owned(),
            data_address: None,
            pending: None,
            version: 0,
        }
    }

    #[test]
    fn in_memory_transfers_compare_before_replacing() {
        let store: AnyConsumerTransferStore =
            Shared(Arc::new(InMemoryConsumerTransfers::default()));
        let t = transfer("c");
        assert!(store.get("c").unwrap().is_none());
        store.insert(&t).unwrap();
        assert!(store.insert(&t).is_err());
        let mut started = t.clone();
        started.state = TransferState::Started;
        started.version = 1;
        assert!(!store.replace_if(&started, 1).unwrap());
        assert!(store.replace_if(&started, 0).unwrap());
        assert!(store.pending().unwrap().is_empty());
        assert!(!store.replace_if(&transfer("d"), 0).unwrap(), "unknown");
        store.insert(&transfer("d")).unwrap();
        assert_eq!(store.list(None, 10).unwrap().len(), 2);
        assert_eq!(store.list(Some("c"), 10).unwrap(), vec![transfer("d")]);
    }

    #[test]
    fn in_memory_negotiations_hold_only_finalized_agreements() {
        let negotiations = InMemoryConsumerNegotiations::default();
        let held: AnyHeldAgreements = Shared(Arc::new(negotiations.clone()));
        let agreement = serde_json::json!({
            "@id": "a", "@type": "Agreement", "target": "d",
            "assigner": "did:web:p.example", "assignee": "did:web:c.example",
            "timestamp": "2026-11-13T00:00:00Z", "permission": [{"action": "use"}]
        });
        let mut n = ConsumerNegotiation {
            consumer_pid: "n".to_owned(),
            provider_pid: Some("pn".to_owned()),
            state: NegotiationState::Agreed,
            provider_address: "https://p.example/dsp".to_owned(),
            provider_id: "did:web:p.example".to_owned(),
            offer: serde_json::from_value(
                serde_json::json!({"@id": "o", "@type": "Offer", "target": "d"}),
            )
            .unwrap(),
            agreement: Some(serde_json::from_value(agreement).unwrap()),
            pending: None,
            version: 0,
        };
        negotiations.insert(&n).unwrap();
        assert_eq!(held.provider_of("a").unwrap(), None);
        assert_eq!(held.agreement("a").unwrap(), None);
        n.state = NegotiationState::Finalized;
        n.version = 1;
        assert!(negotiations.replace_if(&n, 0).unwrap());
        let provider = held.provider_of("a").unwrap().unwrap();
        assert_eq!(provider.address, "https://p.example/dsp");
        assert_eq!(held.provider_of("b").unwrap(), None);
        assert_eq!(held.agreement("a").unwrap(), n.agreement);
        assert_eq!(held.agreement("b").unwrap(), None);
    }
}
