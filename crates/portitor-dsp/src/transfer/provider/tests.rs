// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::unwrap_used)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::Context;
use crate::negotiation::AgreementType;
use crate::transfer::{DataAddressType, EndpointProperty, TransferRequestMessageType};
use crate::{Dataset, DatasetType, Distribution, DistributionType, Rule};

const CONSUMER: &str = "did:web:consumer";

/// Transfers in memory. `.1`: inserts fail; `.2`: how many writes lose
/// the version check as if another write came first.
#[derive(Default)]
struct Store(RefCell<BTreeMap<String, Transfer>>, Cell<bool>, Cell<u32>);

impl TransferStore for &Store {
    fn get(&self, pid: &str) -> Result<Option<Transfer>, StoreError> {
        Ok(self.0.borrow().get(pid).cloned())
    }
    fn requested(&self, party: &str, cpid: &str) -> Result<Option<Transfer>, StoreError> {
        let map = self.0.borrow();
        let mut same_request = map
            .values()
            .filter(|t| t.counter_party == party && t.consumer_pid == cpid);
        Ok(same_request.next().cloned())
    }
    fn insert(&self, t: &Transfer) -> Result<bool, StoreError> {
        if self.1.get() {
            return Err(StoreError("disk full".to_owned()));
        }
        if self.requested(&t.counter_party, &t.consumer_pid)?.is_some() {
            return Ok(false);
        }
        self.0
            .borrow_mut()
            .insert(t.provider_pid.clone(), t.clone());
        Ok(true)
    }
    fn replace_if(&self, t: &Transfer, expected: u64) -> Result<bool, StoreError> {
        let mut map = self.0.borrow_mut();
        if self.2.get() > 0 {
            self.2.set(self.2.get() - 1);
            if let Some(old) = map.get_mut(&t.provider_pid) {
                old.version += 1;
            }
            return Ok(false);
        }
        match map.get(&t.provider_pid) {
            Some(old) if old.version == expected => {
                map.insert(t.provider_pid.clone(), t.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        let map = self.0.borrow();
        Ok(map
            .values()
            .filter(|t| t.pending.is_some())
            .cloned()
            .collect())
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, StoreError> {
        let map = self.0.borrow();
        let after = after.unwrap_or_default();
        Ok(map
            .values()
            .filter(|t| t.provider_pid.as_str() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

/// Agreements: `a1` for dataset `d1`, `a-expired` whose date limit has
/// passed, `a-other` held by another consumer. Only these are finalized.
struct Agreed;

fn agreement(id: &str, assignee: &str, permission: Rule) -> Agreement {
    Agreement {
        id: id.to_owned(),
        kind: AgreementType,
        target: "d1".to_owned(),
        assigner: "did:web:provider".to_owned(),
        assignee: assignee.to_owned(),
        timestamp: Some("2026-11-01T00:00:00Z".to_owned()),
        permission: vec![permission],
        prohibition: vec![],
        obligation: vec![],
        profile: None,
    }
}

impl Agreements for Agreed {
    fn finalized(&self, id: &str) -> Result<Option<Agreement>, StoreError> {
        let use_it: Rule = serde_json::from_value(json!({"action": "use"})).unwrap();
        let until_january: Rule = serde_json::from_value(json!({"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2026-01-01"}]}))
        .unwrap();
        Ok(match id {
            "a1" => Some(agreement(id, CONSUMER, use_it)),
            "a-other" => Some(agreement(id, "did:web:someone-else", use_it)),
            "a-expired" => Some(agreement(id, CONSUMER, until_january)),
            "a-members" => Some(agreement(id, CONSUMER, members_only())),
            _ => None,
        })
    }
    fn negotiation_of(&self, _: &str) -> Result<Option<Negotiation>, StoreError> {
        Ok(None)
    }
    fn replace_negotiation_if(&self, _: &Negotiation, _: u64) -> Result<bool, StoreError> {
        Ok(false)
    }
}

/// Only for callers with a `MembershipCredential` at level `gold`
/// (ADR 0019).
fn members_only() -> Rule {
    serde_json::from_value(json!({"action": "use", "constraint": [
        {"leftOperand": "MembershipCredential.level", "operator": "isAnyOf",
         "rightOperand": ["gold"]}]}))
    .unwrap()
}

/// A caller with a membership at `level`.
fn member_at(level: &str) -> CallerCredentials {
    CallerCredentials {
        understood: crate::policy::CredentialTypes::default(),
        held: vec![crate::policy::HeldCredential {
            types: vec!["MembershipCredential".to_owned()],
            subjects: vec![
                json!({"id": CONSUMER, "level": level})
                    .as_object()
                    .unwrap()
                    .clone(),
            ],
        }],
    }
}

/// The data plane: records each call; fails every call when `.1` is set.
#[derive(Default)]
struct Flows(RefCell<Vec<String>>, Cell<bool>);

impl Flows {
    fn record(&self, call: &str) -> Result<(), String> {
        self.0.borrow_mut().push(call.to_owned());
        if self.1.get() {
            Err("data plane down".to_owned())
        } else {
            Ok(())
        }
    }
    fn calls(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

/// The data address the fake data plane gives for `pid`.
fn address(pid: &str, token: &str) -> DataAddress {
    DataAddress {
        kind: DataAddressType,
        endpoint_type: "https://w3id.org/idsa/v4.1/HTTP".to_owned(),
        endpoint: Some(format!("https://provider.example/data/{pid}")),
        endpoint_properties: vec![
            EndpointProperty::new("authorization", token),
            EndpointProperty::new("authType", "bearer"),
        ],
    }
}

impl DataFlows for &Flows {
    fn supports(&self, _: &str, format: &str) -> bool {
        format == "HttpData-PULL" || format == "HttpData-PUSH"
    }
    /// A pull flow gives an address; a push flow (one with the consumer's
    /// address) none.
    fn start(&self, t: &Transfer) -> Result<Option<DataAddress>, String> {
        if let Some(to) = &t.data_address {
            let token = to.property("authorization").unwrap_or_default();
            self.record(&format!("start {} push with {token}", t.provider_pid))?;
            return Ok(None);
        }
        self.record(&format!("start {}", t.provider_pid))?;
        Ok(Some(address(&t.provider_pid, "t0ken")))
    }
    fn restart(&self, t: &Transfer) -> Result<Option<DataAddress>, String> {
        self.record(&format!("restart {}", t.provider_pid))?;
        if t.data_address.is_some() {
            return Ok(None);
        }
        Ok(Some(address(&t.provider_pid, "t0ken-2")))
    }
    fn suspend(&self, pid: &str) -> Result<(), String> {
        self.record(&format!("suspend {pid}"))
    }
    fn resume(&self, pid: &str, to: Option<&DataAddress>) -> Result<(), String> {
        match to.and_then(|to| to.property("authorization")) {
            Some(token) => self.record(&format!("resume {pid} push with {token}")),
            None => self.record(&format!("resume {pid}")),
        }
    }
    fn complete(&self, pid: &str) -> Result<(), String> {
        self.record(&format!("complete {pid}"))
    }
    fn terminate(&self, pid: &str) -> Result<(), String> {
        self.record(&format!("terminate {pid}"))
    }
}

/// Dataset `d1`, offered as `HttpData-PULL` and as `HttpData-PUSH` (which
/// the data plane cannot do).
struct Catalog;

impl CatalogDatasets for Catalog {
    fn all(&self) -> Vec<Dataset> {
        self.get("d1").into_iter().collect()
    }
    fn get(&self, id: &str) -> Option<Dataset> {
        let distribution = |format: &str| Distribution {
            kind: DistributionType,
            format: format.to_owned(),
            access_service: "svc".to_owned(),
            metadata: crate::Metadata::default(),
        };
        (id == "d1").then(|| Dataset {
            context: None,
            id: id.to_owned(),
            kind: DatasetType,
            has_policy: vec![],
            distribution: vec![distribution("HttpData-PULL"), distribution("HttpData-PUSH")],
            metadata: crate::Metadata::default(),
        })
    }
}

#[derive(Default)]
struct Env(Cell<u32>);

impl Environment for Env {
    fn new_id(&self) -> String {
        self.0.set(self.0.get() + 1);
        format!("urn:uuid:{}", self.0.get())
    }
    fn now(&self) -> String {
        "2026-11-13T12:00:00Z".to_owned()
    }
}

/// The same decision for every transfer.
struct Always(TransferDecision);

impl TransferDecider for Always {
    fn decide(&self, _: &Transfer) -> TransferDecision {
        self.0
    }
}

type Service<'a> = TransferService<&'a Store, Agreed, &'a Flows, Always, Catalog, Env>;

fn service<'a>(store: &'a Store, flows: &'a Flows, decision: TransferDecision) -> Service<'a> {
    TransferService::new(
        store,
        Agreed,
        flows,
        Always(decision),
        Catalog,
        Env::default(),
    )
}

fn request(agreement_id: &str, format: &str) -> TransferRequestMessage {
    TransferRequestMessage {
        context: Context::default(),
        kind: TransferRequestMessageType,
        consumer_pid: "urn:uuid:c".to_owned(),
        agreement_id: agreement_id.to_owned(),
        format: format.to_owned(),
        data_address: None,
        callback_address: "https://consumer.example/cb".to_owned(),
    }
}

const PID: &str = "urn:uuid:1";
const IDS: (&str, &str) = (PID, "urn:uuid:c");

fn reason(failure: &Failure) -> String {
    match failure {
        Failure::BadRequest(e) | Failure::NotFound(e) => format!("{e:?}"),
        other => format!("{other:?}"),
    }
}

/// A transfer that is STARTED: requested, started by the data plane, and
/// the start message acknowledged.
fn started<'a>(store: &'a Store, flows: &'a Flows) -> Service<'a> {
    let s = service(store, flows, TransferDecision::Start);
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    s.delivered(&out.unwrap()).unwrap();
    s
}

fn state(store: &Store) -> TransferState {
    store.0.borrow()[PID].state
}

#[test]
fn a_valid_request_starts_the_data_flow_and_sends_its_address() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let (ack, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    assert_eq!(ack.state, TransferState::Requested);
    assert_eq!(flows.calls(), ["start urn:uuid:1"]);
    let out = out.unwrap();
    assert_eq!(
        out.url,
        "https://consumer.example/cb/transfers/urn:uuid:c/start"
    );
    assert_eq!(out.to, TransferState::Started);
    let sent: TransferStartMessage = serde_json::from_value(out.body.clone()).unwrap();
    let address = sent.data_address.unwrap();
    assert_eq!(address.property("authorization"), Some("t0ken"));
    let saved = store.0.borrow()[PID].clone();
    assert_eq!(
        (saved.agreement_id.as_str(), saved.dataset_id.as_str()),
        ("a1", "d1")
    );
    assert_eq!(saved.pending, Some(out.clone()));

    s.delivered(&out).unwrap();
    assert_eq!(state(&store), TransferState::Started);
    assert!(store.0.borrow()[PID].pending.is_none());
    // a second acknowledgement of the same message changes nothing
    s.delivered(&out).unwrap();
    assert_eq!(state(&store), TransferState::Started);
}

/// DSP 2025-1, "Transfer Request Message": "Providers SHOULD implement
/// idempotent behavior ... based on the value of `consumerPid`".
#[test]
fn a_repeated_request_gets_the_same_transfer_and_starts_nothing() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let none = CallerCredentials::default();
    let msg = request("a1", "HttpData-PULL");
    let (first, out) = s.request(&msg, CONSUMER, &none).unwrap();
    assert!(out.is_some(), "the first request sends the start message");

    // again while the start message is on its way
    let (again, out) = s.request(&msg, CONSUMER, &none).unwrap();
    assert_eq!(again, first);
    assert_eq!(out, None, "no second start message");

    // again once the transfer has started: the answer shows its state now
    let pending = store.0.borrow()[PID].pending.clone().unwrap();
    s.delivered(&pending).unwrap();
    let (again, out) = s.request(&msg, CONSUMER, &none).unwrap();
    assert_eq!(again.provider_pid, first.provider_pid);
    assert_eq!(again.state, TransferState::Started);
    assert_eq!(out, None);

    assert_eq!(store.0.borrow().len(), 1, "one transfer");
    assert_eq!(
        flows.calls(),
        ["start urn:uuid:1"],
        "one data flow, one token"
    );
}

#[test]
fn a_repeated_request_with_other_content_is_refused() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let none = CallerCredentials::default();
    s.request(&request("a1", "HttpData-PULL"), CONSUMER, &none)
        .unwrap();

    let mut other_callback = request("a1", "HttpData-PULL");
    other_callback.callback_address = "https://elsewhere.example/cb".to_owned();
    // `a-members` is another agreement of the same consumer
    let other_agreement = request("a-members", "HttpData-PULL");
    for (msg, credentials) in [
        (other_callback, &none),
        (other_agreement, &member_at("gold")),
    ] {
        let failure = s.request(&msg, CONSUMER, credentials).unwrap_err();
        assert!(matches!(failure, Failure::BadRequest(_)), "{failure:?}");
        assert!(reason(&failure).contains("already used"), "{failure:?}");
        // the refusal does not name the existing transfer
        assert!(!reason(&failure).contains(PID), "{failure:?}");
    }
    assert_eq!(store.0.borrow().len(), 1);
    assert_eq!(flows.calls(), ["start urn:uuid:1"]);
}

/// What makes a repeat the same request: the agreement, the format and
/// the callback address, each compared as text. One other value and the
/// repeat is refused (ADR 0013, amended in 0.12).
#[test]
fn a_repeat_is_the_same_request_only_if_agreement_format_and_callback_match() {
    let (store, flows) = (Store::default(), Flows::default());
    started(&store, &flows);
    let existing = store.0.borrow()[PID].clone();

    let same = request("a1", "HttpData-PULL");
    let (ack, out) = answer_again(&existing, &same).unwrap();
    assert_eq!(ack.provider_pid, PID);
    assert_eq!(out, None);

    let mut other_callback = same.clone();
    // the same address written another way is another address
    other_callback.callback_address = "https://consumer.example/cb/".to_owned();
    for other in [
        request("a-members", "HttpData-PULL"),
        request("a1", "HttpData-PUSH"),
        request("a1", "httpdata-pull"),
        other_callback,
    ] {
        let failure = answer_again(&existing, &other).unwrap_err();
        assert!(matches!(failure, Failure::BadRequest(_)), "{other:?}");
        assert!(reason(&failure).contains("already used"), "{other:?}");
    }
}

/// A `consumerPid` stays taken when its transfer has ended: the request
/// sent again gets the ended transfer as it is, and starts no data flow.
#[test]
fn a_repeated_request_after_the_transfer_ended_gets_the_ended_transfer() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    s.terminate(PID, IDS, CONSUMER).unwrap();
    let calls_before = flows.calls();

    let msg = request("a1", "HttpData-PULL");
    let (again, out) = s
        .request(&msg, CONSUMER, &CallerCredentials::default())
        .unwrap();
    assert_eq!(again.provider_pid, PID);
    assert_eq!(again.state, TransferState::Terminated);
    assert_eq!(out, None);
    assert_eq!(store.0.borrow().len(), 1);
    assert_eq!(flows.calls(), calls_before, "no new data flow");
}

/// A `consumerPid` is only unique for one consumer: another participant
/// that uses the same value must not get, or learn of, the first one's
/// transfer.
#[test]
fn another_caller_with_the_same_consumer_pid_does_not_get_the_transfer() {
    const OTHER: &str = "did:web:someone-else";
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let none = CallerCredentials::default();
    let (first, _) = s
        .request(&request("a1", "HttpData-PULL"), CONSUMER, &none)
        .unwrap();

    // with the first consumer's agreement: refused as any agreement that
    // is not the caller's, before the lookup
    let failure = s
        .request(&request("a1", "HttpData-PULL"), OTHER, &none)
        .unwrap_err();
    assert!(
        reason(&failure).contains("no finalized agreement"),
        "{failure:?}"
    );
    assert!(!reason(&failure).contains(PID), "{failure:?}");

    // with its own agreement: a transfer of its own
    let (own, out) = s
        .request(&request("a-other", "HttpData-PULL"), OTHER, &none)
        .unwrap();
    assert_ne!(own.provider_pid, first.provider_pid);
    assert_eq!(own.state, TransferState::Requested);
    assert!(out.is_some());
    assert_eq!(store.0.borrow().len(), 2);
    assert_eq!(store.0.borrow()[&own.provider_pid].counter_party, OTHER);
}

/// A store in which a copy of the request is saved between the lookup and
/// the insert, as when two copies arrive together.
struct Racing<'a>(&'a Store, Cell<bool>);

impl TransferStore for &Racing<'_> {
    fn get(&self, pid: &str) -> Result<Option<Transfer>, StoreError> {
        self.0.get(pid)
    }
    fn requested(&self, party: &str, cpid: &str) -> Result<Option<Transfer>, StoreError> {
        // the first lookup comes before the other copy is saved
        if self.1.replace(true) {
            self.0.requested(party, cpid)
        } else {
            Ok(None)
        }
    }
    fn insert(&self, t: &Transfer) -> Result<bool, StoreError> {
        self.0.insert(t)
    }
    fn replace_if(&self, t: &Transfer, expected: u64) -> Result<bool, StoreError> {
        self.0.replace_if(t, expected)
    }
    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        self.0.pending()
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, StoreError> {
        self.0.list(after, limit)
    }
}

#[test]
fn two_copies_of_a_request_at_once_give_one_transfer() {
    let (store, flows) = (Store::default(), Flows::default());
    let none = CallerCredentials::default();
    let msg = request("a1", "HttpData-PULL");
    let (first, _) = service(&store, &flows, TransferDecision::Start)
        .request(&msg, CONSUMER, &none)
        .unwrap();

    let racing = Racing(&store, Cell::new(false));
    let late = TransferService::new(
        &racing,
        Agreed,
        &flows,
        Always(TransferDecision::Start),
        Catalog,
        Env(Cell::new(1)),
    );
    let (again, out) = late.request(&msg, CONSUMER, &none).unwrap();
    assert_eq!(again.provider_pid, first.provider_pid);
    assert_eq!(out, None);
    assert_eq!(store.0.borrow().len(), 1);
    // the data flow started for the late copy is ended again
    assert_eq!(
        flows.calls(),
        [
            "start urn:uuid:1",
            "start urn:uuid:2",
            "terminate urn:uuid:2"
        ]
    );
}

#[test]
fn requests_that_break_a_rule_are_refused_before_anything_starts() {
    let cases = [
        ("unknown", "HttpData-PULL", "no finalized agreement"),
        ("a-other", "HttpData-PULL", "no finalized agreement"),
        ("a-expired", "HttpData-PULL", "does not allow it now"),
        ("a1", "S3-PULL", "not offered in this format"),
        // a push request without the consumer's address (ADR 0027)
        (
            "a1",
            "HttpData-PUSH",
            "a push transfer request has a dataAddress",
        ),
    ];
    for (agreement_id, format, expected) in cases {
        let (store, flows) = (Store::default(), Flows::default());
        let s = service(&store, &flows, TransferDecision::Start);
        let failure = s
            .request(
                &request(agreement_id, format),
                CONSUMER,
                &CallerCredentials::default(),
            )
            .unwrap_err();
        assert!(matches!(failure, Failure::BadRequest(_)), "{agreement_id}");
        assert!(
            reason(&failure).contains(expected),
            "{agreement_id}: {failure:?}"
        );
        assert!(store.0.borrow().is_empty());
        assert!(flows.calls().is_empty());
    }
}

#[test]
fn push_requests_and_missing_callbacks_are_refused() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let mut push = request("a1", "HttpData-PULL");
    push.data_address = Some(
        serde_json::from_value(json!(
            {"@type": "DataAddress", "endpointType": "https://w3id.org/idsa/v4.1/HTTP"}
        ))
        .unwrap(),
    );
    assert!(
        reason(
            &s.request(&push, CONSUMER, &CallerCredentials::default())
                .unwrap_err()
        )
        .contains("push")
    );
    let mut no_callback = request("a1", "HttpData-PULL");
    no_callback.callback_address.clear();
    assert!(
        reason(
            &s.request(&no_callback, CONSUMER, &CallerCredentials::default())
                .unwrap_err()
        )
        .contains("callbackAddress")
    );
}

#[test]
fn a_refusing_data_plane_ends_the_transfer() {
    let (store, flows) = (Store::default(), Flows::default());
    flows.1.set(true);
    let s = service(&store, &flows, TransferDecision::Start);
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    assert!(out.url.ends_with("/termination"));
}

#[test]
fn a_failed_save_ends_the_started_data_flow() {
    let (store, flows) = (Store::default(), Flows::default());
    store.1.set(true);
    let s = service(&store, &flows, TransferDecision::Start);
    let failure = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap_err();
    assert!(matches!(failure, Failure::Store(_)));
    assert_eq!(flows.calls(), ["start urn:uuid:1", "terminate urn:uuid:1"]);
}

#[test]
fn the_decider_may_wait_or_terminate() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Wait);
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    assert!(out.is_none());
    assert!(flows.calls().is_empty());

    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Terminate);
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    assert_eq!(out.unwrap().transition, Transition::Terminate);
    // nothing started; ending it anyway is harmless
    assert_eq!(flows.calls(), ["terminate urn:uuid:1"]);
}

