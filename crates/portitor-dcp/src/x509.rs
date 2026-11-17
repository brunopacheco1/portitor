// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Just enough X.509 to tell which key a certificate is for (ADR 0021):
//! the PEM blocks of a chain, and the P-256 public key in the first
//! certificate's `subjectPublicKeyInfo`. The same structure, alone in a
//! `-----BEGIN PUBLIC KEY-----` PEM block (RFC 7468, section 13), is how
//! an operator gives a retired public key ([`public_key_pem`]). Pure, no
//! I/O.
//!
//! Nothing here checks a signature, a date or a name: the identity only
//! publishes the chain (`x5u`), and the party that reads it (a Gaia-X
//! compliance service) checks it against its trust anchors.
//!
//! The certificate is DER (ITU-T X.690): each value is a tag byte, a
//! length and the content. RFC 5280, section 4.1, gives the order of the
//! fields walked here:
//!
//! ```text
//! Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signature }
//! TBSCertificate ::= SEQUENCE {
//!     version [0] EXPLICIT OPTIONAL, serialNumber, signature, issuer,
//!     validity, subject, subjectPublicKeyInfo, ... }
//! SubjectPublicKeyInfo ::= SEQUENCE { algorithm, subjectPublicKey BIT STRING }
//! ```
//!
//! A small reader of our own, rather than a crate: the identity needs one
//! field, and this keeps the dependency list as it is (AGENTS.md).

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as PEM_B64, URL_SAFE_NO_PAD as B64};

use crate::Jwk;

/// Why a certificate chain is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum X509Error {
    /// No `-----BEGIN CERTIFICATE-----` block, or one that is not closed
    /// or not base64.
    #[error("not a PEM certificate chain: {0}")]
    Pem(&'static str),
    /// The DER inside is not a certificate as RFC 5280 writes it.
    #[error("not an X.509 certificate: {0}")]
    Der(&'static str),
    /// The certificate's key is not an EC P-256 key.
    #[error("the certificate's key is not a P-256 key")]
    NotP256,
    /// A private key where only a public key may be given.
    #[error(
        "a private key; give only the public key (openssl pkey -in key.pem -pubout), then destroy the private key"
    )]
    PrivateKey,
}

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

/// DER tags used below (X.690, section 8).
const SEQUENCE: u8 = 0x30;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OBJECT_IDENTIFIER: u8 = 0x06;
/// `[0]`, constructed: the optional `version` of a v2 or v3 certificate.
const VERSION_TAG: u8 = 0xA0;

/// `id-ecPublicKey`, 1.2.840.10045.2.1 (RFC 5480, section 2.1.1), as DER
/// content bytes.
const ID_EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
/// `secp256r1` (P-256), 1.2.840.10045.3.1.7 (RFC 5480, section 2.1.1.1).
const SECP256R1: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];

/// The DER bytes of each certificate in a PEM file, in file order. Text
/// around the blocks (comments that some tools write) is ignored.
///
/// # Errors
/// No certificate block, a block that is not closed, or bad base64.
pub fn pem_certificates(pem: &str) -> Result<Vec<Vec<u8>>, X509Error> {
    let mut certificates = Vec::new();
    let mut rest = pem;
    while let Some((_, after_begin)) = rest.split_once(BEGIN) {
        let (body, after_end) = after_begin
            .split_once(END)
            .ok_or(X509Error::Pem("a block is not closed"))?;
        let base64: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        let der = PEM_B64
            .decode(base64)
            .map_err(|_| X509Error::Pem("bad base64"))?;
        certificates.push(der);
        rest = after_end;
    }
    if certificates.is_empty() {
        return Err(X509Error::Pem("no certificate"));
    }
    Ok(certificates)
}

/// The public key of the first certificate of a PEM chain, as a JWK.
///
/// # Errors
/// Not a PEM chain, not a certificate, or not a P-256 key.
pub fn first_certificate_key(pem: &str) -> Result<Jwk, X509Error> {
    let certificates = pem_certificates(pem)?;
    let first = certificates
        .first()
        .ok_or(X509Error::Pem("no certificate"))?;
    certificate_key(first)
}

