// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The provider side of the transfer process (domain, ADR 0013). Pure
//! logic, like the negotiation provider: transfers are read and written
//! through the [`TransferStore`] port, agreements are found through the
//! [`Agreements`] port, and the data plane is reached through the
//! [`DataFlows`] port. A message for the consumer is returned as an
//! [`Outgoing`] value; the HTTP adapter sends it and reports the
//! acknowledgement with [`TransferService::delivered`].
//!
//! The same rules as for negotiations apply (ADR 0007): a message is saved
//! as `pending` before it is sent (outbox), the state changes once the
//! consumer acknowledges it, a consumer message that only fits the state
//! after our pending message proves that message arrived, and every write
//! compares the transfer's `version`. These steps are the shared core in
//! [`crate::process`]; this file holds what only the provider's transfer
//! does.
//!
//! The data plane is told after the transfer's write, from what was
//! written, once (ADR 0028, decision 6): the write records the step, and
//! the step runs right after it, before our message for that change is
//! sent. How, what happens when the data plane refuses, and how a
//! transfer is leased in the database while its step runs: the module
//! `flow_steps`.
//!
//! A push transfer (`HttpData-PUSH`, ADR 0027) carries the consumer's
//! `dataAddress` in its request; it is kept with the transfer until the
//! transfer ends, and handed to the data plane when the flow starts. The
//! data plane sends the data; [`TransferService::push_finished`] then
//! completes or terminates the transfer.

use crate::catalog::CatalogDatasets;
use crate::negotiation::provider::{Negotiation, finalization_proved};
use crate::negotiation::{Agreement, NegotiationState};
use crate::policy::CallerCredentials;
use crate::process::{self, Lookup, Message, Record, Records, Refused, WRITE_ATTEMPTS, Write};
pub use crate::process::{Environment, StoreError};

use super::{
    DataAddress, Role, TransferError, TransferProcess, TransferRequestMessage,
    TransferStartMessage, TransferState, Transition, TransitionError, check_push_address,
    is_push_format, next_state,
};

mod messages;
mod push;
use messages::{completion, start_message, suspension, termination};

/// One transfer as the provider keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// The provider's process id (chosen here). It is also the data flow
    /// id on the data plane.
    pub provider_pid: String,
    /// The consumer's process id.
    pub consumer_pid: String,
    /// The current state.
    pub state: TransferState,
    /// Where messages to the consumer go.
    pub callback_address: String,
    /// The consumer's participant id (the agreement's assignee).
    pub counter_party: String,
    /// The agreement the transfer is made under.
    pub agreement_id: String,
    /// The dataset (the agreement's target).
    pub dataset_id: String,
    /// The format the consumer asked for, for example `HttpData-PULL`.
    pub format: String,
    /// Where to send the data in a push transfer: the consumer's
    /// `dataAddress`, with its token (ADR 0027). `None` for a pull
    /// transfer, and once the transfer has ended. Never shown or logged.
    pub data_address: Option<DataAddress>,
    /// Our message on its way to the consumer, saved before it is sent.
    pub pending: Option<Outgoing>,
    /// The data plane step a write decided and that has not run yet
    /// (`flow_steps`). Our message waits until it has run.
    pub flow_step: Option<FlowStep>,
    /// The state the transfer had before the write that recorded
    /// `flow_step`: the connector reports the state change from it once
    /// the step has run, whichever process runs it. `None` for a new
    /// transfer, and while no step is recorded.
    pub state_before_step: Option<TransferState>,
    /// Who runs `flow_step` now, and until when (`flow_steps`): taken in
    /// the write that records the step, emptied in the write that records
    /// that it ran. `None` while no step is recorded.
    pub lease: Option<Lease>,
    /// Changes on every write; writes compare it (optimistic locking).
    pub version: u64,
}

/// A lease on a transfer, kept in the database with it (`flow_steps`):
/// while it holds, no other call changes the transfer, in this process or
/// in another one that shares the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The process that holds it ([`Environment::process_id`]).
    pub holder: String,
    /// When it ends, in seconds since 1970-01-01 UTC
    /// ([`Environment::now_seconds`]). From then on another call may take
    /// the step over.
    pub until: u64,
}

/// A step of the data flow, recorded with the transfer's write that
/// decided it and run after that write (`flow_steps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStep {
    /// Start the data flow of a new transfer ([`DataFlows::start`]). The
    /// start message is made once it has run: it carries the address.
    Start,
    /// The provider starts a suspended transfer again
    /// ([`DataFlows::restart`]): a new token, so the start message is
    /// made once it has run, as for [`FlowStep::Start`].
    Restart,
    /// The consumer started the transfer again: access again with the
    /// same token ([`DataFlows::resume`]).
    Resume,
    /// As [`FlowStep::Resume`], for a push the consumer started again
    /// with a new address: the transfer's `data_address` is handed over.
    ResumeWithAddress,
    /// Stop access for now ([`DataFlows::suspend`]).
    Suspend,
    /// End the data flow normally ([`DataFlows::complete`]).
    Complete,
    /// End the data flow early ([`DataFlows::terminate`]).
    Terminate,
}

