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
//! the body is read. These limits apply instead:
//!
//! - One upload at a time per transfer: a second request while one runs
//!   gets 409.
//! - At most [`MAX_CONCURRENT_UPLOADS`] uploads at once, for all
//!   transfers; one more gets 503, as the public port answers when it is
//!   too busy. Each upload holds up to one part
//!   ([`crate::object_storage::PART_BYTES`], 8 MiB) in memory.
//! - A body that sends nothing for [`crate::push::IDLE_TIMEOUT`] is
//!   treated as one that broke off: the upload is aborted, the answer is
//!   400 and the audit line says `cut`.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http_body_util::BodyExt as _;
use hyper::body::{Bytes, Frame, SizeHint};
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

/// How many uploads the receiving route takes at once, for all
/// transfers (module doc). Each holds up to 8 MiB in memory, so at most
/// 64 MiB in all.
pub const MAX_CONCURRENT_UPLOADS: usize = 8;

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
    uploads: Uploads,
}

/// The uploads that run now, by consumer pid.
struct Uploads {
    running: Mutex<BTreeSet<String>>,
    /// The most that may run at once.
    max: usize,
}

/// Why an upload cannot begin now.
#[derive(Debug, PartialEq, Eq)]
enum Busy {
    /// One for the same transfer runs: 409.
    SameTransfer,
    /// [`Uploads::max`] run: 503.
    Full,
}

/// Held while one upload runs; dropping it ends the upload's turn.
struct UploadTurn<'a> {
    uploads: &'a Uploads,
    consumer_pid: String,
}

impl Uploads {
    fn new(max: usize) -> Self {
        Self {
            running: Mutex::new(BTreeSet::new()),
            max,
        }
    }

    /// The turn of an upload for transfer `consumer_pid`, if it may run
    /// now.
    fn begin(&self, consumer_pid: &str) -> Result<UploadTurn<'_>, Busy> {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if running.contains(consumer_pid) {
            return Err(Busy::SameTransfer);
        }
        if running.len() >= self.max {
            return Err(Busy::Full);
        }
        running.insert(consumer_pid.to_owned());
        Ok(UploadTurn {
            uploads: self,
            consumer_pid: consumer_pid.to_owned(),
        })
    }
}

impl Drop for UploadTurn<'_> {
    fn drop(&mut self) {
        let mut running = self
            .uploads
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        running.remove(&self.consumer_pid);
    }
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
        uploads: Uploads::new(MAX_CONCURRENT_UPLOADS),
    });
    Router::new()
        .route(
            &format!("{PUSH_PATH}/{{consumerPid}}"),
            post(receive).put(receive),
        )
        .with_state(receiver)
}

/// `POST` or `PUT /push/{consumerPid}`: checks the token, then streams
/// the body into the push bucket. 204 once the object is complete; 400
/// when the body broke off or stopped coming; 409 or 503 when it cannot
/// run now (module doc); 502 when the object storage refused it or could
/// not be reached.
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
    let _turn = match receiver.uploads.begin(&consumer_pid) {
        Ok(turn) => turn,
        Err(Busy::SameTransfer) => return StatusCode::CONFLICT.into_response(),
        Err(Busy::Full) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let total = Arc::new(AtomicU64::new(0));
    let per_frame = Arc::clone(&total);
    let counted = body.map_frame(move |frame| {
        if let Some(bytes) = frame.data_ref() {
            per_frame.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        frame
    });
    let body = Body::new(StopsWhenIdle::new(
        Body::new(counted),
        crate::push::IDLE_TIMEOUT,
    ));
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

/// A body that ends with an error when nothing comes for `idle`: the
/// upload then stops as for a body that broke off.
struct StopsWhenIdle {
    inner: Body,
    idle: Duration,
    /// Finishes `idle` after the last piece of data.
    timer: Pin<Box<tokio::time::Sleep>>,
}

impl StopsWhenIdle {
    fn new(inner: Body, idle: Duration) -> Self {
        Self {
            inner,
            idle,
            timer: Box::pin(tokio::time::sleep(idle)),
        }
    }
}

impl hyper::body::Body for StopsWhenIdle {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        // every field is `Unpin`, so the pin can be undone
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_frame(context) {
            Poll::Ready(frame) => {
                // something came: the time starts again
                let next = tokio::time::Instant::now() + this.idle;
                this.timer.as_mut().reset(next);
                Poll::Ready(frame)
            }
            Poll::Pending => {
                if this.timer.as_mut().poll(context).is_ready() {
                    let error = std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("no data for {} s", this.idle.as_secs()),
                    );
                    return Poll::Ready(Some(Err(axum::Error::new(error))));
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn one_upload_per_transfer_and_a_few_in_all() {
        let uploads = Uploads::new(2);
        let first = uploads.begin("t1").unwrap();
        assert_eq!(uploads.begin("t1").err(), Some(Busy::SameTransfer));
        let _second = uploads.begin("t2").unwrap();
        assert_eq!(uploads.begin("t3").err(), Some(Busy::Full));
        drop(first);
        assert!(uploads.begin("t1").is_ok(), "its turn ended");
    }

    /// A body that gives one piece, then waits forever.
    struct OnePieceThenNothing(bool);

    impl hyper::body::Body for OnePieceThenNothing {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
            let this = self.get_mut();
            if this.0 {
                return Poll::Pending;
            }
            this.0 = true;
            Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b"some")))))
        }
    }

    #[tokio::test]
    async fn a_body_that_stops_coming_ends_with_an_error() {
        let inner = Body::new(OnePieceThenNothing(false));
        let mut body = StopsWhenIdle::new(inner, Duration::from_millis(100));
        let first = body.frame().await.unwrap().unwrap();
        assert_eq!(first.into_data().unwrap(), "some");
        let waited = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .unwrap();
        assert!(waited.unwrap().is_err());
    }
}
