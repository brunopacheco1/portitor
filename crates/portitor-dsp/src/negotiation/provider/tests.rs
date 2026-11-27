// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::unwrap_used)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use super::*;
use crate::catalog::ListedDataset;
use crate::{Dataset, DatasetType, Distribution, DistributionType, OfferType, Rule};

/// `.1`: every write fails; `.2`: how many writes lose the version check
/// as if another write came first.
#[derive(Default)]
struct Store(
    RefCell<BTreeMap<String, Negotiation>>,
    Cell<bool>,
    Cell<u32>,
);

impl NegotiationStore for &Store {
    fn get(&self, pid: &str) -> Result<Option<Negotiation>, StoreError> {
        Ok(self.0.borrow().get(pid).cloned())
    }
    fn negotiation_requested_by(
        &self,
        party: &str,
        cpid: &str,
    ) -> Result<Option<Negotiation>, StoreError> {
        Ok(self
            .0
            .borrow()
            .values()
            .find(|n| n.counter_party == party && n.consumer_pid == cpid)
            .cloned())
    }
    fn insert(&self, n: &Negotiation) -> Result<bool, StoreError> {
        if self
            .negotiation_requested_by(&n.counter_party, &n.consumer_pid)?
            .is_some()
        {
            return Ok(false);
        }
        self.0
            .borrow_mut()
            .insert(n.provider_pid.clone(), n.clone());
        Ok(true)
    }
    fn replace_if(&self, n: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        if self.1.get() {
            return Err(StoreError("disk full".to_owned()));
        }
        let mut map = self.0.borrow_mut();
        if self.2.get() > 0 {
            self.2.set(self.2.get() - 1);
            if let Some(old) = map.get_mut(&n.provider_pid) {
                old.version += 1;
            }
            return Ok(false);
        }
        match map.get(&n.provider_pid) {
            Some(old) if old.version == expected => {
                map.insert(n.provider_pid.clone(), n.clone());
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        Ok(self
            .0
            .borrow()
            .values()
            .filter(|n| n.pending.is_some())
            .cloned()
            .collect())
    }
}

struct Catalog;

impl CatalogDatasets for Catalog {
    fn all(&self) -> Result<Vec<Dataset>, StoreError> {
        Ok(self.get("d1")?.into_iter().collect())
    }
    fn get(&self, id: &str) -> Result<Option<Dataset>, StoreError> {
        // d2's offer expired before the test clock (2026-11-12)
        let mut expired = offer(None);
        expired.permission[0].constraint = vec![
            serde_json::from_value(serde_json::json!(
                {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2026-01-01"}
            ))
            .unwrap(),
        ];
        let policy = match id {
            "d1" | "d4" => offer(None),
            "d2" => expired,
            "d3" => providers_only(None),
            _ => return Ok(None),
        };
        Ok(Some(Dataset {
            context: None,
            id: id.to_owned(),
            kind: DatasetType,
            has_policy: vec![policy],
            distribution: vec![Distribution {
                kind: DistributionType,
                format: "HttpData-PULL".to_owned(),
                access_service: "svc".to_owned(),
                metadata: crate::Metadata::default(),
            }],
            metadata: crate::Metadata::default(),
        }))
    }
    /// d4 has an access policy: only members with role `provider` see it
    /// (ADR 0019, amendment in 1.4). Its offer has no constraint.
    fn get_with_access_policy(&self, id: &str) -> Result<Option<ListedDataset>, StoreError> {
        let Some(dataset) = self.get(id)? else {
            return Ok(None);
        };
        let access_policy = if id == "d4" {
            Some(
                serde_json::from_value(serde_json::json!({"permission": [{"action": "use",
                    "constraint": [{"leftOperand": "MembershipCredential.role",
                    "operator": "eq", "rightOperand": "provider"}]}]}))
                .unwrap(),
            )
        } else {
            None
        };
        Ok(Some(ListedDataset {
            dataset,
            access_policy,
        }))
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
        "2026-11-12T12:00:00Z".to_owned()
    }
    fn now_seconds(&self) -> u64 {
        // 2026-11-12T12:00:00Z, the time above
        1_794_484_800
    }
    fn process_id(&self) -> &'static str {
        "test-process"
    }
}

/// Scripted decisions, like the TCK's.
struct Script(BTreeMap<NegotiationState, Decision>);

impl Decider for Script {
    fn decide(&self, n: &Negotiation) -> Decision {
        self.0.get(&n.state).copied().unwrap_or(Decision::Wait)
    }
}

fn offer(target: Option<&str>) -> Offer {
    Offer {
        id: "d1:offer".to_owned(),
        kind: OfferType,
        permission: vec![Rule {
            action: "use".to_owned(),
            constraint: vec![],
        }],
        prohibition: vec![],
        obligation: vec![],
        target: target.map(str::to_owned),
        assigner: None,
        assignee: None,
        profile: None,
    }
}

/// d3's offer: only for callers whose membership says role `provider`
/// (ADR 0019).
fn providers_only(target: Option<&str>) -> Offer {
    let mut offer = offer(target);
    offer.permission[0].constraint = vec![
        serde_json::from_value(
            serde_json::json!({"leftOperand": "MembershipCredential.role",
            "operator": "eq", "rightOperand": "provider"}),
        )
        .unwrap(),
    ];
    offer
}

/// A caller whose membership says role `role`.
fn member_with_role(role: &str) -> CallerCredentials {
    let subject = serde_json::json!({"id": "c", "role": role});
    CallerCredentials {
        understood: crate::policy::CredentialTypes::default(),
        held: vec![crate::policy::HeldCredential {
            types: vec!["MembershipCredential".to_owned()],
            subjects: vec![subject.as_object().unwrap().clone()],
        }],
    }
}

fn request(provider_pid: Option<&str>) -> ContractRequestMessage {
    ContractRequestMessage {
        context: Context::default(),
        kind: super::super::ContractRequestMessageType,
        consumer_pid: "c1".to_owned(),
        provider_pid: provider_pid.map(str::to_owned),
        offer: offer(Some("d1")),
        callback_address: Some("https://consumer.example/cb/".to_owned()),
    }
}

fn verification(pid: &str) -> ContractAgreementVerificationMessage {
    ContractAgreementVerificationMessage {
        context: Context::default(),
        kind: super::super::ContractAgreementVerificationMessageType,
        provider_pid: pid.to_owned(),
        consumer_pid: "c1".to_owned(),
    }
}

fn event(pid: &str, e: NegotiationEvent) -> ContractNegotiationEventMessage {
    ContractNegotiationEventMessage {
        context: Context::default(),
        kind: ContractNegotiationEventMessageType,
        provider_pid: pid.to_owned(),
        consumer_pid: "c1".to_owned(),
        event_type: e,
    }
}

fn termination(pid: &str) -> ContractNegotiationTerminationMessage {
    ContractNegotiationTerminationMessage {
        context: Context::default(),
        kind: ContractNegotiationTerminationMessageType,
        provider_pid: pid.to_owned(),
        consumer_pid: "c1".to_owned(),
        code: None,
        reason: vec![],
    }
}

type Service<'a, D> = ProviderService<&'a Store, D, Catalog, Env>;

fn service<D: Decider>(store: &Store, d: D) -> Service<'_, D> {
    ProviderService::new(
        store,
        d,
        Catalog,
        Env::default(),
        "did:web:provider".to_owned(),
    )
}

fn reason(f: &Failure) -> String {
    match f {
        Failure::BadRequest(e) | Failure::NotFound(e) => e.reason[0].to_string(),
        Failure::Store(e) => e.to_string(),
    }
}

#[test]
fn default_policy_agrees_and_finalizes() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (ack, out) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Requested);
    let pid = ack.provider_pid.clone();
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Agree);
    assert_eq!(
        out.url,
        format!("https://consumer.example/cb/negotiations/c1/agreement")
    );
    let agreement = &out.body["agreement"];
    assert_eq!(agreement["target"], "d1");
    assert_eq!(agreement["assigner"], "did:web:provider");
    assert_eq!(agreement["assignee"], "did:web:consumer");
    assert_eq!(agreement["timestamp"], "2026-11-12T12:00:00Z");

    // nothing changes until the consumer acknowledges
    assert_eq!(
        p.get(&pid, "did:web:consumer").unwrap().state,
        NegotiationState::Requested
    );
    assert_eq!(p.delivered(&out).unwrap(), None);
    assert_eq!(
        p.get(&pid, "did:web:consumer").unwrap().state,
        NegotiationState::Agreed
    );
    assert!(store.0.borrow()[&pid].agreement.is_some());

    let (ack, out) = p
        .verify(&pid, &verification(&pid), "did:web:consumer")
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Verified);
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Finalize);
    assert_eq!(out.body["eventType"], "FINALIZED");
    assert!(out.url.ends_with("/negotiations/c1/events"));
    p.delivered(&out).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Finalized);
}

