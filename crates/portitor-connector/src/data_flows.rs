// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Adapter: the transfer process's [`DataFlows`] port, spoken as Data
//! Plane Signaling v1.0-RC4 to a [`DataPlane`] (ADR 0003, ADR 0013). The
//! data plane runs in this process, so the signals are function calls.
//!
//! Every call is safe to repeat (ADR 0013): a signal the data plane
//! refuses because the flow is already where the signal leads counts as
//! done, and so does ending a flow that never started. A start of a flow
//! that exists gives it a new token (`start`). The transfer service
//! calls them after its writes (ADR 0028, decision 6).

use std::sync::Arc;

use portitor_dps::message::{
    DataFlowBaseMessage, DataFlowStartMessage, HTTP_PULL, HTTP_PUSH, ReasonMessage,
};
use portitor_dps::port::{DataPlane, SignalError};
use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::DataAddress;
use portitor_dsp::transfer::provider::{DataFlows, Transfer};
use serde_json::Value;

use crate::flow_membership::{MembershipStore, StoredMembership};
use crate::harvest::UpstreamSources;
use crate::system::new_uuid;
use portitor_dataplane::SOURCE_METADATA_KEY;

/// The DSP formats the built-in data plane serves, with the Data Plane
/// Signaling profile of each: pull since 0.4, push since 1.1 (ADR 0027).
const FORMATS: &[(&str, &str)] = &[
    ("HttpData-PULL", HTTP_PULL),
    (portitor_dsp::transfer::HTTP_DATA_PUSH, HTTP_PUSH),
];

/// The Data Plane Signaling profile for a DSP format, if supported.
fn profile_of(format: &str) -> Option<&'static str> {
    for (name, profile) in FORMATS {
        if *name == format {
            return Some(*profile);
        }
    }
    None
}

/// What a data plane needs besides Data Plane Signaling v1.0-RC4 so the
/// provider can start a pull transfer again: a new token, and the address
/// that carries it (ADR 0014).
pub trait NewAddress {
    /// A new token for data flow `id`; the old one stops working.
    ///
    /// # Errors
    ///
    /// Unknown or ended flow, or the data plane failed.
    fn new_address(&self, id: &str) -> Result<DataAddress, SignalError>;
}

impl NewAddress for portitor_dataplane::HttpDataPlane {
    fn new_address(&self, id: &str) -> Result<DataAddress, SignalError> {
        portitor_dataplane::HttpDataPlane::new_address(self, id)
    }
}

/// Adapter: [`DataFlows`] over a Data Plane Signaling [`DataPlane`].
#[derive(Clone)]
pub struct SignalingDataFlows<P> {
    plane: P,
    participant_id: String,
    dataspace_context: String,
    /// Where each dataset's data comes from: the datasets with a source
    /// are the ones the data plane can serve. Each start message names it.
    sources: Arc<dyn UpstreamSources>,
    /// What the control plane kept of each flow's consumer: a flow of 1.0
    /// is not started again (ADR 0027, section 7). `None` in unit tests.
    memberships: Option<Arc<dyn MembershipStore + Send + Sync>>,
}

impl<P> std::fmt::Debug for SignalingDataFlows<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalingDataFlows")
            .field("participant_id", &self.participant_id)
            .finish_non_exhaustive()
    }
}

impl<P: DataPlane> SignalingDataFlows<P> {
    /// Signals `plane` on behalf of provider `participant_id`, in the
    /// dataspace named `dataspace_context`. `sources`: each dataset's
    /// source, read when a transfer starts and named in the start message;
    /// a start for a dataset without one is refused.
    pub fn new(
        plane: P,
        participant_id: &str,
        dataspace_context: &str,
        sources: Arc<dyn UpstreamSources>,
    ) -> Self {
        Self {
            plane,
            participant_id: participant_id.to_owned(),
            dataspace_context: dataspace_context.to_owned(),
            sources,
            memberships: None,
        }
    }