/// Each decision in turn, then [`TransferDecision::Wait`].
struct Steps(RefCell<Vec<TransferDecision>>);

impl TransferDecider for &Steps {
    fn decide(&self, _: &Transfer) -> TransferDecision {
        let mut steps = self.0.borrow_mut();
        if steps.is_empty() {
            TransferDecision::Wait
        } else {
            steps.remove(0)
        }
    }
}

/// Requests a transfer with a provider that takes `steps`, delivers every
/// message it sends, and returns them.
fn provider_steps(store: &Store, flows: &Flows, steps: Vec<TransferDecision>) -> Vec<Outgoing> {
    let steps = Steps(RefCell::new(steps));
    let s = TransferService::new(store, Agreed, flows, &steps, Catalog, Env::default());
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    let mut sent = Vec::new();
    let mut next = out;
    while let Some(out) = next {
        next = s.delivered(&out).unwrap();
        sent.push(out);
    }
    sent
}

#[test]
fn the_provider_suspends_starts_again_and_completes() {
    use TransferDecision::{Complete, Start, Suspend};
    let (store, flows) = (Store::default(), Flows::default());
    let sent = provider_steps(&store, &flows, vec![Start, Suspend, Start, Complete]);
    let transitions: Vec<_> = sent.iter().map(|o| o.transition).collect();
    assert_eq!(
        transitions,
        [
            Transition::Start,
            Transition::Suspend,
            Transition::Start,
            Transition::Complete
        ]
    );
    assert!(sent[1].url.ends_with("/suspension"));
    assert!(sent[3].url.ends_with("/completion"));
    // started again: a new token, since the old one is not kept
    assert_eq!(
        sent[2].body["dataAddress"]["endpointProperties"][0]["value"],
        "t0ken-2"
    );
    assert_eq!(
        flows.calls(),
        [
            "start urn:uuid:1",
            "suspend urn:uuid:1",
            "restart urn:uuid:1",
            "complete urn:uuid:1"
        ]
    );
    assert_eq!(state(&store), TransferState::Completed);
}

