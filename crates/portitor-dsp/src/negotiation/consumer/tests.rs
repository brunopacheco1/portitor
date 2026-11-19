// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::unwrap_used, clippy::needless_pass_by_value)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::json;

use super::*;

/// `.1`: how many writes lose the version check as if another write came
/// first.
#[derive(Default)]
struct Store(RefCell<BTreeMap<String, ConsumerNegotiation>>, Cell<u32>);

impl ConsumerStore for &Store {
    fn get(&self, cpid: &str) -> Result<Option<ConsumerNegotiation>, StoreError> {
        Ok(self.0.borrow().get(cpid).cloned())
    }
    fn insert(&self, n: &ConsumerNegotiation) -> Result<(), StoreError> {
        self.0
            .borrow_mut()
            .insert(n.consumer_pid.clone(), n.clone());
        Ok(())
    }
    fn replace_if(&self, n: &ConsumerNegotiation, expected: u64) -> Result<bool, StoreError> {
        let mut map = self.0.borrow_mut();
        if self.1.get() > 0 {
            self.1.set(self.1.get() - 1);
            if let Some(old) = map.get_mut(&n.consumer_pid) {
                old.version += 1;
            }
            return Ok(false);
        }
        match map.get(&n.consumer_pid) {
            Some(old) if old.version == expected => {
                map.insert(n.consumer_pid.clone(), n.clone());
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
        let map = self.0.borrow();
        Ok(map
            .values()
            .filter(|n| after.is_none_or(|a| n.consumer_pid.as_str() > a))
            .take(limit)
            .cloned()
            .collect())
    }
    fn pending(&self) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        Ok(self
            .0
            .borrow()
            .values()
            .filter(|n| n.pending.is_some())
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct Env(Cell<u32>);

impl Environment for Env {
    fn new_id(&self) -> String {
        self.0.set(self.0.get() + 1);
        format!("urn:uuid:c{}", self.0.get())
    }
    fn now(&self) -> String {
        "2026-11-12T12:00:00Z".to_owned()
    }
}

struct Fixed(ConsumerDecision);

impl ConsumerDecider for Fixed {
    fn decide(&self, _: &ConsumerNegotiation) -> ConsumerDecision {
        self.0
    }
}

const CB: &str = "https://consumer.example/dsp/2025-1/callback";
const PROVIDER: &str = "https://provider.example/dsp/2025-1/";
/// Our participant id, the assignee of the agreements.
const ME: &str = "did:web:c";

fn offer() -> Offer {
    serde_json::from_value(json!({"@id": "o1", "@type": "Offer", "target": "d1",
        "permission": [{"action": "use"}]}))
    .unwrap()
}

fn ctx() -> serde_json::Value {
    json!(["https://w3id.org/dspace/2025/1/context.jsonld"])
}

fn offer_msg(ppid: &str, cpid: &str) -> ContractOfferMessage {
    serde_json::from_value(json!({"@context": ctx(), "@type": "ContractOfferMessage",
        "providerPid": ppid, "consumerPid": cpid,
        "offer": {"@id": "o2", "@type": "Offer", "target": "d1", "permission": [{"action": "use"}]}}))
    .unwrap()
}

fn agreement_msg(ppid: &str, cpid: &str) -> ContractAgreementMessage {
    serde_json::from_value(
        json!({"@context": ctx(), "@type": "ContractAgreementMessage",
        "providerPid": ppid, "consumerPid": cpid,
        "agreement": {"@id": "a1", "@type": "Agreement", "target": "d1",
            "assigner": "did:web:p", "assignee": "did:web:c",
            "timestamp": "2026-11-12T12:00:00Z", "permission": [{"action": "use"}]}}),
    )
    .unwrap()
}

fn event_msg(ppid: &str, cpid: &str, event: &str) -> ContractNegotiationEventMessage {
    serde_json::from_value(
        json!({"@context": ctx(), "@type": "ContractNegotiationEventMessage",
        "providerPid": ppid, "consumerPid": cpid, "eventType": event}),
    )
    .unwrap()
}

fn termination_msg(ppid: &str, cpid: &str) -> ContractNegotiationTerminationMessage {
    serde_json::from_value(json!({"@context": ctx(),
        "@type": "ContractNegotiationTerminationMessage",
        "providerPid": ppid, "consumerPid": cpid, "reason": ["bye"]}))
    .unwrap()
}

fn ack(ppid: &str, cpid: &str) -> serde_json::Value {
    json!({"@context": ctx(), "@type": "ContractNegotiation", "providerPid": ppid,
        "consumerPid": cpid, "state": "REQUESTED"})
}

fn state(store: &Store, cpid: &str) -> NegotiationState {
    store.0.borrow()[cpid].state
}

fn status(f: Failure) -> &'static str {
    match f {
        Failure::BadRequest(_) => "400",
        Failure::NotFound(_) => "404",
        Failure::Store(_) => "500",
    }
}

#[test]
fn request_offer_accept_agree_verify_finalize() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid.clone();
    assert_eq!(n.state, NegotiationState::Requested);
    assert_eq!(
        req.url,
        "https://provider.example/dsp/2025-1/negotiations/request"
    );
    assert_eq!(req.body["callbackAddress"], CB);
    assert_eq!(req.body["consumerPid"], cpid.as_str());
    assert!(req.body.get("providerPid").is_none());
    assert!(req.initial);

    // the provider acknowledges; the default decider waits for an offer
    assert_eq!(svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap(), None);
    assert_eq!(svc.get(&cpid).unwrap().provider_pid, "p1");

    let (a, accept) = svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Offered);
    let accept = accept.unwrap();
    assert_eq!(
        accept.url,
        "https://provider.example/dsp/2025-1/negotiations/p1/events"
    );
    assert_eq!(accept.body["eventType"], "ACCEPTED");
    assert_eq!(svc.delivered(&accept, None).unwrap(), None);
    assert_eq!(state(&store, &cpid), NegotiationState::Accepted);

    // agreements from another provider, or for another consumer, are
    // refused for that reason and change nothing
    for (field, reason) in [
        ("assigner", "the agreement's assigner is not the provider"),
        ("assignee", "the agreement's assignee is not this consumer"),
    ] {
        let mut wrong = agreement_msg("p1", &cpid);
        if field == "assigner" {
            wrong.agreement.assigner = "did:web:x".to_owned();
        } else {
            wrong.agreement.assignee = "did:web:x".to_owned();
        }
        let Failure::BadRequest(error) = svc.agreement(&cpid, &wrong).unwrap_err() else {
            panic!("{field}: not a 400");
        };
        assert_eq!(error.reason, [json!(reason)], "{field}");
        assert_eq!(state(&store, &cpid), NegotiationState::Accepted);
    }

    let (a, verify) = svc.agreement(&cpid, &agreement_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Agreed);
    let verify = verify.unwrap();
    assert!(
        verify
            .url
            .ends_with("/negotiations/p1/agreement/verification")
    );
    assert_eq!(verify.body["@type"], "ContractAgreementVerificationMessage");
    svc.delivered(&verify, None).unwrap();
    assert_eq!(state(&store, &cpid), NegotiationState::Verified);
    assert_eq!(store.0.borrow()[&cpid].agreement.as_ref().unwrap().id, "a1");

    let (a, next) = svc
        .event(&cpid, &event_msg("p1", &cpid, "FINALIZED"))
        .unwrap();
    assert_eq!(a.state, NegotiationState::Finalized);
    assert!(next.is_none());
}

#[test]
fn provider_message_before_the_acknowledgement() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    let (_, accept) = svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    assert!(
        accept.is_some(),
        "the offer sets the provider pid and is accepted"
    );
    // the late acknowledgement must not trigger a second accept
    assert_eq!(svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap(), None);
    assert_eq!(state(&store, &cpid), NegotiationState::Offered);
}

