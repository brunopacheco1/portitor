// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The transfers this connector runs as provider (ADR 0027, section 7.1):
//! list them, read one, and end one. Until 1.1 the provider had no
//! management route for its own transfers.
//!
//! A termination here is the provider's own step (ADR 0014, "The
//! provider's own steps"): the data flow is terminated at once, which
//! cuts every download of it that runs, and the DSP termination is sent
//! to the consumer in the background. The view never shows the data
//! plane's token or its hash: the provider keeps only the hash, and it
//! is no one's business.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use portitor_dsp::transfer::provider::{MAX_PAGE, Transfer};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{FORBIDDEN, next_after, problem, state_name, transfer_failed};
use crate::Negotiator;
use portitor_runtime::api_key::Problem;

/// A transfer this connector runs as provider.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvidedTransferView {
    /// Our process id; also the id of its data flow (`/data/{providerPid}`).
    provider_pid: String,
    /// The consumer's process id.
    consumer_pid: String,
    /// The DSP state, for example `REQUESTED` or `STARTED`.
    #[schema(example = "STARTED")]
    state: String,
    /// Where our messages to the consumer go (its `callbackAddress`).
    counter_party_address: String,
    /// The consumer's participant id (its DID), proven by its DCP token.
    #[schema(example = "did:web:consumer.example")]
    counter_party_id: String,
    /// The agreement the transfer is made under.
    agreement_id: String,
    /// The dataset whose data is transferred.
    dataset_id: String,
    /// The format the consumer asked for.
    #[schema(example = "HttpData-PULL")]
    format: String,
    /// Whether a message of ours is on its way to the consumer (for
    /// example the termination, until the consumer acknowledges it).
    message_on_its_way: bool,
}

impl From<Transfer> for ProvidedTransferView {
    fn from(transfer: Transfer) -> Self {
        Self {
            state: state_name(transfer.state),
            message_on_its_way: transfer.pending.is_some(),
            provider_pid: transfer.provider_pid,
            consumer_pid: transfer.consumer_pid,
            counter_party_address: transfer.callback_address,
            counter_party_id: transfer.counter_party,
            agreement_id: transfer.agreement_id,
            dataset_id: transfer.dataset_id,
            format: transfer.format,
        }
    }
}

/// One page of transfers as provider.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvidedTransferPage {
    /// The transfers, in provider pid order.
    transfers: Vec<ProvidedTransferView>,
    /// Pass as `after` to get the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
}

/// Paging for the transfers as provider.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProvidedPageQuery {
    /// Return transfers whose provider pid sorts after this one.
    after: Option<String>,
    /// How many to return, 1 to 100 (default 100).
    limit: Option<usize>,
}

/// The transfers this connector runs as provider, a page at a time.
#[utoipa::path(
    get,
    path = "/v1/provided-transfers",
    params(ProvidedPageQuery),
    responses(
        (status = 200, description = "One page", body = ProvidedTransferPage),
        (status = 400, description = "Invalid query", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Provided transfers"
)]
pub(super) async fn list(
    State(negotiator): State<Negotiator>,
    query: Result<Query<ProvidedPageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let limit = q.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE);
    match negotiator.provided_transfers(q.after.as_deref(), limit) {
        Ok(found) => {
            let next = next_after(&found, limit, |transfer| &transfer.provider_pid);
            Json(ProvidedTransferPage {
                transfers: found.into_iter().map(ProvidedTransferView::from).collect(),
                next,
            })
            .into_response()
        }
        Err(f) => transfer_failed(&f),
    }
}

/// One transfer this connector runs as provider.
#[utoipa::path(
    get,
    path = "/v1/provided-transfers/{providerPid}",
    params(("providerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 200, description = "The transfer", body = ProvidedTransferView),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown transfer", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Provided transfers"
)]
pub(super) async fn one(State(negotiator): State<Negotiator>, Path(pid): Path<String>) -> Response {
    match negotiator.provided_transfer(&pid) {
        Ok(transfer) => Json(ProvidedTransferView::from(transfer)).into_response(),
        Err(f) => transfer_failed(&f),
    }
}

/// End a transfer this connector runs as provider. Access ends at once:
/// the data flow is terminated and every download of it that runs is
/// cut. The DSP termination is sent to the consumer in the background;
/// the state becomes `TERMINATED` once the consumer acknowledges it, or
/// when it cannot be reached. Asked again while the termination is on
/// its way, it answers the same and sends nothing more.
#[utoipa::path(
    post,
    path = "/v1/provided-transfers/{providerPid}/termination",
    params(("providerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 202, description = "Access ended; termination on its way", body = ProvidedTransferView),
        (status = 400, description = "The transfer has ended", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown transfer", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Provided transfers"
)]
pub(super) async fn terminate(
    State(negotiator): State<Negotiator>,
    Path(pid): Path<String>,
) -> Response {
    match negotiator.terminate_provided_transfer(&pid) {
        Ok(transfer) => {
            tracing::info!(transfer = %pid, "transfer terminated by the provider's operator");
            let view = ProvidedTransferView::from(transfer);
            (StatusCode::ACCEPTED, Json(view)).into_response()
        }
        Err(f) => transfer_failed(&f),
    }
}
