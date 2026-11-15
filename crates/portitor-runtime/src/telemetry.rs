// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Logs, traces and metrics (ADR 0023). Every Portitor binary calls
//! [`init`] first thing in `main`, and [`Telemetry::shutdown`] when it
//! stops.
//!
//! - Logs: JSON lines on standard output, level from `RUST_LOG` (default
//!   `info`), as before 0.9.
//! - Traces and metrics: exported with OpenTelemetry (OTLP over HTTP with
//!   Protocol Buffers) only when an endpoint is set:
//!   `OTEL_EXPORTER_OTLP_ENDPOINT` for both, or
//!   `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` / `..._METRICS_ENDPOINT` for
//!   one. The other `OTEL_*` variables the SDK reads are listed in
//!   `docs/DEPLOYING.md`. When traces are exported, each JSON log line
//!   written inside a span also carries that span's `trace_id`.
//!
//! All OpenTelemetry code of Portitor lives in this module and its two
//! parts, so the API changes of the 0.x crates stay in one place:
//!
//! - [`http`]: one span and one duration measure per HTTP request, served
//!   ([`http::observe_requests`]) or sent ([`http::ClientCall`]), and the
//!   W3C trace context rule (ADR 0023, section 4).
//! - [`metrics`]: Portitor's own measures (negotiations ended, harvest
//!   runs, ...), each a plain function the binaries call.
//!
//! What goes into an exported span is an allow-list: spans whose target
//! starts with `portitor` (their fields are chosen in this code, never a
//! token, a key, a body or dataset contents), and as span events only the
//! events with the target [`STATE_EVENTS`]. Other log lines are not copied
//! into traces; the `trace_id` in them leads from a trace to its logs.

use std::time::Duration;

