// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! How the connector is wired: which adapter sits behind each port
//! (ADR 0004). [`Ports`] holds them all; [`Ports::new`] keeps every
//! store in one database (`ConnectorStore`) and [`Ports::use_database`]
//! swaps it for another. The wrapper [`Shared`] and the `Any...` names below are what
//! make "pick an adapter at start" possible.

use std::sync::Arc;
use std::time::Duration;

use portitor_dps::state::DataFlowState;
use portitor_dsp::guard::{AllowAll, MessageGuard};
use portitor_dsp::negotiation::consumer::{
    ConsumerDecider, ConsumerDecision, ConsumerNegotiation, ConsumerStore,
};
use portitor_dsp::negotiation::provider::{
    Decider, Decision, Negotiation, NegotiationStore, StoreError,
};
use portitor_dsp::negotiation::{Agreement, NegotiationState};
use portitor_dsp::policy::CredentialTypes;
use portitor_dsp::transfer::consumer::{
    AgreementProvider, ConsumerTransfer, ConsumerTransferDecider, ConsumerTransferDecision,
    ConsumerTransferStore, HeldAgreements,
};
use portitor_dsp::transfer::provider::{
    Agreements, DataFlows, Transfer, TransferDecider, TransferDecision, TransferStore,
};
use portitor_dsp::transfer::{DataAddress, TransferState};

use portitor_runtime::internal_addresses::InternalAddresses;
use portitor_runtime::leases::{LeaseStore, Leases};

use crate::auth::{CallerVerifier, Membership, NoTokens, Prover, RefuseAll};
use crate::flow_membership::{MembershipStore, StoredMembership};
use crate::http_client::Sender;
use crate::state_events::{self, Role};
use crate::store::ConnectorStore;
use crate::transfer::NoDataFlows;
use portitor_dataplane::{Flow, FlowStore, FlowStoreError};

/// How often and how fast to retry a message the consumer did not take.
#[derive(Debug, Clone, Copy)]
pub struct Retry {
    /// Tries in total, at least 1.
    pub attempts: u32,
    /// Wait before the second try; it doubles for each later try.
    pub first_delay: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: 4,
            first_delay: Duration::from_millis(500),
        }
    }
}

/// The adapters behind the negotiation endpoints.
#[derive(Clone)]
pub struct Ports {
    /// Where negotiations are kept.
    pub store: AnyStore,
    /// The provider's decisions.
    pub decider: AnyDecider,
    /// Sends messages to consumers, at the callback address each consumer
    /// chose: the binary gives it the block on internal addresses
    /// (`HttpSender::for_partners`, R-17).
    pub sender: Arc<dyn Sender>,
    /// Sends messages and catalog requests to providers, at the address
    /// our operator gave in the management API: the block on internal
    /// addresses does not apply (R-17). [`Ports::new`] uses `sender` for
    /// both; the binary sets its own.
    pub sender_to_providers: Arc<dyn Sender>,
    /// The block on internal addresses (`PORTITOR_BLOCK_INTERNAL_ADDRESSES`,
    /// R-17), for the clients the connector builds itself: the data
    /// plane's push to a consumer's endpoint. [`Ports::new`] leaves it
    /// off; the binary sets it from the settings.
    pub internal_addresses: InternalAddresses,
    /// Who sent a DSP request (ADR 0016). [`Ports::new`] starts with
    /// [`RefuseAll`]: no caller is let in until a verifier is set.
    pub verifier: Arc<dyn CallerVerifier>,
    /// The `Authorization` header of each DSP request we send (ADR 0016).
    /// [`Ports::new`] starts with [`NoTokens`]: nothing is sent until a
    /// prover is set.
    pub prover: Arc<dyn Prover>,
    /// The dataspace's rules for DSP messages (ADR 0005).
    pub guard: Arc<dyn MessageGuard + Send + Sync>,
    /// Retry policy for outgoing messages.
    pub retry: Retry,
    /// Where the consumer side keeps its negotiations.
    pub consumer_store: AnyConsumerStore,
    /// The consumer's decisions.
    pub consumer_decider: AnyConsumerDecider,
    /// The adapters for transfers as provider.
    pub transfers: TransferPorts,
    /// The adapters for transfers as consumer.
    pub consumer_transfers: ConsumerTransferPorts,
    /// Where the built-in data plane keeps its flows. With a store (and
    /// the `data-plane` feature) the connector runs its HTTP pull data
    /// plane and uses it for transfers; `None` keeps `transfers.flows`.
    pub flow_store: Option<AnyFlowStore>,
    /// Where the tokens of push transfers as consumer are kept (ADR 0027).
    pub push_receipts: crate::receiving::AnyPushReceipts,
    /// The adapters of the dataset harvest (ADR 0020): the store of the
    /// read copy, and optionally another source than the datasets file and
    /// a check after the DSP rules.
    pub harvest: crate::harvest::HarvestPorts,
    /// The connector's S3-compatible object storage (ADR 0027): the
    /// provider's `s3://` sources, and where a consumer's pushed data
    /// lands. [`Ports::new`] starts without; the binary sets it from the
    /// `AWS_*` settings ([`portitor_dataplane::object_storage::Settings`]).
    pub object_storage: Option<Arc<portitor_dataplane::object_storage::ObjectStorage>>,
    /// The credential types that policies may use (ADR 0019): which scopes
    /// to ask for, and grant, for an offer's credential constraints.
    /// [`crate::build`] sets it from [`crate::Config::credential_types`].
    pub(crate) credential_types: CredentialTypes,
    /// The leases of the work one process at a time does, held under
    /// this process's id (ADR 0029, decision 6): sending a process's
    /// pending message (TD-53), a push and an upload (TD-54), the
    /// scheduled harvest (TD-56). [`Ports::new`] keeps them in the
    /// connector's database; a test of two "replicas" gives each its own
    /// holder id.
    pub leases: Leases,
}

impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports")
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Ports {
    /// Every store in `store`, the connector's database (see
    /// [`Ports::use_database`]; tests pass an in-memory one,
    /// [`ConnectorStore::in_memory`]), no data plane, no caller let in
    /// and no tokens to send (set `verifier` and `prover`), no extra
    /// message rules, and the default deciders: as provider, agree to any
    /// offer from the catalog and start every accepted transfer request;
    /// as consumer, accept offers and verify agreements, and act on a
    /// transfer only when asked. With the `tck` feature the deciders follow
    /// the DSP TCK scripts instead.
    #[must_use]
    pub fn new(sender: Arc<dyn Sender>, store: &Arc<ConnectorStore>) -> Self {
        Self {
            store: Shared(Arc::clone(store) as _),
            transfers: TransferPorts {
                store: AnyTransferStore::new(Arc::clone(store) as _),
                agreements: Shared(Arc::clone(store) as _),
                flows: Shared(Arc::new(NoDataFlows)),
                decider: default_transfer_decider(),
            },
            flow_store: Some(Shared(Arc::clone(store) as _)),
            push_receipts: Shared(Arc::clone(store) as _),
            decider: default_decider(),
            sender_to_providers: Arc::clone(&sender),
            sender,
            internal_addresses: InternalAddresses::default(),
            verifier: Arc::new(RefuseAll),
            prover: Arc::new(NoTokens),
            guard: Arc::new(AllowAll),
            retry: Retry::default(),
            consumer_transfers: ConsumerTransferPorts {
                store: Shared(Arc::clone(store) as _),
                decider: default_consumer_transfer_decider(),
                held: Shared(Arc::clone(store) as _),
            },
            consumer_store: Shared(Arc::clone(store) as _),
            consumer_decider: default_consumer_decider(),
            harvest: crate::harvest::HarvestPorts {
                store: Shared(Arc::clone(store) as _),
                source: None,
                extra_check: None,
            },
            object_storage: None,
            credential_types: CredentialTypes::default(),
            leases: leases_in(store),
        }
    }
}

