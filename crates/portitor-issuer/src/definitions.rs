// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the issuer can issue: the credential definitions file
//! (`PORTITOR_ISSUER_CREDENTIALS_FILE`, ADR 0018, "Credential
//! definitions"), checked at start, and the issuer metadata built from it
//! (DCP v1.0.1, section 6.7).
//!
//! Section 6.7 ends with: "Every `CredentialObject` in the
//! `credentialsSupported` array MUST contain all OPTIONAL properties
//! defined in" the `CredentialObject` section, and
//! `issuer-metadata-schema.json` requires `credentialType`,
//! `credentialSchema`, `offerReason`, `bindingMethods` and `profile`.
//! [`metadata`] writes all five (TD-33).

use std::collections::HashSet;

use portitor_dcp::credential::Profile;
use portitor_dcp::message::{
    Context, CredentialObject, CredentialObjectType, IssuerMetadata, IssuerMetadataType,
};
use portitor_runtime::http::outbound_url_allowed;
use serde::Deserialize;

/// The longest a credential may be valid: ten years. A longer validity is
/// most likely a typing error.
pub const MAX_VALIDITY_DAYS: u64 = 3_650;

/// How an issued credential is bound to its holder: its subject `id` is
/// the holder's `did:web` DID.
pub const BINDING_METHOD: &str = "did:web";

/// The `offerReason` of every credential in the issuer metadata.
///
/// The metadata says what can be requested; it is not an offer, so no
/// reason really applies. DCP 6.7 and `issuer-metadata-schema.json`
/// still require the property. The values DCP names are `reissue` and
/// `proof-key-revocation` (section 6.6.2, `CredentialObject`: "Valid values
/// MAY include"). Portitor sends `reissue`, as the specification's
/// metadata example does and as Eclipse EDC `IdentityHub` 1.0.0-RC7 does
/// (`DcpIssuerMetadataServiceImpl`). A credential offer carries the
/// reason the operator gave instead (`management.rs`, `offered_objects`).
pub const METADATA_OFFER_REASON: &str = "reissue";

/// One entry of the definitions file, as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DefinitionEntry {
    id: String,
    credential_type: String,
    profile: String,
    validity_days: u64,
    /// Required. Read as optional so that [`check`] can name the entry
    /// that lacks it; serde's own error would give only a line number.
    #[serde(default)]
    credential_schema: Option<String>,
    #[serde(default)]
    claims: Vec<String>,
}

/// A credential the issuer can issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// Its id in the issuer metadata; a holder asks for it by this id.
    pub id: String,
    /// The credential type, for example `MembershipCredential`.
    pub credential_type: String,
    /// The profile it is signed in.
    pub profile: Profile,
    /// How long an issued credential is valid, in days.
    pub validity_days: u64,
    /// The URL of the schema of the credential's `credentialSubject`
    /// (DCP 6.6.2, `credentialSchema`). Published in the issuer metadata
    /// only: Portitor does not fetch it and does not check credentials
    /// against it (TD-14).
    pub credential_schema: String,
    /// The participant claims copied into `credentialSubject`.
    pub claims: Vec<String>,
}

/// Why the definitions file cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("credential definitions: {0}")]
pub struct DefinitionError(pub String);

/// Reads and checks a definitions file: a JSON array of definitions.
/// Unknown profile, duplicate id, empty id or type, a validity of zero
/// or more than [`MAX_VALIDITY_DAYS`], a missing `credentialSchema` or one
/// that is not an absolute `http(s)` URL, a claim named `id`, or an
/// empty list stop the issuer.
///
/// # Errors
/// The text is not a valid definitions file.
pub fn parse(text: &str) -> Result<Vec<Definition>, DefinitionError> {
    let entries: Vec<DefinitionEntry> =
        serde_json::from_str(text).map_err(|e| DefinitionError(e.to_string()))?;
    if entries.is_empty() {
        return Err(DefinitionError("no credential is defined".to_owned()));
    }
    let mut seen = HashSet::new();
    let mut definitions = Vec::new();
    for entry in entries {
        definitions.push(check(entry, &mut seen)?);
    }
    Ok(definitions)
}

