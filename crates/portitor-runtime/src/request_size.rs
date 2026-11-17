// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Request size limits (R-18): one setting per port
//! ([`RequestLimits`]), and a lower fixed limit on the routes whose
//! messages are small by specification ([`small_message`]). A request
//! whose body is over its limit gets HTTP 413.
//!
//! A route with a fixed limit is behind its port's limit too, so the
//! lower of the two applies. Only request bodies are limited: the data
//! plane streams its answers, and they are not touched.

use std::ffi::OsString;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::MethodRouter;
use http_body_util::{BodyExt as _, LengthLimitError, Limited};

use crate::api_key::problem;

/// The limit of each port unless its setting says otherwise: 2 MiB,
/// the limit axum applied to every parsed body before 0.11.
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

/// The highest value a port's setting may have: 64 MiB.
pub const HIGHEST_MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;

/// The fixed limit of a route whose messages are small by specification:
/// 64 KiB. The largest example of these messages in the DSP 2025-1 and
/// DCP v1.0.1 specifications is 329 bytes (a contract negotiation
/// termination message).
pub const SMALL_MESSAGE_MAX_BYTES: usize = 64 * 1024;

/// The setting of the public port's limit.
pub const PUBLIC_SETTING: &str = "PORTITOR_MAX_REQUEST_BYTES";

/// The setting of the management port's limit.
pub const MANAGEMENT_SETTING: &str = "PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES";

/// The largest request body, in bytes, each port takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLimits {
    /// The public port (`PORTITOR_MAX_REQUEST_BYTES`).
    pub public: usize,
    /// The management port (`PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES`).
    pub management: usize,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            public: DEFAULT_MAX_REQUEST_BYTES,
            management: DEFAULT_MAX_REQUEST_BYTES,
        }
    }
}

/// A limit setting is not a whole number from 1 to
/// [`HIGHEST_MAX_REQUEST_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitError {
    /// The environment variable.
    pub name: &'static str,
    /// Its value.
    pub value: String,
}

impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} has an invalid value {:?}: it must be a whole number of bytes from 1 to {HIGHEST_MAX_REQUEST_BYTES}",
            self.name, self.value
        )
    }
}

impl std::error::Error for LimitError {}

impl RequestLimits {
    /// Reads `PORTITOR_MAX_REQUEST_BYTES` and
    /// `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` from the environment.
    ///
    /// # Errors
    /// See [`RequestLimits::from_vars`].
    pub fn from_env() -> Result<Self, LimitError> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// Like [`RequestLimits::from_env`], with the variables looked up by
    /// `var`. Values are trimmed; an empty value counts as not set, and
    /// a setting not set is [`DEFAULT_MAX_REQUEST_BYTES`].
    ///
    /// # Errors
    /// A value is not a whole number from 1 to
    /// [`HIGHEST_MAX_REQUEST_BYTES`].
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, LimitError> {
        Ok(Self {
            public: bytes_setting(&var, PUBLIC_SETTING)?,
            management: bytes_setting(&var, MANAGEMENT_SETTING)?,
        })
    }
}

/// Reads one limit setting.
fn bytes_setting(
    var: &impl Fn(&str) -> Option<OsString>,
    name: &'static str,
) -> Result<usize, LimitError> {
    let value = match var(name) {
        Some(value) => value.to_string_lossy().trim().to_owned(),
        None => return Ok(DEFAULT_MAX_REQUEST_BYTES),
    };
    if value.is_empty() {
        return Ok(DEFAULT_MAX_REQUEST_BYTES);
    }
    match value.parse::<usize>() {
        Ok(bytes) if (1..=HIGHEST_MAX_REQUEST_BYTES).contains(&bytes) => Ok(bytes),
        _ => Err(LimitError { name, value }),
    }
}

/// `router` with a limit of `max_bytes` on every request body: the
/// limit of a port. axum's own body limit (used by the `Json`, `Form`
/// and other extractors) is set to the same value.
pub fn limit_port(router: Router, max_bytes: usize) -> Router {
    router
        .layer(axum::middleware::from_fn_with_state(
            max_bytes,
            refuse_too_large,
        ))
        .layer(DefaultBodyLimit::max(max_bytes))
}

/// `route` with the fixed limit [`SMALL_MESSAGE_MAX_BYTES`]: for routes
/// whose messages are small by specification. The port's limit still
/// applies, so the lower of the two counts.
pub fn small_message<S>(route: MethodRouter<S>) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    route.layer(axum::middleware::from_fn_with_state(
        SMALL_MESSAGE_MAX_BYTES,
        refuse_too_large,
    ))
}

/// The middleware of [`limit_port`] and [`small_message`].
///
/// A request that declares its length (`Content-Length`) is refused at
/// once when that is over the limit, and otherwise passed on unread: the
/// body cannot be longer than it declares. A request without a declared
/// length (chunked) is read here, up to the limit, and refused as soon
/// as it goes over.
async fn refuse_too_large(
    State(max_bytes): State<usize>,
    request: Request,
    next: Next,
) -> Response {
    match declared_length(&request) {
        Some(length) if length > max_bytes => return too_large(&request, max_bytes),
        Some(_) => return next.run(request).await,
        None => {}
    }
    let (parts, body) = request.into_parts();
    match Limited::new(body, max_bytes).collect().await {
        Ok(collected) => {
            let body = Body::from(collected.to_bytes());
            next.run(Request::from_parts(parts, body)).await
        }
        Err(error) if error.is::<LengthLimitError>() => {
            too_large(&Request::from_parts(parts, ()), max_bytes)
        }
        Err(_) => problem(StatusCode::BAD_REQUEST, "the request body cannot be read"),
    }
}

