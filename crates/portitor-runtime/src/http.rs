// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The outbound HTTP client: the [`Sender`] port, its adapter
//! [`HttpSender`] for messages to other participants, and the rule for
//! which URLs a Portitor process may call.

use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
/// The bytes of a body: cheap to hand on, as they are shared, not copied.
pub use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

use tracing::Instrument as _;

use crate::BoxFuture;
use crate::telemetry::http::{ClientCall, Peer};

/// Why an exchange of [`HttpSender`] got no 2xx answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// The counter-party answered with this status, which is not 2xx.
    Status(u16),
    /// Like [`HttpError::Status`], with the start of the answer's body
    /// (at most [`ERROR_BODY_BYTES`]), which often says why. Read where a
    /// person is told the reason: by [`HttpSender::post_text`] (a Gaia-X
    /// compliance service explains there why it refused a participant)
    /// and by the catalog calls (`post_authorized_within`,
    /// `get_authorized_within`).
    StatusWithBody(u16, String),
    /// No answer: the connection failed, no answer came in time, or the
    /// answer broke off while it was read.
    NoAnswer(String),
    /// The request was not sent: the URL is not allowed, or there are no
    /// CA certificates to check an `https://` server.
    NotSent(String),
    /// A 2xx answer that cannot be used: larger than the limit, or not
    /// UTF-8 text where text was asked for.
    BadAnswer(String),
}

impl HttpError {
    /// Whether the failure may go away when the request is sent again
    /// later: no answer came ([`HttpError::NoAnswer`]), or the answer was
    /// 408, 429 or 5xx ([`asks_to_try_later`]). A request that was not
    /// sent, an unusable answer and any other status are final.
    #[must_use]
    pub fn is_temporary(&self) -> bool {
        match self {
            Self::Status(code) | Self::StatusWithBody(code, _) => asks_to_try_later(*code),
            Self::NoAnswer(_) => true,
            Self::NotSent(_) | Self::BadAnswer(_) => false,
        }
    }
}

/// Whether an answer with HTTP `status` asks to try again later: `408
/// Request Timeout`, `429 Too Many Requests`, or a server error (5xx).
/// Any other status is the server's final answer to this request.
#[must_use]
pub fn asks_to_try_later(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}

/// The text of the error: `HTTP 404 Not Found` for a status (the form
/// [`answered_status`] reads back), the reason otherwise.
impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => write_status(f, *code),
            Self::StatusWithBody(code, body) if body.is_empty() => write_status(f, *code),
            Self::StatusWithBody(code, body) => {
                write_status(f, *code)?;
                write!(f, ": {body}")
            }
            Self::NoAnswer(reason) | Self::NotSent(reason) | Self::BadAnswer(reason) => {
                f.write_str(reason)
            }
        }
    }
}

/// `HTTP 404 Not Found`, or `HTTP 499` for a code without a name.
fn write_status(f: &mut std::fmt::Formatter<'_>, code: u16) -> std::fmt::Result {
    match hyper::StatusCode::from_u16(code) {
        Ok(status) => write!(f, "HTTP {status}"),
        Err(_) => write!(f, "HTTP {code}"),
    }
}

impl std::error::Error for HttpError {}

/// The most bytes of a refusal's body kept in a
/// [`HttpError::StatusWithBody`]: enough for a reason, small enough for a
/// log line and a database row.
pub const ERROR_BODY_BYTES: usize = 2 * 1024;

/// Port: sends a message to a counter-party and waits for its HTTP 2xx.
/// Each method returns, later ([`BoxFuture`]), the counter-party's JSON
/// answer (`Value::Null` if it sent none or no JSON), or why the request
/// failed.
pub trait Sender: Send + Sync {
    /// POSTs `body` as JSON to `url`.
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>>;

    /// GETs `url`, for example a counter-party's negotiation state. The
    /// default implementation always fails: a sender that cannot read
    /// (a test fake) then never makes a failed message look delivered.
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { Err(format!("cannot read {url}")) })
    }

    /// Like [`Sender::post`], with the header `Authorization:
    /// <authorization>` (a DSP request carries the caller's DCP token,
    /// ADR 0016). The default leaves the header out (`_authorization`
    /// is not used): only senders that do
    /// not speak HTTP (test fakes) keep it; [`HttpSender`] sends it.
    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        _authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.post(url, body)
    }

    /// Like [`Sender::get`], with the header `Authorization:
    /// <authorization>`. The default leaves the header out, as
    /// [`Sender::post_authorized`] does.
    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        _authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        self.get(url)
    }

    /// Like [`Sender::post_authorized`], for an answer that may be large
    /// (a provider's catalog): the answer is read up to `limits.max_bytes`
    /// within `limits.timeout` and returned as it came, with its `Link`
    /// headers, and the error keeps its kind ([`HttpError`]). The default
    /// implementation always fails (`_body` and the other arguments are
    /// not used): a sender that cannot read a large answer (a test fake)
    /// says so.
    fn post_authorized_within<'a>(
        &'a self,
        url: &'a str,
        _body: &'a Value,
        _authorization: &'a str,
        _limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        Box::pin(async move { Err(cannot_read_large_answers(url)) })
    }

    /// Like [`Sender::get_authorized`], for an answer that may be large:
    /// see [`Sender::post_authorized_within`].
    fn get_authorized_within<'a>(
        &'a self,
        url: &'a str,
        _authorization: &'a str,
        _limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        Box::pin(async move { Err(cannot_read_large_answers(url)) })
    }
}

