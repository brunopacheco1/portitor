// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The management API: what the operator's own systems use to read a
//! provider's catalog, to start and follow negotiations and transfers as
//! consumer, and to read the dataset harvest jobs (read only, ADR 0020). It is not part of DSP, so it is served
//! on its own port (`PORTITOR_MANAGEMENT_BIND`), never on the DSP port, and
//! every call needs the API key (`Authorization: Bearer <key>`).
//!
//! Its `OpenAPI` description is generated from the handlers below (utoipa)
//! and served at `GET /v1/openapi.json`; `docs/api/management-openapi.json`
//! is a copy that a test keeps in step.
//!
//! Every protected route documents HTTP 403 ([`FORBIDDEN`]), although
//! nothing answers 403 today: ADR 0025 reserves it for scoped access in
//! 1.x, so that it will not be a new status code then.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use portitor_dsp::Offer;
use portitor_dsp::negotiation::consumer::{ConsumerNegotiation, MAX_PAGE};
use portitor_dsp::negotiation::provider::Failure;
use portitor_dsp::transfer::consumer::{ConsumerTransfer, ConsumerTransferDecision};
use portitor_dsp::transfer::provider::Failure as TransferFailure;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, OpenApi, ToSchema};

use crate::Negotiator;
use crate::catalog_reader::{MAX_CATALOG_ANSWER, ProviderAnswer, ReadFailure};
use crate::harvest::{EntryError, HarvestJob};
use crate::http_client::HttpError;
use crate::system::xsd_date_time;
pub use portitor_runtime::api_key::{ApiKey, MIN_KEY_LEN, Problem, problem, require_key};
use portitor_runtime::api_key::{BearerKey, internal_error};
use portitor_runtime::health;

/// Where the management port listens unless `PORTITOR_MANAGEMENT_BIND`
/// says otherwise: on this machine only.
pub const DEFAULT_MANAGEMENT_BIND: &str = "127.0.0.1:8081";

/// Where the management port listens, and the API key: the settings
/// shared by the three components, read with
/// `ManagementConfig::from_env(DEFAULT_MANAGEMENT_BIND)`.
pub use portitor_runtime::management::ManagementConfig;

/// The description of HTTP 403 on every protected route (ADR 0025, "What
/// must be settled before the 0.13 freeze", point 2). Nothing answers 403
/// today; it is documented now so that scoped access in 1.x adds no new
/// status code.
const FORBIDDEN: &str = "Reserved for scoped access in 1.x (ADR 0025): the caller is known but may not make this call. Not returned today. The API key (PORTITOR_MANAGEMENT_API_KEY_FILE) keeps every right and never gets this answer.";

/// 400 with the DSP error's reasons as the detail.
fn refused(reason: &[Value]) -> Response {
    let reasons: Vec<&str> = reason.iter().filter_map(Value::as_str).collect();
    problem(StatusCode::BAD_REQUEST, &reasons.join("; "))
}

fn failed(failure: &Failure) -> Response {
    match failure {
        Failure::BadRequest(e) => refused(&e.reason),
        Failure::NotFound(_) => problem(StatusCode::NOT_FOUND, "no such negotiation"),
        Failure::Store(e) => internal_error("negotiation store", e),
    }
}

fn transfer_failed(failure: &TransferFailure) -> Response {
    match failure {
        TransferFailure::BadRequest(e) => refused(&e.reason),
        TransferFailure::NotFound(_) => problem(StatusCode::NOT_FOUND, "no such transfer"),
        TransferFailure::Store(e) => internal_error("transfer store", e),
        TransferFailure::DataPlane(e) => internal_error("data plane", e),
    }
}

/// The one value of `protocol` (or `profile`) this connector takes: DSP
/// 2025-1 over HTTPS, as Eclipse EDC names it (`dataspace-protocol-http`
/// in `HttpMessageProtocol.java`, then `:` and the profile, in
/// `DataspaceProfileContext.java`; EDC v1.0.0-RC8).
pub const DSP_HTTP_2025_1: &str = "dataspace-protocol-http:2025-1";

