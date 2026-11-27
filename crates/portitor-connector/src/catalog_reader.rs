// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading a provider's catalog as consumer (DSP 2025-1, "Catalog HTTPS
//! Binding"): `POST <provider address>/catalog/request` for the catalog
//! and `GET <provider address>/catalog/datasets/{id}` for one dataset.
//! The management API uses it (`POST /v1/catalog/request`,
//! `POST /v1/catalog/dataset/request`), so an operator needs no token
//! of their own to see what a provider offers (TD-38).
//!
//! Each request carries a fresh DCP token for the provider, from the same
//! [`Prover`] the negotiation and transfer messages use (ADR 0016), and
//! goes out through the same [`Sender`], so the rule for outbound URLs is
//! the same too (`https://` only, unless `PORTITOR_ALLOW_HTTP`).
//!
//! What differs from the outbox ([`crate::outbox`]):
//!
//! - One try, no retries: a person or a script waits for the answer and
//!   can ask again. Nothing is stored.
//! - A larger answer is read ([`MAX_CATALOG_ANSWER`]) and more time is
//!   given ([`CATALOG_TIMEOUT`]).
//! - The provider's answer is handed on as it came, byte for byte. It is
//!   only checked to be a JSON object whose `@type` is `Catalog` (for a
//!   dataset: `Dataset`, or no `@type` at all, as the DSP 2025-1 schema
//!   of a dataset does not require one): reading it into Portitor's own
//!   types and writing it again would drop whatever those types do not
//!   know.
//! - A catalog must be the catalog of the provider that was asked: its
//!   `participantId` is the provider id of the request (the
//!   `counterPartyId` of the management call). DSP 2025-1 requires
//!   `participantId` of a catalog (`catalog-schema.json`, `RootCatalog`),
//!   so a catalog without it is refused too. A dataset has no
//!   `participantId`.
//! - At most [`MAX_CONCURRENT_READS`] answers are read at a time, as each
//!   may be held in memory whole.
//!
//! The code is in three parts: this file (the limits, the reader and the
//! call to the provider), `request` (what is checked and built before
//! anything is sent) and `answer` (the checks of the provider's answer).
//!
//! Pages: a provider MAY send its catalog in pages, named by `Link`
//! headers ("Catalog HTTPS Binding", "Pagination"). Portitor reads the
//! first answer only and does not follow `rel="next"`: the specification
//! does not say how the pages of a catalog are joined into one. The
//! provider's `Link` header values are returned with the answer
//! ([`ProviderAnswer::links`]), so the caller can see that there is more.

use std::sync::Arc;
use std::time::Duration;

use portitor_dsp::guard::MessageGuard;

use crate::auth::Prover;
use crate::http_client::{AnswerLimits, Bytes, HttpError, RawAnswer, Sender};
use crate::ports::Ports;

mod answer;
mod request;

use answer::{check_catalog, check_dataset};
use request::{catalog_request_message, check_dataset_id, check_provider, path_segment};

/// The largest catalog (or dataset) answer read from a provider: 16 MiB.
///
/// Every other answer from a counter-party is read up to 64 KiB, which
/// holds about 250 small datasets (TD-36). The 0.12 benchmark measured
/// 2.6 MB for a catalog of 10,000 small datasets (`docs/BENCHMARKS.md`),
/// and a national gateway lists about 1,100 datasets (TD-36); 16 MiB
/// leaves each of those about 15 kB of metadata. The limit is fixed, not
/// a setting: it only has to stop a provider from making the connector
/// buffer a body without end. A larger answer fails with HTTP 502 on the
/// management API.
pub const MAX_CATALOG_ANSWER: usize = 16 * 1024 * 1024;

/// How long the provider has to send its whole catalog (or dataset)
/// answer: longer than the 10 seconds of other calls, as the answer may
/// be large and the provider builds it on request. With the token
/// request before it (at most 10 seconds) this stays under the 60
/// seconds a request to the management port may take
/// (`portitor_runtime::serve::REQUEST_TIMEOUT`, set on that port in
/// `main.rs`).
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// How many provider answers are read at the same time. Each may hold
/// [`MAX_CATALOG_ANSWER`] in memory (and about twice that while its
/// parts are joined), so without a limit a script sending many catalog
/// requests at once could use up the connector's memory. A request over
/// the limit waits for a free place; the time limit of the management
/// port ends a wait that is too long with HTTP 503.
pub const MAX_CONCURRENT_READS: usize = 4;

