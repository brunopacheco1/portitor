// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! `bench-load`: the load tool of `scripts/bench.sh`, which measures the
//! numbers in `docs/BENCHMARKS.md`. `dsp-load`, next to it, measures the
//! cost of DCP per DSP request; this tool measures the data plane, the
//! stored history and the catalog. One binary, four modes, each in its
//! own file; the first argument picks the mode, the settings come from
//! the environment, and the results are printed as `key=value` lines.
//!
//! | Mode | What it does | File |
//! |---|---|---|
//! | `upstream` | a data source that makes its bytes up, so a 10 GB download needs no 10 GB file | `upstream.rs` |
//! | `contracts` | negotiations and transfers through a consumer connector's management API, timed | `contracts.rs` |
//! | `pulls` | downloads from a provider's data plane (or straight from the source), timed | `pulls.rs` |
//! | `catalog` | catalog requests with DCP, timed, with the answer's size | `catalog.rs` |
//!
//! It is a test tool: it is not part of the connector and is not in the
//! release image.

mod catalog;
mod contracts;
mod pulls;
mod stats;
mod upstream;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let done = match mode.as_str() {
        "upstream" => upstream::run().await,
        "contracts" => contracts::run().await,
        "pulls" => pulls::run().await,
        "catalog" => catalog::run().await,
        _ => Err("usage: bench-load upstream|contracts|pulls|catalog".to_owned()),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bench-load {mode}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The environment variable `name`, which must be set.
fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

/// The environment variable `name` as a number, or `default` when it is
/// not set.
fn number(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(text) => text.parse().map_err(|_| format!("{name}: not a number")),
        Err(_) => Ok(default),
    }
}

/// A count from the environment as a `usize` (a number of tasks or of
/// list entries).
fn count(name: &str, default: u64) -> Result<usize, String> {
    usize::try_from(number(name, default)?).map_err(|e| format!("{name}: {e}"))
}
