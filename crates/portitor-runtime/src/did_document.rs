// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The DID document a Portitor component serves for itself (W3C
//! `did:web`, "Create"), shared by the identity component and the
//! issuer: the current P-256 key, and any retired public keys, in the
//! relationships the component needs, and one service entry
//! (`CredentialService`, DCP 5.2, or `IssuerService`, DCP "Issuer Service
//! Endpoint Discovery").
//!
//! A retired key is listed so that what it signed before a key change
//! keeps verifying (TD-05; EDC `IdentityHub` keeps a rotated key's
//! verification method until it is revoked). Only its public key is read
//! ([`read_retired_keys`]); it never signs.

use std::path::PathBuf;

use portitor_dcp::{
    DidDocument, Jwk, Purpose, Relationship, Service, SigningKey, VerificationMethod, did_web_url,
};
use serde_json::Value;

/// The DID Core context.
pub const DID_CONTEXT: &str = "https://www.w3.org/ns/did/v1";

/// The id of `key` in the DID document of `did`: `<did>#<thumbprint>`,
/// the key's JWK thumbprint (RFC 7638). The current key and each retired
/// key get their id this way, so a key keeps its id across a key change.
#[must_use]
pub fn key_id(did: &str, key: &Jwk) -> String {
    format!("{did}#{}", key.thumbprint())
}

/// What goes into a component's DID document.
#[derive(Debug, Clone, Copy)]
pub struct OwnDocument<'a> {
    /// The component's DID.
    pub did: &'a str,
    /// The id of its key in the document (`<did>#<thumbprint>`).
    pub kid: &'a str,
    /// Its key; only the public part is written.
    pub key: &'a SigningKey,
    /// Public keys that signed before a key change and no longer sign.
    /// Each gets its own verification method ([`key_id`]), in the same
    /// relationships as the current key, after it.
    pub retired_keys: &'a [Jwk],
    /// What the keys are used for: each purpose becomes a relationship
    /// that references the keys.
    pub purposes: &'a [Purpose],
    /// The service entry's id fragment, for example `credential-service`.
    pub service_fragment: &'a str,
    /// The service entry's type, for example `CredentialService`.
    pub service_type: &'a str,
    /// The service's URL.
    pub service_url: &'a str,
}

/// The DID document as JSON, with its `@context`.
#[must_use]
pub fn build(own: &OwnDocument<'_>) -> Value {
    // the current key first, then the retired keys in the order given
    let mut methods = vec![verification_method(own.did, own.kid, own.key.public_jwk())];
    for retired in own.retired_keys {
        let id = key_id(own.did, retired);
        methods.push(verification_method(own.did, &id, retired.clone()));
    }
    let mut references = Vec::new();
    for method in &methods {
        references.push(Relationship::Reference(method.id.clone()));
    }
    let reference = || references.clone();
    let has = |purpose: Purpose| own.purposes.contains(&purpose);
    let mut document = DidDocument {
        id: own.did.to_owned(),
        verification_method: methods.clone(),
        capability_invocation: Vec::new(),
        authentication: Vec::new(),
        assertion_method: Vec::new(),
        service: vec![Service {
            id: format!("{}#{}", own.did, own.service_fragment),
            kind: own.service_type.to_owned(),
            service_endpoint: Value::from(own.service_url),
        }],
    };
    if has(Purpose::CapabilityInvocation) {
        document.capability_invocation = reference();
    }
    if has(Purpose::Authentication) {
        document.authentication = reference();
    }
    if has(Purpose::AssertionMethod) {
        document.assertion_method = reference();
    }
    let mut json = serde_json::to_value(&document).unwrap_or_default();
    if let Value::Object(fields) = &mut json {
        fields.insert(
            "@context".to_owned(),
            serde_json::json!([DID_CONTEXT, portitor_dcp::message::CONTEXT]),
        );
    }
    // RFC 7517 section 4.4: the key is for ES256 only. Optional in JWK and
    // ignored by DCP; Gaia-X services need it to import the key (ADR 0021).
    for method in &methods {
        set_key_member(&mut json, &method.id, "alg", "ES256");
    }
    json
}

