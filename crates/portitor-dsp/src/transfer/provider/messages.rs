// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The messages the provider sends on a transfer, ready for the outbox
//! (DSP 2025-1, "Transfer Process HTTPS Binding").

use super::{Outgoing, Transfer};
use crate::Context;
use crate::transfer::{
    DataAddress, Role, TransferCompletionMessage, TransferCompletionMessageType,
    TransferStartMessage, TransferStartMessageType, TransferSuspensionMessage,
    TransferSuspensionMessageType, TransferTerminationMessage, TransferTerminationMessageType,
    Transition, next_state,
};

/// The start message: for a pull transfer it carries where to fetch the
/// data; for a push transfer it carries no address (DSP 2025-1, "Transfer
/// Start Message": the `dataAddress` "MUST be provided if the current
/// transfer is a pull transfer").
pub(super) fn start_message(transfer: &Transfer, address: Option<DataAddress>) -> Outgoing {
    let msg = TransferStartMessage {
        context: Context::default(),
        kind: TransferStartMessageType,
        provider_pid: transfer.provider_pid.clone(),
        consumer_pid: transfer.consumer_pid.clone(),
        data_address: address,
    };
    outgoing(transfer, "start", &msg, Transition::Start)
}

pub(super) fn suspension(transfer: &Transfer) -> Outgoing {
    let msg = TransferSuspensionMessage {
        context: Context::default(),
        kind: TransferSuspensionMessageType,
        provider_pid: transfer.provider_pid.clone(),
        consumer_pid: transfer.consumer_pid.clone(),
        code: None,
        reason: vec![serde_json::Value::String(
            "suspended by the provider".to_owned(),
        )],
    };
    outgoing(transfer, "suspension", &msg, Transition::Suspend)
}

pub(super) fn completion(transfer: &Transfer) -> Outgoing {
    let msg = TransferCompletionMessage {
        context: Context::default(),
        kind: TransferCompletionMessageType,
        provider_pid: transfer.provider_pid.clone(),
        consumer_pid: transfer.consumer_pid.clone(),
    };
    outgoing(transfer, "completion", &msg, Transition::Complete)
}

pub(super) fn termination(transfer: &Transfer, reason: &str) -> Outgoing {
    let msg = TransferTerminationMessage {
        context: Context::default(),
        kind: TransferTerminationMessageType,
        provider_pid: transfer.provider_pid.clone(),
        consumer_pid: transfer.consumer_pid.clone(),
        code: None,
        reason: vec![serde_json::Value::String(reason.to_owned())],
    };
    outgoing(transfer, "termination", &msg, Transition::Terminate)
}

/// Wraps `msg` for the consumer's callback endpoint `path` (DSP 2025-1,
/// "Transfer Process HTTPS Binding": `/transfers/:consumerPid/...`).
fn outgoing<T: serde::Serialize>(
    transfer: &Transfer,
    path: &str,
    msg: &T,
    transition: Transition,
) -> Outgoing {
    let status_url = format!(
        "{}/transfers/{}",
        transfer.callback_address.trim_end_matches('/'),
        transfer.consumer_pid
    );
    Outgoing {
        provider_pid: transfer.provider_pid.clone(),
        url: format!("{status_url}/{path}"),
        body: serde_json::to_value(msg).unwrap_or_default(),
        transition,
        to: next_state(Some(transfer.state), transition, Role::Provider).unwrap_or(transfer.state),
        status_url,
        counter_party: transfer.counter_party.clone(),
    }
}
