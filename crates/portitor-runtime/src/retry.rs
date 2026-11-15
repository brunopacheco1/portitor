// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! How long to wait before trying a failed call again: the outbox retry
//! policy of the issuer's delivery (`portitor-issuer`, `delivery.rs`), for
//! any component that retries a call on its own (the identity's Gaia-X
//! compliance runs, ADR 0021).

/// The longest wait between two tries: one hour.
pub const MAX_RETRY_DELAY: u64 = 3_600;

/// The wait in seconds before the next try, after `failed` failed tries:
/// 1, 2, 4 ... seconds, at most [`MAX_RETRY_DELAY`].
#[must_use]
pub fn retry_delay(failed: u32) -> u64 {
    let doubled = 1_u64
        .checked_shl(failed.saturating_sub(1))
        .unwrap_or(u64::MAX);
    doubled.min(MAX_RETRY_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_up_to_an_hour() {
        assert_eq!(retry_delay(0), 1);
        assert_eq!(retry_delay(1), 1);
        assert_eq!(retry_delay(2), 2);
        assert_eq!(retry_delay(5), 16);
        assert_eq!(retry_delay(13), 3_600);
        assert_eq!(retry_delay(100), 3_600);
    }
}
