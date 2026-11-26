// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The Data Plane Signaling port ([`DataPlane`]) of the built-in data
//! plane: the control plane's signals start, suspend, resume, complete
//! and terminate a flow (ADR 0013; ADR 0027 for push).
//!
//! - Starting a pull flow makes a random access token (32 bytes from the
//!   OS generator). Only its SHA-256 hash is stored. The token goes to the
//!   consumer once, inside the transfer's start message, in the data
//!   address this data plane answers with.
//! - Starting a push flow keeps the consumer's data address instead of a
//!   token of ours, until the flow ends.
//! - The start message names the dataset's source in its `metadata`,
//!   under [`SOURCE_METADATA_KEY`] (from 1.2; ADR 0028, step 1), and its
//!   agreement and consumer (`agreementId`, `counterPartyId`). The flow
//!   keeps them until it ends: a source changed later (a new harvest)
//!   reaches only new transfers. A start without a source this data plane
//!   can read is refused.
//! - Suspend and terminate also cut every download and push of the flow
//!   that runs ([`crate::running_flows::RunningFlows`]).

use portitor_dps::message::{
    DataFlowBaseMessage, DataFlowPrepareMessage, DataFlowResumeMessage, DataFlowStartMessage,
    DataFlowStartedNotificationMessage, DataFlowStatusMessage, DataFlowStatusResponseMessage,
    DataFlowSuspendMessage, DataFlowTerminateMessage, HTTP_PULL, HTTP_PUSH,
};
use portitor_dps::port::{DataPlane, SignalError};
use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::{DataAddress, DataAddressType, EndpointProperty, check_push_address};
use portitor_runtime::system::{new_uuid, now_secs};

use crate::DATA_PATH;
use crate::flows::{Flow, hash, new_token};
use crate::object_storage::parse_s3_uri;
use crate::plane::HttpDataPlane;

/// The `endpointType` of the data address sent over DSP. It is the value
/// of the DSP 2025-1 examples, the DSP TCK and EDC's DSP tests (ADR 0013);
/// the EDC interop test in 0.5 confirms it.
pub const HTTP_ENDPOINT_TYPE: &str = "https://w3id.org/idsa/v4.1/HTTP";

/// The key, in the `metadata` of a Data Plane Signaling start message,
/// whose value is the dataset's source: an `http://` or `https://` URL, or
/// `s3://{bucket}/{key}`. It is the key operators write in the datasets
/// file. Data Plane Signaling v1.0-RC4 gives `metadata` no fixed shape and
/// names no key for this (maintainer's decision D2, 2026-11-25).
pub const SOURCE_METADATA_KEY: &str = "portitor:source";

impl HttpDataPlane {
    /// Moves flow `id` away from STARTED (to `to`) and cuts every download
    /// of it that runs (ADR 0027, section 5). Cut also when the move is
    /// refused: stopping a download is always safe.
    fn move_and_cut_downloads(&self, id: &str, to: DataFlowState) -> Result<(), SignalError> {
        let moved = self.move_to(id, to);
        let cut = self.running.cut(id);
        if cut > 0 {
            tracing::info!(flow = %id, downloads = cut, state = ?to, "running downloads cut");
        }
        moved
    }

