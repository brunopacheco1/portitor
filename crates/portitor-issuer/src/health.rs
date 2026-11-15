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

/// Each status list the issuer serves (the same value the handler
/// sends) is a JWT signed by its key.
fn lists_are_served(issuer: &Issuer) -> Result<(), String> {
    let public = issuer.key.public_jwk();
    for profile in status_lists::PROFILES {
        let name = status_lists::name(profile);
        let served = issuer.status_lists.get(profile);
        if served.is_empty() {
            return Err(format!("status list {name}: not signed yet"));
        }
        signed_by(&served, &public).map_err(|e| format!("status list {name}: {e}"))?;
    }
    Ok(())
}
