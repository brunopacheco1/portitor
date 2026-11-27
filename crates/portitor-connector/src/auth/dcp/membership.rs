// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Status entries, and a membership checked again (ADR 0027, section
//! 7.2).
//!
//! - [`DcpVerifier::check_status`]: one `credentialStatus` entry of a
//!   credential, looked up in its status list, which comes from the cache
//!   or is fetched and verified (`portitor_runtime::status_lists`). Used by
//!   every credential check of the verifier.
//! - [`DcpVerifier::check_membership_again`]: a `MembershipCredential`
//!   the caller presented when it started a transfer, checked again
//!   without the caller: its issuer is still trusted (for each of its
//!   types, R-48), it has not expired, and none of its status bits is set. The data plane runs it on every
//!   `/data` request, while a download runs, and around a push
//!   ([`CallerVerifier::check_membership`](super::CallerVerifier::check_membership)).
//!   The signature and the holder binding are not checked again: they
//!   cannot change.

use std::time::Duration;

use portitor_dcp::DidDocument;
use portitor_dcp::credential::{Credential, Validity};
use portitor_dcp::status_list::{Status, StatusEntry};
use portitor_runtime::status_lists::{StatusListError, entry_status};

use super::{DcpVerifier, LEEWAY, MEMBERSHIP};
use crate::auth::{DEFAULT_RECHECK, Membership, VerifyError};

/// The shortest time between two checks of a running download: with a
/// status list cache of 0 every check fetches the list, so a running
/// download fetches it at most this often.
pub const MIN_RECHECK: Duration = Duration::from_secs(5);

/// How often a running download checks its membership again, for a
/// status list cache of `cache_seconds`
/// (`PORTITOR_STATUS_LIST_CACHE_SECONDS`): as often as the cache is
/// renewed, but at least once a minute ([`DEFAULT_RECHECK`]) and at most
/// every 5 seconds ([`MIN_RECHECK`]). With the default cache of 300 s, a
/// running download sees a revocation at most 300 + 60 s after it.
#[must_use]
pub fn recheck_interval(cache_seconds: u64) -> Duration {
    Duration::from_secs(cache_seconds).clamp(MIN_RECHECK, DEFAULT_RECHECK)
}

/// What is kept of a verified `MembershipCredential`: its issuer, the end
/// of its validity (the earlier of its stated end and its JWT's `exp`),
/// its status entries and its types.
#[must_use]
pub fn of(credential: &Credential) -> Membership {
    Membership {
        issuer: credential.issuer.clone(),
        valid_until: credential.validity.until,
        status: credential.status.clone(),
        types: credential.types.clone(),
    }
}

impl DcpVerifier {
    /// `membership`, checked again now (module doc).
    ///
    /// # Errors
    ///
    /// Its issuer is no longer trusted, it has expired, or a status bit is
    /// set ([`VerifyError::Rejected`]); or the issuer's DID document or a
    /// status list could not be fetched now ([`VerifyError::Unavailable`]).
    pub(super) async fn check_membership_again(
        &self,
        membership: &Membership,
    ) -> Result<(), VerifyError> {
        let issuer = &membership.issuer;
        // still trusted, for every type of the credential as at the first
        // check: the settings may have changed since
        // (PORTITOR_TRUSTED_ISSUER_TYPES, R-48). A membership kept before
        // 1.4 has no types: `MembershipCredential` then
        let kept_before_1_4 = [MEMBERSHIP.to_owned()];
        let types: &[String] = if membership.types.is_empty() {
            &kept_before_1_4
        } else {
            &membership.types
        };
        let trusted =
            self.trusted_issuers.contains(issuer) && self.issuer_types.allows(issuer, types);
        if !trusted {
            return Err(VerifyError::Rejected(format!(
                "the membership's issuer {issuer} is no longer trusted for the types {} \
                 (PORTITOR_TRUSTED_ISSUERS, PORTITOR_TRUSTED_ISSUER_TYPES)",
                types.join(", ")
            )));
        }
        let now = self.now();
        let validity = Validity {
            from: None,
            until: membership.valid_until,
        };
        // the same leeway as when it was presented
        validity
            .check(now, LEEWAY)
            .map_err(|e| format!("the membership credential from {issuer}: {e}"))?;
        if membership.status.is_empty() {
            return Ok(());
        }
        let issuer_document = self.resolver.resolve(issuer).await?;
        for entry in &membership.status {
            self.check_status(issuer, &issuer_document, entry, now)
                .await?;
        }
        Ok(())
    }

    /// One status entry of a credential issued by `issuer_did`: takes the
    /// status list credential, which must be signed by the same issuer
    /// and valid now (`portitor_runtime::status_lists`, shared with the
    /// identity's status check), and looks up the bit. A list that cannot
    /// be read or checked refuses the credential, unless its server did
    /// not answer or answered 408, 429 or 5xx (a temporary failure).
    pub(super) async fn check_status(
        &self,
        issuer_did: &str,
        issuer_document: &DidDocument,
        entry: &StatusEntry,
        now: u64,
    ) -> Result<(), VerifyError> {
        let url = &entry.status_list_credential;
        let read = self
            .status_lists
            .list_credential(&self.http, issuer_did, issuer_document, url, now, LEEWAY)
            .await;
        let list_credential = match read {
            Ok(read) => {
                if read.fetched {
                    // At DEBUG: with a cache time of 0 this is one line for
                    // each credential checked. The EDC interop test reads
                    // it to see that the list was fetched
                    // (scenarios/edc-interop/run.sh, step 8e).
                    tracing::debug!(%url, issuer = %read.credential.issuer, "status list fetched and verified");
                }
                read.credential
            }
            Err(StatusListError::Unavailable(reason)) => {
                return Err(VerifyError::Unavailable(reason));
            }
            Err(StatusListError::Rejected(reason)) => return Err(VerifyError::Rejected(reason)),
        };
        match entry_status(&list_credential, entry)? {
            Status::Clear => Ok(()),
            Status::Set => Err(VerifyError::Rejected(format!(
                "a credential is revoked or suspended ({})",
                entry.status_purpose
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_download_is_checked_once_a_minute_or_as_often_as_the_cache_renews() {
        assert_eq!(recheck_interval(0), Duration::from_secs(5));
        assert_eq!(recheck_interval(3), Duration::from_secs(5));
        assert_eq!(recheck_interval(30), Duration::from_secs(30));
        assert_eq!(recheck_interval(300), Duration::from_secs(60));
        assert_eq!(recheck_interval(3_600), Duration::from_secs(60));
    }
}
