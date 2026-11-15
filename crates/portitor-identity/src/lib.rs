// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The Portitor identity component (ADR 0015): it holds a participant's
//! key and credentials, and answers for it in the Decentralized Claims
//! Protocol v1.0.1.
//!
//! - its DID document (`did:web`), [`did_document`];
//! - a Secure Token Service (STS) that signs self-issued tokens for the
//!   participant's own connector, [`sts`];
//! - the credential service: the presentation API (scopes only),
//!   [`presentation`]; the Storage API and credential offers, [`storage`];
//! - asking an issuer for credentials (holder side of DCP issuance),
//!   [`issuance`];
//! - a management API (API key, as the connector's), [`management`];
//! - the X.509 chain of its key, [`certificate`], and Gaia-X onboarding,
//!   [`gaiax`] (ADR 0021).
//!
//! The public routes are in [`router`]; the management API has its own
//! router and port.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};
use portitor_dcp::{JtiCache, SigningKey};
use portitor_runtime::api_key::ApiKey;
use portitor_runtime::directory::Directory;

mod caller;
pub mod certificate;
pub mod config;
pub mod did_document;
pub mod gaiax;
pub mod issuance;
pub mod management;
pub mod presentation;
pub mod storage;
pub mod store;
pub mod sts;

use certificate::CertificateChain;
use config::Config;
use gaiax::{GaiaxCompliance, Onboarding};
use issuance::IssuerClient;
use store::IdentityStore;

/// Allowed clock difference with other parties, in seconds.
pub const LEEWAY: u64 = 60;

/// How long the tokens and presentations this component signs are valid,
/// in seconds.
pub const TOKEN_SECONDS: u64 = 300;

/// The identity component's state, shared by all requests.
pub struct Identity {
    /// Our own DID (`PORTITOR_PARTICIPANT_ID`).
    participant_id: String,
    kid: String,
    key: SigningKey,
    document: serde_json::Value,
    did_path: String,
    service_path: String,
    trusted_issuers: Vec<String>,
    type_aliases: Vec<String>,
    sts_client_id: String,
    sts_client_secret: ApiKey,
    store: Arc<dyn IdentityStore>,
    directory: Arc<dyn Directory>,
    issuers: Arc<dyn IssuerClient>,
    used_token_ids: Mutex<JtiCache>,
    /// Where the participant's public documents are: its `did:web` URL
    /// without `did.json` ([`gaiax::public_base`]).
    public_base: String,
    /// The path of `public_base`, where the routes for those documents
    /// are: empty for a DID without a path.
    public_path: String,
    /// The X.509 chain of the key, if configured (ADR 0021).
    certificate: Option<Arc<CertificateChain>>,
    /// Gaia-X onboarding, if configured (ADR 0021).
    gaiax: Option<Onboarding>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("participant_id", &self.participant_id)
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

/// The ports the identity component uses, chosen at start.
pub struct Ports {
    /// Where credentials, requests and offers are kept.
    pub store: Arc<dyn IdentityStore>,
    /// Where other parties' DID documents and metadata are read.
    pub directory: Arc<dyn Directory>,
    /// Where the participant asks issuers for credentials.
    pub issuers: Arc<dyn IssuerClient>,
    /// The Gaia-X compliance service; needed when
    /// `PORTITOR_GAIAX_COMPLIANCE_URL` is set, unused otherwise.
    pub compliance: Option<Arc<dyn GaiaxCompliance>>,
}

impl Identity {
    /// Builds the component from its settings, its key and the STS client
    /// secret. Reads the certificate chain and the Gaia-X participant
    /// file, if configured.
    ///
    /// # Errors
    /// The participant DID or the credential service URL is invalid; the
    /// certificate chain cannot be read or its first certificate is not
    /// for `key`; Gaia-X is on without a compliance adapter, or its
    /// participant file is invalid.
    pub fn new(
        config: &Config,
        key: SigningKey,
        sts_client_secret: ApiKey,
        ports: Ports,
    ) -> Result<Self, String> {
        let kid = format!(
            "{}#{}",
            config.participant_id,
            key.public_jwk().thumbprint()
        );
        let mut document = did_document::build(
            &config.participant_id,
            &kid,
            &key,
            &config.credential_service_url,
        );
        let did_path = did_document::path(&config.participant_id)?;
        let public_base = gaiax::public_base(&config.participant_id)?;
        let public_path = gaiax::public_path(&config.participant_id)?;
        let certificate = match &config.certificate_chain_file {
            Some(file) => {
                let chain = CertificateChain::load(file, &key.public_jwk())
                    .map_err(|e| format!("PORTITOR_IDENTITY_CERTIFICATE_CHAIN: {e}"))?;
                let url = format!("{public_base}{}", certificate::CHAIN_PATH);
                portitor_runtime::did_document::add_x5u(&mut document, &kid, &url);
                Some(Arc::new(chain))
            }
            None => None,
        };
        let gaiax = match (&config.gaiax, ports.compliance) {
            (None, _) => None,
            (Some(_), None) => return Err("Gaia-X is on, but no compliance service".to_owned()),
            (Some(settings), Some(compliance)) => Some(
                Onboarding::new(compliance, &settings.participant_file)
                    .map_err(|e| format!("PORTITOR_GAIAX_PARTICIPANT_FILE: {e}"))?,
            ),
        };
        let service_path = config::base_path(&config.credential_service_url)
            .ok_or("PORTITOR_CREDENTIAL_SERVICE_URL is not an http(s) URL")?;
        Ok(Self {
            participant_id: config.participant_id.clone(),
            kid,
            key,
            document,
            did_path,
            service_path,
            trusted_issuers: config.trusted_issuers.clone(),
            type_aliases: config.scope_type_aliases.clone(),
            sts_client_id: config.sts_client_id.clone(),
            sts_client_secret,
            store: ports.store,
            directory: ports.directory,
            issuers: ports.issuers,
            used_token_ids: Mutex::new(JtiCache::default()),
            public_base,
            public_path,
            certificate,
            gaiax,
        })
    }