/// The error of the [`Sender`] default methods for large answers.
fn cannot_read_large_answers(url: &str) -> HttpError {
    HttpError::NotSent(format!("this sender cannot read a large answer from {url}"))
}

/// How much of a 2xx answer [`Sender::post_authorized_within`] and
/// [`Sender::get_authorized_within`] read, and how long they wait. The
/// caller chooses both, so each use states its own limit; every other
/// call of [`HttpSender`] keeps [`MAX_ANSWER`] and [`SEND_TIMEOUT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnswerLimits {
    /// The largest answer read, in bytes. A larger one is an
    /// [`HttpError::BadAnswer`].
    pub max_bytes: usize,
    /// How long to wait for the whole answer.
    pub timeout: Duration,
}

/// A 2xx answer as it came: the bytes of its body, not parsed, and the
/// values of its `Link` headers (RFC 8288). DSP 2025-1 uses `Link` for
/// catalog pages ("Catalog HTTPS Binding", "Pagination").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawAnswer {
    /// The body.
    pub body: Bytes,
    /// Each `Link` header's value, in the order received; empty without
    /// one. A value that is not visible ASCII is left out.
    pub links: Vec<String>,
}

/// How long one request of [`HttpSender`] may take, from sending until
/// the whole answer is read: a POST or GET to a counter-party, a token
/// request to the STS, a DID document or status list fetch.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest answer read from a counter-party. A `ContractNegotiation`
/// is well under 1 KiB; the limit stops a counter-party from making the
/// connector buffer an unbounded body. A call that expects more (a
/// provider's catalog) states its own limit ([`AnswerLimits`]).
const MAX_ANSWER: usize = 64 * 1024;

/// The hyper client both binaries use: rustls, HTTP/1.
pub type HttpsClient = Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>;

/// Adapter: HTTP(S) client. TLS uses rustls with the ring provider and the
/// system's CA certificates.
#[derive(Clone)]
pub struct HttpSender {
    client: HttpsClient,
    allow_http: bool,
    has_ca_certificates: bool,
    /// Who the requests go to (what is recorded, and whether our trace
    /// context is sent).
    peer: Peer,
    /// Whether each request gets a span and a duration measure.
    observed: bool,
}

impl std::fmt::Debug for HttpSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSender")
            .field("allow_http", &self.allow_http)
            .field("has_ca_certificates", &self.has_ca_certificates)
            .field("peer", &self.peer)
            .field("observed", &self.observed)
            .finish_non_exhaustive()
    }
}

impl HttpSender {
    /// Builds the client. Plain `http://` targets are refused unless
    /// `allow_http` (development and conformance tests only).
    ///
    /// Without the system's CA certificates (the release image has none
    /// unless the operator mounts them) the client still starts, so the
    /// catalog keeps working, but every `https://` send fails with an error
    /// that says why.
    ///
    /// # Errors
    ///
    /// The ring provider supports no safe TLS version (does not happen with
    /// the rustls versions in `Cargo.lock`).
    pub fn new(allow_http: bool) -> Result<Self, String> {
        let (client, has_ca_certificates) = https_client(allow_http)?;
        Ok(Self {
            client,
            allow_http,
            has_ca_certificates,
            peer: Peer::Other,
            observed: true,
        })
    }

    /// This sender for a service inside our participant ([`Peer::Own`]):
    /// our W3C trace context (`traceparent`) goes with every request, so
    /// the other side's spans join our trace, and the measures name the
    /// server. Only for a sender whose calls stay inside our participant,
    /// such as the connector's client of its own STS (ADR 0023, section
    /// 4); never for one that calls other participants.
    #[must_use]
    pub fn for_own_service(self) -> Self {
        Self {
            peer: Peer::Own,
            ..self
        }
    }

    /// This sender without spans, measures or trace context
    /// ([`ClientCall::unobserved`]): for readiness checks, which run
    /// every few seconds and are not part of any request.
    #[must_use]
    pub fn unobserved(self) -> Self {
        Self {
            observed: false,
            ..self
        }
    }
}

/// An HTTP client on rustls with the ring provider and the system's CA
/// certificates, and whether those certificates were found. Plain
/// `http://` is refused unless `allow_http`.
///
/// # Errors
///
/// The ring provider supports no safe TLS version (does not happen with
/// the rustls versions in `Cargo.lock`).
pub fn https_client(allow_http: bool) -> Result<(HttpsClient, bool), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (https, has_ca_certificates) = match hyper_rustls::HttpsConnectorBuilder::new()
        .with_provider_and_native_roots(Arc::clone(&provider))
    {
        Ok(b) => (b, true),
        Err(e) => {
            tracing::warn!(error = %e, "no CA certificates: https calls will fail");
            let tls = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
            (
                hyper_rustls::HttpsConnectorBuilder::new().with_tls_config(tls),
                false,
            )
        }
    };
    let connector = if allow_http {
        https.https_or_http().enable_http1().build()
    } else {
        https.https_only().enable_http1().build()
    };
    let client = Client::builder(TokioExecutor::new()).build(connector);
    Ok((client, has_ca_certificates))
}