use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
use opentelemetry::{Key, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::resource::{EnvResourceDetector, ResourceDetector as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::FilterExt as _;
use tracing_subscriber::fmt::format::{Format, Json, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;

pub mod http;
pub mod metrics;

/// The target of the events that are copied into the exported span as
/// span events: the state changes of negotiations and transfers. Write
/// them as `tracing::event!(target: STATE_EVENTS, tracing::Level::INFO,
/// ...)`: `tracing::info!(target: ...)` does not parse when the first
/// field name has dots (`portitor.dsp.state = ...`). They are log lines
/// too. Only fields that may leave the participant go into them
/// (process ids, states).
pub const STATE_EVENTS: &str = "portitor::state";

/// The variable that switches export on for traces and metrics
/// (OpenTelemetry's own name).
const ENDPOINT_VARIABLE: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// The variable that switches export on for traces only.
const TRACES_ENDPOINT_VARIABLE: &str = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT";

/// The variable that switches export on for metrics only.
const METRICS_ENDPOINT_VARIABLE: &str = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT";

/// How long one export may take, from sending until the answer is read.
/// The same as OpenTelemetry's default export timeout
/// (`OTEL_EXPORTER_OTLP_TIMEOUT`, 10 s), which the SDK applies too.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest answer read from the OTLP receiver. A receiver answers
/// with a small, often empty, message.
const MAX_EXPORT_ANSWER: usize = 64 * 1024;

/// How long [`Telemetry::shutdown`] waits for the providers' last
/// exports. The two providers stop at the same time, so the whole
/// shutdown takes about this long at worst: well inside the 10 s that
/// `docker stop` waits before it kills the process, also when the
/// receiver is down or never answers.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// What [`init`] started, to flush and stop when the process stops: the
/// trace and metric providers, when export is on.
///
/// Call [`Telemetry::shutdown`] after the servers stopped, so the spans
/// and measures of the last requests are sent. Dropping a `Telemetry`
/// without it loses them (but stops nothing else).
#[must_use = "call shutdown() when the process stops, to send the last spans and metrics"]
pub struct Telemetry {
    tracer: Option<SdkTracerProvider>,
    meter: Option<SdkMeterProvider>,
}

impl std::fmt::Debug for Telemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Telemetry")
            .field("export", &self.exporting())
            .finish()
    }
}

impl Telemetry {
    /// Whether traces and metrics are exported.
    #[must_use]
    pub fn exporting(&self) -> bool {
        self.tracer.is_some() || self.meter.is_some()
    }

    /// No export: what [`init`] gives when no endpoint is set or export
    /// cannot start.
    pub fn off() -> Self {
        Self {
            tracer: None,
            meter: None,
        }
    }

    /// Sends what is still buffered and stops the providers. Both stop at
    /// the same time, and the wait ends after [`SHUTDOWN_TIMEOUT`] at
    /// most; a failure is logged and nothing else happens.
    ///
    /// The providers' `shutdown` blocks its thread until the last export
    /// is done, and that export runs as a task on this tokio runtime
    /// ([`OnTokio`]). So it is called on a thread of its own, never on one
    /// of the runtime's threads, which could otherwise all be waiting. A
    /// plain thread rather than `spawn_blocking`: when the time is up, the
    /// process may end without waiting for it (tokio waits for its
    /// blocking threads when the runtime stops).
    pub async fn shutdown(self) {
        let Self { tracer, meter } = self;
        let mut stopping = Vec::new();
        if let Some(tracer) = tracer {
            let stop = move || tracer.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
            stopping.push(("spans", stop_on_own_thread(stop)));
        }
        if let Some(meter) = meter {
            // the meter provider takes a timeout too, but 0.33 does not
            // apply it yet; `stop_on_own_thread` bounds the wait instead
            let stop = move || meter.shutdown_with_timeout(SHUTDOWN_TIMEOUT);
            stopping.push(("metrics", stop_on_own_thread(stop)));
        }
        // both run already; wait for each in turn, until one shared
        // deadline, so the whole wait stays within the budget
        let deadline = tokio::time::Instant::now() + SHUTDOWN_TIMEOUT;
        for (what, done) in stopping {
            match tokio::time::timeout_at(deadline, done).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    tracing::warn!(%error, what, "telemetry: the last data was not sent");
                }
                Ok(Err(_)) => tracing::warn!(what, "telemetry: shutdown failed"),
                Err(_) => {
                    tracing::warn!(
                        what,
                        "telemetry: shutdown took too long; the last data is lost"
                    );
                }
            }
        }
    }
}

/// Runs `stop` on a new thread; the receiver gets its result. The thread
/// is not waited for: see [`Telemetry::shutdown`].
fn stop_on_own_thread<F>(stop: F) -> tokio::sync::oneshot::Receiver<OTelSdkResult>
where
    // `Send + 'static`: the function moves to another thread, which may
    // outlive the caller, so it may borrow nothing
    F: FnOnce() -> OTelSdkResult + Send + 'static,
{
    let (done, result) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        // the receiver is gone when the wait already gave up
        let _ = done.send(stop());
    });
    result
}

/// Starts JSON logs, and the export of traces and metrics when an
/// endpoint is set: `OTEL_EXPORTER_OTLP_ENDPOINT` for both,
/// `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` for traces,
/// `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` for metrics (each an `http://` or
/// `https://` URL: the receiver is the operator's own, so plain HTTP is
/// allowed). Each signal is exported only when it has an endpoint.
/// `service_name` is the `service.name` of the exported data unless the
/// operator sets `OTEL_SERVICE_NAME` (or `service.name` in
/// `OTEL_RESOURCE_ATTRIBUTES`).
///
/// Call it once, inside the tokio runtime, before anything else logs or
/// measures: the metric instruments are bound to the provider in force
/// when they are first used ([`metrics`]).
///
/// When export cannot start (a bad `OTEL_*` setting, no runtime), the
/// reason is logged and the process goes on with logs only: telemetry
/// never stops a component.
// `&'static str`: a text fixed in the program (a literal), which the
// providers may keep for as long as the process runs
pub fn init(service_name: &'static str) -> Telemetry {
    let wanted = Signals {
        traces: is_set(ENDPOINT_VARIABLE) || is_set(TRACES_ENDPOINT_VARIABLE),
        metrics: is_set(ENDPOINT_VARIABLE) || is_set(METRICS_ENDPOINT_VARIABLE),
    };
    if !wanted.traces && !wanted.metrics {
        logs_only();
        return Telemetry::off();
    }
    match start_providers(service_name, wanted) {
        Ok((telemetry, service)) => {
            if let Some(meter) = &telemetry.meter {
                // every later `metrics::*` call records into this provider
                opentelemetry::global::set_meter_provider(meter.clone());
            }
            match &telemetry.tracer {
                Some(tracer) => logs_and_traces(tracer.tracer(service_name)),
                None => logs_only(),
            }
            tracing::info!(
                %service,
                traces = wanted.traces,
                metrics = wanted.metrics,
                "telemetry: exporting"
            );
            telemetry
        }
        Err(error) => {
            logs_only();
            tracing::error!(%error, "telemetry: export is off");
            Telemetry::off()
        }
    }
}

