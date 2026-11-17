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
    DEFAULT_MAX_REQUEST_BYTES, SMALL_MESSAGE_MAX_BYTES, limit_port,
};

mod common;
use common::{API_KEY, World};

#[tokio::test]
async fn the_public_port_takes_its_setting_and_the_sts_its_fixed_limit() {
    let public = limit_port(World::new().app, DEFAULT_MAX_REQUEST_BYTES);
    // a presentation query can carry a presentation definition
    let query = || {
        Request::post("/cs/presentations/query").header(header::CONTENT_TYPE, "application/json")
    };
    assert_size_limit(&public, query, DEFAULT_MAX_REQUEST_BYTES).await;
    let sts = || {
        Request::post("/sts/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
    };
    assert_size_limit(&public, sts, SMALL_MESSAGE_MAX_BYTES).await;
}

#[tokio::test]
async fn the_management_port_takes_its_setting() {
    let max_bytes = 100_000;
    let management = limit_port(World::new().management, max_bytes);
    let request = || {
        Request::post("/v1/credential-requests")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
    };
    assert_size_limit(&management, request, max_bytes).await;
}
