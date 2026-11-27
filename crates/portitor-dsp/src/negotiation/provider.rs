// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider side of contract negotiation (domain). Pure logic: the
//! service reads and writes negotiations through the [`NegotiationStore`]
//! port, asks the [`Decider`] port what to do next, and returns the message
//! to send to the consumer as an [`Outgoing`] value. The HTTP adapter sends
//! it and reports the consumer's acknowledgement back with
//! [`ProviderService::delivered`]; only then does the state change, as DSP
//! requires ("a state changes upon receipt and acknowledgement").
//!
//! Before a message is sent it is saved with the negotiation, as
//! `pending`, in the same write as the state change that caused it (an
//! outbox). This has two uses (ADR 0007):
//!
//! - The consumer may send its next message before its acknowledgement of
//!   ours has been recorded. A consumer message that is valid only in the
//!   state our pending message leads to proves that the consumer received
//!   it, so both are applied together, without waiting.
//! - After a restart, pending messages are sent again.
//!
//! Every write is a compare-and-update on the negotiation's `version`.
//!
//! These steps are the shared core in [`crate::process`], which the other
//! three services use too; this file holds what only the provider's
//! negotiation does.

use crate::catalog::{CatalogDatasets, Requester};
use crate::policy::{CallerCredentials, CredentialTypes, credential_scopes};
use crate::process::{self, Lookup, Message, Record, Records, Refused, Write};
pub use crate::process::{Environment, StoreError};
use crate::{Context, Offer};

use super::{
    Agreement, AgreementType, ContractAgreementMessage, ContractAgreementMessageType,
    ContractAgreementVerificationMessage, ContractNegotiation, ContractNegotiationError,
    ContractNegotiationEventMessage, ContractNegotiationEventMessageType,
    ContractNegotiationTerminationMessage, ContractNegotiationTerminationMessageType,
    ContractOfferMessage, ContractOfferMessageType, ContractRequestMessage, NegotiationEvent,
    NegotiationState, Role, Transition, TransitionError, next_state,
};

/// One negotiation as the provider keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiation {
    /// The provider's process id (chosen here).
    pub provider_pid: String,
    /// The consumer's process id.
    pub consumer_pid: String,
    /// The current state.
    pub state: NegotiationState,
    /// Where messages to the consumer go.
    pub callback_address: String,
    /// The consumer's participant id, from its credentials.
    pub counter_party: String,
    /// The latest offer (the consumer's request or the provider's offer).
    pub offer: Offer,
    /// The agreement, once sent.
    pub agreement: Option<Agreement>,
    /// Our message on its way to the consumer, saved before it is sent.
    pub pending: Option<Outgoing>,
    /// Changes on every write; writes compare it (optimistic locking).
    pub version: u64,
}

impl Negotiation {
    /// The dataset under negotiation.
    #[must_use]
    pub fn dataset_id(&self) -> &str {
        self.offer.target.as_deref().unwrap_or_default()
    }
}

impl Record for Negotiation {
    type State = NegotiationState;
    type Outgoing = Outgoing;
    const PARTY: Role = Role::Provider;

    fn parts(&self) -> (NegotiationState, Option<&Outgoing>, u64) {
        (self.state, self.pending.as_ref(), self.version)
    }
    fn parts_mut(&mut self) -> (&mut NegotiationState, &mut Option<Outgoing>, &mut u64) {
        (&mut self.state, &mut self.pending, &mut self.version)
    }
    /// The shared step, and the provider keeps what it sent: the
    /// agreement, or its offer.
    fn apply_sent(&mut self, sent: &Outgoing) -> Result<(), TransitionError> {
        process::apply_sent(self, sent)?;
        if sent.transition == Transition::Agree {
            self.agreement = serde_json::from_value::<ContractAgreementMessage>(sent.body.clone())
                .ok()
                .map(|m| m.agreement);
        }
        if sent.transition == Transition::Offer
            && let Ok(m) = serde_json::from_value::<ContractOfferMessage>(sent.body.clone())
        {
            self.offer = m.offer;
        }
        Ok(())
    }
}

