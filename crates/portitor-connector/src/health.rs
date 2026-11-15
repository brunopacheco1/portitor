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
//!   start check, run again in the background every
//!   [`IDENTITY_REFRESH`]; see [`OwnIdentityCheck`]).
//!
//! `harvest` and `bundle` are start gates: the connector serves only after
//! the start harvest ran and, with a bundle configured, a version is in
//! force (`main.rs` stops otherwise), and neither goes back later. So
//! while the connector serves, they cannot fail; they are listed so that
//! the answer shows them, and in case the start order changes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use portitor_runtime::BoxFuture;
use portitor_runtime::health::{Check, HealthChecks, blocking, sqlite_file_answers};
use tokio::task::JoinHandle;
use tracing::Instrument as _;

use crate::Negotiator;
use crate::auth::AuthSettings;
use crate::auth::dcp::DcpVerifier;
use crate::auth::sts::{SecureTokenService, StsClient};
use crate::bundle_harvest::BundleHarvester;

/// How often the background task checks our identity again. The probes
/// only read its last result, so however often they come, the STS is
/// asked once in this time.
pub const IDENTITY_REFRESH: Duration = Duration::from_secs(10);

/// How long one identity check may take (DID document and STS token).
/// Generous: it runs in the background, not while a probe waits.
pub const IDENTITY_TIMEOUT: Duration = Duration::from_secs(3);

/// How many identity checks in a row must fail before readiness says
/// "not ready": with [`IDENTITY_REFRESH`], after about 20 s. One slow or
/// lost answer does not make the connector look broken. One success makes
/// it ready again.
pub const IDENTITY_FAILURES_TO_FAIL: u32 = 2;

/// Check: our DID document resolves and our STS signs for it, as at
/// start ([`DcpVerifier::check_own_identity`]).
///
/// The check itself runs in a background task
/// ([`OwnIdentityCheck::spawn_refresh`]) every [`IDENTITY_REFRESH`], each
/// run cut off after [`IDENTITY_TIMEOUT`]; a probe only reads the last
/// results. Ready after a success, until [`IDENTITY_FAILURES_TO_FAIL`]
/// checks in a row failed. Not ready before the first success.
///
/// The background runs make no spans, no measures and send no
/// `traceparent` (ADR 0023): they are not part of any request.
pub struct OwnIdentityCheck {
    /// One run of the real check.
    source: Arc<dyn Check>,
    /// What the runs so far found. A `std` lock: it is held only to read
    /// or update a few fields, never across an `.await`.
    status: Mutex<IdentityStatus>,
    timeout: Duration,
}

impl std::fmt::Debug for OwnIdentityCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnIdentityCheck")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// What the identity checks found so far.
#[derive(Debug, Default)]
struct IdentityStatus {
    /// Whether a check has passed since the process started.
    passed_once: bool,
    /// Checks that failed since the last one that passed.
    failures_in_a_row: u32,
    /// Why the last check failed.
    last_failure: Option<String>,
}

impl IdentityStatus {
    /// Takes in the result of one check.
    fn record(&mut self, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => {
                self.passed_once = true;
                self.failures_in_a_row = 0;
                self.last_failure = None;
            }
            Err(reason) => {
                self.failures_in_a_row = self.failures_in_a_row.saturating_add(1);
                self.last_failure = Some(reason);
            }
        }
    }

    /// The readiness answer (see [`OwnIdentityCheck`]).
    fn readiness(&self) -> Result<(), String> {
        let failing = self.failures_in_a_row >= IDENTITY_FAILURES_TO_FAIL;
        if self.passed_once && !failing {
            return Ok(());
        }
        match &self.last_failure {
            Some(reason) => Err(reason.clone()),
            None => Err("the identity has not been checked yet".to_owned()),
        }
    }
}

/// The real check: [`DcpVerifier::check_own_identity_once`].
struct VerifierCheck(DcpVerifier);

impl Check for VerifierCheck {
    fn check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(self.0.check_own_identity_once())
    }
}

impl OwnIdentityCheck {
    /// The check for the DCP `settings`, with its own STS client (without
    /// spans, measures or trace context).
    ///
    /// # Errors
    ///
    /// The HTTP client cannot be built.
    pub fn new(settings: &AuthSettings) -> Result<Self, String> {
        let sts: Arc<dyn SecureTokenService> = Arc::new(StsClient::new(settings)?.unobserved());
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
        let verifier = DcpVerifier::for_readiness(settings, sts)?;
        Ok(Self::from_source(
            Arc::new(VerifierCheck(verifier)),
            IDENTITY_TIMEOUT,
        ))
    }

    /// The check with `source` as the real check, each run cut off after
    /// `timeout`.
    fn from_source(source: Arc<dyn Check>, timeout: Duration) -> Self {
        Self {
            source,
            status: Mutex::new(IdentityStatus::default()),
            timeout,
        }
    }

