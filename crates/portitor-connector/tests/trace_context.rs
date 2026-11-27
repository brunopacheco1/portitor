// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! W3C trace context stays inside the participant (ADR 0023, section 4),
//! as the connector is wired: with tracing on (as when export is on), the
//! outbox's DSP messages, the DCP verifier's fetches and the readiness
//! check send no `traceparent`; the client of our own STS does.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, header};
use portitor_connector::Config;
use portitor_connector::auth::dcp::DcpVerifier;
use portitor_connector::auth::sts::{SecureTokenService, StsClient, TokenRequest};
use portitor_connector::auth::{AuthSettings, CallerVerifier as _};
use portitor_connector::health::OwnIdentityCheck;
use portitor_connector::management::{ApiKey, router};
use portitor_runtime::database::DatabaseSetting;
use portitor_runtime::health::Check as _;
use portitor_runtime::telemetry::testing::{
    Recorded, recorded, trace_on_this_thread, traceparent_recorder,
};
use serde_json::json;
use tower::ServiceExt as _;
use tracing::Instrument as _;

mod common;
use common::{CONSUMER, authenticating};

const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";

/// The `did:web` DID served by the recorder at `base`.
fn did_of(base: &str) -> String {
    let host = base.trim_start_matches("http://").replace(':', "%3A");
    format!("did:web:{host}")
}

/// DCP settings that point at the recorder: our DID and our STS.
fn settings(base: &str) -> AuthSettings {
    AuthSettings {
        participant_id: did_of(base),
        sts_url: format!("{base}/sts/token"),
        sts_allow_http: true,
        sts_client_id: "test-only-client".to_owned(),
        sts_client_secret: "test-only-secret".to_owned(),
        trusted_issuers: vec!["did:web:issuer.example".to_owned()],
        gaiax_compliance_issuers: Vec::new(),
        scopes: Vec::new(),
        allow_http: true,
        status_list_cache_seconds: 0,
        did_cache_seconds: 300,
    }
}

