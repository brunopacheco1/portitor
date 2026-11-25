// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A small client for S3-compatible object storage (ADR 0027, sections 3
//! and 4): one store per connector, for the provider's `s3://` sources
//! and for the data a consumer receives by push.
//!
//! - Settings: the standard AWS SDK environment variables and
//!   `PORTITOR_PUSH_BUCKET` ([`Settings::from_vars`]). They are checked
//!   only when `PORTITOR_PUSH_BUCKET` is set: other `AWS_*` variables in
//!   the environment do not stop a connector that does not use object
//!   storage, as in 1.0.
//! - Requests are signed with AWS Signature Version 4 ([`sigv4`]) and use
//!   path-style addresses, `{endpoint}/{bucket}/{key}`, which `RustFS`,
//!   `MinIO` and AWS S3 accept.
//! - [`ObjectStorage::get`] streams an object; [`ObjectStorage::upload`]
//!   streams a body of unknown length into an object with a multipart
//!   upload, a part of [`PART_BYTES`] at a time, so the body is never
//!   held in memory whole.
//!
//! The keys and the signatures never go into a log line or an error text.

use std::time::Duration;

use axum::body::Body;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, header};
use portitor_runtime::settings;

use crate::http_client::{HttpsClient, https_client};
use crate::system::now_secs;

pub mod sigv4;
mod xml;

/// The size of each part of a multipart upload: 8 MiB. S3 needs at least
/// 5 MiB for every part but the last, and allows 10,000 parts, so one
/// object can hold up to 80 GiB.
pub const PART_BYTES: usize = 8 * 1024 * 1024;

/// The most parts of one upload (an S3 limit).
const MAX_PARTS: u32 = 10_000;

/// How long the store may take to start its answer to a `GET`.
const GET_TIMEOUT: Duration = Duration::from_secs(30);

/// How long any other request may take, answer included: an upload of
/// one part, or a small control request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The largest answer body read from the store (XML documents only).
const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// The region signed when `AWS_REGION` is not set. `RustFS` and `MinIO`
/// accept any; `us-east-1` is the usual default.
pub const DEFAULT_REGION: &str = "us-east-1";

/// The object storage settings (ADR 0027, section 4).
#[derive(Clone)]
pub struct Settings {
    /// The store's base URL (`AWS_ENDPOINT_URL_S3`), without a trailing
    /// `/`; AWS S3 of the region when not set.
    pub endpoint: String,
    /// The region in the signature (`AWS_REGION`).
    pub region: String,
    /// `AWS_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// `AWS_SECRET_ACCESS_KEY`. Never logged.
    pub secret_access_key: String,
    /// `AWS_SESSION_TOKEN`, for temporary keys; sent and signed as
    /// `x-amz-security-token`. Never logged.
    pub session_token: Option<String>,
    /// Where pushed data lands (`PORTITOR_PUSH_BUCKET`). Without it the
    /// connector does not receive push transfers.
    pub push_bucket: Option<String>,
}