/// The P-256 public key in a PEM file that holds one
/// `-----BEGIN PUBLIC KEY-----` block: a `SubjectPublicKeyInfo` (RFC 5280,
/// section 4.1; RFC 7468, section 13), as `openssl pkey -pubout` writes
/// it. A file with a private key is refused, so that a retired private
/// key is never kept just to be listed.
///
/// # Errors
/// A private key, not exactly one public key block, bad base64, not a
/// `SubjectPublicKeyInfo`, or not a P-256 key.
pub fn public_key_pem(pem: &str) -> Result<Jwk, X509Error> {
    const BEGIN_PUBLIC: &str = "-----BEGIN PUBLIC KEY-----";
    const END_PUBLIC: &str = "-----END PUBLIC KEY-----";
    // "PRIVATE KEY", "EC PRIVATE KEY" and "ENCRYPTED PRIVATE KEY" blocks
    if pem.contains("PRIVATE KEY-----") {
        return Err(X509Error::PrivateKey);
    }
    let (_, after_begin) = pem
        .split_once(BEGIN_PUBLIC)
        .ok_or(X509Error::Pem("no PUBLIC KEY block"))?;
    let (body, after_end) = after_begin
        .split_once(END_PUBLIC)
        .ok_or(X509Error::Pem("a block is not closed"))?;
    if after_end.contains(BEGIN_PUBLIC) {
        return Err(X509Error::Pem("more than one PUBLIC KEY block"));
    }
    let base64: String = body.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let der = PEM_B64
        .decode(base64)
        .map_err(|_| X509Error::Pem("bad base64"))?;
    let (spki, rest) = expect(&der, SEQUENCE, "subjectPublicKeyInfo")?;
    if !rest.is_empty() {
        return Err(X509Error::Der("bytes after subjectPublicKeyInfo"));
    }
    p256_key(spki)
}

/// The DER bytes that start the `SubjectPublicKeyInfo` of every P-256
/// key (RFC 5480, section 2): `SEQUENCE` (89 bytes) { `SEQUENCE` {
/// `id-ecPublicKey`, `secp256r1` }, `BIT STRING` (66 bytes, 0 unused bits)
/// }. The 65-byte point `04 || x || y` follows. Only
/// [`p256_public_key_pem`] writes it, so it exists in test builds only.
#[cfg(any(test, feature = "test-support"))]
const P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01, 0x06, 0x08, 0x2A,
    0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The PEM text of a P-256 public key (`-----BEGIN PUBLIC KEY-----`), as
/// `openssl pkey -pubout` writes it: the reverse of [`public_key_pem`].
/// Tests use it to write a retired key file, so it exists only in test
/// builds (feature `test-support`).
///
/// # Errors
/// Not a P-256 key, or its coordinates are not 32 bytes of base64url.
#[cfg(any(test, feature = "test-support"))]
pub fn p256_public_key_pem(key: &Jwk) -> Result<String, X509Error> {
    let Jwk::EC { crv, x, y } = key else {
        return Err(X509Error::NotP256);
    };
    if crv != "P-256" {
        return Err(X509Error::NotP256);
    }
    let x = B64.decode(x).map_err(|_| X509Error::NotP256)?;
    let y = B64.decode(y).map_err(|_| X509Error::NotP256)?;
    if x.len() != 32 || y.len() != 32 {
        return Err(X509Error::NotP256);
    }
    let mut der = P256_SPKI_PREFIX.to_vec();
    der.push(0x04);
    der.extend(x);
    der.extend(y);
    let base64 = PEM_B64.encode(der);
    // RFC 7468, section 2: lines of 64 characters
    let mut pem = String::from("-----BEGIN PUBLIC KEY-----\n");
    for line in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(line));
        pem.push('\n');
    }
    pem.push_str("-----END PUBLIC KEY-----\n");
    Ok(pem)
}

/// The public key of one DER certificate, as a JWK. Only EC P-256 keys
/// are read; any other key is [`X509Error::NotP256`].
///
/// # Errors
/// Not a certificate, or not a P-256 key.
pub fn certificate_key(der: &[u8]) -> Result<Jwk, X509Error> {
    let spki = subject_public_key_info(der)?;
    p256_key(spki)
}

