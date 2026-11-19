// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What happens when code panics (a bug: an index out of range, a
//! failed `expect`). From 0.12 the release build unwinds instead of
//! aborting (TD-41), so a panic ends only the task it happened in. This
//! module makes sure that no such task ends unseen:
//!
//! - a request handler: [`answer_500_on_panic`] answers HTTP 500, logs an
//!   error and counts `portitor.http.request.panicked`; the server and
//!   the other requests go on;
//! - a background task that runs for as long as the process
//!   ([`spawn_supervised`]): logged, counted (`portitor.task.panicked`)
//!   and started again after [`RESTART_PAUSE`];
//! - a background task that does one piece of work ([`report_panic`]):
//!   logged and counted; the work is left as it was saved;
//! - every panic, wherever it happens: one JSON log line with the place
//!   in the code ([`log_panics`]).
//!
//! The panic's message is never logged or sent: it can quote the value
//! that caused it, which may come from a request.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use crate::api_key::problem;
use crate::telemetry::metrics;

/// How long a supervised task waits before it starts again after a
/// panic, so that a bug that panics every time does not fill the log.
pub const RESTART_PAUSE: Duration = Duration::from_secs(5);

/// The work panicked. Carries nothing: the panic's message is not kept
/// (see the module text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panicked;

/// Runs `work` and gives its result, or [`Panicked`] if it panicked.
///
/// How: an async task runs in steps (each call of `poll` runs it up to
/// its next wait). `std::panic::catch_unwind` runs one step and returns
/// an error instead of letting a panic travel on. `AssertUnwindSafe`
/// says we accept that data the step was changing may be left half
/// changed: the stores recover their locks and roll their transaction
/// back (see `docs/RUST-IDIOMS.md`, "Panics").
///
/// # Errors
/// [`Panicked`]: `work` panicked.
pub async fn catch_panic<F: Future>(work: F) -> Result<F::Output, Panicked> {
    // `Box::pin`: the task gets a fixed place in memory, which `poll` needs
    let mut work: Pin<Box<F>> = Box::pin(work);
    std::future::poll_fn(move |context| {
        let step = AssertUnwindSafe(|| work.as_mut().poll(context));
        match std::panic::catch_unwind(step) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_message) => Poll::Ready(Err(Panicked)),
        }
    })
    .await
}

/// `router` whose handlers answer HTTP 500 when they panic, instead of
/// closing the connection without an answer. Apply it after the routes
/// and the other layers, and before
/// [`crate::telemetry::http::observe_requests`], so that the request's
/// span and duration measure show the 500.
///
/// The body is the problem document every other answer made outside a
/// handler has (the 503 of [`crate::serve::with_limits`], the 413 of the
/// size limits), also on DSP routes: the layer does not know the
/// protocol, and has no process ids to put into a DSP error.
///
/// Only the handler is covered: once an answer has started, a panic
/// while its body is streamed (the data plane) closes that connection.
pub fn answer_500_on_panic(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(catch_handler_panic))
}

/// The middleware of [`answer_500_on_panic`].
async fn catch_handler_panic(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    // the template (`/dsp/2025-1/negotiations/{pid}`), never the path
    // with its ids; none when no route matched
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    if let Ok(response) = catch_panic(next.run(request)).await {
        return response;
    }
    tracing::error!(
        %method,
        route = route.as_deref(),
        "a request handler panicked; answered 500"
    );
    metrics::http_request_panicked(route.as_deref());
    problem(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

/// Starts a background task that runs for as long as the process (a
/// schedule, an outbox). `start` makes the task; it is called once now,
/// and again [`RESTART_PAUSE`] after each panic, so the task never stays
/// dead while the process lives. A task that returns is finished and is
/// not started again. `name` is a fixed text for the log and the
/// `portitor.task.name` attribute.
///
/// Aborting the returned handle stops the task for good.
pub fn spawn_supervised<S, F>(name: &'static str, start: S) -> tokio::task::JoinHandle<()>
where
    // `start` is kept for the restarts and moves to the task's thread
    S: Fn() -> F + Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        while catch_panic(start()).await.is_err() {
            tracing::error!(
                task = name,
                restart_in_seconds = RESTART_PAUSE.as_secs(),
                "a background task panicked; it starts again"
            );
            metrics::task_panicked(name);
            tokio::time::sleep(RESTART_PAUSE).await;
        }
    })
}