/// Ask a provider for its catalog.
///
/// The body is the `CatalogRequest` of Eclipse EDC's management API v4
/// (EDC v1.0.0-RC8: `catalog-request-schema.json` in
/// `management-api-schema-validator`, `CatalogRequest.java` and
/// `CatalogRequestValidator.java`), so a client written for EDC can send
/// the same body. What Portitor does with each member:
///
/// - `counterPartyAddress`, `counterPartyId`: required, as in EDC. A
///   negotiation (`POST /v1/negotiations`) names the provider with the same
///   two members.
/// - `protocol` or `profile` (EDC's newer name for it): exactly one is
///   required, as in EDC's schema, and it must be
///   `dataspace-protocol-http:2025-1`, the only protocol this
///   connector speaks. Any other value is refused with 400.
/// - `additionalScopes`: optional, honoured as in EDC: the scopes are
///   added to the configured ones in the token for the provider.
/// - `querySpec`: refused with 400 when present. EDC sends it to the
///   provider as the `filter` of the `CatalogRequestMessage`; DSP 2025-1
///   leaves the content of `filter` to each implementation, and a
///   provider that does not support it must refuse the request, as
///   Portitor's own provider side does. Portitor always asks for the
///   whole catalog.
/// - `@context`: optional and not read. EDC requires it (its bodies are
///   JSON-LD); Portitor reads the member names as written.
/// - `@type`: optional; when present it must be `CatalogRequest`.
/// - Any other member (EDC's schema also lists `@id`, which EDC does
///   not use) is refused with 400, as on every management call.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogRequest {
    /// Accepted so that a body written for Eclipse EDC works unchanged;
    /// not read.
    #[serde(rename = "@context")]
    #[schema(value_type = Option<Vec<String>>, example = json!(["https://w3id.org/edc/connector/management/v2"]))]
    // `IgnoredAny` takes any JSON value and keeps nothing of it; the
    // leading `_` says the field is never read
    _context: Option<serde::de::IgnoredAny>,
    /// `CatalogRequest`, when present.
    #[serde(rename = "@type")]
    #[schema(example = "CatalogRequest")]
    kind: Option<String>,
    /// The provider's DSP endpoint, for example
    /// `https://provider.example/dsp/2025-1`: the `counterPartyAddress`
    /// of a negotiation. The catalog is read at
    /// `<counterPartyAddress>/catalog/request`.
    #[schema(example = "https://provider.example/dsp/2025-1")]
    counter_party_address: String,
    /// The provider's participant id: a `did:web` DID, the
    /// `counterPartyId` of a negotiation. Our token is made for it.
    #[schema(example = "did:web:provider.example")]
    counter_party_id: String,
    /// `dataspace-protocol-http:2025-1`. Exactly one of `protocol` and
    /// `profile` is required.
    #[schema(example = "dataspace-protocol-http:2025-1")]
    protocol: Option<String>,
    /// Eclipse EDC's newer name for `protocol`; the same value.
    profile: Option<String>,
    /// Scopes the provider may read from our Credential Service with our
    /// token, on top of the configured ones (`PORTITOR_DCP_SCOPES`): for
    /// datasets a provider shows only to holders of a credential.
    #[serde(default)]
    #[schema(example = json!(["org.eclipse.dspace.dcp.vc.type:MembershipCredential:read"]))]
    additional_scopes: Vec<String>,
    /// Not supported: a request with a `querySpec` is refused with 400.
    /// The whole catalog is always asked for.
    #[schema(value_type = Option<Object>)]
    // only whether it is there matters (`null` reads as left out)
    query_spec: Option<serde::de::IgnoredAny>,
}

/// Ask a provider for one dataset.
///
/// The body is the `DatasetRequest` of Eclipse EDC's management API v4
/// (EDC v1.0.0-RC8: `dataset-request-schema.json`, `DatasetRequest.java`
/// and `DatasetRequestValidator.java`). `@id` (the dataset's id),
/// `counterPartyAddress` and `counterPartyId` are required, and exactly
/// one of `protocol` and `profile`; all are read as in
/// [`CatalogRequest`]. EDC's `DatasetRequest` has no `querySpec` and no
/// `additionalScopes`, so neither is taken here.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetRequest {
    /// Accepted so that a body written for Eclipse EDC works unchanged;
    /// not read.
    #[serde(rename = "@context")]
    #[schema(value_type = Option<Vec<String>>, example = json!(["https://w3id.org/edc/connector/management/v2"]))]
    // `IgnoredAny` takes any JSON value and keeps nothing of it; the
    // leading `_` says the field is never read
    _context: Option<serde::de::IgnoredAny>,
    /// `DatasetRequest`, when present.
    #[serde(rename = "@type")]
    #[schema(example = "DatasetRequest")]
    kind: Option<String>,
    /// The dataset's `@id` in the provider's catalog.
    #[serde(rename = "@id")]
    #[schema(example = "urn:uuid:dataset-1")]
    id: String,
    /// The provider's DSP endpoint. The dataset is read at
    /// `<counterPartyAddress>/catalog/datasets/<@id>`.
    #[schema(example = "https://provider.example/dsp/2025-1")]
    counter_party_address: String,
    /// The provider's participant id: a `did:web` DID.
    #[schema(example = "did:web:provider.example")]
    counter_party_id: String,
    /// `dataspace-protocol-http:2025-1`. Exactly one of `protocol` and
    /// `profile` is required.
    #[schema(example = "dataspace-protocol-http:2025-1")]
    protocol: Option<String>,
    /// Eclipse EDC's newer name for `protocol`; the same value.
    profile: Option<String>,
}

