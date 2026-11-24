// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Push, as consumer (ADR 0027, section 2): the receiving route.
//!
//! - A push transfer request carries our data address:
//!   `{PORTITOR_PUBLIC_URL}/push/{consumerPid}`, `endpointType`
//!   `https://w3id.org/dspace-sig/profile/http-push`, and a bearer token
//!   of 32 random bytes. Only the token's SHA-256 is kept, in the
//!   `push_receipt` table, with its end of validity
//!   (`PORTITOR_DATA_TOKEN_SECONDS`). Starting the transfer again makes a
//!   new token, which replaces the old one.
//! - `POST /push/{consumerPid}` (and `PUT`) checks the token, then
//!   streams the body into the push bucket as object `{consumerPid}`
//!   ([`crate::object_storage::ObjectStorage::upload`]). A second request
//!   replaces the object. The token works only while the transfer is
//!   `REQUESTED` or `STARTED`. Every refusal gets the same 401, as on
//!   `/data`.
//! - Each request that passed the token check writes the audit line
//!   `data transfer done` ([`crate::audit`]).
//!
//! The binary serves these routes outside the public port's body size
//! and time limits (a dataset can be large); the token is checked before
//! the body is read.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http_body_util::BodyExt as _;
use portitor_dsp::process::StoreError;
use portitor_dsp::transfer::consumer::ConsumerTransfer;
use portitor_dsp::transfer::{
    DataAddress, DataAddressType, EndpointProperty, HTTP_PUSH_ENDPOINT_TYPE, TransferState,
};
use portitor_runtime::http::bearer_token_in;

use crate::Config;
use crate::audit::{self, Outcome};
use crate::consumer_transfer::TransferConsumer;
use crate::data_plane::{hash, new_token, unauthorized};
use crate::object_storage::{ObjectStorage, StorageError};
use crate::ports::{Ports, Shared};
use crate::system::now_secs;

/// The path of the receiving route, before `/{consumerPid}`.
pub const PUSH_PATH: &str = "/push";

/// What is kept of a push transfer as consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReceipt {
    /// SHA-256 of the token, hex. The token itself is not kept.
    pub token_hash: String,
    /// When the token stops working, in seconds since 1970.
    pub expires_at: u64,
    /// The object the data went into, once some arrived.
    pub object_key: Option<String>,
}

/// Where push receipts are kept: the `SQLite` file
/// (`crate::store::SqliteNegotiations`).
pub trait PushReceipts {
    /// Keeps `token_hash` for transfer `consumer_pid` until `expires_at`,
    /// in place of an older one. The object key, if any, stays.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn keep_token(
        &self,
        consumer_pid: &str,
        token_hash: &str,
        expires_at: u64,
    ) -> Result<(), StoreError>;

    /// The receipt of transfer `consumer_pid`, if it is a push.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn receipt(&self, consumer_pid: &str) -> Result<Option<PushReceipt>, StoreError>;

    /// Notes that the data of transfer `consumer_pid` is in `object_key`.
    ///
    /// # Errors
    ///
    /// The store failed.
    fn set_object_key(&self, consumer_pid: &str, object_key: &str) -> Result<(), StoreError>;
}

/// A shared [`PushReceipts`] adapter, as kept in [`crate::Ports`].
pub type AnyPushReceipts = Shared<dyn PushReceipts + Send + Sync>;

/// Makes the data addresses of our push transfers and keeps their
/// tokens' hashes.
#[derive(Clone)]
pub(crate) struct PushAddresses {
    receipts: AnyPushReceipts,
    public_url: String,
    token_seconds: u64,
}

impl PushAddresses {
    pub(crate) fn new(receipts: AnyPushReceipts, public_url: &str, token_seconds: u64) -> Self {
        Self {
            receipts,
            public_url: public_url.trim_end_matches('/').to_owned(),
            token_seconds,
        }
    }

    /// A new token, not yet kept.
    pub(crate) fn new_token() -> Result<String, String> {
        new_token().map_err(|_| "no randomness for the push token".to_owned())
    }

    /// Our data address for transfer `consumer_pid`, with `token`.
    pub(crate) fn address(&self, consumer_pid: &str, token: &str) -> DataAddress {
        DataAddress {
            kind: DataAddressType,
            endpoint_type: HTTP_PUSH_ENDPOINT_TYPE.to_owned(),
            endpoint: Some(format!("{}{PUSH_PATH}/{consumer_pid}", self.public_url)),
            endpoint_properties: vec![
                EndpointProperty::new("authorization", token),
                EndpointProperty::new("authType", "bearer"),
            ],
        }
    }

    /// Keeps the hash of `token` for transfer `consumer_pid`, valid from
    /// now for `PORTITOR_DATA_TOKEN_SECONDS`.
    pub(crate) fn keep(&self, consumer_pid: &str, token: &str) -> Result<(), StoreError> {
        let expires_at = now_secs().saturating_add(self.token_seconds);
        self.receipts
            .0
            .keep_token(consumer_pid, &hash(token), expires_at)
    }

