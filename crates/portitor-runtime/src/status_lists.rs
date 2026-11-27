// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading a credential's status lists (revocation and suspension), as
//! the connector's DCP verifier does it (ADR 0016), shared since 1.4
//! with the identity's credential status check (TD-20).
//!
//! A credential names its status list credentials by URL
//! (`credentialStatus.statusListCredential`). [`StatusListReader`] gives
//! the list credential at a URL: from its [`cache`], or fetched through a
//! [`StatusListSource`] (the partner HTTP sender: the URL is chosen by an
//! issuer), parsed, and checked: it must be issued by the credential's
//! issuer and verify against that issuer's DID document (signature and
//! dates). Only a list that passed is kept in the cache.
//! [`entry_status`] then looks one status entry up in it
//! (`portitor_dcp::status_list`: `BitstringStatusList` or
//! `StatusList2021`).
//!
//! A fetch that got no answer, or 408, 429 or 5xx, is
//! [`StatusListError::Unavailable`]: trying later may help. Any other
//! failure is [`StatusListError::Rejected`].

use std::sync::Arc;

use portitor_dcp::DidDocument;
use portitor_dcp::credential::Credential;
use portitor_dcp::status_list::{self, Status, StatusEntry};

use crate::BoxFuture;
use crate::http::{HttpError, HttpSender};

pub mod cache;

use cache::StatusListCache;

/// The largest status list credential read, in bytes. A list of the
/// minimum size (16 KiB uncompressed) is a few kilobytes even when many
/// bits are set.
pub const MAX_STATUS_LIST: usize = 2 * 1024 * 1024;

/// Port: where a status list credential is fetched, as text (a JWT).
pub trait StatusListSource: Send + Sync {
    /// The answer at `url`, asked for as `application/vc+jwt`, read up
    /// to [`MAX_STATUS_LIST`] bytes.
    fn status_list_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, HttpError>>;
}

/// Adapter: an HTTP GET with the sender's rules (HTTPS only unless
/// allowed for tests, time and size limits, no redirects).
impl StatusListSource for HttpSender {
    fn status_list_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, HttpError>> {
        Box::pin(self.get_text(url, "application/vc+jwt", MAX_STATUS_LIST))
    }
}

/// Why a status list could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusListError {
    /// Its server did not answer, or answered 408, 429 or 5xx: trying
    /// again later may help.
    Unavailable(String),
    /// Anything else: an answer that is not a status list credential, of
    /// another issuer, with a bad signature, or not valid now.
    Rejected(String),
}

impl StatusListError {
    /// Why, for the log.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Unavailable(reason) | Self::Rejected(reason) => reason,
        }
    }
}

/// A status list credential read by [`StatusListReader::list_credential`].
#[derive(Debug, Clone)]
pub struct ReadList {
    /// The list credential, checked.
    pub credential: Arc<Credential>,
    /// True when it was fetched now, false when it came from the cache.
    pub fetched: bool,
}

/// Status list credentials read and checked, with a cache by URL.
#[derive(Debug)]
pub struct StatusListReader {
    cache: StatusListCache,
}

impl StatusListReader {
    /// A reader that keeps each checked list for `cache_seconds` (0: no
    /// cache, every read fetches).
    #[must_use]
    pub fn new(cache_seconds: u64) -> Self {
        Self {
            cache: StatusListCache::new(cache_seconds),
        }
    }

    /// The status list credential at `url` for a credential of
    /// `issuer_did`, signed by the same issuer (`issuer` is its DID
    /// document) and valid at `now` within `leeway` seconds: from the
    /// cache, or fetched from `source`, checked, and then kept in the
    /// cache.
    ///
    /// # Errors
    /// See [`StatusListError`].
    pub async fn list_credential(
        &self,
        source: &dyn StatusListSource,
        issuer_did: &str,
        issuer: &DidDocument,
        url: &str,
        now: u64,
        leeway: u64,
    ) -> Result<ReadList, StatusListError> {
        let another_issuer =
            || StatusListError::Rejected(format!("status list {url} has another issuer"));
        let rejected = |e: &dyn std::fmt::Display| {
            StatusListError::Rejected(format!("status list {url}: {e}"))
        };
        if let Some(cached) = self.cache.get(url, now) {
            // it was checked against its own issuer when it was fetched;
            // another issuer's credential may name the same URL
            if cached.issuer != issuer_did {
                return Err(another_issuer());
            }
            return Ok(ReadList {
                credential: cached,
                fetched: false,
            });
        }
        let text = match source.status_list_text(url).await {
            Ok(text) => text,
            Err(e) if e.is_temporary() => {
                return Err(StatusListError::Unavailable(format!(
                    "status list {url}: {e}"
                )));
            }
            Err(e) => return Err(rejected(&e)),
        };
        let list_credential = Credential::parse(text.trim()).map_err(|e| rejected(&e))?;
        if list_credential.issuer != issuer_did {
            return Err(another_issuer());
        }
        list_credential
            .verify(issuer, now, leeway)
            .map_err(|e| rejected(&e))?;
        // only now, fetched and verified, may it be kept
        let list_credential = Arc::new(list_credential);
        self.cache.keep(url, Arc::clone(&list_credential), now);
        Ok(ReadList {
            credential: list_credential,
            fetched: true,
        })
    }
}

/// Looks `entry` up in `list_credential`, read from the entry's URL:
/// [`Status::Set`] when its bit is set (revoked or suspended, by the
/// entry's `statusPurpose`).
///
/// # Errors
/// The list credential is not a status list for this entry, or the two
/// do not fit (another purpose, an index out of range); the text names
/// the URL.
pub fn entry_status(list_credential: &Credential, entry: &StatusEntry) -> Result<Status, String> {
    let url = &entry.status_list_credential;
    let list = status_list::list_in(list_credential, entry)
        .map_err(|e| format!("status list {url}: {e}"))?;
    status_list::check(entry, &list).map_err(|e| format!("status list {url}: {e}"))
}
