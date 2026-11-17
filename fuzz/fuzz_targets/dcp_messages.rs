// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Parsing of the DCP messages a Credential Service receives (and the
//! issuer metadata and request status it reads), the credential requests
//! an issuer service receives, the presentation responses the
//! connector's verifier receives, and scopes, from any bytes. Parsing
//! must never panic; a message that parses must be written back and
//! parse again to the same value.
#![no_main]

use libfuzzer_sys::fuzz_target;
use portitor_dcp::message::{
    CredentialMessage, CredentialOfferMessage, CredentialRequestMessage, CredentialStatus,
    IssuerMetadata, PresentationQueryMessage, PresentationResponseMessage,
};
use portitor_dcp::scope::Scope;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn round_trip<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(data: &[u8]) -> Option<T> {
    let parsed: T = serde_json::from_slice(data).ok()?;
    let written = serde_json::to_vec(&parsed).expect("write");
    let again: T = serde_json::from_slice(&written).expect("parse what we wrote");
    assert_eq!(parsed, again);
    Some(parsed)
}

fuzz_target!(|data: &[u8]| {
    if let Some(query) = round_trip::<PresentationQueryMessage>(data)
        && let Ok(scopes) = query.scopes()
    {
        for scope in scopes {
            let _ = Scope::parse(scope, &["org.eclipse.edc.vc.type"]);
        }
    }
    let _ = round_trip::<CredentialMessage>(data);
    let _ = round_trip::<CredentialOfferMessage>(data);
    let _ = round_trip::<IssuerMetadata>(data);
    let _ = round_trip::<PresentationResponseMessage>(data);
    let _ = round_trip::<CredentialRequestMessage>(data);
    let _ = round_trip::<CredentialStatus>(data);
    // the whole input as one scope, as the STS and the credential
    // service read them, without and with the EDC alias
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = Scope::parse(text, &[]);
        let _ = Scope::parse(text, &["org.eclipse.edc.vc.type"]);
    }
});
