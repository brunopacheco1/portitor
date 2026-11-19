// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Percentiles of measured times, printed as `key=value` lines.

use std::time::Duration;

/// `duration` in microseconds.
pub(crate) fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Prints `<name>_p50_ms`, `<name>_p99_ms` and `<name>_max_ms` of
/// `samples` (microseconds). Prints nothing for no samples.
pub(crate) fn print_percentiles(name: &str, mut samples: Vec<u64>) {
    if samples.is_empty() {
        return;
    }
    samples.sort_unstable();
    println!("{name}_p50_ms={}", millis(percentile(&samples, 500)));
    println!("{name}_p99_ms={}", millis(percentile(&samples, 990)));
    println!("{name}_max_ms={}", millis(samples[samples.len() - 1]));
}

/// The value below which `per_mille` thousandths of `sorted` lie.
fn percentile(sorted: &[u64], per_mille: usize) -> u64 {
    let index = (sorted.len() * per_mille / 1000).min(sorted.len() - 1);
    sorted[index]
}

/// Microseconds as milliseconds with two decimals.
fn millis(micros: u64) -> String {
    format!("{}.{:02}", micros / 1000, micros % 1000 / 10)
}