impl Ports {
    /// Keeps negotiations and transfers (as provider and as consumer), the
    /// agreements transfers are checked against, the harvested datasets,
    /// the data flows and the push receipts in one database: PostgreSQL
    /// or a `SQLite` file.
    pub fn use_database(&mut self, store: &Arc<ConnectorStore>) {
        // `Arc::clone(store) as _` makes a new pointer to the same store and
        // lets the compiler turn it into the `Arc<dyn Port>` each field needs
        // (`_`: "the type this field expects").
        self.store = Shared(Arc::clone(store) as _);
        self.consumer_store = Shared(Arc::clone(store) as _);
        self.transfers.store = AnyTransferStore::new(Arc::clone(store) as _);
        self.transfers.agreements = Shared(Arc::clone(store) as _);
        self.consumer_transfers.store = Shared(Arc::clone(store) as _);
        self.consumer_transfers.held = Shared(Arc::clone(store) as _);
        self.harvest.store = Shared(Arc::clone(store) as _);
        self.push_receipts = Shared(Arc::clone(store) as _);
        if self.flow_store.is_some() {
            self.flow_store = Some(Shared(Arc::clone(store) as _));
        }
        self.leases = leases_in(store);
    }
}

/// The leases in `store`'s database, held under this process's id.
fn leases_in(store: &ConnectorStore) -> Leases {
    let database = Arc::clone(store.database()) as Arc<dyn LeaseStore>;
    Leases::new(database, crate::system::process_id())
}

impl Ports {
    /// Applies the rules of the dataspace's bundle (ADR 0005), through
    /// `copy`, which always holds the version in use: the harvest checks
    /// each dataset against its SHACL shapes after the DSP rules (when it
    /// has `datasets` shapes), and DSP messages are checked against its
    /// `messages` shapes in both directions (when it has any). Without a
    /// call, none of this happens.
    pub fn use_bundle_rules(&mut self, copy: &crate::bundle_harvest::BundleCopy) {
        self.harvest.extra_check = Some(Arc::new(copy.clone()) as _);
        self.guard = Arc::new(copy.clone());
    }
}

/// The adapters behind the transfer endpoints (part of [`Ports`]).
#[derive(Clone, Debug)]
pub struct TransferPorts {
    /// Where transfers are kept.
    pub store: AnyTransferStore,
    /// Where the agreements are found: the negotiation store.
    pub agreements: AnyAgreements,
    /// The data plane.
    pub flows: AnyDataFlows,
    /// What to do with a requested transfer.
    pub decider: AnyTransferDecider,
}

/// The adapters behind transfers as consumer (part of [`Ports`]).
#[derive(Clone, Debug)]
pub struct ConsumerTransferPorts {
    /// Where transfers are kept.
    pub store: AnyConsumerTransferStore,
    /// What the consumer does on its own.
    pub decider: AnyConsumerTransferDecider,
    /// The agreements a transfer may be requested under.
    pub held: AnyHeldAgreements,
}

#[cfg(feature = "tck")]
fn default_decider() -> AnyDecider {
    Shared(Arc::new(crate::tck::TckDecider::default()))
}

#[cfg(not(feature = "tck"))]
fn default_decider() -> AnyDecider {
    Shared(Arc::new(
        portitor_dsp::negotiation::provider::AgreeToCatalogOffers,
    ))
}

#[cfg(feature = "tck")]
fn default_consumer_decider() -> AnyConsumerDecider {
    Shared(Arc::new(crate::tck::TckConsumerDecider))
}

#[cfg(not(feature = "tck"))]
fn default_consumer_decider() -> AnyConsumerDecider {
    Shared(Arc::new(
        portitor_dsp::negotiation::consumer::AcceptAndVerify,
    ))
}

#[cfg(feature = "tck")]
fn default_transfer_decider() -> AnyTransferDecider {
    Shared(Arc::new(crate::tck::TckTransferDecider::default()))
}

#[cfg(not(feature = "tck"))]
fn default_transfer_decider() -> AnyTransferDecider {
    Shared(Arc::new(portitor_dsp::transfer::provider::StartAtOnce))
}

#[cfg(feature = "tck")]
fn default_consumer_transfer_decider() -> AnyConsumerTransferDecider {
    Shared(Arc::new(crate::tck::TckConsumerTransferDecider))
}

#[cfg(not(feature = "tck"))]
fn default_consumer_transfer_decider() -> AnyConsumerTransferDecider {
    Shared(Arc::new(portitor_dsp::transfer::consumer::OnlyWhenAsked))
}