    /// Moves flow `id` to `to`, if the state machine allows it.
    fn move_to(&self, id: &str, to: DataFlowState) -> Result<(), SignalError> {
        let flow = self.flow(id)?;
        flow.state.go_to(to)?;
        let moved = self
            .flows
            .set_state(id, flow.state, to)
            .map_err(Self::store_failed)?;
        if moved {
            Ok(())
        } else {
            Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )))
        }
    }

    /// The status message for flow `id` in `state`.
    fn status_message(
        id: &str,
        state: DataFlowState,
        address: Option<DataAddress>,
    ) -> DataFlowStatusMessage {
        DataFlowStatusMessage {
            message_id: new_uuid(),
            data_flow_id: id.to_owned(),
            state,
            data_address: address,
            error: None,
        }
    }

    /// Where the consumer fetches flow `id`'s data, with `token`.
    fn address(&self, id: &str, token: &str) -> DataAddress {
        DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_ENDPOINT_TYPE.to_owned(),
            endpoint: Some(format!("{}{DATA_PATH}/{id}", self.public_url)),
            endpoint_properties: vec![
                EndpointProperty::new("authorization", token),
                EndpointProperty::new("authType", "bearer"),
            ],
        }
    }

    /// Gives flow `id` (SUSPENDED or STARTED) a new token with a full
    /// lifetime, and returns its new address; the old token stops working.
    /// Not part of Data Plane Signaling: the provider needs it to start a
    /// pull transfer again, because only the token's hash is kept (ADR
    /// 0014).
    ///
    /// A flow that started before 1.1 gets no new token; the control
    /// plane checks that before it asks (ADR 0027, section 7).
    ///
    /// # Errors
    ///
    /// Unknown or ended flow, or the store failed.
    pub fn new_address(&self, id: &str) -> Result<DataAddress, SignalError> {
        let flow = self.flow(id)?;
        if flow.push_address.is_some() {
            return Err(SignalError::Invalid(format!(
                "data flow {id} is a push; it has no token of ours"
            )));
        }
        if !matches!(
            flow.state,
            DataFlowState::Suspended | DataFlowState::Started
        ) {
            return Err(SignalError::Invalid(format!(
                "data flow {id} is {:?}; it has no access to renew",
                flow.state
            )));
        }
        let token = new_token()?;
        let expires_at = now_secs().saturating_add(self.token_seconds);
        let renewed = self
            .flows
            .set_token(id, flow.state, &hash(&token), expires_at)
            .map_err(Self::store_failed)?;
        if !renewed {
            return Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )));
        }
        Ok(self.address(id, &token))
    }

    /// `start` for a push flow: the message carries the consumer's data
    /// address, which the flow keeps; the answer carries none.
    fn start_push(
        &self,
        message: &DataFlowStartMessage,
    ) -> Result<DataFlowStatusMessage, SignalError> {
        let request = &message.flow;
        let Some(address) = &message.data_address else {
            return Err(SignalError::Invalid(
                "a push start carries the consumer's data address".to_owned(),
            ));
        };
        check_push_address(address).map_err(SignalError::Invalid)?;
        let source = self.readable_source_named_in(request)?;
        let flow = Flow {
            id: request.data_flow_id.clone(),
            state: DataFlowState::Started,
            dataset_id: request.dataset_id.clone(),
            source: Some(source),
            agreement_id: request.agreement_id.clone(),
            counter_party_id: request.counter_party_id.clone(),
            token_hash: String::new(),
            expires_at: 0,
            push_address: Some(address.clone()),
        };
        if !self.flows.insert(&flow).map_err(Self::store_failed)? {
            return Err(SignalError::DataFlowExists(flow.id));
        }
        Ok(Self::status_message(&flow.id, flow.state, None))
    }

    /// The dataset's source that the start message `request` names in its
    /// `metadata` ([`SOURCE_METADATA_KEY`]), if it is one this data plane
    /// can read: an `http(s)` URL, or an `s3://` object when the
    /// connector has object storage.
    fn readable_source_named_in(
        &self,
        request: &DataFlowBaseMessage,
    ) -> Result<String, SignalError> {
        let dataset_id = &request.dataset_id;
        let named = match &request.metadata {
            Some(metadata) => metadata.get(SOURCE_METADATA_KEY),
            None => None,
        };
        let Some(named) = named else {
            return Err(SignalError::Invalid(format!(
                "dataset {dataset_id} has no source"
            )));
        };
        // the same forms the harvest accepts in the datasets file
        let source = match named.as_str() {
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => url,
            Some(uri) if parse_s3_uri(uri).is_some() => uri,
            _ => {
                return Err(SignalError::Invalid(format!(
                    "dataset {dataset_id}: {SOURCE_METADATA_KEY} must be an http:// or https:// URL, or s3://{{bucket}}/{{key}}"
                )));
            }
        };
        if !self.can_read(source) {
            return Err(SignalError::Invalid(format!(
                "dataset {dataset_id} is in object storage, which this connector does not have"
            )));
        }
        Ok(source.to_owned())
    }

    /// Replaces the consumer's address of push flow `id`, SUSPENDED.
    fn replace_push_address(&self, id: &str, address: &DataAddress) -> Result<(), SignalError> {
        let flow = self.flow(id)?;
        if flow.push_address.is_none() {
            return Err(SignalError::Invalid(
                "a pull resume carries no data address".to_owned(),
            ));
        }
        check_push_address(address).map_err(SignalError::Invalid)?;
        let replaced = self
            .flows
            .set_push_address(id, flow.state, address)
            .map_err(Self::store_failed)?;
        if replaced {
            Ok(())
        } else {
            Err(SignalError::Failed(format!(
                "data flow {id} changed concurrently"
            )))
        }
    }
}