/// The request's `Content-Length`, if it has a valid one. A length too
/// large for a `usize` counts as `usize::MAX`, which is over any limit.
fn declared_length<B>(request: &Request<B>) -> Option<usize> {
    let value = request.headers().get(header::CONTENT_LENGTH)?;
    let length = value.to_str().ok()?.parse::<u64>().ok()?;
    Some(usize::try_from(length).unwrap_or(usize::MAX))
}

/// HTTP 413 with a problem body, and a log line.
fn too_large<B>(request: &Request<B>, max_bytes: usize) -> Response {
    let path = request.uri().path();
    tracing::info!(%path, max_bytes, "request body too large");
    problem(
        StatusCode::PAYLOAD_TOO_LARGE,
        &format!("the request body is larger than {max_bytes} bytes"),
    )
}

/// The check the binaries' tests run on each limited port and route.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use axum::Router;
    use axum::body::Body;
    use axum::http::request::Builder;
    use axum::http::{StatusCode, header};
    use tower::ServiceExt as _;

    /// Checks that `app` refuses a body of `max_bytes + 1` with HTTP 413,
    /// and does not refuse a body of `max_bytes` for its size (any other
    /// answer is fine). Each is sent twice: with `Content-Length`, and
    /// without (as a chunked body is). `request` makes the request
    /// without its body (method, URI, headers).
    ///
    /// # Panics
    /// When `app` does not answer as expected.
    pub async fn assert_size_limit(app: &Router, request: impl Fn() -> Builder, max_bytes: usize) {
        for declared in [true, false] {
            let over = send(app, request(), max_bytes + 1, declared).await;
            let at = send(app, request(), max_bytes, declared).await;
            let uri = request().uri_ref().map(ToString::to_string);
            assert_eq!(
                over,
                StatusCode::PAYLOAD_TOO_LARGE,
                "{uri:?}: {} bytes, Content-Length sent: {declared}",
                max_bytes + 1
            );
            assert_ne!(
                at,
                StatusCode::PAYLOAD_TOO_LARGE,
                "{uri:?}: {max_bytes} bytes, Content-Length sent: {declared}"
            );
        }
    }

    /// Sends `request` with a body of `size` bytes and gives the status.
    async fn send(app: &Router, request: Builder, size: usize, declared: bool) -> StatusCode {
        let request = if declared {
            request.header(header::CONTENT_LENGTH, size)
        } else {
            request
        };
        let request = request.body(Body::from(vec![b' '; size]));
        let request = match request {
            Ok(request) => request,
            Err(error) => panic!("the test request cannot be built: {error}"),
        };
        match app.clone().oneshot(request).await {
            Ok(response) => response.status(),
            Err(error) => match error {},
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::testing::assert_size_limit;
    use super::*;
    use axum::body::Bytes;
    use axum::http::Request;
    use axum::routing::post;

    fn read(pairs: &[(&str, &str)]) -> Result<RequestLimits, LimitError> {
        RequestLimits::from_vars(|name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn settings_default_to_2_mib_and_are_bounded() {
        assert_eq!(read(&[]), Ok(RequestLimits::default()));
        assert_eq!(read(&[(PUBLIC_SETTING, " ")]), Ok(RequestLimits::default()));
        assert_eq!(
            read(&[(PUBLIC_SETTING, " 4096 "), (MANAGEMENT_SETTING, "67108864")]),
            Ok(RequestLimits {
                public: 4096,
                management: HIGHEST_MAX_REQUEST_BYTES,
            })
        );
        for bad in ["0", "-1", "1.5", "2MB", "67108865"] {
            let error = read(&[(MANAGEMENT_SETTING, bad)]).unwrap_err();
            assert_eq!(error.name, MANAGEMENT_SETTING);
            assert!(error.to_string().starts_with(MANAGEMENT_SETTING), "{error}");
        }
    }

    /// `/big` takes the port's limit, `/small` the fixed one; each answers
    /// 200 with the body's size once axum's `Bytes` extractor read it.
    fn app(port_max_bytes: usize) -> Router {
        async fn size(body: Bytes) -> String {
            body.len().to_string()
        }
        let router = Router::new()
            .route("/big", post(size))
            .route("/small", small_message(post(size)));
        limit_port(router, port_max_bytes)
    }

    fn post_to(path: &'static str) -> impl Fn() -> axum::http::request::Builder {
        move || Request::post(path)
    }

    #[tokio::test]
    async fn bodies_over_the_limits_get_413() {
        let app = app(DEFAULT_MAX_REQUEST_BYTES);
        assert_size_limit(&app, post_to("/big"), DEFAULT_MAX_REQUEST_BYTES).await;
        assert_size_limit(&app, post_to("/small"), SMALL_MESSAGE_MAX_BYTES).await;
    }

    #[tokio::test]
    async fn the_lower_of_port_and_route_limit_applies() {
        let app = app(1000);
        assert_size_limit(&app, post_to("/small"), 1000).await;
    }

    /// Above axum's default of 2 MB, the handler still reads the body.
    #[tokio::test]
    async fn a_port_limit_above_2_mib_is_used_by_the_extractors() {
        let max_bytes = 3 * 1024 * 1024;
        let app = app(max_bytes);
        assert_size_limit(&app, post_to("/big"), max_bytes).await;
        for declared in [true, false] {
            let mut request = Request::post("/big");
            if declared {
                request = request.header(header::CONTENT_LENGTH, max_bytes);
            }
            let request = request.body(Body::from(vec![b' '; max_bytes])).unwrap();
            let response = tower::ServiceExt::oneshot(app.clone(), request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap();
            assert_eq!(body, max_bytes.to_string());
        }
    }
}
