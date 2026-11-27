// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The issuer's readiness checks (ADR 0023, section 1), served on the
//! management port by `portitor_runtime::health`. Only its own
//! dependencies, never a holder:
//!
//! - `database`: the database answers (PostgreSQL, or the `SQLite`
//!   file);
//! - `key`: the issuer's key is loaded and signs;
//! - `status-lists`: both status lists it serves (`vc11`, `vc20`) are in
//!   the store and signed by that key, or by a retired key the DID
//!   document still lists (another replica may still run with it during
//!   a key rotation).

use std::sync::Arc;

use portitor_dcp::Jwk;
use portitor_runtime::database::{Database, DatabaseSetting};
use portitor_runtime::health::{HealthChecks, blocking, database_answers, key_signs, signed_by};

use crate::Issuer;
use crate::status_lists;

/// The issuer's checks, for `issuer` with its `database`, opened from
/// `setting`.
#[must_use]
pub fn readiness_checks(
    issuer: &Arc<Issuer>,
    setting: DatabaseSetting,
    database: Arc<Database>,
) -> HealthChecks {
    let mut checks = HealthChecks::new();
    checks.add(
        "database",
        blocking(move || database_answers(&setting, &database)),
    );
    let for_key = Arc::clone(issuer);
    checks.add("key", blocking(move || key_signs(&for_key.key)));
    let for_lists = Arc::clone(issuer);
    checks.add(
        "status-lists",
        blocking(move || lists_are_served(&for_lists)),
    );
    checks
}

/// Check: both status lists of `issuer` are in the store and signed by
/// its key or a retired one ([`lists_signed_by`]). The lists are signed
/// at start (the issuer does not start otherwise) and a new one replaces
/// the old only with the revocation it holds, so this fails only when
/// the store does, or another replica signs with a key this one does
/// not know.
fn lists_are_served(issuer: &Issuer) -> Result<(), String> {
    let mut served = Vec::new();
    for profile in status_lists::PROFILES {
        let stored = issuer
            .store
            .status_list(profile)
            .map_err(|e| format!("status list {}: {e}", status_lists::name(profile)))?;
        let credential = stored.map(|list| list.credential).unwrap_or_default();
        served.push((status_lists::name(profile), credential));
    }
    let mut keys = vec![issuer.key.public_jwk()];
    keys.extend(issuer.retired_keys.iter().cloned());
    lists_signed_by(&served, &keys)
}

/// Each list in `served` (its name and the JWT served) is there and
/// signed by one of `keys`. When none signed it, the error names the
/// first key's failure.
fn lists_signed_by(served: &[(&str, String)], keys: &[Jwk]) -> Result<(), String> {
    for (name, list) in served {
        if list.is_empty() {
            return Err(format!("status list {name}: not signed yet"));
        }
        let first_failure = match keys.first() {
            Some(first) => signed_by(list, first).err(),
            None => Some("no key to check it with".to_owned()),
        };
        let Some(failure) = first_failure else {
            continue;
        };
        if !keys.iter().skip(1).any(|key| signed_by(list, key).is_ok()) {
            return Err(format!("status list {name}: {failure}"));
        }
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
        let public = [key.public_jwk()];
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
        // a retired key the DID document lists is fine
        let with_retired = [key.public_jwk(), other.public_jwk()];
        assert_eq!(lists_signed_by(&foreign, &with_retired), Ok(()));
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
