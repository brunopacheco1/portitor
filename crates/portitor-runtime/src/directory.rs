// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the identity component and the issuer read from other parties:
//! their DID documents (to check their tokens, and to find their
//! services), and an issuer's metadata and status lists (the identity
//! component only).
//! [`Directory`] is the port, [`WebDirectory`] the adapter over HTTPS
//! (`did:web` resolution with a cache, and the shared client: HTTPS only
//! unless allowed, time and size limits, no redirects).

use std::sync::Arc;
use std::time::Duration;

use portitor_dcp::DidDocument;
use serde_json::Value;

use crate::BoxFuture;
use crate::did::{DidWebResolver, ResolveError};
use crate::http::{HttpError, HttpSender, Sender as _};
use crate::internal_addresses::InternalAddresses;
use crate::status_lists::StatusListSource;

/// Port: reads other parties' public documents. Each lookup returns,
/// later ([`BoxFuture`]), the value or the reason it failed.
pub trait Directory: Send + Sync {
    /// The DID document of `did`.
    fn did_document<'a>(&'a self, did: &'a str) -> BoxFuture<'a, Result<Arc<DidDocument>, String>>;

    /// Like [`Directory::did_document`], and the error says whether
    /// trying again later may help ([`ResolveError::is_temporary`]). By
    /// default every failure is final; [`WebDirectory`] tells them apart.
    fn resolve_did_document<'a>(
        &'a self,
        did: &'a str,
    ) -> BoxFuture<'a, Result<Arc<DidDocument>, ResolveError>> {
        Box::pin(async move {
            self.did_document(did).await.map_err(|reason| ResolveError {
                reason,
                temporary: false,
            })
        })
    }

    /// The JSON document at `url` (an issuer's metadata).
    fn json<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>>;
}

/// Adapter: `did:web` resolution with a cache, and HTTP GET.
#[derive(Debug)]
pub struct WebDirectory {
    resolver: DidWebResolver,
    http: HttpSender,
}

impl WebDirectory {
    /// With `allow_http`, plain-HTTP DIDs and URLs are used (tests only).
    /// A DID document is reused for `did_cache_seconds`
    /// (`PORTITOR_DID_CACHE_SECONDS`, read with
    /// [`crate::did::cache_seconds`]); 0 fetches it every time. For DIDs
    /// the operator chose (a bundle authority): the block on internal
    /// addresses does not apply ([`WebDirectory::for_partners`]).
    ///
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn new(allow_http: bool, did_cache_seconds: u64) -> Result<Self, String> {
        Self::for_partners(allow_http, did_cache_seconds, &InternalAddresses::default())
    }

    /// Like [`WebDirectory::new`], for other parties' DIDs and documents
    /// (callers, holders, issuers): each is fetched only from an address
    /// that `internal_addresses` allows (R-17).
    ///
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn for_partners(
        allow_http: bool,
        did_cache_seconds: u64,
        internal_addresses: &InternalAddresses,
    ) -> Result<Self, String> {
        let did_cache = Duration::from_secs(did_cache_seconds);
        Ok(Self {
            resolver: DidWebResolver::for_partners(allow_http, did_cache, internal_addresses)?,
            http: HttpSender::for_partners(allow_http, internal_addresses)?,
        })
    }
}

impl Directory for WebDirectory {
    fn did_document<'a>(&'a self, did: &'a str) -> BoxFuture<'a, Result<Arc<DidDocument>, String>> {
        Box::pin(async move {
            // the error as text: this port does not tell temporary
            // failures apart
            let doc = self.resolver.resolve(did).await?;
            Ok(doc)
        })
    }

    fn resolve_did_document<'a>(
        &'a self,
        did: &'a str,
    ) -> BoxFuture<'a, Result<Arc<DidDocument>, ResolveError>> {
        Box::pin(self.resolver.resolve(did))
    }

    fn json<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        self.http.get(url)
    }
}

/// The status lists of credentials are read with the same sender as the
/// other parties' documents: their URLs are chosen by issuers too (the
/// identity's status check, TD-20).
impl StatusListSource for WebDirectory {
    fn status_list_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, HttpError>> {
        self.http.status_list_text(url)
    }
}
