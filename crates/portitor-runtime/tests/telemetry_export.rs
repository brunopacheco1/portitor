// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Traces and metrics leave the process only when an OTLP endpoint is set
//! (ADR 0023): a fake OTLP/HTTP receiver in this test takes `POST
//! /v1/traces` and `POST /v1/metrics`, and the work is done by a child
//! process (this same test binary, test [`child`]), because
//! `telemetry::init` sets process-wide state once and reads the
//! environment, which a test cannot change in its own process without
//! `unsafe`.
//!
//! The child serves two routes and calls them with Portitor's HTTP
//! client: `/items/{id}` like a DSP endpoint (no trace context either
//! way) and `/sts/token` like the STS (trace context sent, and taken once
//! the client is authenticated). It sends an `Authorization` header and a
//! client secret, which must appear in no export. It also sends a foreign
//! `traceparent` to both routes without a secret: neither may join that
//! trace.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use opentelemetry_proto::tonic::trace::v1::Span;
use prost::Message as _;
use tracing::Instrument as _;

/// Set in the child's environment: run the child's work, in this mode
/// (`normal` or `failing`).
const CHILD_MARKER: &str = "PORTITOR_TELEMETRY_TEST_CHILD";

/// A secret the child sends; it must never be exported.
const SECRET: &str = "s3cret-t0ken-value";

/// A `traceparent` from someone else (the example of the W3C
/// recommendation); no exported span may be in its trace.
const FOREIGN_TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// The trace id in [`FOREIGN_TRACEPARENT`], as bytes.
const FOREIGN_TRACE_ID: [u8; 16] = [
    0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80, 0x31, 0x9c,
];

/// What the fake receiver got: the path, the content type and the body
/// of each request.
type Received = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

/// The fake receiver's handler: keeps what came, answers with an empty
/// body (a valid OTLP answer).
async fn record(
    State(received): State<Received>,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> &'static str {
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    received
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), content_type, body.to_vec()));
    ""
}

/// Starts the fake OTLP/HTTP receiver; returns its base URL.
async fn receiver(received: Received) -> String {
    let app = Router::new()
        .route("/v1/traces", post(record))
        .route("/v1/metrics", post(record))
        .with_state(received);
    serve_in_background(app).await
}

/// A receiver that always answers HTTP 500.
async fn failing_receiver() -> String {
    let app = Router::new().fallback(|| async { StatusCode::INTERNAL_SERVER_ERROR });
    serve_in_background(app).await
}

/// A receiver that takes connections and never answers.
async fn silent_receiver() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            // kept open, never read or answered
            open.push(socket);
        }
    });
    format!("http://{address}")
}

/// An address where nobody listens.
fn closed_receiver() -> String {
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    format!("http://{address}")
}

async fn serve_in_background(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}")
}

/// Runs the child in `mode` with the `endpoints` (variable name and
/// value) set, returns its standard output (the JSON log lines).
async fn run_child(mode: &str, endpoints: &[(&str, String)]) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_MARKER, mode)
        .env_remove("OTEL_SERVICE_NAME")
        .env_remove("OTEL_RESOURCE_ATTRIBUTES")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")
        // export while the child works, not only at its end
        .env("OTEL_BSP_SCHEDULE_DELAY", "100")
        .env("OTEL_METRIC_EXPORT_INTERVAL", "200")
        .env("RUST_LOG", "info");
    for (name, value) in endpoints {
        command.env(name, value);
    }
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

/// The value of the attribute `key`, as text.
fn text(attributes: &[KeyValue], key: &str) -> Option<String> {
    // each `?` stops at a missing part (no such attribute, no value) and
    // gives `None`, as an early `return None` would
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

/// One metric data point: the metric's name and its attributes.
type Point = (String, Vec<KeyValue>);

/// The spans and the metric data points of every export; checks on the
/// way that each is Protocol Buffers, from our service, and holds no
/// secret.
fn decode(received: &[(String, String, Vec<u8>)]) -> (Vec<Span>, Vec<Point>) {
    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
    let mut spans: Vec<Span> = Vec::new();
    let mut points: Vec<Point> = Vec::new();
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
                            let attribute_sets: Vec<Vec<KeyValue>> = match metric.data {
                                Some(Data::Histogram(h)) => {
                                    h.data_points.into_iter().map(|p| p.attributes).collect()
                                }
                                Some(Data::Sum(s)) => {
                                    s.data_points.into_iter().map(|p| p.attributes).collect()
                                }
                                _ => vec![Vec::new()],
                            };
                            for attributes in attribute_sets {
                                points.push((metric.name.clone(), attributes));
                            }
                        }
                    }
                }
            }
            other => panic!("unexpected export to {other}"),
        }
    }
    (spans, points)
}

