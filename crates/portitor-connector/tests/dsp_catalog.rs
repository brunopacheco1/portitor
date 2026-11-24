// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Checks the HTTP responses against the official DSP 2025-1 JSON Schemas,
//! the same schemas the DSP TCK registers for its catalog tests.
#![allow(clippy::unwrap_used, missing_docs)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt as _;

mod common;
use common::{CONSUMER, NoSender, PROVIDER, authenticating, from};

const SCHEMAS: &[(&str, &str)] = &[
    (
        "common/context-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/context-schema.json"),
    ),
    (
        "common/protocol-version-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/protocol-version-schema.json"),
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
        "catalog/catalog-error-schema.json",
        include_str!("../../portitor-dsp/tests/schemas/catalog-error-schema.json"),
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

fn validator(path: &str) -> jsonschema::Validator {
    let raw = SCHEMAS.iter().find(|(p, _)| *p == path).unwrap().1;
    jsonschema::options()
        .with_retriever(Local)
        .build(&serde_json::from_str(raw).unwrap())
        .unwrap()
}

fn assert_valid(path: &str, body: &Value) {
    let v = validator(path);
    let errors: Vec<String> = v.iter_errors(body).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{path}: {errors:?}\n{body:#}");
}

/// The TCK's datasets file.
const TCK_DATASETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tck/datasets.json");

/// The TCK's datasets, as JSON entries.
fn tck_entries() -> Vec<Value> {
    serde_json::from_slice(&std::fs::read(TCK_DATASETS).unwrap()).unwrap()
}

/// A provider configuration that harvests `datasets_file`.
fn provider_config(datasets_file: Option<&str>) -> portitor_connector::Config {
    portitor_connector::Config {
        participant_id: "did:web:provider.example".into(),
        public_url: "https://provider.example".into(),
        datasets_file: datasets_file.map(Into::into),
        harvest_interval_seconds: 0,
        allow_http: false,
        db_file: "unused.db".into(),
        data_token_seconds: 3_600,
        credential_types: portitor_dsp::policy::CredentialTypes::default(),
    }
}

fn ports() -> portitor_connector::Ports {
    authenticating(common::ports(std::sync::Arc::new(NoSender)), PROVIDER)
}

fn app() -> axum::Router {
    portitor_connector::router(&provider_config(Some(TCK_DATASETS)), ports()).unwrap()
}

/// [`app`] with object storage and a push bucket: it has the receiving
/// route of push transfers (1.1). Nothing calls the store.
fn app_with_object_storage() -> axum::Router {
    let mut ports = ports();
    ports.object_storage = Some(std::sync::Arc::new(common::fake_s3::client(
        "http://127.0.0.1:9",
    )));
    portitor_connector::router(&provider_config(Some(TCK_DATASETS)), ports).unwrap()
}

async fn call(req: Request<Body>) -> (StatusCode, Value) {
    let res = app().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn post(uri: &str, body: &Value) -> Request<Body> {
    from(Request::post(uri), CONSUMER)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn met_01_01_version_endpoint() {
    let (s, body) = call(
        Request::get("/.well-known/dspace-version")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_valid("common/protocol-version-schema.json", &body);
    assert_eq!(body["protocolVersions"][0]["version"], "2025-1");
    assert_eq!(body["protocolVersions"][0]["auth"]["protocol"], "DCP");
}

#[tokio::test]
async fn the_catalog_needs_an_authenticated_caller() {
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"});
    let anonymous = Request::post("/dsp/2025-1/catalog/request")
        .header("content-type", "application/json")
        .body(Body::from(msg.to_string()))
        .unwrap();
    let (s, body) = call(anonymous).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_valid("catalog/catalog-error-schema.json", &body);
    assert!(
        body["reason"].to_string().contains("unauthorized"),
        "{body}"
    );
    let dataset = Request::get("/dsp/2025-1/catalog/datasets/CAT0102");
    let (s, _) = call(dataset.body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

/// The DCP TCK asks for the catalog in expanded JSON-LD.
#[tokio::test]
async fn expanded_catalog_requests_are_understood() {
    let msg = json!({
        "@type": "https://w3id.org/dspace/2025/1/CatalogRequestMessage",
        "https://w3id.org/dspace/2025/1/filter": {}
    });
    let (s, body) = call(post("/dsp/2025-1/catalog/request", &msg)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_valid("catalog/catalog-schema.json", &body);
}

#[tokio::test]
async fn cat_01_01_catalog_request() {
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage", "filter": []});
    let (s, body) = call(post("/dsp/2025-1/catalog/request", &msg)).await;
    assert_eq!(s, StatusCode::OK);
    assert_valid("catalog/catalog-schema.json", &body);
    assert!(
        body["dataset"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["@id"] == "CAT0101")
    );
}

#[tokio::test]
async fn cat_01_02_dataset_request() {
    let (s, body) = call(
        from(
            Request::get("/dsp/2025-1/catalog/datasets/CAT0102"),
            CONSUMER,
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_valid("catalog/dataset-schema.json", &body);
    assert_eq!(body["@id"], "CAT0102");
}

#[tokio::test]
async fn cat_01_03_dataset_not_found() {
    let (s, body) = call(
        from(Request::get("/dsp/2025-1/catalog/datasets/nope"), CONSUMER)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_valid("catalog/catalog-error-schema.json", &body);
}

/// EDC v1.0.0-RC7 sends its `QuerySpec` as the filter even when it wants
/// the whole catalog: `JsonObjectFromQuerySpecTransformer` writes `@type`,
/// `limit`, `offset`, `sortOrder` and an empty `filterExpression` in the
/// EDC namespace, and the DSP context compacts `filter` as a set (an
/// array). Such a request gets the whole catalog; one with a filter
/// expression gets 400; a filter that is one object, not an array, is not
/// the DSP shape and gets 400 too.
#[tokio::test]
async fn edc_query_specs_over_http() {
    const EDC: &str = "https://w3id.org/edc/v0.0.1/ns/";
    let query_spec = |expressions: Value| {
        json!({"@type": format!("{EDC}QuerySpec"), format!("{EDC}limit"): 50,
            format!("{EDC}offset"): 0, format!("{EDC}sortOrder"): "ASC",
            format!("{EDC}filterExpression"): expressions})
    };
    let message = |filter: Value| {
        json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "CatalogRequestMessage", "filter": filter})
    };
    // also with EDC's prefix instead of the full IRIs
    let prefixed = json!({"@type": "edc:QuerySpec", "edc:limit": 50, "edc:offset": 0,
        "edc:sortOrder": "ASC", "edc:filterExpression": []});
    for whole in [json!([query_spec(json!([]))]), json!([prefixed])] {
        let (s, body) = call(post("/dsp/2025-1/catalog/request", &message(whole))).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_valid("catalog/catalog-schema.json", &body);
        assert!(
            body["dataset"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["@id"] == "CAT0101"),
            "the whole catalog"
        );
    }
    let expression = json!([{"@type": format!("{EDC}Criterion"),
        format!("{EDC}operandLeft"): "id", format!("{EDC}operator"): "=",
        format!("{EDC}operandRight"): "CAT0101"}]);
    let (s, body) = call(post(
        "/dsp/2025-1/catalog/request",
        &message(json!([query_spec(expression)])),
    ))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_valid("catalog/catalog-error-schema.json", &body);
    assert!(
        body.to_string()
            .contains("filter expressions are not supported"),
        "{body}"
    );
    // one object instead of an array: refused as malformed
    let (s, body) = call(post(
        "/dsp/2025-1/catalog/request",
        &message(query_spec(json!([]))),
    ))
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_valid("catalog/catalog-error-schema.json", &body);
}

#[tokio::test]
async fn bad_requests_get_catalog_errors() {
    for msg in [
        json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "Catalog"}),
        json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage", "filter": [{"x": 1}]}),
        json!({"@type": "CatalogRequestMessage"}),
    ] {
        let (s, body) = call(post("/dsp/2025-1/catalog/request", &msg)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{msg}");
        assert_valid("catalog/catalog-error-schema.json", &body);
    }
}

#[test]
fn schema_check_is_not_vacuous() {
    // a catalog without participantId and with a dataset lacking hasPolicy must fail
    let bad = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@id": "c", "@type": "Catalog",
        "dataset": [{"@id": "d", "distribution": [{"format": "f", "accessService": "s"}]}]
    });
    assert!(!validator("catalog/catalog-schema.json").is_valid(&bad));
    let missing_policy = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
        "@id": "c", "@type": "Catalog", "participantId": "p",
        "dataset": [{"@id": "d", "distribution": [{"format": "f", "accessService": "s"}]}]
    });
    assert!(!validator("catalog/catalog-schema.json").is_valid(&missing_policy));
}

#[tokio::test]
async fn ids_are_stable_and_distributions_point_to_the_service() {
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"});
    let (_, first) = call(post("/dsp/2025-1/catalog/request", &msg)).await;
    let (_, second) = call(post("/dsp/2025-1/catalog/request", &msg)).await;
    assert_eq!(first["@id"], "https://provider.example/dsp/2025-1/catalog");
    assert_eq!(first["@id"], second["@id"]);
    let service = &first["service"][0]["@id"];
    assert_eq!(service, "https://provider.example/dsp/2025-1");
    for d in first["dataset"].as_array().unwrap() {
        assert_eq!(&d["distribution"][0]["accessService"], service);
    }
}

#[tokio::test]
async fn example_datasets_file_is_valid() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/datasets.json");
    let mut config = provider_config(Some(path));
    config.public_url = "https://provider.example/".into();
    let (app, negotiator) = portitor_connector::build(&config, ports()).unwrap();
    let jobs = negotiator.harvest_jobs(None, 10).unwrap();
    assert_eq!(jobs[0].record.counts.rejected, 0, "{jobs:?}");
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"});
    let res = app
        .oneshot(post("/dsp/2025-1/catalog/request", &msg))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_valid("catalog/catalog-schema.json", &body);
    assert_eq!(body["dataset"].as_array().unwrap().len(), 3);
    let weather = body["dataset"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["@id"] == "weather-stations")
        .unwrap();
    assert_eq!(weather["dct:title"], "Weather stations");
}

/// The catalog of a provider whose datasets come from `entries`, and its
/// handle.
fn harvested(entries: Vec<Value>) -> (axum::Router, portitor_connector::Negotiator) {
    let mut ports = ports();
    ports.harvest.source = Some(std::sync::Arc::new(
        portitor_connector::harvest::FixedSource::new(entries),
    ));
    portitor_connector::build(&provider_config(None), ports).unwrap()
}

/// The ids of the datasets in `app`'s catalog.
async fn catalog_ids(app: &axum::Router) -> Vec<String> {
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"});
    let res = app
        .clone()
        .oneshot(post("/dsp/2025-1/catalog/request", &msg))
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap();
    let mut ids = Vec::new();
    // an empty catalog has no `dataset` entry
    let datasets = body["dataset"].as_array().cloned().unwrap_or_default();
    for dataset in &datasets {
        ids.push(dataset["@id"].as_str().unwrap().to_owned());
    }
    ids
}

/// Until 0.6 a broken definition stopped the start; since 0.7 (ADR 0020)
/// that dataset is rejected, with the same message, and the others are
/// served.
#[tokio::test]
async fn a_broken_definition_is_rejected_and_the_others_are_served() {
    let mut entries = tck_entries();
    entries[0]["hasPolicy"] = json!([]);
    let broken = entries[0]["@id"].as_str().unwrap().to_owned();
    let (app, negotiator) = harvested(entries.clone());
    let ids = catalog_ids(&app).await;
    assert_eq!(ids.len(), entries.len() - 1);
    assert!(!ids.contains(&broken));
    let jobs = negotiator.harvest_jobs(None, 10).unwrap();
    assert_eq!(jobs[0].record.outcome.name(), "partial");
    let (_, errors) = negotiator.harvest_job(jobs[0].id).unwrap().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].position, Some(0));
    assert_eq!(errors[0].dataset_id.as_deref(), Some(broken.as_str()));
    assert!(errors[0].message.contains("no offer"), "{errors:?}");
}

