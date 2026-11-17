// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Untrusted DID documents: any party can serve a `did:web` document, and
//! the verifiers read it and pick a key from it. Parsing and key selection
//! must never panic, and a key is only returned when it is the one the
//! `kid` names and it has the verification relationship asked for. Each
//! key is also used to check a signature, as the verifiers do, so the
//! JWK's base64 and key bytes are read too; a junk signature never
//! verifies.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use portitor_dcp::bundle;
use portitor_dcp::{DidDocument, Jws, Purpose, SigningKey, VerificationMethod};

/// A PS256 JWS (`{"alg":"PS256"}`, payload `{}`) with a junk signature.
const PS256_JUNK: &str = "eyJhbGciOiJQUzI1NiJ9.e30.AAAA";

/// An ES256 JWS, correctly signed by a key that is not in any document.
fn es256_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let key = SigningKey::generate().expect("key");
        key.sign_json("#key-1", "JWT", &serde_json::json!({}))
            .expect("sign")
    })
}

/// Checks both tokens with the key of `vm`, as a verifier would.
fn verify_with(vm: &VerificationMethod) {
    if let Ok(jws) = Jws::parse(es256_token()) {
        let _ = jws.verify(&vm.public_key_jwk);
    }
    let junk = Jws::parse(PS256_JUNK).expect("PS256 token");
    assert!(junk.verify(&vm.public_key_jwk).is_err());
}

const PURPOSES: [Purpose; 3] = [
    Purpose::CapabilityInvocation,
    Purpose::Authentication,
    Purpose::AssertionMethod,
];

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = serde_json::from_slice::<DidDocument>(data) else {
        return;
    };
    let _ = doc.credential_service();
    let _ = doc.service_url("IssuerService");
    // no kid, a fixed relative kid, and every id the document names
    let mut kids: Vec<Option<String>> = vec![None, Some("#key-1".to_owned())];
    for vm in &doc.verification_method {
        kids.push(Some(vm.id.clone()));
        if let Some(fragment) = vm.id.strip_prefix(&doc.id) {
            kids.push(Some(fragment.to_owned()));
        }
    }
    for vm in &doc.verification_method {
        verify_with(vm);
    }
    // the bundle key, looked up as the bundle code does
    for kid in kids.iter().flatten() {
        let _ = bundle::key_from_document(&doc, kid, &doc.id);
    }
    for purpose in PURPOSES {
        for kid in &kids {
            let Ok(vm) = doc.key_for(kid.as_deref(), purpose) else {
                continue;
            };
            verify_with(vm);
            if let Some(kid) = kid {
                let named = vm.id == *kid || vm.id.strip_prefix(&doc.id) == Some(kid.as_str());
                assert!(named, "{} is not the key {kid} names", vm.id);
            }
        }
    }
});
