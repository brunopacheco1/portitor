// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Request size limits (R-18): each port's limit, and the fixed limit of
//! the DSP routes whose messages are small by specification. A body one
//! byte over the limit gets 413; a body at the limit is not refused for
//! its size.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::Arc;

use axum::Router;
use axum::http::request::Builder;
use axum::http::{Request, header};
use portitor_connector::http_client::Sender;
use portitor_connector::management::{ApiKey, router as management_router};
use portitor_connector::{Config, Ports};
use portitor_runtime::BoxFuture;
use portitor_runtime::request_size::testing::assert_size_limit;
use portitor_runtime::request_size::{
    DEFAULT_MAX_REQUEST_BYTES, SMALL_MESSAGE_MAX_BYTES, limit_port,
};
use serde_json::Value;

mod common;
use common::{CONSUMER, PROVIDER, authenticating, from};

const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";

/// A management port setting other than the default.
const MANAGEMENT_MAX_BYTES: usize = 100_000;

/// Never reached: every request here is refused or answered locally.
struct NoSender;

impl Sender for NoSender {
    fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Err("not sent in this test".to_owned()) })
    }
}

/// The public and the management port, limited as the binary does.
fn ports() -> (Router, Router) {
    let config = Config {
        participant_id: PROVIDER.into(),
        public_url: "https://provider.example".into(),
        datasets_file: None,
        harvest_interval_seconds: 0,
        allow_http_callbacks: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    };
    let ports = authenticating(Ports::new(Arc::new(NoSender)), PROVIDER);
    let (public, negotiator) = portitor_connector::build(&config, ports).unwrap();
    let management = management_router(negotiator, ApiKey::new(KEY).unwrap());
    (
        limit_port(public, DEFAULT_MAX_REQUEST_BYTES),
        limit_port(management, MANAGEMENT_MAX_BYTES),
    )
}

/// A JSON `POST` to `path` from the consumer, authenticated.
fn dsp_post(path: &str) -> impl Fn() -> Builder + '_ {
    move || {
        let request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        from(request, CONSUMER)
    }
}

#[tokio::test]
async fn the_public_port_takes_its_setting() {
    let (public, _) = ports();
    // a contract request carries an offer with its policy: the port's limit
    let path = "/dsp/2025-1/negotiations/request";
    assert_size_limit(&public, dsp_post(path), DEFAULT_MAX_REQUEST_BYTES).await;
}

#[tokio::test]
async fn small_dsp_messages_have_a_fixed_limit() {
    let (public, _) = ports();
    let paths = [
        "/dsp/2025-1/catalog/request",
        "/dsp/2025-1/negotiations/p1/events",
        "/dsp/2025-1/negotiations/p1/agreement/verification",
        "/dsp/2025-1/negotiations/p1/termination",
        "/dsp/2025-1/transfers/p1/suspension",
        "/dsp/2025-1/transfers/p1/completion",
        "/dsp/2025-1/transfers/p1/termination",
        "/dsp/2025-1/callback/negotiations/c1/events",
        "/dsp/2025-1/callback/negotiations/c1/termination",
        "/dsp/2025-1/callback/transfers/c1/suspension",
        "/dsp/2025-1/callback/transfers/c1/completion",
        "/dsp/2025-1/callback/transfers/c1/termination",
    ];
    for path in paths {
        assert_size_limit(&public, dsp_post(path), SMALL_MESSAGE_MAX_BYTES).await;
    }
}

#[tokio::test]
async fn the_management_port_takes_its_setting() {
    let (_, management) = ports();
    let request = || {
        Request::post("/v1/negotiations")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
    };
    assert_size_limit(&management, request, MANAGEMENT_MAX_BYTES).await;
}