/// `work`, a background task that does one piece of work (sending one
/// saved message, deciding one request), with a panic in it logged and
/// counted instead of ending the task unseen. The work is not tried
/// again here: what it had saved stays, and the caller's own recovery
/// (the next round, or the resend at the next start) picks it up.
pub async fn report_panic<F>(name: &'static str, work: F)
where
    F: Future<Output = ()>,
{
    if catch_panic(work).await.is_err() {
        tracing::error!(
            task = name,
            "a background task panicked; its work is left as saved"
        );
        metrics::task_panicked(name);
    }
}

/// From now on every panic in this process is one JSON log line at error
/// level with the place in the code (`file:line:column`) and the
/// thread's name, instead of Rust's plain text on standard error. The
/// panic's message is left out (see the module text). Called once at
/// start by [`crate::telemetry::init`].
pub fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        // `file:line:column`; empty when Rust does not know the place
        let place = match info.location() {
            Some(location) => location.to_string(),
            None => String::new(),
        };
        tracing::error!(%place, thread, "panic");
    }));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt as _;

    use super::*;

    async fn panics_at_once() -> &'static str {
        panic!("secret-in-message")
    }

    async fn panics_after_a_wait() -> &'static str {
        tokio::task::yield_now().await;
        panic!("after a wait")
    }

    /// A router with a handler that panics, one that panics after a wait
    /// and one that works.
    fn app() -> Router {
        let router = Router::new()
            .route("/panics", get(panics_at_once))
            .route("/panics/{id}", get(panics_after_a_wait))
            .route("/works", get(|| async { "fine" }));
        answer_500_on_panic(router)
    }

    async fn call(app: &Router, path: &str) -> Response {
        let request = Request::get(path).body(Body::empty()).unwrap();
        app.clone().oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn a_panicking_handler_answers_500_and_the_server_goes_on() {
        let app = app();
        let response = call(&app, "/panics").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("internal error"), "{text}");
        assert!(!text.contains("secret"), "the panic's message leaked");
        assert_eq!(
            call(&app, "/panics/7").await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(call(&app, "/works").await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn catch_panic_gives_the_result_of_work_that_does_not_panic() {
        assert_eq!(catch_panic(async { 7 }).await, Ok(7));
        let panicking = async {
            tokio::task::yield_now().await;
            panic!("no")
        };
        assert_eq!(catch_panic(panicking).await, Err::<(), _>(Panicked));
    }

    /// A paused clock: the restart pause passes at once.
    #[tokio::test(start_paused = true)]
    async fn a_supervised_task_starts_again_after_a_panic() {
        let starts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&starts);
        let task = spawn_supervised("test", move || {
            let nth = counted.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                // the first two runs panic, the third finishes
                assert!(nth >= 3, "run {nth} panics");
            }
        });
        task.await.unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 3);
    }

    /// A request that is being worked on when another one panics is
    /// still answered: the panic ends only its own request.
    #[tokio::test]
    async fn a_request_in_flight_is_answered_while_another_panics() {
        let slow = || async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            "slow"
        };
        let router = Router::new()
            .route("/panics", get(panics_at_once))
            .route("/slow", get(slow));
        let app = answer_500_on_panic(router);
        let in_flight = {
            let app = app.clone();
            tokio::spawn(async move { call(&app, "/slow").await.status() })
        };
        tokio::task::yield_now().await;
        for _ in 0..3 {
            assert_eq!(
                call(&app, "/panics").await.status(),
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
        assert_eq!(in_flight.await.unwrap(), StatusCode::OK);
    }

    /// A path no route matches is not a panic: 404 as before.
    #[tokio::test]
    async fn a_path_without_a_route_is_still_404() {
        assert_eq!(
            call(&app(), "/nowhere").await.status(),
            StatusCode::NOT_FOUND
        );
    }

    /// A paused clock shows the pause: each restart comes
    /// [`RESTART_PAUSE`] after the panic, not at once.
    #[tokio::test(start_paused = true)]
    async fn a_supervised_task_waits_the_restart_pause_before_each_restart() {
        let started_at = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = Arc::clone(&started_at);
        let task = spawn_supervised("test", move || {
            let mut starts = recorded.lock().unwrap();
            starts.push(tokio::time::Instant::now());
            let nth = starts.len();
            async move {
                assert!(nth >= 3, "run {nth} panics");
            }
        });
        task.await.unwrap();
        let starts = started_at.lock().unwrap();
        assert_eq!(starts.len(), 3);
        assert_eq!(starts[1] - starts[0], RESTART_PAUSE);
        assert_eq!(starts[2] - starts[1], RESTART_PAUSE);
    }

    /// A task that returns is finished: it is not started again.
    #[tokio::test(start_paused = true)]
    async fn a_supervised_task_that_returns_is_not_started_again() {
        let starts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&starts);
        let task = spawn_supervised("test", move || {
            counted.fetch_add(1, Ordering::SeqCst);
            async {}
        });
        task.await.unwrap();
        tokio::time::sleep(RESTART_PAUSE * 3).await;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    /// A task that panics every time is started again every time, for as
    /// long as the process lives; aborting its handle stops it for good.
    #[tokio::test(start_paused = true)]
    async fn a_task_that_always_panics_is_restarted_until_it_is_aborted() {
        let starts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&starts);
        let task = spawn_supervised("test", move || {
            counted.fetch_add(1, Ordering::SeqCst);
            async { panic!("every time") }
        });
        tokio::time::sleep(RESTART_PAUSE * 10 + Duration::from_secs(1)).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            11,
            "the first start and 10 more"
        );
        assert!(!task.is_finished(), "still supervised");

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(RESTART_PAUSE * 3).await;
        assert_eq!(
            starts.load(Ordering::SeqCst),
            11,
            "no start after the abort"
        );
    }

    /// A panic in one supervised task leaves the others running.
    #[tokio::test(start_paused = true)]
    async fn a_panic_in_one_supervised_task_does_not_stop_another() {
        let rounds = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&rounds);
        let working = spawn_supervised("working", move || {
            let counted = Arc::clone(&counted);
            async move {
                loop {
                    counted.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
        let panicking = spawn_supervised("panicking", || async { panic!("every time") });
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(rounds.load(Ordering::SeqCst) >= 10, "the other task ran on");
        working.abort();
        panicking.abort();
    }

    #[tokio::test]
    async fn report_panic_ends_quietly() {
        report_panic("test", async { panic!("no") }).await;
        report_panic("test", async {}).await;
    }
}
