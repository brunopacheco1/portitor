// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Traces and metrics leave the process only when
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is set (ADR 0023): a fake OTLP/HTTP
//! receiver in this test takes `POST /v1/traces` and `POST /v1/metrics`,
//! and the work is done by a child process (this same test binary, test
//! [`child`]), because `telemetry::init` sets process-wide state once and
//! reads the environment, which a test cannot change in its own process
//! without `unsafe`.
//!
//! The child serves two routes and calls them with Portitor's HTTP
//! client: `/items/{id}` like a DSP endpoint (no trace context either
//! way) and `/sts/token` like the STS (trace context sent and taken). It
//! sends an `Authorization` header and a client secret, which must appear
//! in no export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use opentelemetry_proto::tonic::trace::v1::Span;
use prost::Message as _;

/// Set in the child's environment: run the child's work.
const CHILD_MARKER: &str = "PORTITOR_TELEMETRY_TEST_CHILD";

/// A secret the child sends; it must never be exported.
const SECRET: &str = "s3cret-t0ken-value";

/// What the fake receiver got: the path, the content type and the body
/// of each request.
type Received = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

/// Starts the fake OTLP/HTTP receiver; returns its base URL.
async fn receiver(received: Received) -> String {
    let record = |path: &'static str, received: Received| {
        move |headers: HeaderMap, body: axum::body::Bytes| async move {
            let content_type = headers
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            received
                .lock()
                .unwrap()
                .push((path.to_owned(), content_type, body.to_vec()));
            // an empty export answer is a valid OTLP answer
            ""
        }
    };
    let app = Router::new()
        .route(
            "/v1/traces",
            post(record("/v1/traces", Arc::clone(&received))),
        )
        .route("/v1/metrics", post(record("/v1/metrics", received)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}")
}

/// Runs the child with `endpoint` (or none), returns its standard output
/// (the JSON log lines).
async fn run_child(endpoint: Option<String>) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, "1")
        .env_remove("OTEL_SERVICE_NAME")
        .env_remove("OTEL_RESOURCE_ATTRIBUTES")
        .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")
        .env("RUST_LOG", "info");
    match endpoint {
        Some(endpoint) => command.env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint),
        None => command.env_remove("OTEL_EXPORTER_OTLP_ENDPOINT"),
    };
    let output = tokio::task::spawn_blocking(move || command.output())
        .await
        .unwrap()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

fn text(attributes: &[KeyValue], key: &str) -> Option<String> {
    let attribute = attributes.iter().find(|a| a.key == key)?;
    match attribute.value.as_ref()?.value.as_ref()? {
        AnyValue::StringValue(value) => Some(value.clone()),
        AnyValue::IntValue(value) => Some(value.to_string()),
        other => Some(format!("{other:?}")),
    }
}

/// The log lines the child wrote inside its `work` span.
fn lines_in_span(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| line.contains("inside the work span"))
        .collect()
}

/// The spans and the metric names of every export; checks on the way
/// that each is Protocol Buffers, from our service, and holds no secret.
fn decode(received: &[(String, String, Vec<u8>)]) -> (Vec<Span>, Vec<String>) {
    let mut spans: Vec<Span> = Vec::new();
    let mut metric_names: Vec<String> = Vec::new();
    for (path, content_type, body) in received {
        assert_eq!(content_type, "application/x-protobuf", "{path}");
        // no secret in any export, in whatever field
        let as_text = String::from_utf8_lossy(body);
        assert!(!as_text.contains(SECRET), "{path} holds the secret");
        assert!(
            !as_text.contains("Bearer"),
            "{path} holds an Authorization value"
        );
        match path.as_str() {
            "/v1/traces" => {
                let export = ExportTraceServiceRequest::decode(body.as_slice()).unwrap();
                for resource in export.resource_spans {
                    let attributes = resource.resource.unwrap().attributes;
                    assert_eq!(
                        text(&attributes, "service.name").as_deref(),
                        Some("portitor-telemetry-test")
                    );
                    for scope in resource.scope_spans {
                        spans.extend(scope.spans);
                    }
                }
            }
            "/v1/metrics" => {
                let export = ExportMetricsServiceRequest::decode(body.as_slice()).unwrap();
                for resource in export.resource_metrics {
                    for scope in resource.scope_metrics {
                        for metric in scope.metrics {
                            metric_names.push(metric.name);
                        }
                    }
                }
            }
            other => panic!("unexpected export to {other}"),
        }
    }

    (spans, metric_names)
}