impl FlowStep {
    /// A stable name, for storage.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Restart => "restart",
            Self::Resume => "resume",
            Self::ResumeWithAddress => "resume-with-address",
            Self::Suspend => "suspend",
            Self::Complete => "complete",
            Self::Terminate => "terminate",
        }
    }

    /// The step with this [`FlowStep::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let all = [
            Self::Start,
            Self::Restart,
            Self::Resume,
            Self::ResumeWithAddress,
            Self::Suspend,
            Self::Complete,
            Self::Terminate,
        ];
        all.into_iter().find(|step| step.name() == name)
    }
}

/// A message for the consumer, to be sent by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The transfer.
    pub provider_pid: String,
    /// Full URL: the consumer's callback address plus the binding path.
    pub url: String,
    /// The message.
    pub body: serde_json::Value,
    /// The transition that happens once the consumer acknowledges it.
    pub transition: Transition,
    /// The state after that transition.
    pub to: TransferState,
    /// Where the consumer reports its state for this transfer (`GET`), to
    /// check whether a message it refused had arrived before.
    pub status_url: String,
    /// The counter-party's participant id (its DID): who the message is
    /// for, and the audience of the token it is sent with (ADR 0016).
    pub counter_party: String,
}

impl Record for Transfer {
    type State = TransferState;
    type Outgoing = Outgoing;
    const PARTY: Role = Role::Provider;

    fn parts(&self) -> (TransferState, Option<&Outgoing>, u64) {
        (self.state, self.pending.as_ref(), self.version)
    }
    fn parts_mut(&mut self) -> (&mut TransferState, &mut Option<Outgoing>, &mut u64) {
        (&mut self.state, &mut self.pending, &mut self.version)
    }
}

impl Message<Transition> for Outgoing {
    fn transition_caused(&self) -> Transition {
        self.transition
    }
}

/// Port: where the provider keeps its transfers.
pub trait TransferStore {
    /// One transfer by provider process id.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn get(&self, provider_pid: &str) -> Result<Option<Transfer>, StoreError>;

    /// The transfer that the consumer `counter_party` asked for with its
    /// process id `consumer_pid`, if there is one: how a repeated
    /// `TransferRequestMessage` is recognised (DSP 2025-1, "Transfer
    /// Request Message": idempotent behavior based on `consumerPid`).
    /// Another consumer's transfer with the same `consumerPid` is never
    /// returned. If a store written before 0.12 holds several, the oldest.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn requested(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, StoreError>;

    /// Adds a new transfer, unless its consumer (`counter_party`) already
    /// has one with the same `consumer_pid`: then nothing is written and
    /// the answer is `false`. Both happen in one step, so two copies of
    /// one request that arrive together cannot both be added.
    ///
    /// # Errors
    ///
    /// The store cannot be written, or the provider's id is taken.
    fn insert(&self, transfer: &Transfer) -> Result<bool, StoreError>;

    /// Replaces a transfer only if its stored `version` is still
    /// `expected`, atomically. Returns `false` when another write came
    /// first.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, StoreError>;

    /// Every transfer with a pending message or a data plane step not
    /// run yet (`flow_step`): what a restart takes up again.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn pending(&self) -> Result<Vec<Transfer>, StoreError>;

    /// Every transfer with a data plane step not run yet (`flow_step`),
    /// leased or not: what the periodic sweep looks at (`flow_steps`).
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn recorded_steps(&self) -> Result<Vec<Transfer>, StoreError>;

    /// Up to `limit` transfers in provider pid order, those after `after`
    /// when it is given: one page for the management API.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, StoreError>;
}

/// Port: the provider's agreements.
pub trait Agreements {
    /// The agreement with this `@id`, only if its negotiation is
    /// FINALIZED.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn finalized(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError>;

    /// The provider's negotiation that made the agreement with this
    /// `@id`, in whatever state it is. Read only when
    /// [`Agreements::finalized`] found nothing, to see whether the
    /// finalization event is on its way (ADR 0007, amended in 0.13).
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn negotiation_of(&self, agreement_id: &str) -> Result<Option<Negotiation>, StoreError>;

    /// Replaces a negotiation only if its stored `version` is still
    /// `expected`, atomically, exactly as
    /// [`NegotiationStore::replace_if`](crate::negotiation::provider::NegotiationStore::replace_if)
    /// does. Returns `false` when another write came first.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn replace_negotiation_if(
        &self,
        negotiation: &Negotiation,
        expected: u64,
    ) -> Result<bool, StoreError>;
}