    /// The same adapter, refusing to start again a flow that started before
    /// 1.1, by what `memberships` kept of it.
    #[must_use]
    pub fn with_memberships(mut self, memberships: Arc<dyn MembershipStore + Send + Sync>) -> Self {
        self.memberships = Some(memberships);
        self
    }

    /// A flow of 1.0 is served without a membership check only until its
    /// token expires (ADR 0027, section 7): it gets no new token, which
    /// would keep it unchecked for good. The consumer asks for a new
    /// transfer instead.
    fn refuse_flow_of_1_0(&self, pid: &str) -> Result<(), String> {
        let Some(memberships) = &self.memberships else {
            return Ok(());
        };
        let kept = memberships
            .memberships(pid)
            .map_err(|e| format!("data plane failed: flow store: {}", e.0))?;
        if kept == StoredMembership::StartedBefore1_1 {
            return Err(format!(
                "invalid signal: data flow {pid} started before 1.1 and has no membership to check; \
                 it is not started again: ask for a new transfer"
            ));
        }
        Ok(())
    }

    /// The start message's `metadata`: the dataset's source, under
    /// [`SOURCE_METADATA_KEY`] (maintainer's decision D2; Data Plane
    /// Signaling v1.0-RC4 gives `metadata` no fixed shape). `None` when the
    /// dataset has none: the data plane then refuses the start. An error
    /// when the dataset cannot be read now.
    fn source_metadata(
        &self,
        dataset_id: &str,
    ) -> Result<Option<serde_json::Map<String, Value>>, String> {
        let Some(source) = self.sources.upstream_of(dataset_id)? else {
            return Ok(None);
        };
        let mut metadata = serde_json::Map::new();
        metadata.insert(SOURCE_METADATA_KEY.to_owned(), Value::String(source));
        Ok(Some(metadata))
    }

    /// A reason message with a fresh id.
    fn reason(why: &str) -> ReasonMessage {
        ReasonMessage {
            message_id: new_uuid(),
            reason: Some(why.to_owned()),
        }
    }