#[test]
fn offer_accept_agree() {
    let store = Store::default();
    let script = Script(BTreeMap::from([
        (NegotiationState::Requested, Decision::Offer),
        (NegotiationState::Accepted, Decision::Agree),
    ]));
    let p = service(&store, script);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Offer);
    assert_eq!(out.body["@type"], "ContractOfferMessage");
    assert_eq!(p.delivered(&out).unwrap(), None);
    let (ack, out) = p
        .event(&pid, &event(&pid, NegotiationEvent::Accepted), "c", &none())
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Accepted);
    assert_eq!(out.unwrap().transition, Transition::Agree);

    // counter-offer is not allowed after accepting
    let err = p
        .request(Some(&pid), request(Some(&pid)), "c", &none())
        .unwrap_err();
    assert!(reason(&err).contains("not allowed"), "{err:?}");
}

#[test]
fn counter_offer_and_provider_termination() {
    let store = Store::default();
    let script = Script(BTreeMap::from([(
        NegotiationState::Requested,
        Decision::Terminate,
    )]));
    let p = service(&store, script);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    assert!(out.url.ends_with("/negotiations/c1/termination"));
    p.delivered(&out).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Terminated);
    // nothing more after a terminal state
    let err = p.terminate(&pid, &termination(&pid), "c").unwrap_err();
    assert!(reason(&err).contains("terminal"), "{err:?}");
    // a second acknowledgement of the same message changes nothing
    assert_eq!(p.delivered(&out).unwrap(), None);
    assert!(p.pending().unwrap().is_empty());
}

