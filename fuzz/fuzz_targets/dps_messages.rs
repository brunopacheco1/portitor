// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Data Plane Signaling messages (`portitor_dps`, v1.0-RC4) from any
//! bytes, and any sequence of state changes. In 0.x the data plane runs
//! inside the connector and the signals are function calls, but the
//! messages keep the shape the spec gives them on HTTP, for a data plane
//! in its own process later. Parsing must never panic, what parses must
//! survive a round trip unchanged, and a final state never changes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use portitor_dps::message::{
    AddressMessage, DataFlowBaseMessage, DataFlowStartMessage, DataFlowStatusMessage,
    DataFlowStatusResponseMessage, ReasonMessage,
};
use portitor_dps::state::DataFlowState;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn round_trip<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(data: &[u8]) {
    if let Ok(msg) = serde_json::from_slice::<T>(data) {
        let again: T =
            serde_json::from_slice(&serde_json::to_vec(&msg).expect("serialize")).expect("reparse");
        assert_eq!(msg, again);
    }
}

const STATES: [DataFlowState; 8] = [
    DataFlowState::Initialized,
    DataFlowState::Preparing,
    DataFlowState::Prepared,
    DataFlowState::Starting,
    DataFlowState::Started,
    DataFlowState::Suspended,
    DataFlowState::Completed,
    DataFlowState::Terminated,
];

fuzz_target!(|data: &[u8]| {
    // prepare, start, started and resume, suspend and terminate, and the
    // data plane's answers
    round_trip::<DataFlowBaseMessage>(data);
    round_trip::<DataFlowStartMessage>(data);
    round_trip::<AddressMessage>(data);
    round_trip::<ReasonMessage>(data);
    round_trip::<DataFlowStatusMessage>(data);
    round_trip::<DataFlowStatusResponseMessage>(data);

    // each byte is one move: the low three bits pick the state to go to
    let mut state = DataFlowState::Initialized;
    for byte in data {
        let to = STATES[usize::from(byte & 7)];
        match state.go_to(to) {
            Ok(next) => {
                assert!(!state.is_final(), "{state:?} is final but went to {to:?}");
                assert_eq!(next, to);
                state = next;
            }
            Err(_) => assert!(!state.may_go_to(to)),
        }
    }
});
