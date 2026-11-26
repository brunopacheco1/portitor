// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter for tests: flows kept in memory. Built for this crate's tests
//! and, with the `test-support` feature, for the tests of the crates that
//! use it; never in a release binary.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::DataAddress;

use crate::{Flow, FlowStore, FlowStoreError};

/// Adapter: flows kept in memory (lost on restart; tests).
#[derive(Debug, Clone, Default)]
pub struct InMemoryFlows(Arc<Mutex<BTreeMap<String, Flow>>>);

impl InMemoryFlows {
    fn locked_flows(&self) -> MutexGuard<'_, BTreeMap<String, Flow>> {
        // every write is a single insert, so a panic cannot leave half a flow
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl FlowStore for InMemoryFlows {
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError> {
        Ok(self.locked_flows().get(id).cloned())
    }

    fn insert(&self, flow: &Flow) -> Result<bool, FlowStoreError> {
        let mut flows = self.locked_flows();
        if flows.contains_key(&flow.id) {
            return Ok(false);
        }
        flows.insert(flow.id.clone(), flow.clone());
        Ok(true)
    }

    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError> {
        let mut flows = self.locked_flows();
        match flows.get_mut(id) {
            Some(flow) if flow.state == from => {
                flow.state = to;
                if to.is_final() {
                    flow.push_address = None;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError> {
        let mut flows = self.locked_flows();
        match flows.get_mut(id) {
            Some(flow) if flow.state == state => {
                token_hash.clone_into(&mut flow.token_hash);
                flow.expires_at = expires_at;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError> {
        let mut flows = self.locked_flows();
        match flows.get_mut(id) {
            Some(flow) if flow.state == state && flow.push_address.is_some() => {
                flow.push_address = Some(address.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError> {
        let mut started = Vec::new();
        for flow in self.locked_flows().values() {
            if flow.state == DataFlowState::Started && flow.push_address.is_some() {
                started.push(flow.id.clone());
            }
        }
        Ok(started)
    }
}