/// The content of `subjectPublicKeyInfo`: walks into the certificate and
/// its `tbsCertificate`, and skips the fields before it.
fn subject_public_key_info(der: &[u8]) -> Result<&[u8], X509Error> {
    let (certificate, _) = expect(der, SEQUENCE, "certificate")?;
    let (tbs, _) = expect(certificate, SEQUENCE, "tbsCertificate")?;
    let mut fields = tbs;
    // the version is there for v2 and v3 certificates only
    if fields.first() == Some(&VERSION_TAG) {
        fields = next(fields)?.rest;
    }
    let (_serial, rest) = expect(fields, INTEGER, "serialNumber")?;
    let (_signature, rest) = expect(rest, SEQUENCE, "signature")?;
    let (_issuer, rest) = expect(rest, SEQUENCE, "issuer")?;
    let (_validity, rest) = expect(rest, SEQUENCE, "validity")?;
    let (_subject, rest) = expect(rest, SEQUENCE, "subject")?;
    let (spki, _) = expect(rest, SEQUENCE, "subjectPublicKeyInfo")?;
    Ok(spki)
}

/// The JWK of a `subjectPublicKeyInfo` that holds a P-256 key: the
/// algorithm is `id-ecPublicKey` with the curve `secp256r1`, and the key
/// is the uncompressed point `04 || x || y` (RFC 5480, section 2.2).
fn p256_key(spki: &[u8]) -> Result<Jwk, X509Error> {
    let (algorithm, rest) = expect(spki, SEQUENCE, "algorithm")?;
    let (key_type, parameters) = expect(algorithm, OBJECT_IDENTIFIER, "algorithm id")?;
    if key_type != ID_EC_PUBLIC_KEY {
        return Err(X509Error::NotP256);
    }
    let (curve, _) =
        expect(parameters, OBJECT_IDENTIFIER, "curve").map_err(|_| X509Error::NotP256)?;
    if curve != SECP256R1 {
        return Err(X509Error::NotP256);
    }
    let (bits, _) = expect(rest, BIT_STRING, "subjectPublicKey")?;
    // a BIT STRING starts with the number of unused bits, here 0
    let Some((0, point)) = bits.split_first() else {
        return Err(X509Error::Der("subjectPublicKey"));
    };
    if point.len() != 65 || point[0] != 0x04 {
        return Err(X509Error::NotP256);
    }
    Ok(Jwk::EC {
        crv: "P-256".to_owned(),
        x: B64.encode(&point[1..33]),
        y: B64.encode(&point[33..65]),
    })
}

/// One DER value: its tag, its content, and the bytes after it.
struct Tlv<'a> {
    tag: u8,
    content: &'a [u8],
    rest: &'a [u8],
}

/// The next value must have `tag`: returns its content and what follows.
fn expect<'a>(
    bytes: &'a [u8],
    tag: u8,
    what: &'static str,
) -> Result<(&'a [u8], &'a [u8]), X509Error> {
    let value = next(bytes)?;
    if value.tag != tag {
        return Err(X509Error::Der(what));
    }
    Ok((value.content, value.rest))
}