/// Port: the data plane, as the transfer process needs it. The connector
/// implements it with Data Plane Signaling (`portitor-dps`). Each call is
/// safe to repeat: a change the data plane already made counts as done.
/// The service calls it only after the transfer's write (`flow_steps`).
pub trait DataFlows {
    /// Whether the data plane can serve dataset `dataset_id` in `format`
    /// (for example `HttpData-PULL`): it supports the format and knows
    /// where the dataset's data is.
    ///
    /// # Errors
    ///
    /// Where the dataset's data is cannot be read now, with the reason.
    fn supports(&self, dataset_id: &str, format: &str) -> Result<bool, String>;

    /// Starts the data flow for `transfer`. For a pull transfer, returns
    /// where the consumer fetches the data, with its access token; for a
    /// push transfer `None` (the data plane sends the data to
    /// `transfer.data_address`, and DSP gives the start message no
    /// address).
    ///
    /// Safe to repeat too: the flow may exist already when the process
    /// stopped after the start and before its address was saved. Its
    /// token was never sent, so it gets a new one, as with
    /// [`DataFlows::restart`].
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn start(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String>;

    /// Stops access until [`DataFlows::resume`].
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn suspend(&self, provider_pid: &str) -> Result<(), String>;

    /// Gives access again after [`DataFlows::suspend`], with the same
    /// token. Used when the consumer starts the transfer again. In a push
    /// transfer the consumer may send a new `address` (a fresh token); it
    /// replaces the old one.
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn resume(&self, provider_pid: &str, address: Option<&DataAddress>) -> Result<(), String>;

    /// Gives access again after [`DataFlows::suspend`] with a new token,
    /// and returns the new address. Used when the provider starts the
    /// transfer again: its start message must carry a data address (DSP
    /// 2025-1, "Transfer Start Message"), and only the token's hash is
    /// kept, so the old token cannot be sent again (ADR 0014). A push
    /// transfer is resumed and sends its data again; `None`.
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn restart(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String>;

    /// Ends the data flow normally. Final.
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn complete(&self, provider_pid: &str) -> Result<(), String>;

    /// Ends the data flow early. Final.
    ///
    /// # Errors
    ///
    /// The data plane refused or failed, with the reason.
    fn terminate(&self, provider_pid: &str) -> Result<(), String>;
}

/// What the provider does next with a transfer. A step the state machine
/// does not allow in the transfer's state is not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDecision {
    /// Start the data flow (REQUESTED) or start it again (SUSPENDED), and
    /// send the start message with the data address.
    Start,
    /// Stop access for now and send the suspension (STARTED).
    Suspend,
    /// End the data flow normally and send the completion (STARTED).
    Complete,
    /// End the data flow and the transfer.
    Terminate,
    /// Do nothing now.
    Wait,
}

/// Port: the provider's policy. It is asked when a transfer is new, and
/// again each time the consumer acknowledges one of our messages.
pub trait TransferDecider {
    /// The next step for `transfer`.
    fn decide(&self, transfer: &Transfer) -> TransferDecision;
}

/// The default policy: every accepted request starts at once (the request
/// was already checked against the agreement); after that the provider
/// only answers the consumer.
#[derive(Debug, Clone, Copy, Default)]
pub struct StartAtOnce;

impl TransferDecider for StartAtOnce {
    fn decide(&self, transfer: &Transfer) -> TransferDecision {
        if transfer.state == TransferState::Requested {
            TransferDecision::Start
        } else {
            TransferDecision::Wait
        }
    }
}

/// Why a consumer's message was refused, with the HTTPS status it maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// HTTP 400: invalid message, agreement or state transition.
    BadRequest(TransferError),
    /// HTTP 404: unknown transfer, or the caller is not its consumer.
    NotFound(TransferError),
    /// HTTP 500: the store failed.
    Store(StoreError),
    /// HTTP 500: the data plane failed.
    DataPlane(String),
    /// HTTP 503: another call is changing the transfer now (it holds the
    /// transfer's lease, `flow_steps`); try again shortly. Nothing was
    /// written.
    Busy(TransferError),
}

/// The reason a transfer request is refused with when it names no
/// finalized agreement of the caller.
const NO_FINALIZED_AGREEMENT: &str = "no finalized agreement with this id for the caller";

impl Failure {
    /// Whether another call is changing the transfer now
    /// ([`Failure::Busy`]): the same call may succeed a moment later.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(self, Failure::Busy(_))
    }

    /// Whether a transfer request was refused because it names no
    /// finalized agreement of the caller (unknown id, another consumer's
    /// agreement, or a negotiation that is not finalized). The adapter
    /// logs this refusal (TD-44).
    #[must_use]
    pub fn is_no_finalized_agreement(&self) -> bool {
        match self {
            Failure::BadRequest(error) => error
                .reason
                .iter()
                .any(|reason| reason.as_str() == Some(NO_FINALIZED_AGREEMENT)),
            Failure::NotFound(_) | Failure::Store(_) | Failure::DataPlane(_) | Failure::Busy(_) => {
                false
            }
        }
    }
}

