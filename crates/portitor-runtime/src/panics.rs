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
//!   and started again after a pause: [`RESTART_PAUSE`] the first time,
//!   twice as long after each further panic in a row, at most
//!   [`MAX_RESTART_PAUSE`] ([`next_pause`]);
//! - a background task that does one piece of work ([`report_panic`];
//!   [`report_sync_panic`] for work that is not async): logged and
//!   counted; the work is left as it was saved;
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

/// How long a supervised task waits before it starts again after its
/// first panic, so that a bug that panics every time does not fill the
/// log.
pub const RESTART_PAUSE: Duration = Duration::from_secs(5);

/// The longest pause between two starts of a task that panics every
/// time. A task that ran longer than this before it panicked counts as
/// healthy again: its next pause is [`RESTART_PAUSE`].
pub const MAX_RESTART_PAUSE: Duration = Duration::from_secs(300);

/// How long to wait before work that just panicked is started again.
/// `previous` is the pause before the run that panicked (`None`: it was
/// the first run, or the first after a success), `ran_for` how long
/// that run lasted.
///
/// The pause starts at [`RESTART_PAUSE`] and doubles after each panic in
/// a row (5 s, 10 s, 20 s, ... ), up to [`MAX_RESTART_PAUSE`]. So work
/// that panics every time, and that calls another service on each run
/// (the Gaia-X schedule), calls it every 5 minutes, not every 5 seconds.
/// A run that lasted longer than [`MAX_RESTART_PAUSE`] was not such a
/// loop: the pause starts at [`RESTART_PAUSE`] again.
#[must_use]
pub fn next_pause(previous: Option<Duration>, ran_for: Duration) -> Duration {
    let Some(previous) = previous else {
        return RESTART_PAUSE;
    };
    if ran_for > MAX_RESTART_PAUSE {
        return RESTART_PAUSE;
    }
    previous.saturating_mul(2).min(MAX_RESTART_PAUSE)
}

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

/// [`catch_panic`] for work that is not async: runs `work` and gives its
/// result, or [`Panicked`] if it panicked. No steps here: the closure
/// runs from start to end inside one `catch_unwind`. `AssertUnwindSafe`
/// for the same reason as in [`catch_panic`]: what `work` was changing
/// sits behind locks that recover and in transactions that roll back.
///
/// # Errors
/// [`Panicked`]: `work` panicked.
pub fn catch_sync_panic<T>(work: impl FnOnce() -> T) -> Result<T, Panicked> {
    match std::panic::catch_unwind(AssertUnwindSafe(work)) {
        Ok(output) => Ok(output),
        Err(_message) => Err(Panicked),
    }
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
/// and again after each panic, so the task never stays dead while the
/// process lives. The pause before a restart is [`next_pause`]: 5
/// seconds after the first panic, twice as long after each further one
/// in a row, at most 5 minutes. A task that returns is finished and is
/// not started again. `name` is a fixed text for the log and the
/// `portitor.task.name` attribute.
///
/// A restarted task starts from nothing: what the old one kept in
/// memory (its own retry counter, for example) is gone. The growing
/// pause is what keeps a task that panics on every run from calling
/// other services every few seconds.
///
/// Aborting the returned handle stops the task for good.
pub fn spawn_supervised<S, F>(name: &'static str, start: S) -> tokio::task::JoinHandle<()>
where
    // `start` is kept for the restarts and moves to the task's thread
    S: Fn() -> F + Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        // the pause before the current run; none before the first
        let mut pause: Option<Duration> = None;
        loop {
            let started = tokio::time::Instant::now();
            if catch_panic(start()).await.is_ok() {
                // the task returned: it is finished
                return;
            }
            let wait = next_pause(pause, started.elapsed());
            tracing::error!(
                task = name,
                restart_in_seconds = wait.as_secs(),
                "a background task panicked; it starts again"
            );
            metrics::task_panicked(name);
            tokio::time::sleep(wait).await;
            pause = Some(wait);
        }
    })
}

/// `work`, a background task that does one piece of work (deciding one
/// request, one Gaia-X run), with a panic in it logged and counted
/// instead of ending the task unseen. The work is not tried again here:
/// what it had saved stays, and the caller's own recovery (the next
/// round of its schedule or outbox) picks it up. The connector's DSP
/// outbox does not use this: it sends the message again itself
/// (`portitor-connector/src/outbox.rs`).
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

/// [`report_panic`] for work that is not async (deciding one request
/// inside a loop over many): runs `work` here and now; a panic in it is
/// logged and counted, and the caller goes on. The work is not tried
/// again here.
///
/// # Errors
/// [`Panicked`]: `work` panicked, so that the caller can log which piece
/// of work it was.
pub fn report_sync_panic(name: &'static str, work: impl FnOnce()) -> Result<(), Panicked> {
    let outcome = catch_sync_panic(work);
    if outcome.is_err() {
        tracing::error!(
            task = name,
            "a background task panicked; its work is left as saved"
        );
        metrics::task_panicked(name);
    }
    outcome
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

    /// A paused clock: the restart pauses pass at once, and the test
    /// reads how long they were.
    #[tokio::test(start_paused = true)]
    async fn a_supervised_task_starts_again_after_a_panic() {
        let starts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&starts);
        let begun = tokio::time::Instant::now();
        let task = spawn_supervised("test", move || {
            let nth = counted.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                // the first three runs panic, the fourth finishes
                assert!(nth >= 4, "run {nth} panics");
            }
        });
        task.await.unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 4);
        // pauses of 5, 10 and 20 seconds
        assert_eq!(begun.elapsed(), Duration::from_secs(35));
    }

    #[test]
    fn the_pause_doubles_up_to_five_minutes_and_starts_over_after_a_long_run() {
        let at_once = Duration::ZERO;
        assert_eq!(next_pause(None, at_once), RESTART_PAUSE);
        let mut pause = RESTART_PAUSE;
        let mut pauses = Vec::new();
        for _ in 0..8 {
            pause = next_pause(Some(pause), at_once);
            pauses.push(pause.as_secs());
        }
        assert_eq!(pauses, [10, 20, 40, 80, 160, 300, 300, 300]);
        // a run as long as the cap is still "in a row", a longer one is not
        assert_eq!(
            next_pause(Some(MAX_RESTART_PAUSE), MAX_RESTART_PAUSE),
            MAX_RESTART_PAUSE
        );
        let long_run = MAX_RESTART_PAUSE + Duration::from_secs(1);
        assert_eq!(next_pause(Some(MAX_RESTART_PAUSE), long_run), RESTART_PAUSE);
    }

    #[test]
    fn sync_work_that_panics_is_reported_and_the_caller_goes_on() {
        assert_eq!(catch_sync_panic(|| 7), Ok(7));
        assert_eq!(report_sync_panic("test", || panic!("no")), Err(Panicked));
        assert_eq!(report_sync_panic("test", || {}), Ok(()));
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