/// Whether the environment variable `name` holds a value.
fn is_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
}

/// Which signals are exported.
#[derive(Debug, Clone, Copy)]
struct Signals {
    traces: bool,
    metrics: bool,
}

/// The `RUST_LOG` filter of the JSON logs (default `info`), which never
/// lets a span through: log lines look as they did before 0.9, without
/// the `span` and `spans` fields that the request spans would add.
fn log_filter<S>() -> impl tracing_subscriber::layer::Filter<S>
where
    // `for<'lookup>`: "for every lifetime `'lookup`", a higher-ranked
    // bound; the subscriber can look up spans however long the borrow
    // is. Copied from tracing-subscriber's docs, which ask for it.
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    let level = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    level.and(tracing_subscriber::filter::filter_fn(|metadata| {
        metadata.is_event()
    }))
}

/// Logs only, the same JSON lines as before 0.9.
fn logs_only() {
    let logs = tracing_subscriber::fmt::layer()
        .json()
        .with_filter(log_filter());
    // `try_init` fails only when a subscriber is already set (a second
    // call, or a test); that one stays
    let _ = tracing_subscriber::registry().with(logs).try_init();
}

/// Logs with a `trace_id`, and the OpenTelemetry layer that turns
/// Portitor's spans into exported spans, with its own level (`info`),
/// whatever `RUST_LOG` says.
fn logs_and_traces(tracer: opentelemetry_sdk::trace::Tracer) {
    let logs = tracing_subscriber::fmt::layer()
        .json()
        .event_format(WithTraceId(tracing_subscriber::fmt::format().json()))
        .with_filter(log_filter());
    let spans = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        // thread ids and names say nothing useful about a request
        .with_threads(false)
        .with_filter(tracing_subscriber::filter::filter_fn(exported));
    let _ = tracing_subscriber::registry()
        .with(logs)
        .with(spans)
        .try_init();
}

/// Whether the OpenTelemetry layer sees `metadata`: Portitor's spans at
/// `info` or above, and the state change events ([`STATE_EVENTS`]).
fn exported(metadata: &Metadata<'_>) -> bool {
    // in `tracing`, a more verbose level is the greater one
    if *metadata.level() > Level::INFO {
        return false;
    }
    if metadata.is_span() {
        metadata.target().starts_with("portitor")
    } else {
        metadata.target() == STATE_EVENTS
    }
}

/// A JSON log line with the `trace_id` of the span it was written in
/// first, then everything [`Format<Json>`] writes:
/// `{"trace_id":"4bf9...","timestamp":...,"level":"INFO",...}`.
///
/// The trace id is read from the OpenTelemetry context of the current
/// thread, which the OpenTelemetry layer sets while a span is entered.
/// Outside any span, the line is exactly what [`Format<Json>`] writes.
struct WithTraceId(Format<Json>);