/// What [`TransferService::pending`] found to take up again after a
/// restart, or [`TransferService::run_steps_left_over`] in its sweep.
#[derive(Debug, Default)]
pub struct Pending {
    /// Our messages to send.
    pub messages: Vec<Outgoing>,
    /// The transfers whose data plane step left over ran now, by
    /// provider pid (the connector logs them).
    pub steps_run: Vec<String>,
    /// The transfers whose data plane step could not run or be recorded
    /// now, by provider pid, with the failure. Their step stays recorded:
    /// once its lease has ended it runs at the next call that changes the
    /// transfer, at the next sweep, or at the next restart.
    pub not_taken_up: Vec<(String, Failure)>,
}

/// The acknowledgement for the HTTP response and, possibly, the next
/// message to send.
pub type Handled = Result<(TransferProcess, Option<Outgoing>), Failure>;

/// The provider side of the transfer process.
#[derive(Debug, Clone)]
pub struct TransferService<S, A, F, D, C, E> {
    store: S,
    agreements: A,
    flows: F,
    decider: D,
    catalog: C,
    env: E,
}

impl<S, A, F, D, C, E> TransferService<S, A, F, D, C, E>
where
    S: TransferStore,
    A: Agreements,
    F: DataFlows,
    D: TransferDecider,
    C: CatalogDatasets,
    E: Environment,
{
    /// Builds the service.
    pub fn new(store: S, agreements: A, flows: F, decider: D, catalog: C, env: E) -> Self {
        Self {
            store,
            agreements,
            flows,
            decider,
            catalog,
            env,
        }
    }

    /// `GET /transfers/:providerPid`.
    ///
    /// # Errors
    ///
    /// Unknown transfer, another consumer's transfer, or store failure.
    pub fn get(&self, provider_pid: &str, caller: &str) -> Result<TransferProcess, Failure> {
        match self.store.get(provider_pid).map_err(Failure::Store)? {
            Some(transfer) if transfer.counter_party == caller => Ok(ack(&transfer)),
            _ => Err(not_found(provider_pid, "")),
        }
    }

    /// The finalized agreement `agreement_id`, if `caller` is its
    /// assignee: the adapter derives from it the credentials to ask for
    /// before [`TransferService::request`] (ADR 0019).
    ///
    /// An agreement whose finalization event is still on its way to
    /// `caller` counts too, and its negotiation is recorded as
    /// `FINALIZED` by this call: see
    /// [`TransferService::settle_finalization_on_its_way`].
    ///
    /// # Errors
    ///
    /// The store failed, or the negotiation kept changing.
    pub fn agreement_settling_finalization(
        &self,
        agreement_id: &str,
        caller: &str,
    ) -> Result<Option<Agreement>, Failure> {
        let finalized = self
            .agreements
            .finalized(agreement_id)
            .map_err(Failure::Store)?;
        match finalized {
            // another consumer's agreement looks like a missing one
            Some(agreement) => Ok(Some(agreement).filter(|a| a.assignee == caller)),
            None => self.settle_finalization_on_its_way(agreement_id, caller),
        }
    }

    /// Settles a finalization that is on its way (ADR 0007, amended in
    /// 0.13). The provider records `FINALIZED` only when the consumer
    /// acknowledged the finalization event; the consumer records it on
    /// receipt, a moment earlier, and may ask for a transfer at once
    /// (TD-44). So when `caller`, the consumer of the negotiation that
    /// made `agreement_id`, names that agreement while our finalization
    /// event is pending, the consumer acts on the agreement after
    /// verifying it, and that is taken as enough: the negotiation is
    /// written as `FINALIZED` with nothing pending, and the agreement is
    /// returned. It does not prove that the event itself arrived: the
    /// consumer knows the agreement's id from the agreement message.
    ///
    /// Returns `None`, and writes nothing, when there is no such
    /// agreement, when `caller` is not its consumer and its assignee
    /// (both checked before anything is written, ADR 0013), or when the
    /// negotiation is in any other state (for example `AGREED`, or
    /// `TERMINATED`).
    ///
    /// No lock and no wait: the write compares the negotiation's
    /// `version`. If another write came first (a second transfer request,
    /// or the acknowledgement itself), the negotiation is read again; it
    /// is then usually `FINALIZED` already.
    fn settle_finalization_on_its_way(
        &self,
        agreement_id: &str,
        caller: &str,
    ) -> Result<Option<Agreement>, Failure> {
        for _ in 0..WRITE_ATTEMPTS {
            let found = self
                .agreements
                .negotiation_of(agreement_id)
                .map_err(Failure::Store)?;
            let Some(negotiation) = found else {
                return Ok(None);
            };
            if negotiation.counter_party != caller {
                return Ok(None);
            }
            let agreement = negotiation
                .agreement
                .clone()
                .filter(|a| a.id == agreement_id && a.assignee == caller);
            // not the agreement's assignee: nothing is written
            let Some(agreement) = agreement else {
                return Ok(None);
            };
            if negotiation.state == NegotiationState::Finalized {
                // another write settled it since `finalized` was read
                return Ok(Some(agreement));
            }
            let Some(mut proved) = finalization_proved(&negotiation) else {
                return Ok(None);
            };
            let expected = negotiation.version;
            proved.version = expected.wrapping_add(1);
            if self
                .agreements
                .replace_negotiation_if(&proved, expected)
                .map_err(Failure::Store)?
            {
                return Ok(Some(agreement));
            }
        }
        Err(bad(
            "",
            "",
            "the negotiation keeps changing concurrently; retry",
        ))
    }

    /// `POST /transfers/request`: checks the request against the agreement
    /// (ADR 0013), whose permissions must hold now for a caller with
    /// `credentials`, its verified credentials (ADR 0019), saves the new
    /// transfer and, if the decider says so, starts it.
    ///
    /// A request that the same caller sends again with the same
    /// `consumerPid` starts nothing: the answer is the transfer it already
    /// has (DSP 2025-1, "Transfer Request Message": "Providers SHOULD
    /// implement idempotent behavior for Transfer Request Messages based
    /// on the value of `consumerPid`"; ADR 0013, amended in 0.12). The
    /// request is checked first, then looked up, the order Eclipse EDC
    /// uses (`TransferProcessProtocolServiceImpl.requestedAction`). So a
    /// repeated request gets 400, not the transfer it already has, once
    /// the agreement's permissions no longer hold or the dataset is no
    /// longer offered in the asked format.
    ///
    /// # Errors
    ///
    /// The request breaks a rule of ADR 0013, it uses the `consumerPid` of
    /// the caller's earlier request with other content, or the store
    /// failed.
    pub fn request(
        &self,
        msg: &TransferRequestMessage,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Handled {
        let cpid = msg.consumer_pid.as_str();
        let agreement = self.check_request(msg, caller, credentials)?;
        if let Some(existing) = self.requested_before(caller, cpid)? {
            return answer_again(&existing, msg);
        }
        let state = next_state(None, Transition::Request, Role::Consumer)
            .map_err(|e| bad("", cpid, &e.to_string()))?;
        let mut transfer = Transfer {
            provider_pid: self.env.new_id(),
            consumer_pid: msg.consumer_pid.clone(),
            state,
            callback_address: msg.callback_address.clone(),
            counter_party: caller.to_owned(),
            agreement_id: agreement.id,
            dataset_id: agreement.target,
            format: msg.format.clone(),
            // checked: only a push transfer has one
            data_address: msg.data_address.clone(),
            pending: None,
            flow_step: None,
            state_before_step: None,
            lease: None,
            version: 0,
        };
        self.decide_next(&mut transfer);
        self.save_new(&transfer, msg)
    }

    /// Saves the new `transfer` that the request `msg` asked for, then
    /// starts its data flow if the decider said so, and returns the
    /// answer. A transfer that is not saved starts nothing.
    fn save_new(&self, transfer: &Transfer, msg: &TransferRequestMessage) -> Handled {
        let Some(current) = self.insert_then_run_step(transfer)? else {
            // a copy of this request was saved while this one was being
            // checked: answer with the transfer that was saved
            let (caller, cpid) = (&transfer.counter_party, &transfer.consumer_pid);
            return match self.requested_before(caller, cpid)? {
                Some(existing) => answer_again(&existing, msg),
                None => Err(bad("", cpid, "the request was sent twice; retry")),
            };
        };
        Ok((ack(transfer), ready_to_send(&current)))
    }

    /// The transfer `caller` already asked for with `consumer_pid`, if
    /// any. Only the caller's own transfers are searched, so another
    /// participant that uses the same `consumerPid` finds nothing.
    fn requested_before(
        &self,
        caller: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, Failure> {
        self.store
            .requested(caller, consumer_pid)
            .map_err(Failure::Store)
    }

    /// The rules of ADR 0013 for a new request. Returns the agreement.
    fn check_request(
        &self,
        msg: &TransferRequestMessage,
        caller: &str,
        credentials: &CallerCredentials,
    ) -> Result<Agreement, Failure> {
        let cpid = msg.consumer_pid.as_str();
        if msg.callback_address.is_empty() {
            return Err(bad("", cpid, "callbackAddress is required"));
        }
        // DSP 2025-1, "Transfer Request Message": the dataAddress "MUST
        // only be provided if the format REQUIRES a push transfer"
        match (&msg.data_address, is_push_format(&msg.format)) {
            (Some(address), true) => {
                check_push_address(address).map_err(|reason| bad("", cpid, &reason))?;
            }
            (None, true) => {
                return Err(bad("", cpid, "a push transfer request has a dataAddress"));
            }
            (Some(_), false) => {
                return Err(bad(
                    "",
                    cpid,
                    "only a push transfer request has a dataAddress",
                ));
            }
            (None, false) => {}
        }
        // an unknown agreement and another consumer's agreement get the
        // same answer, so a caller cannot learn which agreement ids exist
        let agreement = self
            .agreement_settling_finalization(&msg.agreement_id, caller)?
            .ok_or_else(|| bad("", cpid, NO_FINALIZED_AGREEMENT))?;
        // checked again at each new transfer, with the credentials of this
        // request: a credential revoked since the agreement stops it
        crate::policy::permissions_hold(&agreement, &self.env.now(), credentials).map_err(|e| {
            bad(
                "",
                cpid,
                &format!("the agreement does not allow it now: {e}"),
            )
        })?;
        // the agreement is a contract: a dataset withdrawn from the
        // catalog since stays available to it (ADR 0020)
        let dataset = self
            .catalog
            .get_for_agreement(&agreement.target)
            .map_err(Failure::Store)?;
        let offered =
            dataset.is_some_and(|d| d.distribution.iter().any(|x| x.format == msg.format));
        if !offered {
            return Err(bad("", cpid, "the dataset is not offered in this format"));
        }
        let supported = self
            .flows
            .supports(&agreement.target, &msg.format)
            .map_err(Failure::DataPlane)?;
        if !supported {
            return Err(bad(
                "",
                cpid,
                "this connector cannot transfer this dataset in this format",
            ));
        }
        Ok(agreement)
    }

    /// `POST /transfers/:providerPid/start`: the consumer starts a
    /// suspended transfer again. A pull data flow resumes with the same
    /// token, so no new address is sent (ADR 0013). A push transfer sends
    /// its data again from the start, to the new `dataAddress` if the
    /// message has one (ADR 0027).
    ///
    /// # Errors
    ///
    /// Unknown transfer, invalid transition, a `dataAddress` on a pull
    /// transfer or an invalid one, data plane or store failure.
    pub fn start(&self, pid: &str, msg: &TransferStartMessage, caller: &str) -> Handled {
        let ids = (msg.provider_pid.as_str(), msg.consumer_pid.as_str());
        if let Some(address) = &msg.data_address {
            check_push_address(address).map_err(|reason| bad(pid, ids.1, &reason))?;
        }
        self.handle(
            pid,
            ids,
            caller,
            Transition::Start,
            msg.data_address.as_ref(),
        )
    }

    /// `POST /transfers/:providerPid/suspension`.
    ///
    /// # Errors
    ///
    /// Unknown transfer, invalid transition, data plane or store failure.
    pub fn suspend(&self, pid: &str, ids: (&str, &str), caller: &str) -> Handled {
        self.handle(pid, ids, caller, Transition::Suspend, None)
    }

    /// `POST /transfers/:providerPid/completion`.
    ///
    /// # Errors
    ///
    /// Unknown transfer, invalid transition, data plane or store failure.
    pub fn complete(&self, pid: &str, ids: (&str, &str), caller: &str) -> Handled {
        self.handle(pid, ids, caller, Transition::Complete, None)
    }

    /// `POST /transfers/:providerPid/termination`.
    ///
    /// # Errors
    ///
    /// Unknown transfer, terminal state, data plane or store failure.
    pub fn terminate(&self, pid: &str, ids: (&str, &str), caller: &str) -> Handled {
        self.handle(pid, ids, caller, Transition::Terminate, None)
    }

    /// Records that the consumer acknowledged `sent`, then asks the
    /// decider for the next step and returns the message to send for it,
    /// if any. A message that is no longer pending changes nothing.
    ///
    /// # Errors
    ///
    /// Unknown transfer, or the store failed.
    pub fn delivered(&self, sent: &Outgoing) -> Result<Option<Outgoing>, Failure> {
        let lookup = Lookup::own(&sent.provider_pid);
        let stepped = self.write_then_run_step(&lookup, IfRefused::EndTheTransfer, |transfer| {
            if !process::is_pending(transfer, sent) {
                return Ok(Write::Skip);
            }
            transfer
                .apply_sent(sent)
                .map_err(|e| transition_failure(transfer, e))?;
            if !transfer.state.is_terminal() {
                self.decide_next(transfer);
            }
            Ok(Write::Save)
        })?;
        match stepped {
            Some(stepped) => Ok(ready_to_send(&stepped.current)),
            None => Ok(None),
        }
    }

    /// `sent` could not be delivered: end the data flow and the transfer,
    /// and return the termination to send. Changes nothing if `sent` is no
    /// longer pending or the transfer has ended.
    ///
    /// # Errors
    ///
    /// Unknown transfer, data plane or store failure. When the data plane
    /// refuses, the transfer stays as it was (`flow_steps`).
    pub fn abort(&self, sent: &Outgoing, reason: &str) -> Result<Option<Outgoing>, Failure> {
        let lookup = Lookup::own(&sent.provider_pid);
        let stepped = self.write_then_run_step(&lookup, IfRefused::TakeBack, |transfer| {
            if !process::can_abort(transfer, sent) {
                return Ok(Write::Skip);
            }
            transfer.pending = Some(termination(transfer, reason));
            transfer.flow_step = Some(FlowStep::Terminate);
            Ok(Write::Save)
        })?;
        match stepped {
            Some(stepped) => Ok(ready_to_send(&stepped.current)),
            None => Ok(None),
        }
    }

    /// Transfers with a message still to send, for example after a
    /// restart. The data plane steps that were recorded but had not run
    /// when the process stopped run first (`flow_steps`); a message
    /// waits for its step.
    ///
    /// A step whose lease still holds is left out: a call of another
    /// process that shares the database may be running it, or the lease
    /// is the one of the process that stopped. Its message follows its
    /// step: sent by the call that holds the lease, or by the sweep once
    /// the lease has ended ([`TransferService::run_steps_left_over`]).
    ///
    /// A transfer whose step cannot run or be recorded now (the store
    /// failed) is left out, with its step still recorded, and listed in
    /// [`Pending::not_taken_up`]: one such transfer does not keep the
    /// others, or the connector, from starting.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    pub fn pending(&self) -> Result<Pending, Failure> {
        let transfers = self.store.pending().map_err(Failure::Store)?;
        Ok(self.take_up(transfers))
    }

    /// Applies a consumer's message: checks the ids and the transition,
    /// saves the new state with the data plane step it needs, then runs
    /// that step (`flow_steps`). `address`: the new push address a start
    /// message may carry.
    fn handle(
        &self,
        pid: &str,
        ids: (&str, &str),
        caller: &str,
        transition: Transition,
        address: Option<&DataAddress>,
    ) -> Handled {
        // the process ids the message itself names
        let (named_provider, named_consumer) = ids;
        if named_provider != pid {
            return Err(bad(
                pid,
                named_consumer,
                "providerPid does not match the path",
            ));
        }
        // a transfer that is not found is answered without a consumer pid
        let lookup = Lookup::from_caller(pid, "", caller);
        let stepped = self.write_then_run_step(&lookup, IfRefused::TakeBack, |transfer| {
            if transfer.consumer_pid != named_consumer {
                return Err(bad(pid, named_consumer, "process ids do not match"));
            }
            if let Some(address) = address {
                if !is_push_format(&transfer.format) {
                    let reason = "only a push transfer is started again with a dataAddress";
                    return Err(bad(pid, named_consumer, reason));
                }
                // the new token replaces the old one (ADR 0027)
                transfer.data_address = Some(address.clone());
            }
            receive(transfer, transition)?;
            if transfer.state.is_terminal() {
                // nothing more is sent on an ended transfer
                transfer.pending = None;
            }
            transfer.flow_step = consumer_step(transfer.state, address.is_some());
            Ok(Write::Save)
        })?;
        let Some(stepped) = stepped else {
            // the change above never skips; answered anyway
            return Err(not_found(pid, named_consumer));
        };
        Ok((ack(&stepped.written), ready_to_send(&stepped.current)))
    }

    /// Records on `transfer` the provider's next step, as the decider
    /// says: the message to send and the data plane step it needs. The
    /// step runs after the write (`flow_steps`). A start has no message
    /// yet: it is made once the data plane has given the address.
    fn decide_next(&self, transfer: &mut Transfer) {
        let transition = match self.decider.decide(transfer) {
            TransferDecision::Wait => return,
            TransferDecision::Start => Transition::Start,
            TransferDecision::Suspend => Transition::Suspend,
            TransferDecision::Complete => Transition::Complete,
            TransferDecision::Terminate => Transition::Terminate,
        };
        // a step the state machine does not allow here is not taken
        if next_state(Some(transfer.state), transition, Role::Provider).is_err() {
            return;
        }
        let (message, step) = match transition {
            Transition::Start if transfer.state == TransferState::Requested => {
                (None, FlowStep::Start)
            }
            Transition::Start => (None, FlowStep::Restart),
            Transition::Suspend => (Some(suspension(transfer)), FlowStep::Suspend),
            Transition::Complete => (Some(completion(transfer)), FlowStep::Complete),
            _ => (
                Some(termination(transfer, "terminated by the provider")),
                FlowStep::Terminate,
            ),
        };
        transfer.pending = message;
        transfer.flow_step = Some(step);
    }
}

/// The data plane step for a transfer that a consumer's message moved to
/// `state`; `new_address`: the message carried a new push address.
fn consumer_step(state: TransferState, new_address: bool) -> Option<FlowStep> {
    match state {
        TransferState::Started if new_address => Some(FlowStep::ResumeWithAddress),
        TransferState::Started => Some(FlowStep::Resume),
        TransferState::Suspended => Some(FlowStep::Suspend),
        TransferState::Completed => Some(FlowStep::Complete),
        TransferState::Terminated => Some(FlowStep::Terminate),
        // a consumer's message never leads to REQUESTED
        TransferState::Requested => None,
    }
}

/// The message of ours on `transfer` that may be sent now: none while a
/// data plane step is still to run, since the message follows the step
/// (`flow_steps`).
fn ready_to_send(transfer: &Transfer) -> Option<Outgoing> {
    if transfer.flow_step.is_some() {
        return None;
    }
    transfer.pending.clone()
}

/// How the shared core ([`process::update`]) reads and writes transfers.
impl<S, A, F, D, C, E> Records for TransferService<S, A, F, D, C, E>
where
    S: TransferStore,
{
    type Record = Transfer;
    type Failure = Failure;

    fn read(&self, lookup: &Lookup<'_>) -> Result<Transfer, Failure> {
        let missing = || not_found(lookup.pid, lookup.named_pid);
        let Some(transfer) = self.store.get(lookup.pid).map_err(Failure::Store)? else {
            return Err(missing());
        };
        // another consumer's transfer looks like a missing one
        // (DSP: "if the client is not authorized ... MUST return 404")
        if let Some(caller) = lookup.caller
            && transfer.counter_party != caller
        {
            return Err(missing());
        }
        Ok(transfer)
    }

    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, Failure> {
        self.store
            .replace_if(transfer, expected)
            .map_err(Failure::Store)
    }

    fn keeps_changing(&self, lookup: &Lookup<'_>) -> Failure {
        let reason = "the transfer keeps changing concurrently; retry";
        bad(lookup.pid, lookup.named_pid, reason)
    }

    fn before_write(&self, transfer: &mut Transfer) {
        if transfer.state.is_terminal() {
            // an ended transfer keeps no token of the consumer's
            transfer.data_address = None;
        }
    }
}

/// The answer to a `TransferRequestMessage` that the consumer of
/// `existing` sent again with the same `consumerPid`: the transfer as it
/// is now, and nothing to send. No new data flow and no new token: the
/// start message of the first request carries the only one.
///
/// DSP 2025-1 does not say what to answer when the second request is not
/// the first one again. Eclipse EDC answers with the existing transfer
/// whatever the message holds. Portitor refuses it (HTTP 400) when the
/// agreement, the format or the callback address differ, as it refuses
/// other mismatched ids: the consumer would otherwise take the answer for
/// a transfer it did not ask for.
fn answer_again(existing: &Transfer, msg: &TransferRequestMessage) -> Handled {
    let same_request = existing.agreement_id == msg.agreement_id
        && existing.format == msg.format
        && existing.callback_address == msg.callback_address
        && (existing.data_address.is_none() || existing.data_address == msg.data_address);
    if !same_request {
        return Err(bad(
            "",
            &msg.consumer_pid,
            "consumerPid is already used by another transfer request of this consumer",
        ));
    }
    Ok((ack(existing), None))
}

fn ack(transfer: &Transfer) -> TransferProcess {
    TransferProcess::new(
        &transfer.provider_pid,
        &transfer.consumer_pid,
        transfer.state,
    )
}

/// Applies a consumer's message to `transfer` with the shared core
/// ([`process::receive`]): proof of delivery included (ADR 0007).
fn receive(transfer: &mut Transfer, transition: Transition) -> Result<(), Failure> {
    match process::receive(transfer, transition) {
        Ok(()) => Ok(()),
        Err(Refused::Ending) => Err(bad(
            &transfer.provider_pid,
            &transfer.consumer_pid,
            "the provider is ending this transfer",
        )),
        Err(Refused::NotAllowed(error)) => Err(transition_failure(transfer, error)),
    }
}

fn transition_failure(transfer: &Transfer, e: TransitionError) -> Failure {
    bad(
        &transfer.provider_pid,
        &transfer.consumer_pid,
        &e.to_string(),
    )
}

fn bad(pid: &str, cpid: &str, reason: &str) -> Failure {
    Failure::BadRequest(TransferError::new(pid, cpid, "400", reason))
}

fn not_found(pid: &str, cpid: &str) -> Failure {
    Failure::NotFound(TransferError::new(pid, cpid, "404", "no such transfer"))
}

mod flow_steps;
mod own_steps;
use flow_steps::IfRefused;
pub use flow_steps::LEASE_SECONDS;
pub use own_steps::MAX_PAGE;

#[cfg(test)]
mod tests;