/// A `JsonWebKey2020` verification method of `did` for `key`.
fn verification_method(did: &str, id: &str, key: Jwk) -> VerificationMethod {
    VerificationMethod {
        id: id.to_owned(),
        kind: "JsonWebKey2020".to_owned(),
        controller: did.to_owned(),
        public_key_jwk: key,
    }
}

/// Reads the retired public keys named in the setting `setting` (for
/// example `PORTITOR_IDENTITY_RETIRED_KEY_FILES`): one PEM file per key,
/// each holding only a P-256 public key (`-----BEGIN PUBLIC KEY-----`,
/// see [`portitor_dcp::x509::public_key_pem`]).
///
/// # Errors
/// A file cannot be read; holds a private key or no P-256 public key;
/// holds the current key `current`; or holds a key already listed.
pub fn read_retired_keys(
    setting: &str,
    files: &[PathBuf],
    current: &Jwk,
) -> Result<Vec<Jwk>, String> {
    let mut keys: Vec<Jwk> = Vec::new();
    for file in files {
        let name = file.display();
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("{setting}: cannot read {name}: {e}"))?;
        let key = portitor_dcp::x509::public_key_pem(&text)
            .map_err(|e| format!("{setting}: {name}: {e}"))?;
        if key == *current {
            return Err(format!(
                "{setting}: {name} is the current key; list only keys that no longer sign"
            ));
        }
        if keys.contains(&key) {
            return Err(format!("{setting}: {name}: this key is listed twice"));
        }
        keys.push(key);
    }
    Ok(keys)
}

/// Adds `x5u` (RFC 7517 section 4.6), the URL of the key's X.509
/// certificate chain, to the JWK of verification method `kid` in a
/// document made by [`build`] (ADR 0021).
pub fn add_x5u(document: &mut Value, kid: &str, url: &str) {
    set_key_member(document, kid, "x5u", url);
}

/// Sets `name` to `value` in the `publicKeyJwk` of verification method
/// `kid`. Nothing happens if there is no such method.
fn set_key_member(document: &mut Value, kid: &str, name: &str, value: &str) {
    let Some(Value::Array(methods)) = document.get_mut("verificationMethod") else {
        return;
    };
    for method in methods {
        if method.get("id").and_then(Value::as_str) != Some(kid) {
            continue;
        }
        if let Some(Value::Object(jwk)) = method.get_mut("publicKeyJwk") {
            jwk.insert(name.to_owned(), Value::from(value));
        }
    }
}

