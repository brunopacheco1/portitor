// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The two management calls that read from a provider, as consumer:
//! `POST /v1/catalog/request` (its catalog) and
//! `POST /v1/catalog/dataset/request` (one dataset). Each handler reads
//! the body, checks it, lets the connector ask the provider
//! ([`crate::catalog_reader`]) and hands the answer on.
//!
//! - `request`: the two request bodies and the checks of their members.
//! - `answer`: the answer to the caller, or the problem (400 or 502).
//!
//! The limits of the call to the provider (size, time, how many at
//! once) and the checks of its answer are in [`crate::catalog_reader`].

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::Response;

use super::{FORBIDDEN, Problem, problem};
use crate::Negotiator;

mod answer;
mod request;

use answer::provider_answer;
pub use request::{CatalogRequest, DSP_HTTP_2025_1, DatasetRequest};
use request::{check_kind_and_protocol, check_scopes};

/// Read a provider's catalog (DSP 2025-1 `Catalog`): the datasets it
/// offers, each with its offers (`hasPolicy`). The connector asks the
/// provider with its own DCP token and waits for the answer (one try, at
/// most 30 seconds); nothing is stored.
///
/// The answer is the provider's JSON as it came. At most 16 MiB are
/// read. The catalog's `participantId` must be the `counterPartyId` of
/// the request: the catalog of another participant, or one without a
/// `participantId` (DSP 2025-1 requires it), is answered with 502 and
/// not handed on. No setting turns this check off. The two ids are
/// compared as written, with one exception: in a `did:web` DID the
/// escaped colon before a port may be `%3A` or `%3a`. If the provider sends its catalog in pages (DSP 2025-1, "Catalog
/// HTTPS Binding", "Pagination"), this is the first page only, and the
/// provider's `Link` headers are copied to the answer.
#[utoipa::path(
    post,
    path = "/v1/catalog/request",
    request_body = CatalogRequest,
    responses(
        (status = 200, description = "The provider's catalog (DSP 2025-1 Catalog), as the provider sent it", body = Object, content_type = "application/json",
            headers(("Link" = String, description = "The provider's Link headers, when it sent its catalog in pages; only this first page is returned"))),
        (status = 400, description = "Invalid request, a protocol other than dataspace-protocol-http:2025-1, a querySpec, or an address that is not an https URL", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "No catalog: the provider refused (the detail has its HTTP status), did not answer, answered with more than 16 MiB or with something that is not a Catalog, answered with a Catalog whose participantId is not the counterPartyId of the request (or that has none), or the identity component gave no token", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Catalog"
)]
pub(super) async fn request_catalog(
    State(negotiator): State<Negotiator>,
    body: Result<Json<CatalogRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(request)) => request,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let checked = check_kind_and_protocol(
        request.kind.as_deref(),
        "CatalogRequest",
        request.protocol.as_deref(),
        request.profile.as_deref(),
    );
    if let Err(reason) = checked {
        return problem(StatusCode::BAD_REQUEST, &reason);
    }
    if request.query_spec.is_some() {
        return problem(
            StatusCode::BAD_REQUEST,
            "querySpec is not supported: the whole catalog is always asked for; leave it out",
        );
    }
    if let Err(reason) = check_scopes(&request.additional_scopes) {
        return problem(StatusCode::BAD_REQUEST, &reason);
    }
    let read = negotiator.read_catalog(
        &request.counter_party_address,
        &request.counter_party_id,
        &request.additional_scopes,
    );
    provider_answer(read.await)
}

/// Read one dataset from a provider (DSP 2025-1 `Dataset`), by its
/// `@id`. Works as `POST /v1/catalog/request` does: the connector's own
/// DCP token, one try, the provider's JSON as it came.
#[utoipa::path(
    post,
    path = "/v1/catalog/dataset/request",
    request_body = DatasetRequest,
    responses(
        (status = 200, description = "The dataset (DSP 2025-1 Dataset), as the provider sent it", body = Object, content_type = "application/json"),
        (status = 400, description = "Invalid request, a protocol other than dataspace-protocol-http:2025-1, a dataset id that is \".\" or \"..\", or an address that is not an https URL", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 502, description = "No dataset: the provider refused (the detail has its HTTP status; 404 for a dataset it does not have or does not show us), did not answer, answered with something that is not a JSON object or whose @type is not Dataset (a dataset without @type is accepted), or the identity component gave no token", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Catalog"
)]
pub(super) async fn request_dataset(
    State(negotiator): State<Negotiator>,
    body: Result<Json<DatasetRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(request)) => request,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let checked = check_kind_and_protocol(
        request.kind.as_deref(),
        "DatasetRequest",
        request.protocol.as_deref(),
        request.profile.as_deref(),
    );
    if let Err(reason) = checked {
        return problem(StatusCode::BAD_REQUEST, &reason);
    }
    let read = negotiator.read_dataset(
        &request.counter_party_address,
        &request.counter_party_id,
        &request.id,
    );
    provider_answer(read.await)
}