/// Any adapter for a port, chosen at start.
///
/// The domain services are generic: `ProviderService<S, ...>` works with
/// any `S` that implements `NegotiationStore`. The connector picks the
/// adapter when it starts (a test may pass its own fake), so
/// it needs one type that can hold either. `Shared<dyn NegotiationStore +
/// Send + Sync>` is that type: an `Arc` (a shared pointer, cheap to clone)
/// to "some value that implements `NegotiationStore`". Each port below
/// gets a short name for its `Shared` type (`AnyStore`, `AnyDecider`, ...)
/// and an `impl` that passes every call on to the adapter inside (`self.0`).
pub struct Shared<T: ?Sized>(pub Arc<T>);

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared")
    }
}

/// The provider's negotiation store chosen at start.
pub type AnyStore = Shared<dyn NegotiationStore + Send + Sync>;

/// The provider's negotiation policy chosen at start.
pub type AnyDecider = Shared<dyn Decider + Send + Sync>;

/// The consumer's negotiation store chosen at start.
pub type AnyConsumerStore = Shared<dyn ConsumerStore + Send + Sync>;

/// The consumer's negotiation policy chosen at start.
pub type AnyConsumerDecider = Shared<dyn ConsumerDecider + Send + Sync>;

// The four process stores below also report each state change
// (`state_events`): `insert` is a new process. `replace_if` first reads
// the record as it is (`previous`), because the state change event names
// the state before the write and the write does not return it. It
// reports only when the write happened and `previous` is the version the
// write replaced (compare-and-update, ADR 0007): then the state read is
// exactly the one replaced, even with other writers.

impl NegotiationStore for AnyStore {
    fn get(&self, provider_pid: &str) -> Result<Option<Negotiation>, StoreError> {
        self.0.get(provider_pid)
    }
    fn negotiation_requested_by(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Negotiation>, StoreError> {
        self.0.negotiation_requested_by(counter_party, consumer_pid)
    }
    fn insert(&self, negotiation: &Negotiation) -> Result<bool, StoreError> {
        let inserted = self.0.insert(negotiation)?;
        // a repeated first request adds nothing: no state change
        if inserted {
            saved_as_provider(negotiation, None);
        }
        Ok(inserted)
    }
    fn replace_if(&self, negotiation: &Negotiation, expected: u64) -> Result<bool, StoreError> {
        // read first: see the comment above `impl NegotiationStore`
        let previous = self.0.get(&negotiation.provider_pid)?;
        let replaced = self.0.replace_if(negotiation, expected)?;
        if replaced
            && let Some(previous) = previous
            && previous.version == expected
        {
            saved_as_provider(negotiation, Some(previous.state));
        }
        Ok(replaced)
    }
    fn pending(&self) -> Result<Vec<Negotiation>, StoreError> {
        self.0.pending()
    }
}

fn saved_as_provider(negotiation: &Negotiation, before: Option<NegotiationState>) {
    state_events::negotiation_saved(
        Role::Provider,
        &negotiation.provider_pid,
        &negotiation.consumer_pid,
        before,
        negotiation.state,
    );
}

impl Decider for AnyDecider {
    fn decide(&self, negotiation: &Negotiation) -> Decision {
        self.0.decide(negotiation)
    }
}

impl ConsumerStore for AnyConsumerStore {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerNegotiation>, StoreError> {
        self.0.get(consumer_pid)
    }
    fn insert(&self, negotiation: &ConsumerNegotiation) -> Result<(), StoreError> {
        self.0.insert(negotiation)?;
        saved_as_consumer(negotiation, None);
        Ok(())
    }
    fn replace_if(
        &self,
        negotiation: &ConsumerNegotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        // read first: see the comment above `impl NegotiationStore`
        let previous = self.0.get(&negotiation.consumer_pid)?;
        let replaced = self.0.replace_if(negotiation, expected)?;
        if replaced
            && let Some(previous) = previous
            && previous.version == expected
        {
            saved_as_consumer(negotiation, Some(previous.state));
        }
        Ok(replaced)
    }
    fn list(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        self.0.list(after, limit)
    }
    fn pending(&self) -> Result<Vec<ConsumerNegotiation>, StoreError> {
        self.0.pending()
    }
}

fn saved_as_consumer(negotiation: &ConsumerNegotiation, before: Option<NegotiationState>) {
    state_events::negotiation_saved(
        Role::Consumer,
        negotiation.provider_pid.as_deref().unwrap_or_default(),
        &negotiation.consumer_pid,
        before,
        negotiation.state,
    );
}

impl ConsumerDecider for AnyConsumerDecider {
    fn decide(&self, negotiation: &ConsumerNegotiation) -> ConsumerDecision {
        self.0.decide(negotiation)
    }
}

/// The transfer store chosen at start, as provider. Unlike the other
/// stores it is not a plain [`Shared`]: it reports a transfer's state
/// change only once the data plane step its write recorded has run
/// (`impl TransferStore` below).
#[derive(Clone)]
pub struct AnyTransferStore {
    store: Arc<dyn TransferStore + Send + Sync>,
}

impl AnyTransferStore {
    /// Reports the state changes of the transfers kept in `store`.
    #[must_use]
    pub fn new(store: Arc<dyn TransferStore + Send + Sync>) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for AnyTransferStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnyTransferStore")
    }
}

