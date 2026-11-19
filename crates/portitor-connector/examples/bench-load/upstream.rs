// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Mode `upstream`: a data source for the provider's datasets
//! (`portitor:source`) that makes its bytes up.
//!
//! `GET /bytes/{count}` answers `count` bytes, in pieces of 64 KiB.
//! `GET /bytes/{count}?pause_ms=100` waits 100 ms after each piece: a
//! slow source, for downloads that must last a known time.
//!
//! Setting: `UPSTREAM_BIND`, the address to listen on.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::serve::ListenerExt as _;
use hyper::body::{Bytes, Frame};
use tokio::sync::mpsc;

/// The size of one piece of the answer.
const PIECE_BYTES: usize = 64 * 1024;

/// What every piece is made of.
static PIECE: [u8; PIECE_BYTES] = [b'p'; PIECE_BYTES];

/// The query of `GET /bytes/{count}`.
#[derive(serde::Deserialize)]
struct Pace {
    /// Milliseconds to wait after each piece (default: none).
    pause_ms: Option<u64>,
}

pub(crate) async fn run() -> Result<(), String> {
    let bind = crate::required("UPSTREAM_BIND")?;
    let app = Router::new().route("/bytes/{count}", get(bytes));
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| format!("{bind}: {e}"))?;
    // no Nagle wait: headers and body are two writes, and without this a
    // small answer waits about 40 ms for the reader's acknowledgement,
    // which a real file server does not (scripts/participants.sh, `serve`)
    let listener = listener.tap_io(|connection| {
        let _ = connection.set_nodelay(true);
    });
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

/// `GET /bytes/{count}`: a task makes the pieces and hands them, through
/// a channel, to the answer's body. The channel holds 4 pieces at most,
/// so the task waits when the reader is slower: no memory is used up.
async fn bytes(Path(count): Path<u64>, Query(pace): Query<Pace>) -> Response {
    let (pieces, body) = mpsc::channel(4);
    let pause = Duration::from_millis(pace.pause_ms.unwrap_or(0));
    tokio::spawn(make_pieces(count, pause, pieces));
    let mut response = Response::new(Body::new(Pieces(body)));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_LENGTH, count.into());
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    response
}

/// Sends `left` bytes as pieces, waiting `pause` after each one.
async fn make_pieces(mut left: u64, pause: Duration, pieces: mpsc::Sender<Bytes>) {
    while left > 0 {
        // the last piece may be shorter
        let size = usize::try_from(left).map_or(PIECE_BYTES, |left| left.min(PIECE_BYTES));
        // `from_static`: the piece is not copied, every answer shares it
        if pieces
            .send(Bytes::from_static(&PIECE[..size]))
            .await
            .is_err()
        {
            // the reader went away
            return;
        }
        left -= size as u64;
        if !pause.is_zero() && left > 0 {
            tokio::time::sleep(pause).await;
        }
    }
}

/// An answer's body that takes its pieces from a channel. `poll_frame` is
/// how hyper asks a body for its next piece: "ready with a piece", "ready,
/// no more" (the channel is closed and empty), or "not yet".
struct Pieces(mpsc::Receiver<Bytes>);

impl hyper::body::Body for Pieces {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.0.poll_recv(context) {
            Poll::Ready(Some(piece)) => Poll::Ready(Some(Ok(Frame::data(piece)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
