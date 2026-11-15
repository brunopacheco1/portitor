// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! One span and one duration measure per HTTP request, with the
//! attributes of OpenTelemetry's HTTP semantic conventions
//! (<https://opentelemetry.io/docs/specs/semconv/http/http-spans/>):
//!
//! - served: [`observe_requests`], on the public and management ports;
//! - sent: [`ClientCall`], in the shared client ([`crate::http`]) and the
//!   connector's data plane.
//!
//! Only these attributes are set: the method, the route template
//! (`http.route`, never the path with its ids), the status code, the
//! server's host and port for calls we send (on the measure only for
//! calls inside our participant, see [`Peer`]), and `error.type`. Never a
//! URL path or query, a header, or a body.
//!
//! W3C trace context (`traceparent`, ADR 0023 section 4) crosses only
//! calls inside one participant: a [`ClientCall`] sends it only to a
//! [`Peer::Own`], and [`observe_requests`] reads it only on the routes it
//! is given, and uses it only once the handler authenticated the caller
//! ([`CallerTrace`]). A DSP call to another participant carries none, and
//! one that arrives on the DSP port is ignored.

use std::time::Instant;

use axum::Router;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry_http::{HeaderExtractor, HeaderInjector};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::Instrument as _;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use super::metrics;

/// Who a request we send goes to. It decides what is recorded about the
/// call and whether our W3C trace context goes with it (ADR 0023,
/// section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Peer {
    /// Another participant (DSP, DCP, DID documents) or a service outside
    /// our control. No `traceparent` is sent. The server's host and port
    /// are on the span, but not on the `http.client.request.duration`
    /// measure: other participants choose these hosts (a token's `iss`, a
    /// callback address), and each new value would make a new series.
    #[default]
    Other,
    /// A service inside our participant: the connector's own identity
    /// component (STS) or its own data source. Our `traceparent` is sent,
    /// and the host and port are on the measure too: the operator sets
    /// these few addresses.
    Own,
}

/// `router` with a span and an `http.server.request.duration` measure
/// for every request. `trusted_routes` are the route templates (as
/// written in the router, for example `/sts/token`) whose callers are
/// inside our participant: only there an incoming `traceparent` becomes
/// the parent of our span. Everywhere else it is ignored.
///
/// Apply it to the whole router, after its routes are added (axum's
/// `Router::layer`), so the route template is known.
pub fn observe_requests(router: Router, trusted_routes: &'static [&'static str]) -> Router {
    router.layer(axum::middleware::from_fn_with_state(
        trusted_routes,
        observe,
    ))
}

/// The middleware of [`observe_requests`].
async fn observe(
    State(trusted_routes): State<&'static [&'static str]>,
    request: Request,
    next: Next,
) -> Response {
    // health probes (Docker, Kubernetes) call these every few seconds:
    // a span and a measure for each would fill the traces with noise
    if request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|path| is_health_probe(path.as_str()))
    {
        return next.run(request).await;
    }
    let started = Instant::now();
    let method = method_name(request.method());
    let version = version_name(request.version());
    // the template, `/dsp/2025-1/negotiations/{pid}`; none when no route
    // matched (a 404)
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    // the span name the conventions ask for: `{method} {route}`
    let name = match &route {
        Some(route) => format!("{method} {route}"),
        None => method.to_owned(),
    };
    // `otel.*` fields are read by the OpenTelemetry layer: the span's
    // name, kind and status; `Empty` fields are filled in later
    let span = tracing::info_span!(
        "http.server.request",
        otel.name = %name,
        otel.kind = "server",
        http.request.method = method,
        url.scheme = "http",
        http.route = route.as_deref(),
        http.response.status_code = Empty,
        "error.type" = Empty,
        otel.status_code = Empty,
    );
    let trusted = route
        .as_deref()
        .is_some_and(|route| trusted_routes.contains(&route));
    let response = if trusted {
        // the caller is not authenticated yet: hand its trace context to
        // the handler, which joins it only after checking the caller
        // ([`CallerTrace::join`]). A span cannot get a parent once it was
        // entered, so here the handler runs outside the span, and enters
        // it itself.
        let parent = TraceContextPropagator::new().extract(&HeaderExtractor(request.headers()));
        let mut request = request;
        request.extensions_mut().insert(CallerTrace {
            span: span.clone(),
            parent,
        });
        next.run(request).await
    } else {
        // `.instrument(span)` takes the span by value, so a clone goes in
        // and `span` stays here to record the status below
        next.run(request).instrument(span.clone()).await
    };
    let status = response.status().as_u16();
    span.record("http.response.status_code", status);
    // a server span is an error only for 5xx; a 4xx is the client's
    if status >= 500 {
        span.record("error.type", status.to_string());
        span.record("otel.status_code", "ERROR");
    }
    let seconds = started.elapsed().as_secs_f64();
    metrics::server_request(seconds, method, route.as_deref(), status, version);
    response
}