#[test]
fn the_provider_terminates_a_started_transfer() {
    use TransferDecision::{Start, Terminate};
    let (store, flows) = (Store::default(), Flows::default());
    let sent = provider_steps(&store, &flows, vec![Start, Terminate]);
    assert_eq!(sent[1].transition, Transition::Terminate);
    assert_eq!(flows.calls(), ["start urn:uuid:1", "terminate urn:uuid:1"]);
    assert_eq!(state(&store), TransferState::Terminated);
}

#[test]
fn steps_the_state_machine_forbids_are_not_taken() {
    use TransferDecision::{Complete, Start, Suspend};
    let (store, flows) = (Store::default(), Flows::default());
    // suspend while REQUESTED, complete while SUSPENDED: neither is allowed
    let sent = provider_steps(&store, &flows, vec![Suspend]);
    assert!(sent.is_empty());
    let (store, flows) = (Store::default(), Flows::default());
    let sent = provider_steps(&store, &flows, vec![Start, Suspend, Complete]);
    assert_eq!(sent.len(), 2);
    assert_eq!(state(&store), TransferState::Suspended);
}

#[test]
fn a_data_plane_failure_on_a_provider_step_ends_the_transfer() {
    use TransferDecision::{Start, Suspend};
    let (store, flows) = (Store::default(), Flows::default());
    let steps = Steps(RefCell::new(vec![Start, Suspend]));
    let s = TransferService::new(&store, Agreed, &flows, &steps, Catalog, Env::default());
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    flows.1.set(true);
    let next = s.delivered(&out.unwrap()).unwrap().unwrap();
    assert_eq!(next.transition, Transition::Terminate);
    assert!(format!("{}", next.body).contains("data plane refused"));
}

