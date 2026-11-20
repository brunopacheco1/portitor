// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The management calls that read a provider's catalog and datasets
//! (`POST /v1/catalog/request`, `POST /v1/catalog/dataset/request`).
//!
//! Two kinds of provider answer here: a real Portitor provider in the
//! same process, reached over the tests' fake network (the answers are
//! checked against the official DSP 2025-1 JSON Schemas), and a small
//! HTTP server that plays a provider that refuses, sends pages, too much
//! or something else, reached with the real HTTP client.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use portitor_connector::auth::Prover;
use portitor_connector::catalog_reader::MAX_CATALOG_ANSWER;
use portitor_connector::http_client::Sender;
use portitor_connector::management::{ApiKey, DSP_HTTP_2025_1, openapi_json, router};
use portitor_connector::{Config, Ports};
use portitor_runtime::BoxFuture;
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::network::Network;
use common::{CONSUMER, PROVIDER as PROVIDER_DID, authenticating, recording};

const KEY: &str = "test-only-0123456789abcdef0123456789abcdef";

/// The DSP endpoint of the Portitor provider on the fake network.
const PROVIDER_ADDRESS: &str = "https://provider.example/dsp/2025-1";

/// The TCK's datasets file: what the Portitor provider offers.
const TCK_DATASETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");

const SCHEMAS: &[(&str, &str)] = &[
    (
        "common/context-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/context-schema.json"),
    ),
    (
        "catalog/catalog-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/catalog-schema.json"),
    ),
    (
        "catalog/dataset-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/dataset-schema.json"),
    ),
    (
        "catalog/catalog-request-message-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/catalog-request-message-schema.json"),
    ),
    (
        "negotiation/contract-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/contract-schema.json"),
    ),
];

/// Serves the bundled schemas by `$id`, so no network access is needed.
struct Local;

impl jsonschema::Retrieve for Local {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let path = uri
            .as_str()
            .strip_prefix("https://w3id.org/dspace/2025/1/")
            .ok_or("unknown schema")?;
        let raw = SCHEMAS
            .iter()
            .find(|(p, _)| *p == path)
            .ok_or("unknown schema")?
            .1;
        Ok(serde_json::from_str(raw)?)
    }
}