/// The W3C trace context a caller sent on a trusted route, not used yet,
/// and the span of the request. [`observe_requests`] puts it into the
/// request's extensions; the handler takes it (axum's
/// `Option<Extension<CallerTrace>>`) and calls [`CallerTrace::join`] once
/// it has authenticated the caller. Until then the caller is anyone on
/// the public port, and its trace id is untrusted input (ADR 0023,
/// section 4). A request whose caller is refused keeps a trace of its
/// own.
#[derive(Debug, Clone)]
pub struct CallerTrace {
    span: tracing::Span,
    parent: opentelemetry::Context,
}

impl CallerTrace {
    /// The caller is authenticated: the request's span joins the
    /// caller's trace, and is entered until the returned guard is
    /// dropped, so that what follows (log lines, spans) belongs to it.
    /// Use it in code that does not `.await` while the guard lives (a
    /// span entered this way does not follow a task across an `.await`).
    #[must_use = "the span is left when the guard is dropped"]
    pub fn join(&self) -> tracing::span::Entered<'_> {
        // fails only when there is no OpenTelemetry layer (export off):
        // there is no trace to join then
        let _ = self.span.set_parent(self.parent.clone());
        self.span.enter()
    }
}

/// Whether `route` is a health check (ADR 0023), which is not traced or
/// measured.
fn is_health_probe(route: &str) -> bool {
    // `/health`: the plain `ok` that each component serves on its public
    // port since before 0.9; `/api/check/...`: the four checks of the
    // management port
    route == "/health" || route.starts_with(crate::health::CHECK_PREFIX)
}

/// One request we send: its span, and its `http.client.request.duration`
/// measure when it ends. Use it as:
///
/// ```ignore
/// let mut call = ClientCall::start(&mut request, Peer::Other);
/// let answer = client.request(request).instrument(call.span().clone()).await;
/// if let Ok(response) = &answer { call.answered(response.status().as_u16()); }
/// call.end();
/// ```
#[derive(Debug)]
pub struct ClientCall {
    span: tracing::Span,
    started: Instant,
    method: &'static str,
    address: String,
    port: u16,
    peer: Peer,
    status: Option<u16>,
    /// The answer's status came, but its body could not be read.
    body_lost: bool,
    /// Whether the measure is recorded when the call ends.
    measured: bool,
}