/// The agreements chosen at start (the negotiation store's).
pub type AnyAgreements = Shared<dyn Agreements + Send + Sync>;

/// The data plane chosen at start.
pub type AnyDataFlows = Shared<dyn DataFlows + Send + Sync>;

/// The transfer policy chosen at start.
pub type AnyTransferDecider = Shared<dyn TransferDecider + Send + Sync>;

/// A transfer's state change is reported once the data plane step that
/// its write recorded has run (`portitor-dsp`,
/// `transfer::provider::flow_steps`), from the state it had before that
/// write. That state is kept in the database with the step
/// (`Transfer::state_before_step`), so the change is reported by whichever
/// process runs the step: the one whose call wrote it, or another one
/// that took it over after its lease ended (TD-48). A write that the data
/// plane's refusal takes back is then reported by neither write: the
/// transfer is in the same state as before (as in 1.1, which wrote
/// nothing), and `state_events::transfer_saved` reports no change to the
/// same state.
impl TransferStore for AnyTransferStore {
    fn get(&self, provider_pid: &str) -> Result<Option<Transfer>, StoreError> {
        self.store.get(provider_pid)
    }
    fn requested(
        &self,
        counter_party: &str,
        consumer_pid: &str,
    ) -> Result<Option<Transfer>, StoreError> {
        self.store.requested(counter_party, consumer_pid)
    }
    fn insert(&self, transfer: &Transfer) -> Result<bool, StoreError> {
        let inserted = self.store.insert(transfer)?;
        // a repeated request adds nothing, so there is no state change; a
        // new transfer with a step is reported once the step has run
        if inserted && transfer.flow_step.is_none() {
            transfer_saved_as_provider(transfer, None);
        }
        Ok(inserted)
    }
    fn replace_if(&self, transfer: &Transfer, expected: u64) -> Result<bool, StoreError> {
        // read first: see the comment above `impl NegotiationStore`
        let previous = self.store.get(&transfer.provider_pid)?;
        let replaced = self.store.replace_if(transfer, expected)?;
        let Some(previous) = previous else {
            return Ok(replaced);
        };
        if !replaced || previous.version != expected {
            return Ok(replaced);
        }
        if transfer.flow_step.is_some() {
            // the step has not run yet: it may be refused and this write
            // taken back. Reported once it has run (below)
            return Ok(replaced);
        }
        let before = if previous.flow_step.is_some() {
            // this write records that a step ran: the change is the one of
            // the write that recorded the step
            previous.state_before_step
        } else {
            Some(previous.state)
        };
        transfer_saved_as_provider(transfer, before);
        Ok(replaced)
    }
    fn pending(&self) -> Result<Vec<Transfer>, StoreError> {
        self.store.pending()
    }
    fn recorded_steps(&self) -> Result<Vec<Transfer>, StoreError> {
        self.store.recorded_steps()
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<Transfer>, StoreError> {
        self.store.list(after, limit)
    }
}

fn transfer_saved_as_provider(transfer: &Transfer, before: Option<TransferState>) {
    state_events::transfer_saved(
        Role::Provider,
        &transfer.provider_pid,
        &transfer.consumer_pid,
        before,
        transfer.state,
    );
}

impl Agreements for AnyAgreements {
    fn finalized(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        self.0.finalized(agreement_id)
    }
    fn negotiation_of(&self, agreement_id: &str) -> Result<Option<Negotiation>, StoreError> {
        self.0.negotiation_of(agreement_id)
    }
    fn replace_negotiation_if(
        &self,
        negotiation: &Negotiation,
        expected: u64,
    ) -> Result<bool, StoreError> {
        // a transfer request settled the finalization event (ADR 0007,
        // amended in 0.13): the state change is reported as for every
        // other write of a negotiation. Read first: see the comment above
        // `impl NegotiationStore`. The read is by agreement id, the only
        // read this port has; a negotiation keeps its agreement.
        let previous = match &negotiation.agreement {
            Some(agreement) => self.0.negotiation_of(&agreement.id)?,
            None => None,
        };
        let replaced = self.0.replace_negotiation_if(negotiation, expected)?;
        if replaced
            && let Some(previous) = previous
            && previous.provider_pid == negotiation.provider_pid
            && previous.version == expected
        {
            saved_as_provider(negotiation, Some(previous.state));
        }
        Ok(replaced)
    }
}

impl DataFlows for AnyDataFlows {
    fn supports(&self, dataset_id: &str, format: &str) -> Result<bool, String> {
        self.0.supports(dataset_id, format)
    }
    fn start(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String> {
        self.0.start(transfer)
    }
    fn suspend(&self, pid: &str) -> Result<(), String> {
        self.0.suspend(pid)
    }
    fn restart(&self, transfer: &Transfer) -> Result<Option<DataAddress>, String> {
        self.0.restart(transfer)
    }
    fn resume(&self, pid: &str, address: Option<&DataAddress>) -> Result<(), String> {
        self.0.resume(pid, address)
    }
    fn complete(&self, pid: &str) -> Result<(), String> {
        self.0.complete(pid)
    }
    fn terminate(&self, pid: &str) -> Result<(), String> {
        self.0.terminate(pid)
    }
}

impl TransferDecider for AnyTransferDecider {
    fn decide(&self, transfer: &Transfer) -> TransferDecision {
        self.0.decide(transfer)
    }
}

/// The consumer transfer store chosen at start.
pub type AnyConsumerTransferStore = Shared<dyn ConsumerTransferStore + Send + Sync>;

/// The consumer transfer policy chosen at start.
pub type AnyConsumerTransferDecider = Shared<dyn ConsumerTransferDecider + Send + Sync>;

/// The held agreements chosen at start (the negotiation store's).
pub type AnyHeldAgreements = Shared<dyn HeldAgreements + Send + Sync>;

impl ConsumerTransferStore for AnyConsumerTransferStore {
    fn get(&self, consumer_pid: &str) -> Result<Option<ConsumerTransfer>, StoreError> {
        self.0.get(consumer_pid)
    }
    fn insert(&self, transfer: &ConsumerTransfer) -> Result<(), StoreError> {
        self.0.insert(transfer)?;
        transfer_saved_as_consumer(transfer, None);
        Ok(())
    }
    fn replace_if(&self, transfer: &ConsumerTransfer, expected: u64) -> Result<bool, StoreError> {
        // read first: see the comment above `impl NegotiationStore`
        let previous = self.0.get(&transfer.consumer_pid)?;
        let replaced = self.0.replace_if(transfer, expected)?;
        if replaced
            && let Some(previous) = previous
            && previous.version == expected
        {
            transfer_saved_as_consumer(transfer, Some(previous.state));
        }
        Ok(replaced)
    }
    fn pending(&self) -> Result<Vec<ConsumerTransfer>, StoreError> {
        self.0.pending()
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<ConsumerTransfer>, StoreError> {
        self.0.list(after, limit)
    }
}

fn transfer_saved_as_consumer(transfer: &ConsumerTransfer, before: Option<TransferState>) {
    state_events::transfer_saved(
        Role::Consumer,
        transfer.provider_pid.as_deref().unwrap_or_default(),
        &transfer.consumer_pid,
        before,
        transfer.state,
    );
}

impl ConsumerTransferDecider for AnyConsumerTransferDecider {
    fn decide(&self, transfer: &ConsumerTransfer) -> ConsumerTransferDecision {
        self.0.decide(transfer)
    }
}

impl HeldAgreements for AnyHeldAgreements {
    fn provider_of(&self, agreement_id: &str) -> Result<Option<AgreementProvider>, StoreError> {
        self.0.provider_of(agreement_id)
    }
    fn agreement(&self, agreement_id: &str) -> Result<Option<Agreement>, StoreError> {
        self.0.agreement(agreement_id)
    }
}

/// What the connector needs of a flow store: the data plane's flows, and
/// the consumer's memberships the control plane keeps with each flow. The
/// connector's database has both; any type with both is one (the empty `impl`
/// below gives it the name).
pub trait ProviderFlowStore: FlowStore + MembershipStore {}

impl<T: FlowStore + MembershipStore> ProviderFlowStore for T {}

/// The flow store chosen at start.
pub type AnyFlowStore = Shared<dyn ProviderFlowStore + Send + Sync>;

impl AnyFlowStore {
    /// The same store, as the data plane sees it: its flows only.
    #[must_use]
    pub fn flows_only(&self) -> Arc<dyn FlowStore + Send + Sync> {
        Arc::clone(&self.0) as Arc<dyn FlowStore + Send + Sync>
    }
}

impl FlowStore for AnyFlowStore {
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError> {
        self.0.get(id)
    }
    fn insert(&self, flow: &Flow) -> Result<bool, FlowStoreError> {
        self.0.insert(flow)
    }
    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError> {
        self.0.set_state(id, from, to)
    }
    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError> {
        self.0.set_token(id, state, token_hash, expires_at)
    }
    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError> {
        self.0.set_push_address(id, state, address)
    }
    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError> {
        self.0.started_pushes()
    }
}

impl MembershipStore for AnyFlowStore {
    fn memberships(&self, id: &str) -> Result<StoredMembership, FlowStoreError> {
        self.0.memberships(id)
    }
    fn keep_memberships(&self, id: &str, memberships: &[Membership]) -> Result<(), FlowStoreError> {
        self.0.keep_memberships(id, memberships)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use portitor_dsp::transfer::provider::{StartAtOnce, TransferService};
    use portitor_runtime::telemetry::STATE_EVENTS;
    use serde_json::Value;

    use super::*;
    use crate::harvest::StoredCatalog;
    use crate::system::SystemEnvironment;

    const PID: &str = "urn:uuid:p";
    const CONSUMER: &str = "did:web:consumer.example";

    /// The log, as the subscriber writes it.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Write for Log {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A data plane that takes every step, or refuses every step.
    struct DataPlane {
        refuses: bool,
    }

    impl DataPlane {
        fn answer(&self) -> Result<(), String> {
            if self.refuses {
                Err("data plane down".to_owned())
            } else {
                Ok(())
            }
        }
    }

    impl DataFlows for DataPlane {
        fn supports(&self, _: &str, _: &str) -> Result<bool, String> {
            Ok(true)
        }
        fn start(&self, _: &Transfer) -> Result<Option<DataAddress>, String> {
            self.answer().map(|()| None)
        }
        fn suspend(&self, _: &str) -> Result<(), String> {
            self.answer()
        }
        fn resume(&self, _: &str, _: Option<&DataAddress>) -> Result<(), String> {
            self.answer()
        }
        fn restart(&self, _: &Transfer) -> Result<Option<DataAddress>, String> {
            self.answer().map(|()| None)
        }
        fn complete(&self, _: &str) -> Result<(), String> {
            self.answer()
        }
        fn terminate(&self, _: &str) -> Result<(), String> {
            self.answer()
        }
    }

    /// A transfer that is `STARTED`, with nothing pending.
    fn started() -> Transfer {
        Transfer {
            provider_pid: PID.to_owned(),
            consumer_pid: "urn:uuid:c".to_owned(),
            state: TransferState::Started,
            callback_address: "https://consumer.example/cb".to_owned(),
            counter_party: CONSUMER.to_owned(),
            agreement_id: "a1".to_owned(),
            dataset_id: "d1".to_owned(),
            format: "HttpData-PULL".to_owned(),
            data_address: None,
            pending: None,
            flow_step: None,
            state_before_step: None,
            lease: None,
            version: 0,
        }
    }

    /// A provider's transfer service on `sqlite`, as one process of the
    /// connector runs it, with a data plane that `refuses` every step or
    /// takes it.
    fn service_on(
        sqlite: &Arc<ConnectorStore>,
        refuses: bool,
    ) -> TransferService<
        AnyTransferStore,
        AnyAgreements,
        DataPlane,
        StartAtOnce,
        StoredCatalog,
        SystemEnvironment,
    > {
        let store = AnyTransferStore::new(Arc::clone(sqlite) as _);
        let agreements: AnyAgreements = Shared(Arc::clone(sqlite) as _);
        TransferService::new(
            store,
            agreements,
            DataPlane { refuses },
            StartAtOnce,
            StoredCatalog::empty(),
            SystemEnvironment,
        )
    }

    /// The state events logged while `action` runs.
    fn state_events_while(action: impl FnOnce()) -> Vec<Value> {
        portitor_runtime::telemetry::testing::keep_a_second_subscriber();
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, action);
        let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        let mut events = Vec::new();
        for line in text.lines() {
            let line: Value = serde_json::from_str(line).unwrap();
            if line["target"] == STATE_EVENTS {
                events.push(line);
            }
        }
        events
    }

