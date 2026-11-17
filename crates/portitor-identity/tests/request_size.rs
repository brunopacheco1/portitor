// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Request size limits (R-18): each port's limit, and the fixed limit of
//! the STS. A body one byte over the limit gets 413; a body at the limit
//! is not refused for its size.
#![allow(clippy::unwrap_used, missing_docs)]

use axum::http::{Request, header};
use portitor_runtime::request_size::testing::assert_size_limit;
use portitor_runtime::request_size::{
    MANAGEMENT_SETTING, PUBLIC_SETTING, RequestLimits, SMALL_MESSAGE_MAX_BYTES,
};
use portitor_runtime::testing::vars;

mod common;
use common::{API_KEY, World};

/// The limits as `main.rs` reads them, with settings other than the
/// default: 100,000 bytes on the public port, 200,000 on the management
/// port.
fn limits() -> RequestLimits {
    let settings = vars(&[(PUBLIC_SETTING, "100000"), (MANAGEMENT_SETTING, "200000")]);
    RequestLimits::from_vars(settings).unwrap()
}

#[tokio::test]
async fn the_public_port_takes_its_setting_and_the_sts_its_fixed_limit() {
    let public = limits().on_public_port(World::new().app);
    // a presentation query can carry a presentation definition
    let query = || {
        Request::post("/cs/presentations/query").header(header::CONTENT_TYPE, "application/json")
    };
    assert_size_limit(&public, query, 100_000).await;
    let sts = || {
        Request::post("/sts/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
    };
    assert_size_limit(&public, sts, SMALL_MESSAGE_MAX_BYTES).await;
}

#[tokio::test]
async fn the_management_port_takes_its_setting() {
    let max_bytes = 200_000;
    let management = limits().on_management_port(World::new().management);
    let request = || {
        Request::post("/v1/credential-requests")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
    };
    assert_size_limit(&management, request, max_bytes).await;
}