impl ClientCall {
    /// Starts the span of `request`, a child of the current span. For
    /// [`Peer::Own`] it writes the span's `traceparent` header into the
    /// request (only when export is on: without it there is no trace).
    pub fn start<B>(request: &mut hyper::Request<B>, peer: Peer) -> Self {
        let method = method_name(request.method());
        let uri = request.uri();
        let address = uri.host().unwrap_or_default().to_owned();
        let default_port = if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        };
        let port = uri.port_u16().unwrap_or(default_port);
        // the span always names the server: one span is one call, so the
        // number of hosts does not matter there
        let span = tracing::info_span!(
            "http.client.request",
            otel.name = method,
            otel.kind = "client",
            http.request.method = method,
            server.address = %address,
            server.port = port,
            http.response.status_code = Empty,
            "error.type" = Empty,
            otel.status_code = Empty,
        );
        if peer == Peer::Own {
            let mut headers = HeaderInjector(request.headers_mut());
            TraceContextPropagator::new().inject_context(&span.context(), &mut headers);
        }
        Self {
            span,
            started: Instant::now(),
            method,
            address,
            port,
            peer,
            status: None,
            body_lost: false,
            measured: true,
        }
    }

    /// A call that is not observed at all: no span, no measure, no
    /// `traceparent`. For work that runs every few seconds on its own,
    /// such as a readiness check, which would otherwise fill the traces
    /// and the measures with noise.
    #[must_use]
    pub fn unobserved() -> Self {
        Self {
            // a span that records nothing
            span: tracing::Span::none(),
            started: Instant::now(),
            method: "_OTHER",
            address: String::new(),
            port: 0,
            peer: Peer::Other,
            status: None,
            body_lost: false,
            measured: false,
        }
    }

    /// The span, to run the request in (`.instrument(call.span().clone())`).
    #[must_use]
    pub fn span(&self) -> &tracing::Span {
        &self.span
    }

    /// The other side answered with `status`.
    pub fn answered(&mut self, status: u16) {
        self.status = Some(status);
        self.span.record("http.response.status_code", status);
    }

    /// The status came, but the answer's body could not be read (the
    /// connection broke, or the body was too large): the call failed,
    /// even with a 2xx status (`error.type` `_OTHER`).
    pub fn body_not_read(&mut self) {
        self.body_lost = true;
    }

    /// The call is over: an answer was recorded with
    /// [`ClientCall::answered`], or there was none (`error.type`
    /// `_OTHER`: the connection failed, or the answer could not be read).
    pub fn end(self) {
        self.finish("_OTHER");
    }

    /// The call got no answer in time (`error.type` `timeout`).
    pub fn end_timed_out(mut self) {
        // a status that came before the body ran out of time is not the
        // answer
        self.status = None;
        self.finish("timeout");
    }

    /// The `error.type` of the call, if it failed; `no_answer` when no
    /// status was recorded.
    fn error_type(&self, no_answer: &'static str) -> Option<String> {
        match self.status {
            // for a client, 4xx and 5xx are errors (the conventions' rule)
            Some(status) if status >= 400 => Some(status.to_string()),
            Some(_) if self.body_lost => Some("_OTHER".to_owned()),
            Some(_) => None,
            None => Some(no_answer.to_owned()),
        }
    }

    /// The attributes of the measure. `server.address` and `server.port`
    /// only for [`Peer::Own`] (see [`Peer::Other`] for why).
    fn metric_attributes(&self, error_type: Option<&str>) -> Vec<opentelemetry::KeyValue> {
        let mut attributes = vec![opentelemetry::KeyValue::new(
            "http.request.method",
            self.method,
        )];
        if self.peer == Peer::Own {
            attributes.push(opentelemetry::KeyValue::new(
                "server.address",
                self.address.clone(),
            ));
            attributes.push(opentelemetry::KeyValue::new(
                "server.port",
                i64::from(self.port),
            ));
        }
        if let Some(status) = self.status {
            attributes.push(opentelemetry::KeyValue::new(
                "http.response.status_code",
                i64::from(status),
            ));
        }
        if let Some(error_type) = error_type {
            attributes.push(opentelemetry::KeyValue::new(
                "error.type",
                error_type.to_owned(),
            ));
        }
        attributes
    }

    /// Records the error on the span, and the measure; `no_answer` is the
    /// `error.type` when no status was recorded.
    fn finish(self, no_answer: &'static str) {
        if !self.measured {
            return;
        }
        let seconds = self.started.elapsed().as_secs_f64();
        let error_type = self.error_type(no_answer);
        if let Some(error_type) = &error_type {
            self.span.record("error.type", error_type.as_str());
            self.span.record("otel.status_code", "ERROR");
        }
        let attributes = self.metric_attributes(error_type.as_deref());
        metrics::client_request(seconds, &attributes);
    }
}