    /// The state events logged while the consumer terminates a `STARTED`
    /// transfer, with a data plane that `refuses` the step or takes it.
    fn state_events_of_a_termination(refuses: bool) -> Vec<Value> {
        let sqlite = Arc::new(ConnectorStore::in_memory().unwrap());
        let service = service_on(&sqlite, refuses);
        AnyTransferStore::new(Arc::clone(&sqlite) as _)
            .insert(&started())
            .unwrap();
        state_events_while(|| {
            let answer = service.terminate(PID, (PID, "urn:uuid:c"), CONSUMER);
            assert_eq!(answer.is_err(), refuses, "{answer:?}");
        })
    }

    /// A consumer's step that the data plane refuses takes its write back
    /// (`flow_steps`): neither write is reported, so nothing counts in
    /// `portitor.transfer.ended`, which `state_events::transfer_saved`
    /// counts with the event. A step taken is reported once.
    #[test]
    fn a_refused_consumer_step_reports_no_state_change() {
        assert_eq!(state_events_of_a_termination(true), Vec::<Value>::new());
        let events = state_events_of_a_termination(false);
        assert_eq!(events.len(), 1, "{events:?}");
        let fields = &events[0]["fields"];
        assert_eq!(fields["portitor.dsp.previous_state"], "STARTED");
        assert_eq!(fields["portitor.dsp.state"], "TERMINATED");
    }