#[test]
fn consumer_suspends_starts_again_and_completes() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    let (ack, out) = s.suspend(PID, IDS, CONSUMER).unwrap();
    assert_eq!((ack.state, out), (TransferState::Suspended, None));
    let restart: TransferStartMessage = serde_json::from_value(json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "TransferStartMessage", "providerPid": PID, "consumerPid": "urn:uuid:c"
    }))
    .unwrap();
    // no new address: the same token works again (ADR 0013)
    let (ack, out) = s.start(PID, &restart, CONSUMER).unwrap();
    assert_eq!((ack.state, out), (TransferState::Started, None));
    let (ack, _) = s.complete(PID, IDS, CONSUMER).unwrap();
    assert_eq!(ack.state, TransferState::Completed);
    assert_eq!(
        flows.calls()[1..],
        [
            "suspend urn:uuid:1",
            "resume urn:uuid:1",
            "complete urn:uuid:1"
        ]
    );
}

#[test]
fn a_consumer_message_proves_our_start_arrived() {
    // the consumer suspends before its acknowledgement of our start is
    // recorded: it could not suspend otherwise, so both apply
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    s.request(
        &request("a1", "HttpData-PULL"),
        CONSUMER,
        &CallerCredentials::default(),
    )
    .unwrap();
    let (ack, _) = s.suspend(PID, IDS, CONSUMER).unwrap();
    assert_eq!(ack.state, TransferState::Suspended);
    assert!(store.0.borrow()[PID].pending.is_none());
}

#[test]
fn consumer_termination_of_a_requested_transfer_drops_our_start() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    s.request(
        &request("a1", "HttpData-PULL"),
        CONSUMER,
        &CallerCredentials::default(),
    )
    .unwrap();
    let (ack, _) = s.terminate(PID, IDS, CONSUMER).unwrap();
    assert_eq!(ack.state, TransferState::Terminated);
    assert!(store.0.borrow()[PID].pending.is_none());
    assert_eq!(flows.calls(), ["start urn:uuid:1", "terminate urn:uuid:1"]);
}

