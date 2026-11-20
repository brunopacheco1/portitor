// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Mode `pulls`: downloads, each timed from sending the request to the
//! first byte of the answer's body and to its end.
//!
//! Settings (environment):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `PULL_TARGETS_FILE` | a file with one download per line: the URL, a space, the bearer token (`-` for none: a download straight from the source). The `contracts` mode writes such a file |
//! | `PULL_PARALLEL` | downloads running at the same time (default 1) |
//! | `PULL_SECONDS` | 0 (default): every line is downloaded once. Otherwise: the lines are downloaded again and again, `PULL_PARALLEL` at a time, for this long |
//! | `PULL_EXPECT_STATUS` | the HTTP status that counts as success (default 200; 401 to time the refusal of a wrong token) |
//! | `PULL_TOKEN` | a token to send instead of the file's (a wrong one, to time the refusal) |
//!
//! It prints the number of pulls and errors, the bytes and megabytes per
//! second of all pulls together, and the percentiles of the time to the
//! first byte (for an answer without a body: to its headers) and of the
//! whole download.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use portitor_connector::http_client::{HttpsClient, https_client};
use tokio::time::Instant;

use crate::stats::{micros, print_percentiles};

/// One download to make.
struct Target {
    url: String,
    /// The bearer token; `None` for a download without one.
    token: Option<String>,
}

/// One download that got the expected status.
struct Pull {
    /// From sending the request to the first byte of the body.
    first_byte_micros: u64,
    /// From sending the request to the end of the body.
    whole_micros: u64,
    bytes: u64,
}

/// What the workers share.
struct Work {
    targets: Vec<Target>,
    client: HttpsClient,
    expect_status: u16,
    /// When to stop (`PULL_SECONDS`); `None`: after one pass over the list.
    deadline: Option<Instant>,
    /// The index of the next line to download, counted up by every worker.
    next: AtomicUsize,
}

pub(crate) async fn run() -> Result<(), String> {
    let targets = read_targets(&crate::required("PULL_TARGETS_FILE")?)?;
    let parallel = crate::count("PULL_PARALLEL", 1)?;
    let seconds = crate::number("PULL_SECONDS", 0)?;
    let expect_status = u16::try_from(crate::number("PULL_EXPECT_STATUS", 200)?)
        .map_err(|e| format!("PULL_EXPECT_STATUS: {e}"))?;
    let (client, _) = https_client(true)?;
    let started = Instant::now();
    let deadline = (seconds > 0).then(|| started + Duration::from_secs(seconds));
    let work = Arc::new(Work {
        targets,
        client,
        expect_status,
        deadline,
        next: AtomicUsize::new(0),
    });
    let mut workers = Vec::new();
    for _ in 0..parallel {
        workers.push(tokio::spawn(worker(Arc::clone(&work))));
    }
    let mut samples = Vec::new();
    for handle in workers {
        samples.extend(handle.await.map_err(|e| e.to_string())?);
    }
    report(&samples, started.elapsed());
    Ok(())
}

/// Reads the targets file (see the module text).
fn read_targets(path: &str) -> Result<Vec<Target>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let replacement = std::env::var("PULL_TOKEN").ok();
    let mut targets = Vec::new();
    for line in text.lines() {
        let Some((url, token)) = line.split_once(' ') else {
            return Err(format!("{path}: a line without a token: {line}"));
        };
        let token = match (&replacement, token) {
            (Some(replacement), _) => Some(replacement.clone()),
            (None, "-") => None,
            (None, token) => Some(token.to_owned()),
        };
        targets.push(Target {
            url: url.to_owned(),
            token,
        });
    }
    if targets.is_empty() {
        return Err(format!("{path}: no targets"));
    }
    Ok(targets)
}

/// One worker: takes the next line and downloads it, until the list is
/// done (one pass) or the deadline has passed.
async fn worker(work: Arc<Work>) -> Vec<Result<Pull, String>> {
    let mut samples = Vec::new();
    loop {
        let index = work.next.fetch_add(1, Ordering::Relaxed);
        let stop = match work.deadline {
            Some(deadline) => Instant::now() >= deadline,
            None => index >= work.targets.len(),
        };
        if stop {
            return samples;
        }
        let target = &work.targets[index % work.targets.len()];
        samples.push(one_pull(&work, target).await);
    }
}

/// Downloads `target` and reads its body to the end, piece by piece,
/// without keeping it.
async fn one_pull(work: &Work, target: &Target) -> Result<Pull, String> {
    let mut request = hyper::Request::get(&target.url);
    if let Some(token) = &target.token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = request
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let response = work
        .client
        .request(request)
        .await
        .map_err(|e| e.to_string())?;
    let headers_at = start.elapsed();
    let status = response.status().as_u16();
    if status != work.expect_status {
        return Err(format!("HTTP {status}"));
    }
    let mut body = response.into_body();
    let mut first_byte_at = None;
    let mut bytes = 0_u64;
    // `frame()` gives the next piece of the body, `None` at its end
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| format!("cut off after {bytes} bytes: {e}"))?;
        if let Some(data) = frame.data_ref() {
            first_byte_at.get_or_insert_with(|| start.elapsed());
            bytes += data.len() as u64;
        }
    }
    Ok(Pull {
        // an answer without a body (a 401): the time to its headers
        first_byte_micros: micros(first_byte_at.unwrap_or(headers_at)),
        whole_micros: micros(start.elapsed()),
        bytes,
    })
}

/// Prints the counts, the rate and the percentiles.
fn report(samples: &[Result<Pull, String>], elapsed: Duration) {
    let mut first_bytes = Vec::new();
    let mut wholes = Vec::new();
    let mut bytes = 0_u64;
    let mut first_error = None;
    for sample in samples {
        match sample {
            Ok(pull) => {
                first_bytes.push(pull.first_byte_micros);
                wholes.push(pull.whole_micros);
                bytes += pull.bytes;
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    println!("pulls={}", samples.len());
    println!("errors={}", samples.len() - wholes.len());
    if let Some(error) = first_error {
        println!("first_error={error}");
    }
    println!("bytes={bytes}");
    println!("seconds={:.2}", elapsed.as_secs_f64());
    #[allow(clippy::cast_precision_loss)] // a rate: the last digits do not matter
    let megabytes_per_second = bytes as f64 / 1_000_000.0 / elapsed.as_secs_f64();
    println!("mb_per_sec={megabytes_per_second:.1}");
    print_percentiles("first_byte", first_bytes);
    print_percentiles("whole", wholes);
}