/// Checks the members that say what kind of request this is: `@type`,
/// if present, is `expected_type`, and exactly one of `protocol` and
/// `profile` is there and names DSP 2025-1 over HTTPS.
///
/// # Errors
///
/// What is wrong, for the 400 answer.
fn check_kind_and_protocol(
    kind: Option<&str>,
    expected_type: &str,
    protocol: Option<&str>,
    profile: Option<&str>,
) -> Result<(), String> {
    if let Some(kind) = kind
        && kind != expected_type
    {
        return Err(format!("@type must be {expected_type}"));
    }
    let named = match (protocol, profile) {
        (Some(protocol), None) => protocol,
        (None, Some(profile)) => profile,
        (Some(_), Some(_)) => return Err("give protocol or profile, not both".to_owned()),
        (None, None) => {
            return Err(format!(
                "protocol (or profile) is required: {DSP_HTTP_2025_1}"
            ));
        }
    };
    if named == DSP_HTTP_2025_1 {
        Ok(())
    } else {
        Err(format!(
            "protocol {named} is not supported; this connector speaks {DSP_HTTP_2025_1} only"
        ))
    }
}

/// Checks `additionalScopes`: a scope is one word, as the scopes go to
/// our STS in one field, separated by spaces (`bearer_access_scope`,
/// `auth/sts.rs`).
fn check_scopes(scopes: &[String]) -> Result<(), String> {
    for scope in scopes {
        if scope.is_empty() || scope.contains(char::is_whitespace) {
            return Err("each of additionalScopes must be one word, without spaces".to_owned());
        }
    }
    Ok(())
}

/// The answer for a provider's catalog or dataset: its JSON as it came
/// and its `Link` headers (catalog pages), or the problem that says why
/// there is none.
fn provider_answer(read: Result<ProviderAnswer, ReadFailure>) -> Response {
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

/// Read a provider's catalog (DSP 2025-1 `Catalog`): the datasets it
/// offers, each with its offers (`hasPolicy`). The connector asks the
/// provider with its own DCP token and waits for the answer (one try, at
/// most 30 seconds); nothing is stored.
///
/// The answer is the provider's JSON as it came. At most 16 MiB are
/// read. If the provider sends its catalog in pages (DSP 2025-1, "Catalog
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
        (status = 502, description = "No catalog: the provider refused (the detail has its HTTP status), did not answer, answered with more than 16 MiB or with something that is not a Catalog, or the identity component gave no token", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Catalog"
)]
async fn request_catalog(
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
async fn request_dataset(
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

/// Start a negotiation as consumer.
///
/// The other party is named as in the catalog calls and as Eclipse EDC
/// names it: `counterPartyAddress` and `counterPartyId` (before 0.13:
/// `providerAddress` and `providerId`, which are now refused as unknown
/// members). EDC's `ContractRequest` and `TransferRequest` carry only
/// `counterPartyAddress` (EDC v1.0.0-RC8, `ContractRequest.java`,
/// `TransferRequest.java`); `counterPartyId` is the name of EDC's
/// `CatalogRequest` and of its `ContractNegotiation` view, and Portitor
/// requires it here, because its token is made for that participant.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartNegotiation {
    /// The provider's DSP endpoint, for example
    /// `https://provider.example/dsp/2025-1` (the catalog's
    /// `DataService` `endpointURL`).
    #[schema(example = "https://provider.example/dsp/2025-1")]
    counter_party_address: String,
    /// The provider's participant id (the catalog's `participantId`): a
    /// `did:web` DID. Our DSP requests are tokens for it, and only it may
    /// answer.
    #[schema(example = "did:web:provider.example")]
    counter_party_id: String,
    /// The dataset id. Optional if the offer has a `target`; offers in a
    /// catalog have none, as the dataset holds them. An empty one is
    /// refused.
    #[schema(example = "urn:uuid:dataset-1")]
    dataset_id: Option<String>,
    /// The offer from the provider's catalog (a DSP 2025-1 `Offer`), copied
    /// unchanged from the dataset's `hasPolicy`.
    #[schema(value_type = Object, example = json!({
        "@id": "urn:uuid:offer-1", "@type": "Offer", "permission": [{"action": "use"}]
    }))]
    offer: Offer,
}