/// The TCK's negative provider cases (TP:03-01 to TP:03-06): the message
/// is refused, the state and the data flow stay as they were.
#[test]
fn messages_that_do_not_fit_the_state_change_nothing() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Wait);
    s.request(
        &request("a1", "HttpData-PULL"),
        CONSUMER,
        &CallerCredentials::default(),
    )
    .unwrap();
    assert!(s.complete(PID, IDS, CONSUMER).is_err());
    assert!(s.suspend(PID, IDS, CONSUMER).is_err());
    assert_eq!(state(&store), TransferState::Requested);

    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    s.suspend(PID, IDS, CONSUMER).unwrap();
    assert!(s.complete(PID, IDS, CONSUMER).is_err());
    assert_eq!(state(&store), TransferState::Suspended);
    s.terminate(PID, IDS, CONSUMER).unwrap();
    assert!(s.suspend(PID, IDS, CONSUMER).is_err());
    assert!(s.complete(PID, IDS, CONSUMER).is_err());
    assert_eq!(state(&store), TransferState::Terminated);
    assert_eq!(flows.calls().len(), 3, "{:?}", flows.calls());
}

#[test]
fn only_the_transfers_consumer_sees_it() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    assert_eq!(s.get(PID, CONSUMER).unwrap().state, TransferState::Started);
    assert!(matches!(
        s.get(PID, "did:web:other"),
        Err(Failure::NotFound(_))
    ));
    assert!(matches!(
        s.suspend(PID, IDS, "did:web:other"),
        Err(Failure::NotFound(_))
    ));
    assert!(reason(&s.suspend(PID, (PID, "urn:uuid:x"), CONSUMER).unwrap_err()).contains("ids"));
    assert!(reason(&s.suspend("urn:uuid:9", IDS, CONSUMER).unwrap_err()).contains("path"));
}

#[test]
fn a_data_plane_failure_keeps_the_state() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    flows.1.set(true);
    assert!(matches!(
        s.suspend(PID, IDS, CONSUMER),
        Err(Failure::DataPlane(_))
    ));
    assert_eq!(state(&store), TransferState::Started);
}

#[test]
fn an_undeliverable_start_ends_the_data_flow_and_the_transfer() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let (_, out) = s
        .request(
            &request("a1", "HttpData-PULL"),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    let out = out.unwrap();
    assert_eq!(s.pending().unwrap(), std::slice::from_ref(&out));
    let termination = s
        .abort(&out, "the consumer could not be reached")
        .unwrap()
        .unwrap();
    assert_eq!(termination.transition, Transition::Terminate);
    assert_eq!(flows.calls()[1], "terminate urn:uuid:1");
    // a second abort of the same message changes nothing
    assert!(s.abort(&out, "again").unwrap().is_none());
    s.delivered(&termination).unwrap();
    assert_eq!(state(&store), TransferState::Terminated);
}

#[test]
fn concurrent_writes_are_retried() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    store.2.set(2);
    assert_eq!(
        s.suspend(PID, IDS, CONSUMER).unwrap().0.state,
        TransferState::Suspended
    );
    store.2.set(3);
    assert!(reason(&s.terminate(PID, IDS, CONSUMER).unwrap_err()).contains("concurrently"));
}

/// ADR 0019: the agreement is evaluated again at each new transfer, with
/// the credentials verified for that request; a credential the caller no
/// longer has (revoked, expired) stops the transfer before the data plane
/// is told.
#[test]
fn a_transfer_starts_only_while_the_caller_meets_the_agreement() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let msg = request("a-members", "HttpData-PULL");
    for credentials in [member_at("silver"), CallerCredentials::default()] {
        let failure = s.request(&msg, CONSUMER, &credentials).unwrap_err();
        assert!(
            reason(&failure).contains("does not allow it now"),
            "{failure:?}"
        );
        assert!(flows.calls().is_empty());
    }
    let (ack, out) = s.request(&msg, CONSUMER, &member_at("gold")).unwrap();
    assert_eq!(ack.state, TransferState::Requested);
    assert_eq!(out.unwrap().to, TransferState::Started);
    // the agreement, for the adapter to derive scopes from
    assert!(
        s.agreement_settling_finalization("a-members", CONSUMER)
            .unwrap()
            .is_some()
    );
    assert!(
        s.agreement_settling_finalization("a-other", CONSUMER)
            .unwrap()
            .is_none()
    );
    assert!(
        s.agreement_settling_finalization("unknown", CONSUMER)
            .unwrap()
            .is_none()
    );
}

// ---- ADR 0007, amended in 0.13 (TD-44): a transfer request that arrives
// ---- before the acknowledgement of our finalization event is recorded

use crate::negotiation::Transition as NegotiationTransition;
use crate::negotiation::provider::Outgoing as NegotiationMessage;

/// Agreements kept as the provider's negotiations, as the connector's
/// store keeps them. `.1`: how many settling writes lose the version
/// check; `.2`: what another writer did to the negotiation meanwhile.
struct Negotiations(RefCell<Vec<Negotiation>>, Cell<u32>, OtherWriter);

/// What the write that came first did.
#[derive(Clone, Copy)]
enum OtherWriter {
    /// It recorded the acknowledgement of the finalization event.
    RecordedTheAcknowledgement,
    /// It changed the negotiation without finalizing it.
    ChangedSomethingElse,
    /// It gave up the finalization event and saved our termination in
    /// its place (the outbox, when the consumer cannot be reached).
    StartedToTerminate,
}

impl Agreements for &Negotiations {
    fn finalized(&self, id: &str) -> Result<Option<Agreement>, StoreError> {
        Ok(self
            .negotiation_of(id)?
            .filter(|n| n.state == NegotiationState::Finalized)
            .and_then(|n| n.agreement))
    }
    fn negotiation_of(&self, id: &str) -> Result<Option<Negotiation>, StoreError> {
        let all = self.0.borrow();
        let made_it = |n: &&Negotiation| n.agreement.as_ref().is_some_and(|a| a.id == id);
        Ok(all.iter().find(made_it).cloned())
    }
    fn replace_negotiation_if(&self, new: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        let mut all = self.0.borrow_mut();
        let Some(old) = all.iter_mut().find(|n| n.provider_pid == new.provider_pid) else {
            return Ok(false);
        };
        if self.1.get() > 0 {
            self.1.set(self.1.get() - 1);
            old.version += 1;
            match self.2 {
                OtherWriter::RecordedTheAcknowledgement => {
                    old.state = NegotiationState::Finalized;
                    old.pending = None;
                }
                OtherWriter::ChangedSomethingElse => {}
                OtherWriter::StartedToTerminate => {
                    if let Some(pending) = &mut old.pending {
                        pending.transition = NegotiationTransition::Terminate;
                        pending.to = NegotiationState::Terminated;
                    }
                }
            }
            return Ok(false);
        }
        if old.version != expected {
            return Ok(false);
        }
        *old = new.clone();
        Ok(true)
    }
}