#[test]
fn consumer_counter_offer_and_termination() {
    let store = Store::default();
    let script = Script(BTreeMap::from([(
        NegotiationState::Requested,
        Decision::Offer,
    )]));
    let p = service(&store, script);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    p.delivered(&out.unwrap()).unwrap();
    let (ack, _) = p
        .request(Some(&pid), request(Some(&pid)), "c", &none())
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Requested);
    let err = p
        .request(Some(&pid), request(Some("x")), "c", &none())
        .unwrap_err();
    assert!(reason(&err).contains("does not match"), "{err:?}");
    let (ack, out) = p.terminate(&pid, &termination(&pid), "c").unwrap();
    assert_eq!(ack.state, NegotiationState::Terminated);
    assert_eq!(out, None);
}

/// A request on a running negotiation must name the `consumerPid` the
/// negotiation was started with, like every other message on it (TD-36).
#[test]
fn a_counter_request_with_another_consumer_pid_is_refused() {
    let store = Store::default();
    let script = Script(BTreeMap::from([(
        NegotiationState::Requested,
        Decision::Offer,
    )]));
    let p = service(&store, script);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    p.delivered(&out.unwrap()).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Offered);

    let mut other_pid = request(Some(&pid));
    other_pid.consumer_pid = "c2".to_owned();
    let err = p.request(Some(&pid), other_pid, "c", &none()).unwrap_err();
    assert!(matches!(err, Failure::BadRequest(_)), "{err:?}");
    assert!(reason(&err).contains("process ids do not match"), "{err:?}");
    // nothing changed: still OFFERED, with the first consumer pid
    let stored = store.0.borrow()[&pid].clone();
    assert_eq!(stored.state, NegotiationState::Offered);
    assert_eq!(stored.consumer_pid, "c1");
}

#[test]
fn refuses_bad_requests() {
    let store = Store::default();
    // waits: nothing is pending, so nothing can prove a later state
    let p = service(&store, Script(BTreeMap::new()));

    let mut no_cb = request(None);
    no_cb.callback_address = None;
    assert!(reason(&p.request(None, no_cb, "c", &none()).unwrap_err()).contains("callbackAddress"));

    let mut unknown = request(None);
    unknown.offer.id = "other".to_owned();
    assert!(reason(&p.request(None, unknown, "c", &none()).unwrap_err()).contains("unknown offer"));

    let mut no_target = request(None);
    no_target.offer.target = None;
    assert!(reason(&p.request(None, no_target, "c", &none()).unwrap_err()).contains("no target"));

    let mut other_dataset = request(None);
    other_dataset.offer.target = Some("d7".to_owned());
    assert!(p.request(None, other_dataset, "c", &none()).is_err());

    // unknown negotiation, and another consumer's negotiation: 404
    assert!(matches!(p.get("nope", "c"), Err(Failure::NotFound(_))));
    let (ack, _) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    assert!(matches!(p.get(&pid, "intruder"), Err(Failure::NotFound(_))));
    assert!(matches!(
        p.verify(&pid, &verification(&pid), "intruder"),
        Err(Failure::NotFound(_))
    ));

    // wrong state, wrong ids
    let err = p.verify(&pid, &verification(&pid), "c").unwrap_err();
    assert!(reason(&err).contains("not allowed"), "{err:?}");
    let err = p.verify(&pid, &verification("other"), "c").unwrap_err();
    assert!(reason(&err).contains("do not match"), "{err:?}");
    let err = p
        .event(
            &pid,
            &event(&pid, NegotiationEvent::Finalized),
            "c",
            &none(),
        )
        .unwrap_err();
    assert!(reason(&err).contains("Consumer"), "{err:?}");
}

