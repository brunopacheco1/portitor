// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The binary's wiring of its clients to other parties (R-17):
//! `Ports::for_partners`, as `main.rs` calls it, with the block on
//! internal addresses on and no exceptions. DID documents, issuers'
//! metadata and status lists, and requests to issuers, are refused on an
//! internal address.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::Arc;

use portitor_dcp::message::CredentialRequestMessage;
use portitor_identity::Ports;
use portitor_identity::issuance::SendError;
use portitor_identity::store::DatabaseStore;
use portitor_runtime::internal_addresses::{BLOCK_SETTING, InternalAddresses};

/// The ports the binary builds, with plain http allowed (so that the
/// block is what refuses) and the block on without exceptions; and the
/// address of a listening port on 127.0.0.1, which the listener keeps
/// open.
fn binary_ports_and_a_local_address() -> (Ports, String, std::net::TcpListener) {
    let blocked = InternalAddresses {
        block: true,
        ..InternalAddresses::default()
    };
    let ports = Ports::for_partners(
        Arc::new(DatabaseStore::in_memory().unwrap()),
        None,
        true,
        0,
        &blocked,
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    (ports, address, listener)
}

#[tokio::test]
async fn a_did_document_on_an_internal_address_is_refused() {
    let (ports, address, _listener) = binary_ports_and_a_local_address();
    let did = format!("did:web:{}", address.replace(':', "%3A"));
    let refused = ports.directory.did_document(&did).await.unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
    let metadata = format!("http://{address}/api/metadata");
    let refused = ports.directory.json(&metadata).await.unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
}

#[tokio::test]
async fn a_status_list_on_an_internal_address_is_refused() {
    let (ports, address, _listener) = binary_ports_and_a_local_address();
    let list = format!("http://{address}/status/1");
    let refused = ports
        .status_lists
        .status_list_text(&list)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains(BLOCK_SETTING), "{refused}");
}

#[tokio::test]
async fn a_credential_request_to_an_issuer_on_an_internal_address_is_refused() {
    let (ports, address, _listener) = binary_ports_and_a_local_address();
    let url = format!("http://{address}/api/credentials");
    let message = CredentialRequestMessage::new("urn:uuid:h".to_owned(), vec!["m".to_owned()]);
    let refused = ports
        .issuers
        .request_credentials(&url, &message, "token")
        .await
        .unwrap_err();
    let text = match refused {
        SendError::Refused(text) | SendError::Unknown(text) => text,
    };
    assert!(text.contains(BLOCK_SETTING), "{text}");
    let status = format!("http://{address}/api/requests/r");
    let refused = ports
        .issuers
        .request_status(&status, "token")
        .await
        .unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
}
