// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The object storage client against a fake S3-compatible store
//! (`common/fake_s3.rs`): signed requests, a streamed `GET`, and uploads
//! in one request or in parts (ADR 0027, sections 3 and 4).
#![allow(clippy::unwrap_used, missing_docs)]

use axum::body::Body;
use http_body_util::BodyExt as _;
use portitor_connector::object_storage::{ObjectStorage, PART_BYTES, Settings, StorageError};

mod common;
use common::fake_s3;

fn storage(endpoint: &str) -> ObjectStorage {
    ObjectStorage::new(Settings {
        endpoint: endpoint.to_owned(),
        region: "us-east-1".to_owned(),
        access_key_id: fake_s3::KEY_ID.to_owned(),
        secret_access_key: fake_s3::SECRET.to_owned(),
        push_bucket: Some("received".to_owned()),
    })
    .unwrap()
}

#[tokio::test]
async fn an_object_is_streamed_with_its_content_type() {
    let (endpoint, fake) = fake_s3::start().await;
    fake.put("data", "dir/a b.csv", Some("text/csv"), b"x,y\n1,2\n");
    let store = storage(&endpoint);
    let object = store.get("data", "dir/a b.csv").await.unwrap();
    assert_eq!(object.content_type.unwrap(), "text/csv");
    let bytes = object.body.collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"x,y\n1,2\n");
    assert_eq!(
        store.get("data", "missing").await.unwrap_err(),
        StorageError::Status(404, Some("NoSuchKey".to_owned()))
    );
}

#[tokio::test]
async fn a_small_body_goes_up_in_one_request() {
    let (endpoint, fake) = fake_s3::start().await;
    let store = storage(&endpoint);
    let bytes = store
        .upload("received", "t1", Some("text/plain"), Body::from("hello"))
        .await
        .unwrap();
    assert_eq!(bytes, 5);
    let stored = fake.object("received", "t1").unwrap();
    assert_eq!(stored.bytes, b"hello");
    assert_eq!(stored.content_type.as_deref(), Some("text/plain"));
    assert_eq!(fake.calls(), vec!["PUT"]);
    // an empty body is an empty object
    store
        .upload("received", "t2", None, Body::empty())
        .await
        .unwrap();
    assert!(fake.object("received", "t2").unwrap().bytes.is_empty());
}

#[tokio::test]
async fn a_large_body_goes_up_in_parts() {
    let (endpoint, fake) = fake_s3::start().await;
    let store = storage(&endpoint);
    let data: Vec<u8> = (0..PART_BYTES * 2 + 1234)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let bytes = store
        .upload(
            "received",
            "big",
            Some("application/octet-stream"),
            Body::from(data.clone()),
        )
        .await
        .unwrap();
    assert_eq!(bytes, data.len() as u64);
    let stored = fake.object("received", "big").unwrap();
    assert!(stored.bytes == data, "the parts are put together in order");
    assert_eq!(
        stored.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(
        fake.calls(),
        vec![
            "POST uploads",
            "PUT partNumber&uploadId",
            "PUT partNumber&uploadId",
            "PUT partNumber&uploadId",
            "POST uploadId",
        ]
    );
}

#[tokio::test]
async fn a_failed_part_aborts_the_upload() {
    let (endpoint, fake) = fake_s3::start().await;
    *fake.fail_part.lock().unwrap() = Some(2);
    let store = storage(&endpoint);
    let error = store
        .upload(
            "received",
            "big",
            None,
            Body::from(vec![1u8; PART_BYTES * 2]),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        StorageError::Status(500, Some("InternalError".to_owned()))
    );
    assert!(fake.object("received", "big").is_none());
    assert_eq!(fake.open_uploads(), 0, "aborted");
    assert_eq!(fake.calls().last().unwrap(), "DELETE uploadId");
}

#[tokio::test]
async fn a_wrong_key_id_is_refused_by_the_store() {
    let (endpoint, _) = fake_s3::start().await;
    let store = ObjectStorage::new(Settings {
        endpoint,
        region: "us-east-1".to_owned(),
        access_key_id: "someone-else".to_owned(),
        secret_access_key: "x".to_owned(),
        push_bucket: None,
    })
    .unwrap();
    let error = store.get("data", "k").await.unwrap_err();
    assert_eq!(
        error,
        StorageError::Status(403, Some("SignatureDoesNotMatch".to_owned()))
    );
    assert!(!error.to_string().contains("someone-else"));
}
