// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The data plane's flows and where it keeps them: [`Flow`], the
//! [`FlowStore`] port (the connector's adapter is a table of its
//! database), and the access tokens of pull flows, which are kept only as
//! their SHA-256 hash ([`hash`], [`new_token`]).

use base64::Engine as _;
use portitor_dps::port::SignalError;
use portitor_dps::state::DataFlowState;
use portitor_dsp::transfer::DataAddress;
use portitor_runtime::system::hex;

use crate::audit::Parties;

/// One data flow as the data plane keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// The data flow id (the transfer's provider process id).
    pub id: String,
    /// The current state.
    pub state: DataFlowState,
    /// The dataset whose data is served.
    pub dataset_id: String,
    /// Where the data comes from: the dataset's source as the start
    /// message named it ([`crate::SOURCE_METADATA_KEY`]). `None` only for a flow
    /// started before 1.2 whose dataset had no source in the harvest's
    /// read copy when the database was migrated: such a flow is never
    /// served, as 1.1 did not serve it either.
    pub source: Option<String>,
    /// The agreement the transfer is made under, as the start message
    /// named it (`agreementId`). Empty only for a flow started before 1.2
    /// whose transfer could not be found when the database was migrated.
    pub agreement_id: String,
    /// The consumer's participant id, as the start message named it
    /// (`counterPartyId`). Empty in the same case as `agreement_id`.
    pub counter_party_id: String,
    /// SHA-256 of the access token, hex. Empty for a push flow, which
    /// has no token of ours: no hash matches it.
    pub token_hash: String,
    /// When the token stops working, in seconds since 1970.
    pub expires_at: u64,
    /// A push flow: where to send the data, the consumer's data address
    /// with its token (ADR 0027). `None` for a pull flow, and once the
    /// flow has ended. Never logged.
    pub push_address: Option<DataAddress>,
}

impl Flow {
    /// The agreement and the consumer of this flow, for the audit line.
    #[must_use]
    pub fn parties(&self) -> Parties {
        Parties {
            agreement_id: self.agreement_id.clone(),
            counter_party_id: self.counter_party_id.clone(),
        }
    }
}

/// A flow store failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStoreError(pub String);

/// Port: where the data plane keeps its flows.
pub trait FlowStore {
    /// One flow by id.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn get(&self, id: &str) -> Result<Option<Flow>, FlowStoreError>;

    /// Adds a new flow. Returns `false` if the id is taken.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn insert(&self, flow: &Flow) -> Result<bool, FlowStoreError>;

    /// Changes the state of a flow from `from` to `to`, atomically.
    /// Returns `false` if the flow was not in state `from`.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_state(
        &self,
        id: &str,
        from: DataFlowState,
        to: DataFlowState,
    ) -> Result<bool, FlowStoreError>;

    /// Replaces the token of a flow that is in `state`: the old token stops
    /// working. Returns `false` if the flow was not in `state`.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_token(
        &self,
        id: &str,
        state: DataFlowState,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<bool, FlowStoreError>;

    /// Replaces the consumer's address of a push flow that is in `state`
    /// (a start again with a new token). Returns `false` if the flow was
    /// not in `state`. A flow that moves to a final state through
    /// [`FlowStore::set_state`] loses its address there.
    ///
    /// # Errors
    ///
    /// The store cannot be written.
    fn set_push_address(
        &self,
        id: &str,
        state: DataFlowState,
        address: &DataAddress,
    ) -> Result<bool, FlowStoreError>;

    /// The ids of the push flows that are STARTED: after a restart, the
    /// pushes to send again.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    fn started_pushes(&self) -> Result<Vec<String>, FlowStoreError>;
}

/// SHA-256 of `token`, hex. Comparing two such hashes with `==` is safe:
/// what a timing difference could reveal is a prefix of the hash, and no
/// one can work back from a hash prefix to a token.
#[must_use]
pub fn hash(token: &str) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()).as_ref())
}

/// A new random access token: 32 bytes from the OS generator, base64url.
/// # Errors
///
/// The OS has no randomness to give.
pub fn new_token() -> Result<String, SignalError> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| SignalError::Failed("no randomness for the token".to_owned()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}