/// The provider's negotiation `n1` with `CONSUMER`, which made agreement
/// `a-new` for dataset `d1`, in `state` with `pending` on its way.
fn negotiation(state: NegotiationState, pending: Option<NegotiationTransition>) -> Negotiation {
    let use_it: Rule = serde_json::from_value(json!({"action": "use"})).unwrap();
    let pending = pending.map(|transition| NegotiationMessage {
        provider_pid: "n1".to_owned(),
        url: "https://consumer.example/cb/negotiations/c1/events".to_owned(),
        body: json!({}),
        transition,
        to: match transition {
            NegotiationTransition::Finalize => NegotiationState::Finalized,
            _ => NegotiationState::Terminated,
        },
        status_url: "https://consumer.example/cb/negotiations/c1".to_owned(),
        counter_party: CONSUMER.to_owned(),
    });
    Negotiation {
        provider_pid: "n1".to_owned(),
        consumer_pid: "c1".to_owned(),
        state,
        callback_address: "https://consumer.example/cb".to_owned(),
        counter_party: CONSUMER.to_owned(),
        offer: serde_json::from_value(json!({
            "@id": "d1:offer", "@type": "Offer", "target": "d1",
            "permission": [{"action": "use"}]
        }))
        .unwrap(),
        agreement: Some(agreement("a-new", CONSUMER, use_it)),
        pending,
        version: 4,
    }
}

/// `VERIFIED`, the finalization event on its way: the race of TD-44.
fn finalization_on_its_way() -> Negotiation {
    negotiation(
        NegotiationState::Verified,
        Some(NegotiationTransition::Finalize),
    )
}

fn negotiations(negotiation: Negotiation) -> Negotiations {
    Negotiations(
        RefCell::new(vec![negotiation]),
        Cell::new(0),
        OtherWriter::RecordedTheAcknowledgement,
    )
}

type RacingService<'a> =
    TransferService<&'a Store, &'a Negotiations, &'a Flows, Always, Catalog, Env>;

fn racing<'a>(
    store: &'a Store,
    negotiations: &'a Negotiations,
    flows: &'a Flows,
) -> RacingService<'a> {
    TransferService::new(
        store,
        negotiations,
        flows,
        Always(TransferDecision::Start),
        Catalog,
        Env::default(),
    )
}

fn request_as(s: &RacingService<'_>, caller: &str) -> Handled {
    let msg = request("a-new", "HttpData-PULL");
    s.request(&msg, caller, &CallerCredentials::default())
}

/// The provider always makes the consumer the agreement's assignee; if
/// a stored negotiation ever said otherwise, nothing is written for it.
#[test]
fn nothing_is_written_when_the_caller_is_not_the_agreements_assignee() {
    let (store, flows) = (Store::default(), Flows::default());
    let mut negotiation = finalization_on_its_way();
    let use_it: Rule = serde_json::from_value(json!({"action": "use"})).unwrap();
    negotiation.agreement = Some(agreement("a-new", "did:web:someone-else", use_it));
    let held = negotiations(negotiation.clone());
    let s = racing(&store, &held, &flows);

    assert!(request_as(&s, CONSUMER).is_err());
    assert_eq!(held.0.borrow()[0], negotiation);
    assert!(flows.calls().is_empty());
}

#[test]
fn a_request_while_the_finalization_is_on_its_way_settles_it() {
    let (store, flows) = (Store::default(), Flows::default());
    let held = negotiations(finalization_on_its_way());
    let s = racing(&store, &held, &flows);

    let (ack, out) = request_as(&s, CONSUMER).unwrap();
    assert_eq!(ack.state, TransferState::Requested);
    assert_eq!(out.unwrap().to, TransferState::Started);
    assert_eq!(flows.calls(), ["start urn:uuid:1"]);

    // the negotiation is settled as if the acknowledgement had come:
    // FINALIZED, nothing pending, one write
    let settled = held.0.borrow()[0].clone();
    assert_eq!(settled.state, NegotiationState::Finalized);
    assert_eq!(settled.pending, None);
    assert_eq!(settled.version, 5);
    assert!(settled.agreement.is_some());

    // a second transfer under it finds a finalized agreement: no write
    let mut second = request("a-new", "HttpData-PULL");
    second.consumer_pid = "urn:uuid:c2".to_owned();
    s.request(&second, CONSUMER, &CallerCredentials::default())
        .unwrap();
    assert_eq!(held.0.borrow()[0], settled);
}

/// ADR 0013: the caller must be the agreement's consumer, and this is
/// checked before anything is settled.
#[test]
fn another_caller_proves_nothing_and_settles_nothing() {
    let (store, flows) = (Store::default(), Flows::default());
    let held = negotiations(finalization_on_its_way());
    let s = racing(&store, &held, &flows);

    let failure = request_as(&s, "did:web:someone-else").unwrap_err();
    assert!(failure.is_no_finalized_agreement(), "{failure:?}");
    assert_eq!(held.0.borrow()[0], finalization_on_its_way());
    assert!(store.0.borrow().is_empty());
    assert!(flows.calls().is_empty());
    // the same answer as for an agreement that does not exist
    let mut unknown = request("a-unknown", "HttpData-PULL");
    unknown.consumer_pid = "urn:uuid:c".to_owned();
    let other = s
        .request(
            &unknown,
            "did:web:someone-else",
            &CallerCredentials::default(),
        )
        .unwrap_err();
    assert_eq!(failure, other);
}

#[test]
fn an_agreement_of_a_negotiation_in_any_other_state_is_still_refused() {
    use NegotiationState::{Agreed, Terminated, Verified};
    let cases = [
        // agreed, not verified by the consumer
        (Agreed, None),
        // verified, but the provider has not decided to finalize
        (Verified, None),
        // verified, and the provider is ending the negotiation
        (Verified, Some(NegotiationTransition::Terminate)),
        (Terminated, None),
    ];
    for (state, pending) in cases {
        let (store, flows) = (Store::default(), Flows::default());
        let held = negotiations(negotiation(state, pending));
        let s = racing(&store, &held, &flows);
        let failure = request_as(&s, CONSUMER).unwrap_err();
        assert!(
            failure.is_no_finalized_agreement(),
            "{state:?}: {failure:?}"
        );
        assert_eq!(held.0.borrow()[0], negotiation(state, pending), "{state:?}");
        assert!(store.0.borrow().is_empty());
        assert!(flows.calls().is_empty());
    }
}

/// Only this refusal answers `true`; the adapter logs it.
#[test]
fn other_refusals_are_not_the_no_finalized_agreement_refusal() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let none = CallerCredentials::default();
    let expired = s.request(&request("a-expired", "HttpData-PULL"), CONSUMER, &none);
    assert!(!expired.unwrap_err().is_no_finalized_agreement());
    let unknown = s.request(&request("unknown", "HttpData-PULL"), CONSUMER, &none);
    assert!(unknown.unwrap_err().is_no_finalized_agreement());
    assert!(!Failure::DataPlane("down".to_owned()).is_no_finalized_agreement());
}

/// The acknowledgement (or a second transfer request) is written at the
/// same moment and comes first: the settling write loses the version
/// check, the negotiation is read again, is FINALIZED, and the request
/// goes on. Nothing is written twice.
#[test]
fn a_settling_write_that_comes_second_reads_the_negotiation_again() {
    let (store, flows) = (Store::default(), Flows::default());
    let held = negotiations(finalization_on_its_way());
    held.1.set(1);
    let s = racing(&store, &held, &flows);

    let (ack, _) = request_as(&s, CONSUMER).unwrap();
    assert_eq!(ack.state, TransferState::Requested);
    let finalized = held.0.borrow()[0].clone();
    assert_eq!(finalized.state, NegotiationState::Finalized);
    assert_eq!(finalized.version, 5, "only the other writer wrote");
}

