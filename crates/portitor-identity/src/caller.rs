// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Who is calling: every DCP request to the credential service carries a
//! self-issued ID token in `Authorization: Bearer <token>` (DCP base
//! protocol, section 4.3). The check itself is shared with the issuer
//! (`portitor_runtime::caller`); this module says who may call.

use axum::http::HeaderMap;
use axum::response::Response;
use portitor_dcp::Claims;
use portitor_runtime::caller::Audience;

use crate::{Identity, LEEWAY};

pub(crate) use portitor_runtime::caller::refused;

/// The caller's verified token claims, if the caller is a trusted
/// issuer (the Storage and Offer APIs), or a 401 answer. The DID is
/// checked before it is resolved, so an untrusted caller cannot make the
/// component fetch a URL of its choice. Why a token was refused is
/// logged, not told to the caller.
///
/// The presentation API, which verifiers call, has its own rule: an
/// access token from our STS ([`authenticate_if`], `presentation.rs`).
pub(crate) async fn authenticate_trusted_issuer(
    identity: &Identity,
    headers: &HeaderMap,
) -> Result<Claims, Response> {
    let admit = |claimed: &Claims| identity.trusts(&claimed.sub) && identity.trusts(&claimed.iss);
    authenticate_if(identity, headers, admit).await
}

/// Like [`authenticate_trusted_issuer`], with the caller's own `admit` rule: it sees the
/// token's claims before they are verified and before the caller's DID
/// is resolved, and says whether to go on. `false` gives the same 401 as
/// any other refusal, and no DID is fetched.
pub(crate) async fn authenticate_if(
    identity: &Identity,
    headers: &HeaderMap,
    admit: impl FnOnce(&Claims) -> bool,
) -> Result<Claims, Response> {
    let audience = Audience {
        did: &identity.participant_id,
        leeway: LEEWAY,
        // the store keeps them, for every replica (TD-09); `&*` borrows
        // the store the shared pointer points to
        used_token_ids: &*identity.store,
    };
    // `&*` turns the shared pointer (`Arc<dyn Directory>`) into a plain
    // borrow of the directory it points to
    portitor_runtime::caller::authenticate(headers, audience, admit, &*identity.directory).await
}
