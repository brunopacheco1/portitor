// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! `GET /data/{flowId}`: the data of a pull transfer (ADR 0013, "The
//! data"; ADR 0027).
//!
//! - A request must carry the flow's token as `Authorization: Bearer
//!   ...`. It is served only while the flow is STARTED and the token has
//!   not expired. Unknown flow, wrong token, suspended or ended flow, a
//!   push flow (it has no token), and expired token all get the same 401
//!   answer, so a caller learns nothing about which flows exist.
//! - From 1.1: every request also checks the consumer's membership again
//!   ([`AccessGate`], refused with the same 401), and a download that runs
//!   is cut when the flow is suspended or terminated
//!   ([`crate::running_flows::RunningFlows`]) or when the membership
//!   check, run again while it streams, fails (`download.rs`).
//! - The data is streamed from the source the flow keeps (`sources.rs`).
//!   The consumer's request headers are not passed on.
//! - Each served request writes one audit line, `data transfer done`, with
//!   the bytes sent, how it ended, and the agreement and the consumer the
//!   flow keeps ([`crate::audit`]).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use portitor_dps::state::DataFlowState;
use portitor_runtime::http::bearer_token_in;
use portitor_runtime::system::now_secs;

use crate::access::{Access, AccessGate};
use crate::audit::{AuditedBody, Direction, Outcome, Parties, transfer_done};
use crate::download;
use crate::flows::hash;
use crate::plane::HttpDataPlane;

/// Path of the data endpoint; the flow id follows.
pub const DATA_PATH: &str = "/data";

/// A request for data that may be served.
pub(crate) struct Authorized {
    /// Where its data comes from.
    pub(crate) source: String,
    /// The agreement and the consumer, for the audit line.
    pub(crate) parties: Parties,
}

/// Why a request for data is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No token, wrong token, unknown flow, flow not STARTED, or token
    /// expired: HTTP 401, without saying which.
    Unauthorized,
    /// The flow store failed: HTTP 500.
    StoreFailed,
}

impl HttpDataPlane {
    /// Checks a request for flow `id` with bearer `token` at time `now`,
    /// and returns the flow's source, to read with
    /// [`HttpDataPlane::open_source`]: an upstream URL or an `s3://`
    /// object.
    ///
    /// # Errors
    ///
    /// See [`Denied`].
    pub fn source_if_authorized(&self, id: &str, token: &str, now: u64) -> Result<String, Denied> {
        let allowed = self.check_request(id, token, now)?;
        Ok(allowed.source)
    }

    /// Checks a request for flow `id` with bearer `token` at time `now`
    /// ([`HttpDataPlane::source_if_authorized`]), and returns the flow's
    /// source and who the data goes to.
    pub(crate) fn check_request(
        &self,
        id: &str,
        token: &str,
        now: u64,
    ) -> Result<Authorized, Denied> {
        let flow = self
            .flows
            .get(id)
            .map_err(|_| Denied::StoreFailed)?
            .ok_or(Denied::Unauthorized)?;
        let allowed = flow.state == DataFlowState::Started
            && flow.push_address.is_none()
            && now < flow.expires_at
            && hash(token) == flow.token_hash;
        if !allowed {
            return Err(Denied::Unauthorized);
        }
        let parties = flow.parties();
        // a flow of 1.1 whose source could not be filled in has none
        // (`Flow::source`): it is refused like any other
        let Some(source) = flow.source else {
            return Err(Denied::Unauthorized);
        };
        Ok(Authorized { source, parties })
    }
}

/// The answer to a refused request for data: 401 with a `Bearer`
/// challenge, the same for every reason.
#[must_use]
pub fn unauthorized() -> Response {
    let challenge = [(header::WWW_AUTHENTICATE, "Bearer")];
    (StatusCode::UNAUTHORIZED, challenge).into_response()
}

/// `GET /data/{flowId}`: checks the token and the consumer's
/// membership, then streams the upstream answer (status 2xx only; its
/// `Content-Type` is kept). The stream stops when the flow is suspended
/// or terminated, or when the membership check fails while it runs.
/// Once the answer has ended, was cut or failed, one audit line says so;
/// a request that is refused (401) writes none.
pub async fn serve(
    State(plane): State<Arc<HttpDataPlane>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = bearer_token_in(&headers) else {
        return unauthorized();
    };
    // watch first, check second: a suspend between the two still cuts
    // this download (`running_flows` module doc)
    let cut = plane.running.watch(&id);
    let Authorized { source, parties } = match plane.check_request(&id, token, now_secs()) {
        Ok(allowed) => allowed,
        Err(Denied::Unauthorized) => return unauthorized(),
        Err(Denied::StoreFailed) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if let Some(gate) = &plane.gate
        && !member_may_download(gate.as_ref(), &id).await
    {
        return unauthorized();
    }
    match plane.open_source(&source).await {
        Ok(data) => {
            let mut response = Response::new(data.body);
            if let Some(value) = data.content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            // the bytes are counted as they go to the consumer; the cut
            // wraps the count, so a cut download is audited as `cut`.
            // `Response::map` changes only the body; the status and the
            // headers stay.
            let response = response.map(|body| Body::new(AuditedBody::new(body, &id, parties)));
            let watchdog = plane.membership_watchdog(&id);
            download::cut_when_told(response, cut, watchdog)
        }
        Err(error) => {
            // the source is private: the log names the flow, not the URL
            tracing::warn!(flow = %id, %error, "upstream fetch failed");
            let (agreement, consumer) = (&parties.agreement_id, &parties.counter_party_id);
            transfer_done(
                &id,
                agreement,
                consumer,
                Direction::Sent,
                0,
                Outcome::Failed,
            );
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Whether the consumer of flow `id` is still a member ([`AccessGate`]).
/// When it is not, its transfer is ended too; when the check could not
/// finish, the request is refused and the transfer kept. The reason goes
/// to the log, never to the caller.
async fn member_may_download(gate: &dyn AccessGate, id: &str) -> bool {
    match gate.check(id).await {
        Access::Granted => true,
        Access::Unchecked(reason) => {
            tracing::warn!(flow = %id, %reason, "request for data refused: the consumer's membership could not be checked now");
            false
        }
        Access::Revoked(reason) => {
            gate.end_transfer(id, &reason);
            false
        }
    }
}