#[test]
fn counter_offer_and_terminations() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        Fixed(ConsumerDecision::Request),
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    // decided at REQUESTED too, but a request is not a valid next step
    // there; the provider would refuse it, the domain only proposes it
    let first = svc
        .delivered(&req, Some(&ack("p1", &cpid)))
        .unwrap()
        .unwrap();
    assert!(first.url.ends_with("/negotiations/p1/request"));
    let (_, counter) = svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    let counter = counter.unwrap();
    assert_eq!(counter.body["providerPid"], "p1");
    assert!(counter.body.get("callbackAddress").is_none());
    assert_eq!(counter.body["offer"]["@id"], "o2");
    svc.delivered(&counter, None).unwrap();
    assert_eq!(state(&store, &cpid), NegotiationState::Requested);

    let (a, _) = svc.terminate(&cpid, &termination_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Terminated);
    assert_eq!(
        status(
            svc.terminate(&cpid, &termination_msg("p1", &cpid))
                .unwrap_err()
        ),
        "400"
    );
    assert_eq!(svc.abort(&counter, "x").unwrap(), None);
}

#[test]
fn consumer_terminates_and_aborts() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        Fixed(ConsumerDecision::Terminate),
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    // no provider pid yet: nothing can be sent
    assert_eq!(svc.abort(&req, "x").unwrap(), None);
    let term = svc
        .delivered(&req, Some(&ack("p1", &cpid)))
        .unwrap()
        .unwrap();
    assert!(term.url.ends_with("/negotiations/p1/termination"));
    assert_eq!(term.body["reason"][0], "terminated by the consumer");
    svc.delivered(&term, None).unwrap();
    assert_eq!(state(&store, &cpid), NegotiationState::Terminated);
}

