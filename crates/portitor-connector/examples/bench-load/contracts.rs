// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Mode `contracts`: negotiates agreements, and starts a transfer under
//! each, through a consumer connector's management API, the way an
//! operator's script does (`scripts/demo-steps.sh`). Both connectors do
//! the real work: every DSP message with DCP, every state saved.
//!
//! Settings (environment):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `CONTRACTS_MANAGEMENT` | the consumer's management API, for example `http://127.0.0.1:18391` |
//! | `CONTRACTS_KEY_FILE` | the file with its API key |
//! | `CONTRACTS_PROVIDER_ADDRESS` | the provider's DSP address, for example `http://127.0.0.1:18380/dsp/2025-1` |
//! | `CONTRACTS_PROVIDER_ID` | the provider's DID |
//! | `CONTRACTS_DATASET` | the dataset id; its offer `<id>:offer:use` is requested |
//! | `CONTRACTS_COUNT` | how many negotiations (default 1) |
//! | `CONTRACTS_PARALLEL` | how many run at the same time (default 1) |
//! | `CONTRACTS_PER_SECOND` | instead of `CONTRACTS_PARALLEL`: start this many each second, however long each takes |
//! | `CONTRACTS_SECONDS` | start no new negotiation after this many seconds, even if fewer than `CONTRACTS_COUNT` were made (default: no such limit) |
//! | `CONTRACTS_TRANSFERS` | `0`: negotiations only (default `1`: a transfer under each agreement) |
//! | `CONTRACTS_TARGETS_FILE` | where to write each started transfer's URL and token, for the `pulls` mode (default: not written) |
//!
//! A negotiation is timed from the request to the management API until
//! the consumer shows it `FINALIZED`; a transfer until it shows
//! `STARTED`. The state is read every 2 ms, so the times are up to 2 ms
//! too long.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use portitor_runtime::http::{HttpSender, Sender as _};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::stats::{micros, print_percentiles};

/// How often a state is read again.
const POLL_EVERY: Duration = Duration::from_millis(2);

/// How long a negotiation or a transfer may take.
const GIVE_UP_AFTER: Duration = Duration::from_secs(60);

struct Settings {
    management: String,
    /// The `Authorization` header with the API key.
    key: String,
    provider_address: String,
    provider_id: String,
    dataset: String,
    transfers: bool,
    /// When to stop starting negotiations (`CONTRACTS_SECONDS`), if set.
    deadline: Option<Instant>,
    http: HttpSender,
}

/// One agreement made, and its transfer when one was started.
struct Made {
    negotiation_micros: u64,
    transfer_micros: Option<u64>,
    /// The transfer's URL and token, as a line of the targets file.
    target: Option<String>,
}

/// The management API key in `key_file`. Reading a file blocks, so it
/// runs on a thread for blocking work and not on a request thread.
async fn read_key(key_file: String) -> Result<String, String> {
    let read = tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&key_file).map_err(|e| format!("{key_file}: {e}"))
    });
    match read.await {
        Ok(key) => key,
        Err(e) => Err(format!("reading the key file: {e}")),
    }
}

pub(crate) async fn run() -> Result<(), String> {
    let key_file = crate::required("CONTRACTS_KEY_FILE")?;
    let key = read_key(key_file).await?;
    let started = Instant::now();
    let seconds = crate::number("CONTRACTS_SECONDS", 0)?;
    let settings = Arc::new(Settings {
        management: crate::required("CONTRACTS_MANAGEMENT")?,
        key: format!("Bearer {}", key.trim()),
        provider_address: crate::required("CONTRACTS_PROVIDER_ADDRESS")?,
        provider_id: crate::required("CONTRACTS_PROVIDER_ID")?,
        dataset: crate::required("CONTRACTS_DATASET")?,
        transfers: crate::number("CONTRACTS_TRANSFERS", 1)? != 0,
        deadline: (seconds > 0).then(|| started + Duration::from_secs(seconds)),
        http: HttpSender::new(true)?,
    });
    let count = crate::number("CONTRACTS_COUNT", 1)?;
    let parallel = crate::number("CONTRACTS_PARALLEL", 1)?;
    let per_second = crate::number("CONTRACTS_PER_SECOND", 0)?;
    let workers = if per_second > 0 {
        start_paced(&settings, count, per_second).await
    } else {
        start_parallel(&settings, count, parallel)
    };
    let mut results = Vec::new();
    for worker in workers {
        results.extend(worker.await.map_err(|e| e.to_string())?);
    }
    report(&results, started.elapsed())
}

/// The results of one worker.
type Worker = tokio::task::JoinHandle<Vec<Result<Made, String>>>;

/// `parallel` workers share `count` contracts: each makes one after the
/// other until none is left.
fn start_parallel(settings: &Arc<Settings>, count: u64, parallel: u64) -> Vec<Worker> {
    let left = Arc::new(AtomicU64::new(count));
    let mut workers = Vec::new();
    for _ in 0..parallel {
        workers.push(tokio::spawn(worker(
            Arc::clone(settings),
            Arc::clone(&left),
        )));
    }
    workers
}

