// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The issuer's status lists (ADR 0018): one Bitstring Status List
//! credential per profile, 131,072 entries, purpose `revocation`, served
//! at `<issuer service>/status-lists/vc11` and `.../vc20`.
//!
//! A list is built from the revoked credentials in the store. The process
//! that revokes a credential signs the new list and stores it with the
//! revocation, in one transaction, as Eclipse EDC's `IssuerService` does
//! (TD-51). Every process serves the list from the store, so a revocation
//! made on one replica is served by all of them at once. At start a
//! process signs a list only when the store has none, or has one signed
//! by another key (after a key rotation).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use portitor_dcp::credential::Profile;
use portitor_dcp::issuance::{sign_status_list, status_list_with};
use portitor_runtime::api_key::{internal_error, problem};
use portitor_runtime::system::now_secs;

use crate::Issuer;
use crate::store::SignedStatusList;

/// The media type of a status list credential (a JWT, VC Data Model 2.0).
pub const MEDIA_TYPE: &str = "application/vc+jwt";

/// Every profile; each has its own status list.
pub(crate) const PROFILES: [Profile; 2] = [Profile::Vc11, Profile::Vc20];

/// The last part of a list's URL. The one place that maps a profile to
/// its list name; [`from_name`] maps back through it.
#[must_use]
pub fn name(profile: Profile) -> &'static str {
    match profile {
        Profile::Vc11 => "vc11",
        Profile::Vc20 => "vc20",
    }
}

/// The profile whose list has this name, if any.
#[must_use]
pub fn from_name(list_name: &str) -> Option<Profile> {
    PROFILES
        .into_iter()
        .find(|profile| name(*profile) == list_name)
}

/// The public URL of the status list of `profile`: the credentials of
/// that profile point to it, so it must stay the same.
#[must_use]
pub fn url(issuer: &Issuer, profile: Profile) -> String {
    format!("{}/status-lists/{}", issuer.service_url, name(profile))
}

/// Signs the list of `profile` with the issuer's key, the positions in
/// `revoked` set. The store calls it inside its transaction
/// (`crate::store::SignStatusList`).
///
/// # Errors
/// A position is outside the list, or signing fails.
pub(crate) fn sign(
    issuer: &Issuer,
    profile: Profile,
    revoked: &[u64],
) -> Result<SignedStatusList, String> {
    let bits = status_list_with(revoked).map_err(|e| e.to_string())?;
    let credential = sign_status_list(issuer.signer(), &url(issuer, profile), &bits, now_secs())
        .map_err(|e| e.to_string())?;
    Ok(SignedStatusList {
        credential,
        kid: issuer.kid.clone(),
    })
}

/// At start: signs and stores each list the store does not have yet, or
/// has signed by another key.
///
/// # Errors
/// The store or signing fails.
pub(crate) fn sign_if_needed(issuer: &Issuer) -> Result<(), String> {
    let mut sign_list = |profile, revoked: &[u64]| sign(issuer, profile, revoked);
    for profile in PROFILES {
        let signed = issuer
            .store
            .sign_status_list_if_needed(profile, &issuer.kid, &mut sign_list)
            .map_err(|e| e.to_string())?;
        if signed {
            tracing::info!(list = name(profile), "status list signed and stored");
        }
    }
    Ok(())
}

/// `GET <issuer service>/status-lists/{name}`: the signed list, as
/// `application/vc+jwt`, read from the store.
pub(crate) async fn serve(State(issuer): State<Arc<Issuer>>, Path(list): Path<String>) -> Response {
    let Some(profile) = from_name(&list) else {
        return problem(StatusCode::NOT_FOUND, "no such status list");
    };
    match issuer.store.status_list(profile) {
        Ok(Some(signed)) => {
            ([(header::CONTENT_TYPE, MEDIA_TYPE)], signed.credential).into_response()
        }
        // signed at start, so only a store emptied behind the issuer's
        // back has none
        Ok(None) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "status list not signed yet",
        ),
        Err(e) => internal_error("status list", &e),
    }
}