#[test]
fn invalid_provider_messages_are_refused() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        Fixed(ConsumerDecision::Wait),
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();

    // FINALIZED while REQUESTED
    let e = svc
        .event(&cpid, &event_msg("p1", &cpid, "FINALIZED"))
        .unwrap_err();
    assert_eq!(status(e), "400");
    // ACCEPTED is a consumer's event
    let e = svc
        .event(&cpid, &event_msg("p1", &cpid, "ACCEPTED"))
        .unwrap_err();
    assert_eq!(status(e), "400");
    // another provider's pid, or another negotiation's consumer pid
    assert_eq!(
        status(svc.offer(&cpid, &offer_msg("p2", &cpid)).unwrap_err()),
        "400"
    );
    assert_eq!(
        status(svc.offer(&cpid, &offer_msg("p1", "other")).unwrap_err()),
        "400"
    );
    assert_eq!(
        status(
            svc.offer("unknown", &offer_msg("p1", "unknown"))
                .unwrap_err()
        ),
        "404"
    );
    assert_eq!(status(svc.get("unknown").unwrap_err()), "404");

    svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    // agreement while OFFERED (the consumer has not accepted)
    assert_eq!(
        status(
            svc.agreement(&cpid, &agreement_msg("p1", &cpid))
                .unwrap_err()
        ),
        "400"
    );
    // a second offer while OFFERED
    assert_eq!(
        status(svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap_err()),
        "400"
    );
    assert_eq!(state(&store, &cpid), NegotiationState::Offered);
    // an agreement for another dataset
    let mut other = agreement_msg("p1", &cpid);
    other.agreement.target = "d2".into();
    assert_eq!(status(svc.agreement(&cpid, &other).unwrap_err()), "400");
}

#[test]
fn bad_starts() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let mut o = offer();
    o.target = None;
    assert_eq!(status(svc.start(PROVIDER, "p", o).unwrap_err()), "400");
    let mut o = offer();
    o.permission.clear();
    assert_eq!(status(svc.start(PROVIDER, "p", o).unwrap_err()), "400");

    let (n, req) = svc.start(PROVIDER, "p", offer()).unwrap();
    // an answer without a provider pid
    let e = svc
        .delivered(&req, Some(&json!({"state": "REQUESTED"})))
        .unwrap_err();
    assert_eq!(status(e), "400");
    assert_eq!(status(svc.delivered(&req, None).unwrap_err()), "400");
    svc.not_started(&n.consumer_pid).unwrap();
    assert_eq!(state(&store, &n.consumer_pid), NegotiationState::Terminated);
    // the provider's message without a provider pid
    let e = svc
        .offer(&n.consumer_pid, &offer_msg("", &n.consumer_pid))
        .unwrap_err();
    assert_eq!(status(e), "400");
}

#[test]
fn reads_and_pages() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let mut pids = Vec::new();
    for _ in 0..3 {
        pids.push(svc.start(PROVIDER, "p", offer()).unwrap().0.consumer_pid);
    }
    pids.sort();
    assert_eq!(svc.negotiation(&pids[1]).unwrap().consumer_pid, pids[1]);
    assert_eq!(status(svc.negotiation("unknown").unwrap_err()), "404");
    let first = svc.list(None, 2).unwrap();
    assert_eq!(first.len(), 2);
    let rest = svc.list(Some(&first[1].consumer_pid), 0).unwrap();
    assert_eq!(rest.len(), 1, "a limit of 0 still returns one");
    assert_eq!(rest[0].consumer_pid, pids[2]);
    assert_eq!(svc.list(None, 10_000).unwrap().len(), 3);
}

