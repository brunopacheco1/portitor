// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The identity component's readiness checks (ADR 0023, section 1),
//! served on the management port by `portitor_runtime::health`. Only its
//! own dependencies, never an issuer or a verifier:
//!
//! - `database`: the `SQLite` file answers;
//! - `key`: the participant's key is loaded and signs;
//! - `did-document`: the DID document it serves names the participant
//!   and lists that key for signing tokens (`capabilityInvocation`).

use std::path::PathBuf;
use std::sync::Arc;

use portitor_dcp::{DidDocument, Purpose};
use portitor_runtime::health::{HealthChecks, blocking, key_signs, sqlite_file_answers};

use crate::Identity;

/// The identity component's checks, for `identity` with its `SQLite` file
/// `db_file` (`PORTITOR_DB_FILE`).
#[must_use]
pub fn readiness_checks(identity: &Arc<Identity>, db_file: PathBuf) -> HealthChecks {
    let mut checks = HealthChecks::new();
    checks.add("database", blocking(move || sqlite_file_answers(&db_file)));
    let for_key = Arc::clone(identity);
    checks.add("key", blocking(move || key_signs(&for_key.key)));
    let for_document = Arc::clone(identity);
    checks.add(
        "did-document",
        blocking(move || document_lists_the_key(&for_document)),
    );
    checks
}

/// Check: the DID document of `identity` names the participant and lists
/// its loaded key for `capabilityInvocation` ([`document_lists`]). The
/// document is built from the key at start and the key never changes, so
/// this (and the `key` check) cannot fail while the component serves;
/// they guard against a change in how the document is built.
fn document_lists_the_key(identity: &Identity) -> Result<(), String> {
    document_lists(
        &identity.document,
        &identity.participant_id,
        &identity.kid,
        &identity.key.public_jwk(),
    )
}

/// `document` is a DID document for `did` that lists the key `public`
/// as `kid` for `capabilityInvocation`.
fn document_lists(
    document: &serde_json::Value,
    did: &str,
    kid: &str,
    public: &portitor_dcp::Jwk,
) -> Result<(), String> {
    let document: DidDocument = serde_json::from_value(document.clone())
        .map_err(|e| format!("the DID document is not valid: {e}"))?;
    if document.id != did {
        return Err("the DID document names another DID".to_owned());
    }
    let listed = document
        .key_for(Some(kid), Purpose::CapabilityInvocation)
        .map_err(|e| format!("the DID document does not list the key: {e}"))?;
    if listed.public_key_jwk == *public {
        Ok(())
    } else {
        Err("the DID document lists another key than the one loaded".to_owned())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_dcp::SigningKey;
    use serde_json::json;
    use tower::ServiceExt as _;

    const DID: &str = "did:web:holder.example";
    const KID: &str = "did:web:holder.example#key-1";

    fn document(did: &str, key: &SigningKey) -> serde_json::Value {
        json!({
            "@context": ["https://www.w3.org/ns/did/v1"],
            "id": did,
            "verificationMethod": [{"id": KID, "type": "JsonWebKey2020",
                                    "controller": did, "publicKeyJwk": key.public_jwk()}],
            "capabilityInvocation": [KID]
        })
    }

    #[tokio::test]
    async fn the_document_must_name_us_and_list_the_loaded_key() {
        let key = SigningKey::generate().unwrap();
        let other = SigningKey::generate().unwrap();
        let public = key.public_jwk();
        assert_eq!(
            document_lists(&document(DID, &key), DID, KID, &public),
            Ok(())
        );
        let err = document_lists(&document("did:web:other.example", &key), DID, KID, &public);
        assert_eq!(err.unwrap_err(), "the DID document names another DID");
        let err = document_lists(&document(DID, &other), DID, KID, &public).unwrap_err();
        assert!(err.contains("another key"), "{err}");
        let err = document_lists(
            &document(DID, &key),
            DID,
            "did:web:holder.example#nope",
            &public,
        )
        .unwrap_err();
        assert!(err.contains("does not list the key"), "{err}");
        let err = document_lists(&json!({"id": 1}), DID, KID, &public).unwrap_err();
        assert!(err.contains("not valid"), "{err}");
        // as a readiness check: 503
        let mut checks = HealthChecks::new();
        let bad = document(DID, &other);
        checks.add(
            "did-document",
            blocking(move || document_lists(&bad, DID, KID, &public)),
        );
        let app = portitor_runtime::health::router(Arc::new(checks));
        let request = axum::http::Request::get(portitor_runtime::health::READINESS_PATH)
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