    /// A step left recorded by another process (it wrote the consumer's
    /// suspension, then stopped) is run here, by the sweep, once its lease
    /// has ended: the change is reported once, from the state kept with
    /// the step in the database, not from anything in this process's
    /// memory (TD-48).
    #[test]
    fn a_step_run_by_another_process_reports_its_change_once() {
        use portitor_dsp::transfer::provider::{FlowStep, Lease};

        let sqlite = Arc::new(ConnectorStore::in_memory().unwrap());
        let mut left_over = started();
        left_over.state = TransferState::Suspended;
        left_over.flow_step = Some(FlowStep::Suspend);
        left_over.state_before_step = Some(TransferState::Started);
        left_over.lease = Some(Lease {
            holder: "urn:uuid:a-process-that-stopped".to_owned(),
            until: 1,
        });
        let file: &dyn TransferStore = sqlite.as_ref();
        file.insert(&left_over).unwrap();
        let service = service_on(&sqlite, false);

        let events = state_events_while(|| {
            let swept = service.run_steps_left_over().unwrap();
            assert_eq!(swept.steps_run, [PID]);
        });
        assert_eq!(events.len(), 1, "{events:?}");
        let fields = &events[0]["fields"];
        assert_eq!(fields["portitor.dsp.previous_state"], "STARTED");
        assert_eq!(fields["portitor.dsp.state"], "SUSPENDED");
        let saved = file.get(PID).unwrap().unwrap();
        assert_eq!((saved.flow_step, saved.lease), (None, None));
    }
}
