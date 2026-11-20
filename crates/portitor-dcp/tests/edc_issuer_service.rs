// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What an EDC Issuer Service (`IdentityHub` v1.0.0-RC8, DCP profile
//! `vc20-bssl/jwt`) issues, read as a verifier reads it: the credential,
//! and its Bitstring Status List before and after the revocation.
//!
//! The files in `fixtures/edc-issuerservice-rc8/` are that runtime's own
//! output, recorded on 2026-11-20 with test-only keys made for the
//! recording: the issuer's DID document, a credential it delivered to a
//! Portitor identity, and its status list credential fetched before and
//! after `POST .../credentials/{id}/revoke` on its admin API. The EDC
//! interop test (`scripts/edc-interop.sh`) does the same with running
//! components; this test keeps the formats under `cargo test`.
#![allow(clippy::unwrap_used, missing_docs)]

use portitor_dcp::DidDocument;
use portitor_dcp::credential::{Credential, Profile};
use portitor_dcp::status_list::{self, Status};

const DID_DOCUMENT: &str = include_str!("fixtures/edc-issuerservice-rc8/did.json");
const CREDENTIAL: &str = include_str!("fixtures/edc-issuerservice-rc8/credential.jwt");
const LIST_BEFORE: &str = include_str!("fixtures/edc-issuerservice-rc8/status-list-before.jwt");
const LIST_AFTER: &str = include_str!("fixtures/edc-issuerservice-rc8/status-list-after.jwt");

const ISSUER: &str = "did:web:localhost%3A9083";
const HOLDER: &str = "did:web:localhost%3A8090";
/// 2026-11-20T23:00:00Z: after the credential's `validFrom`, before its
/// `validUntil` a day later.
const NOW: u64 = 1_795_215_600;

fn issuer() -> DidDocument {
    serde_json::from_str(DID_DOCUMENT).unwrap()
}

#[test]
fn the_credential_is_read_and_verified() {
    let credential = Credential::parse(CREDENTIAL.trim()).unwrap();
    assert_eq!(credential.profile, Profile::Vc20);
    assert_eq!(credential.issuer, ISSUER);
    assert!(credential.has_type("InteropTestCredential"));
    // EDC writes credentialSubject as an array of one, and validFrom and
    // validUntil with nine fractional digits
    assert!(credential.is_only_about(HOLDER));
    assert_eq!(credential.subjects[0]["level"], "basic");
    credential.verify(&issuer(), NOW, 0).unwrap();
    // the recording's credential definition gave one day of validity
    assert!(credential.verify(&issuer(), NOW + 86_400, 0).is_err());

    // credentialStatus is one object, its index a string
    let [entry] = credential.status.as_slice() else {
        panic!("one status entry expected: {:?}", credential.status);
    };
    assert_eq!(entry.kind, "BitstringStatusListEntry");
    assert_eq!(entry.status_purpose, "revocation");
    assert_eq!(entry.status_list_index, "0");
}

#[test]
fn the_status_list_shows_the_revocation() {
    let credential = Credential::parse(CREDENTIAL.trim()).unwrap();
    let entry = &credential.status[0];
    for (jwt, expected) in [(LIST_BEFORE, Status::Clear), (LIST_AFTER, Status::Set)] {
        let list_credential = Credential::parse(jwt.trim()).unwrap();
        // the list is the credential the entry names, from the same issuer
        assert_eq!(
            list_credential.id.as_deref(),
            Some(entry.status_list_credential.as_str())
        );
        assert_eq!(list_credential.issuer, credential.issuer);
        list_credential.verify(&issuer(), NOW, 0).unwrap();
        let list = status_list::list_in(&list_credential, entry).unwrap();
        assert_eq!(list.status_purpose, "revocation");
        assert_eq!(status_list::check(entry, &list).unwrap(), expected);
    }
}