impl Message<Transition> for Outgoing {
    fn transition_caused(&self) -> Transition {
        self.transition
    }
}

/// Port: where the provider keeps its negotiations.
pub trait NegotiationStore {
    /// One negotiation by provider process id.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn get(&self, provider_pid: &str) -> Result<Option<Negotiation>, StoreError>;

    /// The negotiation that the consumer `counter_party` started with its
    /// process id `consumer_pid`, if there is one: how a repeated first
    /// `ContractRequestMessage` is recognised (TD-60). Another consumer's
    /// negotiation with the same `consumerPid` is never returned. If a
    /// store written before 1.4 holds several, the oldest.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn negotiation_requested_by(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Negotiation>, StoreError>;

    /// Adds a new negotiation, unless its consumer (`counter_party`)
    /// already has one with the same `consumer_pid`: then nothing is
    /// written and the answer is `false`. Both happen in one step, so two
    /// copies of one request that arrive together, in one process or in
    /// two that share the database, cannot both be added.
    ///
    /// # Errors
    ///
    /// The store cannot be written, or the provider's id is taken.
    fn insert(&self, negotiation: &Negotiation) -> Result<bool, StoreError>;

    /// Replaces a negotiation only if its stored `version` is still
    /// `expected`, atomically. Returns `false` when another write came
    /// first.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, StoreError>;

    /// Every negotiation with a pending message (to send again after a
    /// restart).
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn pending(&self) -> Result<Vec<Negotiation>, StoreError>;
}

/// What the provider does next, decided after a consumer's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Send the agreement.
    Agree,
    /// Send (again) the offer under negotiation.
    Offer,
    /// Send the finalization event.
    Finalize,
    /// End the negotiation.
    Terminate,
    /// Do nothing now.
    Wait,
}

/// Port: the provider's policy for a negotiation in a given state
/// (`REQUESTED`, `ACCEPTED` or `VERIFIED`).
pub trait Decider {
    /// The next step for `negotiation`.
    fn decide(&self, negotiation: &Negotiation) -> Decision;
}

/// The default policy: agree to any offer that is in the catalog, and
/// finalize every verified agreement. The service itself refuses to agree
/// when the offer's terms differ from the catalog or its permissions do
/// not hold (ADR 0009, ADR 0019), whatever the decider says.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgreeToCatalogOffers;

impl Decider for AgreeToCatalogOffers {
    fn decide(&self, negotiation: &Negotiation) -> Decision {
        match negotiation.state {
            NegotiationState::Requested | NegotiationState::Accepted => Decision::Agree,
            NegotiationState::Verified => Decision::Finalize,
            _ => Decision::Wait,
        }
    }
}

/// A message for the consumer, to be sent by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The negotiation.
    pub provider_pid: String,
    /// Full URL: the consumer's callback address plus the binding path.
    pub url: String,
    /// The message.
    pub body: serde_json::Value,
    /// The transition that happens once the consumer acknowledges it.
    pub transition: Transition,
    /// The state after that transition.
    pub to: NegotiationState,
    /// Where the consumer reports its state for this negotiation (`GET`),
    /// to check whether a message it refused had arrived before.
    pub status_url: String,
    /// The counter-party's participant id (its DID): who the message is
    /// for, and the audience of the token it is sent with (ADR 0016).
    pub counter_party: String,
}

/// Why a consumer's message was refused, with the HTTPS status it maps to.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// HTTP 400: invalid message or state transition.
    BadRequest(ContractNegotiationError),
    /// HTTP 404: unknown negotiation, or the caller is not its consumer.
    NotFound(ContractNegotiationError),
    /// HTTP 500: the store failed.
    Store(StoreError),
}

/// The provider side of contract negotiation.
#[derive(Debug, Clone)]
pub struct ProviderService<S, D, C, E> {
    store: S,
    decider: D,
    catalog: C,
    env: E,
    participant_id: String,
}

/// Result of handling a consumer's message: the acknowledgement for the
/// HTTP response and, possibly, the next message to send.
pub type Handled = Result<(ContractNegotiation, Option<Outgoing>), Failure>;

