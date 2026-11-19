// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Process plumbing every Portitor binary needs: serving the public and
//! the management API (each request observed, see
//! [`crate::telemetry::http`], and a handler's panic answered with HTTP
//! 500, see [`crate::panics`]), limits for the public server, and
//! stopping on Ctrl-C or SIGTERM. Logs, traces and metrics are started by
//! [`crate::telemetry::init`].

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

use crate::api_key::problem;
use crate::panics::answer_500_on_panic;
use crate::telemetry::http::observe_requests;

/// How long the public server works on one request before it answers
/// 503. Longer than the slowest step behind it: the connector's data plane
/// waits up to 30 s for its upstream, and checking a DSP caller is cut
/// off after 20 s.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// How many requests the public server works on at once. Further requests
/// wait for a free slot, within [`REQUEST_TIMEOUT`].
pub const MAX_CONCURRENT_REQUESTS: usize = 256;

/// The limits of [`with_limits`], shared by every request.
#[derive(Debug, Clone)]
struct Limits {
    slots: Arc<Semaphore>,
    timeout: Duration,
}

/// `router` with a time limit per request and a limit on how many
/// requests are worked on at once: anyone can call the public routes, and
/// some of them make the server wait on others (DID documents, credential
/// services). A request still waiting for a slot, or still being worked on,
/// when `timeout` has passed gets HTTP 503.
///
/// Built from an axum middleware and a tokio semaphore, both already
/// dependencies, rather than `tower-http`. The time limit ends when the
/// answer starts: a long download that has started is not cut off.
pub fn with_limits(router: Router, timeout: Duration, max_concurrent: usize) -> Router {
    let limits = Limits {
        slots: Arc::new(Semaphore::new(max_concurrent)),
        timeout,
    };
    router.layer(axum::middleware::from_fn_with_state(limits, limit))
}

/// The middleware of [`with_limits`]. Dropping the handler when time is up
/// is safe: hyper drops it the same way when a client disconnects.
async fn limit(State(limits): State<Limits>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let handled = async move {
        // the semaphore is never closed, so this waits until a slot is free
        let _slot = limits.slots.acquire().await;
        next.run(request).await
    };
    if let Ok(response) = tokio::time::timeout(limits.timeout, handled).await {
        return response;
    }
    tracing::warn!(%path, timeout_secs = limits.timeout.as_secs(), "request took too long or waited too long");
    problem(StatusCode::SERVICE_UNAVAILABLE, "the request took too long")
}

/// Runs a component: starts logs and telemetry ([`crate::telemetry::init`]),
/// runs `work` until it ends (the servers stopped on SIGTERM or Ctrl-C, or
/// failed), sends the last spans and metrics, and gives the exit code: 0,
/// or 1 after logging `<name> stopped` with the error.
///
/// `work` is the component's `async fn run()`, called but not started:
/// an `async fn`'s body only runs when awaited, here after telemetry is
/// up. `name` is `'static` (a literal such as `"portitor-connector"`)
/// because telemetry keeps it as the service name.
pub async fn run_component<F>(name: &'static str, work: F) -> ExitCode
where
    F: Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let telemetry = crate::telemetry::init(name);
    let result = work.await;
    // after the servers stopped (SIGTERM, Ctrl-C) or failed: send the
    // last spans and metrics
    telemetry.shutdown().await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "{name} stopped");
            ExitCode::FAILURE
        }
    }
}

/// Resolves once the stop signal was sent.
pub async fn wait(mut stopped: tokio::sync::watch::Receiver<bool>) {
    let _ = stopped.wait_for(|s| *s).await;
}

/// Ctrl-C, or SIGTERM from `docker stop` and Kubernetes. As PID 1 in a
/// container the process gets no default SIGTERM handling, so without this
/// it would only stop when killed.
pub async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot listen for SIGTERM");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

/// Binds the management port's socket, and logs where it listens.
///
/// # Errors
/// The address is invalid or taken.
pub async fn bind_management(bind: &str) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(bind = %bind, "management port listening");
    Ok(listener)
}

/// Serves `public`, and `management` when it is on, until Ctrl-C or
/// SIGTERM. One signal stops both servers; each finishes the requests it
/// has started. Every request gets a span and a duration measure, and
/// HTTP 500 if its handler panics; no caller's trace context is taken
/// (see [`serve_with_trusted_routes`]).
///
/// # Errors
/// A server fails, or the management server's task panicked.
pub async fn serve(
    public: (TcpListener, Router),
    management: Option<(TcpListener, Router)>,
) -> std::io::Result<()> {
    serve_with_trusted_routes(public, management, &[]).await
}