/// A negotiation this connector runs as consumer.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationView {
    /// Our process id.
    consumer_pid: String,
    /// The provider's process id, once it has answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_pid: Option<String>,
    /// The DSP state, for example `REQUESTED` or `FINALIZED`.
    #[schema(example = "FINALIZED")]
    state: String,
    /// The other party's DSP endpoint: the provider's, as this connector
    /// is the consumer of every negotiation it lists.
    counter_party_address: String,
    /// The other party's participant id: the provider's.
    counter_party_id: String,
    /// The latest offer (DSP 2025-1 `Offer`).
    #[schema(value_type = Object)]
    offer: Value,
    /// The agreement (DSP 2025-1 `Agreement`), once the provider sent it.
    /// Valid for use once `state` is `FINALIZED`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    agreement: Option<Value>,
}

impl From<ConsumerNegotiation> for NegotiationView {
    fn from(negotiation: ConsumerNegotiation) -> Self {
        Self {
            state: state_name(negotiation.state),
            offer: serde_json::to_value(&negotiation.offer).unwrap_or_default(),
            agreement: negotiation
                .agreement
                .as_ref()
                .and_then(|a| serde_json::to_value(a).ok()),
            consumer_pid: negotiation.consumer_pid,
            provider_pid: negotiation.provider_pid,
            counter_party_address: negotiation.provider_address,
            counter_party_id: negotiation.provider_id,
        }
    }
}

/// One page of negotiations.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationPage {
    /// The negotiations, in consumer pid order.
    negotiations: Vec<NegotiationView>,
    /// Pass as `after` to get the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
}

/// Paging for the lists.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// Return items whose consumer pid sorts after this one.
    after: Option<String>,
    /// How many to return, 1 to 100 (default 100).
    limit: Option<usize>,
}

/// Start a negotiation as consumer. The request to the provider is sent in
/// the background; follow the negotiation with `GET /v1/negotiations/{consumerPid}`.
#[utoipa::path(
    post,
    path = "/v1/negotiations",
    request_body = StartNegotiation,
    responses(
        (status = 201, description = "Started (state REQUESTED)", body = NegotiationView),
        (status = 400, description = "Invalid request", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Negotiations"
)]
async fn start(
    State(negotiator): State<Negotiator>,
    body: Result<Json<StartNegotiation>, JsonRejection>,
) -> Response {
    let mut req = match body {
        Ok(Json(r)) => r,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    if req
        .dataset_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return problem(StatusCode::BAD_REQUEST, "datasetId must not be empty");
    }
    match (req.offer.target.as_deref(), req.dataset_id.take()) {
        (Some(target), Some(dataset_id)) if target != dataset_id => {
            return problem(
                StatusCode::BAD_REQUEST,
                "datasetId and the offer's target differ",
            );
        }
        (None, d) => req.offer.target = d,
        _ => {}
    }
    if let Err(e) = portitor_dcp::did_web_url(&req.counter_party_id) {
        let detail = format!("counterPartyId must be a did:web DID: {e}");
        return problem(StatusCode::BAD_REQUEST, &detail);
    }
    match negotiator.start(&req.counter_party_address, &req.counter_party_id, req.offer) {
        Ok(started) => (StatusCode::CREATED, Json(NegotiationView::from(started))).into_response(),
        Err(f) => failed(&f),
    }
}

/// One negotiation, with its agreement once the provider sent it.
#[utoipa::path(
    get,
    path = "/v1/negotiations/{consumerPid}",
    params(("consumerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 200, description = "The negotiation", body = NegotiationView),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown negotiation", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Negotiations"
)]
async fn one(State(negotiator): State<Negotiator>, Path(cpid): Path<String>) -> Response {
    match negotiator.get(&cpid) {
        Ok(found) => Json(NegotiationView::from(found)).into_response(),
        Err(f) => failed(&f),
    }
}

/// The negotiations this connector runs as consumer, a page at a time.
#[utoipa::path(
    get,
    path = "/v1/negotiations",
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = NegotiationPage),
        (status = 400, description = "Invalid query", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Negotiations"
)]
async fn list(
    State(negotiator): State<Negotiator>,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let limit = q.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE);
    match negotiator.list(q.after.as_deref(), limit) {
        Ok(found) => {
            let next = next_after(&found, limit, |negotiation| &negotiation.consumer_pid);
            Json(NegotiationPage {
                negotiations: found.into_iter().map(NegotiationView::from).collect(),
                next,
            })
            .into_response()
        }
        Err(f) => failed(&f),
    }
}