/// Waits until the recorder got `count` requests (the outbox sends in the
/// background).
async fn wait_for(recorder: &Recorded, count: usize) {
    for _ in 0..200 {
        if recorded(recorder).len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the recorder got {:?}", recorded(recorder));
}

#[tokio::test]
async fn the_outbox_sends_no_trace_context() {
    let traced = trace_on_this_thread();
    let (base, recorder) = traceparent_recorder().await;
    let config = Config {
        participant_id: CONSUMER.into(),
        public_url: "https://consumer.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http: true,
        database: DatabaseSetting::SqliteFile("unused.db".into()),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    };
    // the sender `main` gives the outbox
    let sender = portitor_connector::dsp_sender(true).unwrap();
    let ports = authenticating(common::ports(Arc::new(sender)), CONSUMER);
    let (_, negotiator) = portitor_connector::build(&config, ports).unwrap();
    let app = router(negotiator, ApiKey::new(KEY).unwrap());
    let body = json!({
        "counterPartyAddress": format!("{base}/dsp/2025-1"),
        "counterPartyId": "did:web:provider.example",
        "offer": {"@id": "d:offer", "@type": "Offer", "target": "d",
                  "permission": [{"action": "use"}]}
    });
    let request = Request::post("/v1/negotiations")
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    // a request span, as the server makes one, that the outbox's span
    // and its call are children of
    let span = tracing::info_span!(target: "portitor_test", "request");
    app.oneshot(request).instrument(span).await.unwrap();
    wait_for(&recorder, 1).await;
    assert_eq!(recorded(&recorder)[0], None);
    // tracing was on: the call had a span
    assert!(
        traced.spans().iter().any(|span| span.name == "POST"),
        "{:?}",
        traced.spans()
    );
}

/// A token whose `iss` is the recorder's DID: the verifier fetches that
/// DID document before it checks anything else.
fn token_from(did: &str) -> String {
    let key = portitor_dcp::SigningKey::generate().unwrap();
    let claims = json!({"iss": did, "sub": did, "aud": "did:web:us.example",
                        "jti": "test-only-jti", "exp": 4_000_000_000_u64, "iat": 1});
    key.sign_json(&format!("{did}#key-1"), "JWT", &claims)
        .unwrap()
}

#[tokio::test]
async fn the_dcp_verifier_sends_no_trace_context() {
    let traced = trace_on_this_thread();
    let (base, recorder) = traceparent_recorder().await;
    let settings = settings(&base);
    let sts: Arc<dyn SecureTokenService> = Arc::new(StsClient::new(&settings).unwrap());
    let used_token_ids = Arc::new(portitor_connector::store::ConnectorStore::in_memory().unwrap());
    let verifier = DcpVerifier::new(&settings, sts, used_token_ids).unwrap();
    let authorization = format!("Bearer {}", token_from(&did_of(&base)));
    let span = tracing::info_span!(target: "portitor_test", "request");
    // the caller is refused (the recorder serves no DID document); what
    // matters is the fetch it made on the way
    let refused = verifier.verify(Some(&authorization)).instrument(span).await;
    assert!(refused.is_err());
    let got = recorded(&recorder);
    assert!(!got.is_empty());
    assert!(got.iter().all(Option::is_none), "{got:?}");
    assert!(traced.spans().iter().any(|span| span.name == "GET"));
}

#[tokio::test]
async fn our_sts_gets_our_trace_context() {
    let _traced = trace_on_this_thread();
    let (base, recorder) = traceparent_recorder().await;
    let sts = StsClient::new(&settings(&base)).unwrap();
    let request = TokenRequest {
        audience: "did:web:other.example",
        scopes: &[],
        access_token: None,
    };
    let span = tracing::info_span!(target: "portitor_test", "request");
    // the recorder's answer has no token: the call itself is what counts
    let _ = sts.token(request).instrument(span).await;
    let got = recorded(&recorder);
    assert_eq!(got.len(), 1);
    assert!(
        got[0].as_deref().is_some_and(|t| t.starts_with("00-")),
        "{got:?}"
    );
}

/// The path and the `traceparent` of each request [`own_identity`] got.
type Seen = Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

/// Our identity as the readiness check sees it: a DID document with a
/// Credential Service at `/.well-known/did.json`, and an STS that answers
/// without a token. Records the path and `traceparent` of each request.
async fn own_identity() -> (String, Seen) {
    use axum::http::{HeaderMap, Uri};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let did = did_of(&base);
    let seen = Seen::default();
    let record = Arc::clone(&seen);
    let app = axum::Router::new().fallback(move |uri: Uri, headers: HeaderMap| {
        let record = Arc::clone(&record);
        let did = did.clone();
        async move {
            let traceparent = headers
                .get("traceparent")
                .map(|value| value.to_str().unwrap().to_owned());
            record
                .lock()
                .unwrap()
                .push((uri.path().to_owned(), traceparent));
            axum::Json(json!({
                "@context": ["https://www.w3.org/ns/did/v1"],
                "id": did,
                "service": [{"id": "#cs", "type": "CredentialService",
                             "serviceEndpoint": "https://cs.example"}]
            }))
        }
    });
    tokio::spawn(async move { axum::serve(listener, app).await });
    (base, seen)
}

#[tokio::test]
async fn the_readiness_check_makes_no_trace() {
    let traced = trace_on_this_thread();
    let (base, seen) = own_identity().await;
    let check = OwnIdentityCheck::new(&settings(&base)).unwrap();
    // even inside a span, as a probe's would be
    let span = tracing::info_span!(target: "portitor_test", "probe");
    check.refresh().instrument(span).await;
    // the STS gave no token: not ready, but both were asked
    assert!(check.check().await.is_err());
    let seen = seen.lock().unwrap().clone();
    let paths: Vec<&str> = seen.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(paths, ["/.well-known/did.json", "/sts/token"], "{seen:?}");
    assert!(
        seen.iter().all(|(_, traceparent)| traceparent.is_none()),
        "{seen:?}"
    );
    // no client spans: only the test's own
    let spans = traced.spans();
    assert!(spans.iter().all(|span| span.name == "probe"), "{spans:?}");
}