impl DataPlane for HttpDataPlane {
    fn prepare(&self, _: &DataFlowPrepareMessage) -> Result<DataFlowStatusMessage, SignalError> {
        Err(SignalError::Invalid(
            "prepare is for push transfers and consumer data planes".to_owned(),
        ))
    }

    fn start(&self, message: &DataFlowStartMessage) -> Result<DataFlowStatusMessage, SignalError> {
        let request = &message.flow;
        if request.profile == HTTP_PUSH {
            return self.start_push(message);
        }
        if request.profile != HTTP_PULL {
            return Err(SignalError::Invalid(format!(
                "unsupported profile {}",
                request.profile
            )));
        }
        if message.data_address.is_some() {
            // the spec: a data address on start for a pull "MUST result in a 4xx"
            return Err(SignalError::Invalid(
                "a pull start carries no data address".to_owned(),
            ));
        }
        let source = self.readable_source_named_in(request)?;
        let token = new_token()?;
        let flow = Flow {
            id: request.data_flow_id.clone(),
            state: DataFlowState::Started,
            dataset_id: request.dataset_id.clone(),
            source: Some(source),
            agreement_id: request.agreement_id.clone(),
            counter_party_id: request.counter_party_id.clone(),
            token_hash: hash(&token),
            expires_at: now_secs().saturating_add(self.token_seconds),
            push_address: None,
        };
        if !self.flows.insert(&flow).map_err(Self::store_failed)? {
            return Err(SignalError::DataFlowExists(flow.id));
        }
        let address = self.address(&flow.id, &token);
        Ok(Self::status_message(&flow.id, flow.state, Some(address)))
    }

    fn started(&self, _: &str, _: &DataFlowStartedNotificationMessage) -> Result<(), SignalError> {
        Err(SignalError::Invalid(
            "started is for consumer data planes".to_owned(),
        ))
    }

    fn suspend(&self, id: &str, _: &DataFlowSuspendMessage) -> Result<(), SignalError> {
        self.move_and_cut_downloads(id, DataFlowState::Suspended)
    }

    fn resume(
        &self,
        id: &str,
        message: &DataFlowResumeMessage,
    ) -> Result<DataFlowStatusMessage, SignalError> {
        if let Some(address) = &message.data_address {
            // a push started again with a new token (ADR 0027)
            self.replace_push_address(id, address)?;
        }
        // the same token works again (ADR 0013), so no new address
        self.move_to(id, DataFlowState::Started)?;
        Ok(Self::status_message(id, DataFlowState::Started, None))
    }

    fn terminate(&self, id: &str, _: &DataFlowTerminateMessage) -> Result<(), SignalError> {
        self.move_and_cut_downloads(id, DataFlowState::Terminated)
    }

    fn completed(&self, id: &str) -> Result<(), SignalError> {
        self.move_to(id, DataFlowState::Completed)
    }

