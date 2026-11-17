// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A key change with the old public key listed as retired
//! (`PORTITOR_IDENTITY_RETIRED_KEY_FILES`, TD-05): what the old key
//! signed keeps verifying while it is listed, and stops verifying once
//! the operator removes it from the list.
#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::*;
use portitor_dcp::credential::Presentation;
use portitor_dcp::x509::p256_public_key_pem;
use portitor_dcp::{
    DidDocument, JtiCache, Jws, SigningKey, unverified_claims, validate_self_issued,
};
use portitor_identity::config::Config;
use portitor_identity::store::{IdentityStore, SqliteStore};
use portitor_runtime::system::{new_uuid, now_secs};
use serde_json::json;

#[tokio::test]
async fn a_key_change_with_the_old_key_retired_leaves_no_gap() {
    let dir = TempDir::new();
    let old_key = SigningKey::generate().unwrap();
    let new_key = SigningKey::generate().unwrap();

    // before the change: the identity signs an STS token and a
    // presentation with the old key
    let before = start(&config(), &old_key);
    let old_kid = before.identity.kid().to_owned();
    let (status, answer) = before.sts_token(VERIFIER, None).await;
    assert_eq!(status, StatusCode::OK);
    let old_token = answer["access_token"].as_str().unwrap().to_owned();
    let old_presentation = presentation(&before).await;
    for signed in [&old_token, &old_presentation] {
        assert_eq!(Jws::parse(signed).unwrap().kid(), Some(old_kid.as_str()));
    }

    // after: the new key signs, the old public key is listed as retired
    let retired_file = dir.write("old-key.pub.pem", &public_pem(&old_key));
    let after = start(&with_retired(vec![retired_file]), &new_key);
    assert_ne!(after.identity.kid(), old_kid);
    assert_eq!(
        after.identity.retired_kids(),
        std::slice::from_ref(&old_kid)
    );
    let document = did_document(&after).await;
    assert!(token_verifies(&old_token, &document));
    assert!(presentation_verifies(&old_presentation, &document));
    // what the new key signs verifies too, and names the new key
    let (_, answer) = after.sts_token(VERIFIER, None).await;
    let new_token = answer["access_token"].as_str().unwrap().to_owned();
    assert_eq!(
        Jws::parse(&new_token).unwrap().kid(),
        Some(after.identity.kid())
    );
    assert!(token_verifies(&new_token, &document));
    let new_presentation = presentation(&after).await;
    assert!(presentation_verifies(&new_presentation, &document));

    // the operator removes the old key from the list: what it signed no
    // longer verifies; what the new key signed still does
    let removed = start(&config(), &new_key);
    let document = did_document(&removed).await;
    assert!(!token_verifies(&old_token, &document));
    assert!(!presentation_verifies(&old_presentation, &document));
    assert!(token_verifies(&new_token, &document));
    assert!(presentation_verifies(&new_presentation, &document));
}