impl<S, N> FormatEvent<S, N> for WithTraceId
where
    // `for<'a>`: "for every lifetime"; copied from tracing-subscriber's
    // docs of `FormatEvent`. `'static`: `N` holds no borrowed data.
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let context = opentelemetry::Context::current();
        let span = context.span();
        let span_context = span.span_context();
        if !span_context.is_valid() {
            return self.0.format_event(ctx, writer, event);
        }
        // the JSON formatter writes one object, `{...}` and a line end;
        // write it to a buffer, then put the trace id after its `{`
        let mut line = String::new();
        self.0.format_event(ctx, Writer::new(&mut line), event)?;
        let Some(rest) = line.strip_prefix('{') else {
            // not an object: leave it as it is
            return writer.write_str(&line);
        };
        // a trace id is 32 hexadecimal digits: nothing to escape
        write!(writer, "{{\"trace_id\":\"{}\",", span_context.trace_id())?;
        writer.write_str(rest)
    }
}

/// Builds the exporters and providers of the `wanted` signals from the
/// `OTEL_*` variables; returns them and the `service.name` they report.
///
/// # Errors
/// No tokio runtime, or the SDK refuses a setting (for example an
/// endpoint that is not a URL); the text says which.
fn start_providers(
    default_service_name: &'static str,
    wanted: Signals,
) -> Result<(Telemetry, String), String> {
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| "telemetry::init was called outside the tokio runtime".to_owned())?;
    let (client, _has_ca_certificates) = crate::http::https_client(true)?;
    let client = OnTokio { client, runtime };
    let resource = resource(default_service_name);
    let service = resource
        .get(&Key::new("service.name"))
        .map(|value| value.to_string())
        .unwrap_or_default();
    // the sampler (`OTEL_TRACES_SAMPLER`), the batch (`OTEL_BSP_*`) and
    // the interval (`OTEL_METRIC_EXPORT_INTERVAL`) come from the
    // environment, read by the SDK
    let mut telemetry = Telemetry::off();
    if wanted.traces {
        let spans = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_http_client(client.clone())
            .build()
            .map_err(|e| format!("span exporter: {e}"))?;
        let tracer = SdkTracerProvider::builder()
            .with_batch_exporter(spans)
            .with_resource(resource.clone())
            .build();
        telemetry.tracer = Some(tracer);
    }
    if wanted.metrics {
        let metrics = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_http_client(client)
            .build()
            .map_err(|e| format!("metric exporter: {e}"))?;
        let meter = SdkMeterProvider::builder()
            .with_periodic_exporter(metrics)
            .with_resource(resource)
            .build();
        telemetry.meter = Some(meter);
    }
    Ok((telemetry, service))
}

/// The resource (what every span and measure is about): the attributes
/// the SDK reads (`OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, the
/// SDK's name and version), with `default_service_name` when the operator
/// named no service, and Portitor's version as `service.version` unless
/// set.
fn resource(default_service_name: &'static str) -> Resource {
    let from_attributes = EnvResourceDetector::new().detect();
    let named_by_operator = std::env::var("OTEL_SERVICE_NAME").is_ok_and(|name| !name.is_empty())
        || from_attributes.get(&Key::new("service.name")).is_some();
    let mut builder = Resource::builder();
    if !named_by_operator {
        builder = builder.with_service_name(default_service_name);
    }
    if from_attributes.get(&Key::new("service.version")).is_none() {
        builder =
            builder.with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")));
    }
    builder.build()
}

/// The HTTP client of the exports: Portitor's own ([`crate::http`]:
/// HTTP/1.1, rustls with ring), with each export run as a task on the
/// tokio runtime captured at start.
///
/// Why (spike 0.9, finding 1): the SDK's batch span processor and
/// periodic metric reader export from their own threads, outside tokio,
/// and hyper's client needs tokio. `send_bytes` is called on such a
/// thread; it starts the request on the runtime and waits for its result.
#[derive(Clone)]
struct OnTokio {
    client: crate::http::HttpsClient,
    runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for OnTokio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OnTokio")
    }
}

