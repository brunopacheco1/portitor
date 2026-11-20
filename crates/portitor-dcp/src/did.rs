// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! DID documents (the parts DCP needs) and the did:web URL rule.

use serde::{Deserialize, Serialize};

use crate::{Error, Jwk};

/// A DID document verification method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationMethod {
    /// Full id, for example `did:web:example.com#key-1`.
    pub id: String,
    /// Method type, for example `JsonWebKey2020`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Controller DID.
    pub controller: String,
    /// Public key.
    pub public_key_jwk: Jwk,
}

/// An entry of a verification relationship: a reference or an embedded method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Relationship {
    /// Reference to a verification method id.
    Reference(String),
    /// Embedded verification method.
    Embedded(VerificationMethod),
}

impl Relationship {
    fn id(&self) -> &str {
        match self {
            Self::Reference(id) => id,
            Self::Embedded(vm) => &vm.id,
        }
    }
}

/// A service a DID document lists, such as the participant's Credential
/// Service (DCP presentation protocol, section 5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// Service id, for example `did:web:example.com#credential-service`.
    pub id: String,
    /// Service type, for example `CredentialService`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Where the service is. DID Core allows a string, a map or a list;
    /// DCP uses a URL string.
    pub service_endpoint: serde_json::Value,
}

/// What a key is used for: each use has its own verification
/// relationship in the DID document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Signing self-issued ID tokens (DCP base protocol, section 4.3).
    CapabilityInvocation,
    /// Signing presentations (DCP presentation protocol, section 5.4.3).
    Authentication,
    /// Signing credentials (the issuer's key; VC Data Model).
    AssertionMethod,
}

impl Purpose {
    /// The error text when a key lacks this relationship.
    const fn missing_message(self) -> &'static str {
        match self {
            Self::CapabilityInvocation => "key lacks capabilityInvocation",
            Self::Authentication => "key lacks authentication",
            Self::AssertionMethod => "key lacks assertionMethod",
        }
    }
}

/// The parts of a DID document DCP needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DidDocument {
    /// The DID.
    pub id: String,
    /// Verification methods.
    #[serde(default)]
    pub verification_method: Vec<VerificationMethod>,
    /// Keys allowed to sign self-issued ID tokens.
    #[serde(default)]
    pub capability_invocation: Vec<Relationship>,
    /// Keys allowed to sign presentations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authentication: Vec<Relationship>,
    /// Keys allowed to sign credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertion_method: Vec<Relationship>,
    /// Services, such as the Credential Service.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service: Vec<Service>,
}

impl DidDocument {
    /// The URL of the first `CredentialService` listed, if any.
    #[must_use]
    pub fn credential_service(&self) -> Option<&str> {
        self.service_url("CredentialService")
    }

    /// The URL of the first service of type `kind` (for example
    /// `IssuerService`, DCP issuance protocol, section 6.1), if any.
    #[must_use]
    pub fn service_url(&self, kind: &str) -> Option<&str> {
        self.service
            .iter()
            .find(|s| s.kind == kind)
            .and_then(|s| s.service_endpoint.as_str())
    }

    fn relationship(&self, purpose: Purpose) -> &[Relationship] {
        match purpose {
            Purpose::CapabilityInvocation => &self.capability_invocation,
            Purpose::Authentication => &self.authentication,
            Purpose::AssertionMethod => &self.assertion_method,
        }
    }

    /// The key to check a signature with: the one named by `kid` (a full
    /// id, or `#fragment` relative to this DID), or, without `kid`, the
    /// only verification method. The key must have the verification
    /// relationship of `purpose`, by reference or embedded there.
    ///
    /// # Errors
    /// No such key, `kid` is missing while there are several keys, or the
    /// key lacks the relationship.
    pub fn key_for(
        &self,
        kid: Option<&str>,
        purpose: Purpose,
    ) -> Result<&VerificationMethod, Error> {
        let allowed = self.relationship(purpose);
        let vm = match kid {
            Some(kid) => self
                .verification_method
                .iter()
                .chain(embedded_methods(allowed))
                .find(|vm| self.names_kid(vm, kid))
                .ok_or(Error::Invalid("kid not found"))?,
            None => match self.verification_method.as_slice() {
                [only] => only,
                [] => return Err(Error::Invalid("no verification method")),
                _ => return Err(Error::Invalid("no kid and several verification methods")),
            },
        };
        if allowed.iter().any(|r| self.same_key(r.id(), &vm.id)) {
            Ok(vm)
        } else {
            Err(Error::Invalid(purpose.missing_message()))
        }
    }