/// A provider's answer to a catalog or dataset request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAnswer {
    /// The body as the provider sent it: a DSP 2025-1 `Catalog` or
    /// `Dataset` in JSON.
    pub body: Bytes,
    /// The provider's `Link` header values (catalog pages); empty when
    /// the answer is the whole catalog.
    pub links: Vec<String>,
}

/// Why a provider's catalog or dataset could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    /// The request cannot be sent as asked; the text says why. Nothing
    /// was sent. (HTTP 400 on the management API.)
    BadRequest(String),
    /// Our identity component gave no token for the provider. The reason
    /// is in the log. (HTTP 502.)
    NoToken,
    /// The provider did not give a usable answer: it refused, could not
    /// be reached, or sent more than [`MAX_CATALOG_ANSWER`]. (HTTP 502.)
    Provider(HttpError),
    /// The provider answered 2xx, but not with what was asked for (also:
    /// with the catalog of another participant); the text says what is
    /// wrong. (HTTP 502.)
    NotDsp(String),
}

/// What to read from a provider.
enum Wanted<'a> {
    /// Its catalog.
    Catalog,
    /// One dataset, by its `@id`.
    Dataset(&'a str),
}

/// Reads catalogs and datasets from providers, with our token.
pub(crate) struct CatalogReader {
    sender: Arc<dyn Sender>,
    prover: Arc<dyn Prover>,
    guard: Arc<dyn MessageGuard + Send + Sync>,
    allow_http: bool,
    /// One permit per answer being read ([`MAX_CONCURRENT_READS`]).
    reads: tokio::sync::Semaphore,
}

impl CatalogReader {
    pub(crate) fn new(ports: &Ports, allow_http: bool) -> Self {
        Self {
            sender: Arc::clone(&ports.sender),
            prover: Arc::clone(&ports.prover),
            guard: Arc::clone(&ports.guard),
            allow_http,
            reads: tokio::sync::Semaphore::new(MAX_CONCURRENT_READS),
        }
    }

    /// The catalog of the provider `provider_id` (a `did:web` DID) whose
    /// DSP endpoint is `provider_address`. Our token lets the provider
    /// read `additional_scopes` from our Credential Service, on top of
    /// the configured scopes.
    pub(crate) async fn catalog(
        &self,
        provider_address: &str,
        provider_id: &str,
        additional_scopes: &[String],
    ) -> Result<ProviderAnswer, ReadFailure> {
        self.read(
            provider_address,
            provider_id,
            additional_scopes,
            &Wanted::Catalog,
        )
        .await
    }

    /// The dataset `dataset_id` of the provider `provider_id` whose DSP
    /// endpoint is `provider_address`. Our token grants
    /// `additional_scopes` too, as for [`CatalogReader::catalog`].
    pub(crate) async fn dataset(
        &self,
        provider_address: &str,
        provider_id: &str,
        dataset_id: &str,
        additional_scopes: &[String],
    ) -> Result<ProviderAnswer, ReadFailure> {
        check_dataset_id(dataset_id)?;
        let wanted = Wanted::Dataset(dataset_id);
        self.read(provider_address, provider_id, additional_scopes, &wanted)
            .await
    }

    /// Checks the address and the id, gets a token, sends the request
    /// once and checks the answer (`check_catalog`, `check_dataset`).
    async fn read(
        &self,
        provider_address: &str,
        provider_id: &str,
        scopes: &[String],
        wanted: &Wanted<'_>,
    ) -> Result<ProviderAnswer, ReadFailure> {
        check_provider(provider_address, provider_id, self.allow_http)?;
        // the semaphore is never closed, so this waits for a free place
        let _reading = self.reads.acquire().await;
        let sent = self.send(provider_address, provider_id, scopes, wanted);
        let RawAnswer { body, links } = sent.await?;
        match wanted {
            Wanted::Catalog => check_catalog(&body, provider_id)?,
            Wanted::Dataset(_) => check_dataset(&body)?,
        }
        if !links.is_empty() {
            tracing::warn!(
                "the provider sent Link headers (catalog pages); only this first answer is read"
            );
        }
        Ok(ProviderAnswer { body, links })
    }

