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

/// The DID document `GET /.well-known/did.json` serves (the same value
/// the handler sends) names the participant and lists the key it signs
/// with.
fn document_lists_the_key(identity: &Identity) -> Result<(), String> {
    let document: DidDocument = serde_json::from_value(identity.document.clone())
        .map_err(|e| format!("the DID document is not valid: {e}"))?;
    if document.id != identity.participant_id {
        return Err("the DID document names another DID".to_owned());
    }
    let listed = document
        .key_for(Some(&identity.kid), Purpose::CapabilityInvocation)
        .map_err(|e| format!("the DID document does not list the key: {e}"))?;
    if listed.public_key_jwk == identity.key.public_jwk() {
        Ok(())
    } else {
        Err("the DID document lists another key than the one loaded".to_owned())
    }
}