/// One entry, checked; `seen` holds the ids before it.
fn check(
    entry: DefinitionEntry,
    seen: &mut HashSet<String>,
) -> Result<Definition, DefinitionError> {
    let fail = |problem: &str| Err(DefinitionError(format!("{:?}: {problem}", entry.id)));
    if entry.id.trim().is_empty() {
        return fail("empty id");
    }
    if !seen.insert(entry.id.clone()) {
        return fail("duplicate id");
    }
    if entry.credential_type.trim().is_empty() {
        return fail("empty credentialType");
    }
    let Some(profile) = Profile::from_alias(&entry.profile) else {
        return fail("unknown profile (vc11-sl2021/jwt or vc20-bssl/jwt)");
    };
    if entry.validity_days == 0 || entry.validity_days > MAX_VALIDITY_DAYS {
        return fail("validityDays must be from 1 to 3650");
    }
    let Some(credential_schema) = entry.credential_schema.clone() else {
        return fail("credentialSchema is required (the URL of the credential's schema)");
    };
    if !is_schema_url(&credential_schema) {
        return fail("credentialSchema must be an absolute http:// or https:// URL");
    }
    if entry
        .claims
        .iter()
        .any(|claim| claim == "id" || claim.is_empty())
    {
        return fail("a claim may not be empty or named id");
    }
    Ok(Definition {
        id: entry.id,
        credential_type: entry.credential_type,
        profile,
        validity_days: entry.validity_days,
        credential_schema,
        claims: entry.claims,
    })
}

/// Whether `url` can be published as a `credentialSchema`: an absolute
/// `https://` or `http://` URL with a host, without user credentials and
/// without spaces. It is the rule for every URL Portitor calls
/// ([`outbound_url_allowed`]), with plain HTTP allowed, because the issuer
/// only publishes this URL and never calls it.
fn is_schema_url(url: &str) -> bool {
    let has_space = url.chars().any(char::is_whitespace);
    !has_space && outbound_url_allowed(url, true)
}