/// The keys are left out.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("push_bucket", &self.push_bucket)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Reads the settings from the environment; see
    /// [`Settings::from_vars`].
    ///
    /// # Errors
    ///
    /// See [`Settings::from_vars`].
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// Reads `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (both, or
    /// neither: then there is no object storage, `None`),
    /// `AWS_SESSION_TOKEN` (optional), `AWS_REGION` (default
    /// [`DEFAULT_REGION`]), `AWS_ENDPOINT_URL_S3` (default AWS S3 of the
    /// region) and `PORTITOR_PUSH_BUCKET`, looked up by `var`. The keys
    /// follow the AWS SDK rule (the value itself), not Portitor's `_FILE`
    /// rule.
    ///
    /// Without `PORTITOR_PUSH_BUCKET`, settings that cannot be used are
    /// not an error: the connector then has no object storage (logged at
    /// INFO), as a 1.0 connector with other `AWS_*` variables set had
    /// none. Its `s3://` sources, if any, are then refused per transfer.
    ///
    /// # Errors
    ///
    /// With `PORTITOR_PUSH_BUCKET` set: the keys are not both set, or
    /// `AWS_ENDPOINT_URL_S3` is not an `http(s)` URL without a query.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Option<Self>, String> {
        let push_bucket = settings::text(&var, "PORTITOR_PUSH_BUCKET");
        match Self::read(&var, push_bucket.clone()) {
            Err(reason) if push_bucket.is_none() => {
                tracing::info!(%reason, "no object storage: its AWS_* settings cannot be used");
                Ok(None)
            }
            read => read,
        }
    }

    /// [`Settings::from_vars`], with every problem an error.
    fn read(
        var: impl Fn(&str) -> Option<std::ffi::OsString>,
        push_bucket: Option<String>,
    ) -> Result<Option<Self>, String> {
        let key_id = settings::text(&var, "AWS_ACCESS_KEY_ID");
        let secret = settings::text(&var, "AWS_SECRET_ACCESS_KEY");
        let (access_key_id, secret_access_key) = match (key_id, secret) {
            (Some(id), Some(secret)) => (id, secret),
            (None, None) if push_bucket.is_some() => {
                return Err(
                    "PORTITOR_PUSH_BUCKET is set, but AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY are not: \
                     receiving push transfers needs object storage"
                        .to_owned(),
                );
            }
            (None, None) => return Ok(None),
            _ => {
                return Err(
                    "AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY: set both, or neither".to_owned(),
                );
            }
        };
        let region =
            settings::text(&var, "AWS_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let endpoint = match settings::text(&var, "AWS_ENDPOINT_URL_S3") {
            Some(url) => url,
            None => format!("https://s3.{region}.amazonaws.com"),
        };
        if settings::base_path(&endpoint).is_none() {
            return Err(format!(
                "AWS_ENDPOINT_URL_S3: {endpoint:?} is not an http(s) URL without a query or fragment"
            ));
        }
        Ok(Some(Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            region,
            access_key_id,
            secret_access_key,
            session_token: settings::text(&var, "AWS_SESSION_TOKEN"),
            push_bucket,
        }))
    }
}

/// An object named by an `s3://{bucket}/{key}` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectAddress {
    /// The bucket.
    pub bucket: String,
    /// The key, which may contain `/`.
    pub key: String,
}

/// The bucket and key of `uri` if it is `s3://{bucket}/{key}` with both
/// parts non-empty, the form the AWS command line and SDKs use
/// (`portitor:source`, ADR 0027, section 3).
#[must_use]
pub fn parse_s3_uri(uri: &str) -> Option<ObjectAddress> {
    let rest = uri.strip_prefix("s3://")?;
    let (bucket, key) = rest.split_once('/')?;
    let bucket_ok = !bucket.is_empty()
        && bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
    if !bucket_ok || key.is_empty() || key.contains(['?', '#']) {
        return None;
    }
    Some(ObjectAddress {
        bucket: bucket.to_owned(),
        key: key.to_owned(),
    })
}

/// Why a request to the store failed. The texts name no key, URL or
/// signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The store answered with this status, and the S3 error code of its
    /// answer if it sent one (for example `NoSuchKey`).
    Status(u16, Option<String>),
    /// No usable answer: no connection, too late, or an answer that
    /// cannot be read.
    NoAnswer(String),
    /// The body to upload could not be read (the sender stopped).
    Body(String),
    /// The body is larger than one object can hold ([`MAX_PARTS`] parts).
    TooLarge,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code, Some(s3_code)) => {
                write!(f, "object storage answered HTTP {code} ({s3_code})")
            }
            Self::Status(code, None) => write!(f, "object storage answered HTTP {code}"),
            Self::NoAnswer(why) => write!(f, "no answer from object storage: {why}"),
            Self::Body(why) => write!(f, "the body could not be read: {why}"),
            Self::TooLarge => write!(
                f,
                "the body is larger than {MAX_PARTS} parts of {PART_BYTES} bytes"
            ),
        }
    }
}