    /// The object the data of `consumer_pid` went into, if any arrived.
    pub(crate) fn object_key(&self, consumer_pid: &str) -> Option<String> {
        self.receipts
            .0
            .receipt(consumer_pid)
            .ok()
            .flatten()
            .and_then(|r| r.object_key)
    }
}

/// The state of the receiving route.
pub(crate) struct Receiver {
    transfers: Arc<TransferConsumer>,
    receipts: AnyPushReceipts,
    storage: Arc<ObjectStorage>,
    bucket: String,
}

/// Why a push request is refused.
enum Refused {
    /// 401, whatever the reason.
    Unauthorized,
    /// The store failed: 500.
    Store,
}

impl Receiver {
    /// The transfer that `token` lets push into `consumer_pid`.
    fn admit(&self, consumer_pid: &str, token: &str) -> Result<ConsumerTransfer, Refused> {
        let receipt = self
            .receipts
            .0
            .receipt(consumer_pid)
            .map_err(|_| Refused::Store)?
            .ok_or(Refused::Unauthorized)?;
        if now_secs() >= receipt.expires_at || hash(token) != receipt.token_hash {
            return Err(Refused::Unauthorized);
        }
        let transfer = self
            .transfers
            .transfer(consumer_pid)
            .map_err(|_| Refused::Unauthorized)?;
        match transfer.state {
            TransferState::Requested | TransferState::Started => Ok(transfer),
            _ => Err(Refused::Unauthorized),
        }
    }
}

/// The consumer side of transfers, and its receiving routes: with push
/// when `ports` has object storage and a push bucket, otherwise without
/// (no routes, and push transfers are refused).
pub(crate) fn consumer_transfers(
    config: &Config,
    ports: &Ports,
    callback_address: String,
) -> (Arc<TransferConsumer>, Router) {
    let push = ports
        .object_storage
        .as_ref()
        .and_then(|s| s.push_bucket())
        .map(|_| {
            PushAddresses::new(
                ports.push_receipts.clone(),
                &config.public_url,
                config.data_token_seconds,
            )
        });
    let transfers = Arc::new(TransferConsumer::new(
        ports,
        callback_address,
        config.allow_http,
        push,
    ));
    let routes = routes(
        Arc::clone(&transfers),
        ports.push_receipts.clone(),
        ports.object_storage.clone(),
    );
    (transfers, routes)
}

/// The receiving routes, when the connector has object storage and a
/// push bucket; otherwise none.
fn routes(
    transfers: Arc<TransferConsumer>,
    receipts: AnyPushReceipts,
    storage: Option<Arc<ObjectStorage>>,
) -> Router {
    let Some((storage, bucket)) = storage.and_then(|s| {
        let bucket = s.push_bucket()?.to_owned();
        Some((s, bucket))
    }) else {
        return Router::new();
    };
    let receiver = Arc::new(Receiver {
        transfers,
        receipts,
        storage,
        bucket,
    });
    Router::new()
        .route(
            &format!("{PUSH_PATH}/{{consumerPid}}"),
            post(receive).put(receive),
        )
        .with_state(receiver)
}

/// `POST` or `PUT /push/{consumerPid}`: checks the token, then streams
/// the body into the push bucket. 204 once the object is complete; 502
/// when the object storage refused it or could not be reached.
pub(crate) async fn receive(
    State(receiver): State<Arc<Receiver>>,
    Path(consumer_pid): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Some(token) = bearer_token_in(&headers) else {
        return unauthorized();
    };
    let transfer = match receiver.admit(&consumer_pid, token) {
        Ok(transfer) => transfer,
        Err(Refused::Unauthorized) => return unauthorized(),
        Err(Refused::Store) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let total = Arc::new(AtomicU64::new(0));
    let per_frame = Arc::clone(&total);
    let body = Body::new(body.map_frame(move |frame| {
        if let Some(bytes) = frame.data_ref() {
            per_frame.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        frame
    }));
    let stored = receiver
        .storage
        .upload(&receiver.bucket, &consumer_pid, content_type, body)
        .await;
    let (outcome, response) = match stored {
        Ok(_) => match receiver
            .receipts
            .0
            .set_object_key(&consumer_pid, &consumer_pid)
        {
            Ok(()) => (Outcome::Complete, StatusCode::NO_CONTENT.into_response()),
            Err(_) => (
                Outcome::Failed,
                StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            ),
        },
        // the sender's body broke off: nothing was stored
        Err(StorageError::Body(_)) => (Outcome::Cut, StatusCode::BAD_REQUEST.into_response()),
        Err(error) => {
            // the error names a status or a cause, never a key
            tracing::warn!(flow = %consumer_pid, %error, "push received: the object storage failed");
            (Outcome::Failed, StatusCode::BAD_GATEWAY.into_response())
        }
    };
    audit::transfer_done(
        &consumer_pid,
        &transfer.agreement_id,
        &transfer.provider_id,
        total.load(Ordering::Relaxed),
        outcome,
    );
    response
}