/// The first write was something else (not a finalization): the rule is
/// applied again to what is stored now. After a few lost writes the
/// request is refused and the consumer may try again.
#[test]
fn settling_is_tried_again_a_few_times_and_then_given_up() {
    let (store, flows) = (Store::default(), Flows::default());
    let mut held = negotiations(finalization_on_its_way());
    held.2 = OtherWriter::ChangedSomethingElse;
    held.1.set(2);
    let s = racing(&store, &held, &flows);
    request_as(&s, CONSUMER).unwrap();
    assert_eq!(held.0.borrow()[0].state, NegotiationState::Finalized);
    assert_eq!(held.0.borrow()[0].version, 7, "two lost writes, then ours");

    let (store, flows) = (Store::default(), Flows::default());
    let mut held = negotiations(finalization_on_its_way());
    held.2 = OtherWriter::ChangedSomethingElse;
    held.1.set(3);
    let s = racing(&store, &held, &flows);
    let failure = request_as(&s, CONSUMER).unwrap_err();
    assert!(reason(&failure).contains("concurrently"), "{failure:?}");
    assert!(!failure.is_no_finalized_agreement());
    assert_eq!(held.0.borrow()[0].state, NegotiationState::Verified);
    assert!(store.0.borrow().is_empty());
    assert!(flows.calls().is_empty());
}

/// The provider gives up the finalization event (the consumer could not
/// be reached) at the moment the transfer request arrives, and its write
/// comes first: the settling write loses the version check, the
/// negotiation is read again, and now our termination is on its way.
/// That is no finalization, so the request is refused and nothing of the
/// termination is undone.
#[test]
fn a_termination_written_first_makes_the_request_refused() {
    let (store, flows) = (Store::default(), Flows::default());
    let mut held = negotiations(finalization_on_its_way());
    held.2 = OtherWriter::StartedToTerminate;
    held.1.set(1);
    let s = racing(&store, &held, &flows);

    let failure = request_as(&s, CONSUMER).unwrap_err();
    assert!(failure.is_no_finalized_agreement(), "{failure:?}");
    let ending = held.0.borrow()[0].clone();
    assert_eq!(ending.state, NegotiationState::Verified);
    assert_eq!(
        ending.pending.unwrap().transition,
        NegotiationTransition::Terminate
    );
    assert_eq!(ending.version, 5, "only the other writer wrote");
    assert!(store.0.borrow().is_empty());
    assert!(flows.calls().is_empty());
}

/// The table of ADR 0007 (amended in 0.13), for every state a
/// negotiation can be in and every message that can be on its way: a
/// transfer request is served only under a `FINALIZED` negotiation, or a
/// `VERIFIED` one whose finalization event is on its way. In every other
/// case it is refused and the negotiation is left exactly as it was.
#[test]
fn only_finalized_or_verified_with_the_finalization_on_its_way_is_served() {
    use NegotiationState::{Accepted, Agreed, Finalized, Offered, Requested, Terminated, Verified};
    use NegotiationTransition::{Agree, Finalize, Offer, Terminate};
    let states = [
        Requested, Offered, Accepted, Agreed, Verified, Finalized, Terminated,
    ];
    let on_its_way = [
        None,
        Some(Offer),
        Some(Agree),
        Some(Finalize),
        Some(Terminate),
    ];
    for state in states {
        for pending in on_its_way {
            let served = state == Finalized || (state == Verified && pending == Some(Finalize));
            let (store, flows) = (Store::default(), Flows::default());
            let held = negotiations(negotiation(state, pending));
            let s = racing(&store, &held, &flows);
            let handled = request_as(&s, CONSUMER);
            let case = format!("{state:?} with {pending:?} on its way");
            if served {
                assert!(handled.is_ok(), "{case}: {handled:?}");
                assert_eq!(held.0.borrow()[0].state, Finalized, "{case}");
                assert_eq!(flows.calls(), ["start urn:uuid:1"], "{case}");
            } else {
                let failure = handled.unwrap_err();
                assert!(failure.is_no_finalized_agreement(), "{case}: {failure:?}");
                assert_eq!(held.0.borrow()[0], negotiation(state, pending), "{case}");
                assert!(store.0.borrow().is_empty(), "{case}");
                assert!(flows.calls().is_empty(), "{case}");
            }
        }
    }
}

/// The consumer's address of a push transfer, with `token`.
fn push_address(token: &str) -> DataAddress {
    DataAddress {
        kind: DataAddressType,
        endpoint_type: crate::transfer::HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
        endpoint: Some("https://consumer.example/push/urn:uuid:c".to_owned()),
        endpoint_properties: vec![
            EndpointProperty::new("authorization", token),
            EndpointProperty::new("authType", "bearer"),
        ],
    }
}

fn push_request(address: Option<DataAddress>) -> TransferRequestMessage {
    let mut push = request("a1", "HttpData-PUSH");
    push.data_address = address;
    push
}

/// A push transfer that is STARTED, its start message acknowledged.
fn started_push<'a>(store: &'a Store, flows: &'a Flows) -> Service<'a> {
    let s = service(store, flows, TransferDecision::Start);
    let (_, out) = s
        .request(
            &push_request(Some(push_address("p1"))),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    s.delivered(&out.unwrap()).unwrap();
    s
}

#[test]
fn a_push_request_needs_a_valid_push_address() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let refused = |msg: &TransferRequestMessage| {
        reason(
            &s.request(msg, CONSUMER, &CallerCredentials::default())
                .unwrap_err(),
        )
    };
    assert!(refused(&push_request(None)).contains("has a dataAddress"));
    let mut pull_type = push_address("p1");
    pull_type.endpoint_type = "https://w3id.org/idsa/v4.1/HTTP".to_owned();
    assert!(refused(&push_request(Some(pull_type))).contains("endpointType"));
    let mut no_endpoint = push_address("p1");
    no_endpoint.endpoint = None;
    assert!(refused(&push_request(Some(no_endpoint))).contains("endpoint"));
    let mut basic = push_address("p1");
    basic.endpoint_properties[1] = EndpointProperty::new("authType", "basic");
    let refusal = refused(&push_request(Some(basic)));
    assert!(refusal.contains("bearer"), "{refusal}");
    assert!(!refusal.contains("p1"), "the token is not repeated");
    assert!(store.0.borrow().is_empty());
    assert!(flows.calls().is_empty());
}