impl std::error::Error for StorageError {}

/// A streamed object: its `Content-Type` if the store gave one, and its
/// body.
#[derive(Debug)]
pub struct Object {
    /// The object's media type.
    pub content_type: Option<header::HeaderValue>,
    /// The bytes, as they come.
    pub body: Incoming,
}

/// The client of one S3-compatible store.
#[derive(Clone)]
pub struct ObjectStorage {
    settings: Settings,
    /// `host[:port]` of the endpoint: the `host` header, signed.
    host: String,
    /// The endpoint's path (often empty), put before `/{bucket}/{key}`.
    base_path: String,
    client: HttpsClient,
}

impl std::fmt::Debug for ObjectStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStorage")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl ObjectStorage {
    /// A client for the store of `settings`. The endpoint is set by the
    /// operator, so plain `http://` is allowed (a store on the same
    /// private network).
    ///
    /// # Errors
    ///
    /// The endpoint has no host, or the HTTP client cannot be built.
    pub fn new(settings: Settings) -> Result<Self, String> {
        let after_scheme = settings
            .endpoint
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let (host, path) = match after_scheme.split_once('/') {
            Some((host, path)) => (host, format!("/{path}")),
            None => (after_scheme, String::new()),
        };
        if host.is_empty() || host.contains('@') {
            return Err("AWS_ENDPOINT_URL_S3 has no host".to_owned());
        }
        let (client, _) = https_client(true)?;
        Ok(Self {
            host: host.to_owned(),
            base_path: path.trim_end_matches('/').to_owned(),
            settings,
            client,
        })
    }

    /// The bucket for pushed data, if one is set.
    #[must_use]
    pub fn push_bucket(&self) -> Option<&str> {
        self.settings.push_bucket.as_deref()
    }

    /// The encoded path of `key` in `bucket`.
    fn path(&self, bucket: &str, key: &str) -> String {
        format!(
            "{}/{}/{}",
            self.base_path,
            sigv4::uri_encode(bucket, false),
            sigv4::uri_encode(key, true)
        )
    }

    /// A signed request. `query` is not encoded yet; `body` is the whole
    /// body (a part at most).
    fn signed(
        &self,
        method: &Method,
        path: &str,
        query: &[(&str, &str)],
        content_type: Option<&str>,
        body: Bytes,
    ) -> Result<hyper::Request<Full<Bytes>>, StorageError> {
        let payload_sha256 = sigv4::sha256_hex(&body);
        let amz_date = sigv4::amz_date(now_secs());
        let mut headers = vec![
            ("host", self.host.as_str()),
            ("x-amz-content-sha256", payload_sha256.as_str()),
            ("x-amz-date", amz_date.as_str()),
        ];
        if let Some(content_type) = content_type {
            headers.push(("content-type", content_type));
        }
        if let Some(token) = &self.settings.session_token {
            // temporary keys: the token is sent and signed (SigV4)
            headers.push(("x-amz-security-token", token));
        }
        let scope = sigv4::Scope {
            amz_date: &amz_date,
            region: &self.settings.region,
            service: "s3",
        };
        let to_sign = sigv4::Request {
            method: method.as_str(),
            path,
            query,
            headers: &headers,
            payload_sha256: &payload_sha256,
        };
        let authorization = sigv4::authorization(
            &self.settings.access_key_id,
            &self.settings.secret_access_key,
            &scope,
            &to_sign,
        );
        let query_text = sigv4::canonical_query(query);
        let scheme = if self.settings.endpoint.starts_with("http://") {
            "http"
        } else {
            "https"
        };
        let url = if query_text.is_empty() {
            format!("{scheme}://{}{path}", self.host)
        } else {
            format!("{scheme}://{}{path}?{query_text}", self.host)
        };
        let mut builder = hyper::Request::builder()
            .method(method.clone())
            .uri(url)
            .header(header::AUTHORIZATION, authorization);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        builder
            .body(Full::new(body))
            .map_err(|_| StorageError::NoAnswer("the request could not be built".to_owned()))
    }

    /// Sends `request` and waits up to `timeout` for the answer's
    /// headers. A non-2xx answer is read (a little) for its S3 error code.
    async fn send(
        &self,
        request: hyper::Request<Full<Bytes>>,
        timeout: Duration,
    ) -> Result<hyper::Response<Incoming>, StorageError> {
        let answer = match tokio::time::timeout(timeout, self.client.request(request)).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(error)) => return Err(StorageError::NoAnswer(error.to_string())),
            Err(_) => {
                return Err(StorageError::NoAnswer(format!(
                    "no answer within {}s",
                    timeout.as_secs()
                )));
            }
        };
        if answer.status().is_success() {
            return Ok(answer);
        }
        let status = answer.status().as_u16();
        let text = read_text(answer.into_body()).await.unwrap_or_default();
        Err(StorageError::Status(status, xml::text_of(&text, "Code")))
    }

    /// Sends `request` and reads its whole (small) answer as text.
    async fn exchange(
        &self,
        request: hyper::Request<Full<Bytes>>,
    ) -> Result<(hyper::HeaderMap, String), StorageError> {
        let answer = self.send(request, REQUEST_TIMEOUT).await?;
        let headers = answer.headers().clone();
        let text = tokio::time::timeout(REQUEST_TIMEOUT, read_text(answer.into_body()))
            .await
            .map_err(|_| StorageError::NoAnswer("the answer took too long".to_owned()))??;
        Ok((headers, text))
    }

    /// `GetObject`: streams the object `key` of `bucket`. The store must
    /// start its answer within 30 seconds; the body then comes as it is
    /// read.
    ///
    /// # Errors
    ///
    /// The store answered with an error (404 for an unknown object), or
    /// did not answer.
    pub async fn get(&self, bucket: &str, key: &str) -> Result<Object, StorageError> {
        let path = self.path(bucket, key);
        let request = self.signed(&Method::GET, &path, &[], None, Bytes::new())?;
        let answer = self.send(request, GET_TIMEOUT).await?;
        let content_type = answer.headers().get(header::CONTENT_TYPE).cloned();
        Ok(Object {
            content_type,
            body: answer.into_body(),
        })
    }

    /// `PutObject`: stores `body` as the object `key` of `bucket` in one
    /// request.
    ///
    /// # Errors
    ///
    /// The store refused or did not answer.
    pub async fn put(
        &self,
        bucket: &str,
        key: &str,
        content_type: Option<&str>,
        body: Bytes,
    ) -> Result<(), StorageError> {
        let path = self.path(bucket, key);
        let request = self.signed(&Method::PUT, &path, &[], content_type, body)?;
        self.exchange(request).await.map(|_| ())
    }

    /// Streams `body` into the object `key` of `bucket` and returns how
    /// many bytes it held. A body that fits into one part goes up with
    /// one `PutObject`; a larger one with a multipart upload
    /// (`CreateMultipartUpload`, one `UploadPart` per [`PART_BYTES`],
    /// `CompleteMultipartUpload`), which is aborted
    /// (`AbortMultipartUpload`) when anything fails, and also when the
    /// upload is dropped before it ends (in the background then). The
    /// object then does not change. An existing object is replaced.
    ///
    /// # Errors
    ///
    /// The body could not be read, or the store refused or did not answer.
    pub async fn upload(
        &self,
        bucket: &str,
        key: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> Result<u64, StorageError> {
        let mut reader = PartReader::new(body);
        let first = reader.next_part().await?;
        if reader.finished {
            let bytes = first.len() as u64;
            self.put(bucket, key, content_type, first).await?;
            return Ok(bytes);
        }
        let upload_id = self.create_multipart(bucket, key, content_type).await?;
        let mut unfinished = Unfinished {
            storage: self.clone(),
            bucket: bucket.to_owned(),
            key: key.to_owned(),
            upload_id: upload_id.clone(),
            ended: false,
        };
        let uploaded = self
            .upload_parts(bucket, key, &upload_id, first, &mut reader)
            .await;
        // completed, or aborted just below: nothing for the guard to do
        unfinished.ended = true;
        match uploaded {
            Ok(bytes) => Ok(bytes),
            Err(error) => {
                // best effort: the store drops the parts sent so far
                if let Err(abort_error) = self.abort_multipart(bucket, key, &upload_id).await {
                    tracing::warn!(error = %abort_error, "object storage: an unfinished upload could not be aborted");
                }
                Err(error)
            }
        }
    }

    /// The parts after `first`, read from `reader`, then the completion.
    async fn upload_parts(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        first: Bytes,
        reader: &mut PartReader,
    ) -> Result<u64, StorageError> {
        let mut parts: Vec<(u32, String)> = Vec::new();
        let mut bytes = 0u64;
        let mut part = first;
        loop {
            let number = u32::try_from(parts.len() + 1).unwrap_or(u32::MAX);
            if number > MAX_PARTS {
                return Err(StorageError::TooLarge);
            }
            bytes += part.len() as u64;
            let etag = self
                .upload_part(bucket, key, upload_id, number, part)
                .await?;
            parts.push((number, etag));
            if reader.finished {
                break;
            }
            part = reader.next_part().await?;
            if part.is_empty() && reader.finished {
                break;
            }
        }
        self.complete_multipart(bucket, key, upload_id, &parts)
            .await?;
        Ok(bytes)
    }

    /// `CreateMultipartUpload`: returns the upload id.
    async fn create_multipart(
        &self,
        bucket: &str,
        key: &str,
        content_type: Option<&str>,
    ) -> Result<String, StorageError> {
        let path = self.path(bucket, key);
        let query = [("uploads", "")];
        let request = self.signed(&Method::POST, &path, &query, content_type, Bytes::new())?;
        let (_, text) = self.exchange(request).await?;
        xml::text_of(&text, "UploadId")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| StorageError::NoAnswer("the answer has no UploadId".to_owned()))
    }

    /// `UploadPart`: returns the part's `ETag`, as the completion needs it.
    async fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        number: u32,
        part: Bytes,
    ) -> Result<String, StorageError> {
        let path = self.path(bucket, key);
        let number_text = number.to_string();
        let query = [
            ("partNumber", number_text.as_str()),
            ("uploadId", upload_id),
        ];
        let request = self.signed(&Method::PUT, &path, &query, None, part)?;
        let (headers, _) = self.exchange(request).await?;
        headers
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| StorageError::NoAnswer("the answer to a part has no ETag".to_owned()))
    }

    /// `CompleteMultipartUpload` with the parts' numbers and `ETag`s. S3
    /// may answer 200 with an `<Error>` document; that is a failure too.
    async fn complete_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> Result<(), StorageError> {
        let path = self.path(bucket, key);
        let query = [("uploadId", upload_id)];
        let document = xml::complete_multipart_upload(parts);
        let request = self.signed(
            &Method::POST,
            &path,
            &query,
            Some("application/xml"),
            Bytes::from(document),
        )?;
        let (_, text) = self.exchange(request).await?;
        if text.contains("<Error>") {
            return Err(StorageError::Status(200, xml::text_of(&text, "Code")));
        }
        Ok(())
    }

    /// `AbortMultipartUpload`.
    async fn abort_multipart(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), StorageError> {
        let path = self.path(bucket, key);
        let query = [("uploadId", upload_id)];
        let request = self.signed(&Method::DELETE, &path, &query, None, Bytes::new())?;
        self.exchange(request).await.map(|_| ())
    }
}