/// The listener, with `TCP_NODELAY` set on every connection it accepts:
/// an answer is sent at once and does not wait for more bytes to fill a
/// packet. Without it about 5% of small data pulls took 43 ms, with it
/// 0.4 ms (docs/BENCHMARKS.md, 0.12). A connection where the option
/// cannot be set is still served.
fn without_delay(listener: TcpListener) -> impl axum::serve::Listener<Addr = std::net::SocketAddr> {
    use axum::serve::ListenerExt;
    listener.tap_io(|connection| {
        // the option only changes timing, so a failure is not an error
        let _ = connection.set_nodelay(true);
    })
}

/// Like [`serve`], and the public routes in `trusted_routes` (route
/// templates such as `/sts/token`) take the caller's W3C trace context:
/// routes called only from inside our own participant (ADR 0023, section
/// 4). `trusted_routes` is `&'static`, a list fixed in the program, as
/// [`observe_requests`] needs it.
///
/// # Errors
/// A server fails, or the management server's task panicked.
pub async fn serve_with_trusted_routes(
    public: (TcpListener, Router),
    management: Option<(TcpListener, Router)>,
    trusted_routes: &'static [&'static str],
) -> std::io::Result<()> {
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let management = management.map(|(listener, app)| {
        // the panic layer is inside the observing one, so the span and
        // the duration measure show the 500
        let app = observe_requests(answer_500_on_panic(app), &[]);
        let serving = tokio::spawn(
            axum::serve(without_delay(listener), app)
                .with_graceful_shutdown(wait(stopped.clone()))
                .into_future(),
        );
        tokio::spawn(stop_all_when_ended(serving, stop.clone()))
    });
    tokio::spawn(async move {
        shutdown().await;
        let _ = stop.send(true);
    });
    let (listener, app) = public;
    let app = observe_requests(answer_500_on_panic(app), trusted_routes);
    axum::serve(without_delay(listener), app)
        .with_graceful_shutdown(wait(stopped))
        .await?;
    if let Some(management) = management {
        // the watching task's own result, then the management server's
        management.await.map_err(std::io::Error::other)??;
    }
    Ok(())
}

/// Waits for the management server's task, then sends the stop signal,
/// and gives the server's result. On Ctrl-C or SIGTERM the signal was
/// sent already. If the management server ended on its own (it failed,
/// or its task panicked), the public server stops too and the process
/// exits with the error: a process without its health checks and
/// management API must not go on unseen.
async fn stop_all_when_ended(
    serving: tokio::task::JoinHandle<std::io::Result<()>>,
    stop: tokio::sync::watch::Sender<bool>,
) -> std::io::Result<()> {
    let ended = serving.await;
    let _ = stop.send(true);
    // `ended` is the task's result (an error if it panicked) around the
    // server's own; the first `?` turns a panic into an I/O error
    ended.map_err(std::io::Error::other)?
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt as _;

    /// A router whose `/slow` answers after `delay`, and `/fast` at once.
    fn app(delay: Duration, timeout: Duration, max_concurrent: usize) -> Router {
        let router = Router::new()
            .route(
                "/slow",
                get(move || async move {
                    tokio::time::sleep(delay).await;
                    "slow"
                }),
            )
            .route("/fast", get(|| async { "fast" }));
        with_limits(router, timeout, max_concurrent)
    }

    async fn status(app: &Router, path: &str) -> StatusCode {
        let request = Request::get(path).body(Body::empty()).unwrap();
        app.clone().oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn a_slow_request_gets_503() {
        let app = app(Duration::from_secs(5), Duration::from_millis(50), 4);
        assert_eq!(status(&app, "/slow").await, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(&app, "/fast").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn no_more_than_the_limit_are_worked_on_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let handler = {
            let (running, most) = (Arc::clone(&running), Arc::clone(&most));
            move || async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                "done"
            }
        };
        let app = with_limits(
            Router::new().route("/work", get(handler)),
            Duration::from_secs(5),
            2,
        );
        let mut calls = Vec::new();
        for _ in 0..6 {
            let app = app.clone();
            calls.push(tokio::spawn(async move { status(&app, "/work").await }));
        }
        for call in calls {
            assert_eq!(call.await.unwrap(), StatusCode::OK, "the others waited");
        }
        assert_eq!(most.load(Ordering::SeqCst), 2);
    }
}