impl Sender for HttpSender {
    fn post<'a>(&'a self, url: &'a str, body: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.json(json_post(url, body, None)?, url).await })
    }

    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.json(json_get(url, None)?, url).await })
    }

    fn post_authorized<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let request = json_post(url, body, Some(authorization))?;
            self.json(request, url).await
        })
    }

    fn get_authorized<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move { self.json(json_get(url, Some(authorization))?, url).await })
    }

    fn post_authorized_within<'a>(
        &'a self,
        url: &'a str,
        body: &'a Value,
        authorization: &'a str,
        limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        Box::pin(async move {
            let request = json_post(url, body, Some(authorization)).map_err(HttpError::NotSent)?;
            self.raw_answer(url, request, limits).await
        })
    }

    fn get_authorized_within<'a>(
        &'a self,
        url: &'a str,
        authorization: &'a str,
        limits: AnswerLimits,
    ) -> BoxFuture<'a, Result<RawAnswer, HttpError>> {
        Box::pin(async move {
            let request = json_get(url, Some(authorization)).map_err(HttpError::NotSent)?;
            self.raw_answer(url, request, limits).await
        })
    }
}

/// A POST of `body` as JSON, with an `Authorization` header if given.
fn json_post(
    url: &str,
    body: &Value,
    authorization: Option<&str>,
) -> Result<hyper::Request<Full<Bytes>>, String> {
    let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let mut builder =
        hyper::Request::post(url).header(hyper::header::CONTENT_TYPE, "application/json");
    if let Some(value) = authorization {
        builder = builder.header(hyper::header::AUTHORIZATION, value);
    }
    builder
        .body(Full::new(Bytes::from(bytes)))
        .map_err(|e| e.to_string())
}

/// A GET that asks for JSON, with an `Authorization` header if given.
fn json_get(url: &str, authorization: Option<&str>) -> Result<hyper::Request<Full<Bytes>>, String> {
    let mut builder = hyper::Request::get(url).header(hyper::header::ACCEPT, "application/json");
    if let Some(value) = authorization {
        builder = builder.header(hyper::header::AUTHORIZATION, value);
    }
    builder
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())
}

impl HttpSender {
    /// POSTs `form`, already encoded as `application/x-www-form-urlencoded`
    /// (for example a token request to a Secure Token Service), and reads
    /// the JSON answer.
    ///
    /// # Errors
    ///
    /// The URL is not allowed, the request failed, or the answer is not
    /// HTTP 2xx.
    pub async fn post_form(&self, url: &str, form: String) -> Result<Value, String> {
        let request = hyper::Request::post(url)
            .header(
                hyper::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(hyper::header::ACCEPT, "application/json")
            .body(Full::new(Bytes::from(form)))
            .map_err(|e| e.to_string())?;
        self.json(request, url).await
    }

    /// POSTs `body` as JSON with the header `Authorization:
    /// <authorization>`, and returns the answer's `Location` header, if
    /// any. A DCP issuer answers a credential request with `201 Created`
    /// and the `Location` of the request status (DCP v1.0.1, section 6.4).
    ///
    /// # Errors
    ///
    /// The URL is not allowed ([`HttpError::NotSent`]), the request failed
    /// ([`HttpError::NoAnswer`]), or the answer is not HTTP 2xx
    /// ([`HttpError::Status`]).
    pub async fn post_for_location(
        &self,
        url: &str,
        body: &Value,
        authorization: &str,
    ) -> Result<Option<String>, HttpError> {
        let request = json_post(url, body, Some(authorization)).map_err(HttpError::NotSent)?;
        let (headers, _body) = self.call_with_headers(url, request, MAX_ANSWER).await?;
        let location = headers
            .get(hyper::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok(location)
    }

    /// GETs `url` asking for the media type `accept`, and returns the
    /// answer as text, read up to `max_bytes` (for example a status list
    /// credential as a JWT, `application/vc+jwt`).
    ///
    /// # Errors
    ///
    /// The URL is not allowed, the request failed, the answer is not HTTP
    /// 2xx, larger than `max_bytes`, or not UTF-8. [`HttpError::is_temporary`]
    /// tells whether trying again later may help.
    pub async fn get_text(
        &self,
        url: &str,
        accept: &str,
        max_bytes: usize,
    ) -> Result<String, HttpError> {
        let request = hyper::Request::get(url)
            .header(hyper::header::ACCEPT, accept)
            .body(Full::new(Bytes::new()))
            .map_err(|e| HttpError::NotSent(e.to_string()))?;
        let bytes = self.call(url, request, max_bytes).await?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| HttpError::BadAnswer("the answer is not UTF-8 text".to_owned()))
    }

    /// GETs `url` asking for JSON, and reads the answer as
    /// [`Sender::get`] does, but keeps the error's kind: a caller that
    /// must tell a temporary failure from a refusal
    /// ([`HttpError::is_temporary`]) uses this.
    ///
    /// # Errors
    ///
    /// As [`HttpSender::get_text`].
    pub async fn get_json(&self, url: &str) -> Result<Value, HttpError> {
        let request = json_get(url, None).map_err(HttpError::NotSent)?;
        self.json_answer(request, url).await
    }

    /// Like [`Sender::post_authorized`], but keeps the error's kind, as
    /// [`HttpSender::get_json`] does.
    ///
    /// # Errors
    ///
    /// As [`HttpSender::get_text`].
    pub async fn post_json_authorized(
        &self,
        url: &str,
        body: &Value,
        authorization: &str,
    ) -> Result<Value, HttpError> {
        let request = json_post(url, body, Some(authorization)).map_err(HttpError::NotSent)?;
        self.json_answer(request, url).await
    }

    /// POSTs `body` (text, for example a `vp+jwt`) with the media type
    /// `content_type`, and returns the answer as text, read up to
    /// `max_bytes`, waiting at most `timeout` for it. For services slower
    /// than a participant, such as a Gaia-X compliance service, which
    /// fetches documents itself before it answers.
    ///
    /// # Errors
    ///
    /// The URL is not allowed ([`HttpError::NotSent`]), the request failed
    /// ([`HttpError::NoAnswer`]), the answer is too large or not text
    /// ([`HttpError::BadAnswer`]), or the answer is not HTTP 2xx
    /// ([`HttpError::StatusWithBody`], with the start of the answer).
    /// Redirects are not followed: a `3xx` is a status error.
    pub async fn post_text(
        &self,
        url: &str,
        content_type: &str,
        body: String,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<String, HttpError> {
        let request = hyper::Request::post(url)
            .header(hyper::header::CONTENT_TYPE, content_type)
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| HttpError::NotSent(e.to_string()))?;
        let limits = Limits {
            max_bytes,
            timeout,
            error_body_bytes: ERROR_BODY_BYTES,
        };
        let (_headers, bytes) = self.call_within(url, request, limits).await?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| HttpError::BadAnswer("the answer is not UTF-8 text".to_owned()))
    }