/// A dataset's source is read, taken out of the dataset, and never
/// shown in the catalog; a source that is not an HTTP URL rejects that
/// dataset only.
#[tokio::test]
async fn sources_are_read_but_never_shown() {
    let mut entries = tck_entries();
    entries[0]["portitor:source"] = json!("https://upstream.internal/secret.csv");
    entries[1]["portitor:source"] = json!("file:///etc/passwd");
    let refused = entries[1]["@id"].as_str().unwrap().to_owned();
    let (app, negotiator) = harvested(entries.clone());
    let msg = json!({"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"});
    let res = app
        .clone()
        .oneshot(post("/dsp/2025-1/catalog/request", &msg))
        .await
        .unwrap();
    let shown =
        String::from_utf8(to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
    assert!(!shown.contains("upstream.internal"), "{shown}");
    assert!(!shown.contains("portitor:source"), "{shown}");
    let ids = catalog_ids(&app).await;
    assert_eq!(ids.len(), entries.len() - 1);
    assert!(!ids.contains(&refused));
    let jobs = negotiator.harvest_jobs(None, 10).unwrap();
    let (_, errors) = negotiator.harvest_job(jobs[0].id).unwrap().unwrap();
    assert_eq!(errors[0].kind.name(), "dsp");
    assert!(errors[0].message.contains("portitor:source"), "{errors:?}");
}

/// A datasets file that cannot be read, or is not a JSON array, does not
/// stop the start: the run fails and the catalog is empty (first start).
#[tokio::test]
async fn unreadable_or_malformed_files_leave_an_empty_catalog() {
    let dir = std::env::temp_dir().join(format!("portitor-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"{"@id": "x", "@type": "Catalog"}"#).unwrap();
    for file in ["/nonexistent/datasets.json", bad.to_str().unwrap()] {
        let (app, negotiator) =
            portitor_connector::build(&provider_config(Some(file)), ports()).unwrap();
        assert!(catalog_ids(&app).await.is_empty());
        let jobs = negotiator.harvest_jobs(None, 10).unwrap();
        assert_eq!(jobs[0].record.outcome.name(), "failed");
        assert!(jobs[0].record.summary.contains("datasets file"), "{jobs:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The two connector settings without a default (from 0.13).
const ID: (&str, &str) = ("PORTITOR_PARTICIPANT_ID", "did:web:p.example");
const URL: (&str, &str) = ("PORTITOR_PUBLIC_URL", "https://p.example");

/// The connector's configuration from [`ID`], [`URL`] and then `more`
/// (a name given again in `more` wins).
fn config_with(
    more: &[(&str, &str)],
) -> Result<portitor_connector::Config, portitor_connector::ConfigError> {
    let mut all = vec![ID, URL];
    all.extend_from_slice(more);
    portitor_connector::Config::from_vars(portitor_runtime::testing::vars(&all))
}

/// The id must be a did:web DID and the URL an http(s) URL, as for the
/// identity component and the issuer: a wrong one would only fail later,
/// in front of a partner. A switch is `true` or `false`.
#[test]
fn a_wrong_id_url_or_switch_stops_the_start() {
    for (name, value) in [
        ("PORTITOR_PARTICIPANT_ID", "p.example"),
        ("PORTITOR_PARTICIPANT_ID", "did:key:z6Mk"),
        ("PORTITOR_PUBLIC_URL", "p.example"),
        ("PORTITOR_PUBLIC_URL", "ftp://p.example"),
        ("PORTITOR_PUBLIC_URL", "https://p.example/dsp?x=1"),
        ("PORTITOR_PUBLIC_URL", "https://p.example#x"),
        ("PORTITOR_ALLOW_HTTP", "1"),
        ("PORTITOR_ALLOW_HTTP", "yes"),
    ] {
        let error = config_with(&[(name, value)]).unwrap_err();
        assert!(
            matches!(&error, portitor_connector::ConfigError::Refused(n, _) if *n == name),
            "{name}={value}: {error}"
        );
        assert!(error.to_string().starts_with(name), "{error}");
    }
    for (name, value) in [
        ("PORTITOR_PUBLIC_URL", "http://localhost:8080/base/"),
        ("PORTITOR_PARTICIPANT_ID", "did:web:p.example:connector"),
        ("PORTITOR_ALLOW_HTTP", "False"),
    ] {
        assert!(config_with(&[(name, value)]).is_ok(), "{name}={value}");
    }
}

/// The rule of every setting: spaces around a value are dropped, and an
/// empty value means "not set", so the default is used.
#[test]
fn an_empty_setting_is_not_set_and_values_are_trimmed() {
    let read = config_with;
    let defaults = read(&[]).unwrap();
    let trimmed = read(&[
        ("PORTITOR_PARTICIPANT_ID", " did:web:q.example "),
        ("PORTITOR_DB_FILE", " /data/y.db\n"),
        ("PORTITOR_DATA_TOKEN_SECONDS", " 60 "),
        ("PORTITOR_ALLOW_HTTP", " true "),
    ])
    .unwrap();
    assert_eq!(trimmed.participant_id, "did:web:q.example");
    assert_eq!(trimmed.db_file, std::path::PathBuf::from("/data/y.db"));
    assert_eq!(trimmed.data_token_seconds, 60);
    assert!(trimmed.allow_http);
    let empty = read(&[
        ("PORTITOR_DATASETS_FILE", ""),
        ("PORTITOR_DB_FILE", "  "),
        ("PORTITOR_HARVEST_INTERVAL_SECONDS", ""),
        ("PORTITOR_DATA_TOKEN_SECONDS", " "),
        ("PORTITOR_ALLOW_HTTP", ""),
        ("PORTITOR_POLICY_CREDENTIALS", ""),
        // a removed name that is empty is not set either
        ("PORTITOR_DATASETS", ""),
    ])
    .unwrap();
    assert_eq!(empty.datasets_file, None);
    assert_eq!(empty.db_file, defaults.db_file);
    assert_eq!(empty.harvest_interval_seconds, 3_600);
    assert_eq!(empty.data_token_seconds, 3_600);
    assert!(!empty.allow_http);
    assert_eq!(empty.credential_types, defaults.credential_types);
}

#[test]
fn config_from_variables() {
    use portitor_runtime::testing::vars;
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/datasets.json");

    let read = config_with;

    // the id and the URL are required: nothing is assumed for them
    for (given, missing) in [
        (vec![], "PORTITOR_PARTICIPANT_ID"),
        (vec![URL], "PORTITOR_PARTICIPANT_ID"),
        (vec![ID], "PORTITOR_PUBLIC_URL"),
        (
            vec![ID, ("PORTITOR_PUBLIC_URL", " ")],
            "PORTITOR_PUBLIC_URL",
        ),
    ] {
        let error = portitor_connector::Config::from_vars(vars(&given)).unwrap_err();
        let expected = format!("{missing}: required");
        assert!(error.to_string().starts_with(&expected), "{error}");
    }

    let defaults = read(&[]).unwrap();
    assert_eq!(defaults.participant_id, "did:web:p.example");
    assert_eq!(defaults.public_url, "https://p.example");
    assert_eq!(defaults.datasets_file, None);
    assert_eq!(defaults.harvest_interval_seconds, 3_600);
    assert!(!defaults.allow_http);
    assert_eq!(defaults.db_file, std::path::PathBuf::from("portitor.db"));

    let set = read(&[
        ("PORTITOR_DATASETS_FILE", file),
        ("PORTITOR_HARVEST_INTERVAL_SECONDS", "0"),
        ("PORTITOR_ALLOW_HTTP", "TRUE"),
        ("PORTITOR_DB_FILE", "/data/x.db"),
    ])
    .unwrap();
    assert_eq!(set.participant_id, "did:web:p.example");
    assert!(set.allow_http);
    assert_eq!(set.db_file, std::path::PathBuf::from("/data/x.db"));
    assert_eq!(set.datasets_file, Some(std::path::PathBuf::from(file)));
    assert_eq!(set.harvest_interval_seconds, 0);

    let removed = read(&[("PORTITOR_DATASETS", "a,b")]).unwrap_err();
    assert!(removed.to_string().contains("PORTITOR_DATASETS_FILE"));
    // renamed in 0.13: the old name stops the start and names the new one
    let renamed = read(&[("PORTITOR_ALLOW_HTTP_CALLBACKS", "true")]).unwrap_err();
    assert_eq!(
        renamed.to_string(),
        "PORTITOR_ALLOW_HTTP_CALLBACKS was renamed in 0.13; set PORTITOR_ALLOW_HTTP instead"
    );

    // above one year: refused, as a schedule that far ahead overflows the
    // clock (and would abort the connector) instead of a config error
    let too_large = ["31536001", "18446744073709551615"];
    for bad in ["hourly", "-1", "1 h"].into_iter().chain(too_large) {
        for name in [
            "PORTITOR_HARVEST_INTERVAL_SECONDS",
            "PORTITOR_DATA_TOKEN_SECONDS",
        ] {
            let invalid = read(&[(name, bad)]).unwrap_err();
            assert!(
                invalid.to_string().contains(name),
                "{name}={bad}: {invalid}"
            );
        }
    }
    let one_year = read(&[
        ("PORTITOR_HARVEST_INTERVAL_SECONDS", "31536000"),
        ("PORTITOR_DATA_TOKEN_SECONDS", "31536000"),
    ])
    .unwrap();
    assert_eq!(
        one_year.harvest_interval_seconds,
        portitor_connector::MAX_SETTING_SECONDS
    );
    assert_eq!(one_year.data_token_seconds, 31_536_000);
    // a token must live at least one second
    assert!(read(&[("PORTITOR_DATA_TOKEN_SECONDS", "0")]).is_err());

    // the file is read by the harvest, not here: a missing file is no
    // configuration error (since 0.7)
    let missing = read(&[("PORTITOR_DATASETS_FILE", "/nonexistent.json")]).unwrap();
    assert!(missing.datasets_file.is_some());
}

/// Every operation in docs/api/openapi.json is routed: the router answers
/// neither 405 nor its own empty 404.
#[tokio::test]
async fn openapi_paths_are_served() {
    let raw = include_str!("../../../docs/api/openapi.json");
    let spec: Value = serde_json::from_str(raw).unwrap();
    let app = app_with_object_storage();
    let mut count = 0;
    for (path, ops) in spec["paths"].as_object().unwrap() {
        let uri = path
            .replace("{id}", "CAT0101")
            .replace("{providerPid}", "urn:uuid:x")
            .replace("{consumerPid}", "urn:uuid:y");
        for method in ops.as_object().unwrap().keys() {
            let req = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&uri)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert!(
                status != StatusCode::NOT_FOUND || !bytes.is_empty(),
                "{method} {path} is not routed"
            );
            count += 1;
        }
    }
    assert_eq!(count, 29);
}
