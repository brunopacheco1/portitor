// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Gaia-X credentials: the parts both the identity (ADR 0021) and the
//! connector (ADR 0022) need. Pure: no I/O.
//!
//! - [`digest_sri`]: the integrity value a Gaia-X compliance credential
//!   keeps for each credential it checked: `sha256-` and the hex SHA-256
//!   of the credential in JSON Canonicalization Scheme form (RFC 8785).
//!   Source: gx-compliance v2.16.0, `CompliantCredentialSubjectMapper`
//!   (`HashingUtils.sha256(canonicalize(verifiableCredential))`), and its
//!   `docs/api-usage.md`, "Credential integrity check".
//! - [`compliant_credentials`]: the `gx:compliantCredentials` list of a
//!   `gx:ParticipantComplianceCredential`.
//! - [`matches_scope_type`]: how a DCP scope type (no colon allowed)
//!   names a Gaia-X type (a compact IRI such as `gx:LegalPerson`),
//!   ADR 0021 "Credential types and scopes".

use serde_json::{Map, Value};

use crate::credential::{Credential, CredentialError};

/// Gaia-X JSON-LD contexts start with this IRI, for example
/// `https://w3id.org/gaia-x/development#`.
pub const GAIAX_CONTEXT_PREFIX: &str = "https://w3id.org/gaia-x/";

/// The prefix Gaia-X contexts define for their terms.
pub const GAIAX_PREFIX: &str = "gx:";

/// The type of the credential a Gaia-X compliance service issues for a
/// participant (gx-compliance v2.16.0, `EvidenceService`).
pub const COMPLIANCE_TYPE: &str = "gx:ParticipantComplianceCredential";

/// The type of a participant's legal person credential.
pub const LEGAL_PERSON_TYPE: &str = "gx:LegalPerson";

/// The type of the terms and conditions credential.
pub const ISSUER_TYPE: &str = "gx:Issuer";

/// The scope type for [`COMPLIANCE_TYPE`] (ADR 0021).
pub const COMPLIANCE_SCOPE_TYPE: &str = "ParticipantComplianceCredential";

/// The scope type for [`LEGAL_PERSON_TYPE`] (ADR 0021).
pub const LEGAL_PERSON_SCOPE_TYPE: &str = "LegalPerson";

/// One entry of `gx:compliantCredentials`: a credential the compliance
/// service checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompliantCredential {
    /// The checked credential's `id`.
    pub id: String,
    /// Its Gaia-X types, comma separated, for example `gx:LegalPerson`.
    pub credential_type: String,
    /// `sha256-<hex>` of the credential in JCS form.
    pub digest_sri: String,
}

/// `sha256-` and the hex SHA-256 of `credential` in JCS form (RFC 8785).
/// `credential` is the decoded payload of a VC 2.0 JWT.
#[must_use]
pub fn digest_sri(credential: &Map<String, Value>) -> String {
    let canonical = canonical_json(&Value::Object(credential.clone()));
    format!("sha256-{}", crate::sha256_hex(canonical.as_bytes()))
}

/// The digest of a credential JWT's payload, see [`digest_sri`].
///
/// # Errors
/// The JWT is malformed or its payload is not a JSON object.
pub fn digest_sri_of(credential: &Credential) -> Result<String, CredentialError> {
    Ok(digest_sri(&credential.payload()?))
}

/// The `gx:compliantCredentials` of a compliance credential. Entries
/// without an `id` or a `digestSRI` are left out; a credential that is
/// not a compliance credential has none.
#[must_use]
pub fn compliant_credentials(credential: &Credential) -> Vec<CompliantCredential> {
    if !credential.has_type(COMPLIANCE_TYPE) {
        return Vec::new();
    }
    credential
        .subjects
        .iter()
        .filter_map(|subject| subject.get("gx:compliantCredentials"))
        .flat_map(one_or_many)
        .filter_map(compliant_credential)
        .collect()
}

