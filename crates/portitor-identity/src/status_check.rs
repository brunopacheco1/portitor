// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The credential status check (TD-20, from 1.4; maintainer decision D6,
//! 2026-11-27): every `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS` (default
//! 60, 0 off), the identity reads the status lists of the credentials it
//! holds and records what they say, so that the presentation API leaves
//! out a revoked or suspended credential ([`crate::presentation`]).
//! Without it, a verifier refuses the whole presentation that holds one
//! revoked credential (DCP v1.0.1, 5.4.3, step 8), and the operator had
//! to delete the credential by hand. Eclipse EDC's `IdentityHub` does the
//! same with its `CredentialWatchdog`
//! (`edc.iam.credential.status.check.period`).
//!
//! For each held credential with `credentialStatus` entries:
//!
//! 1. its issuer's DID document is read ([`Directory`]);
//! 2. each status list credential is read and checked as the connector
//!    checks it (`portitor_runtime::status_lists`: signed by the
//!    credential's issuer, valid now, a `BitstringStatusList` or
//!    `StatusList2021` credential), through the partner HTTP sender (the
//!    URL is the issuer's choice);
//! 3. a set bit in a `revocation` list makes it [`HeldStatus::Revoked`],
//!    in another list (`suspension`) [`HeldStatus::Suspended`]; no bit
//!    set makes it [`HeldStatus::Valid`] again, so a lifted suspension
//!    shows it again;
//! 4. the result is recorded with the time, for that credential's JWT
//!    only ([`IdentityStore::record_credential_status`]).
//!
//! A credential whose lists cannot be read keeps its last recorded
//! status, and a warning is logged: an outage of an issuer's status list
//! must not cut a member off. A credential never checked is presented.
//!
//! Several replicas: one round runs at a time, under the lease
//! [`STATUS_CHECK_LEASE`] in the identity's database, as the connector's
//! dataset harvest (ADR 0029, decision 6; TD-56). After a round, the
//! lease is kept until a second before the next tick, so no other
//! replica runs one within the period. A round that runs twice (a lease
//! lost during a stall) only records the same answers again.
//!
//! The lists are cached for a little less than the period
//! ([`cache_seconds`]): credentials of one issuer share a list, which is
//! then read once per round, and each round reads it fresh.

use std::sync::Arc;
use std::time::Duration;

use portitor_dcp::credential::Credential;
use portitor_dcp::status_list::Status;
use portitor_runtime::directory::Directory;
use portitor_runtime::leases::{LeaseStore, Leases};
use portitor_runtime::panics::spawn_every;
use portitor_runtime::status_lists::{
    StatusListReader, StatusListSource, cache::MAX_SECONDS, entry_status,
};
use portitor_runtime::system::now_secs;

use crate::store::{CheckedStatus, HeldStatus, IdentityStore, StoredCredential};
use crate::{Identity, LEEWAY};

/// The kind and the id of the lease of the status check in the
/// database (`portitor_runtime::leases`).
pub const STATUS_CHECK_LEASE: (&str, &str) = ("job", "credential_status_check");

/// The `statusPurpose` whose set bit means "revoked for good".
pub const REVOCATION: &str = "revocation";

/// The most status entries read from one credential, as the connector
/// (one revocation and one suspension list is usual). A credential with
/// more is not checked: it keeps its last status.
pub const MAX_STATUS_ENTRIES: usize = 4;

/// The status check of one identity process: its period, its reader of
/// status lists with their cache, where the lists are fetched, and the
/// leases of this process.
pub struct StatusCheck {
    every: Duration,
    reader: StatusListReader,
    source: Arc<dyn StatusListSource>,
    leases: Leases,
}

impl std::fmt::Debug for StatusCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusCheck")
            .field("every", &self.every)
            .finish_non_exhaustive()
    }
}

impl StatusCheck {
    /// The check every `every_seconds`, fetching lists from `source` and
    /// keeping its lease in `leases` (the identity's database). `None`
    /// for 0: the check is off.
    #[must_use]
    pub fn new(
        every_seconds: u64,
        source: Arc<dyn StatusListSource>,
        leases: Arc<dyn LeaseStore>,
    ) -> Option<Self> {
        if every_seconds == 0 {
            return None;
        }
        Some(Self {
            every: Duration::from_secs(every_seconds),
            reader: StatusListReader::new(cache_seconds(every_seconds)),
            source,
            leases: Leases::new(leases, portitor_runtime::system::process_id()),
        })
    }

    /// The period.
    #[must_use]
    pub fn every(&self) -> Duration {
        self.every
    }
}

/// How long a status list is kept in the cache for a check every
/// `every_seconds`: a second less, so the next round reads it fresh, at
/// most the cache's own limit; 0 (no cache) for a check every second.
#[must_use]
pub fn cache_seconds(every_seconds: u64) -> u64 {
    every_seconds.saturating_sub(1).min(MAX_SECONDS)
}

/// What one round found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoundCounts {
    /// Credentials whose status was recorded as valid (no bit set, or no
    /// status entry).
    pub valid: usize,
    /// Credentials recorded as revoked.
    pub revoked: usize,
    /// Credentials recorded as suspended.
    pub suspended: usize,
    /// Credentials whose lists could not be read: their last status is
    /// kept.
    pub unreadable: usize,
}