#[tokio::test(flavor = "multi_thread")]
async fn spans_and_metrics_are_exported_only_with_an_endpoint() {
    let received = Received::default();
    let endpoint = receiver(Arc::clone(&received)).await;

    // without an endpoint: nothing is sent, the logs are as before
    let stdout = run_child(None).await;
    assert!(
        received.lock().unwrap().is_empty(),
        "sent without an endpoint"
    );
    let lines = lines_in_span(&stdout);
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(!lines[0].contains("trace_id"), "{}", lines[0]);
    assert!(!lines[0].contains("\"span\""), "{}", lines[0]);

    // with an endpoint
    let stdout = run_child(Some(endpoint)).await;
    let lines = lines_in_span(&stdout);
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(lines[0].starts_with("{\"trace_id\":\""), "{}", lines[0]);
    let received = received.lock().unwrap().clone();
    let (spans, metric_names) = decode(&received);

    let work = spans
        .iter()
        .find(|span| span.name == "work")
        .expect("the work span");
    // the server spans: the route template, never the path with its id
    let items: Vec<&Span> = spans
        .iter()
        .filter(|span| span.name == "GET /items/{id}")
        .collect();
    assert_eq!(items.len(), 2, "{spans:?}");
    for item in &items {
        assert_eq!(
            text(&item.attributes, "http.route").as_deref(),
            Some("/items/{id}")
        );
        assert_eq!(
            text(&item.attributes, "http.response.status_code").as_deref(),
            Some("200")
        );
        // no trace context crossed: the server started its own trace
        assert_ne!(item.trace_id, work.trace_id);
        assert!(item.parent_span_id.is_empty());
    }
    for span in &spans {
        for attribute in &span.attributes {
            let value = text(std::slice::from_ref(attribute), &attribute.key).unwrap_or_default();
            assert!(!value.contains("/items/42"), "{}: {value}", attribute.key);
        }
    }
    let sts = spans
        .iter()
        .find(|span| span.name == "POST /sts/token")
        .expect("a span for POST /sts/token");
    // the client spans, one per call, in the work span's trace
    let clients: Vec<&Span> = spans
        .iter()
        .filter(|span| span.name == "GET" || span.name == "POST")
        .collect();
    assert_eq!(clients.len(), 3, "{spans:?}");
    for client in &clients {
        assert_eq!(
            text(&client.attributes, "server.address").as_deref(),
            Some("127.0.0.1")
        );
        assert!(text(&client.attributes, "server.port").is_some());
        assert_eq!(client.trace_id, work.trace_id);
    }
    let sts_client = clients.iter().find(|span| span.name == "POST").unwrap();
    // the STS call carried our trace context: one trace, the server span
    // a child of the client span
    assert_eq!(sts.trace_id, sts_client.trace_id);
    assert_eq!(sts.parent_span_id, sts_client.span_id);
    // the state event is a span event of the work span
    assert!(
        work.events
            .iter()
            .any(|event| event.name == "negotiation state changed"),
        "{work:?}"
    );

    for name in [
        "http.server.request.duration",
        "http.client.request.duration",
        "portitor.negotiation.ended",
    ] {
        assert!(
            metric_names.iter().any(|n| n == name),
            "{name} in {metric_names:?}"
        );
    }
}

/// The child's work (see the module doc); does nothing unless started by
/// the test above.
#[tokio::test(flavor = "multi_thread")]
async fn child() {
    use portitor_runtime::http::{HttpSender, Sender as _};
    use portitor_runtime::telemetry::{self, STATE_EVENTS, http::observe_requests, metrics};
    use tracing::Instrument as _;

    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let telemetry = telemetry::init("portitor-telemetry-test");
    // both routes answer with the traceparent they got, if any
    let traceparent = |headers: HeaderMap| async move {
        headers
            .get("traceparent")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let app = Router::new()
        .route("/items/{id}", get(traceparent))
        .route("/sts/token", post(traceparent));
    let app = observe_requests(app, &["/sts/token"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    let work = async {
        tracing::info!("inside the work span");
        // like a DSP call: an Authorization header, no trace context
        let dsp = HttpSender::new(true).unwrap();
        let authorization = format!("Bearer {SECRET}");
        let answer = dsp
            .get_text(&format!("{base}/items/42"), "text/plain", 1024)
            .await;
        assert_eq!(answer.unwrap(), "", "no traceparent to another participant");
        let _ = dsp
            .get_authorized(&format!("{base}/items/42"), &authorization)
            .await;
        // like the STS: a client secret in the form, trace context sent
        let sts = HttpSender::new(true).unwrap().sending_trace_context();
        let form = format!("grant_type=client_credentials&client_secret={SECRET}");
        let answer = sts.post_form(&format!("{base}/sts/token"), form).await;
        // the answer is not JSON: read as `null`, the call itself worked
        assert!(answer.is_ok(), "{answer:?}");
        tracing::event!(
            target: STATE_EVENTS,
            tracing::Level::INFO,
            portitor.dsp.provider_pid = "urn:uuid:p",
            portitor.dsp.state = "FINALIZED",
            "negotiation state changed"
        );
        metrics::negotiation_ended(metrics::Role::Provider, "FINALIZED");
    };
    // only spans whose target starts with `portitor` are exported
    work.instrument(tracing::info_span!(target: "portitor_test", "work"))
        .await;
    telemetry.shutdown().await;
}
