// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! A fake S3-compatible store for the tests, on a real local port: the
//! connector's object storage client talks HTTP to it. Path-style
//! addresses only (`/{bucket}/{key}`), objects in memory, and only the
//! calls the connector makes: `GetObject`, `PutObject`,
//! `CreateMultipartUpload`, `UploadPart`, `CompleteMultipartUpload` and
//! `AbortMultipartUpload`.
//!
//! It checks what it can without the secret key: an `Authorization`
//! header of AWS Signature Version 4 for the test key id, and a body
//! whose SHA-256 is the signed `x-amz-content-sha256`. The signature
//! itself is checked by the unit tests against the AWS test vectors and
//! by the scenario against `RustFS`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The key id the fake store takes.
pub const KEY_ID: &str = "test-only-key-id";
/// The secret key the tests configure (the fake store cannot check it).
pub const SECRET: &str = "test-only-secret-key";
/// The push bucket of [`client`].
pub const PUSH_BUCKET: &str = "received";

/// The connector's object storage client for the store at `endpoint`,
/// with the test keys and [`PUSH_BUCKET`].
pub fn client(endpoint: &str) -> portitor_connector::object_storage::ObjectStorage {
    portitor_connector::object_storage::ObjectStorage::new(
        portitor_connector::object_storage::Settings {
            endpoint: endpoint.to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: KEY_ID.to_owned(),
            secret_access_key: SECRET.to_owned(),
            session_token: None,
            push_bucket: Some(PUSH_BUCKET.to_owned()),
        },
    )
    .unwrap()
}

/// One stored object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    /// Its `Content-Type`, if one was given.
    pub content_type: Option<String>,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// A multipart upload under way.
struct Upload {
    bucket: String,
    key: String,
    content_type: Option<String>,
    parts: BTreeMap<u32, Vec<u8>>,
}

/// What the fake store holds and saw.
#[derive(Default)]
pub struct FakeS3 {
    objects: Mutex<HashMap<(String, String), StoredObject>>,
    uploads: Mutex<HashMap<String, Upload>>,
    /// Each call: method and query names, for example `POST uploads`.
    calls: Mutex<Vec<String>>,
    /// Refuse `UploadPart` with this number (HTTP 500), to test the abort.
    pub fail_part: Mutex<Option<u32>>,
}

impl FakeS3 {
    /// The object `key` of `bucket`, if stored.
    pub fn object(&self, bucket: &str, key: &str) -> Option<StoredObject> {
        let objects = self.objects.lock().unwrap();
        objects.get(&(bucket.to_owned(), key.to_owned())).cloned()
    }

    /// Stores an object directly (a provider's source).
    pub fn put(&self, bucket: &str, key: &str, content_type: Option<&str>, bytes: &[u8]) {
        let stored = StoredObject {
            content_type: content_type.map(str::to_owned),
            bytes: bytes.to_vec(),
        };
        let mut objects = self.objects.lock().unwrap();
        objects.insert((bucket.to_owned(), key.to_owned()), stored);
    }

    /// The calls so far.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// Multipart uploads neither completed nor aborted.
    pub fn open_uploads(&self) -> usize {
        self.uploads.lock().unwrap().len()
    }
}

/// Starts a fake store on a free local port; returns its base URL
/// (`http://127.0.0.1:<port>`) and its state.
pub async fn start() -> (String, Arc<FakeS3>) {
    let fake = Arc::new(FakeS3::default());
    let app = Router::new().fallback(handle).with_state(Arc::clone(&fake));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), fake)
}

fn error(status: StatusCode, code: &str) -> Response {
    let body = format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code></Error>");
    (status, body).into_response()
}

/// The checks a real store makes with the signature, as far as they can
/// be made without the secret.
fn signed_right(headers: &HeaderMap, body: &[u8]) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let authorization = text("authorization").unwrap_or_default();
    let expected_start = format!("AWS4-HMAC-SHA256 Credential={KEY_ID}/");
    let digest = ring::digest::digest(&ring::digest::SHA256, body);
    let hash = portitor_runtime::system::hex(digest.as_ref());
    authorization.starts_with(&expected_start)
        && authorization.contains("host;x-amz-content-sha256;x-amz-date, Signature=")
        && authorization.contains("Signature=")
        && text("x-amz-date").is_some_and(|d| d.len() == 16 && d.ends_with('Z'))
        && text("x-amz-content-sha256") == Some(hash.as_str())
}