/// `#[async_trait]`: the trait is declared with this macro by
/// opentelemetry-http, so the implementation uses it too. It turns the
/// `async fn` into one that returns a boxed task (see `BoxFuture` in
/// `lib.rs` for why traits need that).
#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for OnTokio {
    async fn send_bytes(
        &self,
        request: hyper::Request<hyper::body::Bytes>,
    ) -> Result<hyper::Response<hyper::body::Bytes>, opentelemetry_http::HttpError> {
        let client = self.client.clone();
        let (parts, body) = request.into_parts();
        let request = hyper::Request::from_parts(parts, http_body_util::Full::new(body));
        let task = self.runtime.spawn(export(client, request));
        // the outer error: the task panicked or was cancelled (the
        // runtime stops); the inner one is the export's own
        match task.await {
            Ok(answer) => answer,
            Err(error) => Err(Box::new(error)),
        }
    }
}

/// One export: sends `request` and reads the answer, within
/// [`EXPORT_TIMEOUT`] and up to [`MAX_EXPORT_ANSWER`] bytes. Runs on
/// tokio.
async fn export(
    client: crate::http::HttpsClient,
    request: hyper::Request<http_body_util::Full<hyper::body::Bytes>>,
) -> Result<hyper::Response<hyper::body::Bytes>, opentelemetry_http::HttpError> {
    use http_body_util::BodyExt as _;
    let exchange = async move {
        let response = client.request(request).await?;
        let (parts, body) = response.into_parts();
        let body = http_body_util::Limited::new(body, MAX_EXPORT_ANSWER)
            .collect()
            .await?
            .to_bytes();
        // `Ok::<_, HttpError>`: an `async` block has no declared return
        // type, so the error type is named here (`_` lets the compiler
        // fill in the value's type). `HttpError` is a boxed error, and the
        // `?` above box each error type into it.
        Ok::<_, opentelemetry_http::HttpError>(hyper::Response::from_parts(parts, body))
    };
    match tokio::time::timeout(EXPORT_TIMEOUT, exchange).await {
        Ok(answer) => answer,
        Err(_) => Err(format!("no answer within {}s", EXPORT_TIMEOUT.as_secs()).into()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_portitor_spans_and_state_events_are_exported() {
        use tracing::callsite::Identifier;
        use tracing::field::FieldSet;
        use tracing::metadata::Kind;
        struct Site;
        impl tracing::Callsite for Site {
            fn set_interest(&self, _: tracing::subscriber::Interest) {}
            fn metadata(&self) -> &Metadata<'_> {
                unreachable!()
            }
        }
        static SITE: Site = Site;
        let meta = |target: &'static str, level: Level, kind: Kind| {
            Metadata::new(
                "x",
                target,
                level,
                None,
                None,
                None,
                FieldSet::new(&[], Identifier(&SITE)),
                kind,
            )
        };
        assert!(exported(&meta(
            "portitor_runtime::telemetry::http",
            Level::INFO,
            Kind::SPAN
        )));
        assert!(!exported(&meta(
            "portitor_connector",
            Level::DEBUG,
            Kind::SPAN
        )));
        assert!(!exported(&meta("hyper::client", Level::INFO, Kind::SPAN)));
        assert!(exported(&meta(STATE_EVENTS, Level::INFO, Kind::EVENT)));
        // ordinary log lines stay out of traces
        assert!(!exported(&meta(
            "portitor_connector::outbox",
            Level::WARN,
            Kind::EVENT
        )));
    }

    #[test]
    fn the_service_name_is_the_default_unless_the_operator_sets_one() {
        // the test environment sets neither variable
        if std::env::var_os("OTEL_SERVICE_NAME").is_some()
            || std::env::var_os("OTEL_RESOURCE_ATTRIBUTES").is_some()
        {
            return;
        }
        let resource = resource("portitor-test");
        assert_eq!(
            resource.get(&Key::new("service.name")).unwrap().to_string(),
            "portitor-test"
        );
        assert_eq!(
            resource
                .get(&Key::new("service.version"))
                .unwrap()
                .to_string(),
            env!("CARGO_PKG_VERSION")
        );
    }
}