/// The provider agrees before our acknowledged acceptance is recorded,
/// and finalizes before our acknowledged verification is: each message
/// proves our pending one arrived.
#[test]
fn a_reply_proves_the_pending_message_arrived() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    assert_eq!(svc.pending().unwrap(), vec![req.clone()]);
    assert_eq!(
        (req.to, req.status_url.as_str()),
        (NegotiationState::Requested, "")
    );
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();
    assert!(svc.pending().unwrap().is_empty());

    let (_, accept) = svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    let accept = accept.unwrap();
    assert_eq!(accept.to, NegotiationState::Accepted);
    assert_eq!(
        accept.status_url,
        "https://provider.example/dsp/2025-1/negotiations/p1"
    );
    let (a, verify) = svc.agreement(&cpid, &agreement_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Agreed);
    let verify = verify.unwrap();
    assert_eq!(svc.delivered(&accept, None).unwrap(), None);
    assert_eq!(svc.abort(&accept, "timed out").unwrap(), None);
    assert_eq!(state(&store, &cpid), NegotiationState::Agreed);

    let (a, next) = svc
        .event(&cpid, &event_msg("p1", &cpid, "FINALIZED"))
        .unwrap();
    assert_eq!((a.state, next), (NegotiationState::Finalized, None));
    assert_eq!(svc.delivered(&verify, None).unwrap(), None);
    assert_eq!(state(&store, &cpid), NegotiationState::Finalized);
    assert!(svc.pending().unwrap().is_empty());
}

#[test]
fn not_started_only_before_the_provider_answers() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        Fixed(ConsumerDecision::Wait),
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, _) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    // the request timed out, but the provider had received it
    svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    svc.not_started(&cpid).unwrap();
    assert_eq!(state(&store, &cpid), NegotiationState::Offered);
}

#[test]
fn abort_blocks_other_steps_and_retries_lost_writes() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    // two other writes come first: the third try succeeds
    store.1.set(2);
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();
    assert_eq!(store.0.borrow()[&cpid].provider_pid.as_deref(), Some("p1"));

    let (_, accept) = svc.offer(&cpid, &offer_msg("p1", &cpid)).unwrap();
    let term = svc
        .abort(&accept.unwrap(), "provider unreachable")
        .unwrap()
        .unwrap();
    assert_eq!(term.to, NegotiationState::Terminated);
    assert_eq!(svc.pending().unwrap(), vec![term.clone()]);
    let e = svc
        .agreement(&cpid, &agreement_msg("p1", &cpid))
        .unwrap_err();
    assert_eq!(status(e), "400");
    // the provider's own termination crosses ours
    let (a, _) = svc.terminate(&cpid, &termination_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Terminated);
    assert_eq!(svc.delivered(&term, None).unwrap(), None);

    // other writes keep coming first: give up
    let (n, _) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    store.1.set(3);
    assert_eq!(status(svc.not_started(&n.consumer_pid).unwrap_err()), "400");
    assert_eq!(state(&store, &n.consumer_pid), NegotiationState::Requested);
}

/// ADR 0009: the consumer accepts only an offer whose constraints hold
/// now, and verifies only an agreement with the terms it asked for.
#[test]
fn checks_offers_and_agreements() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );

    // an offer that expired before the test clock (2026-11-12)
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();
    let mut expired = offer_msg("p1", &cpid);
    expired.offer.permission[0].constraint = vec![
        serde_json::from_value(json!(
            {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2026-01-01"}
        ))
        .unwrap(),
    ];
    let (a, next) = svc.offer(&cpid, &expired).unwrap();
    assert_eq!(a.state, NegotiationState::Offered);
    let next = next.unwrap();
    assert_eq!(next.transition, Transition::Terminate);
    let why = next.body["reason"][0].as_str().unwrap();
    assert!(why.contains("not satisfied"), "{why}");

    // an agreement whose terms are not the ones we asked for
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();
    let mut other = agreement_msg("p1", &cpid);
    other.agreement.permission[0].action = "distribute".to_owned();
    let (a, next) = svc.agreement(&cpid, &other).unwrap();
    assert_eq!(a.state, NegotiationState::Agreed);
    let next = next.unwrap();
    assert_eq!(next.transition, Transition::Terminate);
    let why = next.body["reason"][0].as_str().unwrap();
    assert!(why.contains("differ"), "{why}");
}