    /// The certificate chain of the key, if configured.
    #[must_use]
    pub fn certificate_chain(&self) -> Option<Arc<CertificateChain>> {
        self.certificate.clone()
    }

    /// The store of credentials, requests, offers and Gaia-X runs.
    #[must_use]
    pub fn store(&self) -> Arc<dyn IdentityStore> {
        Arc::clone(&self.store)
    }

    /// Whether Gaia-X onboarding is on.
    #[must_use]
    pub fn gaiax_on(&self) -> bool {
        self.gaiax.is_some()
    }

    /// The participant's DID.
    #[must_use]
    pub fn participant_id(&self) -> &str {
        &self.participant_id
    }

    /// The id of the participant's key in its DID document.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// Whether `did` is a trusted issuer (`PORTITOR_TRUSTED_ISSUERS`).
    fn trusts(&self, did: &str) -> bool {
        self.trusted_issuers.iter().any(|trusted| trusted == did)
    }

    /// The scope aliases that mean "credentials of this type".
    fn type_aliases(&self) -> Vec<&str> {
        self.type_aliases.iter().map(String::as_str).collect()
    }

    /// Who signs: this participant, with its key.
    fn signer(&self) -> portitor_dcp::Signer<'_> {
        portitor_dcp::Signer {
            key: &self.key,
            kid: &self.kid,
            did: &self.participant_id,
        }
    }
}

/// The public HTTP routes: the DID document, the STS, the credential
/// service (under the path of `PORTITOR_CREDENTIAL_SERVICE_URL`), the
/// certificate chain and the public Gaia-X credentials (under the DID's
/// path, see [`gaiax::public_path`]).
pub fn router(identity: Arc<Identity>) -> Router {
    let service = identity.service_path.clone();
    // below the DID's path, where the x5u and credential ids point
    let public = identity.public_path.clone();
    Router::new()
        .route(&identity.did_path, get(did_document::serve))
        .route("/sts/token", post(sts::token))
        .route(
            &format!("{service}/presentations/query"),
            post(presentation::query),
        )
        .route(
            &format!("{service}/credentials"),
            post(storage::credentials),
        )
        .route(&format!("{service}/offers"), post(storage::offers))
        .route(
            &format!("{public}{}", certificate::CHAIN_PATH),
            get(certificate::serve),
        )
        .route(
            &format!("{public}{}/{{name}}", gaiax::CREDENTIALS_PATH),
            get(gaiax::serve_credential),
        )
        .route("/health", get(|| async { "ok" }))
        .with_state(identity)
}
