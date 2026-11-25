// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading a flow's source, for a pull (`serving.rs`) and a push
//! (`push.rs`): an upstream URL, fetched with a plain `GET`, or an object
//! in the connector's object storage (`s3://{bucket}/{key}`, ADR 0027,
//! section 3), read with a signed `GET`. Either is streamed through,
//! never held whole. The source is the operator's: errors and logs never
//! name its URL.

use std::time::Duration;

use axum::body::Body;
use axum::http::header;
use http_body_util::Full;
use hyper::body::Bytes;
use portitor_runtime::http::HttpsClient;
use portitor_runtime::telemetry::http::{ClientCall, Peer};
use tracing::Instrument as _;

use crate::object_storage::parse_s3_uri;
use crate::plane::HttpDataPlane;

/// How long the upstream may take to send its answer's headers.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

/// A source's answer: its media type and its bytes as they come.
pub struct SourceData {
    /// The source's `Content-Type`, if it gave one.
    pub content_type: Option<header::HeaderValue>,
    /// The bytes.
    pub body: Body,
}

impl HttpDataPlane {
    /// Whether this data plane can read `source`: an `http(s)` URL, or an
    /// `s3://` object when the connector has object storage.
    pub(crate) fn can_read(&self, source: &str) -> bool {
        parse_s3_uri(source).is_none() || self.storage.is_some()
    }

    /// Opens `source`, an upstream URL or an `s3://` object, and returns
    /// its media type and its bytes as a stream (2xx answers only).
    ///
    /// # Errors
    ///
    /// The source did not answer, or not with 2xx; the text names no URL.
    pub async fn open_source(&self, source: &str) -> Result<SourceData, String> {
        match parse_s3_uri(source) {
            Some(object) => {
                let Some(storage) = &self.storage else {
                    return Err(
                        "the source is in object storage, which is not configured".to_owned()
                    );
                };
                let found = storage
                    .get(&object.bucket, &object.key)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(SourceData {
                    content_type: found.content_type,
                    body: Body::new(found.body),
                })
            }
            None => fetch(&self.client, source).await,
        }
    }
}

/// GETs `url` and returns a 2xx answer's media type and body, streamed.
/// The data source is our participant's own, so the request carries our
/// trace context (ADR 0023, section 4). Its span ends when the answer's
/// headers came; the body is streamed afterwards.
async fn fetch(client: &HttpsClient, url: &str) -> Result<SourceData, String> {
    let mut request = hyper::Request::get(url)
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let mut call = ClientCall::start(&mut request, Peer::Own);
    let sent = client.request(request).instrument(call.span().clone());
    // two results in one: the outer `Ok`/`Err` is the timeout's (did the
    // answer come in time?), the inner one the request's own (did it
    // work?)
    let answer = match tokio::time::timeout(UPSTREAM_TIMEOUT, sent).await {
        Ok(Ok(answer)) => {
            call.answered(answer.status().as_u16());
            call.end();
            answer
        }
        Ok(Err(error)) => {
            call.end();
            return Err(error.to_string());
        }
        Err(_) => {
            call.end_timed_out();
            return Err(format!("no answer within {}s", UPSTREAM_TIMEOUT.as_secs()));
        }
    };
    if !answer.status().is_success() {
        return Err(format!("upstream answered HTTP {}", answer.status()));
    }
    let content_type = answer.headers().get(header::CONTENT_TYPE).cloned();
    Ok(SourceData {
        content_type,
        body: Body::new(answer.into_body()),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_runtime::http::https_client;

    use super::*;

    /// The data source is our own: the fetch carries our trace context
    /// (ADR 0023, section 4).
    #[tokio::test]
    async fn the_fetch_from_our_data_source_carries_trace_context() {
        use portitor_runtime::telemetry::testing::{
            recorded, trace_on_this_thread, traceparent_recorder,
        };
        let traced = trace_on_this_thread();
        let (base, recorder) = traceparent_recorder().await;
        let (client, _) = https_client(true).unwrap();
        let span = tracing::info_span!(target: "portitor_test", "request");
        let response = fetch(&client, &format!("{base}/d1")).instrument(span).await;
        assert!(response.is_ok());
        let got = recorded(&recorder);
        assert_eq!(got.len(), 1);
        assert!(
            got[0].as_deref().is_some_and(|t| t.starts_with("00-")),
            "{got:?}"
        );
        assert!(traced.spans().iter().any(|span| span.name == "GET"));
    }
}
