// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the two catalog calls answer: the provider's JSON as it came, or
//! the problem that says why there is none (400 for a request that was
//! not sent, 502 for a provider or an identity component that did not
//! give what was needed).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use portitor_runtime::api_key::problem;

use crate::catalog_reader::{MAX_CATALOG_ANSWER, ProviderAnswer, ReadFailure};
use crate::http_client::HttpError;

/// The answer for a provider's catalog or dataset: its JSON as it came
/// and its `Link` headers (catalog pages), or the problem that says why
/// there is none.
pub(super) fn provider_answer(read: Result<ProviderAnswer, ReadFailure>) -> Response {
    let answer = match read {
        Ok(answer) => answer,
        Err(failure) => return read_failed(&failure),
    };
    let mut response = ([(header::CONTENT_TYPE, "application/json")], answer.body).into_response();
    for link in &answer.links {
        // a value that cannot be a header (it came as one, so it can) is
        // left out
        if let Ok(value) = header::HeaderValue::from_str(link) {
            response.headers_mut().append(header::LINK, value);
        }
    }
    response
}

/// 400 for a request that was not sent; 502 (Bad Gateway, as Eclipse EDC
/// answers: `BaseCatalogApiController.java`) when our identity component
/// or the provider did not give what was needed.
fn read_failed(failure: &ReadFailure) -> Response {
    match failure {
        ReadFailure::BadRequest(reason) => problem(StatusCode::BAD_REQUEST, reason),
        ReadFailure::NoToken => problem(
            StatusCode::BAD_GATEWAY,
            "no token for the provider from the identity component (STS); the connector's log says why",
        ),
        ReadFailure::Provider(error) => problem(StatusCode::BAD_GATEWAY, &provider_error(error)),
        ReadFailure::NotDsp(reason) => problem(StatusCode::BAD_GATEWAY, reason),
    }
}

/// What to tell the caller about a provider that gave no usable answer.
fn provider_error(error: &HttpError) -> String {
    match error {
        HttpError::Status(_) | HttpError::StatusWithBody(_, _) => {
            format!("the provider answered {error}")
        }
        HttpError::NoAnswer(reason) => format!("the provider did not answer: {reason}"),
        HttpError::NotSent(reason) => format!("the request was not sent: {reason}"),
        HttpError::BadAnswer(reason) => format!(
            "the provider's answer cannot be used ({reason}); at most {MAX_CATALOG_ANSWER} bytes are read"
        ),
    }
}