/// Starts `per_second` contracts each second, each on a worker of its
/// own, until `count` are started.
async fn start_paced(settings: &Arc<Settings>, count: u64, per_second: u64) -> Vec<Worker> {
    let mut ticks = tokio::time::interval(Duration::from_micros(1_000_000 / per_second));
    let mut workers = Vec::new();
    for _ in 0..count {
        ticks.tick().await;
        let one = Arc::new(AtomicU64::new(1));
        workers.push(tokio::spawn(worker(Arc::clone(settings), one)));
    }
    workers
}

/// Makes contracts while any is `left` and the deadline has not passed.
async fn worker(settings: Arc<Settings>, left: Arc<AtomicU64>) -> Vec<Result<Made, String>> {
    let mut results = Vec::new();
    loop {
        let too_late = settings
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        // takes one from `left` unless it is 0 (`checked_sub` gives `None` then)
        let took_one = left
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok();
        if too_late || !took_one {
            return results;
        }
        results.push(one_contract(&settings).await);
    }
}

/// One negotiation and, if asked for, one transfer under its agreement.
async fn one_contract(settings: &Settings) -> Result<Made, String> {
    let start = Instant::now();
    let request = json!({
        "providerAddress": settings.provider_address,
        "providerId": settings.provider_id,
        "datasetId": settings.dataset,
        "offer": {
            "@id": format!("{}:offer:use", settings.dataset),
            "@type": "Offer",
            "permission": [{"action": "use"}]
        }
    });
    let negotiation = start_and_wait(settings, "negotiations", &request, "FINALIZED").await?;
    let negotiation_micros = micros(start.elapsed());
    if !settings.transfers {
        return Ok(Made {
            negotiation_micros,
            transfer_micros: None,
            target: None,
        });
    }
    let agreement = text_at(&negotiation, "/agreement/@id")?;
    let start = Instant::now();
    let request = json!({"agreementId": agreement});
    let transfer = start_and_wait(settings, "transfers", &request, "STARTED").await?;
    Ok(Made {
        negotiation_micros,
        transfer_micros: Some(micros(start.elapsed())),
        target: Some(target_line(&transfer)?),
    })
}

/// POSTs `request` to the management API's `/v1/<kind>`, then reads the
/// new process until it is in state `wanted`, and returns it.
async fn start_and_wait(
    settings: &Settings,
    kind: &str,
    request: &Value,
    wanted: &str,
) -> Result<Value, String> {
    let url = format!("{}/v1/{kind}", settings.management);
    let created = settings
        .http
        .post_authorized(&url, request, &settings.key)
        .await?;
    let pid = text_at(&created, "/consumerPid")?;
    let url = format!("{url}/{pid}");
    let deadline = Instant::now() + GIVE_UP_AFTER;
    loop {
        let view = settings.http.get_authorized(&url, &settings.key).await?;
        let state = text_at(&view, "/state")?;
        if state == wanted {
            return Ok(view);
        }
        if state == "TERMINATED" {
            return Err(format!("{kind} {pid} was TERMINATED"));
        }
        if Instant::now() >= deadline {
            return Err(format!("{kind} {pid} still {state}"));
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// The text at `pointer` (a JSON Pointer such as `/agreement/@id`).
fn text_at(value: &Value, pointer: &str) -> Result<String, String> {
    match value.pointer(pointer).and_then(Value::as_str) {
        Some(text) => Ok(text.to_owned()),
        None => Err(format!("no {pointer} in {value}")),
    }
}

/// A started transfer's `<URL> <token>`, from its data address.
fn target_line(transfer: &Value) -> Result<String, String> {
    let endpoint = text_at(transfer, "/dataAddress/endpoint")?;
    let properties = transfer
        .pointer("/dataAddress/endpointProperties")
        .and_then(Value::as_array)
        .ok_or("no endpointProperties")?;
    for property in properties {
        if property.get("name").and_then(Value::as_str) == Some("authorization") {
            let token = text_at(property, "/value")?;
            return Ok(format!("{endpoint} {token}"));
        }
    }
    Err("no authorization property in the data address".to_owned())
}

/// Prints the counts and percentiles, and writes the targets file.
fn report(results: &[Result<Made, String>], elapsed: Duration) -> Result<(), String> {
    let mut negotiations = Vec::new();
    let mut transfers = Vec::new();
    let mut targets = String::new();
    let mut first_error = None;
    for result in results {
        match result {
            Ok(made) => {
                negotiations.push(made.negotiation_micros);
                transfers.extend(made.transfer_micros);
                if let Some(target) = &made.target {
                    targets.push_str(target);
                    targets.push('\n');
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    println!("contracts={}", results.len());
    println!("errors={}", results.len() - negotiations.len());
    if let Some(error) = first_error {
        println!("first_error={error}");
    }
    println!("seconds={:.2}", elapsed.as_secs_f64());
    print_percentiles("negotiation", negotiations);
    print_percentiles("transfer_start", transfers);
    if let Ok(path) = std::env::var("CONTRACTS_TARGETS_FILE") {
        std::fs::write(&path, targets).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}