    /// Sends `request` and returns the 2xx answer as it came, read within
    /// `limits`. A refusal keeps the start of its body (at most
    /// [`ERROR_BODY_BYTES`]): a DSP error says there why.
    async fn raw_answer(
        &self,
        url: &str,
        request: hyper::Request<Full<Bytes>>,
        limits: AnswerLimits,
    ) -> Result<RawAnswer, HttpError> {
        let limits = Limits {
            max_bytes: limits.max_bytes,
            timeout: limits.timeout,
            error_body_bytes: ERROR_BODY_BYTES,
        };
        let (headers, body) = self.call_within(url, request, limits).await?;
        let mut links = Vec::new();
        for value in headers.get_all(hyper::header::LINK) {
            // `to_str` fails for a value that is not visible ASCII
            if let Ok(text) = value.to_str() {
                links.push(text.to_owned());
            }
        }
        Ok(RawAnswer { body, links })
    }

    /// Like [`HttpSender::json_answer`], with the error as text, as the
    /// [`Sender`] port carries it.
    async fn json(&self, request: hyper::Request<Full<Bytes>>, url: &str) -> Result<Value, String> {
        self.json_answer(request, url)
            .await
            .map_err(|e| e.to_string())
    }

    /// Sends `request` and reads the answer as JSON (`Value::Null` when it
    /// is empty or not JSON), up to [`MAX_ANSWER`] bytes.
    async fn json_answer(
        &self,
        request: hyper::Request<Full<Bytes>>,
        url: &str,
    ) -> Result<Value, HttpError> {
        let bytes = self.call(url, request, MAX_ANSWER).await?;
        Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn call(
        &self,
        url: &str,
        request: hyper::Request<Full<Bytes>>,
        max_bytes: usize,
    ) -> Result<Bytes, HttpError> {
        let (_headers, body) = self.call_with_headers(url, request, max_bytes).await?;
        Ok(body)
    }

    /// Like [`HttpSender::call`], and returns the answer's headers too.
    async fn call_with_headers(
        &self,
        url: &str,
        request: hyper::Request<Full<Bytes>>,
        max_bytes: usize,
    ) -> Result<(hyper::HeaderMap, Bytes), HttpError> {
        let limits = Limits {
            max_bytes,
            timeout: SEND_TIMEOUT,
            error_body_bytes: 0,
        };
        self.call_within(url, request, limits).await
    }

    /// Like [`HttpSender::call_with_headers`], within `limits`. Each call
    /// that is sent gets a span and a duration measure ([`ClientCall`]).
    async fn call_within(
        &self,
        url: &str,
        mut request: hyper::Request<Full<Bytes>>,
        limits: Limits,
    ) -> Result<(hyper::HeaderMap, Bytes), HttpError> {
        let Limits { timeout, .. } = limits;
        // the text names callbacks, the first use of this rule; it now
        // covers every outbound URL, and tests look for this text
        if !outbound_url_allowed(url, self.allow_http) {
            return Err(HttpError::NotSent(format!("refused callback URL {url}")));
        }
        if !self.has_ca_certificates && url.to_ascii_lowercase().starts_with("https://") {
            return Err(HttpError::NotSent(
                "no CA certificates to verify the counter-party; mount the system bundle"
                    .to_owned(),
            ));
        }
        let mut call = if self.observed {
            ClientCall::start(&mut request, self.peer)
        } else {
            ClientCall::unobserved()
        };
        let span = call.span().clone();
        // `exchange` borrows `call` (`&mut call`) until it is done or
        // dropped; `timeout` drops it when the time is up, so `call` can
        // be ended in both branches below
        let exchange = self.exchange(request, limits, &mut call).instrument(span);
        // `timeout` gives an error only when the time is up; when the
        // exchange finished in time, `answer` is its own result, which may
        // be an error too, and is returned as it is
        let Ok(answer) = tokio::time::timeout(timeout, exchange).await else {
            call.end_timed_out();
            return Err(HttpError::NoAnswer(format!(
                "no answer within {}s",
                timeout.as_secs()
            )));
        };
        call.end();
        answer
    }

    /// Sends `request` and reads the answer within `limits`; records the
    /// answer's status in `call`.
    async fn exchange(
        &self,
        request: hyper::Request<Full<Bytes>>,
        limits: Limits,
        call: &mut ClientCall,
    ) -> Result<(hyper::HeaderMap, Bytes), HttpError> {
        use http_body_util::BodyExt as _;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|e| HttpError::NoAnswer(e.to_string()))?;
        let status = response.status();
        call.answered(status.as_u16());
        if !status.is_success() {
            return Err(refusal(response, limits.error_body_bytes).await);
        }
        // read at most max_bytes: the counter-party controls the size
        let (parts, body) = response.into_parts();
        let body = match http_body_util::Limited::new(body, limits.max_bytes)
            .collect()
            .await
        {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                // a 2xx status without its body is a failed call
                call.body_not_read();
                let reason = format!("answer not read: {e}");
                // too large: the same answer would come again; otherwise
                // the answer broke off, which may not happen next time
                if e.is::<http_body_util::LengthLimitError>() {
                    return Err(HttpError::BadAnswer(reason));
                }
                return Err(HttpError::NoAnswer(reason));
            }
        };
        Ok((parts.headers, body))
    }
}