/// A multipart upload that has begun. When it is dropped before `ended`
/// is set (the upload's future was dropped half-way: its request was cut
/// or timed out), it aborts the upload in the background, so the store
/// does not keep the parts.
struct Unfinished {
    storage: ObjectStorage,
    bucket: String,
    key: String,
    upload_id: String,
    ended: bool,
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        // no runtime: the process is stopping, and the store's lifecycle
        // rule (DEPLOYING) removes the parts later
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let storage = self.storage.clone();
        let bucket = std::mem::take(&mut self.bucket);
        let key = std::mem::take(&mut self.key);
        let upload_id = std::mem::take(&mut self.upload_id);
        runtime.spawn(async move {
            if let Err(error) = storage.abort_multipart(&bucket, &key, &upload_id).await {
                tracing::warn!(%error, "object storage: a dropped upload could not be aborted");
            }
        });
    }
}

/// Reads a small answer body as text, up to [`MAX_ANSWER_BYTES`].
async fn read_text(body: Incoming) -> Result<String, StorageError> {
    let collected = Limited::new(body, MAX_ANSWER_BYTES)
        .collect()
        .await
        .map_err(|e| StorageError::NoAnswer(format!("the answer could not be read: {e}")))?;
    Ok(String::from_utf8_lossy(&collected.to_bytes()).into_owned())
}

