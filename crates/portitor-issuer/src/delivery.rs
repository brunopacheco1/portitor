// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Sending to holders: the outbox of `CredentialMessage`s (ADR 0007, ADR
//! 0018) and credential offers (DCP 6.6).
//!
//! An issued request leaves one message in the outbox, written together
//! with the decision. [`run_outbox`] sends each due message to the
//! holder's Storage API, `POST <CredentialService>/credentials` (DCP 6.5;
//! the service is found in the holder's DID document, DCP 5.2), with a
//! self-issued ID token of the issuer whose `token` claim carries back
//! the holder's access token (issuance flow, step 7).
//!
//! A send that failed for a passing reason (no answer, the holder's DID
//! or credential service not found, or the answers 401, 408, 429 and
//! 5xx) is tried again later, waiting twice as long each time, at most
//! an hour (`portitor_runtime::retry`); after
//! [`MAX_ATTEMPTS`] the delivery is given up. Any other 4xx answer (400,
//! 403, 404, ...) means the holder will not take this message: it is
//! given up at once. Either way it is logged and shown as `FAILED` by the
//! management API; the credentials stay issued. A holder answers 2xx to a
//! message it took before (same `issuerPid`), so a lost answer costs one
//! more try, not a `FAILED` delivery. Messages still in the outbox at a
//! restart are sent then.
//!
//! Several issuer processes may share one database (TD-57). A process
//! claims a delivery before it sends it: a lease in the store, held for
//! [`LEASE_SECONDS`] by its process id, which the store gives to one
//! process only (as Eclipse EDC's lease columns). The lease ends when the
//! try is recorded. A process that stops in the middle leaves its lease
//! to run out; then another process takes the delivery over. A try that
//! outlasts the lease may be sent twice, which the holder takes (see
//! above).

use std::sync::Arc;
use std::time::Duration;

use portitor_dcp::issuance::{TokenTimes, delivery_token};
use portitor_runtime::http::answered_status;
use portitor_runtime::retry::retry_delay;
use portitor_runtime::system::{new_uuid, now_secs};
use portitor_runtime::telemetry::metrics;
use serde_json::Value;

use crate::store::{Delivery, DeliveryState, Lease};
use crate::{Issuer, TOKEN_SECONDS, issue};

/// How many times a delivery is tried. With the waits of
/// [`retry_delay`], the last try is about 8 hours after the first.
pub const MAX_ATTEMPTS: u32 = 20;

/// How long the outbox sleeps when nothing is due, unless woken.
const IDLE: Duration = Duration::from_secs(60);

/// How long a replica holds a delivery it claimed, in seconds: every
/// lease's length (`portitor_runtime::leases`), longer than one try takes
/// (the holder's DID document, then the send, each within 10 seconds).
pub use portitor_runtime::leases::LEASE_SECONDS;

/// Whether a failed send should not be tried again: the holder answered
/// with a 4xx status other than 401 (its check of our token may pass
/// later), 408 (timeout) and 429 (too many requests). No answer at all,
/// and 5xx answers, are worth another try.
#[must_use]
pub fn is_permanent(error: &str) -> bool {
    // the delivery goes through the `Sender` port, whose errors are text;
    // `answered_status` reads the status back from the text that
    // `HttpError` writes (`HTTP 404 Not Found`)
    match answered_status(error) {
        Some(401 | 408 | 429) | None => false,
        Some(code) => (400..500).contains(&code),
    }
}