/// The format of a pull transfer over HTTP, the only kind in 0.4.
const PULL_FORMAT: &str = "HttpData-PULL";

fn pull_format() -> String {
    PULL_FORMAT.to_owned()
}

/// Request a transfer as consumer.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartTransfer {
    /// The `@id` of the agreement of one of our `FINALIZED` negotiations.
    /// The transfer is requested from the provider of that negotiation.
    #[schema(example = "urn:uuid:agreement-1")]
    agreement_id: String,
    /// A format of one of the dataset's distributions (default
    /// `HttpData-PULL`, the only one this connector's data plane serves).
    #[serde(default = "pull_format")]
    #[schema(example = "HttpData-PULL")]
    format: String,
}

/// A transfer this connector runs as consumer.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    /// Our process id.
    consumer_pid: String,
    /// The provider's process id, once it has answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_pid: Option<String>,
    /// The DSP state, for example `REQUESTED` or `STARTED`.
    #[schema(example = "STARTED")]
    state: String,
    /// The other party's DSP endpoint: the provider's, as this connector
    /// is the consumer of every transfer it lists.
    counter_party_address: String,
    /// The other party's participant id: the provider's.
    counter_party_id: String,
    /// The agreement the transfer is made under.
    agreement_id: String,
    /// The format asked for.
    format: String,
    /// Where to fetch the data, with the access token (DSP 2025-1
    /// `DataAddress`): send `GET <endpoint>` with `Authorization: Bearer
    /// <the authorization property>` (its `authType` property says
    /// `bearer`). Present while the transfer is `STARTED` or `SUSPENDED`;
    /// the token works only while it is `STARTED`. The token is an opaque
    /// string: send it as it is and do not parse it; a 1.x release may
    /// issue another form.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    data_address: Option<Value>,
}

impl From<ConsumerTransfer> for TransferView {
    fn from(transfer: ConsumerTransfer) -> Self {
        Self {
            state: state_name(transfer.state),
            data_address: transfer
                .data_address
                .as_ref()
                .and_then(|a| serde_json::to_value(a).ok()),
            consumer_pid: transfer.consumer_pid,
            provider_pid: transfer.provider_pid,
            counter_party_address: transfer.provider_address,
            counter_party_id: transfer.provider_id,
            agreement_id: transfer.agreement_id,
            format: transfer.format,
        }
    }
}

/// A DSP state's name, for example `STARTED`.
fn state_name<S: Serialize>(state: S) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// One page of transfers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferPage {
    /// The transfers, in consumer pid order.
    transfers: Vec<TransferView>,
    /// Pass as `after` to get the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<String>,
}

/// Request a transfer as consumer, under an agreement from one of our
/// negotiations. The request is sent in the background; follow the
/// transfer with `GET /v1/transfers/{consumerPid}` until it is `STARTED`
/// and has a `dataAddress`.
#[utoipa::path(
    post,
    path = "/v1/transfers",
    request_body = StartTransfer,
    responses(
        (status = 201, description = "Requested (state REQUESTED)", body = TransferView),
        (status = 400, description = "Invalid request, or no finalized negotiation holds the agreement", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Transfers"
)]
async fn start_transfer(
    State(negotiator): State<Negotiator>,
    body: Result<Json<StartTransfer>, JsonRejection>,
) -> Response {
    let req = match body {
        Ok(Json(r)) => r,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    match negotiator.request_transfer(&req.agreement_id, &req.format) {
        Ok(transfer) => (StatusCode::CREATED, Json(TransferView::from(transfer))).into_response(),
        Err(f) => transfer_failed(&f),
    }
}

/// One transfer, with its data address once the provider started it.
#[utoipa::path(
    get,
    path = "/v1/transfers/{consumerPid}",
    params(("consumerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 200, description = "The transfer", body = TransferView),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown transfer", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Transfers"
)]
async fn one_transfer(State(negotiator): State<Negotiator>, Path(cpid): Path<String>) -> Response {
    match negotiator.transfer(&cpid) {
        Ok(transfer) => Json(TransferView::from(transfer)).into_response(),
        Err(f) => transfer_failed(&f),
    }
}

/// The transfers this connector runs as consumer, a page at a time.
#[utoipa::path(
    get,
    path = "/v1/transfers",
    params(PageQuery),
    responses(
        (status = 200, description = "One page", body = TransferPage),
        (status = 400, description = "Invalid query", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Transfers"
)]
async fn list_transfers(
    State(negotiator): State<Negotiator>,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let limit = q.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE);
    match negotiator.transfers(q.after.as_deref(), limit) {
        Ok(found) => {
            let next = next_after(&found, limit, |transfer| &transfer.consumer_pid);
            Json(TransferPage {
                transfers: found.into_iter().map(TransferView::from).collect(),
                next,
            })
            .into_response()
        }
        Err(f) => transfer_failed(&f),
    }
}