/// Cuts a streamed body into parts of [`PART_BYTES`].
struct PartReader {
    body: Body,
    /// Bytes read past the end of the last part.
    left_over: Bytes,
    /// Whether the body has ended.
    finished: bool,
}

impl PartReader {
    fn new(body: Body) -> Self {
        Self {
            body,
            left_over: Bytes::new(),
            finished: false,
        }
    }

    /// The next part: [`PART_BYTES`] bytes, or fewer when the body ends
    /// (then `finished` is set).
    async fn next_part(&mut self) -> Result<Bytes, StorageError> {
        let mut part = Vec::with_capacity(PART_BYTES.min(1024 * 1024));
        let mut chunk = std::mem::take(&mut self.left_over);
        loop {
            let room = PART_BYTES - part.len();
            if chunk.len() >= room {
                part.extend_from_slice(&chunk[..room]);
                // the part is full; whether the body ends here is learned
                // on the next read
                self.left_over = chunk.slice(room..);
                return Ok(Bytes::from(part));
            }
            part.extend_from_slice(&chunk);
            match self.body.frame().await {
                None => {
                    self.finished = true;
                    return Ok(Bytes::from(part));
                }
                Some(Err(error)) => return Err(StorageError::Body(error.to_string())),
                Some(Ok(frame)) => {
                    // trailers carry no data
                    chunk = frame.into_data().unwrap_or_default();
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        let list: Vec<(String, String)> = list
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| list.iter().find(|(k, _)| k == name).map(|(_, v)| v.into())
    }

    #[test]
    fn settings_need_both_keys_or_none() {
        assert!(Settings::from_vars(vars(&[])).unwrap().is_none());
        let half = [("AWS_ACCESS_KEY_ID", "a"), ("PORTITOR_PUSH_BUCKET", "b")];
        assert!(Settings::from_vars(vars(&half)).is_err());
        assert!(
            Settings::from_vars(vars(&[("PORTITOR_PUSH_BUCKET", "b")])).is_err(),
            "a push bucket needs keys"
        );
        let set = Settings::from_vars(vars(&[
            ("AWS_ACCESS_KEY_ID", "a"),
            ("AWS_SECRET_ACCESS_KEY", "s"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(set.region, "us-east-1");
        assert_eq!(set.endpoint, "https://s3.us-east-1.amazonaws.com");
        assert_eq!(set.push_bucket, None);
        let rustfs = Settings::from_vars(vars(&[
            ("AWS_ACCESS_KEY_ID", "a"),
            ("AWS_SECRET_ACCESS_KEY", "s"),
            ("AWS_ENDPOINT_URL_S3", "http://rustfs:9000/"),
            ("AWS_REGION", "eu-west-1"),
            ("PORTITOR_PUSH_BUCKET", "received"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(rustfs.endpoint, "http://rustfs:9000");
        assert_eq!(rustfs.push_bucket.as_deref(), Some("received"));
        assert!(
            !format!("{rustfs:?}").contains("\"s\""),
            "the secret is not shown"
        );
        assert!(
            Settings::from_vars(vars(&[
                ("AWS_ACCESS_KEY_ID", "a"),
                ("AWS_SECRET_ACCESS_KEY", "s"),
                ("AWS_ENDPOINT_URL_S3", "ftp://x"),
                ("PORTITOR_PUSH_BUCKET", "b"),
            ]))
            .is_err()
        );
    }

    /// A 1.0 configuration may hold `AWS_*` variables for another tool:
    /// without a push bucket they are not checked, and the connector
    /// starts without object storage, as 1.0 did.
    #[test]
    fn without_a_push_bucket_unusable_settings_are_ignored() {
        for unusable in [
            vec![("AWS_ACCESS_KEY_ID", "a")],
            vec![("AWS_SECRET_ACCESS_KEY", "s")],
            vec![
                ("AWS_ACCESS_KEY_ID", "a"),
                ("AWS_SECRET_ACCESS_KEY", "s"),
                ("AWS_ENDPOINT_URL_S3", "ftp://x"),
            ],
        ] {
            assert_eq!(
                Settings::from_vars(vars(&unusable))
                    .unwrap()
                    .map(|s| s.endpoint),
                None,
                "{unusable:?}"
            );
        }
    }

    /// `AWS_SESSION_TOKEN` is read, sent as `x-amz-security-token` and
    /// signed; it is never shown.
    #[test]
    fn a_session_token_is_sent_and_signed() {
        let settings = Settings::from_vars(vars(&[
            ("AWS_ACCESS_KEY_ID", "a"),
            ("AWS_SECRET_ACCESS_KEY", "s"),
            ("AWS_SESSION_TOKEN", "the-token"),
            ("AWS_ENDPOINT_URL_S3", "http://rustfs:9000"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(settings.session_token.as_deref(), Some("the-token"));
        assert!(!format!("{settings:?}").contains("the-token"));
        let storage = ObjectStorage::new(settings).unwrap();
        let request = storage
            .signed(&Method::GET, "/b/k", &[], None, Bytes::new())
            .unwrap();
        let headers = request.headers();
        assert_eq!(headers["x-amz-security-token"], "the-token");
        let authorization = headers[header::AUTHORIZATION].to_str().unwrap();
        assert!(
            authorization.contains(
                "SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token,"
            ),
            "{authorization}"
        );
    }

    #[test]
    fn s3_uris_name_a_bucket_and_a_key() {
        assert_eq!(
            parse_s3_uri("s3://data/dir/file.csv"),
            Some(ObjectAddress {
                bucket: "data".to_owned(),
                key: "dir/file.csv".to_owned()
            })
        );
        for bad in [
            "s3://data",
            "s3://data/",
            "s3:///key",
            "s3://Data/key",
            "s3://data/key?x",
            "https://data/key",
        ] {
            assert_eq!(parse_s3_uri(bad), None, "{bad}");
        }
    }

    #[tokio::test]
    async fn bodies_are_cut_into_parts_of_the_part_size() {
        let body = Body::from(vec![7u8; PART_BYTES * 2 + 10]);
        let mut reader = PartReader::new(body);
        let first = reader.next_part().await.unwrap();
        assert_eq!((first.len(), reader.finished), (PART_BYTES, false));
        let second = reader.next_part().await.unwrap();
        assert_eq!(second.len(), PART_BYTES);
        let third = reader.next_part().await.unwrap();
        assert_eq!((third.len(), reader.finished), (10, true));

        let mut exact = PartReader::new(Body::from(vec![1u8; PART_BYTES]));
        assert_eq!(exact.next_part().await.unwrap().len(), PART_BYTES);
        assert!(!exact.finished, "the end is seen on the next read");
        assert!(exact.next_part().await.unwrap().is_empty());
        assert!(exact.finished);
    }
}