/// Checks `body` against the DSP 2025-1 schema at `path`.
fn assert_valid(path: &str, body: &Value) {
    let raw = SCHEMAS.iter().find(|(p, _)| *p == path).unwrap().1;
    let validator = jsonschema::options()
        .with_retriever(Local)
        .build(&serde_json::from_str(raw).unwrap())
        .unwrap();
    let errors: Vec<String> = validator.iter_errors(body).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{path}: {errors:?}\n{body:#}");
}

fn config(participant_id: &str, public_url: &str, datasets_file: Option<&str>) -> Config {
    Config {
        participant_id: participant_id.into(),
        public_url: public_url.into(),
        datasets_file: datasets_file.map(Into::into),
        harvest_interval_seconds: 0,
        allow_http: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

/// A consumer's management router and a Portitor provider with the TCK's
/// datasets, on one fake network. Also returns the audiences of the
/// tokens the consumer made.
fn consumer_and_portitor_provider() -> (Router, common::Audiences) {
    let network = Arc::new(Network::default());
    let provider_ports = authenticating(Ports::new(Arc::clone(&network) as _), PROVIDER_DID);
    let provider_config = config(PROVIDER_DID, "https://provider.example", Some(TCK_DATASETS));
    let provider = portitor_connector::router(&provider_config, provider_ports).unwrap();
    network.join("https://provider.example", provider);
    let (consumer_ports, audiences) = recording(Ports::new(network as _), CONSUMER);
    let consumer_config = config(CONSUMER, "https://consumer.example", None);
    let (_, negotiator) = portitor_connector::build(&consumer_config, consumer_ports).unwrap();
    (router(negotiator, ApiKey::new(KEY).unwrap()), audiences)
}

/// What one management call answered.
struct Answer {
    status: StatusCode,
    content_type: String,
    links: Vec<String>,
    body: Value,
}

/// POSTs `body` to the management route `uri`, with the API key.
async fn post(app: &Router, uri: &str, body: &Value) -> Answer {
    let request = Request::post(uri)
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_owned();
    let mut links = Vec::new();
    for value in response.headers().get_all(header::LINK) {
        links.push(value.to_str().unwrap().to_owned());
    }
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Answer {
        status,
        content_type,
        links,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// A catalog request as an Eclipse EDC client writes it (management API
/// v4, `catalog-request-schema.json`).
fn catalog_request(address: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/edc/connector/management/v2"],
        "@type": "CatalogRequest",
        "counterPartyAddress": address,
        "counterPartyId": PROVIDER_DID,
        "protocol": DSP_HTTP_2025_1
    })
}

/// A dataset request as an Eclipse EDC client writes it
/// (`dataset-request-schema.json`).
fn dataset_request(address: &str, dataset_id: &str) -> Value {
    json!({
        "@context": ["https://w3id.org/edc/connector/management/v2"],
        "@type": "DatasetRequest",
        "@id": dataset_id,
        "counterPartyAddress": address,
        "counterPartyId": PROVIDER_DID,
        "protocol": DSP_HTTP_2025_1
    })
}

/// Checks that `answer` is the problem `status` and returns its detail.
fn problem_detail(answer: &Answer, status: StatusCode) -> String {
    assert_eq!(answer.status, status, "{}", answer.body);
    assert_eq!(answer.content_type, "application/problem+json");
    assert_eq!(answer.body["status"], status.as_u16());
    answer.body["detail"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn the_catalog_of_a_portitor_provider_is_returned_as_it_is() {
    let (app, audiences) = consumer_and_portitor_provider();
    let answer = post(
        &app,
        "/v1/catalog/request",
        &catalog_request(PROVIDER_ADDRESS),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.content_type, "application/json");
    assert!(answer.links.is_empty());
    assert_valid("catalog/catalog-schema.json", &answer.body);
    assert_eq!(answer.body["@type"], "Catalog");
    assert_eq!(answer.body["participantId"], PROVIDER_DID);
    let listed = answer.body["dataset"].as_array().unwrap();
    let in_file: Vec<Value> =
        serde_json::from_slice(&std::fs::read(TCK_DATASETS).unwrap()).unwrap();
    assert_eq!(listed.len(), in_file.len());
    // the offer of a dataset is there to copy into a negotiation
    assert!(listed[0]["hasPolicy"][0]["@id"].is_string());
    // one token, made for the provider
    assert_eq!(*audiences.lock().unwrap(), [PROVIDER_DID]);
    // a trailing slash on the address changes nothing, and `profile` is
    // EDC's other name for `protocol`
    let mut request = catalog_request(&format!("{PROVIDER_ADDRESS}/"));
    request.as_object_mut().unwrap().remove("protocol");
    request["profile"] = json!(DSP_HTTP_2025_1);
    let again = post(&app, "/v1/catalog/request", &request).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.body);
    assert_eq!(again.body, answer.body);
}

#[tokio::test]
async fn one_dataset_of_a_portitor_provider_is_returned() {
    let (app, audiences) = consumer_and_portitor_provider();
    let request = dataset_request(PROVIDER_ADDRESS, "CAT0102");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.content_type, "application/json");
    assert_valid("catalog/dataset-schema.json", &answer.body);
    assert_eq!(answer.body["@id"], "CAT0102");
    assert_eq!(*audiences.lock().unwrap(), [PROVIDER_DID]);
}

#[tokio::test]
async fn a_dataset_the_provider_does_not_have_is_a_bad_gateway() {
    let (app, _) = consumer_and_portitor_provider();
    let request = dataset_request(PROVIDER_ADDRESS, "urn:uuid:no-such-dataset");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert_eq!(detail, "the provider answered HTTP 404 Not Found");
}

#[tokio::test]
async fn the_calls_need_the_key() {
    let (app, audiences) = consumer_and_portitor_provider();
    for uri in ["/v1/catalog/request", "/v1/catalog/dataset/request"] {
        let request = Request::post(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(catalog_request(PROVIDER_ADDRESS).to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    assert!(audiences.lock().unwrap().is_empty());
}

/// Every request below is refused with 400 before a token is made or
/// anything is sent.
#[tokio::test]
async fn bad_catalog_requests_are_refused() {
    let (app, audiences) = consumer_and_portitor_provider();
    let good = catalog_request(PROVIDER_ADDRESS);
    // (what to change in a good request, a part of the detail)
    let changes: Vec<(Value, &str)> = vec![
        (
            json!({"protocol": null}),
            "protocol (or profile) is required",
        ),
        (
            json!({"protocol": "dataspace-protocol-http"}),
            "is not supported",
        ),
        (
            json!({"protocol": "dataspace-protocol-http:2024/1"}),
            "is not supported",
        ),
        (json!({"profile": DSP_HTTP_2025_1}), "not both"),
        (json!({"@type": "DatasetRequest"}), "@type must be"),
        (
            json!({"querySpec": {"@type": "QuerySpec", "limit": 10}}),
            "querySpec is not supported",
        ),
        (json!({"querySpec": {}}), "querySpec is not supported"),
        (json!({"additionalScopes": ["a b"]}), "additionalScopes"),
        (json!({"additionalScopes": [""]}), "additionalScopes"),
        (json!({"additionalScopes": "a"}), "additionalScopes"),
        (json!({"counterPartyId": "provider"}), "did:web"),
        (
            json!({"counterPartyAddress": "http://provider.example/dsp/2025-1"}),
            "https URL",
        ),
        (
            json!({"counterPartyAddress": "https://provider.example/dsp?x=1"}),
            "query",
        ),
        (json!({"@id": "urn:uuid:1"}), "unknown field"),
        (
            json!({"providerAddress": "https://x.example"}),
            "unknown field",
        ),
    ];
    for (change, expected) in changes {
        let mut request = good.clone();
        for (name, value) in change.as_object().unwrap() {
            if value.is_null() {
                request.as_object_mut().unwrap().remove(name);
            } else {
                request[name] = value.clone();
            }
        }
        let answer = post(&app, "/v1/catalog/request", &request).await;
        let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
        assert!(detail.contains(expected), "{change}: {detail}");
    }
    for missing in ["counterPartyAddress", "counterPartyId"] {
        let mut request = good.clone();
        request.as_object_mut().unwrap().remove(missing);
        let answer = post(&app, "/v1/catalog/request", &request).await;
        let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
        assert!(detail.contains(missing), "{detail}");
    }
    let not_json = Request::post("/v1/catalog/request")
        .header(header::AUTHORIZATION, format!("Bearer {KEY}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{"))
        .unwrap();
    let response = app.clone().oneshot(not_json).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(audiences.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bad_dataset_requests_are_refused() {
    let (app, audiences) = consumer_and_portitor_provider();
    let good = dataset_request(PROVIDER_ADDRESS, "CAT0102");
    let changes: Vec<(Value, &str)> = vec![
        (json!({"@id": null}), "@id"),
        (json!({"@id": " "}), "dataset id"),
        (
            json!({"protocol": null}),
            "protocol (or profile) is required",
        ),
        (json!({"protocol": "ids-multipart"}), "is not supported"),
        (json!({"@type": "CatalogRequest"}), "@type must be"),
        // EDC's DatasetRequest has neither
        (json!({"querySpec": {}}), "unknown field"),
        (json!({"additionalScopes": []}), "unknown field"),
        (
            json!({"counterPartyAddress": "http://provider.example/dsp/2025-1"}),
            "https URL",
        ),
    ];
    for (change, expected) in changes {
        let mut request = good.clone();
        for (name, value) in change.as_object().unwrap() {
            if value.is_null() {
                request.as_object_mut().unwrap().remove(name);
            } else {
                request[name] = value.clone();
            }
        }
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
        assert!(detail.contains(expected), "{change}: {detail}");
    }
    assert!(audiences.lock().unwrap().is_empty());
}

/// What a played provider received, in order.
#[derive(Default)]
struct Received {
    /// The `Authorization` header of each request.
    authorizations: Vec<String>,
    /// The body of each `POST .../catalog/request`.
    catalog_requests: Vec<Value>,
}

type Seen = Arc<Mutex<Received>>;

/// A small catalog, as a provider other than Portitor may send it: with
/// a member Portitor's own types do not have.
fn played_catalog() -> Value {
    json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@id": "urn:uuid:catalog-1",
        "@type": "Catalog",
        "participantId": PROVIDER_DID,
        "dct:title": "A played catalog",
        "dataset": [{
            "@id": "urn:uuid:dataset-1",
            "@type": "Dataset",
            "hasPolicy": [{"@id": "urn:uuid:offer-1", "@type": "Offer",
                           "permission": [{"action": "use"}]}],
            "distribution": [{"@type": "Distribution", "format": "HttpData-PULL",
                              "accessService": {"@id": "urn:uuid:service-1"}}]
        }]
    })
}

/// `POST /{case}/catalog/request`: the played provider answers as `case`
/// says.
async fn played_catalog_request(
    State(seen): State<Seen>,
    Path(case): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    {
        let mut seen = seen.lock().unwrap();
        let authorization = headers.get(header::AUTHORIZATION);
        let authorization = authorization.map(|value| value.to_str().unwrap().to_owned());
        seen.authorizations.push(authorization.unwrap_or_default());
        seen.catalog_requests
            .push(serde_json::from_str(&body).unwrap_or(Value::Null));
    }
    let json = [(header::CONTENT_TYPE, "application/json")];
    match case.as_str() {
        "whole" => (json, played_catalog().to_string()).into_response(),
        "pages" => {
            let mut response = (json, played_catalog().to_string()).into_response();
            for link in [
                "<https://provider.example/catalog?page=0>; rel=\"previous\"",
                "<https://provider.example/catalog?page=2>; rel=\"next\"",
            ] {
                response
                    .headers_mut()
                    .append(header::LINK, link.parse().unwrap());
            }
            response
        }
        "unauthorized" => {
            let error = json!({
                "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
                "@type": "CatalogError", "code": "401", "reason": ["unauthorized"]
            });
            (StatusCode::UNAUTHORIZED, json, error.to_string()).into_response()
        }
        "down" => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        // one byte more than is read
        "too-large" => (json, " ".repeat(MAX_CATALOG_ANSWER + 1)).into_response(),
        // exactly as much as is read: spaces after a JSON value are JSON
        "largest" => {
            let mut text = played_catalog().to_string();
            let padding = MAX_CATALOG_ANSWER - text.len();
            text.push_str(&" ".repeat(padding));
            (json, text).into_response()
        }
        "a-dataset" => (json, json!({"@type": "Dataset", "@id": "x"}).to_string()).into_response(),
        "no-type" => (json, "{}".to_owned()).into_response(),
        "html" => "<html>a login page</html>".into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /{case}/catalog/datasets/{id}`: a dataset whose `@id` is the id
/// asked for; 404 for the case `unknown`, a catalog for `a-catalog`.
async fn played_dataset(
    State(seen): State<Seen>,
    Path((case, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers.get(header::AUTHORIZATION);
    let authorization = authorization.map(|value| value.to_str().unwrap().to_owned());
    let mut seen = seen.lock().unwrap();
    seen.authorizations.push(authorization.unwrap_or_default());
    if case == "unknown" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if case == "a-catalog" {
        return axum::Json(played_catalog()).into_response();
    }
    let mut dataset = played_catalog()["dataset"][0].clone();
    dataset["@context"] = json!(["https://w3id.org/dspace/2025/1/context.jsonld"]);
    dataset["@id"] = json!(id);
    axum::Json(dataset).into_response()
}

/// Starts the played provider on a local port; returns its base URL
/// (`http://127.0.0.1:<port>`) and what it receives.
async fn played_provider() -> (String, Seen) {
    let seen = Seen::default();
    let app = Router::new()
        .route(
            "/{case}/catalog/request",
            axum::routing::post(played_catalog_request),
        )
        .route(
            "/{case}/catalog/datasets/{id}",
            axum::routing::get(played_dataset),
        )
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{address}"), seen)
}

/// A consumer's management router whose connector sends with the real
/// HTTP client. `allow_http` is `PORTITOR_ALLOW_HTTP`.
fn consumer_over_http(allow_http: bool) -> Router {
    let sender = portitor_connector::dsp_sender(allow_http).unwrap();
    let ports = authenticating(Ports::new(Arc::new(sender)), CONSUMER);
    let mut config = config(CONSUMER, "https://consumer.example", None);
    config.allow_http = allow_http;
    let (_, negotiator) = portitor_connector::build(&config, ports).unwrap();
    router(negotiator, ApiKey::new(KEY).unwrap())
}

#[tokio::test]
async fn the_request_sent_is_a_dsp_catalog_request_with_our_token() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let request = catalog_request(&format!("{base}/whole"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    // as it came: the member Portitor's types do not know is still there
    assert_eq!(answer.body, played_catalog());
    assert!(answer.links.is_empty());
    let seen = seen.lock().unwrap();
    assert_eq!(seen.authorizations, [common::bearer(CONSUMER)]);
    let sent = &seen.catalog_requests[0];
    assert_valid("catalog/catalog-request-message-schema.json", sent);
    assert_eq!(sent["@type"], "CatalogRequestMessage");
    // the whole catalog is asked for
    assert!(sent.get("filter").is_none());
}

#[tokio::test]
async fn a_dataset_id_is_sent_as_one_path_segment() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    for id in ["urn:uuid:dataset-1", "a/b c?d#e", "données"] {
        let request = dataset_request(&format!("{base}/whole"), id);
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        assert_eq!(answer.body["@id"], id);
    }
    assert_eq!(seen.lock().unwrap().authorizations.len(), 3);
}

/// DSP 2025-1, "Catalog HTTPS Binding", "Pagination": the first page is
/// returned, with the provider's `Link` headers; no other page is read.
#[tokio::test]
async fn of_a_catalog_in_pages_the_first_is_returned_with_its_links() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let request = catalog_request(&format!("{base}/pages"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body, played_catalog());
    assert_eq!(
        answer.links,
        [
            "<https://provider.example/catalog?page=0>; rel=\"previous\"",
            "<https://provider.example/catalog?page=2>; rel=\"next\"",
        ]
    );
    assert_eq!(seen.lock().unwrap().authorizations.len(), 1);
}

#[tokio::test]
async fn a_provider_that_refuses_is_a_bad_gateway_with_its_status() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let request = catalog_request(&format!("{base}/unauthorized"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    // the status, and the start of the provider's error, which says why
    assert!(
        detail.starts_with("the provider answered HTTP 401 Unauthorized: "),
        "{detail}"
    );
    assert!(detail.contains("CatalogError"), "{detail}");
    let request = catalog_request(&format!("{base}/down"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert_eq!(detail, "the provider answered HTTP 503 Service Unavailable");
    // one try each: a person waits for the answer, so nothing is retried
    assert_eq!(seen.lock().unwrap().authorizations.len(), 2);
    let request = dataset_request(&format!("{base}/unknown"), "urn:uuid:dataset-1");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert_eq!(detail, "the provider answered HTTP 404 Not Found");
}

#[tokio::test]
async fn a_provider_that_cannot_be_reached_is_a_bad_gateway() {
    // a port nobody listens on: taken, then given back
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let app = consumer_over_http(true);
    let request = catalog_request(&format!("http://{address}/dsp/2025-1"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(
        detail.starts_with("the provider did not answer: "),
        "{detail}"
    );
    let request = dataset_request(&format!("http://{address}/dsp/2025-1"), "d");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    problem_detail(&answer, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn an_answer_up_to_the_limit_is_read_and_a_larger_one_is_a_bad_gateway() {
    let (base, _) = played_provider().await;
    let app = consumer_over_http(true);
    let request = catalog_request(&format!("{base}/largest"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.body, played_catalog());
    let request = catalog_request(&format!("{base}/too-large"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(detail.contains("cannot be used"), "{detail}");
    assert!(
        detail.contains(&format!("at most {MAX_CATALOG_ANSWER} bytes")),
        "{detail}"
    );
}

#[tokio::test]
async fn an_answer_that_is_not_what_was_asked_for_is_a_bad_gateway() {
    let (base, _) = played_provider().await;
    let app = consumer_over_http(true);
    for case in ["a-dataset", "no-type", "html"] {
        let request = catalog_request(&format!("{base}/{case}"));
        let answer = post(&app, "/v1/catalog/request", &request).await;
        let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
        assert!(detail.contains("the provider's answer is not"), "{detail}");
    }
    // a catalog where a dataset was asked for
    let request = dataset_request(&format!("{base}/a-catalog"), "d");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(detail.contains("is not a DSP 2025-1 Dataset"), "{detail}");
}

/// Without `PORTITOR_ALLOW_HTTP` a plain `http://` address is
/// refused, and the provider is never called.
#[tokio::test]
async fn an_http_address_is_refused_without_the_allow_flag() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(false);
    let request = catalog_request(&format!("{base}/whole"));
    let answer = post(&app, "/v1/catalog/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
    assert_eq!(detail, "the provider address must be an https URL");
    let request = dataset_request(&format!("{base}/whole"), "urn:uuid:dataset-1");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
    assert_eq!(detail, "the provider address must be an https URL");
    assert!(seen.lock().unwrap().authorizations.is_empty());
}

/// The audience and the scopes of each token asked for, in order.
type Asked = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// A prover that records the audience and the scopes of each token, or
/// has no token at all.
struct RecordingProver {
    asked: Asked,
    has_tokens: bool,
}

impl Prover for RecordingProver {
    fn authorization<'a>(
        &'a self,
        audience: &'a str,
        scopes: &'a [String],
    ) -> BoxFuture<'a, Result<String, String>> {
        let mut asked = self.asked.lock().unwrap();
        asked.push((audience.to_owned(), scopes.to_vec()));
        let has_tokens = self.has_tokens;
        Box::pin(async move {
            if has_tokens {
                Ok(common::bearer(CONSUMER))
            } else {
                Err("the STS at http://sts.internal answered HTTP 401".to_owned())
            }
        })
    }
}

/// A consumer's management router with a [`RecordingProver`], next to a
/// Portitor provider on the fake network.
fn consumer_with_prover(has_tokens: bool) -> (Router, Asked) {
    let network = Arc::new(Network::default());
    let provider_ports = authenticating(Ports::new(Arc::clone(&network) as _), PROVIDER_DID);
    let provider_config = config(PROVIDER_DID, "https://provider.example", Some(TCK_DATASETS));
    let provider = portitor_connector::router(&provider_config, provider_ports).unwrap();
    network.join("https://provider.example", provider);
    let asked = Asked::default();
    let mut ports = authenticating(Ports::new(network as _), CONSUMER);
    ports.prover = Arc::new(RecordingProver {
        asked: Arc::clone(&asked),
        has_tokens,
    });
    let consumer_config = config(CONSUMER, "https://consumer.example", None);
    let (_, negotiator) = portitor_connector::build(&consumer_config, ports).unwrap();
    (router(negotiator, ApiKey::new(KEY).unwrap()), asked)
}

#[tokio::test]
async fn additional_scopes_go_into_the_token() {
    let (app, asked) = consumer_with_prover(true);
    let scope = "org.eclipse.dspace.dcp.vc.type:ResearchCredential:read";
    let mut request = catalog_request(PROVIDER_ADDRESS);
    request["additionalScopes"] = json!([scope]);
    let answer = post(&app, "/v1/catalog/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    // without the member: no scope beyond the configured ones
    let answer = post(
        &app,
        "/v1/catalog/request",
        &catalog_request(PROVIDER_ADDRESS),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let request = dataset_request(PROVIDER_ADDRESS, "CAT0102");
    let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let provider = PROVIDER_DID.to_owned();
    assert_eq!(
        *asked.lock().unwrap(),
        [
            (provider.clone(), vec![scope.to_owned()]),
            (provider.clone(), Vec::new()),
            (provider, Vec::new()),
        ]
    );
}

#[tokio::test]
async fn without_a_token_the_answer_is_a_bad_gateway_that_does_not_say_why() {
    let (app, asked) = consumer_with_prover(false);
    let answer = post(
        &app,
        "/v1/catalog/request",
        &catalog_request(PROVIDER_ADDRESS),
    )
    .await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(detail.contains("no token"), "{detail}");
    // the reason may name our STS: it is in the log only
    assert!(!detail.contains("sts.internal"), "{detail}");
    assert_eq!(asked.lock().unwrap().len(), 1);
}

/// A sender with only the required method, as most test fakes are: it
/// cannot read a catalog, and says so.
struct PostOnly;

impl Sender for PostOnly {
    fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async { Ok(Value::Null) })
    }
}

#[tokio::test]
async fn a_sender_that_cannot_read_large_answers_says_so() {
    let ports = authenticating(Ports::new(Arc::new(PostOnly)), CONSUMER);
    let consumer_config = config(CONSUMER, "https://consumer.example", None);
    let (_, negotiator) = portitor_connector::build(&consumer_config, ports).unwrap();
    let app = router(negotiator, ApiKey::new(KEY).unwrap());
    let answer = post(
        &app,
        "/v1/catalog/request",
        &catalog_request(PROVIDER_ADDRESS),
    )
    .await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(detail.starts_with("the request was not sent: "), "{detail}");
}

/// ADR 0025, "What must be settled before the 0.13 freeze", point 2:
/// every route that needs the key documents 403, and says that it is
/// reserved.
#[test]
fn the_description_has_the_catalog_calls_and_403_on_every_protected_route() {
    let spec: Value = serde_json::from_str(&openapi_json()).unwrap();
    for path in ["/v1/catalog/request", "/v1/catalog/dataset/request"] {
        let operation = &spec["paths"][path]["post"];
        assert!(operation.is_object(), "{path}");
        for status in ["200", "400", "401", "403", "502"] {
            assert!(
                operation["responses"][status].is_object(),
                "{path} {status}"
            );
        }
    }
    let request = &spec["components"]["schemas"]["CatalogRequest"];
    assert_eq!(
        request["required"],
        json!(["counterPartyAddress", "counterPartyId"])
    );
    let request = &spec["components"]["schemas"]["DatasetRequest"];
    assert_eq!(
        request["required"],
        json!(["@id", "counterPartyAddress", "counterPartyId"])
    );
    let mut protected = 0;
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            // the health checks and this description need no key
            if operation.get("security").is_none() {
                continue;
            }
            protected += 1;
            let forbidden = &operation["responses"]["403"];
            let description = forbidden["description"].as_str().unwrap_or_default();
            assert!(
                description.contains("Reserved for scoped access in 1.x"),
                "{method} {path}: {forbidden}"
            );
            assert!(description.contains("keeps every right"), "{method} {path}");
        }
    }
    // 2 catalog, 3 negotiation, 5 transfer and 2 harvest calls
    assert_eq!(protected, 12);
    // ADR 0025, point 3: `dataAddress` stays optional
    let transfer = &spec["components"]["schemas"]["TransferView"];
    let required = transfer["required"].as_array().unwrap();
    assert!(!required.contains(&json!("dataAddress")));
}

// ---- A provider that answers badly, on purpose or not. The played
// ---- provider below writes its answer byte for byte, so it can send
// ---- what a web framework would never send.

/// Starts a provider that answers every request with exactly the bytes
/// of `answer` (a whole HTTP/1.1 response) and then closes the
/// connection, or, with `then_stalls`, keeps it open and sends nothing
/// more. Returns its base URL and the first line of each request.
async fn raw_provider(answer: Vec<u8>, then_stalls: bool) -> (String, Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&request_lines);
    tokio::spawn(async move {
        loop {
            let (mut connection, _) = listener.accept().await.unwrap();
            let (answer, seen) = (answer.clone(), Arc::clone(&seen));
            tokio::spawn(async move {
                // read the whole request first: closing a connection with
                // unread bytes can cut our answer short
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = connection.read(&mut buffer).await.unwrap_or(0);
                    request.extend_from_slice(&buffer[..read]);
                    if read == 0 || whole_request(&request) {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let first_line = text.lines().next().unwrap_or_default().to_owned();
                seen.lock().unwrap().push(first_line);
                // the reader may stop reading early (too large): not an
                // error of this provider
                let _ = connection.write_all(&answer).await;
                if then_stalls {
                    tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
                }
                let _ = connection.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), request_lines)
}

/// Whether `request` holds a whole HTTP/1.1 request: its head, and as
/// many body bytes as its `content-length` says.
fn whole_request(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request).to_ascii_lowercase();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let mut expected = 0;
    for line in head.lines() {
        if let Some(length) = line.strip_prefix("content-length:") {
            expected = length.trim().parse().unwrap();
        }
    }
    body.len() >= expected
}

/// A whole `200 OK` response with `body` as JSON and the extra header
/// lines `headers` (each ends with `\r\n`).
fn ok_response(headers: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n{headers}content-length: {}\r\n\r\n",
        body.len()
    );
    [head.as_bytes(), body].concat()
}

/// A whole `200 OK` response that sends `body` in chunks (`Transfer-Encoding:
/// chunked`, RFC 9112, section 7.1), so without saying its length first.
fn chunked_response(body: &[u8]) -> Vec<u8> {
    let mut response =
        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n"
            .to_vec();
    for chunk in body.chunks(1024 * 1024) {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    response
}

/// Asks the provider that answers with `response` for its catalog,
/// through the management API.
async fn catalog_of_raw_provider(response: Vec<u8>) -> Answer {
    let (base, _) = raw_provider(response, false).await;
    let app = consumer_over_http(true);
    post(&app, "/v1/catalog/request", &catalog_request(&base)).await
}

/// The played catalog as text, padded with spaces to `length` bytes
/// (spaces after a JSON value are JSON).
fn played_catalog_of_length(length: usize) -> Vec<u8> {
    let mut text = played_catalog().to_string();
    text.push_str(&" ".repeat(length - text.len()));
    text.into_bytes()
}

/// The limit of 16 MiB holds for a body sent in chunks too, where no
/// `Content-Length` announces the size.
#[tokio::test]
async fn a_chunked_answer_is_read_up_to_the_limit_and_one_byte_more_is_a_bad_gateway() {
    let at_the_limit = chunked_response(&played_catalog_of_length(MAX_CATALOG_ANSWER));
    let answer = catalog_of_raw_provider(at_the_limit).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.body, played_catalog());

    let one_more = chunked_response(&played_catalog_of_length(MAX_CATALOG_ANSWER + 1));
    let answer = catalog_of_raw_provider(one_more).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert!(detail.contains("cannot be used"), "{detail}");
}

/// A provider cannot make the connector buffer response headers without
/// end: the HTTP client gives up, and the caller gets 502, not 500.
#[tokio::test]
async fn an_answer_with_a_huge_header_or_too_many_headers_is_a_bad_gateway() {
    let catalog = played_catalog().to_string();
    let huge = format!("x-fill: {}\r\n", "a".repeat(2 * 1024 * 1024));
    let many = "x-fill: a\r\n".repeat(500);
    for headers in [huge, many] {
        let answer = catalog_of_raw_provider(ok_response(&headers, catalog.as_bytes())).await;
        let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
        assert!(
            detail.starts_with("the provider did not answer: "),
            "{detail}"
        );
    }
}

/// An answer that says 200 but is cut off, or is no answer at all.
#[tokio::test]
async fn an_answer_that_breaks_off_is_a_bad_gateway() {
    let catalog = played_catalog().to_string();
    // announces more bytes than it sends, then closes
    let mut cut_off = ok_response("", catalog.as_bytes());
    cut_off.truncate(cut_off.len() - 10);
    // closes without a word
    let nothing = Vec::new();
    // not HTTP
    let not_http = b"SSH-2.0-OpenSSH_9.6\r\n".to_vec();
    for response in [cut_off, nothing, not_http] {
        let answer = catalog_of_raw_provider(response).await;
        let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
        assert!(
            detail.starts_with("the provider did not answer: "),
            "{detail}"
        );
    }
}

/// The 30 seconds are for the whole answer, not only for its first
/// byte: a provider that sends the status line, the headers and a part
/// of the body and then nothing more is given up after 30 seconds.
///
/// The clock is paused: tokio moves it forward when every task waits,
/// so the test does not take 30 seconds.
#[tokio::test(start_paused = true)]
async fn a_provider_that_stops_in_the_middle_of_the_body_is_given_up_after_30_seconds() {
    let catalog = played_catalog().to_string();
    let mut half = ok_response("", catalog.as_bytes());
    half.truncate(half.len() - 10);
    let (base, request_lines) = raw_provider(half, true).await;
    let app = consumer_over_http(true);
    let started = tokio::time::Instant::now();
    let answer = post(&app, "/v1/catalog/request", &catalog_request(&base)).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert_eq!(detail, "the provider did not answer: no answer within 30s");
    let waited = started.elapsed();
    assert!(waited >= std::time::Duration::from_secs(30), "{waited:?}");
    assert!(waited < std::time::Duration::from_secs(31), "{waited:?}");
    // the request did arrive: it is the answer that never ended
    let request_lines = request_lines.lock().unwrap();
    assert_eq!(request_lines.len(), 1, "one try");
    assert!(request_lines[0].starts_with("POST /catalog/request "));
}

/// What is handed on as `application/json` is JSON: text in UTF-8 (RFC
/// 8259, section 8.1) with one object whose `@type` is the plain text
/// `Catalog`, as the DSP 2025-1 schema says (`catalog-schema.json`:
/// `"@type": {"type": "string", "const": "Catalog"}`; so no list, no
/// prefix, no IRI).
#[tokio::test]
async fn an_answer_that_is_not_a_json_catalog_is_never_handed_on() {
    let bodies: Vec<(&str, Vec<u8>)> = vec![
        ("no body", Vec::new()),
        ("not UTF-8 at all", vec![0xff, 0xfe, 0x00]),
        (
            "a text that is not UTF-8 inside the catalog",
            b"{\"@type\":\"Catalog\",\"dct:title\":\"caf\xe9\"}".to_vec(),
        ),
        (
            "a member name that is not UTF-8",
            b"{\"@type\":\"Catalog\",\"caf\xe9\":1}".to_vec(),
        ),
        (
            "a byte order mark first",
            b"\xef\xbb\xbf{\"@type\":\"Catalog\"}".to_vec(),
        ),
        ("@type as a list", br#"{"@type":["Catalog"]}"#.to_vec()),
        (
            "@type with a prefix",
            br#"{"@type":"dcat:Catalog"}"#.to_vec(),
        ),
        (
            "@type as an IRI",
            br#"{"@type":"http://www.w3.org/ns/dcat#Catalog"}"#.to_vec(),
        ),
        ("@type in other letters", br#"{"@type":"catalog"}"#.to_vec()),
        (
            "@type twice",
            br#"{"@type":"Catalog","@type":"Dataset"}"#.to_vec(),
        ),
        (
            "@type only further inside",
            br#"{"dataset":[{"@type":"Catalog"}]}"#.to_vec(),
        ),
        (
            "more after the catalog",
            br#"{"@type":"Catalog"}{"@type":"Catalog"}"#.to_vec(),
        ),
        ("a list of catalogs", br#"[{"@type":"Catalog"}]"#.to_vec()),
        ("a text", br#""Catalog""#.to_vec()),
        (
            "cut in the middle",
            br#"{"@type":"Catalog","dataset":["#.to_vec(),
        ),
    ];
    for (name, body) in bodies {
        let answer = catalog_of_raw_provider(ok_response("", &body)).await;
        assert_eq!(answer.status, StatusCode::BAD_GATEWAY, "{name}");
        let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
        assert!(
            detail.starts_with("the provider's answer is not "),
            "{name}: {detail}"
        );
    }
    // HTTP 204: a success without a catalog
    let no_content = b"HTTP/1.1 204 No Content\r\n\r\n".to_vec();
    let answer = catalog_of_raw_provider(no_content).await;
    problem_detail(&answer, StatusCode::BAD_GATEWAY);
}

/// A redirect is not followed: our token was made for the provider, and
/// the provider may not send it elsewhere.
#[tokio::test]
async fn a_redirect_is_not_followed() {
    let (elsewhere, seen_elsewhere) = played_provider().await;
    let redirect = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nlocation: {elsewhere}/whole/catalog/request\r\ncontent-length: 0\r\n\r\n"
    );
    let answer = catalog_of_raw_provider(redirect.into_bytes()).await;
    let detail = problem_detail(&answer, StatusCode::BAD_GATEWAY);
    assert_eq!(detail, "the provider answered HTTP 307 Temporary Redirect");
    assert!(seen_elsewhere.lock().unwrap().authorizations.is_empty());
}

/// What the code does today, written down (a question for the
/// maintainer, not a rule of DSP): the catalog's `participantId` is not
/// compared with `counterPartyId`. The token is made for
/// `counterPartyId`, so a provider with another identity normally
/// refuses it (401, then 502 here); one that answers anyway is believed.
#[tokio::test]
async fn a_catalog_that_names_another_participant_is_returned_as_it_is() {
    let mut catalog = played_catalog();
    catalog["participantId"] = json!("did:web:someone-else.example");
    let response = ok_response("", catalog.to_string().as_bytes());
    let answer = catalog_of_raw_provider(response).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(answer.body["participantId"], "did:web:someone-else.example");
}

/// The address is used as written: a path stays, slashes at the end go,
/// the scheme may be in capitals. An address with a user name, spaces
/// around it or no host is refused with 400 and nothing is sent.
#[tokio::test]
async fn the_address_may_have_a_path_but_no_user_name_and_no_spaces() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let capitals = base.replace("http://", "HTTP://");
    for address in [
        format!("{base}/whole"),
        format!("{base}/whole/"),
        format!("{base}/whole///"),
        format!("{capitals}/whole"),
    ] {
        let answer = post(&app, "/v1/catalog/request", &catalog_request(&address)).await;
        assert_eq!(answer.status, StatusCode::OK, "{address}: {}", answer.body);
        let request = dataset_request(&address, "urn:uuid:dataset-1");
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        assert_eq!(answer.status, StatusCode::OK, "{address}: {}", answer.body);
    }
    let sent = seen.lock().unwrap().authorizations.len();
    assert_eq!(sent, 8);

    let host = base.strip_prefix("http://").unwrap();
    for address in [
        format!("http://user:password@{host}/whole"),
        format!("http://user@{host}/whole"),
        format!(" {base}/whole"),
        "http://".to_owned(),
        "http:///whole".to_owned(),
        format!("{host}/whole"),
        format!("//{host}/whole"),
        "file:///etc/passwd".to_owned(),
        String::new(),
    ] {
        let answer = post(&app, "/v1/catalog/request", &catalog_request(&address)).await;
        let detail = problem_detail(&answer, StatusCode::BAD_REQUEST);
        assert!(detail.contains("https URL"), "{address:?}: {detail}");
        let request = dataset_request(&address, "urn:uuid:dataset-1");
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        problem_detail(&answer, StatusCode::BAD_REQUEST);
    }
    assert_eq!(seen.lock().unwrap().authorizations.len(), sent);
}

/// Whatever the dataset id holds, it reaches the provider as one path
/// segment and unchanged, and the path of the address is not left. Only
/// `.` and `..` are refused.
#[tokio::test]
async fn any_dataset_id_reaches_the_provider_unchanged() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let long = "d".repeat(4_000);
    let ids = [
        "100%",
        "a%2Fb",
        "%",
        "...",
        "../../whole/catalog/request",
        "a\\b",
        "a b",
        " a ",
        "a\tb\nc",
        "a;b=c&d=e+f",
        "https://provider.example/datasets/1?x=1#y",
        "données 数据 🗂",
        long.as_str(),
    ];
    for id in ids {
        let request = dataset_request(&format!("{base}/whole"), id);
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        assert_eq!(answer.status, StatusCode::OK, "{id:?}: {}", answer.body);
        assert_eq!(answer.body["@id"], id);
    }
    // `.` and `..` are path segments a server or proxy may resolve, so
    // they are refused and never sent
    for id in [".", ".."] {
        let request = dataset_request(&format!("{base}/whole"), id);
        let answer = post(&app, "/v1/catalog/dataset/request", &request).await;
        problem_detail(&answer, StatusCode::BAD_REQUEST);
    }
    assert_eq!(seen.lock().unwrap().authorizations.len(), ids.len());
    assert!(seen.lock().unwrap().catalog_requests.is_empty());
}

/// Many calls at once do not disturb each other: each gets its answer,
/// and each has its own token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calls_at_once_each_get_their_answer() {
    let (base, seen) = played_provider().await;
    let app = consumer_over_http(true);
    let mut running = Vec::new();
    for number in 0..24 {
        let (app, base) = (app.clone(), base.clone());
        running.push(tokio::spawn(async move {
            let id = format!("urn:uuid:dataset-{number}");
            let (uri, request) = match number % 3 {
                0 => (
                    "/v1/catalog/request",
                    catalog_request(&format!("{base}/whole")),
                ),
                1 => (
                    "/v1/catalog/dataset/request",
                    dataset_request(&format!("{base}/whole"), &id),
                ),
                _ => (
                    "/v1/catalog/request",
                    catalog_request(&format!("{base}/down")),
                ),
            };
            (number, id, post(&app, uri, &request).await)
        }));
    }
    for task in running {
        let (number, id, answer) = task.await.unwrap();
        match number % 3 {
            0 => assert_eq!(answer.body, played_catalog()),
            1 => assert_eq!(answer.body["@id"], id.as_str()),
            _ => assert_eq!(answer.status, StatusCode::BAD_GATEWAY),
        }
    }
    assert_eq!(seen.lock().unwrap().authorizations.len(), 24);
}
