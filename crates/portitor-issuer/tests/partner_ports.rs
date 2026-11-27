// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The binary's wiring of its clients to holders (R-17):
//! `Ports::for_partners`, as `main.rs` calls it, with the block on
//! internal addresses on and no exceptions. A holder's DID document, and
//! deliveries and offers to its credential service, are refused on an
//! internal address.
#![allow(clippy::unwrap_used, missing_docs)]

use std::sync::Arc;

use portitor_issuer::Ports;
use portitor_issuer::store::DatabaseStore;
use portitor_runtime::internal_addresses::{BLOCK_SETTING, InternalAddresses};
use serde_json::json;

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
async fn a_holder_did_document_on_an_internal_address_is_refused() {
    let (ports, address, _listener) = binary_ports_and_a_local_address();
    let did = format!("did:web:{}", address.replace(':', "%3A"));
    let refused = ports.directory.did_document(&did).await.unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
}

#[tokio::test]
async fn a_delivery_to_a_credential_service_on_an_internal_address_is_refused() {
    let (ports, address, _listener) = binary_ports_and_a_local_address();
    let credentials = format!("http://{address}/api/credentials");
    let refused = ports
        .sender
        .post_authorized(&credentials, &json!({}), "Bearer t")
        .await
        .unwrap_err();
    assert!(refused.contains(BLOCK_SETTING), "{refused}");
}