impl<S: NegotiationStore, D: Decider, C: CatalogDatasets, E: Environment>
    ProviderService<S, D, C, E>
{
    /// Builds the service for provider `participant_id`.
    pub fn new(store: S, decider: D, catalog: C, env: E, participant_id: String) -> Self {
        Self {
            store,
            decider,
            catalog,
            env,
            participant_id,
        }
    }

    /// `GET /negotiations/:providerPid`.
    ///
    /// # Errors
    ///
    /// Unknown negotiation, another consumer's negotiation, or store failure.
    pub fn get(&self, provider_pid: &str, caller: &str) -> Result<ContractNegotiation, Failure> {
        let negotiation = self.load(provider_pid, "", caller)?;
        Ok(ack(&negotiation))
    }

    /// The offer under negotiation `provider_pid`, for its consumer
    /// `caller`: the adapter derives from it the credentials to ask for
    /// before an event that may lead to an agreement (ADR 0019).
    ///
    /// # Errors
    ///
    /// Unknown negotiation, another consumer's negotiation, or store failure.
    pub fn offer_under_negotiation(
        &self,
        provider_pid: &str,
        caller: &str,
    ) -> Result<Offer, Failure> {
        Ok(self.load(provider_pid, "", caller)?.offer)
    }

    /// The DCP scopes to ask the caller's credential service for before a
    /// contract request with `offer` (ADR 0019): those of the offer's
    /// credential constraints, then those of the access policy of the
    /// dataset it names (amendment in 1.4), each once. One presentation
    /// query brings both.
    ///
    /// # Errors
    ///
    /// The catalog cannot be read.
    pub fn request_scopes(
        &self,
        offer: &Offer,
        types: &CredentialTypes,
    ) -> Result<Vec<String>, Failure> {
        let mut scopes = credential_scopes(offer, types);
        let Some(target) = offer.target.as_deref() else {
            return Ok(scopes);
        };
        let listed = self
            .catalog
            .get_with_access_policy(target)
            .map_err(Failure::Store)?;
        if let Some(policy) = listed.and_then(|listed| listed.access_policy) {
            for scope in credential_scopes(&policy, types) {
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
        Ok(scopes)
    }

    /// `POST /negotiations/request` (no `provider_pid`) and
    /// `POST /negotiations/:providerPid/request` (counter-offer).
    ///
    /// `credentials` are the caller's verified credentials: the access
    /// policy of the offer's dataset must hold for them (ADR 0019,
    /// amendment in 1.4), and the offer's permissions are evaluated with
    /// them if the provider agrees now (ADR 0019).
    ///
    /// # Errors
    ///
    /// Unknown offer or dataset, missing callback, invalid transition, or
    /// store failure.
    pub fn request(
        &self,
        provider_pid: Option<&str>,
        msg: ContractRequestMessage,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Handled {
        let consumer_pid = msg.consumer_pid.clone();
        let Some(pid) = provider_pid else {
            return self.start(msg, caller, credentials);
        };
        if msg.provider_pid.as_deref().is_some_and(|p| p != pid) {
            return Err(bad(
                pid,
                &consumer_pid,
                "providerPid does not match the path",
            ));
        }
        self.handle(pid, &consumer_pid, caller, credentials, |negotiation| {
            // the request must name the consumer pid this negotiation was
            // started with, like every other message on it (TD-36)
            check_ids(negotiation, pid, &consumer_pid)?;
            self.check_offer(&msg.offer, pid, &consumer_pid, caller, credentials)?;
            receive(negotiation, Transition::Request)?;
            negotiation.offer = msg.offer.clone();
            Ok(())
        })
    }

    /// A first request (no `providerPid`): starts a new negotiation.
    ///
    /// A first request that the same caller sends again with the same
    /// `consumerPid` starts nothing: the answer is the negotiation it
    /// already has, as it is now (TD-60). This happens when our answer to
    /// the first one was lost and the consumer's outbox sent it again.
    /// DSP 2025-1 has no such rule for contract requests (it has one for
    /// transfer requests); Eclipse EDC does the same
    /// (`findRequestedBy`: consumer pid and counter-party). The lookup
    /// comes before the checks: the first copy passed them, and a repeat
    /// must carry the same terms ([`answer_again`]).
    fn start(
        &self,
        msg: ContractRequestMessage,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Handled {
        if let Some(existing) = self.negotiation_requested_by(caller, &msg.consumer_pid)? {
            return answer_again(&existing, &msg.offer, msg.callback_address.as_deref());
        }
        let consumer_pid = msg.consumer_pid;
        let Some(callback_address) = msg.callback_address.filter(|c| !c.is_empty()) else {
            return Err(bad("", &consumer_pid, "callbackAddress is required"));
        };
        self.check_offer(&msg.offer, "", &consumer_pid, caller, credentials)?;
        let state = next_state(None, Transition::Request, Role::Consumer)
            .map_err(|e| bad("", &consumer_pid, &e.to_string()))?;
        let mut negotiation = Negotiation {
            provider_pid: self.env.new_id(),
            consumer_pid,
            state,
            callback_address,
            counter_party: caller.to_owned(),
            offer: msg.offer,
            agreement: None,
            pending: None,
            version: 0,
        };
        negotiation.pending = self.next(&negotiation, credentials)?;
        let inserted = self.store.insert(&negotiation).map_err(Failure::Store)?;
        if !inserted {
            // a copy of this request was saved while this one was being
            // checked: answer with the negotiation that was saved
            return match self.negotiation_requested_by(caller, &negotiation.consumer_pid)? {
                Some(existing) => answer_again(
                    &existing,
                    &negotiation.offer,
                    Some(negotiation.callback_address.as_str()),
                ),
                None => Err(bad(
                    "",
                    &negotiation.consumer_pid,
                    "the request was sent twice; retry",
                )),
            };
        }
        Ok((ack(&negotiation), negotiation.pending))
    }

    /// The negotiation `caller` already started with `consumer_pid`, if
    /// any. Only the caller's own negotiations are searched, so another
    /// participant that uses the same `consumerPid` finds nothing.
    fn negotiation_requested_by(
        &self,
        caller: &str,
        consumer_pid: &str,
    ) -> Result<Option<Negotiation>, Failure> {
        self.store
            .negotiation_requested_by(caller, consumer_pid)
            .map_err(Failure::Store)
    }

    /// `POST /negotiations/:providerPid/events` (`ACCEPTED` only).
    /// `credentials` are the caller's verified credentials, as for
    /// [`ProviderService::request`].
    ///
    /// # Errors
    ///
    /// Unknown negotiation, invalid transition, or store failure.
    pub fn event(
        &self,
        provider_pid: &str,
        msg: &ContractNegotiationEventMessage,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Handled {
        self.handle(
            provider_pid,
            &msg.consumer_pid,
            caller,
            credentials,
            |negotiation| {
                check_ids(negotiation, &msg.provider_pid, &msg.consumer_pid)?;
                receive(negotiation, msg.transition())
            },
        )
    }

    /// `POST /negotiations/:providerPid/agreement/verification`.
    ///
    /// # Errors
    ///
    /// Unknown negotiation, invalid transition, or store failure.
    pub fn verify(
        &self,
        provider_pid: &str,
        msg: &ContractAgreementVerificationMessage,
        caller: &str,
    ) -> Handled {
        let none = CallerCredentials::default();
        self.handle(
            provider_pid,
            &msg.consumer_pid,
            caller,
            &none,
            |negotiation| {
                check_ids(negotiation, &msg.provider_pid, &msg.consumer_pid)?;
                receive(negotiation, Transition::Verify)
            },
        )
    }

    /// `POST /negotiations/:providerPid/termination`.
    ///
    /// # Errors
    ///
    /// Unknown negotiation, terminal state, or store failure.
    pub fn terminate(
        &self,
        provider_pid: &str,
        msg: &ContractNegotiationTerminationMessage,
        caller: &str,
    ) -> Handled {
        let none = CallerCredentials::default();
        self.handle(
            provider_pid,
            &msg.consumer_pid,
            caller,
            &none,
            |negotiation| {
                check_ids(negotiation, &msg.provider_pid, &msg.consumer_pid)?;
                receive(negotiation, Transition::Terminate)
            },
        )
    }

    /// Records that the consumer acknowledged `sent`, and returns what to
    /// send next, if anything. A message that is no longer pending (the
    /// consumer's next message already proved it arrived, or the
    /// negotiation moved on) changes nothing.
    ///
    /// # Errors
    ///
    /// Unknown negotiation, or the store failed.
    pub fn delivered(&self, sent: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        let none = CallerCredentials::default();
        let saved = process::update(self, &Lookup::own(&sent.provider_pid), |negotiation| {
            if !process::is_pending(negotiation, sent) {
                return Ok(Write::Skip);
            }
            negotiation
                .apply_sent(sent)
                .map_err(|e| transition_failure(negotiation, e))?;
            negotiation.pending = self.next(negotiation, &none)?;
            Ok(Write::Save)
        })?;
        Ok(saved.and_then(|negotiation| negotiation.pending))
    }

    /// `sent` could not be delivered: the provider ends the negotiation
    /// with a termination saying `reason`, and returns it to send. Changes
    /// nothing if `sent` is no longer pending (the consumer's next message
    /// proved it arrived) or the negotiation has ended.
    ///
    /// # Errors
    ///
    /// Unknown negotiation, or store failure.
    pub fn abort(&self, sent: &Outgoing, reason: &str) -> Result<Option<Outgoing>, Failure> {
        let saved = process::update(self, &Lookup::own(&sent.provider_pid), |negotiation| {
            if !process::can_abort(negotiation, sent) {
                return Ok(Write::Skip);
            }
            negotiation.pending = Some(Self::termination(negotiation, reason));
            Ok(Write::Save)
        })?;
        Ok(saved.and_then(|negotiation| negotiation.pending))
    }

    /// Whether `sent`, a finalization event, no longer has to arrive: the
    /// negotiation is `FINALIZED`, so `sent` is not pending any more.
    /// `false` for any other message, and for an unknown negotiation.
    ///
    /// The outbox asks this for an event it could not deliver. The
    /// negotiation can then be `FINALIZED` only because a transfer
    /// request from the consumer was taken as proof of delivery
    /// ([`finalization_proved`]; ADR 0007, amended in 0.13): such an
    /// event is not given up. [`Service::abort`] changes nothing then.
    ///
    /// # Errors
    ///
    /// Store failure.
    pub fn finalization_settled(&self, sent: &Outgoing) -> Result<bool, Failure> {
        if sent.transition != Transition::Finalize {
            return Ok(false);
        }
        let stored = self.store.get(&sent.provider_pid);
        let Some(negotiation) = stored.map_err(Failure::Store)? else {
            return Ok(false);
        };
        Ok(negotiation.state == NegotiationState::Finalized)
    }

    /// Negotiations with a message still to send, for example after a
    /// restart.
    ///
    /// # Errors
    ///
    /// Store failure.
    pub fn pending(&self) -> Result<Vec<Outgoing>, Failure> {
        let negotiations = self.store.pending().map_err(Failure::Store)?;
        Ok(process::pending_messages(negotiations))
    }

    /// Applies a consumer's message with `receive`, then decides the next
    /// message (with the caller's `credentials`) and saves it as pending
    /// with the new state. Returns the acknowledgement and that message.
    /// Only the negotiation of `caller` is found.
    fn handle(
        &self,
        pid: &str,
        cpid: &str,
        caller: &str,
        credentials: &CallerCredentials,
        receive: impl Fn(&mut Negotiation) -> Result<(), Failure>,
    ) -> Handled {
        let lookup = Lookup::from_caller(pid, cpid, caller);
        let saved = process::update(self, &lookup, |negotiation| {
            receive(negotiation)?;
            negotiation.pending = self.next(negotiation, credentials)?;
            Ok(Write::Save)
        })?;
        let negotiation = saved.ok_or_else(|| not_found(pid, cpid))?;
        Ok((ack(&negotiation), negotiation.pending))
    }

    /// The message to send next, if any. Fails only when the catalog
    /// cannot be read to check an agreement.
    fn next(
        &self,
        negotiation: &Negotiation,
        credentials: &CallerCredentials,
    ) -> Result<Option<Outgoing>, Failure> {
        if negotiation.state.is_terminal() {
            return Ok(None);
        }
        let outgoing = match self.decider.decide(negotiation) {
            Decision::Wait => None,
            Decision::Terminate => {
                Some(Self::termination(negotiation, "terminated by the provider"))
            }
            Decision::Offer => Some(Self::offer(negotiation)),
            // never agree to terms that are not the catalog's, or whose
            // constraints do not hold now for this caller (ADR 0009, 0019)
            Decision::Agree => Some(match self.agreeable(negotiation, credentials)? {
                Ok(()) => self.agreement(negotiation),
                Err(reason) => Self::termination(negotiation, &reason),
            }),
            Decision::Finalize => Some(Self::finalization(negotiation)),
        };
        Ok(outgoing)
    }

    fn offer(negotiation: &Negotiation) -> Outgoing {
        let msg = ContractOfferMessage {
            context: Context::default(),
            kind: ContractOfferMessageType,
            provider_pid: negotiation.provider_pid.clone(),
            consumer_pid: Some(negotiation.consumer_pid.clone()),
            offer: negotiation.offer.clone(),
            callback_address: None,
        };
        outgoing(negotiation, "offers", &msg, Transition::Offer)
    }

    fn agreement(&self, negotiation: &Negotiation) -> Outgoing {
        let msg = ContractAgreementMessage {
            context: Context::default(),
            kind: ContractAgreementMessageType,
            provider_pid: negotiation.provider_pid.clone(),
            consumer_pid: negotiation.consumer_pid.clone(),
            agreement: Agreement {
                id: self.env.new_id(),
                kind: AgreementType,
                target: negotiation.dataset_id().to_owned(),
                assigner: self.participant_id.clone(),
                assignee: negotiation.counter_party.clone(),
                timestamp: Some(self.env.now()),
                permission: negotiation.offer.permission.clone(),
                prohibition: negotiation.offer.prohibition.clone(),
                obligation: negotiation.offer.obligation.clone(),
                profile: negotiation.offer.profile.clone(),
            },
        };
        outgoing(negotiation, "agreement", &msg, Transition::Agree)
    }

    fn finalization(negotiation: &Negotiation) -> Outgoing {
        let msg = ContractNegotiationEventMessage {
            context: Context::default(),
            kind: ContractNegotiationEventMessageType,
            provider_pid: negotiation.provider_pid.clone(),
            consumer_pid: negotiation.consumer_pid.clone(),
            event_type: NegotiationEvent::Finalized,
        };
        outgoing(negotiation, "events", &msg, Transition::Finalize)
    }

    fn termination(negotiation: &Negotiation, reason: &str) -> Outgoing {
        let msg = ContractNegotiationTerminationMessage {
            context: Context::default(),
            kind: ContractNegotiationTerminationMessageType,
            provider_pid: negotiation.provider_pid.clone(),
            consumer_pid: negotiation.consumer_pid.clone(),
            code: None,
            reason: vec![serde_json::Value::String(reason.to_owned())],
        };
        outgoing(negotiation, "termination", &msg, Transition::Terminate)
    }

    /// The negotiated offer has the terms of the catalog offer with its id,
    /// and its permissions hold now for a caller with `credentials`. The
    /// outer `Err`: the catalog cannot be read; the inner one: why the
    /// offer cannot be agreed to.
    fn agreeable(
        &self,
        negotiation: &Negotiation,
        credentials: &CallerCredentials,
    ) -> Result<Result<(), String>, Failure> {
        let dataset = self
            .catalog
            .get_with_access_policy(negotiation.dataset_id())
            .map_err(Failure::Store)?
            .map(|listed| listed.dataset);
        let listed = dataset.and_then(|d| {
            d.has_policy
                .into_iter()
                .find(|o| o.id == negotiation.offer.id)
        });
        let Some(listed) = listed else {
            return Ok(Err("the offer is no longer in the catalog".to_owned()));
        };
        if !crate::policy::same_terms(&listed, &negotiation.offer) {
            return Ok(Err("the offer's terms differ from the catalog".to_owned()));
        }
        let holds =
            crate::policy::permissions_hold(&negotiation.offer, &self.env.now(), credentials)
                .map_err(|e| e.to_string());
        Ok(holds)
    }

    /// The offer must name a dataset of this provider and one of its
    /// offers. If it names its parties, they must be this provider and
    /// `caller`, the consumer that sent it. The dataset's access policy,
    /// if it has one, must hold for the caller's `credentials` (ADR 0019,
    /// amendment in 1.4, as Eclipse EDC's `validateInitialOffer`): a
    /// dataset the caller may not see gets the answer of an unknown offer,
    /// as a dataset request for it gets the answer of an unknown dataset.
    fn check_offer(
        &self,
        offer: &Offer,
        pid: &str,
        cpid: &str,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Result<(), Failure> {
        let Some(target) = offer.target.as_deref() else {
            return Err(bad(pid, cpid, "the offer has no target"));
        };
        if let Err(reason) = crate::policy::check_parties(offer, &self.participant_id, caller) {
            return Err(bad(pid, cpid, reason));
        }
        let listed = self
            .catalog
            .get_with_access_policy(target)
            .map_err(Failure::Store)?;
        let now = self.env.now();
        let requester = Requester {
            now: &now,
            credentials,
        };
        let known = match listed {
            None => false,
            Some(listed) => {
                listed.dataset.has_policy.iter().any(|o| o.id == offer.id)
                    && requester.may_see(listed.access_policy.as_ref())
            }
        };
        if known {
            Ok(())
        } else {
            Err(bad(pid, cpid, "unknown offer for this dataset"))
        }
    }

    fn load(&self, provider_pid: &str, cpid: &str, caller: &str) -> Result<Negotiation, Failure> {
        match self.store.get(provider_pid).map_err(Failure::Store)? {
            // another consumer's negotiation looks like a missing one (DSP:
            // "if the client is not authorized ... MUST return 404")
            Some(negotiation) if negotiation.counter_party == caller => Ok(negotiation),
            _ => Err(not_found(provider_pid, cpid)),
        }
    }
}

/// How the shared core ([`process::update`]) reads and writes
/// negotiations.
impl<S: NegotiationStore, D, C, E> Records for ProviderService<S, D, C, E> {
    type Record = Negotiation;
    type Failure = Failure;

    fn read(&self, lookup: &Lookup<'_>) -> Result<Negotiation, Failure> {
        let missing = || not_found(lookup.pid, lookup.named_pid);
        let Some(negotiation) = self.store.get(lookup.pid).map_err(Failure::Store)? else {
            return Err(missing());
        };
        // another consumer's negotiation looks like a missing one
        // (DSP: "if the client is not authorized ... MUST return 404")
        if let Some(caller) = lookup.caller
            && negotiation.counter_party != caller
        {
            return Err(missing());
        }
        Ok(negotiation)
    }

    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, Failure> {
        self.store
            .replace_if(negotiation, expected)
            .map_err(Failure::Store)
    }

    fn keeps_changing(&self, lookup: &Lookup<'_>) -> Failure {
        let reason = "the negotiation keeps changing concurrently; retry";
        bad(lookup.pid, lookup.named_pid, reason)
    }
}

/// The answer to a first `ContractRequestMessage`, with `offer` and
/// `callback_address`, that the consumer of `existing` sent again with
/// the same `consumerPid` (TD-60): the
/// negotiation as it is now, and nothing new to send. A message of ours
/// still pending is sent by the outbox, as for any negotiation.
///
/// A repeat is the first request again only if it names the same offer
/// and callback address. Otherwise it is refused (HTTP 400), as for
/// transfers (ADR 0013, amended in 0.12): the consumer would take the
/// answer for a negotiation it did not ask for. A counter-offer of ours
/// replaces the offer kept, so a repeat that comes after one is refused
/// too.
fn answer_again(existing: &Negotiation, offer: &Offer, callback_address: Option<&str>) -> Handled {
    let same_request =
        existing.offer == *offer && callback_address == Some(existing.callback_address.as_str());
    if !same_request {
        return Err(bad(
            "",
            &existing.consumer_pid,
            "consumerPid is already used by another contract request of this consumer",
        ));
    }
    Ok((ack(existing), None))
}

fn ack(negotiation: &Negotiation) -> ContractNegotiation {
    ContractNegotiation::new(
        &negotiation.provider_pid,
        &negotiation.consumer_pid,
        negotiation.state,
    )
}

fn outgoing<T: serde::Serialize>(
    negotiation: &Negotiation,
    path: &str,
    msg: &T,
    transition: Transition,
) -> Outgoing {
    let status_url = format!(
        "{}/negotiations/{}",
        negotiation.callback_address.trim_end_matches('/'),
        negotiation.consumer_pid
    );
    Outgoing {
        provider_pid: negotiation.provider_pid.clone(),
        url: format!("{status_url}/{path}"),
        body: serde_json::to_value(msg).unwrap_or_default(),
        transition,
        to: next_state(Some(negotiation.state), transition, Role::Provider)
            .unwrap_or(negotiation.state),
        status_url,
        counter_party: negotiation.counter_party.clone(),
    }
}

/// The negotiation as it is once its pending finalization event counts as
/// delivered: `FINALIZED`, nothing pending. `None` when no finalization
/// event is on its way (any other state, or another pending message).
///
/// The transfer provider uses it (ADR 0007, amended in 0.13): a transfer
/// request under this negotiation's agreement, from its consumer, that
/// arrives before the acknowledgement of our finalization event is
/// recorded is proof of delivery, as the consumer's next negotiation
/// message is in `receive` ([`process::receive`]). The same step applies
/// the message here: `Negotiation::apply_sent`.
#[must_use]
pub fn finalization_proved(negotiation: &Negotiation) -> Option<Negotiation> {
    let sent = negotiation.pending.as_ref()?;
    if sent.transition != Transition::Finalize {
        return None;
    }
    let mut proved = negotiation.clone();
    proved.apply_sent(sent).ok()?;
    (proved.state == NegotiationState::Finalized).then_some(proved)
}

/// Applies a consumer's message to `negotiation` with the shared core
/// ([`process::receive`]). If it does not fit the current state but fits
/// the state our pending message leads to, the consumer has received that
/// message (it could not have sent this one otherwise), so both are
/// applied.
fn receive(negotiation: &mut Negotiation, transition: Transition) -> Result<(), Failure> {
    match process::receive(negotiation, transition) {
        Ok(()) => Ok(()),
        Err(Refused::Ending) => Err(bad(
            &negotiation.provider_pid,
            &negotiation.consumer_pid,
            "the provider is ending this negotiation",
        )),
        Err(Refused::NotAllowed(error)) => Err(transition_failure(negotiation, error)),
    }
}

fn transition_failure(negotiation: &Negotiation, e: TransitionError) -> Failure {
    bad(
        &negotiation.provider_pid,
        &negotiation.consumer_pid,
        &e.to_string(),
    )
}

fn check_ids(
    negotiation: &Negotiation,
    provider_pid: &str,
    consumer_pid: &str,
) -> Result<(), Failure> {
    if negotiation.provider_pid == provider_pid && negotiation.consumer_pid == consumer_pid {
        Ok(())
    } else {
        Err(bad(provider_pid, consumer_pid, "process ids do not match"))
    }
}

fn bad(pid: &str, cpid: &str, reason: &str) -> Failure {
    Failure::BadRequest(ContractNegotiationError::new(pid, cpid, "400", reason))
}

fn not_found(pid: &str, cpid: &str) -> Failure {
    Failure::NotFound(ContractNegotiationError::new(
        pid,
        cpid,
        "404",
        "no such negotiation",
    ))
}

#[cfg(test)]
mod tests;