/// Reads one DER value (X.690, section 8.1): a one-byte tag (every tag
/// in a certificate fits one byte), then the length, either one byte
/// below 128, or `0x80 + n` followed by `n` length bytes. DER has no
/// "indefinite" length, so `0x80` alone is refused.
fn next(bytes: &[u8]) -> Result<Tlv<'_>, X509Error> {
    let truncated = X509Error::Der("truncated");
    let (&tag, after_tag) = bytes.split_first().ok_or(truncated.clone())?;
    let (&first, after_first) = after_tag.split_first().ok_or(truncated.clone())?;
    let (length, after_length) = if first < 0x80 {
        (usize::from(first), after_first)
    } else {
        let count = usize::from(first & 0x7F);
        // four length bytes are 4 GiB, far more than any certificate
        if count == 0 || count > 4 || after_first.len() < count {
            return Err(X509Error::Der("length"));
        }
        let (length_bytes, after) = after_first.split_at(count);
        let mut length = 0_usize;
        for byte in length_bytes {
            length = (length << 8) | usize::from(*byte);
        }
        (length, after)
    };
    if after_length.len() < length {
        return Err(truncated);
    }
    let (content, rest) = after_length.split_at(length);
    Ok(Tlv { tag, content, rest })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::SigningKey;

    const KEY: &str = include_str!("../tests/fixtures/test-only-p256.pem");
    /// A certificate for [`KEY`] (version 1, no extensions) and the
    /// test CA's certificate (version 3), made with `openssl req` and
    /// `openssl x509 -req`.
    const CHAIN: &str = include_str!("../tests/fixtures/test-only-p256-chain.pem");
    /// A self-signed version 3 certificate for another P-256 key.
    const OTHER: &str = include_str!("../tests/fixtures/test-only-other-p256-cert.pem");
    /// A self-signed certificate for an RSA key.
    const RSA: &str = include_str!("../tests/fixtures/test-only-rsa-cert.pem");
    /// The public key of [`KEY`], written by `openssl pkey -pubout`.
    const PUBLIC: &str = include_str!("../tests/fixtures/test-only-p256-public.pem");

    #[test]
    fn the_first_certificate_names_the_key() {
        let key = SigningKey::from_pem(KEY).unwrap();
        assert_eq!(pem_certificates(CHAIN).unwrap().len(), 2);
        assert_eq!(first_certificate_key(CHAIN).unwrap(), key.public_jwk());
        // a version 3 certificate, with the [0] version field
        let other = first_certificate_key(OTHER).unwrap();
        assert_ne!(other, key.public_jwk());
        assert!(matches!(other, Jwk::EC { .. }));
    }

    #[test]
    fn other_keys_and_broken_input_are_refused() {
        assert_eq!(first_certificate_key(RSA), Err(X509Error::NotP256));
        assert_eq!(
            first_certificate_key("no PEM here"),
            Err(X509Error::Pem("no certificate"))
        );
        assert_eq!(
            first_certificate_key(&format!("{BEGIN}\nMIIB\n")),
            Err(X509Error::Pem("a block is not closed"))
        );
        assert_eq!(
            first_certificate_key(&format!("{BEGIN}\n!!\n{END}\n")),
            Err(X509Error::Pem("bad base64"))
        );
        // a valid certificate cut short anywhere is refused, never a panic
        let der = pem_certificates(CHAIN).unwrap().remove(0);
        for length in 0..der.len() {
            assert!(certificate_key(&der[..length]).is_err(), "{length}");
        }
        assert!(certificate_key(&[0x30, 0x80]).is_err(), "indefinite length");
        assert!(certificate_key(&[0x30, 0x85, 1, 1, 1, 1, 1]).is_err());
    }

    #[test]
    fn a_public_key_pem_gives_its_p256_key() {
        let key = SigningKey::from_pem(KEY).unwrap();
        assert_eq!(public_key_pem(PUBLIC).unwrap(), key.public_jwk());
        // written the same way as openssl writes it
        assert_eq!(p256_public_key_pem(&key.public_jwk()).unwrap(), PUBLIC);
        let other = SigningKey::generate().unwrap().public_jwk();
        let pem = p256_public_key_pem(&other).unwrap();
        assert_eq!(public_key_pem(&pem).unwrap(), other);
        // RSA and P-384 public keys, written by `openssl pkey -pubout`
        let rsa = include_str!("../tests/fixtures/test-only-rsa-public.pem");
        assert_eq!(public_key_pem(rsa), Err(X509Error::NotP256));
        let p384 = include_str!("../tests/fixtures/test-only-p384-public.pem");
        assert_eq!(public_key_pem(p384), Err(X509Error::NotP256));
    }

    #[test]
    fn a_private_key_or_a_broken_public_key_is_refused() {
        assert_eq!(public_key_pem(KEY), Err(X509Error::PrivateKey));
        let both = format!("{PUBLIC}{KEY}");
        assert_eq!(public_key_pem(&both), Err(X509Error::PrivateKey));
        assert_eq!(
            public_key_pem(CHAIN),
            Err(X509Error::Pem("no PUBLIC KEY block"))
        );
        assert_eq!(
            public_key_pem(&format!("{PUBLIC}{PUBLIC}")),
            Err(X509Error::Pem("more than one PUBLIC KEY block"))
        );
        let unclosed = PUBLIC.replace("-----END PUBLIC KEY-----", "");
        assert_eq!(
            public_key_pem(&unclosed),
            Err(X509Error::Pem("a block is not closed"))
        );
        // a valid key cut short anywhere is refused, never a panic
        let mut body = String::new();
        for line in PUBLIC.lines() {
            if !line.starts_with("-----") {
                body.push_str(line);
            }
        }
        let der = PEM_B64.decode(body).unwrap();
        for length in 0..der.len() {
            let cut = format!(
                "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
                PEM_B64.encode(&der[..length])
            );
            assert!(public_key_pem(&cut).is_err(), "{length}");
        }
    }
}