#[tokio::test(flavor = "multi_thread")]
async fn spans_and_metrics_are_exported_only_with_an_endpoint() {
    let received = Received::default();
    let endpoint = receiver(Arc::clone(&received)).await;

    // without an endpoint: nothing is sent, the logs are as before
    let stdout = run_child("normal", &[]).await;
    assert!(
        received.lock().unwrap().is_empty(),
        "sent without an endpoint"
    );
    let lines = lines_in_span(&stdout);
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(!lines[0].contains("trace_id"), "{}", lines[0]);
    assert!(!lines[0].contains("\"span\""), "{}", lines[0]);

    // with an endpoint
    let stdout = run_child("normal", &[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)]).await;
    let lines = lines_in_span(&stdout);
    assert_eq!(lines.len(), 1, "{stdout}");
    assert!(lines[0].starts_with("{\"trace_id\":\""), "{}", lines[0]);
    let received = received.lock().unwrap().clone();
    let (spans, points) = decode(&received);

    // nobody joined the foreign trace, on any route
    for span in &spans {
        assert_ne!(span.trace_id, FOREIGN_TRACE_ID, "{span:?}");
    }
    let work = spans
        .iter()
        .find(|span| span.name == "work")
        .expect("the work span");
    // the server spans: the route template, never the path with its id
    let items: Vec<&Span> = spans
        .iter()
        .filter(|span| span.name == "GET /items/{id}")
        .collect();
    assert_eq!(items.len(), 3, "{spans:?}");
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
    let sts_spans: Vec<&Span> = spans
        .iter()
        .filter(|span| span.name == "POST /sts/token")
        .collect();
    assert_eq!(sts_spans.len(), 2, "{spans:?}");
    let status_of = |span: &Span| text(&span.attributes, "http.response.status_code");
    let refused = sts_spans
        .iter()
        .find(|span| status_of(span).as_deref() == Some("401"))
        .expect("the refused STS call");
    // not authenticated: a trace of its own
    assert!(refused.parent_span_id.is_empty(), "{refused:?}");
    let sts = sts_spans
        .iter()
        .find(|span| status_of(span).as_deref() == Some("200"))
        .expect("the STS call");
    // the client spans, one per call of the HTTP client, in the work
    // span's trace
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
    // the STS call carried our trace context and the client was
    // authenticated: one trace, the server span a child of the client span
    assert_eq!(sts.trace_id, sts_client.trace_id);
    assert_eq!(sts.parent_span_id, sts_client.span_id);
    // the state event is a span event of the work span
    assert!(
        work.events
            .iter()
            .any(|event| event.name == "negotiation state changed"),
        "{work:?}"
    );

    check_measures(&points);
}

/// The measures of the `normal` child.
fn check_measures(points: &[Point]) {
    for name in [
        "http.server.request.duration",
        "http.client.request.duration",
        "portitor.negotiation.ended",
    ] {
        assert!(
            points.iter().any(|(n, _)| n == name),
            "{name} in {points:?}"
        );
    }
    // the client measure names the server only for our own service (the
    // STS call); calls to others carry no host or port
    for (name, attributes) in points {
        if name != "http.client.request.duration" {
            continue;
        }
        let address = text(attributes, "server.address");
        let method = text(attributes, "http.request.method");
        if method.as_deref() == Some("POST") {
            assert_eq!(address.as_deref(), Some("127.0.0.1"));
        } else {
            assert_eq!(address, None, "{attributes:?}");
            assert_eq!(text(attributes, "server.port"), None);
        }
    }
}

/// `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or `..._METRICS_ENDPOINT` alone
/// switch export on, for that signal only.
#[tokio::test(flavor = "multi_thread")]
async fn a_signal_endpoint_alone_exports_that_signal() {
    let received = Received::default();
    let base = receiver(Arc::clone(&received)).await;
    // a signal's own endpoint is the full URL, path included
    let stdout = run_child(
        "normal",
        &[(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            format!("{base}/v1/traces"),
        )],
    )
    .await;
    let paths: Vec<String> = received
        .lock()
        .unwrap()
        .iter()
        .map(|(path, _, _)| path.clone())
        .collect();
    assert!(paths.iter().any(|p| p == "/v1/traces"), "{paths:?}");
    assert!(!paths.iter().any(|p| p == "/v1/metrics"), "{paths:?}");
    assert!(lines_in_span(&stdout)[0].contains("trace_id"), "{stdout}");

    received.lock().unwrap().clear();
    let stdout = run_child(
        "normal",
        &[(
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            format!("{base}/v1/metrics"),
        )],
    )
    .await;
    let paths: Vec<String> = received
        .lock()
        .unwrap()
        .iter()
        .map(|(path, _, _)| path.clone())
        .collect();
    assert!(paths.iter().any(|p| p == "/v1/metrics"), "{paths:?}");
    assert!(!paths.iter().any(|p| p == "/v1/traces"), "{paths:?}");
    // no traces: the log lines carry no trace id
    assert!(!lines_in_span(&stdout)[0].contains("trace_id"), "{stdout}");
}