/// Starts the check in the background, every period, the first round
/// one period after now ([`spawn_every`]). `None` when the check is off.
/// Must be called inside the Tokio runtime; aborting the handle stops it.
#[must_use]
pub fn schedule(identity: &Arc<Identity>) -> Option<tokio::task::JoinHandle<()>> {
    let every = identity.status_check.as_ref()?.every;
    let identity = Arc::clone(identity);
    // one round; `spawn_every` runs it on the timer, supervised
    let round = move || {
        let identity = Arc::clone(&identity);
        async move {
            // the counts are logged by the round itself
            let _counts = run_if_due(&identity).await;
        }
    };
    Some(spawn_every("credential_status_check", every, round))
}

/// One tick: a round, if this process can take the lease. `None` when the
/// check is off, another replica holds the lease in this period, or the
/// store failed (logged): the next tick tries again. After the round, the
/// lease is kept until a second before this process's next tick.
pub async fn run_if_due(identity: &Identity) -> Option<RoundCounts> {
    let check = identity.status_check.as_ref()?;
    let ticked = check.leases.now();
    let (kind, id) = STATUS_CHECK_LEASE;
    let Some(lease) = check.leases.claim(kind, id).taken() else {
        tracing::debug!("credential status check: another process runs it in this period");
        return None;
    };
    let counts = round(identity, check, now_secs()).await;
    let next_due = ticked
        .saturating_add(check.every.as_secs())
        .saturating_sub(1);
    lease.hold_until(next_due);
    Some(counts)
}

/// Checks every held credential once (module doc) at `now`.
async fn round(identity: &Identity, check: &StatusCheck, now: u64) -> RoundCounts {
    let mut counts = RoundCounts::default();
    let credentials = match identity.store.credentials() {
        Ok(credentials) => credentials,
        Err(error) => {
            tracing::warn!(%error, "credential status check: cannot read the credentials");
            return counts;
        }
    };
    for credential in &credentials {
        let state = match status_of(identity.directory.as_ref(), check, credential, now).await {
            Ok(state) => state,
            Err(reason) => {
                tracing::warn!(
                    credential = %credential.id,
                    issuer = %credential.issuer,
                    %reason,
                    "credential status cannot be read; its last known status is kept"
                );
                counts.unreadable += 1;
                continue;
            }
        };
        record(identity.store.as_ref(), credential, state, now);
        match state {
            HeldStatus::Valid => counts.valid += 1,
            HeldStatus::Revoked => counts.revoked += 1,
            HeldStatus::Suspended => counts.suspended += 1,
        }
    }
    if counts.revoked + counts.suspended + counts.unreadable > 0 {
        tracing::info!(
            valid = counts.valid,
            revoked = counts.revoked,
            suspended = counts.suspended,
            unreadable = counts.unreadable,
            "credential status check"
        );
    }
    counts
}

/// Records `state` for `credential`, and logs a change. A store failure
/// is logged; the next round records it again.
fn record(store: &dyn IdentityStore, credential: &StoredCredential, state: HeldStatus, now: u64) {
    let before = credential.status.map(|checked| checked.state);
    let checked = CheckedStatus {
        state,
        checked_at: now,
    };
    match store.record_credential_status(&credential.id, &credential.jwt, checked) {
        Ok(true) => {
            // a first check that finds it valid is no news
            let changed = match before {
                Some(before) => before != state,
                None => state != HeldStatus::Valid,
            };
            if changed {
                tracing::info!(
                    credential = %credential.id,
                    issuer = %credential.issuer,
                    status = state.text(),
                    "credential status changed"
                );
            }
        }
        // replaced or deleted meanwhile: the next round checks the new one
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, credential = %credential.id, "cannot record a credential status");
        }
    }
}

/// What the status lists of `credential` say at `now`.
///
/// # Errors
/// Why they cannot be read: the credential, its issuer's DID document or
/// a list cannot be read or checked.
async fn status_of(
    directory: &dyn Directory,
    check: &StatusCheck,
    credential: &StoredCredential,
    now: u64,
) -> Result<HeldStatus, String> {
    let parsed = Credential::parse(&credential.jwt).map_err(|e| format!("credential: {e}"))?;
    if parsed.status.is_empty() {
        return Ok(HeldStatus::Valid);
    }
    if parsed.status.len() > MAX_STATUS_ENTRIES {
        return Err(format!(
            "the credential has more than {MAX_STATUS_ENTRIES} status entries"
        ));
    }
    let issuer = directory.did_document(&parsed.issuer).await?;
    let mut state = HeldStatus::Valid;
    for entry in &parsed.status {
        let url = &entry.status_list_credential;
        let read = check
            .reader
            .list_credential(
                check.source.as_ref(),
                &parsed.issuer,
                &issuer,
                url,
                now,
                LEEWAY,
            )
            .await
            .map_err(|e| e.reason().to_owned())?;
        match entry_status(&read.credential, entry)? {
            Status::Clear => {}
            Status::Set if entry.status_purpose == REVOCATION => return Ok(HeldStatus::Revoked),
            Status::Set => state = HeldStatus::Suspended,
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_is_kept_a_second_less_than_the_period() {
        assert_eq!(cache_seconds(1), 0);
        assert_eq!(cache_seconds(2), 1);
        assert_eq!(cache_seconds(60), 59);
        assert_eq!(cache_seconds(86_400), MAX_SECONDS);
    }
}