#[test]
fn concurrent_change_and_store_failure() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    // a consumer of its own: one consumer pid starts one negotiation
    let (ack, out) = p.request(None, request(None), "c0", &none()).unwrap();
    let pid = ack.provider_pid.clone();
    let out = out.unwrap();
    // the consumer terminates while the agreement is on its way; the
    // late acknowledgement of the agreement then changes nothing
    p.terminate(&pid, &termination(&pid), "c0").unwrap();
    assert_eq!(p.delivered(&out).unwrap(), None);
    let n = store.0.borrow()[&pid].clone();
    assert_eq!(n.state, NegotiationState::Terminated);
    assert_eq!((n.agreement, n.pending), (None, None));

    let (ack, agree) = p.request(None, request(None), "c", &none()).unwrap();
    let agree = agree.unwrap();
    store.1.set(true);
    let err = p
        .terminate(&ack.provider_pid, &termination(&ack.provider_pid), "c")
        .unwrap_err();
    assert!(matches!(err, Failure::Store(_)));
    assert!(reason(&err).contains("disk full"));

    // abort saves a termination as pending, for a live negotiation only
    store.1.set(false);
    let out = p.abort(&agree, "consumer unreachable").unwrap().unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    assert_eq!(out.to, NegotiationState::Terminated);
    assert_eq!(out.body["reason"][0], "consumer unreachable");
    assert_eq!(p.pending().unwrap(), vec![out.clone()]);
    // the consumer cannot move on while the provider is ending it
    let err = p
        .verify(&ack.provider_pid, &verification(&ack.provider_pid), "c")
        .unwrap_err();
    assert!(reason(&err).contains("ending"), "{err:?}");
    p.delivered(&out).unwrap();
    assert_eq!(p.abort(&out, "again").unwrap(), None);
    let unknown = Outgoing {
        provider_pid: "nope".to_owned(),
        ..out
    };
    assert!(matches!(p.abort(&unknown, "x"), Err(Failure::NotFound(_))));
}

#[test]
fn a_lost_write_is_read_and_tried_again() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    let out = out.unwrap();
    assert_eq!(store.0.borrow()[&pid].version, 0);
    // two other writes come first: the third try succeeds
    store.2.set(2);
    assert_eq!(p.delivered(&out).unwrap(), None);
    let n = store.0.borrow()[&pid].clone();
    assert_eq!((n.state, n.version), (NegotiationState::Agreed, 3));
    // other writes keep coming first: give up, change nothing
    store.2.set(3);
    let err = p.verify(&pid, &verification(&pid), "c").unwrap_err();
    assert!(reason(&err).contains("concurrently"), "{err:?}");
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Agreed);
}

/// The consumer verifies before its acknowledgement of the agreement has
/// been recorded: the verification proves the agreement arrived.
#[test]
fn a_reply_proves_the_pending_message_arrived() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (ack, agree) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    let agree = agree.unwrap();
    assert_eq!(agree.to, NegotiationState::Agreed);
    assert_eq!(
        agree.status_url,
        "https://consumer.example/cb/negotiations/c1"
    );
    assert_eq!(p.pending().unwrap(), vec![agree.clone()]);

    let (ack, finalize) = p.verify(&pid, &verification(&pid), "c").unwrap();
    assert_eq!(ack.state, NegotiationState::Verified);
    let n = store.0.borrow()[&pid].clone();
    assert_eq!(
        n.agreement.unwrap().id,
        agree.body["agreement"]["@id"].as_str().unwrap()
    );
    assert_eq!(n.pending, finalize.clone());
    // the late acknowledgement, or a late failure, changes nothing
    assert_eq!(p.delivered(&agree).unwrap(), None);
    assert_eq!(p.abort(&agree, "timed out").unwrap(), None);
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Verified);
    p.delivered(&finalize.unwrap()).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Finalized);
}

/// The same with the provider's offer and the consumer's acceptance; a
/// message that fits neither state is refused and changes nothing.
#[test]
fn proof_of_delivery_for_an_offer() {
    let store = Store::default();
    let script = Script(BTreeMap::from([(
        NegotiationState::Requested,
        Decision::Offer,
    )]));
    let p = service(&store, script);
    let (ack, out) = p.request(None, request(None), "c", &none()).unwrap();
    let pid = ack.provider_pid;
    assert_eq!(out.unwrap().to, NegotiationState::Offered);
    let err = p.verify(&pid, &verification(&pid), "c").unwrap_err();
    assert!(reason(&err).contains("not allowed"), "{err:?}");
    let before = store.0.borrow()[&pid].clone();
    assert_eq!(before.state, NegotiationState::Requested);
    let (ack, out) = p
        .event(&pid, &event(&pid, NegotiationEvent::Accepted), "c", &none())
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Accepted);
    assert_eq!(out, None);
    assert!(p.pending().unwrap().is_empty());
}