    /// Runs the real check once, within the timeout, and records the
    /// result.
    pub async fn refresh(&self) {
        let outcome = match tokio::time::timeout(self.timeout, self.source.check()).await {
            Ok(outcome) => outcome,
            Err(_) => Err(format!(
                "DID document and STS: no answer within {} s",
                self.timeout.as_secs()
            )),
        };
        self.lock_status().record(outcome);
    }

    /// Starts the background task: [`OwnIdentityCheck::refresh`] now, then
    /// every [`IDENTITY_REFRESH`], until the process stops.
    pub fn spawn_refresh(self: &Arc<Self>) -> JoinHandle<()> {
        let check = Arc::clone(self);
        let refreshing = async move {
            loop {
                check.refresh().await;
                tokio::time::sleep(IDENTITY_REFRESH).await;
            }
        };
        // no span: these runs belong to no request and start no trace
        tokio::spawn(refreshing.instrument(tracing::Span::none()))
    }

    /// The status, also after a panic while it was held (the fields are
    /// always consistent: each update is one assignment per field).
    fn lock_status(&self) -> std::sync::MutexGuard<'_, IdentityStatus> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Check for OwnIdentityCheck {
    fn check(&self) -> BoxFuture<'_, Result<(), String>> {
        let answer = self.lock_status().readiness();
        Box::pin(async move { answer })
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A check that gives the outcomes of its list in turn, then never
    /// answers; counts its runs.
    struct Scripted {
        outcomes: Mutex<VecDeque<Result<(), String>>>,
        runs: AtomicUsize,
    }

    impl Scripted {
        fn new(outcomes: Vec<Result<(), String>>) -> Arc<Self> {
            Arc::new(Self {
                outcomes: Mutex::new(outcomes.into()),
                runs: AtomicUsize::new(0),
            })
        }
    }

    impl Check for Scripted {
        fn check(&self) -> BoxFuture<'_, Result<(), String>> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            match self.outcomes.lock().unwrap().pop_front() {
                Some(outcome) => Box::pin(async move { outcome }),
                None => Box::pin(std::future::pending()),
            }
        }
    }

    fn down() -> Result<(), String> {
        Err("STS: connection refused".to_owned())
    }

    fn check_with(source: Arc<Scripted>) -> Arc<OwnIdentityCheck> {
        Arc::new(OwnIdentityCheck::from_source(source, IDENTITY_TIMEOUT))
    }

    #[tokio::test(start_paused = true)]
    async fn not_ready_before_the_first_result_and_after_a_timeout() {
        let check = check_with(Scripted::new(vec![]));
        let err = check.check().await.unwrap_err();
        assert_eq!(err, "the identity has not been checked yet");
        // the source never answers: the run gives up after the timeout
        // (tokio's test clock jumps ahead when nothing else can run)
        let started = tokio::time::Instant::now();
        check.refresh().await;
        assert_eq!(started.elapsed(), IDENTITY_TIMEOUT);
        let err = check.check().await.unwrap_err();
        assert_eq!(err, "DID document and STS: no answer within 3 s");
    }

    #[tokio::test]
    async fn two_failures_in_a_row_make_it_not_ready_and_one_success_ready() {
        let check = check_with(Scripted::new(vec![
            Ok(()),
            down(),
            Ok(()),
            down(),
            down(),
            Ok(()),
        ]));
        let mut seen = Vec::new();
        for _ in 0..6 {
            check.refresh().await;
            seen.push(check.check().await.is_ok());
        }
        assert_eq!(seen, [true, true, true, true, false, true]);
    }

    #[tokio::test]
    async fn failures_before_any_success_are_not_ready() {
        let check = check_with(Scripted::new(vec![down()]));
        check.refresh().await;
        assert_eq!(check.check().await.unwrap_err(), "STS: connection refused");
    }

    #[tokio::test(start_paused = true)]
    async fn the_background_task_refreshes_every_ten_seconds() {
        let source = Scripted::new(vec![Ok(()), down(), down(), Ok(())]);
        let check = check_with(Arc::clone(&source));
        let task = check.spawn_refresh();
        // the first run is at once
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(source.runs.load(Ordering::SeqCst), 1);
        assert_eq!(check.check().await, Ok(()));
        // 10 s: one failure, still ready
        tokio::time::sleep(IDENTITY_REFRESH).await;
        assert_eq!(source.runs.load(Ordering::SeqCst), 2);
        assert_eq!(check.check().await, Ok(()));
        // 20 s: the second failure in a row
        tokio::time::sleep(IDENTITY_REFRESH).await;
        assert_eq!(source.runs.load(Ordering::SeqCst), 3);
        assert!(check.check().await.is_err());
        // 30 s: one success, ready again
        tokio::time::sleep(IDENTITY_REFRESH).await;
        assert_eq!(source.runs.load(Ordering::SeqCst), 4);
        assert_eq!(check.check().await, Ok(()));
        task.abort();
    }
}