/// DSP 2025-1, "Contract Agreement Message": "An Agreement MUST contain a
/// `timestamp` property defined as an XSD `DateTime` type."
#[test]
fn an_agreement_without_a_timestamp_is_refused() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid;
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();

    for (timestamp, why) in [
        (None, "no timestamp"),
        (Some("yesterday"), "not an XSD dateTime"),
        // a date alone is not a date and time
        (Some("2026-11-12"), "not an XSD dateTime"),
    ] {
        let mut msg = agreement_msg("p1", &cpid);
        msg.agreement.timestamp = timestamp.map(str::to_owned);
        let Failure::BadRequest(error) = svc.agreement(&cpid, &msg).unwrap_err() else {
            panic!("{timestamp:?}: not a 400");
        };
        let text = serde_json::to_string(&error).unwrap();
        assert!(text.contains(why), "{timestamp:?}: {text}");
        assert!(text.contains("ContractNegotiationError"), "{text}");
        assert_eq!(state(&store, &cpid), NegotiationState::Requested);
        assert!(store.0.borrow()[&cpid].agreement.is_none());
    }

    // with a time zone, without one (xsd:dateTime allows both), and with
    // a fraction: accepted
    for timestamp in ["2026-11-12T12:00:00", "2026-11-12T12:00:00.5+01:00"] {
        assert!(is_xsd_date_time(timestamp), "{timestamp}");
    }
    let (a, _) = svc.agreement(&cpid, &agreement_msg("p1", &cpid)).unwrap();
    assert_eq!(a.state, NegotiationState::Agreed);
}

/// The forms of `xsd:dateTime` an agreement's `timestamp` may take (W3C
/// XML Schema Part 2, "dateTime"; the pattern of `timestamp` in DSP
/// 2025-1 `contract-schema.json`), and texts that only look like one.
#[test]
fn the_forms_of_an_agreement_timestamp() {
    for accepted in [
        "2026-11-12T12:00:00Z",
        "2026-11-12T12:00:00",
        "2026-11-12T12:00:00+00:00",
        "2026-11-12T12:00:00-00:00",
        "2026-11-12T23:59:59+14:00",
        "2026-11-12T00:00:00-14:00",
        "2026-11-12T12:00:00.123456789Z",
        // more fraction digits than a nanosecond
        "2026-11-12T12:00:00.1234567890123Z",
        "2026-11-12T12:00:00.5",
        // 29 February of a leap year
        "2028-02-29T12:00:00Z",
        "0001-01-01T00:00:00Z",
        "9999-12-31T23:59:59Z",
    ] {
        assert!(is_xsd_date_time(accepted), "{accepted}");
    }
    for refused in [
        "",
        "Z",
        " 2026-11-12T12:00:00Z",
        "2026-11-12T12:00:00Z ",
        "2026-11-12T12:00:00Z\n",
        // lower case, a space for the `T`, no separators
        "2026-11-12t12:00:00z",
        "2026-11-12 12:00:00Z",
        "20261112T120000Z",
        // a time alone, a date alone, a date with a zone
        "12:00:00Z",
        "2026-11-12",
        "2026-11-12Z",
        "2026-11-12T",
        // not on the calendar or the clock
        "2026-13-01T12:00:00Z",
        "2026-00-10T12:00:00Z",
        "2026-02-29T12:00:00Z",
        "2026-11-31T12:00:00Z",
        "2026-11-12T25:00:00Z",
        "2026-11-12T12:60:00Z",
        "2026-11-12T12:00:60Z",
        // a zone out of range, half a zone, two zones
        "2026-11-12T12:00:00+14:01",
        "2026-11-12T12:00:00+15:00",
        "2026-11-12T12:00:00+01",
        "2026-11-12T12:00:00+0100",
        "2026-11-12T12:00:00-",
        "2026-11-12T12:00:00ZZ",
        "2026-11-12T12:00:00+01:00Z",
        // a fraction without digits, a sign, digits that are not ASCII
        "2026-11-12T12:00:00.Z",
        "2026-11-12T12:00:00.",
        "2026-11-12T12:00:00,5Z",
        "+2026-11-12T12:00:00Z",
        "2026-11-12T12:00:0\u{0660}Z",
        "\u{ff12}026-11-12T12:00:00Z",
        // seconds since 1970, as a JWT would have it
        "1794484800",
    ] {
        assert!(!is_xsd_date_time(refused), "{refused:?}");
        assert_eq!(
            check_timestamp(Some(refused)),
            Err("the agreement's timestamp is not an XSD dateTime"),
            "{refused:?}"
        );
    }
}

