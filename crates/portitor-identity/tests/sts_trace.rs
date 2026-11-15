// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The STS joins the connector's trace only after the client is
//! authenticated (ADR 0023, section 4): the public port is open to
//! anyone, and a trace id from an unknown caller is untrusted input.
#![allow(clippy::unwrap_used, missing_docs)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::*;
use portitor_runtime::telemetry::http::observe_requests;
use portitor_runtime::telemetry::testing::trace_on_this_thread;
use tower::ServiceExt as _;

/// The caller's trace and span ids in [`TRACEPARENT`].
const CALLER_TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const CALLER_SPAN: &str = "b7ad6b7169203331";
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

async fn token_request(app: &axum::Router, secret: &str) -> StatusCode {
    let form = format!(
        "grant_type=client_credentials&client_id={}&client_secret={secret}&audience={}",
        urlencode(HOLDER),
        urlencode(VERIFIER)
    );
    let request = Request::post(portitor_identity::STS_ROUTE)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("traceparent", TRACEPARENT)
        .body(Body::from(form))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn the_sts_joins_the_callers_trace_only_once_authenticated() {
    let traced = trace_on_this_thread();
    let world = World::new();
    // as `main` serves it: the STS route takes the caller's trace context
    let app = observe_requests(world.app.clone(), &[portitor_identity::STS_ROUTE]);

    // a wrong secret: refused, and a trace of its own
    assert_eq!(
        token_request(&app, "test-only-wrong-secret").await,
        StatusCode::UNAUTHORIZED
    );
    let spans = traced.spans();
    let refused = spans
        .iter()
        .find(|span| span.name == "POST /sts/token")
        .unwrap();
    assert_ne!(refused.trace_id, CALLER_TRACE, "{spans:?}");
    assert_ne!(refused.parent_span_id, CALLER_SPAN);

    // the right secret: the request joins the caller's trace
    assert_eq!(token_request(&app, STS_SECRET).await, StatusCode::OK);
    let spans = traced.spans();
    let joined: Vec<_> = spans
        .iter()
        .filter(|span| span.name == "POST /sts/token")
        .collect();
    assert_eq!(joined.len(), 2, "{spans:?}");
    assert_eq!(joined[1].trace_id, CALLER_TRACE);
    assert_eq!(joined[1].parent_span_id, CALLER_SPAN);
}