/// The method as the conventions write it: one of the methods of RFC
/// 9110 and RFC 5789 (`PATCH`), or `_OTHER`, so that a caller cannot fill
/// the measures with methods of its own.
fn method_name(method: &hyper::Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "PATCH" => "PATCH",
        "CONNECT" => "CONNECT",
        "TRACE" => "TRACE",
        _ => "_OTHER",
    }
}

/// `network.protocol.version`: `1.1` for HTTP/1.1 (the only version
/// Portitor's servers speak, with `1.0`).
fn version_name(version: hyper::Version) -> &'static str {
    if version == hyper::Version::HTTP_10 {
        "1.0"
    } else if version == hyper::Version::HTTP_11 {
        "1.1"
    } else {
        "_OTHER"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn health_probes_are_not_traced() {
        assert!(is_health_probe("/health"));
        assert!(is_health_probe("/api/check/readiness"));
        assert!(!is_health_probe("/dsp/2025-1/catalog/request"));
        assert!(!is_health_probe("/v1/negotiations"));
    }

    #[test]
    fn unknown_methods_are_other() {
        assert_eq!(method_name(&hyper::Method::POST), "POST");
        assert_eq!(method_name(&hyper::Method::PATCH), "PATCH");
        let custom = hyper::Method::from_bytes(b"BREW").unwrap();
        assert_eq!(method_name(&custom), "_OTHER");
    }

    /// Without the OpenTelemetry layer there is no trace, so nothing is
    /// written into the request, even for [`Peer::Own`].
    #[test]
    fn no_traceparent_without_export() {
        let mut request = hyper::Request::get("https://sts.example/token")
            .body(())
            .unwrap();
        let call = ClientCall::start(&mut request, Peer::Own);
        assert!(request.headers().get("traceparent").is_none());
        assert_eq!(call.port, 443);
        assert_eq!(call.address, "sts.example");
        call.end();
    }

    /// Calls to other participants: whatever host a caller names (a
    /// token's `iss`, a callback address), the measure gets one of a few
    /// fixed attribute sets, never one per host.
    #[test]
    fn other_hosts_do_not_grow_the_measure() {
        use std::collections::BTreeSet;
        let mut sets = BTreeSet::new();
        for n in 0..500 {
            let url = format!("https://caller-{n}.example:{}/did.json", 1000 + n);
            let mut request = hyper::Request::get(url).body(()).unwrap();
            let mut call = ClientCall::start(&mut request, Peer::Other);
            call.answered(200);
            // the span still names the server
            assert_eq!(call.address, format!("caller-{n}.example"));
            let attributes = call.metric_attributes(call.error_type("_OTHER").as_deref());
            sets.insert(format!("{attributes:?}"));
        }
        assert_eq!(sets.len(), 1, "{sets:?}");
        let only = sets.first().unwrap();
        assert!(!only.contains("server.address"), "{only}");
        assert!(!only.contains("server.port"), "{only}");
    }

    /// Calls inside our participant name the server on the measure too.
    #[test]
    fn own_services_are_named_on_the_measure() {
        let mut request = hyper::Request::post("http://identity:8090/sts/token")
            .body(())
            .unwrap();
        let mut call = ClientCall::start(&mut request, Peer::Own);
        call.answered(200);
        let attributes = format!("{:?}", call.metric_attributes(None));
        assert!(attributes.contains("server.address"), "{attributes}");
        assert!(attributes.contains("identity"), "{attributes}");
        assert!(attributes.contains("8090"), "{attributes}");
    }

    /// A 2xx whose body could not be read is a failed call.
    #[test]
    fn a_lost_body_is_an_error() {
        let mut request = hyper::Request::get("https://p.example/").body(()).unwrap();
        let mut call = ClientCall::start(&mut request, Peer::Other);
        call.answered(200);
        assert_eq!(call.error_type("_OTHER"), None);
        call.body_not_read();
        assert_eq!(call.error_type("_OTHER").as_deref(), Some("_OTHER"));
        let mut call = ClientCall::start(&mut request, Peer::Other);
        call.answered(404);
        call.body_not_read();
        assert_eq!(call.error_type("_OTHER").as_deref(), Some("404"));
    }
}
