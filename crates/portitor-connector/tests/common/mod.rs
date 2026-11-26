// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Test doubles for caller authentication, shared by the tests: a
//! verifier that takes `Bearer <did>` at its word, and a prover that
//! sends our own DID that way. Real DCP checks are tested in the
//! connector's `auth::dcp` tests.
//!
//! Every caller the test verifier lets in holds one membership of
//! [`ISSUER`], so the data plane, which checks it again (ADR 0027), serves
//! it. [`Revocable`] lets a test revoke it.
// each test binary uses only part of this module
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::request::Builder;
use portitor_connector::Ports;
use portitor_connector::auth::{Caller, CallerVerifier, Membership, Prover, VerifyError};
use portitor_connector::http_client::Sender;
use portitor_connector::store::ConnectorStore;
use portitor_dsp::policy::HeldCredential;
use portitor_runtime::BoxFuture;
use serde_json::Value;

pub mod fake_s3;
pub mod network;

/// The connector's ports, with every store in a new in-memory database.
pub fn ports(sender: Arc<dyn Sender>) -> Ports {
    Ports::new(sender, &Arc::new(ConnectorStore::in_memory().unwrap()))
}

/// The consumer's DID in the tests.
pub const CONSUMER: &str = "did:web:consumer.example";
/// The provider's DID in the tests.
pub const PROVIDER: &str = "did:web:provider.example";

/// Sends nothing: for tests where every request is answered locally.
pub struct NoSender;

impl Sender for NoSender {
    fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("not sent in this test".to_owned()) })
    }
}

/// The issuer of every test caller's membership.
pub const ISSUER: &str = "did:web:issuer.example";

/// The membership every test caller holds.
#[must_use]
pub fn membership() -> Membership {
    Membership {
        issuer: ISSUER.to_owned(),
        valid_until: None,
        status: Vec::new(),
    }
}

/// The caller `Bearer <did>` names, with [`membership`].
fn named(authorization: Option<&str>) -> Result<Caller, VerifyError> {
    let did = authorization.and_then(|value| value.strip_prefix("Bearer "));
    match did {
        Some(did) => Ok(Caller::new(did).with_memberships(vec![membership()])),
        None => Err(VerifyError::from("no caller")),
    }
}

/// Believes `Bearer <did>`: the caller is `<did>`, a member for good.
pub struct NamedCaller;

impl CallerVerifier for NamedCaller {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Caller, VerifyError>> {
        let caller = named(authorization);
        Box::pin(async move { caller })
    }

    /// No credentials: the tests' offers have no credential constraints.
    fn credentials<'a>(
        &'a self,
        caller: &'a Caller,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
        Box::pin(async move { Ok(caller.credentials.clone()) })
    }

    fn check_membership<'a>(&'a self, _: &'a Membership) -> BoxFuture<'a, Result<(), VerifyError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Like [`NamedCaller`], but the membership can be revoked, and a running
/// download checks it again every `recheck`. While revoked, a DSP request
/// is refused as from a caller whose token is valid but whose credentials
/// are not ([`VerifyError::NotAMember`], TD-47). While `unreachable` is
/// set, a membership check cannot finish ([`VerifyError::Unavailable`]),
/// as when a status list does not answer.
pub struct Revocable {
    pub revoked: Arc<AtomicBool>,
    pub unreachable: Arc<AtomicBool>,
    pub recheck: Duration,
}

impl Revocable {
    /// A member, checked again every `recheck`; and the switch that
    /// revokes it.
    #[must_use]
    pub fn new(recheck: Duration) -> (Self, Arc<AtomicBool>) {
        let revoked = Arc::new(AtomicBool::new(false));
        let verifier = Self {
            revoked: Arc::clone(&revoked),
            unreachable: Arc::new(AtomicBool::new(false)),
            recheck,
        };
        (verifier, revoked)
    }

    fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::SeqCst)
    }
}

impl CallerVerifier for Revocable {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Caller, VerifyError>> {
        let caller = match named(authorization) {
            Ok(caller) if self.is_revoked() => Err(VerifyError::NotAMember {
                caller: caller.id,
                reason: "a credential is revoked or suspended (revocation)".to_owned(),
            }),
            other => other,
        };
        Box::pin(async move { caller })
    }

    fn credentials<'a>(
        &'a self,
        caller: &'a Caller,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<HeldCredential>, String>> {
        Box::pin(async move { Ok(caller.credentials.clone()) })
    }

    fn check_membership<'a>(&'a self, _: &'a Membership) -> BoxFuture<'a, Result<(), VerifyError>> {
        let answer = if self.unreachable.load(Ordering::SeqCst) {
            Err(VerifyError::Unavailable(
                "the status list did not answer".to_owned(),
            ))
        } else if self.is_revoked() {
            Err(VerifyError::from(
                "a credential is revoked or suspended (revocation)",
            ))
        } else {
            Ok(())
        };
        Box::pin(async move { answer })
    }

    fn membership_recheck_interval(&self) -> Duration {
        self.recheck
    }
}

/// The audience of every token a [`NameSelf`] gave, in order.
pub type Audiences = Arc<Mutex<Vec<String>>>;

/// Proves `did` by naming it: `Bearer <did>`, whatever the audience. Each
/// call stands for a fresh token, and its audience is recorded.
pub struct NameSelf {
    pub did: String,
    pub audiences: Audiences,
}

impl Prover for NameSelf {
    fn authorization<'a>(
        &'a self,
        audience: &'a str,
        _: &'a [String],
    ) -> BoxFuture<'a, Result<String, String>> {
        self.audiences.lock().unwrap().push(audience.to_owned());
        Box::pin(async move { Ok(bearer(&self.did)) })
    }
}

/// The `Authorization` value that names `did` as the caller.
#[must_use]
pub fn bearer(did: &str) -> String {
    format!("Bearer {did}")
}

/// `ports` with the test verifier, and the test prover for `participant_id`.
#[must_use]
pub fn authenticating(ports: Ports, participant_id: &str) -> Ports {
    recording(ports, participant_id).0
}

/// Like [`authenticating`], and the audiences of the tokens the prover
/// gives.
#[must_use]
pub fn recording(mut ports: Ports, participant_id: &str) -> (Ports, Audiences) {
    let audiences = Audiences::default();
    ports.verifier = Arc::new(NamedCaller);
    ports.prover = Arc::new(NameSelf {
        did: participant_id.to_owned(),
        audiences: Arc::clone(&audiences),
    });
    (ports, audiences)
}

/// `request` sent by `did`.
#[must_use]
pub fn from(request: Builder, did: &str) -> Builder {
    request.header("authorization", bearer(did))
}
