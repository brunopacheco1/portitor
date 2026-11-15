// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The issuer's readiness checks (ADR 0023, section 1), served on the
//! management port by `portitor_runtime::health`. Only its own
//! dependencies, never a holder:
//!
//! - `database`: the `SQLite` file answers;
//! - `key`: the issuer's key is loaded and signs;
//! - `status-lists`: both status lists it serves (`vc11`, `vc20`) are
//!   there and signed by that key.

use std::path::PathBuf;
use std::sync::Arc;

use portitor_runtime::health::{HealthChecks, blocking, key_signs, signed_by, sqlite_file_answers};

use crate::Issuer;
use crate::status_lists;

/// The issuer's checks, for `issuer` with its `SQLite` file `db_file`
/// (`PORTITOR_DB_FILE`).
#[must_use]
pub fn readiness_checks(issuer: &Arc<Issuer>, db_file: PathBuf) -> HealthChecks {
    let mut checks = HealthChecks::new();
    checks.add("database", blocking(move || sqlite_file_answers(&db_file)));
    let for_key = Arc::clone(issuer);
    checks.add("key", blocking(move || key_signs(&for_key.key)));
    let for_lists = Arc::clone(issuer);
    checks.add(
        "status-lists",
        blocking(move || lists_are_served(&for_lists)),
    );
    checks
}

/// Check: both status lists of `issuer` are signed by its key
/// ([`lists_signed_by`]). The lists are signed at start (the issuer does
/// not start otherwise) and a new one replaces the old only once it is
/// signed, so this cannot fail while the issuer serves; it guards
/// against a change in how they are made.
fn lists_are_served(issuer: &Issuer) -> Result<(), String> {
    let mut served = Vec::new();
    for profile in status_lists::PROFILES {
        served.push((
            status_lists::name(profile),
            issuer.status_lists.get(profile),
        ));
    }
    lists_signed_by(&served, &issuer.key.public_jwk())
}

/// Each list in `served` (its name and the JWT served) is there and
/// signed by `public`.
fn lists_signed_by(served: &[(&str, String)], public: &portitor_dcp::Jwk) -> Result<(), String> {
    for (name, list) in served {
        if list.is_empty() {
            return Err(format!("status list {name}: not signed yet"));
        }
        signed_by(list, public).map_err(|e| format!("status list {name}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use portitor_dcp::SigningKey;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn every_list_must_be_there_and_signed_by_the_key() {
        let key = SigningKey::generate().unwrap();
        let other = SigningKey::generate().unwrap();
        let signed = |by: &SigningKey| by.sign_json("k", "JWT", &serde_json::json!({})).unwrap();
        let public = key.public_jwk();
        let good = [("vc11", signed(&key)), ("vc20", signed(&key))];
        assert_eq!(lists_signed_by(&good, &public), Ok(()));
        let missing = [("vc11", signed(&key)), ("vc20", String::new())];
        assert_eq!(
            lists_signed_by(&missing, &public).unwrap_err(),
            "status list vc20: not signed yet"
        );
        let foreign = [("vc11", signed(&other)), ("vc20", signed(&key))];
        let err = lists_signed_by(&foreign, &public).unwrap_err();
        assert!(err.starts_with("status list vc11: not signed by"), "{err}");
        // as a readiness check: 503
        let mut checks = HealthChecks::new();
        checks.add(
            "status-lists",
            blocking(move || lists_signed_by(&missing, &public)),
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
