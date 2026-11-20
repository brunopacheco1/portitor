// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What is checked and built before a request goes to a provider: the
//! provider's address and id, a dataset's id, the DSP message that asks
//! for the catalog, and a dataset id as part of a URL path. Nothing here
//! sends anything.

use std::fmt::Write as _;

use portitor_dsp::guard::{Direction, MessageGuard};
use portitor_dsp::{CatalogRequestMessage, CatalogRequestMessageType, Context};

use super::ReadFailure;
use crate::http_client::outbound_url_allowed;

/// Checks where the request goes and to whom: `provider_address` is a
/// URL the connector may call (`https://` only, unless `allow_http`)
/// without a query or a fragment, and `provider_id` is a `did:web` DID.
pub(super) fn check_provider(
    provider_address: &str,
    provider_id: &str,
    allow_http: bool,
) -> Result<(), ReadFailure> {
    if !outbound_url_allowed(provider_address, allow_http) {
        return Err(ReadFailure::BadRequest(
            "the provider address must be an https URL".to_owned(),
        ));
    }
    if provider_address.contains(['?', '#']) {
        // a path is added to the address: a query or fragment would
        // swallow it
        return Err(ReadFailure::BadRequest(
            "the provider address must not have a query or a fragment".to_owned(),
        ));
    }
    if let Err(error) = portitor_dcp::did_web_url(provider_id) {
        return Err(ReadFailure::BadRequest(format!(
            "the provider id must be a did:web DID: {error}"
        )));
    }
    Ok(())
}

/// Checks the id of the dataset to ask for: not empty, and not a path
/// segment with a meaning of its own.
pub(super) fn check_dataset_id(dataset_id: &str) -> Result<(), ReadFailure> {
    if dataset_id.trim().is_empty() {
        return Err(ReadFailure::BadRequest(
            "the dataset id must not be empty".to_owned(),
        ));
    }
    if matches!(dataset_id, "." | "..") {
        // as a path segment these mean "this folder" and "the folder
        // above" (RFC 3986, section 5.2.4): a provider or a proxy
        // that resolves them would answer for another path
        return Err(ReadFailure::BadRequest(
            "the dataset id must not be \".\" or \"..\"".to_owned(),
        ));
    }
    Ok(())
}

/// The `CatalogRequestMessage` to send (DSP 2025-1, "Catalog Protocol",
/// "Catalog Request Message"): no `filter`, so the whole catalog is asked
/// for. Checked against the dataspace's message rules first, as every
/// outgoing DSP message is (ADR 0005).
pub(super) fn catalog_request_message(
    guard: &dyn MessageGuard,
) -> Result<serde_json::Value, ReadFailure> {
    let message = CatalogRequestMessage {
        context: Context::default(),
        kind: CatalogRequestMessageType,
        filter: None,
    };
    let message = match serde_json::to_value(&message) {
        Ok(message) => message,
        Err(error) => return Err(ReadFailure::BadRequest(error.to_string())),
    };
    match guard.check(Direction::Outbound, "CatalogRequestMessage", &message) {
        Ok(()) => Ok(message),
        Err(rejection) => Err(ReadFailure::BadRequest(rejection.to_string())),
    }
}

/// `text` as one segment of a URL path (RFC 3986, section 3.3): letters,
/// digits and `-._~!$&'()*+,;=:@` stay; every other byte becomes `%XX`. A
/// dataset id is often a URN (`urn:uuid:...`), which stays readable, but
/// it may hold a `/` or a `?`, which would change the path.
pub(super) fn path_segment(text: &str) -> String {
    const KEPT: &[u8] = b"-._~!$&'()*+,;=:@";
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || KEPT.contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // writing to a `String` cannot fail
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dataset_id_becomes_one_path_segment() {
        assert_eq!(path_segment("urn:uuid:dataset-1"), "urn:uuid:dataset-1");
        assert_eq!(path_segment("a/b?c#d e"), "a%2Fb%3Fc%23d%20e");
        assert_eq!(path_segment("é%"), "%C3%A9%25");
    }
}