/// The issuer metadata (DCP 6.7): one `CredentialObject` per definition,
/// whose id is the definition's, so it stays the same across restarts.
/// Each object has every property `issuer-metadata-schema.json` requires.
/// `issuancePolicy` is left out: the schema does not require it, and
/// Portitor asks for no presentation before it issues (TD-14).
#[must_use]
pub fn metadata(issuer: &str, definitions: &[Definition]) -> IssuerMetadata {
    let mut supported = Vec::new();
    for definition in definitions {
        supported.push(CredentialObject {
            id: definition.id.clone(),
            kind: Some(CredentialObjectType),
            credential_type: Some(definition.credential_type.clone()),
            credential_schema: Some(definition.credential_schema.clone()),
            offer_reason: Some(METADATA_OFFER_REASON.to_owned()),
            binding_methods: vec![BINDING_METHOD.to_owned()],
            profile: Some(definition.profile.alias().to_owned()),
            issuance_policy: None,
        });
    }
    IssuerMetadata {
        context: Context::default(),
        kind: IssuerMetadataType,
        issuer: issuer.to_owned(),
        credentials_supported: supported,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"[
      {"id": "membership-vc11", "credentialType": "MembershipCredential",
       "profile": "vc11-sl2021/jwt", "validityDays": 365,
       "credentialSchema": "https://example.com/membership-credential-schema.json",
       "claims": ["role"]},
      {"id": "membership-vc20", "credentialType": "MembershipCredential",
       "profile": "vc20-bssl/jwt", "validityDays": 30,
       "credentialSchema": "https://example.com/membership-credential-schema.json"}
    ]"#;

    #[test]
    fn the_adr_example_parses() {
        let definitions = parse(EXAMPLE).unwrap();
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].profile, Profile::Vc11);
        assert_eq!(definitions[0].claims, ["role"]);
        assert_eq!(
            definitions[0].credential_schema,
            "https://example.com/membership-credential-schema.json"
        );
        assert!(definitions[1].claims.is_empty());
    }

    #[test]
    fn mistakes_stop_the_issuer() {
        let entry = |field: &str, value: &str| {
            let mut json: serde_json::Value = serde_json::json!({
                "id": "m", "credentialType": "MembershipCredential",
                "profile": "vc20-bssl/jwt", "validityDays": 1,
                "credentialSchema": "https://example.com/m.json"
            });
            json[field] = serde_json::from_str(value).unwrap();
            format!("[{json}]")
        };
        // the entry itself is fine: each change below is what stops it
        assert!(parse(&entry("validityDays", "2")).is_ok());
        for (field, value) in [
            ("profile", r#""vc30/jwt""#),
            ("credentialType", r#""""#),
            ("id", r#"" ""#),
            ("validityDays", "0"),
            ("validityDays", "3651"),
            ("claims", r#"["id"]"#),
            ("unknown", "1"),
            ("credentialSchema", "null"),
            ("credentialSchema", r#""""#),
            ("credentialSchema", r#""membership-credential-schema.json""#),
            ("credentialSchema", r#""/schemas/membership.json""#),
            ("credentialSchema", r#""ftp://example.com/m.json""#),
            ("credentialSchema", r#""https://""#),
            ("credentialSchema", r#""https://user@example.com/m.json""#),
            ("credentialSchema", r#""https://example.com/m .json""#),
            ("credentialSchema", "1"),
        ] {
            assert!(parse(&entry(field, value)).is_err(), "{field} = {value}");
        }
        let duplicate = r#"[
          {"id": "m", "credentialType": "A", "profile": "vc20-bssl/jwt", "validityDays": 1,
           "credentialSchema": "https://example.com/a.json"},
          {"id": "m", "credentialType": "B", "profile": "vc11-sl2021/jwt", "validityDays": 1,
           "credentialSchema": "https://example.com/b.json"}
        ]"#;
        assert!(parse(duplicate).unwrap_err().0.contains("duplicate"));
        assert!(parse("[]").is_err());
        assert!(parse("{}").is_err());
    }

    /// A definitions file written for 0.11 has no `credentialSchema`: the
    /// message names the entry and the field.
    #[test]
    fn an_entry_without_credential_schema_is_named() {
        let before_0_12 = r#"[
          {"id": "membership-vc20", "credentialType": "MembershipCredential",
           "profile": "vc20-bssl/jwt", "validityDays": 365}
        ]"#;
        let error = parse(before_0_12).unwrap_err().to_string();
        assert_eq!(
            error,
            "credential definitions: \"membership-vc20\": credentialSchema is required \
             (the URL of the credential's schema)"
        );
        let plain_http = before_0_12.replace(
            r#""validityDays": 365"#,
            r#""validityDays": 365, "credentialSchema": "http://schemas.example/m.json""#,
        );
        assert!(parse(&plain_http).is_ok());
    }

    /// A definitions file of one entry whose `credentialSchema` is the
    /// JSON `value`.
    fn file_with_schema(value: &str) -> String {
        format!(
            r#"[{{"id": "m", "credentialType": "MembershipCredential",
                 "profile": "vc20-bssl/jwt", "validityDays": 1, "credentialSchema": {value}}}]"#
        )
    }

    /// `credentialSchema` is one URL as text (DCP 6.6.2): any other JSON
    /// type, a list of URLs included, stops the issuer, and so does a URL
    /// with a space around it.
    #[test]
    fn a_credential_schema_that_is_not_one_url_as_text_is_refused() {
        assert!(parse(&file_with_schema(r#""https://example.com/m.json""#)).is_ok());
        for value in [
            "true",
            "1.5",
            r#"["https://example.com/m.json"]"#,
            r#"{"id": "https://example.com/m.json", "type": "JsonSchema"}"#,
            r#"" https://example.com/m.json""#,
            r#""https://example.com/m.json ""#,
            r#""https:example.com/m.json""#,
            r#""//example.com/m.json""#,
            r#""urn:example:schema""#,
        ] {
            assert!(parse(&file_with_schema(value)).is_err(), "{value}");
        }
    }

    /// A field written twice in one entry is refused by name: one of the
    /// two values would be dropped unseen.
    #[test]
    fn a_field_given_twice_in_an_entry_is_refused() {
        let twice = file_with_schema(
            r#""https://example.com/a.json", "credentialSchema": "https://example.com/b.json""#,
        );
        let error = parse(&twice).unwrap_err().to_string();
        assert!(
            error.contains("duplicate field `credentialSchema`"),
            "{error}"
        );
    }

    /// Each entry is checked on its own: one entry without
    /// `credentialSchema` among good ones stops the issuer, and the
    /// message names that entry.
    #[test]
    fn one_entry_without_credential_schema_among_good_ones_is_named() {
        let mixed = r#"[
          {"id": "first", "credentialType": "A", "profile": "vc20-bssl/jwt", "validityDays": 1,
           "credentialSchema": "https://example.com/a.json"},
          {"id": "second", "credentialType": "B", "profile": "vc20-bssl/jwt", "validityDays": 1},
          {"id": "third", "credentialType": "C", "profile": "vc20-bssl/jwt", "validityDays": 1,
           "credentialSchema": "https://example.com/c.json"}
        ]"#;
        let error = parse(mixed).unwrap_err().to_string();
        assert!(
            error.contains("\"second\": credentialSchema is required"),
            "{error}"
        );
    }

    #[test]
    fn metadata_lists_every_definition_fully() {
        let metadata = metadata("did:web:issuer.example", &parse(EXAMPLE).unwrap());
        let json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(json["type"], "IssuerMetadata");
        assert_eq!(json["issuer"], "did:web:issuer.example");
        let first = &json["credentialsSupported"][0];
        assert_eq!(first["id"], "membership-vc11");
        assert_eq!(first["type"], "CredentialObject");
        assert_eq!(first["credentialType"], "MembershipCredential");
        assert_eq!(first["profile"], "vc11-sl2021/jwt");
        assert_eq!(first["bindingMethods"], serde_json::json!(["did:web"]));
        assert_eq!(
            first["credentialSchema"],
            "https://example.com/membership-credential-schema.json"
        );
        assert_eq!(first["offerReason"], "reissue");
        assert!(first.get("issuancePolicy").is_none());
    }
}
