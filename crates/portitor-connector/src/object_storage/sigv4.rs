// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! AWS Signature Version 4 for request headers, written with `ring`
//! (HMAC-SHA256 and SHA-256) as ADR 0027, section 4 says. Pure functions,
//! no I/O. The steps and their names follow the AWS documentation
//! "Create a signed AWS API request" (IAM user guide, Signature Version 4):
//!
//! 1. the canonical request ([`canonical_request`]);
//! 2. the string to sign ([`string_to_sign`]);
//! 3. the signing key, derived from the secret key, the date, the region
//!    and the service ([`signature`]);
//! 4. the signature, and the `Authorization` header ([`authorization`]).
//!
//! What is signed is in [`Request`]. A signature is a secret for as long
//! as the request may be replayed: it is never logged.

use std::fmt::Write as _;

use ring::{digest, hmac};

/// The algorithm name in the string to sign and the header.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// SHA-256 of the empty body, hex: the payload hash of a `GET`.
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Lower-case hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    portitor_runtime::system::hex(bytes)
}

/// SHA-256 of `bytes`, lower-case hex: the payload hash
/// (`x-amz-content-sha256`) of a body.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, bytes).as_ref())
}

/// The parts of a request that are signed.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// `GET`, `PUT`, `POST` or `DELETE`.
    pub method: &'a str,
    /// The path, already encoded with [`uri_encode`] (keeping `/`), for
    /// example `/bucket/key`.
    pub path: &'a str,
    /// The query parameters, not encoded: (name, value), in any order. A
    /// parameter without a value (`?uploads`) has an empty value.
    pub query: &'a [(&'a str, &'a str)],
    /// The headers to sign: (name, value), in any order; `host` and
    /// `x-amz-date` must be among them. Names are compared in lower case.
    pub headers: &'a [(&'a str, &'a str)],
    /// SHA-256 of the body, hex ([`sha256_hex`]).
    pub payload_sha256: &'a str,
}

/// Where and when a request is signed: the credential scope.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    /// The time as `x-amz-date` writes it: `20150830T123600Z`.
    pub amz_date: &'a str,
    /// The region, for example `us-east-1`.
    pub region: &'a str,
    /// The service, `s3` for object storage.
    pub service: &'a str,
}

impl Scope<'_> {
    /// The date part of [`Scope::amz_date`]: `20150830`.
    fn date(&self) -> &str {
        self.amz_date.get(..8).unwrap_or(self.amz_date)
    }

    /// `20150830/us-east-1/s3/aws4_request`.
    fn text(&self) -> String {
        format!(
            "{}/{}/{}/aws4_request",
            self.date(),
            self.region,
            self.service
        )
    }
}

/// `UriEncode` of the AWS documentation: every byte but the unreserved
/// characters (`A-Z a-z 0-9 - _ . ~`) becomes `%XY` (upper-case hex);
/// `/` is kept when `keep_slash` (for a path).
#[must_use]
pub fn uri_encode(text: &str, keep_slash: bool) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved || (keep_slash && byte == b'/') {
            encoded.push(char::from(byte));
        } else {
            // writing to a String cannot fail
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// The canonical query string: each name and value encoded, sorted by
/// name (then value), joined with `&`. Also the query of the URL sent, so
/// that both are the same text.
#[must_use]
pub fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (uri_encode(name, false), uri_encode(value, false)))
        .collect();
    pairs.sort();
    let joined: Vec<String> = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    joined.join("&")
}

/// The signed headers, lower-case and sorted, with their values trimmed
/// and inner runs of spaces made one.
fn canonical_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut canonical: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            let words: Vec<&str> = value.split_whitespace().collect();
            (name.to_ascii_lowercase(), words.join(" "))
        })
        .collect();
    canonical.sort();
    canonical
}

/// `host;x-amz-content-sha256;x-amz-date`: the names of the signed
/// headers.
#[must_use]
pub fn signed_headers(headers: &[(&str, &str)]) -> String {
    let names: Vec<String> = canonical_headers(headers)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.join(";")
}

/// Step 1: the canonical request.
#[must_use]
pub fn canonical_request(request: &Request<'_>) -> String {
    let mut header_lines = String::new();
    for (name, value) in canonical_headers(request.headers) {
        // writing to a String cannot fail
        let _ = writeln!(header_lines, "{name}:{value}");
    }
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        request.path,
        canonical_query(request.query),
        header_lines,
        signed_headers(request.headers),
        request.payload_sha256
    )
}

/// Step 2: the string to sign, for `canonical_request` in `scope`.
#[must_use]
pub fn string_to_sign(scope: &Scope<'_>, canonical_request: &str) -> String {
    format!(
        "{ALGORITHM}\n{}\n{}\n{}",
        scope.amz_date,
        scope.text(),
        sha256_hex(canonical_request.as_bytes())
    )
}

/// HMAC-SHA256 of `data` with `key`.
fn hmac_sha256(key: &[u8], data: &str) -> hmac::Tag {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes())
}

