// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Helpers for tests (this crate's own, and with the cargo feature
//! `test-support`, which only the binaries' `[dev-dependencies]` switch
//! on, theirs; never in a release build):
//! traces collected in memory on the test's thread, and a server that
//! records the `traceparent` header of each request it gets.
//!
//! Use them in a `#[tokio::test]` on the default runtime, which runs
//! every task on the test's own thread, where the tracing set up here
//! applies.

use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use axum::Router;
use axum::http::HeaderMap;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;

/// One finished span, as exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedSpan {
    /// The span's name (`otel.name`).
    pub name: String,
    /// Its trace id, 32 hexadecimal digits.
    pub trace_id: String,
    /// Its span id, 16 hexadecimal digits.
    pub span_id: String,
    /// Its parent's span id; `0000000000000000` for a root span.
    pub parent_span_id: String,
}

/// Collected spans; shared with the exporter.
type Collected = Arc<Mutex<Vec<FinishedSpan>>>;

/// An exporter that keeps every span in memory.
#[derive(Debug, Clone)]
struct Collector(Collected);

impl SpanExporter for Collector {
    // a plain `fn` returning a ready task: the trait asks for a task,
    // and there is nothing to wait for
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        let mut spans = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for span in batch {
            spans.push(FinishedSpan {
                name: span.name.into_owned(),
                trace_id: span.span_context.trace_id().to_string(),
                span_id: span.span_context.span_id().to_string(),
                parent_span_id: span.parent_span_id.to_string(),
            });
        }
        std::future::ready(Ok(()))
    }
}

/// Tracing with the OpenTelemetry layer, as when export is on, on this
/// thread until it is dropped. Spans that end are kept ([`Traced::spans`]).
#[derive(Debug)]
pub struct Traced {
    spans: Collected,
    _provider: SdkTracerProvider,
    _guard: tracing::subscriber::DefaultGuard,
}

impl Traced {
    /// The spans that ended so far.
    #[must_use]
    pub fn spans(&self) -> Vec<FinishedSpan> {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// A subscriber that sees nothing, kept for the whole test process.
///
/// `tracing` keeps, for each call site, whether any subscriber listens.
/// While only one subscriber exists, it asks only the subscriber of the
/// thread that meets a call site first. Tests run in parallel: when this
/// test's subscriber is the only one, another test's thread, which has
/// none, can meet a call site first and store "no one listens", and this
/// test then misses the span. With a second subscriber always alive,
/// `tracing` asks every subscriber, so the answer includes this test's.
static SECOND_DISPATCHER: LazyLock<tracing::Dispatch> =
    LazyLock::new(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));

/// Starts tracing on this thread (see [`Traced`]), with the same choice of
/// spans as the real export (Portitor's own, at `info`).
#[must_use]
pub fn trace_on_this_thread() -> Traced {
    LazyLock::force(&SECOND_DISPATCHER);
    let spans = Collected::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(Collector(Arc::clone(&spans)))
        .build();
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("portitor-test"))
        .with_filter(tracing_subscriber::filter::filter_fn(super::exported));
    let subscriber = tracing_subscriber::registry().with(layer);
    Traced {
        spans,
        _provider: provider,
        _guard: tracing::subscriber::set_default(subscriber),
    }
}

/// The `traceparent` header of each request a [`traceparent_recorder`]
/// got, `None` for a request without one.
pub type Recorded = Arc<Mutex<Vec<Option<String>>>>;

/// A server on this machine that answers every request with `{}` and
/// records its `traceparent`. Returns its base URL (`http://127.0.0.1:...`).
///
/// # Panics
/// No port of this machine can be bound (the test cannot run then).
pub async fn traceparent_recorder() -> (String, Recorded) {
    let recorded = Recorded::default();
    let record = Arc::clone(&recorded);
    let app = Router::new().fallback(move |headers: HeaderMap| {
        let record = Arc::clone(&record);
        async move {
            let traceparent = headers
                .get("traceparent")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            record
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(traceparent);
            axum::Json(serde_json::json!({}))
        }
    });
    let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
        panic!("test server: no free port");
    };
    let Ok(address) = listener.local_addr() else {
        panic!("test server: no address");
    };
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{address}"), recorded)
}

/// What [`traceparent_recorder`] recorded so far.
#[must_use]
pub fn recorded(recorded: &Recorded) -> Vec<Option<String>> {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The senders of this crate, as each binary uses them, with tracing on
/// (ADR 0023, section 4): calls to others carry no `traceparent`, calls
/// to our own services do, unobserved calls carry none and make no span.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use tracing::Instrument as _;

    use super::*;
    use crate::did::DidWebResolver;
    use crate::directory::{Directory as _, WebDirectory};
    use crate::http::{HttpSender, Sender as _};

    #[tokio::test]
    async fn only_calls_to_our_own_services_carry_trace_context() {
        let traced = trace_on_this_thread();
        let (base, recorder) = traceparent_recorder().await;
        let host = base.trim_start_matches("http://").replace(':', "%3A");
        let did = format!("did:web:{host}");
        let url = format!("{base}/x");
        let work = async {
            // to other participants
            let _ = HttpSender::new(true).unwrap().get(&url).await;
            let resolver = DidWebResolver::new(true, Duration::ZERO).unwrap();
            let _ = resolver.resolve(&did).await;
            let directory = WebDirectory::new(true, 300).unwrap();
            let _ = directory.did_document(&did).await;
            let _ = directory.json(&url).await;
            // unobserved, even to our own service
            let quiet = HttpSender::new(true)
                .unwrap()
                .for_own_service()
                .unobserved();
            let _ = quiet.get(&url).await;
            // to our own service
            let own = HttpSender::new(true).unwrap().for_own_service();
            let _ = own.get(&url).await;
        };
        work.instrument(tracing::info_span!(target: "portitor_test", "request"))
            .await;
        let got = recorded(&recorder);
        assert_eq!(got.len(), 6, "{got:?}");
        assert!(got[..5].iter().all(Option::is_none), "{got:?}");
        assert!(got[5].as_deref().is_some_and(|t| t.starts_with("00-")));
        // five observed calls, five client spans; none for the unobserved
        let clients = traced
            .spans()
            .iter()
            .filter(|span| span.name == "GET")
            .count();
        assert_eq!(clients, 5);
    }
}