/// Complete a `STARTED` transfer: we have the data and need no more. The
/// completion is sent in the background.
#[utoipa::path(
    post,
    path = "/v1/transfers/{consumerPid}/completion",
    params(("consumerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 202, description = "Completion on its way; the state changes once the provider acknowledges it", body = TransferView),
        (status = 400, description = "The transfer is not STARTED, or a message is still on its way", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown transfer", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Transfers"
)]
async fn complete_transfer(
    State(negotiator): State<Negotiator>,
    Path(cpid): Path<String>,
) -> Response {
    act(&negotiator, &cpid, ConsumerTransferDecision::Complete)
}

/// End a transfer that has not ended yet. The termination is sent in the
/// background.
#[utoipa::path(
    post,
    path = "/v1/transfers/{consumerPid}/termination",
    params(("consumerPid" = String, Path, description = "Our process id")),
    responses(
        (status = 202, description = "Termination on its way; the state changes once the provider acknowledges it", body = TransferView),
        (status = 400, description = "The transfer has ended, or a message is still on its way", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown transfer", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Transfers"
)]
async fn terminate_transfer(
    State(negotiator): State<Negotiator>,
    Path(cpid): Path<String>,
) -> Response {
    act(&negotiator, &cpid, ConsumerTransferDecision::Terminate)
}

fn act(
    negotiator: &Negotiator,
    consumer_pid: &str,
    decision: ConsumerTransferDecision,
) -> Response {
    match negotiator.act_on_transfer(consumer_pid, decision) {
        Ok(transfer) => (StatusCode::ACCEPTED, Json(TransferView::from(transfer))).into_response(),
        Err(f) => transfer_failed(&f),
    }
}

/// What a harvest run did, counted.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarvestCountsView {
    /// Entries in the snapshot (or stored datasets checked again).
    seen: u64,
    /// Datasets that entered the catalog (new, or back after a withdrawal).
    added: u64,
    /// Datasets whose content or state changed.
    updated: u64,
    /// Datasets that passed with the same content.
    unchanged: u64,
    /// Datasets no longer in the source: out of the catalog, kept for
    /// transfers under agreements already made.
    withdrawn: u64,
    /// Entries that failed a check; the previous version, if any, stays.
    rejected: u64,
}

/// A harvest run: of the datasets, or of the authority's bundle.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarvestJobView {
    /// The job id; a newer job has a higher id.
    #[schema(example = 42)]
    id: u64,
    /// What it harvested: `datasets`, or `bundle` (a check for a new
    /// version of the authority's schema and vocabulary bundle; its
    /// counts are all 0, and its summary says which version is in use).
    #[schema(example = "datasets")]
    kind: String,
    /// When it started (UTC).
    #[schema(example = "2026-11-14T10:00:00Z")]
    started_at: String,
    /// When it ended (UTC).
    finished_at: String,
    /// What started it: `start`, `interval`, or `bundle` (a new bundle
    /// version came in, so the stored datasets were checked again).
    #[schema(example = "interval")]
    trigger: String,
    /// The source's name, for example `file:/etc/portitor/datasets.json`,
    /// or the bundles' URL.
    source: String,
    /// SHA-256 of the snapshot (for a bundle, of the new version's
    /// `manifest.json`), hex; absent when the source failed, nothing new
    /// was fetched, or the stored datasets were checked again.
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_sha256: Option<String>,
    /// `succeeded` (nothing rejected; for a bundle, a new version is in
    /// use), `partial` (some entries rejected), `unchanged` (same snapshot
    /// as the last run; for a bundle, no newer version) or `failed` (the
    /// source could not be read, or a new bundle version was refused; the
    /// catalog and the rules were left as they were).
    #[schema(example = "succeeded")]
    outcome: String,
    /// What the run did.
    counts: HarvestCountsView,
    /// One line for people.
    #[schema(example = "3 seen, 1 added, 0 updated, 2 unchanged, 0 withdrawn, 0 rejected")]
    summary: String,
}