    /// Turns the data plane's answer to a signal into the port's answer.
    /// A refusal is still success when the flow is already in `target`.
    fn done_if_already_in(
        &self,
        pid: &str,
        target: DataFlowState,
        answer: Result<(), SignalError>,
    ) -> Result<(), String> {
        match answer {
            Ok(()) => Ok(()),
            Err(SignalError::NotAllowed(_)) if self.flow_is_in(pid, target) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Whether the data plane says flow `pid` is in `state`.
    fn flow_is_in(&self, pid: &str, state: DataFlowState) -> bool {
        self.plane.status(pid).is_ok_and(|s| s.state == state)
    }
}

impl<P: DataPlane + NewAddress> DataFlows for SignalingDataFlows<P> {
    fn supports(&self, dataset_id: &str, format: &str) -> Result<bool, String> {
        if profile_of(format).is_none() {
            return Ok(false);
        }
        Ok(self.sources.upstream_of(dataset_id)?.is_some())
    }

    fn start(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String> {
        let Some(profile) = profile_of(&transfer.format) else {
            return Err("unsupported format".to_owned());
        };
        let push = profile == HTTP_PUSH;
        let message = DataFlowStartMessage {
            flow: DataFlowBaseMessage {
                message_id: new_uuid(),
                participant_id: self.participant_id.clone(),
                counter_party_id: transfer.counter_party.clone(),
                dataspace_context: self.dataspace_context.clone(),
                data_flow_id: transfer.provider_pid.clone(),
                agreement_id: transfer.agreement_id.clone(),
                dataset_id: transfer.dataset_id.clone(),
                profile: profile.to_owned(),
                // the consumer's verified claims come with DCP (0.5)
                claims: serde_json::Map::new(),
                metadata: self.source_metadata(&transfer.dataset_id)?,
                labels: Vec::new(),
            },
            // pull: the data plane makes the address (the spec forbids one
            // here); push: where the data plane sends the data
            data_address: if push {
                transfer.data_address.clone()
            } else {
                None
            },
        };
        let status = match self.plane.start(&message) {
            Ok(status) => status,
            // The flow exists: the process stopped after this start and
            // before the transfer recorded it, and the start runs again
            // after the restart (`portitor-dsp`, `flow_steps`). Its token
            // was never sent, and only its hash is kept, so the flow gets
            // a new one, as when the provider starts it again.
            Err(SignalError::DataFlowExists(_)) => return self.restart(transfer),
            Err(e) => return Err(e.to_string()),
        };
        match (status.state, status.data_address) {
            (DataFlowState::Started, _) if push => Ok(None),
            (DataFlowState::Started, Some(address)) => Ok(Some(address)),
            (DataFlowState::Started, None) => Err("the data plane gave no data address".to_owned()),
            (other, _) => Err(format!(
                "the data plane answered {other:?}; only an immediate start is supported"
            )),
        }
    }

    fn suspend(&self, pid: &str) -> Result<(), String> {
        let answer = self.plane.suspend(pid, &Self::reason("suspended over DSP"));
        self.done_if_already_in(pid, DataFlowState::Suspended, answer)
    }

    fn resume(&self, pid: &str, address: Option<&DataAddress>) -> Result<(), String> {
        let message = portitor_dps::message::AddressMessage {
            message_id: new_uuid(),
            // a push started again with a new token of the consumer's
            data_address: address.cloned(),
        };
        // the status message is dropped: a resume gives no new address
        // (the same token works again), so only success or failure counts
        let answer = self.plane.resume(pid, &message).map(|_| ());
        self.done_if_already_in(pid, DataFlowState::Started, answer)
    }

    fn restart(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String> {
        let pid = transfer.provider_pid.as_str();
        if profile_of(&transfer.format) == Some(HTTP_PUSH) {
            // the data goes to the consumer's address the flow keeps
            self.resume(pid, None)?;
            return Ok(None);
        }
        self.refuse_flow_of_1_0(pid)?;
        // the new token first, so the old one never works again
        let address = self.plane.new_address(pid).map_err(|e| e.to_string())?;
        self.resume(pid, None)?;
        Ok(Some(address))
    }

    fn complete(&self, pid: &str) -> Result<(), String> {
        let answer = self.plane.completed(pid);
        self.done_if_already_in(pid, DataFlowState::Completed, answer)
    }

    fn terminate(&self, pid: &str) -> Result<(), String> {
        match self
            .plane
            .terminate(pid, &Self::reason("terminated over DSP"))
        {
            // a transfer that never started has no data flow to end
            Err(SignalError::UnknownDataFlow(_)) => Ok(()),
            answer => self.done_if_already_in(pid, DataFlowState::Terminated, answer),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use portitor_dps::message::{
        DataFlowPrepareMessage, DataFlowResumeMessage, DataFlowStartedNotificationMessage,
        DataFlowStatusMessage, DataFlowStatusResponseMessage, DataFlowSuspendMessage,
        DataFlowTerminateMessage,
    };
    use portitor_dsp::transfer::{DataAddressType, TransferState};

    use super::*;

    /// A data plane that keeps the state of each flow and follows the
    /// spec's state machine. `RefCell`: the `DataPlane` methods take
    /// `&self`, yet this plane changes what it holds; a `RefCell` lets it
    /// (it checks the borrows while the test runs).
    #[derive(Default)]
    struct Plane {
        /// The state of each flow.
        flows: RefCell<BTreeMap<String, DataFlowState>>,
        /// What `start` answers; `None`: STARTED.
        start_answer: RefCell<Option<DataFlowState>>,
        /// The last start message it got.
        last_start: RefCell<Option<DataFlowStartMessage>>,
    }

    impl Plane {
        /// Moves flow `id` to `to`, if the state machine allows it.
        fn move_flow(&self, id: &str, to: DataFlowState) -> Result<(), SignalError> {
            let mut flows = self.flows.borrow_mut();
            let from = *flows
                .get(id)
                .ok_or_else(|| SignalError::UnknownDataFlow(id.to_owned()))?;
            flows.insert(id.to_owned(), from.go_to(to)?);
            Ok(())
        }
    }

    // For `&Plane`, not `Plane`: the adapter takes its data plane by
    // value (`SignalingDataFlows::new(plane, ..)`), so it gets a reference
    // and the test can still look inside the plane afterwards.
    impl DataPlane for &Plane {
        fn prepare(
            &self,
            _: &DataFlowPrepareMessage,
        ) -> Result<DataFlowStatusMessage, SignalError> {
            Err(SignalError::Invalid("not used".to_owned()))
        }
        fn start(&self, m: &DataFlowStartMessage) -> Result<DataFlowStatusMessage, SignalError> {
            // a push start carries the consumer's address, a pull start none
            assert_eq!(m.flow.profile == HTTP_PUSH, m.data_address.is_some());
            let push = m.flow.profile == HTTP_PUSH;
            *self.last_start.borrow_mut() = Some(m.clone());
            if self.flows.borrow().contains_key(&m.flow.data_flow_id) {
                return Err(SignalError::DataFlowExists(m.flow.data_flow_id.clone()));
            }
            let state = self.start_answer.borrow().unwrap_or(DataFlowState::Started);
            self.flows
                .borrow_mut()
                .insert(m.flow.data_flow_id.clone(), state);
            Ok(DataFlowStatusMessage {
                message_id: "m".to_owned(),
                data_flow_id: m.flow.data_flow_id.clone(),
                state,
                data_address: (!push).then(|| DataAddress {
                    kind: DataAddressType,
                    endpoint_type: HTTP_PULL.to_owned(),
                    endpoint: Some("https://p.example/data".to_owned()),
                    endpoint_properties: vec![],
                }),
                error: None,
            })
        }
        fn started(
            &self,
            _: &str,
            _: &DataFlowStartedNotificationMessage,
        ) -> Result<(), SignalError> {
            Err(SignalError::Invalid("consumer side only".to_owned()))
        }
        fn suspend(&self, id: &str, _: &DataFlowSuspendMessage) -> Result<(), SignalError> {
            self.move_flow(id, DataFlowState::Suspended)
        }
        fn resume(
            &self,
            id: &str,
            _: &DataFlowResumeMessage,
        ) -> Result<DataFlowStatusMessage, SignalError> {
            self.move_flow(id, DataFlowState::Started)?;
            Ok(DataFlowStatusMessage {
                message_id: "m".to_owned(),
                data_flow_id: id.to_owned(),
                state: DataFlowState::Started,
                data_address: None,
                error: None,
            })
        }
        fn terminate(&self, id: &str, _: &DataFlowTerminateMessage) -> Result<(), SignalError> {
            self.move_flow(id, DataFlowState::Terminated)
        }
        fn completed(&self, id: &str) -> Result<(), SignalError> {
            self.move_flow(id, DataFlowState::Completed)
        }
        fn status(&self, id: &str) -> Result<DataFlowStatusResponseMessage, SignalError> {
            let state = *self
                .flows
                .borrow()
                .get(id)
                .ok_or_else(|| SignalError::UnknownDataFlow(id.to_owned()))?;
            Ok(DataFlowStatusResponseMessage {
                data_flow_id: id.to_owned(),
                state,
            })
        }
    }

    fn flows_over(plane: &Plane) -> SignalingDataFlows<&Plane> {
        let sources = BTreeMap::from([("d".to_owned(), "https://upstream.example/d".to_owned())]);
        SignalingDataFlows::new(plane, "did:web:p", "default", Arc::new(sources))
    }

    impl NewAddress for &Plane {
        fn new_address(&self, id: &str) -> Result<DataAddress, SignalError> {
            match self.status(id)?.state {
                DataFlowState::Suspended | DataFlowState::Started => Ok(DataAddress {
                    kind: DataAddressType,
                    endpoint_type: HTTP_PULL.to_owned(),
                    endpoint: Some("https://p.example/data".to_owned()),
                    endpoint_properties: vec![],
                }),
                other => Err(SignalError::Invalid(format!("{other:?}"))),
            }
        }
    }

    fn transfer(format: &str) -> Transfer {
        Transfer {
            provider_pid: "p".to_owned(),
            consumer_pid: "c".to_owned(),
            state: TransferState::Requested,
            callback_address: "https://c.example".to_owned(),
            counter_party: "did:web:c".to_owned(),
            agreement_id: "a".to_owned(),
            dataset_id: "d".to_owned(),
            format: format.to_owned(),
            data_address: None,
            pending: None,
            flow_step: None,
            state_before_step: None,
            lease: None,
            version: 0,
        }
    }

    /// A push transfer, with the consumer's address.
    fn push_transfer() -> Transfer {
        let mut push = transfer("HttpData-PUSH");
        push.data_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_PUSH.to_owned(),
            endpoint: Some("https://c.example/push/c".to_owned()),
            endpoint_properties: vec![],
        });
        push
    }

    #[test]
    fn supports_http_pull_and_push_of_known_datasets_only() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        assert!(flows.supports("d", "HttpData-PULL").unwrap());
        assert!(flows.supports("d", "HttpData-PUSH").unwrap());
        assert!(!flows.supports("d", "S3-PUSH").unwrap());
        assert!(
            !flows.supports("other", "HttpData-PULL").unwrap(),
            "no data for it"
        );
        assert!(flows.start(&transfer("S3-PUSH")).is_err());
    }

    #[test]
    fn a_push_flow_gets_the_consumers_address_and_gives_none() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        let push = push_transfer();
        assert_eq!(flows.start(&push).unwrap(), None);
        flows.suspend("p").unwrap();
        assert_eq!(flows.restart(&push).unwrap(), None, "started again");
        assert_eq!((&plane).status("p").unwrap().state, DataFlowState::Started);
    }

    #[test]
    fn a_full_flow_and_repeated_signals() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        let pull = transfer("HttpData-PULL");
        let address = flows.start(&pull).unwrap().unwrap();
        assert_eq!(address.endpoint.as_deref(), Some("https://p.example/data"));
        flows.suspend("p").unwrap();
        flows.suspend("p").unwrap(); // already suspended: done
        flows.resume("p", None).unwrap();
        flows.resume("p", None).unwrap(); // already started: done
        flows.suspend("p").unwrap();
        assert!(
            flows.restart(&pull).unwrap().is_some(),
            "a new address, then resumed"
        );
        assert_eq!((&plane).status("p").unwrap().state, DataFlowState::Started);
        flows.complete("p").unwrap();
        flows.complete("p").unwrap();
        // completed is final: terminating it is refused, not hidden
        assert!(flows.terminate("p").unwrap_err().contains("final"));
        assert!(flows.suspend("p").is_err());
        assert!(flows.restart(&pull).is_err(), "nothing to start again");
    }

    /// The process stopped after a start and before the transfer
    /// recorded it, so the start runs again after the restart (ADR 0028,
    /// decision 6): the flow exists, and gets a new token (the first was
    /// never sent). A push flow simply goes on.
    #[test]
    fn a_start_that_runs_again_gives_a_new_token() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        let pull = transfer("HttpData-PULL");
        flows.start(&pull).unwrap().unwrap();
        let again = flows.start(&pull).unwrap();
        assert!(again.is_some(), "a new address for the consumer");
        assert_eq!((&plane).status("p").unwrap().state, DataFlowState::Started);

        let plane = Plane::default();
        let flows = flows_over(&plane);
        let push = push_transfer();
        assert_eq!(flows.start(&push).unwrap(), None);
        assert_eq!(flows.start(&push).unwrap(), None);
        assert_eq!((&plane).status("p").unwrap().state, DataFlowState::Started);
    }

