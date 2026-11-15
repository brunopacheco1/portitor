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
//! server's host and port for calls we send, and `error.type`. Never a
//! URL path or query, a header, or a body.
//!
//! W3C trace context (`traceparent`, ADR 0023 section 4) crosses only
//! calls inside one participant: a [`ClientCall`] sends it only with
//! [`TraceContext::Send`], and [`observe_requests`] reads it only on the
//! routes it is given. A DSP call to another participant carries none,
//! and one that arrives on the DSP port is ignored.

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

/// Whether a request we send carries our W3C trace context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceContext {
    /// Never: the call goes to another participant (DSP, DCP, DID
    /// documents) or to a service outside our control.
    #[default]
    Withhold,
    /// The call stays inside our participant: the connector to its own
    /// identity component (STS) or to its own data source.
    Send,
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
    if trusted {
        let parent = TraceContextPropagator::new().extract(&HeaderExtractor(request.headers()));
        // fails only when there is no OpenTelemetry layer (export off):
        // there is no trace to join then
        let _ = span.set_parent(parent);
    }
    let response = next.run(request).instrument(span.clone()).await;
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

/// One request we send: its span, and its `http.client.request.duration`
/// measure when it ends. Use it as:
///
/// ```ignore
/// let mut call = ClientCall::start(&mut request, TraceContext::Withhold);
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
    status: Option<u16>,
}

impl ClientCall {
    /// Starts the span of `request`, a child of the current span, and
    /// with [`TraceContext::Send`] writes its `traceparent` header into
    /// the request (only when export is on: without it there is no trace).
    pub fn start<B>(request: &mut hyper::Request<B>, trace_context: TraceContext) -> Self {
        let method = method_name(request.method());
        let uri = request.uri();
        let address = uri.host().unwrap_or_default().to_owned();
        let default_port = if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        };
        let port = uri.port_u16().unwrap_or(default_port);
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
        if trace_context == TraceContext::Send {
            let mut headers = HeaderInjector(request.headers_mut());
            TraceContextPropagator::new().inject_context(&span.context(), &mut headers);
        }
        Self {
            span,
            started: Instant::now(),
            method,
            address,
            port,
            status: None,
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

    /// Records the measure; `no_answer` is the `error.type` when no
    /// status was recorded.
    fn finish(self, no_answer: &'static str) {
        let seconds = self.started.elapsed().as_secs_f64();
        let mut attributes = vec![
            opentelemetry::KeyValue::new("http.request.method", self.method),
            opentelemetry::KeyValue::new("server.address", self.address),
            opentelemetry::KeyValue::new("server.port", i64::from(self.port)),
        ];
        // for a client, 4xx and 5xx are errors (the conventions' rule)
        let error_type = match self.status {
            Some(status) if status >= 400 => Some(status.to_string()),
            Some(_) => None,
            None => Some(no_answer.to_owned()),
        };
        if let Some(status) = self.status {
            attributes.push(opentelemetry::KeyValue::new(
                "http.response.status_code",
                i64::from(status),
            ));
        }
        if let Some(error_type) = error_type {
            self.span.record("error.type", error_type.as_str());
            self.span.record("otel.status_code", "ERROR");
            attributes.push(opentelemetry::KeyValue::new("error.type", error_type));
        }
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
    fn unknown_methods_are_other() {
        assert_eq!(method_name(&hyper::Method::POST), "POST");
        assert_eq!(method_name(&hyper::Method::PATCH), "PATCH");
        let custom = hyper::Method::from_bytes(b"BREW").unwrap();
        assert_eq!(method_name(&custom), "_OTHER");
    }

    /// Without the OpenTelemetry layer there is no trace, so nothing is
    /// written into the request, even with `TraceContext::Send`.
    #[test]
    fn no_traceparent_without_export() {
        let mut request = hyper::Request::get("https://sts.example/token")
            .body(())
            .unwrap();
        let call = ClientCall::start(&mut request, TraceContext::Send);
        assert!(request.headers().get("traceparent").is_none());
        assert_eq!(call.port, 443);
        assert_eq!(call.address, "sts.example");
        call.end();
    }
}