/// How much one exchange may read and how long it may take.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// The largest successful answer read.
    max_bytes: usize,
    /// How long to wait for the whole answer.
    timeout: Duration,
    /// How much of a refusal's body to keep; 0 reads none of it.
    error_body_bytes: usize,
}

/// The error for a non-2xx `response`: its status, and with
/// `keep_bytes` above 0 the start of its body as text.
async fn refusal(response: hyper::Response<Incoming>, keep_bytes: usize) -> HttpError {
    use http_body_util::BodyExt as _;
    let code = response.status().as_u16();
    if keep_bytes == 0 {
        return HttpError::Status(code);
    }
    // read frame by frame and stop once enough is there: the counter-party
    // chooses the size, and the rest is not needed
    let mut body = response.into_body();
    let mut start = Vec::new();
    while start.len() < keep_bytes {
        match body.frame().await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    start.extend_from_slice(&data);
                }
            }
            // a body that breaks off keeps what came
            Some(Err(_)) | None => break,
        }
    }
    HttpError::StatusWithBody(code, text_start(&start, keep_bytes))
}

/// At most `max_bytes` of `bytes` as text, without a character cut in
/// two at the end; bytes that are not UTF-8 become U+FFFD.
fn text_start(bytes: &[u8], max_bytes: usize) -> String {
    let mut end = bytes.len().min(max_bytes);
    if let Err(error) = std::str::from_utf8(&bytes[..end])
        && error.error_len().is_none()
    {
        // only the last character is incomplete: leave it out
        end = error.valid_up_to();
    }
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// The HTTP status code in the text of an [`HttpError::Status`]
/// (`HTTP 404 Not Found`, for example); `None` when no answer came
/// (refused URL, connection error, timeout) or for any other text.
///
/// Only for errors that came through the [`Sender`] port, which carries
/// them as text (its test fakes word them the same way). Code that calls
/// [`HttpSender`] directly gets an [`HttpError`] and matches on it.
#[must_use]
pub fn answered_status(error: &str) -> Option<u16> {
    let after = error.strip_prefix("HTTP ")?;
    let code = after.get(..3)?;
    let rest = after.get(3..)?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    code.parse().ok().filter(|code| (100..600).contains(code))
}

/// Whether a Portitor process may send to `url`. Other participants
/// choose many of the URLs called (callback addresses, DID documents,
/// Credential Services, status lists), so only `https://` URLs with a
/// host are allowed, plus `http://` when `allow_http` is set.
#[must_use]
pub fn outbound_url_allowed(url: &str, allow_http: bool) -> bool {
    let lower = url.to_ascii_lowercase();
    let after_scheme = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if allow_http && let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return false;
    };
    // the host is everything up to the first '/', '?' or '#'
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // a '@' would put user credentials in the URL
    !host.is_empty() && !host.contains('@')
}