/// ADR 0009: the provider agrees only to the catalog's terms, and only
/// while their constraints hold; otherwise it terminates.
#[test]
fn agrees_only_to_catalog_terms_that_hold_now() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);

    let mut expired = request(None);
    expired.offer.target = Some("d2".to_owned());
    expired.offer.permission[0].constraint = vec![
        serde_json::from_value(serde_json::json!(
            {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2026-01-01"}
        ))
        .unwrap(),
    ];
    expired.consumer_pid = "c-expired".to_owned();
    let (_, out) = p.request(None, expired, "c", &none()).unwrap();
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    let why = out.body["reason"][0].as_str().unwrap();
    assert!(why.contains("not satisfied"), "{why}");

    // the catalog offer's id, with its constraint left out
    let mut altered = request(None);
    altered.offer.target = Some("d2".to_owned());
    altered.consumer_pid = "c-altered".to_owned();
    let (_, out) = p.request(None, altered, "c", &none()).unwrap();
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    let why = out.body["reason"][0].as_str().unwrap();
    assert!(why.contains("differ"), "{why}");

    // the dataset left the catalog while the consumer considered our offer
    (&store)
        .insert(&Negotiation {
            provider_pid: "p9".to_owned(),
            consumer_pid: "c1".to_owned(),
            state: NegotiationState::Offered,
            callback_address: "https://consumer.example/cb".to_owned(),
            counter_party: "c".to_owned(),
            offer: offer(Some("d9")),
            agreement: None,
            pending: None,
            version: 0,
        })
        .unwrap();
    let (ack, out) = p
        .event("p9", &event("p9", NegotiationEvent::Accepted), "c", &none())
        .unwrap();
    assert_eq!(ack.state, NegotiationState::Accepted);
    let out = out.unwrap();
    assert_eq!(out.transition, Transition::Terminate);
    let why = out.body["reason"][0].as_str().unwrap();
    assert!(why.contains("no longer in the catalog"), "{why}");
}

/// No credentials: enough for offers without credential constraints.
fn none() -> CallerCredentials {
    CallerCredentials::default()
}

/// ADR 0019: a credential constraint is evaluated with the credentials the
/// adapter verified for this request, on a request and on an acceptance.
#[test]
fn agrees_only_when_the_callers_credentials_meet_the_offer() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let mut msg = request(None);
    msg.offer = providers_only(Some("d3"));

    let (_, out) = p
        .request(None, msg.clone(), "c", &member_with_role("provider"))
        .unwrap();
    assert_eq!(out.unwrap().transition, Transition::Agree);

    // one consumer pid starts one negotiation: each one its own
    for (cpid, credentials) in [
        ("c-consumer", member_with_role("consumer")),
        ("c-none", none()),
    ] {
        let mut msg = msg.clone();
        msg.consumer_pid = cpid.to_owned();
        let (_, out) = p.request(None, msg, "c", &credentials).unwrap();
        let out = out.unwrap();
        assert_eq!(out.transition, Transition::Terminate);
        let why = out.body["reason"][0].as_str().unwrap();
        assert!(why.contains("not satisfied"), "{why}");
    }

    // the provider offers first; the consumer's acceptance brings the
    // credentials
    let p = service(
        &store,
        Script(BTreeMap::from([
            (NegotiationState::Requested, Decision::Offer),
            (NegotiationState::Accepted, Decision::Agree),
        ])),
    );
    for (role, expected) in [
        ("provider", Transition::Agree),
        ("consumer", Transition::Terminate),
    ] {
        // a consumer for each role: one consumer pid starts one negotiation
        let caller = format!("c-{role}");
        let (ack, out) = p.request(None, msg.clone(), &caller, &none()).unwrap();
        let pid = ack.provider_pid;
        p.delivered(&out.unwrap()).unwrap();
        assert_eq!(p.offer_under_negotiation(&pid, &caller).unwrap(), msg.offer);
        assert!(p.offer_under_negotiation(&pid, "other").is_err());
        let (_, out) = p
            .event(
                &pid,
                &event(&pid, NegotiationEvent::Accepted),
                &caller,
                &member_with_role(role),
            )
            .unwrap();
        assert_eq!(out.unwrap().transition, expected, "{role}");
    }
}

/// ADR 0019, amendment in 1.4: a contract request is checked against the
/// access policy of its dataset, as in Eclipse EDC's
/// `validateInitialOffer`. A caller the access policy does not hold for
/// gets the answer of an unknown offer, even when it meets the offer.
#[test]
fn a_contract_request_is_checked_against_the_access_policy() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let mut msg = request(None);
    msg.offer = offer(Some("d4"));

    // meets the offer (it has no constraint), not the access policy
    let mut unknown = request(None);
    unknown.offer = offer(Some("d9"));
    let unknown_answer = reason(&p.request(None, unknown, "c", &none()).unwrap_err());
    for credentials in [member_with_role("consumer"), none()] {
        let failure = p.request(None, msg.clone(), "c", &credentials).unwrap_err();
        assert!(matches!(failure, Failure::BadRequest(_)));
        assert_eq!(reason(&failure), unknown_answer);
    }
    assert!(store.0.borrow().is_empty(), "nothing is started");

    // meets both: the agreement
    let (_, out) = p
        .request(None, msg.clone(), "c", &member_with_role("provider"))
        .unwrap();
    assert_eq!(out.unwrap().transition, Transition::Agree);

    // a counter request is checked too (EDC checks every contract request)
    let p = service(
        &store,
        Script(BTreeMap::from([(
            NegotiationState::Requested,
            Decision::Offer,
        )])),
    );
    // another consumer pid: a repeat of the first request would get the
    // negotiation already started (TD-60)
    msg.consumer_pid = "c2".to_owned();
    let (ack_offered, _) = p
        .request(None, msg.clone(), "c", &member_with_role("provider"))
        .unwrap();
    let pid = ack_offered.provider_pid;
    let mut counter = msg.clone();
    counter.provider_pid = Some(pid.clone());
    let failure = p
        .request(Some(&pid), counter, "c", &member_with_role("consumer"))
        .unwrap_err();
    assert_eq!(reason(&failure), unknown_answer);

    // a dataset without an access policy: unchanged
    let p = service(&store, AgreeToCatalogOffers);
    let mut plain = request(None);
    plain.consumer_pid = "c3".to_owned();
    let (_, out) = p.request(None, plain, "c", &none()).unwrap();
    assert_eq!(out.unwrap().transition, Transition::Agree);
}