/// The component checks its own access tokens too: one for a verifier
/// (presentation API, valid 5 minutes) and one for an issuer (Storage
/// API, valid 30 days), both signed with the old key before the change.
#[tokio::test]
async fn access_tokens_signed_before_a_key_change_stay_usable_while_listed() {
    let dir = TempDir::new();
    let old_key = SigningKey::generate().unwrap();
    let new_key = SigningKey::generate().unwrap();
    let store: Arc<dyn IdentityStore> = Arc::new(SqliteStore::in_memory().unwrap());

    // before the change: an access token for the verifier, and two
    // credential requests whose access tokens the issuer will send back
    let before = start_on(&config(), &old_key, &store);
    let (_, answer) = before.sts_token(VERIFIER, Some(MEMBERSHIP_SCOPE)).await;
    let verifier_access = World::access_token_of(&answer);
    let first = ask_issuer(&before).await;
    let second = ask_issuer(&before).await;

    // after: the old key is retired, the tokens it signed still work
    let retired_file = dir.write("old-key.pub.pem", &public_pem(&old_key));
    let after = start_on(&with_retired(vec![retired_file]), &new_key, &store);
    assert_eq!(query_with(&after, &verifier_access).await, StatusCode::OK);
    assert_eq!(deliver_with(&after, &first).await, StatusCode::OK);

    // the old key removed from the list: they are refused
    let removed = start_on(&config(), &new_key, &store);
    assert_eq!(
        query_with(&removed, &verifier_access).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        deliver_with(&removed, &second).await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn the_identity_does_not_start_with_a_wrong_retired_key_file() {
    let dir = TempDir::new();
    let key = SigningKey::generate().unwrap();
    let old = SigningKey::generate().unwrap();
    let current_file = dir.write("current.pub.pem", &public_pem(&key));
    let old_file = dir.write("old.pub.pem", &public_pem(&old));
    let private_file = dir.write(
        "old-private.pem",
        include_str!("fixtures/test-only-identity-key.pem"),
    );
    let refusals = [
        (vec![current_file], "is the current key"),
        (vec![old_file.clone(), old_file], "listed twice"),
        (vec![private_file], "a private key"),
        (vec![dir.0.join("missing.pem")], "cannot read"),
    ];
    for (files, reason) in refusals {
        let error = World::try_with(
            &with_retired(files),
            Arc::new(SqliteStore::in_memory().unwrap()),
            SigningKey::from_pkcs8(key.pkcs8()).unwrap(),
            None,
        )
        .err()
        .unwrap();
        assert!(
            error.starts_with("PORTITOR_IDENTITY_RETIRED_KEY_FILES: "),
            "{error}"
        );
        assert!(error.contains(reason), "{error}");
    }
}

/// The identity component with `config`, signing with a copy of `key`.
fn start(config: &Config, key: &SigningKey) -> World {
    World::try_with(
        config,
        Arc::new(SqliteStore::in_memory().unwrap()),
        SigningKey::from_pkcs8(key.pkcs8()).unwrap(),
        None,
    )
    .unwrap()
}

/// The identity component with `config` and `store`, signing with a
/// copy of `key`; its issuer serves metadata for `membership-vc11`.
fn start_on(config: &Config, key: &SigningKey, store: &Arc<dyn IdentityStore>) -> World {
    let world = World::try_with(
        config,
        Arc::clone(store),
        SigningKey::from_pkcs8(key.pkcs8()).unwrap(),
        None,
    )
    .unwrap();
    world.directory.json.lock().unwrap().insert(
        "https://issuer.example/api/metadata".to_owned(),
        json!({
            "@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
            "type": "IssuerMetadata",
            "issuer": ISSUER,
            "credentialsSupported": [
                {"id": "membership-vc11", "type": "CredentialObject",
                 "credentialType": "MembershipCredential", "profile": "vc11-sl2021/jwt"}
            ]
        }),
    );
    *world.issuer_service.location.lock().unwrap() = Some("/api/requests/i-1".to_owned());
    world
}

/// A credential request sent to the issuer: its `holderPid` and the
/// access token the issuer must send back with the delivery.
struct SentRequest {
    holder_pid: String,
    access_token: String,
}

/// Asks the issuer for a membership credential through the management
/// API.
async fn ask_issuer(world: &World) -> SentRequest {
    let (status, created) = call(
        &world.management,
        Request::post("/v1/credential-requests")
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"issuerDid": ISSUER, "credentialIds": ["membership-vc11"]}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let sent = world.issuer_service.requests().pop().unwrap();
    SentRequest {
        holder_pid: created["holderPid"].as_str().unwrap().to_owned(),
        access_token: unverified_claims(&sent.id_token).unwrap().token.unwrap(),
    }
}

/// The issuer delivers a credential for `request`, sending its access
/// token back; returns the Storage API's answer.
async fn deliver_with(world: &World, request: &SentRequest) -> StatusCode {
    let jwt = world.issuer.vc11("MembershipCredential", HOLDER);
    let message = credential_message(
        Some(&request.holder_pid),
        "ISSUED",
        &[("vc11-sl2021/jwt", "MembershipCredential", &jwt)],
    );
    let id_token = world.issuer.id_token(HOLDER, Some(&request.access_token));
    world.send_delivery(&id_token, &message).await.0
}

/// The verifier queries the presentation API with `access_token`.
async fn query_with(world: &World, access_token: &str) -> StatusCode {
    let id_token = world.verifier.id_token(HOLDER, Some(access_token));
    let query = scope_query(&[MEMBERSHIP_SCOPE]);
    let request = post_json("/cs/presentations/query", Some(&id_token), &query);
    call(&world.app, request).await.0
}

/// The test configuration with `files` as the retired key files.
fn with_retired(files: Vec<PathBuf>) -> Config {
    Config {
        retired_key_files: files,
        ..config()
    }
}

/// The public key of `key`, as `openssl pkey -pubout` writes it.
fn public_pem(key: &SigningKey) -> String {
    p256_public_key_pem(&key.public_jwk()).unwrap()
}

/// A presentation of a delivered membership credential, signed by the
/// identity for the verifier.
async fn presentation(world: &World) -> String {
    let pid = world.request_credentials().await;
    let membership = world.issuer.vc11("MembershipCredential", HOLDER);
    let (status, _) = world
        .deliver(
            &pid,
            &[("vc11-sl2021/jwt", "MembershipCredential", &membership)],
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = world
        .query(MEMBERSHIP_SCOPE, &scope_query(&[MEMBERSHIP_SCOPE]))
        .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["presentation"][0].as_str().unwrap().to_owned()
}

/// Whether the STS token `token` passes the DCP checks with `document`.
fn token_verifies(token: &str, document: &DidDocument) -> bool {
    validate_self_issued(
        token,
        document,
        VERIFIER,
        now_secs(),
        60,
        &mut JtiCache::default(),
    )
    .is_ok()
}

/// Whether the presentation `jwt` verifies with `document`.
fn presentation_verifies(jwt: &str, document: &DidDocument) -> bool {
    let presentation = Presentation::parse(jwt).unwrap();
    presentation
        .verify(document, VERIFIER, now_secs(), 60)
        .is_ok()
}

/// The DID document the identity serves.
async fn did_document(world: &World) -> DidDocument {
    let (status, json) = call(
        &world.app,
        Request::get("/.well-known/did.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_value(json).unwrap()
}

/// A temporary directory, removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "portitor-identity-keys-{}",
            new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes `text` to the file `name` in the directory.
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