impl From<HarvestJob> for HarvestJobView {
    fn from(job: HarvestJob) -> Self {
        let record = job.record;
        let counts = record.counts;
        Self {
            id: job.id,
            kind: record.kind.name().to_owned(),
            started_at: xsd_date_time(record.started_at),
            finished_at: xsd_date_time(record.finished_at),
            trigger: record.trigger.name().to_owned(),
            source: record.source,
            snapshot_sha256: record.snapshot_sha256,
            outcome: record.outcome.name().to_owned(),
            counts: HarvestCountsView {
                seen: counts.seen,
                added: counts.added,
                updated: counts.updated,
                unchanged: counts.unchanged,
                withdrawn: counts.withdrawn,
                rejected: counts.rejected,
            },
            summary: record.summary,
        }
    }
}

/// Why one entry was rejected.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarvestErrorView {
    /// The entry's position in the source, from 0; absent for a stored
    /// dataset checked again (`bundle` job).
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<u64>,
    /// The dataset's `@id`, if it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    dataset_id: Option<String>,
    /// The check that failed: `json`, `dsp`, `odrl` or `shacl`.
    #[schema(example = "odrl")]
    check: String,
    /// What is wrong.
    message: String,
    /// The report of the check, for example a SHACL validation report. A
    /// SHACL report keeps its first 50 results; `resultsNotStored` says
    /// how many more there were.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    report: Option<Value>,
}

impl From<EntryError> for HarvestErrorView {
    fn from(error: EntryError) -> Self {
        Self {
            position: error.position,
            dataset_id: error.dataset_id,
            check: error.kind.name().to_owned(),
            message: error.message,
            report: error.report,
        }
    }
}

/// A harvest job with its entry errors.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarvestJobDetail {
    /// The job.
    #[serde(flatten)]
    job: HarvestJobView,
    /// The entries it rejected, in the order it found them: at most the
    /// first 1000 (`counts.rejected` has the total, and the summary says
    /// when some were not stored).
    errors: Vec<HarvestErrorView>,
}

/// One page of harvest jobs.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarvestJobPage {
    /// The jobs, newest first.
    jobs: Vec<HarvestJobView>,
    /// Pass as `after` to get the next (older) page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<u64>,
}

/// Paging for the harvest jobs.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct JobPageQuery {
    /// Return jobs older than the job with this id.
    after: Option<u64>,
    /// How many to return, 1 to 100 (default 100).
    limit: Option<usize>,
}

/// The dataset harvest runs, newest first, a page at a time. The last
/// 100 runs are kept.
#[utoipa::path(
    get,
    path = "/v1/harvest-jobs",
    params(JobPageQuery),
    responses(
        (status = 200, description = "One page", body = HarvestJobPage),
        (status = 400, description = "Invalid query", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Harvest"
)]
async fn list_harvest_jobs(
    State(negotiator): State<Negotiator>,
    query: Result<Query<JobPageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Query(q) = match query {
        Ok(q) => q,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    let limit = q.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE);
    match negotiator.harvest_jobs(q.after, limit) {
        Ok(found) => {
            let next = if found.len() == limit {
                found.last().map(|job| job.id)
            } else {
                None
            };
            Json(HarvestJobPage {
                jobs: found.into_iter().map(HarvestJobView::from).collect(),
                next,
            })
            .into_response()
        }
        Err(e) => internal_error("harvest store", &e),
    }
}

/// One dataset harvest run, with the entries it rejected and why. A job
/// stores at most the first 1000 entry errors, so the answer has no pages.
#[utoipa::path(
    get,
    path = "/v1/harvest-jobs/{id}",
    params(("id" = u64, Path, description = "The job id")),
    responses(
        (status = 200, description = "The job", body = HarvestJobDetail),
        (status = 400, description = "The id is not a number", body = Problem, content_type = "application/problem+json"),
        (status = 401, description = "Missing or wrong API key", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = FORBIDDEN, body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Unknown job (or deleted: only the last 100 are kept)", body = Problem, content_type = "application/problem+json"),
    ),
    security(("api_key" = [])),
    tag = "Harvest"
)]
async fn one_harvest_job(
    State(negotiator): State<Negotiator>,
    id: Result<Path<u64>, axum::extract::rejection::PathRejection>,
) -> Response {
    let Path(id) = match id {
        Ok(id) => id,
        Err(e) => return problem(StatusCode::BAD_REQUEST, &e.body_text()),
    };
    match negotiator.harvest_job(id) {
        Ok(Some((job, errors))) => Json(HarvestJobDetail {
            job: HarvestJobView::from(job),
            errors: errors.into_iter().map(HarvestErrorView::from).collect(),
        })
        .into_response(),
        Ok(None) => problem(StatusCode::NOT_FOUND, "no such harvest job"),
        Err(e) => internal_error("harvest store", &e),
    }
}