/// One presentation query brings the scopes of the offer and of its
/// dataset's access policy.
#[test]
fn a_contract_request_asks_for_the_access_policy_scopes_too() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let mut types = crate::policy::CredentialTypes::default();
    types.add("MembershipCredential", "scope:membership");
    assert_eq!(
        p.request_scopes(&offer(Some("d4")), &types).unwrap(),
        vec!["scope:membership".to_owned()]
    );
    // the offer's and the access policy's scope, each once
    assert_eq!(
        p.request_scopes(&providers_only(Some("d4")), &types)
            .unwrap(),
        vec!["scope:membership".to_owned()]
    );
    assert!(
        p.request_scopes(&offer(Some("d1")), &types)
            .unwrap()
            .is_empty()
    );
    assert!(p.request_scopes(&offer(None), &types).unwrap().is_empty());
    assert!(
        p.request_scopes(&offer(Some("d9")), &types)
            .unwrap()
            .is_empty()
    );
}

/// An offer that names its parties (Eclipse EDC sends `assigner`, the DSP
/// TCK both) must name this provider and the caller; the names are kept
/// with the negotiation, not dropped.
#[test]
fn the_parties_an_offer_names_are_checked_and_kept() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);

    let mut other_provider = request(None);
    other_provider.offer.assigner = Some("did:web:someone-else".to_owned());
    let failure = p
        .request(None, other_provider, "did:web:consumer", &none())
        .unwrap_err();
    assert!(reason(&failure).contains("the offer's assigner is not the provider"));

    let mut other_consumer = request(None);
    other_consumer.offer.assignee = Some("did:web:someone-else".to_owned());
    let failure = p
        .request(None, other_consumer, "did:web:consumer", &none())
        .unwrap_err();
    assert!(reason(&failure).contains("the offer's assignee is not the consumer"));
    assert!(store.0.borrow().is_empty());

    let mut named = request(None);
    named.offer.assigner = Some("did:web:provider".to_owned());
    named.offer.assignee = Some("did:web:consumer".to_owned());
    let (ack, out) = p.request(None, named, "did:web:consumer", &none()).unwrap();
    let stored = store.0.borrow()[&ack.provider_pid].offer.clone();
    assert_eq!(stored.assigner.as_deref(), Some("did:web:provider"));
    assert_eq!(stored.assignee.as_deref(), Some("did:web:consumer"));
    // the parties are not terms: the offer still equals the catalog's
    assert_eq!(out.unwrap().transition, Transition::Agree);

    // a counter-request is checked the same way
    let waiting = Store::default();
    let p = service(&waiting, Script(BTreeMap::new()));
    let (ack, _) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let pid = ack.provider_pid;
    let mut counter = request(Some(&pid));
    counter.offer.assigner = Some("did:web:someone-else".to_owned());
    let failure = p
        .request(Some(&pid), counter, "did:web:consumer", &none())
        .unwrap_err();
    assert!(reason(&failure).contains("the offer's assigner is not the provider"));
}

/// A negotiation of the default policy, taken to `VERIFIED` with the
/// finalization event on its way. Returns its id and that event.
fn verified_with_finalization_on_its_way(
    p: &Service<'_, AgreeToCatalogOffers>,
) -> (String, Outgoing) {
    let (ack, agreement) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let pid = ack.provider_pid;
    p.delivered(&agreement.unwrap()).unwrap();
    let (_, finalization) = p
        .verify(&pid, &verification(&pid), "did:web:consumer")
        .unwrap();
    (pid, finalization.unwrap())
}