/// Steps 3 and 4: the signing key, derived from `secret_access_key` for
/// `scope`, then the signature of `string_to_sign` with it, hex.
#[must_use]
pub fn signature(secret_access_key: &str, scope: &Scope<'_>, string_to_sign: &str) -> String {
    let date_key = hmac_sha256(format!("AWS4{secret_access_key}").as_bytes(), scope.date());
    let region_key = hmac_sha256(date_key.as_ref(), scope.region);
    let service_key = hmac_sha256(region_key.as_ref(), scope.service);
    let signing_key = hmac_sha256(service_key.as_ref(), "aws4_request");
    hex(hmac_sha256(signing_key.as_ref(), string_to_sign).as_ref())
}

/// The `Authorization` header of `request`, signed with the key pair
/// (`access_key_id`, `secret_access_key`) in `scope`.
#[must_use]
pub fn authorization(
    access_key_id: &str,
    secret_access_key: &str,
    scope: &Scope<'_>,
    request: &Request<'_>,
) -> String {
    let to_sign = string_to_sign(scope, &canonical_request(request));
    format!(
        "{ALGORITHM} Credential={access_key_id}/{}, SignedHeaders={}, Signature={}",
        scope.text(),
        signed_headers(request.headers),
        signature(secret_access_key, scope, &to_sign)
    )
}

/// `x-amz-date` for `secs` seconds since 1970: `20150830T123600Z`.
#[must_use]
pub fn amz_date(secs: u64) -> String {
    // `2015-08-30T12:36:00Z` without its separators
    portitor_runtime::system::xsd_date_time(secs).replace(['-', ':'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The key pair of the AWS Signature Version 4 test suite (a published
    // example key, not a secret).
    const SUITE_KEY_ID: &str = "AKIDEXAMPLE";
    const SUITE_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

    fn suite_scope() -> Scope<'static> {
        Scope {
            amz_date: "20150830T123600Z",
            region: "us-east-1",
            service: "service",
        }
    }

    /// The AWS Signature Version 4 test suite, case `get-vanilla`: files
    /// `header-canonical-request.txt`, `header-string-to-sign.txt` and
    /// `header-signature.txt` of
    /// `awslabs/aws-c-auth/tests/aws-signing-test-suite/v4/get-vanilla`
    /// (branch `main`, read on 2026-11-24).
    #[test]
    fn get_vanilla_of_the_aws_test_suite() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let request = Request {
            method: "GET",
            path: "/",
            query: &[],
            headers: &headers,
            payload_sha256: EMPTY_PAYLOAD_SHA256,
        };
        let canonical = canonical_request(&request);
        assert_eq!(
            canonical,
            "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\n\
             host;x-amz-date\n\
             e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let to_sign = string_to_sign(&suite_scope(), &canonical);
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n20150830T123600Z\n20150830/us-east-1/service/aws4_request\n\
             bb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63"
        );
        assert_eq!(
            signature(SUITE_SECRET, &suite_scope(), &to_sign),
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(
            authorization(SUITE_KEY_ID, SUITE_SECRET, &suite_scope(), &request),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    /// The same suite, case `get-vanilla-query-order-key-case`
    /// (`GET /?Param2=value2&Param1=value1`): the query is sorted.
    #[test]
    fn get_vanilla_query_order_key_case_of_the_aws_test_suite() {
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let request = Request {
            method: "GET",
            path: "/",
            query: &[("Param2", "value2"), ("Param1", "value1")],
            headers: &headers,
            payload_sha256: EMPTY_PAYLOAD_SHA256,
        };
        let canonical = canonical_request(&request);
        assert!(canonical.starts_with("GET\n/\nParam1=value1&Param2=value2\n"));
        let to_sign = string_to_sign(&suite_scope(), &canonical);
        assert!(
            to_sign.ends_with("816cd5b414d056048ba4f7c5386d6e0533120fb1fcfa93762cf0fc39e2cf19e0")
        );
        assert_eq!(
            signature(SUITE_SECRET, &suite_scope(), &to_sign),
            "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
        );
    }

    /// The S3 example of the AWS documentation "Signature Calculations
    /// for the Authorization Header: Transferring Payload in a Single
    /// Chunk", "Example: GET Object" (Amazon S3 API reference): object
    /// `test.txt` of `examplebucket`, the first ten bytes, with the
    /// documentation's example key pair.
    #[test]
    fn get_object_example_of_the_s3_documentation() {
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY_PAYLOAD_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let request = Request {
            method: "GET",
            path: "/test.txt",
            query: &[],
            headers: &headers,
            payload_sha256: EMPTY_PAYLOAD_SHA256,
        };
        let scope = Scope {
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            service: "s3",
        };
        let to_sign = string_to_sign(&scope, &canonical_request(&request));
        assert_eq!(
            to_sign,
            "AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n\
             7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
        );
        assert_eq!(
            signature("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", &scope, &to_sign),
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn encoding_keeps_only_unreserved_characters() {
        assert_eq!(uri_encode("a b/c~d_e.f-g", false), "a%20b%2Fc~d_e.f-g");
        assert_eq!(uri_encode("dir/ä.txt", true), "dir/%C3%A4.txt");
        assert_eq!(
            canonical_query(&[("uploads", "")]),
            "uploads=",
            "a parameter without a value"
        );
        assert_eq!(
            canonical_query(&[("uploadId", "a+b"), ("partNumber", "2")]),
            "partNumber=2&uploadId=a%2Bb"
        );
    }

    #[test]
    fn the_date_is_written_as_amz_date() {
        assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
    }
}