    fn status(&self, id: &str) -> Result<DataFlowStatusResponseMessage, SignalError> {
        let flow = self.flow(id)?;
        Ok(DataFlowStatusResponseMessage {
            data_flow_id: flow.id,
            state: flow.state,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use portitor_dps::message::{AddressMessage, ReasonMessage};

    use super::*;
    use crate::audit::Parties;
    use crate::serving::Denied;

    fn plane() -> HttpDataPlane {
        let flows = Arc::new(crate::test_store::InMemoryFlows::default());
        HttpDataPlane::new(flows, "https://provider.example/", 60).unwrap()
    }

    /// The `metadata` of a start message that names `source`.
    fn metadata_naming(source: &str) -> serde_json::Map<String, serde_json::Value> {
        let mut metadata = serde_json::Map::new();
        metadata.insert(SOURCE_METADATA_KEY.to_owned(), source.into());
        metadata
    }

    /// A start message for `dataset`, as the control plane sends it: only
    /// dataset `d1` has a source, `http://upstream.example/d1`.
    fn start(dataset: &str, profile: &str) -> DataFlowStartMessage {
        let metadata = if dataset == "d1" {
            Some(metadata_naming("http://upstream.example/d1"))
        } else {
            None
        };
        DataFlowStartMessage {
            flow: DataFlowBaseMessage {
                message_id: "m".to_owned(),
                participant_id: "did:web:p".to_owned(),
                counter_party_id: "did:web:c".to_owned(),
                dataspace_context: "default".to_owned(),
                data_flow_id: "f1".to_owned(),
                agreement_id: "a".to_owned(),
                dataset_id: dataset.to_owned(),
                profile: profile.to_owned(),
                claims: serde_json::Map::new(),
                metadata,
                labels: vec![],
            },
            data_address: None,
        }
    }

    fn token_of(status: &DataFlowStatusMessage) -> String {
        let address = status.data_address.as_ref().unwrap();
        address.property("authorization").unwrap().to_owned()
    }

    fn reason() -> ReasonMessage {
        ReasonMessage {
            message_id: "m".to_owned(),
            reason: None,
        }
    }

    #[test]
    fn a_new_address_replaces_the_token() {
        let p = plane();
        let old = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        p.suspend("f1", &reason()).unwrap();
        let address = p.new_address("f1").unwrap();
        let new = address.property("authorization").unwrap().to_owned();
        assert_ne!(new, old);
        let resume = AddressMessage {
            message_id: "m".to_owned(),
            data_address: None,
        };
        p.resume("f1", &resume).unwrap();
        let now = now_secs();
        assert!(p.source_if_authorized("f1", &new, now).is_ok());
        assert_eq!(
            p.source_if_authorized("f1", &old, now),
            Err(Denied::Unauthorized)
        );

        p.completed("f1").unwrap();
        assert!(p.new_address("f1").is_err(), "an ended flow gets no token");
        assert!(p.new_address("unknown").is_err());
    }

    #[test]
    fn start_gives_an_address_with_a_token_that_only_works_while_started() {
        let p = plane();
        let status = p.start(&start("d1", HTTP_PULL)).unwrap();
        assert_eq!(status.state, DataFlowState::Started);
        let address = status.data_address.clone().unwrap();
        assert_eq!(
            address.endpoint.as_deref(),
            Some("https://provider.example/data/f1")
        );
        assert_eq!(address.endpoint_type, HTTP_ENDPOINT_TYPE);
        assert_eq!(address.property("authType"), Some("bearer"));
        let token = token_of(&status);
        assert!(token.len() >= 43, "32 random bytes");

        let now = now_secs();
        assert_eq!(
            p.source_if_authorized("f1", &token, now).unwrap(),
            "http://upstream.example/d1"
        );
        // the token is not stored, only its hash
        let stored = p.flows.get("f1").unwrap().unwrap();
        assert_ne!(stored.token_hash, token);
        assert_eq!(
            p.source_if_authorized("f1", "wrong", now),
            Err(Denied::Unauthorized)
        );
        assert_eq!(
            p.source_if_authorized("f2", &token, now),
            Err(Denied::Unauthorized)
        );
        assert_eq!(
            p.source_if_authorized("f1", &token, now + 61),
            Err(Denied::Unauthorized),
            "expired"
        );

        p.suspend("f1", &reason()).unwrap();
        assert_eq!(
            p.source_if_authorized("f1", &token, now),
            Err(Denied::Unauthorized)
        );
        let resume = AddressMessage {
            message_id: "m".to_owned(),
            data_address: None,
        };
        let resumed = p.resume("f1", &resume).unwrap();
        assert!(resumed.data_address.is_none(), "same token");
        assert!(p.source_if_authorized("f1", &token, now).is_ok());
        p.completed("f1").unwrap();
        assert_eq!(
            p.source_if_authorized("f1", &token, now),
            Err(Denied::Unauthorized)
        );
        assert_eq!(p.status("f1").unwrap().state, DataFlowState::Completed);
        assert!(matches!(
            p.terminate("f1", &reason()),
            Err(SignalError::NotAllowed(_))
        ));
    }

    #[test]
    fn start_refuses_what_it_cannot_serve() {
        let p = plane();
        let refused = |m: &DataFlowStartMessage| matches!(p.start(m), Err(SignalError::Invalid(_)));
        assert!(refused(&start(
            "d1",
            "https://w3id.org/dspace-sig/profile/http-push"
        )));
        assert!(refused(&start("no-source", HTTP_PULL)));
        let mut with_address = start("d1", HTTP_PULL);
        with_address.data_address = Some(
            serde_json::from_value(serde_json::json!(
                {"@type": "DataAddress", "endpointType": HTTP_ENDPOINT_TYPE}
            ))
            .unwrap(),
        );
        assert!(refused(&with_address));
        p.start(&start("d1", HTTP_PULL)).unwrap();
        assert!(matches!(
            p.start(&start("d1", HTTP_PULL)),
            Err(SignalError::DataFlowExists(_))
        ));
        assert!(matches!(
            p.suspend("nope", &reason()),
            Err(SignalError::UnknownDataFlow(_))
        ));
    }

    /// From 1.2 the source comes with the start message (ADR 0028, step
    /// 1): the flow keeps it and serves from it.
    #[test]
    fn the_flow_serves_the_source_its_start_message_named() {
        let p = plane();
        let mut message = start("d1", HTTP_PULL);
        message.flow.metadata = Some(metadata_naming("https://elsewhere.example/the-data"));
        let token = token_of(&p.start(&message).unwrap());
        let stored = p.flows.get("f1").unwrap().unwrap();
        let source = Some("https://elsewhere.example/the-data".to_owned());
        assert_eq!(stored.source, source);
        assert_eq!(
            p.source_if_authorized("f1", &token, now_secs()).ok(),
            source
        );
    }

    /// A start whose metadata names no source this data plane can read is
    /// refused, and no flow is kept.
    #[test]
    fn a_start_without_a_readable_source_is_refused() {
        let p = plane();
        let mut no_key = start("d1", HTTP_PULL);
        no_key.flow.metadata = Some(serde_json::Map::new());
        let mut not_text = start("d1", HTTP_PULL);
        not_text.flow.metadata = Some(serde_json::Map::from_iter([(
            SOURCE_METADATA_KEY.to_owned(),
            serde_json::json!(42),
        )]));
        let mut file = start("d1", HTTP_PULL);
        file.flow.metadata = Some(metadata_naming("file:///etc/passwd"));
        let mut no_storage = start("d1", HTTP_PULL);
        no_storage.flow.metadata = Some(metadata_naming("s3://datasets/d1.csv"));
        let mut push_without = start("d1", HTTP_PUSH);
        push_without.flow.metadata = None;
        push_without.data_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        for message in [no_key, not_text, file, no_storage, push_without] {
            let refused = p.start(&message);
            assert!(
                matches!(refused, Err(SignalError::Invalid(_))),
                "{refused:?}"
            );
        }
        assert_eq!(p.flows.get("f1").unwrap(), None);
    }

    /// A flow kept by 1.1 whose source the migration could not fill in
    /// (`Flow::source`) is never served, and its push never sent.
    #[test]
    fn a_flow_without_a_source_is_not_served() {
        let p = plane();
        let token = new_token().unwrap();
        let mut flow = Flow {
            id: "old".to_owned(),
            state: DataFlowState::Started,
            dataset_id: "gone".to_owned(),
            source: None,
            agreement_id: String::new(),
            counter_party_id: String::new(),
            token_hash: hash(&token),
            expires_at: now_secs() + 60,
            push_address: None,
        };
        p.flows.insert(&flow).unwrap();
        assert_eq!(
            p.source_if_authorized("old", &token, now_secs()),
            Err(Denied::Unauthorized)
        );
        flow.id = "old-push".to_owned();
        flow.push_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        p.flows.insert(&flow).unwrap();
        assert!(matches!(
            p.push_job("old-push"),
            Err(SignalError::Invalid(_))
        ));
    }

    /// The flow keeps the agreement and the consumer its start message
    /// names, for the audit line of each pull and push.
    #[test]
    fn the_flow_keeps_the_parties_of_its_start_message() {
        let p = plane();
        let token = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        let allowed = p.check_request("f1", &token, now_secs()).unwrap();
        let parties = Parties {
            agreement_id: "a".to_owned(),
            counter_party_id: "did:web:c".to_owned(),
        };
        assert_eq!(allowed.parties, parties);

        let mut push = start("d1", HTTP_PUSH);
        push.flow.data_flow_id = "f2".to_owned();
        push.data_address = Some(DataAddress {
            kind: DataAddressType,
            endpoint_type: portitor_dsp::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some("https://consumer.example/in".to_owned()),
            endpoint_properties: vec![],
        });
        p.start(&push).unwrap();
        let job = p.push_job("f2").unwrap().unwrap();
        assert_eq!(job.parties, parties);
        assert_eq!(job.source, "http://upstream.example/d1");
    }

    #[test]
    fn tokens_differ_per_flow() {
        let p = plane();
        let a = token_of(&p.start(&start("d1", HTTP_PULL)).unwrap());
        let mut second = start("d1", HTTP_PULL);
        second.flow.data_flow_id = "f2".to_owned();
        let b = token_of(&p.start(&second).unwrap());
        assert_ne!(a, b);
        assert_eq!(
            p.source_if_authorized("f2", &a, now_secs()),
            Err(Denied::Unauthorized)
        );
    }
}