/// ADR 0007, amended in 0.13 (TD-44): only a pending finalization event
/// can be taken as delivered by a transfer request.
#[test]
fn only_a_pending_finalization_event_can_be_proved_by_a_transfer_request() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);

    // REQUESTED, the agreement on its way: not a finalization
    let (ack, agreement) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let pid = ack.provider_pid;
    assert_eq!(finalization_proved(&store.0.borrow()[&pid]), None);

    // AGREED, nothing on its way
    p.delivered(&agreement.unwrap()).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Agreed);
    assert_eq!(finalization_proved(&store.0.borrow()[&pid]), None);

    // VERIFIED, the finalization event on its way
    let (_, finalization) = p
        .verify(&pid, &verification(&pid), "did:web:consumer")
        .unwrap();
    let verified = store.0.borrow()[&pid].clone();
    assert_eq!(verified.state, NegotiationState::Verified);
    let proved = finalization_proved(&verified).unwrap();
    assert_eq!(proved.state, NegotiationState::Finalized);
    assert_eq!(proved.pending, None);
    assert_eq!(proved.agreement, verified.agreement);

    // VERIFIED, but the provider is ending the negotiation
    let ending = p.abort(&finalization.unwrap(), "unreachable").unwrap();
    assert_eq!(ending.unwrap().transition, Transition::Terminate);
    assert_eq!(finalization_proved(&store.0.borrow()[&pid]), None);

    // TERMINATED and FINALIZED: nothing is pending
    p.terminate(&pid, &termination(&pid), "did:web:consumer")
        .unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Terminated);
    assert_eq!(finalization_proved(&store.0.borrow()[&pid]), None);
    assert_eq!(finalization_proved(&proved), None);
}

/// After a transfer request settled the finalization event, the outbox
/// still holds that event: its acknowledgement, or its failure, arrives
/// later and must change nothing.
#[test]
fn a_late_acknowledgement_or_failure_of_a_settled_finalization_changes_nothing() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (pid, finalization) = verified_with_finalization_on_its_way(&p);

    // what the transfer provider writes
    let verified = store.0.borrow()[&pid].clone();
    let mut settled = finalization_proved(&verified).unwrap();
    settled.version = verified.version + 1;
    assert!((&store).replace_if(&settled, verified.version).unwrap());

    assert_eq!(p.delivered(&finalization).unwrap(), None);
    assert_eq!(store.0.borrow()[&pid], settled);
    assert_eq!(p.abort(&finalization, "unreachable").unwrap(), None);
    assert_eq!(store.0.borrow()[&pid], settled);
    assert!(p.pending().unwrap().is_empty(), "nothing is sent again");
}

/// The outbox asks whether a finalization event it could not deliver
/// was settled without it (by a transfer request): then the event is
/// not counted as given up.
#[test]
fn a_finalization_event_is_settled_only_once_the_negotiation_is_finalized() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);

    // the agreement on its way is not a finalization event
    let (ack, agreement) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let pid = ack.provider_pid;
    let agreement = agreement.unwrap();
    assert!(!p.finalization_settled(&agreement).unwrap());
    p.delivered(&agreement).unwrap();

    // VERIFIED, the event on its way: not settled
    let (_, finalization) = p
        .verify(&pid, &verification(&pid), "did:web:consumer")
        .unwrap();
    let finalization = finalization.unwrap();
    assert!(!p.finalization_settled(&finalization).unwrap());

    // what the transfer provider writes for a transfer request
    let verified = store.0.borrow()[&pid].clone();
    let mut settled = finalization_proved(&verified).unwrap();
    settled.version = verified.version + 1;
    assert!((&store).replace_if(&settled, verified.version).unwrap());
    assert!(p.finalization_settled(&finalization).unwrap());
    // the earlier agreement message stays "not settled": only a
    // finalization event is asked about
    assert!(!p.finalization_settled(&agreement).unwrap());

    // an unknown negotiation
    let mut unknown = finalization.clone();
    unknown.provider_pid = "urn:uuid:unknown".to_owned();
    assert!(!p.finalization_settled(&unknown).unwrap());
}

/// An event the outbox gave up ends the negotiation: it is not settled.
#[test]
fn a_finalization_event_that_was_given_up_is_not_settled() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (pid, finalization) = verified_with_finalization_on_its_way(&p);
    let ending = p.abort(&finalization, "unreachable").unwrap().unwrap();
    assert!(!p.finalization_settled(&finalization).unwrap());
    p.delivered(&ending).unwrap();
    assert_eq!(store.0.borrow()[&pid].state, NegotiationState::Terminated);
    assert!(!p.finalization_settled(&finalization).unwrap());
}

/// The acknowledgement and the transfer request arrive at the same
/// moment: the acknowledgement is written first, the transfer provider's
/// write then loses the version check and writes nothing.
#[test]
fn an_acknowledgement_written_first_makes_the_settling_write_lose() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (pid, finalization) = verified_with_finalization_on_its_way(&p);
    let verified = store.0.borrow()[&pid].clone();
    let mut settled = finalization_proved(&verified).unwrap();
    settled.version = verified.version + 1;

    p.delivered(&finalization).unwrap();
    let finalized = store.0.borrow()[&pid].clone();
    assert_eq!(finalized.state, NegotiationState::Finalized);
    assert!(!(&store).replace_if(&settled, verified.version).unwrap());
    assert_eq!(store.0.borrow()[&pid], finalized);
}

/// A catalog that cannot be read now (its database is down).
struct Unreadable;