    /// The same with the built-in data plane: the start run again makes a
    /// new token, and the first one, never sent, opens nothing.
    #[test]
    fn with_the_built_in_plane_a_start_run_again_replaces_the_token() {
        let store = crate::test_stores::InMemoryFlows::default();
        let sources: Arc<dyn UpstreamSources> = Arc::new(BTreeMap::from([(
            "d".to_owned(),
            "https://upstream.example/d".to_owned(),
        )]));
        let plane =
            portitor_dataplane::HttpDataPlane::new(Arc::new(store), "https://p.example", 3_600)
                .unwrap();
        let flows = SignalingDataFlows::new(plane.clone(), "did:web:p", "default", sources);
        let pull = transfer("HttpData-PULL");
        let first = flows.start(&pull).unwrap().unwrap();
        let again = flows.start(&pull).unwrap().unwrap();
        let token = |address: &DataAddress| address.property("authorization").unwrap().to_owned();
        assert_ne!(token(&first), token(&again));
        // any time before the tokens expire
        let now = 1;
        assert!(
            plane
                .source_if_authorized("p", &token(&first), now)
                .is_err()
        );
        assert!(plane.source_if_authorized("p", &token(&again), now).is_ok());
    }

    #[test]
    fn ending_a_flow_that_never_started_is_done() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        flows.terminate("never").unwrap();
        assert!(flows.suspend("never").is_err());
    }

    #[test]
    fn only_an_immediate_start_is_supported() {
        let plane = Plane::default();
        *plane.start_answer.borrow_mut() = Some(DataFlowState::Starting);
        let flows = flows_over(&plane);
        assert!(
            flows
                .start(&transfer("HttpData-PULL"))
                .unwrap_err()
                .contains("Starting")
        );
    }

    /// The start message names the dataset's source in its `metadata`
    /// (ADR 0028, step 1; maintainer's decision D2), with the agreement
    /// and the consumer the data plane writes in the audit line.
    #[test]
    fn the_start_message_names_the_source_and_the_parties() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        for transfer in [transfer("HttpData-PULL"), push_transfer()] {
            plane.flows.borrow_mut().clear();
            flows.start(&transfer).unwrap();
            let sent = plane.last_start.borrow_mut().take().unwrap();
            let metadata = serde_json::to_value(&sent.flow.metadata).unwrap();
            assert_eq!(
                metadata,
                serde_json::json!({"portitor:source": "https://upstream.example/d"})
            );
            assert_eq!(sent.flow.agreement_id, "a");
            assert_eq!(sent.flow.counter_party_id, "did:web:c");
            // and on the wire, as Data Plane Signaling spells it
            let json = serde_json::to_value(&sent).unwrap();
            assert_eq!(
                json["metadata"]["portitor:source"],
                "https://upstream.example/d"
            );
        }
    }

    /// A dataset without a source gets a start message without one, which
    /// the data plane refuses.
    #[test]
    fn a_dataset_without_a_source_is_named_with_none() {
        let plane = Plane::default();
        let flows = flows_over(&plane);
        let mut other = transfer("HttpData-PULL");
        other.dataset_id = "other".to_owned();
        flows.start(&other).unwrap();
        let sent = plane.last_start.borrow_mut().take().unwrap();
        assert_eq!(sent.flow.metadata, None);
    }

    /// A flow of 1.0 is served without a membership check only until its
    /// token expires (ADR 0027, section 7): it gets no new token, which
    /// would keep it unchecked for good.
    #[test]
    fn a_flow_from_before_1_1_gets_no_new_token() {
        let plane = Plane::default();
        let memberships = crate::test_stores::InMemoryFlows::default();
        let flows = flows_over(&plane).with_memberships(Arc::new(memberships.clone()));
        let pull = transfer("HttpData-PULL");
        flows.start(&pull).unwrap();
        memberships.started_before_1_1("p");
        flows.suspend("p").unwrap();
        let refused = flows.restart(&pull).unwrap_err();
        assert!(refused.contains("before 1.1"), "{refused}");
    }
}