    /// Whether `kid` names `vm`: either its full id
    /// (`did:web:example.com#key-1`), or only the `#key-1` part, which
    /// a JWS may use for a key of the signer's own DID document.
    fn names_kid(&self, vm: &VerificationMethod, kid: &str) -> bool {
        self.same_key(&vm.id, kid)
    }

    /// Whether two key ids name the same key of this document. Each may
    /// be written in full or as `#fragment` only: inside a DID document
    /// a relative id is read against the document's own DID (W3C DID
    /// Core, "Relative DID URLs").
    fn same_key(&self, one: &str, other: &str) -> bool {
        self.fragment_of_own(one) == self.fragment_of_own(other)
    }

    /// The `#fragment` of a key id of this DID; an id of another DID, or
    /// one without a fragment, is returned whole and so equals only
    /// itself.
    fn fragment_of_own<'a>(&self, id: &'a str) -> &'a str {
        match id.strip_prefix(self.id.as_str()) {
            Some(fragment) if fragment.starts_with('#') => fragment,
            _ => id,
        }
    }
}

/// The verification methods written in full inside a relationship list,
/// rather than named by reference.
fn embedded_methods(relationships: &[Relationship]) -> impl Iterator<Item = &VerificationMethod> {
    relationships.iter().filter_map(|r| match r {
        Relationship::Embedded(vm) => Some(vm),
        Relationship::Reference(_) => None,
    })
}

/// Returns the HTTPS URL of the DID document for a `did:web` DID.
///
/// # Errors
/// Fails when the DID is not `did:web` or has an empty host.
pub fn did_web_url(did: &str) -> Result<String, Error> {
    let rest = did
        .strip_prefix("did:web:")
        .ok_or(Error::InvalidDid("not did:web"))?;
    let mut parts = rest.split(':');
    let host = parts
        .next()
        .filter(|h| !h.is_empty())
        .ok_or(Error::InvalidDid("empty host"))?;
    let host = host_with_port(host);
    if host.contains('/') {
        return Err(Error::InvalidDid("host contains '/'"));
    }
    let path: Vec<&str> = parts.collect();
    if path.iter().any(|p| p.is_empty() || p.contains('/')) {
        return Err(Error::InvalidDid("bad path segment"));
    }
    if path.is_empty() {
        Ok(format!("https://{host}/.well-known/did.json"))
    } else {
        Ok(format!("https://{host}/{}/did.json", path.join("/")))
    }
}

/// The host of a `did:web` as it is in a URL: the colon before a port is
/// written `%3A` in the DID (did:web method, "Method-specific
/// identifier"), and percent escapes may be in either letter case
/// (RFC 3986, section 2.1), so `%3a` is read the same way.
fn host_with_port(host_as_written: &str) -> String {
    host_as_written.replace("%3A", ":").replace("%3a", ":")
}

/// Whether two texts name the same DID.
///
/// They do when they are equal as written, or when both are `did:web`
/// DIDs that differ only in the letter case of the escaped colon before
/// the port (`%3A` or `%3a`): [`did_web_url`] reads both spellings and
/// resolves them to the same document. Nothing else is normalised: the
/// letter case of the host, the path and any surrounding space count.
#[must_use]
pub fn same_did(one: &str, other: &str) -> bool {
    if one == other {
        return true;
    }
    match (did_web_host_and_path(one), did_web_host_and_path(other)) {
        (Some((one_host, one_path)), Some((other_host, other_path))) => {
            host_with_port(one_host) == host_with_port(other_host) && one_path == other_path
        }
        _ => false,
    }
}

