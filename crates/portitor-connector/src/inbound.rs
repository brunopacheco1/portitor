// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading a DSP message that another participant sent us, the same for
//! negotiations and transfers, on both sides: the body must be JSON, pass
//! the dataspace's message rules (the bundle's guard, ADR 0005), and be
//! the message type the route takes. Each refusal is counted
//! (`portitor.dsp.request.refused`, ADR 0023).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use portitor_dsp::guard::{Direction, MessageGuard};
use portitor_runtime::telemetry::metrics::{self, Refusal};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A DSP request's body as axum gives it: JSON, or why it is not.
pub(crate) type Body = Result<Json<Value>, JsonRejection>;

/// Checks a message's shape with `guard`, then reads it as a `T`. A
/// refusal is built by `refuse` from the other party's process id (the
/// message's `other_pid_key` field, `""` when there is none) and the
/// reason; `E` is the DSP error type of the process (negotiation or
/// transfer).
pub(crate) fn read_message<T: DeserializeOwned, E>(
    guard: &(dyn MessageGuard + Send + Sync),
    body: Body,
    message_type: &str,
    other_pid_key: &str,
    refuse: impl Fn(&str, &str) -> E,
) -> Result<T, Box<E>> {
    // counts the refusal, then builds it
    let refused = |kind: Refusal, other_pid: &str, reason: &str| {
        metrics::dsp_request_refused(kind);
        Box::new(refuse(other_pid, reason))
    };
    let value = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            return Err(refused(Refusal::InvalidMessage, "", &rejection.body_text()));
        }
    };
    let other_pid = value
        .get(other_pid_key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if let Err(reason) = guard.check(Direction::Inbound, message_type, &value) {
        return Err(refused(
            Refusal::MessageRules,
            &other_pid,
            &reason.to_string(),
        ));
    }
    match serde_json::from_value(value) {
        Ok(message) => Ok(message),
        Err(error) => Err(refused(
            Refusal::InvalidMessage,
            &other_pid,
            &error.to_string(),
        )),
    }
}
