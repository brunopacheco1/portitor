// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! `xsd:date` and `xsd:dateTime` texts (`portitor_time`), as ODRL
//! policies and verifiable credentials from other parties write them.
//! Parsing must never panic; a time covers one nanosecond and a date one
//! day; and a time read as whole seconds, written back and read again,
//! gives the same seconds.
#![no_main]

use libfuzzer_sys::fuzz_target;
use portitor_time::{NANOS_PER_SEC, parse, timestamp_seconds, xsd_date_time};

/// A day in nanoseconds.
const DAY: i128 = 86_400 * NANOS_PER_SEC;

/// 10000-01-01T00:00:00Z in seconds since 1970. The parser reads four
/// digit years only, and a time zone can move 9999-12-31 past it.
const YEAR_10000: u64 = 253_402_300_800;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(span) = parse(text) {
        let length = span.end - span.start;
        assert!(length == 1 || length == DAY, "{text:?} covers {length} ns");
    }
    let Some(seconds) = timestamp_seconds(text) else {
        return;
    };
    // xsd_date_time writes seconds since 1970, so only times after it,
    // and before the year 10000
    let Ok(unsigned) = u64::try_from(seconds) else {
        return;
    };
    if unsigned >= YEAR_10000 {
        return;
    }
    let written = xsd_date_time(unsigned);
    assert_eq!(
        timestamp_seconds(&written),
        Some(seconds),
        "{text:?} as {written}"
    );
});