/// The host of a `did:web` DID as written, and what follows it (the path
/// with its leading colon, or nothing). `None` for any other text.
fn did_web_host_and_path(did: &str) -> Option<(&str, &str)> {
    let rest = did.strip_prefix("did:web:")?;
    match rest.find(':') {
        Some(colon) => Some(rest.split_at(colon)),
        None => Some((rest, "")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The two spellings of the port's colon are one DID, as they are
    /// one document for [`did_web_url`]; everything else is as written.
    #[test]
    fn two_did_web_dids_are_the_same_whatever_the_case_of_the_port_escape() {
        let upper = "did:web:provider.example%3A8443:user:alice";
        let lower = "did:web:provider.example%3a8443:user:alice";
        assert!(same_did(upper, lower));
        assert!(same_did(lower, upper));
        assert!(same_did(upper, upper));
        assert_eq!(did_web_url(upper).unwrap(), did_web_url(lower).unwrap());
        // not a DID method we resolve: equal only as written
        assert!(same_did("did:key:z6Mk", "did:key:z6Mk"));
        assert!(!same_did("did:key:z6Mk%3A", "did:key:z6Mk%3a"));
        for (one, other) in [
            // a path segment is not a port
            (
                "did:web:provider.example%3A8443",
                "did:web:provider.example:8443",
            ),
            (
                "did:web:provider.example%3A8443",
                "did:web:provider.example%3A8444",
            ),
            ("did:web:provider.example", "did:web:Provider.example"),
            ("did:web:provider.example", "did:web:provider.example "),
            ("did:web:provider.example", " did:web:provider.example"),
            ("did:web:provider.example", "did:web:provider.example:x"),
            // an escape in the path is not touched
            (
                "did:web:provider.example:a%3Ab",
                "did:web:provider.example:a%3ab",
            ),
            ("did:web:provider.example", ""),
        ] {
            assert!(!same_did(one, other), "{one} and {other}");
            assert!(!same_did(other, one), "{other} and {one}");
        }
    }

    #[test]
    fn did_web_resolution_urls() {
        assert_eq!(
            did_web_url("did:web:example.com").unwrap(),
            "https://example.com/.well-known/did.json"
        );
        assert_eq!(
            did_web_url("did:web:example.com%3A8443:user:alice").unwrap(),
            "https://example.com:8443/user/alice/did.json"
        );
        assert!(did_web_url("did:key:z6Mk").is_err());
        assert!(did_web_url("did:web:").is_err());
        assert!(did_web_url("did:web:a.com::x").is_err());
    }

    /// The DID document of the Gaia-X Lab compliance service, as served
    /// at <https://compliance.lab.gaia-x.eu/v2/did.json> on 2026-11-15
    /// (modulus shortened): a `JsonWebKey` method with an RSA key and
    /// `kid`, `alg` and `x5u` members, and no `capabilityInvocation`.
    #[test]
    fn reads_the_gaiax_compliance_service_document() {
        let did = "did:web:compliance.lab.gaia-x.eu:v2";
        assert_eq!(
            did_web_url(did).unwrap(),
            "https://compliance.lab.gaia-x.eu/v2/did.json"
        );
        let doc: DidDocument = serde_json::from_value(serde_json::json!({
            "@context": ["https://www.w3.org/ns/did/v1", "https://w3id.org/security/jwk/v1"],
            "id": did,
            "verificationMethod": [{
                "id": "did:web:compliance.lab.gaia-x.eu:v2#X509-JWK",
                "type": "JsonWebKey",
                "controller": did,
                "publicKeyJwk": {"kty": "RSA", "n": "wCsJ0D9iTSwdZpvpieaIgzQqVWO3OtXvJ2lpGgtifcw",
                    "e": "AQAB", "kid": "X509-JWK", "alg": "PS256",
                    "x5u": "https://compliance.lab.gaia-x.eu/v2/.well-known/x509CertificateChain.pem"}
            }],
            "assertionMethod": ["did:web:compliance.lab.gaia-x.eu:v2#X509-JWK"]
        }))
        .unwrap();
        let key = doc
            .key_for(
                Some("did:web:compliance.lab.gaia-x.eu:v2#X509-JWK"),
                Purpose::AssertionMethod,
            )
            .unwrap();
        assert!(matches!(key.public_key_jwk, Jwk::RSA { .. }));
        assert!(
            doc.key_for(Some("#X509-JWK"), Purpose::CapabilityInvocation)
                .is_err()
        );
    }
}