/// The token of an `Authorization` header value `Bearer <token>` (RFC
/// 6750, section 2.1), the one rule for every Portitor endpoint that
/// takes a bearer token. The scheme name may be written in any case
/// (`bearer`, `BEARER`; RFC 7235, section 2.1), and spaces around the
/// token are ignored. `None` for another scheme, an empty token, or a
/// token without the scheme in front (EDC and the DCP TCK refuse that
/// too).
#[must_use]
pub fn bearer_token(header_value: &str) -> Option<&str> {
    let (scheme, token) = header_value.split_once(' ')?;
    let token = token.trim();
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

/// The bearer token of a request's `Authorization` header, if any (see
/// [`bearer_token`]). The value is read as UTF-8, so a management API
/// key with non-ASCII characters still works.
#[must_use]
pub fn bearer_token_in(headers: &hyper::HeaderMap) -> Option<&str> {
    let value = headers.get(hyper::header::AUTHORIZATION)?;
    let text = std::str::from_utf8(value.as_bytes()).ok()?;
    bearer_token(text)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn answered_status_is_read_back_from_the_error() {
        let error = format!("HTTP {}", hyper::StatusCode::NOT_FOUND);
        assert_eq!(answered_status(&error), Some(404));
        assert_eq!(answered_status("HTTP 503"), Some(503));
        assert_eq!(answered_status("no answer within 30s"), None);
        assert_eq!(answered_status("HTTP 4040 x"), None);
        assert_eq!(answered_status("HTTP abc"), None);
        assert_eq!(answered_status("HTTP 999 x"), None);
    }

    #[test]
    fn temporary_failures_are_no_answer_408_429_and_5xx() {
        let no_answer = HttpError::NoAnswer("no answer within 10s".to_owned());
        assert!(no_answer.is_temporary());
        for code in [408, 429, 500, 502, 503, 504] {
            assert!(HttpError::Status(code).is_temporary(), "{code}");
            let with_body = HttpError::StatusWithBody(code, String::new());
            assert!(with_body.is_temporary(), "{code}");
        }
        for code in [302, 400, 401, 403, 404, 409, 410] {
            assert!(!HttpError::Status(code).is_temporary(), "{code}");
        }
        let not_sent = HttpError::NotSent("refused callback URL http://x".to_owned());
        assert!(!not_sent.is_temporary());
        let too_large = HttpError::BadAnswer("answer not read: length limit exceeded".to_owned());
        assert!(!too_large.is_temporary());
    }

    #[test]
    fn bearer_tokens_are_read_from_the_authorization_header() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer  abc "), Some("abc"));
        assert_eq!(bearer_token("BEARER abc"), Some("abc"));
        for refused in ["", "abc", "Bearer", "Bearer  ", "Basic abc", "Bearerabc"] {
            assert_eq!(bearer_token(refused), None, "{refused:?}");
        }
        let mut headers = hyper::HeaderMap::new();
        assert_eq!(bearer_token_in(&headers), None);
        headers.insert(hyper::header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert_eq!(bearer_token_in(&headers), Some("abc"));
        headers.insert(
            hyper::header::AUTHORIZATION,
            hyper::header::HeaderValue::from_bytes("Bearer k\u{e9}y".as_bytes()).unwrap(),
        );
        assert_eq!(bearer_token_in(&headers), Some("k\u{e9}y"));
    }

    #[test]
    fn outbound_urls() {
        assert!(outbound_url_allowed("https://c.example/cb", false));
        assert!(outbound_url_allowed("HTTPS://c.example", false));
        assert!(!outbound_url_allowed("http://c.example/cb", false));
        assert!(outbound_url_allowed("http://c.example/cb", true));
        for bad in [
            "",
            "https://",
            "https:///x",
            "https://user@c.example",
            "file:///etc",
            "c.example",
        ] {
            assert!(!outbound_url_allowed(bad, true), "{bad}");
        }
    }

    async fn serve(status: axum::http::StatusCode) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app =
            axum::Router::new().route("/cb", axum::routing::post(move || async move { status }));
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}/cb")
    }

    #[tokio::test]
    async fn http_sender_posts_and_reports_status() {
        let sender = HttpSender::new(true).unwrap();
        let body = serde_json::json!({"@type": "ContractOfferMessage"});
        let ok = serve(axum::http::StatusCode::OK).await;
        assert!(sender.post(&ok, &body).await.is_ok());
        let failing = serve(axum::http::StatusCode::INTERNAL_SERVER_ERROR).await;
        assert!(
            sender
                .post(&failing, &body)
                .await
                .unwrap_err()
                .contains("500")
        );
        assert!(sender.post("http://127.0.0.1:1/cb", &body).await.is_err());
    }

    /// Echoes the request's `Authorization` and `Content-Type` headers and
    /// body as JSON (POST /echo), and serves a plain-text JWT (GET /jwt).
    async fn echo() -> String {
        use axum::http::HeaderMap;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let header = |headers: &HeaderMap, name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        let app = axum::Router::new()
            .route(
                "/echo",
                axum::routing::post(move |headers: HeaderMap, body: String| async move {
                    axum::Json(serde_json::json!({
                        "authorization": header(&headers, "authorization"),
                        "contentType": header(&headers, "content-type"),
                        "body": body,
                    }))
                })
                .get(move |headers: HeaderMap| async move {
                    axum::Json(
                        serde_json::json!({"authorization": header(&headers, "authorization")}),
                    )
                }),
            )
            .route(
                "/jwt",
                axum::routing::get(move |headers: HeaderMap| async move {
                    format!("a.b.c accept={}", header(&headers, "accept"))
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn http_sender_sends_authorization_forms_and_reads_text() {
        let base = echo().await;
        let sender = HttpSender::new(true).unwrap();
        let url = format!("{base}/echo");
        let body = serde_json::json!({"a": 1});
        let answer = sender
            .post_authorized(&url, &body, "Bearer t0ken")
            .await
            .unwrap();
        assert_eq!(answer["authorization"], "Bearer t0ken");
        assert_eq!(answer["contentType"], "application/json");
        let answer = sender.get_authorized(&url, "Bearer x").await.unwrap();
        assert_eq!(answer["authorization"], "Bearer x");
        // without the header
        assert_eq!(sender.get(&url).await.unwrap()["authorization"], "");
        let answer = sender.post_form(&url, "a=1&b=2".to_owned()).await.unwrap();
        assert_eq!(answer["contentType"], "application/x-www-form-urlencoded");
        assert_eq!(answer["body"], "a=1&b=2");
        let jwt = format!("{base}/jwt");
        let text = sender
            .get_text(&jwt, "application/vc+jwt", 1024)
            .await
            .unwrap();
        assert_eq!(text, "a.b.c accept=application/vc+jwt");
        assert!(sender.get_text(&jwt, "text/plain", 3).await.is_err());
        let echoed = sender
            .post_text(
                &url,
                "application/vp+jwt",
                "h.p.s".to_owned(),
                1024,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        let echoed: Value = serde_json::from_str(&echoed).unwrap();
        assert_eq!(echoed["contentType"], "application/vp+jwt");
        assert_eq!(echoed["body"], "h.p.s");
        let too_long = sender
            .post_text(
                &url,
                "text/plain",
                "x".to_owned(),
                3,
                Duration::from_secs(5),
            )
            .await;
        assert!(matches!(too_long, Err(HttpError::BadAnswer(_))));
    }

    #[tokio::test]
    async fn http_sender_reads_the_location_of_a_created_resource() {
        use axum::http::{HeaderMap, StatusCode, header};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route(
                "/credentials",
                axum::routing::post(move |headers: HeaderMap| async move {
                    let authorization = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    let location = format!("/requests/{authorization}");
                    (StatusCode::CREATED, [(header::LOCATION, location)])
                }),
            )
            .route("/plain", axum::routing::post(|| async { StatusCode::OK }));
        tokio::spawn(async move { axum::serve(listener, app).await });
        let sender = HttpSender::new(true).unwrap();
        let body = serde_json::json!({"type": "CredentialRequestMessage"});
        let created = sender
            .post_for_location(&format!("http://{addr}/credentials"), &body, "t0ken")
            .await
            .unwrap();
        assert_eq!(created.as_deref(), Some("/requests/t0ken"));
        let plain = sender
            .post_for_location(&format!("http://{addr}/plain"), &body, "t0ken")
            .await
            .unwrap();
        assert_eq!(plain, None);
    }

    #[tokio::test]
    async fn the_status_of_a_refusal_can_be_read_from_the_error() {
        use axum::http::StatusCode;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route(
                "/bad",
                axum::routing::post(|| async { StatusCode::BAD_REQUEST }),
            )
            .route(
                "/down",
                axum::routing::post(|| async { StatusCode::SERVICE_UNAVAILABLE }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let sender = HttpSender::new(true).unwrap();
        let body = serde_json::json!({});
        let status_of = |path: &'static str| {
            let sender = &sender;
            let body = &body;
            async move {
                let url = format!("http://{addr}{path}");
                let error = sender.post_for_location(&url, body, "t").await.unwrap_err();
                // the text form, as the `Sender` port passes it on, says
                // the same
                assert_eq!(
                    answered_status(&error.to_string()),
                    match error {
                        HttpError::Status(code) | HttpError::StatusWithBody(code, _) => Some(code),
                        HttpError::NoAnswer(_)
                        | HttpError::NotSent(_)
                        | HttpError::BadAnswer(_) => None,
                    }
                );
                error
            }
        };
        assert_eq!(status_of("/bad").await, HttpError::Status(400));
        assert_eq!(status_of("/down").await, HttpError::Status(503));
        assert_eq!(status_of("/bad").await.to_string(), "HTTP 400 Bad Request");
        // no answer at all: a refused URL (not sent), or nobody listening
        let refused = sender
            .post_for_location("ftp://example.com/x", &body, "t")
            .await
            .unwrap_err();
        assert!(matches!(refused, HttpError::NotSent(_)), "{refused}");
        assert_eq!(answered_status(&refused.to_string()), None);
        let unreachable = sender
            .post_for_location("http://127.0.0.1:9/x", &body, "t")
            .await
            .unwrap_err();
        assert!(
            matches!(unreachable, HttpError::NoAnswer(_)),
            "{unreachable}"
        );
    }

    /// `post_text` keeps the start of a refusal's body, so a compliance
    /// service's reason reaches the run's error.
    #[tokio::test]
    async fn post_text_keeps_the_start_of_a_refusal() {
        use axum::http::StatusCode;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // "é" is two bytes; with one "x" first, byte 2,048 is in the
        // middle of one
        let long = format!("x{}", "é".repeat(ERROR_BODY_BYTES));
        let app = axum::Router::new()
            .route(
                "/reason",
                axum::routing::post(|| async { (StatusCode::CONFLICT, "  bad digestSRI\n") }),
            )
            .route(
                "/long",
                axum::routing::post(move || async move { (StatusCode::BAD_REQUEST, long) }),
            )
            .route(
                "/empty",
                axum::routing::post(|| async { StatusCode::BAD_GATEWAY }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let sender = HttpSender::new(true).unwrap();
        let refusal = |path: &'static str| {
            let sender = &sender;
            async move {
                let url = format!("http://{addr}{path}");
                let timeout = Duration::from_secs(5);
                let answer = sender.post_text(&url, "text/plain", String::new(), 64, timeout);
                answer.await.unwrap_err()
            }
        };
        let error = refusal("/reason").await;
        assert_eq!(error.to_string(), "HTTP 409 Conflict: bad digestSRI");
        assert_eq!(answered_status(&error.to_string()), Some(409));
        let HttpError::StatusWithBody(400, start) = refusal("/long").await else {
            panic!("not a refusal with a body");
        };
        assert_eq!(start.len(), ERROR_BODY_BYTES - 1);
        assert!(start.starts_with("xé"));
        assert_eq!(refusal("/empty").await.to_string(), "HTTP 502 Bad Gateway");
    }

    #[test]
    fn text_start_cuts_at_a_character() {
        assert_eq!(text_start("aé".as_bytes(), 2), "a");
        assert_eq!(text_start("aé".as_bytes(), 3), "aé");
        assert_eq!(text_start(b"a\xffb", 3), "a\u{fffd}b");
    }

    /// A sender with only the required methods: the authorized variants
    /// fall back to them.
    struct Plain;

    impl Sender for Plain {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Ok(Value::from("posted")) })
        }
    }

    /// A large answer is read up to the caller's limit and comes back
    /// as it was sent, with its `Link` headers; a refusal keeps the
    /// start of its body.
    #[tokio::test]
    async fn http_sender_reads_an_answer_within_the_callers_limits() {
        use axum::http::{StatusCode, header};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let paged = || async {
            // `AppendHeaders` sends both headers; a plain array would
            // keep only the last one of the same name
            let links = axum::response::AppendHeaders([
                (header::LINK, "<https://p.example/c?page=2>; rel=\"next\""),
                (
                    header::LINK,
                    "<https://p.example/c?page=0>; rel=\"previous\"",
                ),
            ]);
            (links, "0123456789")
        };
        let app = axum::Router::new()
            .route("/paged", axum::routing::post(paged).get(paged))
            .route(
                "/refused",
                axum::routing::get(|| async { (StatusCode::NOT_FOUND, "no such dataset") }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await });
        let sender = HttpSender::new(true).unwrap();
        let limits = |max_bytes| AnswerLimits {
            max_bytes,
            timeout: Duration::from_secs(5),
        };
        let url = format!("http://{addr}/paged");
        let body = serde_json::json!({});
        let posted = sender.post_authorized_within(&url, &body, "Bearer t", limits(10));
        let posted = posted.await.unwrap();
        assert_eq!(&posted.body[..], b"0123456789");
        assert_eq!(
            posted.links,
            [
                "<https://p.example/c?page=2>; rel=\"next\"",
                "<https://p.example/c?page=0>; rel=\"previous\"",
            ]
        );
        let got = sender.get_authorized_within(&url, "Bearer t", limits(10));
        assert_eq!(got.await.unwrap(), posted);
        let too_large = sender.get_authorized_within(&url, "Bearer t", limits(9));
        assert!(matches!(too_large.await, Err(HttpError::BadAnswer(_))));
        let refused = format!("http://{addr}/refused");
        let refused = sender.get_authorized_within(&refused, "Bearer t", limits(10));
        assert_eq!(
            refused.await.unwrap_err().to_string(),
            "HTTP 404 Not Found: no such dataset"
        );
    }

    #[tokio::test]
    async fn default_methods_fall_back() {
        let body = serde_json::json!({});
        assert_eq!(
            Plain.post_authorized("u", &body, "x").await.unwrap(),
            "posted"
        );
        assert!(Plain.get_authorized("u", "x").await.is_err());
        // a large answer cannot be read through the plain methods
        let limits = AnswerLimits {
            max_bytes: 1,
            timeout: Duration::from_secs(1),
        };
        let posted = Plain.post_authorized_within("u", &body, "x", limits);
        assert!(matches!(posted.await, Err(HttpError::NotSent(_))));
        let got = Plain.get_authorized_within("u", "x", limits);
        assert!(matches!(got.await, Err(HttpError::NotSent(_))));
    }

    #[tokio::test]
    async fn http_sender_refuses_http_without_the_flag() {
        let sender = HttpSender::new(false).unwrap();
        let err = sender
            .post("http://127.0.0.1:1/cb", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("refused"));
    }

    #[tokio::test]
    async fn without_ca_certificates_https_fails_with_a_reason() {
        let mut sender = HttpSender::new(false).unwrap();
        sender.has_ca_certificates = false;
        let err = sender
            .post("https://127.0.0.1:1/cb", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("CA certificates"));
    }
}
