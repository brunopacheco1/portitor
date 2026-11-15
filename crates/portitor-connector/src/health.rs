// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The connector's readiness checks (ADR 0023, section 1), served on the
//! management port by `portitor_runtime::health`. Only the connector's
//! own dependencies are checked, never another participant, so a
//! partner's outage never makes a healthy connector look broken:
//!
//! - `database`: the `SQLite` file answers;
//! - `harvest`: the start harvest ran (the catalog is loaded);
//! - `bundle`: a bundle version is in force, when a bundle is configured;
//! - `identity`: our DID document resolves and our STS signs for it (the
//!   start check, run again at most every [`IDENTITY_RECHECK`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use portitor_runtime::BoxFuture;
use portitor_runtime::health::{Check, HealthChecks, blocking, sqlite_file_answers};

use crate::Negotiator;
use crate::auth::AuthSettings;
use crate::auth::dcp::DcpVerifier;
use crate::auth::sts::{SecureTokenService, StsClient};
use crate::bundle_harvest::BundleHarvester;

/// How long the result of the identity check is reused: the STS is asked
/// at most once in this time, however often the probes come.
pub const IDENTITY_RECHECK: Duration = Duration::from_secs(30);

/// How long the identity check may take (DID document and STS token).
/// Below `portitor_runtime::health::CHECK_TIMEOUT`, so that a slow answer
/// is recorded, and reused, as a failure.
pub const IDENTITY_TIMEOUT: Duration = Duration::from_millis(400);

/// Check: our DID document resolves and our STS signs for it, as at
/// start ([`DcpVerifier::check_own_identity`]). The result is kept for
/// [`IDENTITY_RECHECK`].
pub struct OwnIdentityCheck {
    verifier: DcpVerifier,
    /// The last result and when it was made. A tokio `Mutex`, held while
    /// the check runs, so that probes arriving together wait for one run
    /// instead of each asking the STS.
    last: tokio::sync::Mutex<Option<Checked>>,
    recheck: Duration,
    timeout: Duration,
}

impl std::fmt::Debug for OwnIdentityCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnIdentityCheck")
            .field("verifier", &self.verifier)
            .finish_non_exhaustive()
    }
}

/// One result of [`OwnIdentityCheck`].
struct Checked {
    at: Instant,
    outcome: Result<(), String>,
}

impl OwnIdentityCheck {
    /// The check for the DCP `settings`, with its own STS client.
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn new(settings: &AuthSettings) -> Result<Self, String> {
        let sts: Arc<dyn SecureTokenService> = Arc::new(StsClient::new(settings)?);
        Self::with_sts(settings, sts)
    }

    /// The check for `settings`, getting tokens from `sts` (tests).
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn with_sts(
        settings: &AuthSettings,
        sts: Arc<dyn SecureTokenService>,
    ) -> Result<Self, String> {
        Ok(Self {
            verifier: DcpVerifier::for_readiness(settings, sts)?,
            last: tokio::sync::Mutex::new(None),
            recheck: IDENTITY_RECHECK,
            timeout: IDENTITY_TIMEOUT,
        })
    }

    /// The last result if it is recent, otherwise a new one.
    async fn outcome(&self) -> Result<(), String> {
        let mut last = self.last.lock().await;
        if let Some(checked) = last.as_ref()
            && checked.at.elapsed() < self.recheck
        {
            return checked.outcome.clone();
        }
        let run = tokio::time::timeout(self.timeout, self.verifier.check_own_identity_once());
        let outcome = match run.await {
            Ok(outcome) => outcome,
            Err(_) => Err(format!(
                "DID document and STS: no answer within {} ms",
                self.timeout.as_millis()
            )),
        };
        *last = Some(Checked {
            at: Instant::now(),
            outcome: outcome.clone(),
        });
        outcome
    }
}

impl Check for OwnIdentityCheck {
    fn check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(self.outcome())
    }
}

/// What the connector's readiness checks look at.
#[derive(Debug)]
pub struct Dependencies {
    /// The `SQLite` file (`PORTITOR_DB_FILE`).
    pub db_file: PathBuf,
    /// The handle that knows whether the start harvest ran.
    pub negotiator: Negotiator,
    /// The bundle harvester, when a bundle is configured.
    pub bundle: Option<Arc<BundleHarvester>>,
    /// The identity check, when DCP is configured (always, except in the
    /// TCK build without `PORTITOR_STS_URL`).
    pub identity: Option<Arc<OwnIdentityCheck>>,
}

/// The connector's readiness checks, in the order of the module doc.
#[must_use]
pub fn readiness_checks(dependencies: Dependencies) -> HealthChecks {
    let mut checks = HealthChecks::new();
    let db_file = dependencies.db_file;
    checks.add("database", blocking(move || sqlite_file_answers(&db_file)));
    let negotiator = dependencies.negotiator;
    checks.add(
        "harvest",
        blocking(move || {
            if negotiator.harvest_started() {
                Ok(())
            } else {
                Err("the start harvest has not run yet".to_owned())
            }
        }),
    );
    if let Some(bundle) = dependencies.bundle {
        checks.add(
            "bundle",
            blocking(move || {
                if bundle.in_use().version.is_empty() {
                    Err("no bundle version is in force".to_owned())
                } else {
                    Ok(())
                }
            }),
        );
    }
    if let Some(identity) = dependencies.identity {
        // `as _`: the pointer to the check becomes the `Arc<dyn Check>`
        // the list holds
        checks.add("identity", identity as _);
    }
    checks
}