/// `xsd:dateTime` writes midnight at the end of a day as `24:00:00` (W3C
/// XML Schema Part 2, "dateTime": "hh ... or 24 if the minutes and
/// seconds are zero"); the DSP 2025-1 pattern for `timestamp` has it too
/// (`contract-schema.json`, `Agreement`: `24:00:00(\.0+)?`).
#[test]
fn a_timestamp_at_the_end_of_the_day_is_accepted() {
    assert!(is_xsd_date_time("2026-11-12T24:00:00Z"));
    assert!(is_xsd_date_time("2026-11-12T24:00:00.000+01:00"));
    // only with minutes and seconds of zero
    assert!(!is_xsd_date_time("2026-11-12T24:00:01Z"));
    assert!(!is_xsd_date_time("2026-11-12T24:30:00Z"));
}

#[test]
fn only_the_provider_it_was_started_with_may_answer() {
    // ADR 0016: the caller's proven DID must be the negotiation's provider
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    let (n, req) = svc.start(PROVIDER, "did:web:p", offer()).unwrap();
    let cpid = n.consumer_pid.clone();
    assert_eq!(req.counter_party, "did:web:p");
    assert!(svc.from_provider(&cpid, "did:web:p").is_ok());
    assert_eq!(
        status(svc.from_provider(&cpid, "did:web:eve").unwrap_err()),
        "404"
    );
    assert_eq!(
        status(svc.from_provider("unknown", "did:web:p").unwrap_err()),
        "404"
    );

    // an agreement whose assigner is another participant is refused
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();
    let mut msg = agreement_msg("p1", &cpid);
    msg.agreement.assigner = "did:web:eve".to_owned();
    assert_eq!(status(svc.agreement(&cpid, &msg).unwrap_err()), "400");
    assert_eq!(state(&store, &cpid), NegotiationState::Requested);
    assert!(svc.agreement(&cpid, &agreement_msg("p1", &cpid)).is_ok());
}

/// An offer that names its parties must name the provider of the
/// negotiation and this consumer: when we start with it, and when the
/// provider sends one.
#[test]
fn the_parties_an_offer_names_are_checked() {
    let store = Store::default();
    let svc = ConsumerService::new(
        &store,
        AcceptAndVerify,
        Env::default(),
        CB.to_owned(),
        ME.to_owned(),
    );
    // our own start, with an offer for another provider or consumer
    let mut other_provider = offer();
    other_provider.assigner = Some("did:web:x".to_owned());
    let failure = svc
        .start(PROVIDER, "did:web:p", other_provider)
        .unwrap_err();
    assert_eq!(status(failure), "400");
    let mut other_consumer = offer();
    other_consumer.assignee = Some("did:web:x".to_owned());
    let failure = svc
        .start(PROVIDER, "did:web:p", other_consumer)
        .unwrap_err();
    assert_eq!(status(failure), "400");
    assert!(store.0.borrow().is_empty());

    // an offer naming the right parties starts, and the request carries them
    let mut named = offer();
    named.assigner = Some("did:web:p".to_owned());
    named.assignee = Some(ME.to_owned());
    let (n, req) = svc.start(PROVIDER, "did:web:p", named).unwrap();
    let cpid = n.consumer_pid.clone();
    assert_eq!(req.body["offer"]["assigner"], "did:web:p");
    assert_eq!(req.body["offer"]["assignee"], ME);
    svc.delivered(&req, Some(&ack("p1", &cpid))).unwrap();

    // the provider's offer
    for (field, reason) in [
        ("assigner", "the offer's assigner is not the provider"),
        ("assignee", "the offer's assignee is not the consumer"),
    ] {
        let mut wrong = offer_msg("p1", &cpid);
        if field == "assigner" {
            wrong.offer.assigner = Some("did:web:x".to_owned());
        } else {
            wrong.offer.assignee = Some("did:web:x".to_owned());
        }
        let Failure::BadRequest(error) = svc.offer(&cpid, &wrong).unwrap_err() else {
            panic!("{field}: not a 400");
        };
        assert_eq!(error.reason, [json!(reason)], "{field}");
        assert_eq!(state(&store, &cpid), NegotiationState::Requested);
    }
    let mut right = offer_msg("p1", &cpid);
    right.offer.assigner = Some("did:web:p".to_owned());
    right.offer.assignee = Some(ME.to_owned());
    let (a, _) = svc.offer(&cpid, &right).unwrap();
    assert_eq!(a.state, NegotiationState::Offered);
}
