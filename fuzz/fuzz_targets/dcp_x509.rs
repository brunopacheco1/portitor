// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The X.509 reader (`portitor_dcp::x509`, ADR 0021): a certificate chain
//! comes from a certificate authority, and the DER reader is our own. The
//! input is read as a PEM chain and as one DER certificate. Reading must
//! never panic, and a key it returns is always a P-256 key with a 32-byte
//! x and y.
#![no_main]

use base64::Engine;
use base64::engine::general_purpose::STANDARD as PEM_B64;
use libfuzzer_sys::fuzz_target;
use portitor_dcp::Jwk;
use portitor_dcp::x509::{self, X509Error};

/// A P-256 JWK has `x` and `y` of 32 bytes, 43 base64url characters each.
fn check_key(key: &Result<Jwk, X509Error>) {
    if let Ok(jwk) = key {
        match jwk {
            Jwk::EC { crv, x, y } => {
                assert_eq!(crv, "P-256");
                assert_eq!(x.len(), 43);
                assert_eq!(y.len(), 43);
            }
            // the key is not printed: a log of key material is flagged by
            // CodeQL, and the input that found it is saved by the fuzzer
            _ => panic!("not an EC key"),
        }
    }
}

/// The input as the base64 body of one PEM block, so the DER reader sees
/// the same bytes through the PEM path.
fn as_pem_block(der: &[u8]) -> String {
    format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        PEM_B64.encode(der)
    )
}

fuzz_target!(|data: &[u8]| {
    // any bytes as one DER certificate
    let from_der = x509::certificate_key(data);
    check_key(&from_der);
    // the same bytes in a PEM block give the same answer
    let from_pem = x509::first_certificate_key(&as_pem_block(data));
    check_key(&from_pem);
    assert!(from_der == from_pem, "DER and PEM paths disagree");
    // any text as a PEM chain
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(certificates) = x509::pem_certificates(text) {
            assert!(!certificates.is_empty());
        }
        check_key(&x509::first_certificate_key(text));
    }
});
