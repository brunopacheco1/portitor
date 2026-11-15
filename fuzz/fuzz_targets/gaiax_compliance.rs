// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The Gaia-X code that reads what a caller or a compliance service sends
//! (`portitor_dcp::gaiax` and `gaiax_sign::check_answer`, ADRs 0021 and
//! 0022). The input is read three ways: as any JSON, put in RFC 8785 (JCS)
//! form; as a credential JWT; and as the payload of an unsigned VC-JWT,
//! so the Gaia-X checks see any JSON object as a credential. Nothing may
//! panic; the JCS form must be valid JSON that does not change when put in
//! JCS form again (else the same credential could have two digests); every
//! digest is `sha256-` and 64 hex digits; and an answer that lists any
//! credential is never accepted when nothing was sent.
#![no_main]

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use libfuzzer_sys::fuzz_target;
use portitor_dcp::credential::Credential;
use portitor_dcp::gaiax::{self, COMPLIANCE_SCOPE_TYPE};
use portitor_dcp::gaiax_sign::check_answer;
use serde_json::Value;

/// `{"alg":"ES256","typ":"vc+jwt"}`: the header a compliance answer has.
const VC_JWT_HEADER: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6InZjK2p3dCJ9";

fn check_digest(digest: &str) {
    let hex = digest.strip_prefix("sha256-").expect("sha256- prefix");
    assert_eq!(hex.len(), 64);
    assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()));
}

fn check_credential(credential: &Credential) {
    if let Ok(payload) = credential.payload() {
        check_digest(&gaiax::digest_sri(&payload));
        let contexts = gaiax::contexts(&payload);
        let _ = gaiax::matches_scope_type(&credential.types, &contexts, COMPLIANCE_SCOPE_TYPE);
    }
    let listed = gaiax::compliant_credentials(credential);
    if check_answer(credential, "urn:fuzz:compliance", &[]).is_ok() {
        assert!(listed.is_empty());
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let canonical = gaiax::canonical_json(&value);
        let again: Value = serde_json::from_str(&canonical).expect("JCS form is JSON");
        assert_eq!(gaiax::canonical_json(&again), canonical);
        if let Value::Object(object) = &value {
            check_digest(&gaiax::digest_sri(object));
        }
    }
    if let Ok(credential) = Credential::parse(text) {
        check_credential(&credential);
    }
    let jwt = format!("{VC_JWT_HEADER}.{}.AA", B64.encode(data));
    if let Ok(credential) = Credential::parse(&jwt) {
        check_credential(&credential);
    }
});