    /// Sends the one request for `wanted` to the provider, with a fresh
    /// token, and reads the answer within the limits of this module
    /// ([`MAX_CATALOG_ANSWER`], [`CATALOG_TIMEOUT`]): a `POST` of a
    /// `CatalogRequestMessage` for the catalog, a `GET` for a dataset.
    async fn send(
        &self,
        provider_address: &str,
        provider_id: &str,
        scopes: &[String],
        wanted: &Wanted<'_>,
    ) -> Result<RawAnswer, ReadFailure> {
        let base = provider_address.trim_end_matches('/');
        let limits = AnswerLimits {
            max_bytes: MAX_CATALOG_ANSWER,
            timeout: CATALOG_TIMEOUT,
        };
        let sent = match wanted {
            Wanted::Catalog => {
                let message = catalog_request_message(self.guard.as_ref())?;
                let authorization = self.authorization(provider_id, scopes).await?;
                let url = format!("{base}/catalog/request");
                self.sender
                    .post_authorized_within(&url, &message, &authorization, limits)
                    .await
            }
            Wanted::Dataset(dataset_id) => {
                let authorization = self.authorization(provider_id, scopes).await?;
                let url = format!("{base}/catalog/datasets/{}", path_segment(dataset_id));
                self.sender
                    .get_authorized_within(&url, &authorization, limits)
                    .await
            }
        };
        sent.map_err(ReadFailure::Provider)
    }

    /// The `Authorization` header for one request to `provider_id`.
    async fn authorization(
        &self,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<String, ReadFailure> {
        match self.prover.authorization(provider_id, scopes).await {
            Ok(authorization) => Ok(authorization),
            Err(error) => {
                // the text may name our STS: for the log, not the caller
                tracing::warn!(%error, "no token to read a provider's catalog");
                Err(ReadFailure::NoToken)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dsp::guard::{Direction, Rejection};
    use portitor_runtime::BoxFuture;
    use serde_json::Value;

    use super::*;

    const PROVIDER: &str = "did:web:provider.example";

    /// Never sends: the tests below fail before a request goes out.
    struct NeverSends;

    impl Sender for NeverSends {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Err("sent".to_owned()) })
        }
    }

    struct RefusesAll;

    impl MessageGuard for RefusesAll {
        fn check(&self, _: Direction, _: &str, _: &Value) -> Result<(), Rejection> {
            Err(Rejection {
                reasons: vec!["no catalog requests here".to_owned()],
            })
        }
    }

    fn reader(allow_http: bool) -> CatalogReader {
        CatalogReader::new(&Ports::in_memory(Arc::new(NeverSends)), allow_http)
    }

    #[tokio::test]
    async fn a_bad_address_or_id_is_refused_before_anything_is_sent() {
        let provider = PROVIDER;
        for (address, id, allow_http) in [
            ("http://provider.example/dsp", provider, false),
            ("ftp://provider.example/dsp", provider, true),
            ("https://provider.example/dsp?x=1", provider, false),
            ("https://provider.example/dsp#x", provider, false),
            ("https://provider.example/dsp", "provider", false),
            ("https://provider.example/dsp", "did:key:z6Mk", false),
        ] {
            let read = reader(allow_http).catalog(address, id, &[]).await;
            assert!(
                matches!(read, Err(ReadFailure::BadRequest(_))),
                "{address} {id}"
            );
        }
        let read = reader(false)
            .dataset("https://provider.example/dsp", provider, " ", &[])
            .await;
        assert!(matches!(read, Err(ReadFailure::BadRequest(_))));
        for dots in [".", ".."] {
            let read = reader(false)
                .dataset("https://provider.example/dsp", provider, dots, &[])
                .await;
            assert!(matches!(read, Err(ReadFailure::BadRequest(_))), "{dots}");
        }
    }

    #[tokio::test]
    async fn without_a_token_nothing_is_sent() {
        // `Ports::new` starts with a prover that has no tokens
        let read = reader(false)
            .catalog(
                "https://provider.example/dsp",
                "did:web:provider.example",
                &[],
            )
            .await;
        assert_eq!(read, Err(ReadFailure::NoToken));
    }

    #[tokio::test]
    async fn the_dataspace_rules_can_refuse_the_request() {
        let mut ports = Ports::in_memory(Arc::new(NeverSends));
        ports.guard = Arc::new(RefusesAll);
        let read = CatalogReader::new(&ports, false)
            .catalog(
                "https://provider.example/dsp",
                "did:web:provider.example",
                &[],
            )
            .await;
        let Err(ReadFailure::BadRequest(reason)) = read else {
            panic!("not refused");
        };
        assert!(reason.contains("no catalog requests here"), "{reason}");
    }
}
