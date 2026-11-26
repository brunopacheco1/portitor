// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the connector takes from the operating system: random ids and the
//! current time (shared helpers in `portitor_runtime::system`).

use std::sync::OnceLock;

use portitor_dsp::negotiation::provider::Environment;
pub use portitor_runtime::system::{hex, new_uuid, now_secs, xsd_date_time};

/// Adapter: identifiers from the OS random generator, time from the clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

/// This process's id: a random UUID, made the first time it is asked for
/// (at start, [`process_id`]) and the same until the process stops. It
/// names who holds a lease on a provider's transfer (TD-48). Random, not
/// the host name: two containers on one host, or a container started
/// again, are other processes.
static PROCESS_ID: OnceLock<String> = OnceLock::new();

/// This process's id ([`PROCESS_ID`]).
pub fn process_id() -> &'static str {
    PROCESS_ID.get_or_init(new_uuid)
}

impl Environment for SystemEnvironment {
    fn new_id(&self) -> String {
        new_uuid()
    }

    fn now(&self) -> String {
        xsd_date_time(now_secs())
    }

    fn now_seconds(&self) -> u64 {
        now_secs()
    }

    fn process_id(&self) -> &str {
        process_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_gives_utc_times_and_uuids() {
        assert!(SystemEnvironment.now().ends_with('Z'));
        assert!(SystemEnvironment.new_id().starts_with("urn:uuid:"));
        assert!(SystemEnvironment.now_seconds() > 1_700_000_000);
    }

    /// One id per process, random (TD-48).
    #[test]
    fn the_process_id_stays_the_same() {
        assert!(process_id().starts_with("urn:uuid:"));
        assert_eq!(SystemEnvironment.process_id(), process_id());
    }
}