fn compliant_credential(entry: &Value) -> Option<CompliantCredential> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(CompliantCredential {
        id: text("id")?,
        credential_type: text("gx:credentialType").unwrap_or_default(),
        digest_sri: text("digestSRI")?,
    })
}

fn one_or_many(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(many) => many.iter().collect(),
        one => vec![one],
    }
}

/// Whether a credential of `types`, with JSON-LD `contexts`, matches the
/// scope type `scope_type` (ADR 0021): a type equal to `scope_type`, or
/// `gx:<scope_type>` when one of the contexts is a Gaia-X context.
#[must_use]
pub fn matches_scope_type(types: &[String], contexts: &[String], scope_type: &str) -> bool {
    if types.iter().any(|t| t == scope_type) {
        return true;
    }
    let gaiax = contexts.iter().any(|c| c.starts_with(GAIAX_CONTEXT_PREFIX));
    gaiax
        && types
            .iter()
            .any(|t| t.strip_prefix(GAIAX_PREFIX) == Some(scope_type))
}

/// The string `@context` entries of a credential payload.
#[must_use]
pub fn contexts(payload: &Map<String, Value>) -> Vec<String> {
    payload
        .get("@context")
        .map(one_or_many)
        .unwrap_or_default()
        .into_iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// `value` in JSON Canonicalization Scheme form (RFC 8785): no
/// whitespace, object members sorted by the UTF-16 code units of their
/// names, strings escaped as `JSON.stringify` does, numbers written as
/// ECMAScript writes them (RFC 8785 section 3.2.2.3).
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => out.push_str(&value.to_string()),
        Value::Number(number) => out.push_str(&ecmascript_number(number)),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut names: Vec<&String> = members.keys().collect();
            names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (index, name) in names.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(name.clone()).to_string());
                out.push(':');
                if let Some(member) = members.get(name) {
                    write_canonical(member, out);
                }
            }
            out.push('}');
        }
    }
}

/// A JSON number as ECMAScript's `Number.prototype.toString` writes it
/// (ECMA-262, `Number::toString`), which RFC 8785 requires.
fn ecmascript_number(number: &serde_json::Number) -> String {
    const EXACT: u64 = 1 << 53;
    if let Some(int) = number.as_i64()
        && int.unsigned_abs() < EXACT
    {
        return int.to_string();
    }
    if let Some(int) = number.as_u64()
        && int < EXACT
    {
        return int.to_string();
    }
    number
        .as_f64()
        .map_or_else(|| number.to_string(), ecmascript_f64)
}

/// A double as ECMAScript writes it in base 10 (ECMA-262,
/// `Number::toString`). `serde_json` numbers are always finite, so the
/// NaN and Infinity steps are left out.
fn ecmascript_f64(value: f64) -> String {
    // "If x is +0 or -0, return "0"."
    if value == 0.0 {
        return "0".to_owned();
    }
    // "If x < -0, return "-" and Number::toString(-x)."
    let sign = if value < 0.0 { "-" } else { "" };
    let shortest = ShortestDecimal::of(value.abs());
    format!("{sign}{}", shortest.ecmascript_text())
}

/// The digits ECMAScript picks for a positive double: the shortest digit
/// string `s` (`k` digits) and the position `n` of the decimal point, so
/// that the value is `0.s × 10^n`. 4.5 is `s = "45"`, `n = 1`; 0.002 is
/// `s = "2"`, `n = -2`.
struct ShortestDecimal {
    digits: String,
    point: i64,
}