/// The HTTP path where `did:web` says the document of `did` is:
/// `/.well-known/did.json`, or `/<path>/did.json` for a DID with a path.
///
/// # Errors
/// `did` is not a valid `did:web` DID.
pub fn path(did: &str) -> Result<String, String> {
    let url = did_web_url(did).map_err(|e| e.to_string())?;
    let after_scheme = url.trim_start_matches("https://");
    match after_scheme.find('/') {
        Some(slash) => Ok(after_scheme[slash..].to_owned()),
        None => Err(format!("no path in {url}")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths() {
        assert_eq!(
            path("did:web:localhost%3A8090").unwrap(),
            "/.well-known/did.json"
        );
        assert_eq!(
            path("did:web:identity.example:participants:alice").unwrap(),
            "/participants/alice/did.json"
        );
        assert!(path("did:key:z6Mk").is_err());
    }

    #[test]
    fn only_the_named_relationships_reference_the_key() {
        let key = SigningKey::generate().unwrap();
        let did = "did:web:issuer.example";
        let kid = format!("{did}#key-1");
        let json = build(&OwnDocument {
            did,
            kid: &kid,
            key: &key,
            retired_keys: &[],
            purposes: &[Purpose::AssertionMethod, Purpose::CapabilityInvocation],
            service_fragment: "issuer-service",
            service_type: "IssuerService",
            service_url: "https://issuer.example/issuer",
        });
        assert_eq!(json["@context"][0], DID_CONTEXT);
        assert_eq!(json["verificationMethod"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["verificationMethod"][0]["publicKeyJwk"]["alg"],
            "ES256"
        );
        let mut with_chain = json.clone();
        add_x5u(&mut with_chain, &kid, "https://issuer.example/chain.pem");
        assert_eq!(
            with_chain["verificationMethod"][0]["publicKeyJwk"]["x5u"],
            "https://issuer.example/chain.pem"
        );
        let document: DidDocument = serde_json::from_value(json).unwrap();
        assert_eq!(
            document.verification_method[0].public_key_jwk,
            key.public_jwk()
        );
        for purpose in [Purpose::AssertionMethod, Purpose::CapabilityInvocation] {
            assert_eq!(document.key_for(Some(&kid), purpose).unwrap().id, kid);
        }
        assert!(
            document
                .key_for(Some(&kid), Purpose::Authentication)
                .is_err()
        );
        assert_eq!(
            document.service[0].id,
            "did:web:issuer.example#issuer-service"
        );
        assert_eq!(
            document.service_url("IssuerService"),
            Some("https://issuer.example/issuer")
        );
    }

    /// A key signed with before a key change still verifies through its
    /// retired entry; the chain (`x5u`) stays on the current key only.
    #[test]
    fn retired_keys_have_the_same_relationships_and_never_the_chain() {
        let old = SigningKey::generate().unwrap();
        let current = SigningKey::generate().unwrap();
        let did = "did:web:identity.example";
        let kid = key_id(did, &current.public_jwk());
        let old_kid = key_id(did, &old.public_jwk());
        let purposes = [Purpose::Authentication, Purpose::CapabilityInvocation];
        let mut json = build(&OwnDocument {
            did,
            kid: &kid,
            key: &current,
            retired_keys: &[old.public_jwk()],
            purposes: &purposes,
            service_fragment: "credential-service",
            service_type: "CredentialService",
            service_url: "https://identity.example/cs",
        });
        add_x5u(&mut json, &kid, "https://identity.example/chain.pem");
        let methods = json["verificationMethod"].as_array().unwrap();
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0]["id"], kid.as_str());
        assert_eq!(methods[1]["id"], old_kid.as_str());
        assert_eq!(methods[1]["publicKeyJwk"]["alg"], "ES256");
        assert!(methods[1]["publicKeyJwk"].get("x5u").is_none());
        assert_eq!(json["authentication"], json!([kid, old_kid]));
        assert!(json.get("assertionMethod").is_none());

        let document: DidDocument = serde_json::from_value(json).unwrap();
        for purpose in purposes {
            let listed = document.key_for(Some(&old_kid), purpose).unwrap();
            assert_eq!(listed.public_key_jwk, old.public_jwk());
            assert_eq!(document.key_for(Some(&kid), purpose).unwrap().id, kid);
        }
        // with two keys, a token without `kid` names no key
        assert!(document.key_for(None, Purpose::Authentication).is_err());
    }

    #[test]
    fn retired_key_files_hold_one_public_key_each() {
        let dir = std::env::temp_dir().join(format!(
            "portitor-retired-{}",
            crate::system::new_uuid().trim_start_matches("urn:uuid:")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let current = SigningKey::generate().unwrap();
        let old = SigningKey::generate().unwrap();
        let older = SigningKey::generate().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            path
        };
        let public_pem =
            |key: &SigningKey| portitor_dcp::x509::p256_public_key_pem(&key.public_jwk()).unwrap();
        let old_file = write("old.pem", &public_pem(&old));
        let older_file = write("older.pem", &public_pem(&older));
        let current_file = write("current.pem", &public_pem(&current));
        // refused before its content is read
        let private_file = write(
            "private.pem",
            "-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n",
        );
        let setting = "PORTITOR_IDENTITY_RETIRED_KEY_FILES";
        let read = |files: &[PathBuf]| read_retired_keys(setting, files, &current.public_jwk());

        assert_eq!(read(&[]).unwrap(), []);
        assert_eq!(
            read(&[old_file.clone(), older_file.clone()]).unwrap(),
            [old.public_jwk(), older.public_jwk()]
        );
        let refusals = [
            (vec![current_file], "is the current key"),
            (vec![old_file.clone(), old_file.clone()], "listed twice"),
            (vec![private_file], "a private key"),
            (vec![dir.join("missing.pem")], "cannot read"),
        ];
        for (files, reason) in refusals {
            let error = read(&files).unwrap_err();
            assert!(error.starts_with(setting), "{error}");
            assert!(error.contains(reason), "{error}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