async fn handle(State(fake): State<Arc<FakeS3>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, 64 * 1024 * 1024).await else {
        return error(StatusCode::BAD_REQUEST, "IncompleteBody");
    };
    if !signed_right(&parts.headers, &body) {
        return error(StatusCode::FORBIDDEN, "SignatureDoesNotMatch");
    }
    let path = parts.uri.path().trim_start_matches('/');
    let Some((bucket, key)) = path.split_once('/') else {
        return error(StatusCode::BAD_REQUEST, "InvalidRequest");
    };
    let key = percent_decode(key);
    let query: HashMap<String, String> = parts
        .uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) => (name.to_owned(), percent_decode(value)),
            None => (pair.to_owned(), String::new()),
        })
        .collect();
    let mut names: Vec<&str> = query.keys().map(String::as_str).collect();
    names.sort_unstable();
    fake.calls.lock().unwrap().push(
        format!("{} {}", parts.method, names.join("&"))
            .trim()
            .to_owned(),
    );
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let call = Call {
        bucket: bucket.to_owned(),
        key,
        query,
        content_type,
        body: body.to_vec(),
    };
    answer(&fake, &parts.method, call)
}

/// One request, read.
struct Call {
    bucket: String,
    key: String,
    query: HashMap<String, String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn answer(fake: &FakeS3, method: &Method, call: Call) -> Response {
    let upload_id = call.query.get("uploadId").cloned();
    match (method, upload_id) {
        (&Method::GET, None) => get_object(fake, &call),
        (&Method::PUT, None) => {
            fake.put(
                &call.bucket,
                &call.key,
                call.content_type.as_deref(),
                &call.body,
            );
            StatusCode::OK.into_response()
        }
        (&Method::POST, None) if call.query.contains_key("uploads") => create_upload(fake, call),
        (&Method::PUT, Some(id)) => upload_part(fake, &id, &call),
        (&Method::POST, Some(id)) => complete_upload(fake, &id, &call),
        (&Method::DELETE, Some(id)) => {
            fake.uploads.lock().unwrap().remove(&id);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    }
}

fn get_object(fake: &FakeS3, call: &Call) -> Response {
    let Some(object) = fake.object(&call.bucket, &call.key) else {
        return error(StatusCode::NOT_FOUND, "NoSuchKey");
    };
    let mut response = Response::new(Body::from(object.bytes));
    if let Some(content_type) = object.content_type {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    }
    response
}

fn create_upload(fake: &FakeS3, call: Call) -> Response {
    let id = format!("upload-{}", fake.calls().len());
    let upload = Upload {
        bucket: call.bucket,
        key: call.key,
        content_type: call.content_type,
        parts: BTreeMap::new(),
    };
    fake.uploads.lock().unwrap().insert(id.clone(), upload);
    let answer = format!(
        "<?xml version=\"1.0\"?><InitiateMultipartUploadResult><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"
    );
    (StatusCode::OK, answer).into_response()
}

fn upload_part(fake: &FakeS3, id: &str, call: &Call) -> Response {
    let number: u32 = call
        .query
        .get("partNumber")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    if *fake.fail_part.lock().unwrap() == Some(number) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError");
    }
    let mut uploads = fake.uploads.lock().unwrap();
    let Some(upload) = uploads.get_mut(id) else {
        return error(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    upload.parts.insert(number, call.body.clone());
    let mut response = StatusCode::OK.into_response();
    let etag = format!("\"etag-{number}\"");
    response
        .headers_mut()
        .insert(header::ETAG, etag.parse().unwrap());
    response
}

fn complete_upload(fake: &FakeS3, id: &str, call: &Call) -> Response {
    let Some(upload) = fake.uploads.lock().unwrap().remove(id) else {
        return error(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    let document = String::from_utf8_lossy(&call.body).into_owned();
    for number in upload.parts.keys() {
        let expected =
            format!("<PartNumber>{number}</PartNumber><ETag>&quot;etag-{number}&quot;</ETag>");
        if !document.contains(&expected) {
            return error(StatusCode::BAD_REQUEST, "InvalidPart");
        }
    }
    let bytes: Vec<u8> = upload.parts.values().flatten().copied().collect();
    fake.put(
        &upload.bucket,
        &upload.key,
        upload.content_type.as_deref(),
        &bytes,
    );
    (StatusCode::OK, "<CompleteMultipartUploadResult/>").into_response()
}

/// `%XY` back to bytes (the keys and ids the tests use are UTF-8).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