impl CatalogDatasets for Unreadable {
    fn all(&self) -> Result<Vec<Dataset>, StoreError> {
        Err(StoreError("database down".to_owned()))
    }
    fn get(&self, _: &str) -> Result<Option<Dataset>, StoreError> {
        Err(StoreError("database down".to_owned()))
    }
}

/// A catalog that cannot be read is a store failure (HTTP 500), not a
/// refusal of the offer (HTTP 400), which would end the consumer's
/// negotiation: the datasets are read from the database on each request
/// (ADR 0029, D4).
#[test]
fn a_catalog_that_cannot_be_read_is_a_store_failure() {
    let store = Store::default();
    let p = ProviderService::new(
        &store,
        AgreeToCatalogOffers,
        Unreadable,
        Env::default(),
        "did:web:provider".to_owned(),
    );
    let failure = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap_err();
    assert!(matches!(failure, Failure::Store(_)), "{failure:?}");
    assert!(store.0.borrow().is_empty(), "nothing was stored");
}

/// TD-60: a first request that the consumer's outbox sends again (our
/// answer was lost) gets the negotiation it already started, as it is
/// now, and starts nothing.
#[test]
fn a_repeated_first_request_gets_the_same_negotiation_and_starts_nothing() {
    let store = Store::default();
    let p = service(&store, AgreeToCatalogOffers);
    let (first, out) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let agreement = out.unwrap();

    // again while our agreement is on its way: the same negotiation, and
    // no second agreement
    let (again, out) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(out, None);

    // again once the agreement arrived: the answer shows the state now
    p.delivered(&agreement).unwrap();
    let (again, out) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    assert_eq!(again.provider_pid, first.provider_pid);
    assert_eq!(again.state, NegotiationState::Agreed);
    assert_eq!(out, None);
    assert_eq!(store.0.borrow().len(), 1, "one negotiation");
}

#[test]
fn a_repeated_first_request_with_other_terms_is_refused() {
    let store = Store::default();
    let p = service(&store, Script(BTreeMap::new()));
    let (first, _) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();

    let mut other_callback = request(None);
    other_callback.callback_address = Some("https://elsewhere.example/cb/".to_owned());
    let mut other_offer = request(None);
    other_offer.offer.permission[0].action = "read".to_owned();
    let mut no_callback = request(None);
    no_callback.callback_address = None;
    for msg in [other_callback, other_offer, no_callback] {
        let failure = p
            .request(None, msg, "did:web:consumer", &none())
            .unwrap_err();
        assert!(matches!(failure, Failure::BadRequest(_)), "{failure:?}");
        assert!(reason(&failure).contains("already used"), "{failure:?}");
        // the refusal does not name the existing negotiation
        assert!(
            !reason(&failure).contains(&first.provider_pid),
            "{failure:?}"
        );
    }
    assert_eq!(store.0.borrow().len(), 1);
}

/// Another consumer may use the same `consumerPid`: it gets a negotiation
/// of its own, and learns nothing of the first one.
#[test]
fn the_same_consumer_pid_of_another_consumer_starts_its_own_negotiation() {
    let store = Store::default();
    let p = service(&store, Script(BTreeMap::new()));
    let (first, _) = p
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    let (other, _) = p
        .request(None, request(None), "did:web:other", &none())
        .unwrap();
    assert_ne!(other.provider_pid, first.provider_pid);
    assert_eq!(store.0.borrow().len(), 2);
}

/// A store in which a copy of the request is saved between the lookup and
/// the insert, as when two copies arrive together (in two replicas).
struct Racing<'a>(&'a Store, Cell<bool>);

impl NegotiationStore for &Racing<'_> {
    fn get(&self, pid: &str) -> Result<Option<Negotiation>, StoreError> {
        (&self.0).get(pid)
    }
    fn negotiation_requested_by(
        &self,
        party: &str,
        cpid: &str,
    ) -> Result<Option<Negotiation>, StoreError> {
        // the first lookup comes before the other copy is saved
        if self.1.replace(true) {
            (&self.0).negotiation_requested_by(party, cpid)
        } else {
            Ok(None)
        }
    }
    fn insert(&self, n: &Negotiation) -> Result<bool, StoreError> {
        (&self.0).insert(n)
    }
    fn replace_if(&self, n: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        (&self.0).replace_if(n, expected)
    }
    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        (&self.0).pending()
    }
}

#[test]
fn two_copies_of_a_first_request_at_once_give_one_negotiation() {
    let store = Store::default();
    let (first, _) = service(&store, AgreeToCatalogOffers)
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();

    let racing = Racing(&store, Cell::new(false));
    let late = ProviderService::new(
        &racing,
        AgreeToCatalogOffers,
        Catalog,
        Env(Cell::new(1)),
        "did:web:provider".to_owned(),
    );
    let (again, out) = late
        .request(None, request(None), "did:web:consumer", &none())
        .unwrap();
    assert_eq!(again.provider_pid, first.provider_pid);
    assert_eq!(out, None, "the late copy sends no agreement");
    assert_eq!(store.0.borrow().len(), 1);
}