/// A receiver that is down, answers 5xx or never answers: the child keeps
/// serving, and its shutdown ends within the budget (the child checks
/// both, see `failing_work`).
#[tokio::test(flavor = "multi_thread")]
async fn a_failing_receiver_does_not_stop_the_process() {
    for endpoint in [
        closed_receiver(),
        failing_receiver().await,
        silent_receiver().await,
    ] {
        let started = Instant::now();
        let stdout = run_child(
            "failing",
            &[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.clone())],
        )
        .await;
        assert!(stdout.contains("served while export failed"), "{stdout}");
        // the child's own run is short; a few seconds more for starting
        // the process on a busy machine
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{endpoint}: {:?}",
            started.elapsed()
        );
    }
}

/// The routes of the child: `/items/{id}` answers with the
/// `traceparent` it got, if any.
async fn item_answer(headers: HeaderMap) -> String {
    headers
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// `/sts/token` of the child: like the identity's STS, it joins the
/// caller's trace only once the client secret is right.
async fn sts_answer(
    caller_trace: Option<axum::Extension<portitor_runtime::telemetry::http::CallerTrace>>,
    form: String,
) -> StatusCode {
    if !form.contains(&format!("client_secret={SECRET}")) {
        return StatusCode::UNAUTHORIZED;
    }
    if let Some(axum::Extension(trace)) = &caller_trace {
        let _joined = trace.join();
        tracing::info!("the STS client is authenticated");
    }
    StatusCode::OK
}

/// The child's server; returns its base URL.
async fn child_server() -> String {
    use portitor_runtime::telemetry::http::observe_requests;
    let app = Router::new()
        .route("/items/{id}", get(item_answer))
        .route("/sts/token", post(sts_answer));
    serve_in_background(observe_requests(app, &["/sts/token"])).await
}

/// Sends `request` with a plain client (no span, our own headers).
async fn send_raw(request: hyper::Request<http_body_util::Full<hyper::body::Bytes>>) -> u16 {
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http();
    client.request(request).await.unwrap().status().as_u16()
}

/// The child's work (see the module doc); does nothing unless started by
/// the tests above.
#[tokio::test(flavor = "multi_thread")]
async fn child() {
    use portitor_runtime::telemetry;

    let Some(mode) = std::env::var_os(CHILD_MARKER) else {
        return;
    };
    let telemetry = telemetry::init("portitor-telemetry-test");
    let base = child_server().await;
    if mode == "failing" {
        failing_work(&base).await;
        let started = Instant::now();
        telemetry.shutdown().await;
        // both providers stop at the same time, each within the budget
        let most = telemetry::SHUTDOWN_TIMEOUT + Duration::from_secs(1);
        assert!(started.elapsed() < most, "{:?}", started.elapsed());
        return;
    }
    // only spans whose target starts with `portitor` are exported
    normal_work(&base)
        .instrument(tracing::info_span!(target: "portitor_test", "work"))
        .await;
    telemetry.shutdown().await;
}

/// The calls of the `normal` child.
async fn normal_work(base: &str) {
    use portitor_runtime::http::{HttpSender, Sender as _};
    use portitor_runtime::telemetry::{STATE_EVENTS, metrics};

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
    let sts = HttpSender::new(true).unwrap().for_own_service();
    let form = format!("grant_type=client_credentials&client_secret={SECRET}");
    let answer = sts.post_form(&format!("{base}/sts/token"), form).await;
    // the answer is not JSON: read as `null`, the call itself worked
    assert!(answer.is_ok(), "{answer:?}");
    // someone else's trace context, on a DSP-like route and on the STS
    // without the secret: both ignore it
    for (method, path) in [("GET", "/items/7"), ("POST", "/sts/token")] {
        let request = hyper::Request::builder()
            .method(method)
            .uri(format!("{base}{path}"))
            .header("traceparent", FOREIGN_TRACEPARENT)
            .body(http_body_util::Full::new(hyper::body::Bytes::from(
                "grant_type=client_credentials",
            )))
            .unwrap();
        send_raw(request).await;
    }
    tracing::event!(
        target: STATE_EVENTS,
        tracing::Level::INFO,
        portitor.dsp.provider_pid = "urn:uuid:p",
        portitor.dsp.state = "FINALIZED",
        "negotiation state changed"
    );
    metrics::negotiation_ended(metrics::Role::Provider, "FINALIZED");
}

/// The calls of the `failing` child: serves while the exports fail.
async fn failing_work(base: &str) {
    use portitor_runtime::http::{HttpSender, Sender as _};
    let client = HttpSender::new(true).unwrap();
    for round in 0..3 {
        let span = tracing::info_span!(target: "portitor_test", "round", round);
        let answer = client
            .get(&format!("{base}/items/{round}"))
            .instrument(span)
            .await;
        assert!(answer.is_ok(), "{answer:?}");
        // the exports run (and fail) meanwhile
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    println!("served while export failed");
}