#[test]
fn a_push_starts_with_the_consumers_address_and_no_address_of_ours() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let (_, out) = s
        .request(
            &push_request(Some(push_address("p1"))),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    assert_eq!(flows.calls(), ["start urn:uuid:1 push with p1"]);
    let out = out.unwrap();
    let sent: TransferStartMessage = serde_json::from_value(out.body.clone()).unwrap();
    assert_eq!(sent.data_address, None, "DSP: no dataAddress for a push");
    assert_eq!(
        store.0.borrow()[PID].data_address,
        Some(push_address("p1")),
        "kept until the transfer ends"
    );
    // the same request again is the same transfer; another address is not
    let (again, none) = s
        .request(
            &push_request(Some(push_address("p1"))),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    assert_eq!((again.provider_pid.as_str(), none), (PID, None));
    assert!(
        s.request(
            &push_request(Some(push_address("other"))),
            CONSUMER,
            &CallerCredentials::default()
        )
        .is_err()
    );
}

#[test]
fn a_finished_push_completes_and_a_failed_one_terminates() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started_push(&store, &flows);
    let out = s.push_finished(PID, None).unwrap().unwrap();
    assert_eq!(out.transition, Transition::Complete);
    assert_eq!(flows.calls().last().unwrap(), "complete urn:uuid:1");
    s.delivered(&out).unwrap();
    assert_eq!(state(&store), TransferState::Completed);
    assert_eq!(store.0.borrow()[PID].data_address, None, "no token kept");

    let (store, flows) = (Store::default(), Flows::default());
    let s = started_push(&store, &flows);
    let failed = Some("the consumer answered HTTP 503");
    let out = s.push_finished(PID, failed).unwrap().unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    assert!(out.body.to_string().contains("HTTP 503"));
    assert_eq!(flows.calls().last().unwrap(), "terminate urn:uuid:1");
}

#[test]
fn a_push_that_ends_after_a_suspension_changes_nothing() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started_push(&store, &flows);
    s.suspend(PID, IDS, CONSUMER).unwrap();
    assert_eq!(s.push_finished(PID, None).unwrap(), None);
    assert_eq!(state(&store), TransferState::Suspended);
}

#[test]
fn a_push_started_again_by_the_consumer_takes_its_new_address() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started_push(&store, &flows);
    s.suspend(PID, IDS, CONSUMER).unwrap();
    let mut restart: TransferStartMessage = serde_json::from_value(json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@type": "TransferStartMessage", "providerPid": PID, "consumerPid": "urn:uuid:c"
    }))
    .unwrap();
    restart.data_address = Some(push_address("p2"));
    let (ack, out) = s.start(PID, &restart, CONSUMER).unwrap();
    assert_eq!((ack.state, out), (TransferState::Started, None));
    assert_eq!(
        flows.calls().last().unwrap(),
        "resume urn:uuid:1 push with p2"
    );
    assert_eq!(store.0.borrow()[PID].data_address, Some(push_address("p2")));

    // a pull transfer is not started again with an address
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    s.suspend(PID, IDS, CONSUMER).unwrap();
    let refusal = reason(&s.start(PID, &restart, CONSUMER).unwrap_err());
    assert!(refusal.contains("only a push transfer"), "{refusal}");
    assert_eq!(state(&store), TransferState::Suspended);
}

#[test]
fn the_provider_starts_a_push_again_without_an_address() {
    use TransferDecision::{Start, Suspend};
    let (store, flows) = (Store::default(), Flows::default());
    let steps = Steps(RefCell::new(vec![Start, Suspend, Start]));
    let s = TransferService::new(&store, Agreed, &flows, &steps, Catalog, Env::default());
    let (_, out) = s
        .request(
            &push_request(Some(push_address("p1"))),
            CONSUMER,
            &CallerCredentials::default(),
        )
        .unwrap();
    let mut sent = Vec::new();
    let mut next = out;
    while let Some(out) = next {
        next = s.delivered(&out).unwrap();
        sent.push(out);
    }
    assert_eq!(sent[2].transition, Transition::Start);
    assert!(sent[2].body.get("dataAddress").is_none());
    assert_eq!(flows.calls().last().unwrap(), "restart urn:uuid:1");
    assert_eq!(state(&store), TransferState::Started);
}

/// ADR 0027, section 7.1: the provider ends a started transfer on its
/// own. The data flow ends first, then the termination is sent; once the
/// consumer acknowledges it, the transfer is TERMINATED.
#[test]
fn the_provider_ends_a_transfer_on_its_own() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    let (transfer, out) = s.terminate_own(PID, "access withdrawn").unwrap();
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    assert_eq!(out.body["reason"][0], "access withdrawn");
    assert_eq!(transfer.state, TransferState::Started, "until acknowledged");
    assert_eq!(flows.calls().last().unwrap(), "terminate urn:uuid:1");
    // asked again while it is on its way: nothing new to send
    let (_, again) = s.terminate_own(PID, "again").unwrap();
    assert!(again.is_none());
    s.delivered(&out).unwrap();
    assert_eq!(state(&store), TransferState::Terminated);
    // an ended transfer cannot be ended again
    let failure = s.terminate_own(PID, "once more").unwrap_err();
    assert!(reason(&failure).contains("has ended"), "{failure:?}");
    assert!(matches!(
        s.terminate_own("unknown", "x"),
        Err(Failure::NotFound(_))
    ));
}

/// A termination of the provider replaces its start message that is still
/// on its way; a late acknowledgement of the start then changes nothing.
#[test]
fn the_providers_termination_replaces_a_message_on_its_way() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = service(&store, &flows, TransferDecision::Start);
    let none = CallerCredentials::default();
    let (_, start) = s
        .request(&request("a1", "HttpData-PULL"), CONSUMER, &none)
        .unwrap();
    let start = start.unwrap();
    let (_, termination) = s.terminate_own(PID, "revoked").unwrap();
    let termination = termination.unwrap();
    assert!(s.delivered(&start).unwrap().is_none(), "no longer pending");
    assert_eq!(state(&store), TransferState::Requested);
    s.delivered(&termination).unwrap();
    assert_eq!(state(&store), TransferState::Terminated);
}

/// A data plane that fails stops the termination: nothing is sent while
/// access may still be open.
#[test]
fn the_providers_termination_needs_the_data_plane() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    flows.1.set(true);
    assert!(matches!(
        s.terminate_own(PID, "x"),
        Err(Failure::DataPlane(_))
    ));
    assert!(store.0.borrow()[PID].pending.is_none());
}

#[test]
fn the_provider_reads_and_lists_its_transfers() {
    let (store, flows) = (Store::default(), Flows::default());
    let s = started(&store, &flows);
    assert_eq!(s.transfer(PID).unwrap().counter_party, CONSUMER);
    assert!(matches!(s.transfer("unknown"), Err(Failure::NotFound(_))));
    let mut second = request("a1", "HttpData-PULL");
    second.consumer_pid = "urn:uuid:c2".to_owned();
    s.request(&second, CONSUMER, &CallerCredentials::default())
        .unwrap();
    let all = s.list(None, 100).unwrap();
    assert_eq!(all.len(), 2);
    let page = s.list(None, 1).unwrap();
    assert_eq!(page.len(), 1);
    let rest = s.list(Some(&page[0].provider_pid), 0).unwrap();
    assert_eq!(rest.len(), 1, "a limit of 0 counts as 1");
    assert_ne!(rest[0].provider_pid, page[0].provider_pid);
}