impl ShortestDecimal {
    fn of(value: f64) -> Self {
        // Rust's `{:e}` writes the shortest digits that read back to the
        // same value, as ECMAScript does: "d.ddde<exp>" or "de<exp>".
        let scientific = format!("{value:e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
        let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        let point = exponent.parse::<i64>().unwrap_or(0) + 1;
        let digits = even_digits_on_tie(digits, point, value);
        Self { digits, point }
    }

    /// The number of digits, `k`.
    fn len(&self) -> i64 {
        i64::try_from(self.digits.len()).unwrap_or(i64::MAX)
    }

    fn ecmascript_text(&self) -> String {
        let (digits, n, k) = (&self.digits, self.point, self.len());
        if k <= n && n <= 21 {
            // An integer below 10^21: the digits, then n - k zeros.
            // 1e20 is "100000000000000000000".
            format!("{digits}{}", zeros(n - k))
        } else if 0 < n && n <= 21 {
            // A point inside the digits. 4.5 is "4.5".
            let (int, fraction) = digits.split_at(usize::try_from(n).unwrap_or(0));
            format!("{int}.{fraction}")
        } else if -6 < n && n <= 0 {
            // Small, but at most six zeros after the point. 0.002 is "0.002".
            format!("0.{}{digits}", zeros(-n))
        } else {
            // Anything else uses an exponent with a sign: 1e21 is "1e+21",
            // 1.5e-9 is "1.5e-9" (one digit: no point).
            let exponent = n - 1;
            let exponent_sign = if exponent < 0 { "-" } else { "+" };
            let (first, rest) = digits.split_at(1);
            let point_and_rest = if rest.is_empty() {
                String::new()
            } else {
                format!(".{rest}")
            };
            format!("{first}{point_and_rest}e{exponent_sign}{}", exponent.abs())
        }
    }
}

fn zeros(count: i64) -> String {
    "0".repeat(usize::try_from(count).unwrap_or(0))
}

/// ECMAScript: when two shortest digit strings are equally close to the
/// value, "choose the one that is even". Rust's `{:e}` may pick the odd
/// one: 1424953923781206.25 is written 1424953923781206.3 by Rust and
/// 1424953923781206.2 by JavaScript (RFC 8785 appendix B).
fn even_digits_on_tie(digits: String, point: i64, value: f64) -> String {
    let Some(last) = digits.bytes().last().map(|byte| byte - b'0') else {
        return digits;
    };
    if last % 2 == 0 {
        return digits;
    }
    let head = &digits[..digits.len() - 1];
    // An odd last digit is 1 to 9, so last - 1 is at least 0; last + 1
    // may be 10, which is not a digit and is left out.
    for neighbour in [last - 1, last + 1] {
        if neighbour > 9 {
            continue;
        }
        let candidate = format!("{head}{neighbour}");
        let lower = if neighbour < last {
            &candidate
        } else {
            &digits
        };
        // A tie: the value is exactly halfway, so its exact decimal
        // digits are the lower string followed by a 5.
        let halfway = format!("{lower}5");
        if exact_digits(value) == (halfway, point) && reads_back(&candidate, point, value) {
            return candidate;
        }
    }
    digits
}

/// All decimal digits of `value` (no trailing zeros) and the position of
/// the point, as in [`ShortestDecimal`]. Rust writes a double exactly
/// when asked for enough digits; 800 is more than the 767 significant
/// digits the smallest double needs.
fn exact_digits(value: f64) -> (String, i64) {
    let scientific = format!("{value:.800e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0').to_owned();
    (digits, exponent.parse::<i64>().unwrap_or(0) + 1)
}

/// Whether `0.digits × 10^point` reads back as `value`.
fn reads_back(digits: &str, point: i64, value: f64) -> bool {
    format!("0.{digits}e{point}").parse::<f64>() == Ok(value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_follows_rfc_8785_examples() {
        // RFC 8785 section 3.2.2.3 and appendix B.
        let numbers = json!([333_333_333.333_333_3, 1e30, 4.5, 0.002, 1e-27, 0, -0.0]);
        assert_eq!(
            canonical_json(&numbers),
            "[333333333.3333333,1e+30,4.5,0.002,1e-27,0,0]"
        );
        let object = json!({"b": 1, "a": {"d": true, "c": null}, "\u{20ac}": "\u{1f600}\n"});
        assert_eq!(
            canonical_json(&object),
            "{\"a\":{\"c\":null,\"d\":true},\"b\":1,\"\u{20ac}\":\"\u{1f600}\\n\"}"
        );
    }

    #[test]
    fn names_sort_by_utf16_code_units() {
        // In UTF-16, U+1F600 starts with the surrogate 0xD83D, which is
        // smaller than 0xFB33, so it sorts first (in UTF-8 it sorts last).
        let object = json!({"\u{fb33}": 1, "\u{1f600}": 2});
        assert_eq!(canonical_json(&object), "{\"\u{1f600}\":2,\"\u{fb33}\":1}");
    }

    #[test]
    fn large_and_small_numbers_use_exponents_like_ecmascript() {
        assert_eq!(ecmascript_f64(1e21), "1e+21");
        assert_eq!(ecmascript_f64(1e20), "100000000000000000000");
        assert_eq!(ecmascript_f64(0.000_001), "0.000001");
        assert_eq!(ecmascript_f64(1e-7), "1e-7");
        assert_eq!(ecmascript_f64(-1.5e-9), "-1.5e-9");
    }

    /// Parses `text` as `serde_json` does for a received credential, then
    /// writes it in JCS form.
    fn canonical_text(text: &str) -> String {
        canonical_json(&serde_json::from_str::<Value>(text).unwrap())
    }

    #[test]
    fn decimals_read_back_as_javascript_reads_them() {
        // Without serde_json's `float_roundtrip` feature this parses one
        // ULP off and is written as -94.99557343857764.
        assert_eq!(canonical_text("-94.99557343857765"), "-94.99557343857765");
        assert_eq!(canonical_text("1.0"), "1");
        assert_eq!(canonical_text("-0.0"), "0");
    }

    #[test]
    fn integers_from_two_to_the_53_are_written_as_doubles() {
        // JavaScript keeps every number as a double: 2^53 + 1 becomes 2^53.
        assert_eq!(canonical_text("9007199254740992"), "9007199254740992");
        assert_eq!(canonical_text("9007199254740993"), "9007199254740992");
        assert_eq!(canonical_text("-9007199254740993"), "-9007199254740992");
        assert_eq!(
            canonical_text("295147905179352825856"),
            "295147905179352830000"
        );
    }

    #[test]
    fn control_characters_use_lowercase_unicode_escapes() {
        // RFC 8785 section 3.2.2.2: \u001f, not \u001F; DEL stays as is.
        assert_eq!(
            canonical_text("\"\\u001f\\u0008\\u007f\""),
            "\"\\u001f\\b\u{7f}\""
        );
    }

    #[test]
    fn extreme_doubles_survive_parsing() {
        assert_eq!(canonical_text("5e-324"), "5e-324");
        assert_eq!(canonical_text("-5e-324"), "-5e-324");
        assert_eq!(
            canonical_text("1.7976931348623157e308"),
            "1.7976931348623157e+308"
        );
    }

    /// RFC 8785 appendix B: the bits of a double and how JCS writes it.
    #[test]
    fn rfc_8785_appendix_b_numbers() {
        let cases: [(u64, &str); 22] = [
            (0x0000_0000_0000_0001, "5e-324"),
            (0x8000_0000_0000_0001, "-5e-324"),
            (0x7fef_ffff_ffff_ffff, "1.7976931348623157e+308"),
            (0xffef_ffff_ffff_ffff, "-1.7976931348623157e+308"),
            (0x4340_0000_0000_0000, "9007199254740992"),
            (0xc340_0000_0000_0000, "-9007199254740992"),
            (0x4430_0000_0000_0000, "295147905179352830000"),
            (0x44b5_2d02_c7e1_4af5, "9.999999999999997e+22"),
            (0x44b5_2d02_c7e1_4af6, "1e+23"),
            (0x44b5_2d02_c7e1_4af7, "1.0000000000000001e+23"),
            (0x444b_1ae4_d6e2_ef4e, "999999999999999700000"),
            (0x444b_1ae4_d6e2_ef4f, "999999999999999900000"),
            (0x444b_1ae4_d6e2_ef50, "1e+21"),
            (0x3eb0_c6f7_a0b5_ed8c, "9.999999999999997e-7"),
            (0x3eb0_c6f7_a0b5_ed8d, "0.000001"),
            (0x41b3_de43_5555_5553, "333333333.3333332"),
            (0x41b3_de43_5555_5554, "333333333.33333325"),
            (0x41b3_de43_5555_5555, "333333333.3333333"),
            (0x41b3_de43_5555_5556, "333333333.3333334"),
            (0x41b3_de43_5555_5557, "333333333.33333343"),
            (0xbecb_f647_612f_3696, "-0.0000033333333333333333"),
            (0x4314_3ff3_c1cb_0959, "1424953923781206.2"),
        ];
        for (bits, expected) in cases {
            let value = f64::from_bits(bits);
            assert_eq!(ecmascript_f64(value), expected, "bits {bits:#018x}");
            // The same number, read from its JCS text, gives the same text.
            assert_eq!(canonical_text(expected), expected);
        }
    }

    /// The legal person credential of the 0.8 spike and the digest the
    /// Gaia-X compliance service v2.16.0 computed for it.
    #[test]
    fn digest_matches_the_compliance_service() {
        let credential = json!({
            "@context": ["https://www.w3.org/ns/credentials/v2", "https://w3id.org/gaia-x/development#",
                {"vcard": "http://www.w3.org/2006/vcard/ns#", "schema": "https://schema.org/"}],
            "id": "https://participant.test/credentials/legal-person.json",
            "type": ["VerifiableCredential", "gx:LegalPerson"],
            "issuer": "did:web:participant.test",
            "validFrom": "2026-11-15T09:29:05.663+00:00",
            "validUntil": "2027-02-13T09:29:05.663+00:00",
            "credentialSubject": {
                "id": "https://participant.test/credentials/legal-person.json#cs",
                "schema:name": "Portitor Spike S.A.",
                "gx:registrationNumber": {"id": "https://participant.test/credentials/vatID.json#cs"},
                "gx:headquartersAddress": {"type": "gx:Address", "gx:countryCode": "LU",
                    "vcard:street-address": "1 Rue Test", "vcard:locality": "Luxembourg",
                    "vcard:postal-code": "1111"},
                "gx:legalAddress": {"type": "gx:Address", "gx:countryCode": "LU",
                    "vcard:street-address": "1 Rue Test", "vcard:locality": "Luxembourg",
                    "vcard:postal-code": "1111"}
            }
        });
        let Value::Object(credential) = credential else {
            unreachable!()
        };
        assert_eq!(digest_sri(&credential), SPIKE_LEGAL_PERSON_DIGEST);
    }

    const SPIKE_LEGAL_PERSON_DIGEST: &str =
        "sha256-c8092b21a491c0fd695905240697d0d09d5da244f07d14716eaa9450fbdf4dfa";

    #[test]
    fn scope_types_name_gaiax_types_without_the_prefix() {
        let types = vec![
            "VerifiableCredential".to_owned(),
            "gx:LegalPerson".to_owned(),
        ];
        let gaiax = vec![
            "https://www.w3.org/ns/credentials/v2".to_owned(),
            "https://w3id.org/gaia-x/development#".to_owned(),
        ];
        let other = vec!["https://www.w3.org/ns/credentials/v2".to_owned()];
        assert!(matches_scope_type(&types, &gaiax, "LegalPerson"));
        assert!(matches_scope_type(&types, &gaiax, "gx:LegalPerson"));
        assert!(!matches_scope_type(&types, &other, "LegalPerson"));
        assert!(!matches_scope_type(&types, &gaiax, "Person"));
    }
}