/// The `after` value for the next page: the last key of a full page.
fn next_after<T>(found: &[T], limit: usize, key: impl Fn(&T) -> &String) -> Option<String> {
    (found.len() == limit)
        .then(|| found.last().map(|l| key(l).clone()))
        .flatten()
}

/// The `OpenAPI` description of the management API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Portitor management API",
        description = "Reads a provider's catalog, starts and follows contract negotiations and transfers in which this connector is the consumer, and shows the dataset harvest jobs (read only). Served on its own port (PORTITOR_MANAGEMENT_BIND), never on the DSP port. Every call except this description, the health checks (/api/check/...) and GET /health needs the API key. Without PORTITOR_MANAGEMENT_API_KEY_FILE, the port serves the health checks only. The API key is opaque (see the api_key security scheme). HTTP 403 is reserved for tokens with limited rights in 1.x; 1.0 never answers it. Every route answers 503 with a problem document when the request took longer than 60 seconds; at most 4 catalog or dataset answers are read from providers at a time, and a further request waits within those 60 seconds. The data plane token (the authorization property of a transfer's dataAddress) is opaque too: send it as it is, do not parse it; a 1.x release may issue another form.",
        license(name = "Apache-2.0", identifier = "Apache-2.0"),
    ),
    paths(
        request_catalog,
        request_dataset,
        start,
        one,
        list,
        start_transfer,
        one_transfer,
        list_transfers,
        complete_transfer,
        terminate_transfer,
        list_harvest_jobs,
        one_harvest_job
    ),
    components(schemas(
        CatalogRequest,
        DatasetRequest,
        StartNegotiation,
        NegotiationView,
        NegotiationPage,
        StartTransfer,
        TransferView,
        TransferPage,
        HarvestCountsView,
        HarvestJobView,
        HarvestErrorView,
        HarvestJobDetail,
        HarvestJobPage,
        Problem
    )),
    modifiers(&BearerKey),
    tags(
        (name = "Catalog", description = "A provider's catalog and datasets, read as consumer (DSP 2025-1). The request bodies are those of Eclipse EDC's management API v4."),
        (name = "Negotiations", description = "Contract negotiations as consumer (DSP 2025-1)"),
        (name = "Transfers", description = "Transfer processes as consumer (DSP 2025-1), pull only"),
        (name = "Harvest", description = "Dataset harvest runs: datasets come from the source into the catalog (ADR 0020). Read only.")
    )
)]
pub struct ManagementApi;

/// The `OpenAPI` description as pretty JSON, with the crate's version.
#[must_use]
pub fn openapi_json() -> String {
    let mut doc = health::with_health_api(ManagementApi::openapi());
    env!("CARGO_PKG_VERSION").clone_into(&mut doc.info.version);
    doc.to_pretty_json().unwrap_or_default()
}

/// The management API router. Every route except `GET /v1/openapi.json`
/// and `GET /health` needs `key`.
pub fn router(negotiator: Negotiator, key: ApiKey) -> Router {
    let key = std::sync::Arc::new(key);
    let protected = Router::new()
        .route("/v1/catalog/request", post(request_catalog))
        .route("/v1/catalog/dataset/request", post(request_dataset))
        .route("/v1/negotiations", get(list).post(start))
        .route("/v1/negotiations/{consumerPid}", get(one))
        .route("/v1/transfers", get(list_transfers).post(start_transfer))
        .route("/v1/transfers/{consumerPid}", get(one_transfer))
        .route(
            "/v1/transfers/{consumerPid}/completion",
            post(complete_transfer),
        )
        .route(
            "/v1/transfers/{consumerPid}/termination",
            post(terminate_transfer),
        )
        .route("/v1/harvest-jobs", get(list_harvest_jobs))
        .route("/v1/harvest-jobs/{id}", get(one_harvest_job))
        .route_layer(middleware::from_fn_with_state(key, require_key))
        .with_state(negotiator);
    let spec = openapi_json();
    Router::new()
        .route(
            "/v1/openapi.json",
            get(move || async move { ([(header::CONTENT_TYPE, "application/json")], spec) }),
        )
        .route("/health", get(|| async { "ok" }))
        .merge(protected)
}