/// The outbox: on each round, decides the requests still `RECEIVED`
/// (left by a restart, or whose decision failed), then sends the due
/// deliveries, until the process stops. Woken at once when a request is
/// issued, otherwise at the next due delivery or after a minute. A
/// decision that panics is logged and counted, and the round goes on
/// with the next request and the deliveries (`issue::decide_waiting`).
pub async fn run_outbox(issuer: Arc<Issuer>) {
    loop {
        issue::decide_waiting(&issuer);
        send_due(&issuer).await;
        let wait = match issuer.store.next_delivery_at() {
            Ok(Some(at)) => Duration::from_secs(at.saturating_sub(now_secs())).min(IDLE),
            Ok(None) => IDLE,
            Err(error) => {
                tracing::error!(%error, "cannot read the outbox");
                IDLE
            }
        };
        tokio::select! {
            () = issuer.outbox.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// What the outbox reads the time from: seconds since 1970.
pub type Clock<'a> = &'a (dyn Fn() -> u64 + Sync);

/// Sends every delivery due now, one after the other.
pub async fn send_due(issuer: &Issuer) {
    send_due_with(issuer, &now_secs).await;
}

/// [`send_due_with`] on a clock that stands still at `now`: for tests,
/// which pass a later time instead of waiting.
pub async fn send_due_at(issuer: &Issuer, now: u64) {
    send_due_with(issuer, &move || now).await;
}

/// Sends every delivery that is due when the round begins and that no
/// other process holds, one after the other, and schedules the next try
/// of a failed one.
///
/// The time is read from `clock` again for each claim and each record,
/// not once for the round: one try may take 20 seconds, so a lease taken
/// for the third delivery with the round's first time could end at once,
/// and another process would send that delivery too.
pub async fn send_due_with(issuer: &Issuer, clock: Clock<'_>) {
    // what is due when the round begins: a recorded try ends the lease
    // and moves the delivery at least a second after that time (or out
    // of the outbox), so the loop ends
    let due = clock();
    while let Some(delivery) = claim_next(issuer, due, clock()) {
        if delivery.attempts > 0 {
            metrics::outbox_resent(metrics::Outbox::CredentialDelivery);
        }
        let outcome = send(issuer, &delivery).await;
        record(issuer, &delivery, outcome, clock());
    }
}

/// Claims the next delivery due at `due` for this process, from `now`
/// for [`LEASE_SECONDS`]. `None` when none is due and free, or the store
/// fails (logged).
fn claim_next(issuer: &Issuer, due: u64, now: u64) -> Option<Delivery> {
    let lease = Lease {
        holder: issuer.process_id.clone(),
        until: now.saturating_add(LEASE_SECONDS),
    };
    // `due` also says which leases have ended: the earlier time, so a
    // lease is never taken over early
    match issuer.store.claim_due_delivery(due, lease) {
        Ok(claimed) => claimed,
        Err(error) => {
            tracing::error!(%error, "cannot read the outbox");
            None
        }
    }
}

/// Records what became of one try at `now`, and so ends this process's
/// lease.
fn record(issuer: &Issuer, delivery: &Delivery, outcome: Result<(), String>, now: u64) {
    let pid = delivery.issuer_pid.as_str();
    let holder = issuer.process_id.as_str();
    let stored = match outcome {
        Ok(()) => {
            tracing::info!(issuer_pid = pid, holder = %delivery.holder_did, "credentials delivered");
            issuer
                .store
                .end_delivery(pid, holder, DeliveryState::Delivered)
        }
        Err(error) => {
            let failed = delivery.attempts + 1;
            if is_permanent(&error) {
                tracing::error!(issuer_pid = pid, %error, failed, "delivery refused by the holder, given up");
                metrics::outbox_given_up(metrics::Outbox::CredentialDelivery);
                issuer
                    .store
                    .end_delivery(pid, holder, DeliveryState::Failed)
            } else if failed >= MAX_ATTEMPTS {
                tracing::error!(issuer_pid = pid, %error, failed, "delivery given up");
                metrics::outbox_given_up(metrics::Outbox::CredentialDelivery);
                issuer
                    .store
                    .end_delivery(pid, holder, DeliveryState::Failed)
            } else {
                let next = now + retry_delay(failed);
                tracing::warn!(issuer_pid = pid, %error, failed, "delivery failed, will retry");
                issuer.store.retry_later(pid, holder, failed, next)
            }
        }
    };
    match stored {
        Ok(true) => {}
        // the try took longer than the lease, and another process took
        // the delivery over: it records what becomes of it
        Ok(false) => {
            tracing::warn!(
                issuer_pid = pid,
                "the lease on the delivery ended before the try did; another process holds it now"
            );
        }
        Err(error) => tracing::error!(issuer_pid = pid, %error, "cannot record the delivery"),
    }
}

/// One try: the holder's credential service, a fresh token, the POST.
async fn send(issuer: &Issuer, delivery: &Delivery) -> Result<(), String> {
    let body: Value = serde_json::from_str(&delivery.message).map_err(|e| e.to_string())?;
    post_to_holder(
        issuer,
        &delivery.holder_did,
        "credentials",
        &body,
        delivery.access_token.as_deref(),
    )
    .await
}

/// POSTs `body` to `<holder's CredentialService>/<path>`, with a fresh
/// self-issued ID token of the issuer for `holder`, carrying
/// `access_token` if given.
///
/// # Errors
/// The holder's DID cannot be resolved, has no credential service, or the
/// call fails.
pub async fn post_to_holder(
    issuer: &Issuer,
    holder: &str,
    path: &str,
    body: &Value,
    access_token: Option<&str>,
) -> Result<(), String> {
    let document = issuer
        .directory
        .did_document(holder)
        .await
        .map_err(|e| format!("cannot resolve {holder}: {e}"))?;
    let Some(service) = document.credential_service() else {
        return Err(format!("{holder} has no CredentialService"));
    };
    let url = format!("{}/{path}", service.trim_end_matches('/'));
    let times = TokenTimes {
        now: now_secs(),
        lifetime: TOKEN_SECONDS,
    };
    let token = delivery_token(issuer.signer(), holder, access_token, &new_uuid(), times)
        .map_err(|e| e.to_string())?;
    issuer
        .sender
        .post_authorized(&url, body, &format!("Bearer {token}"))
        .await
        .map(|_answer| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_try_is_about_8_hours_after_the_first() {
        let total: u64 = (1..MAX_ATTEMPTS).map(retry_delay).sum();
        assert!(total > 7 * 3_600 && total < 9 * 3_600, "{total}");
    }

    #[test]
    fn only_a_refusal_by_the_holder_is_permanent() {
        for permanent in [
            "HTTP 400 Bad Request",
            "HTTP 403 Forbidden",
            "HTTP 404 Not Found",
        ] {
            assert!(is_permanent(permanent), "{permanent}");
        }
        for passing in [
            "HTTP 401 Unauthorized",
            "HTTP 408 Request Timeout",
            "HTTP 429 Too Many Requests",
            "HTTP 500 Internal Server Error",
            "HTTP 503 Service Unavailable",
            "no answer within 30s",
            "cannot resolve did:web:holder.example: HTTP 404 Not Found",
            "did:web:holder.example has no CredentialService",
        ] {
            assert!(!is_permanent(passing), "{passing}");
        }
    }
}
