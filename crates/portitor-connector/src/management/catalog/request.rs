// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The request bodies of the two catalog calls, and the checks of their
//! members. The bodies are those of Eclipse EDC's management API v4, so
//! a client written for EDC can send the same body.

use serde::Deserialize;
use utoipa::ToSchema;

/// The one value of `protocol` (or `profile`) this connector takes: DSP
/// 2025-1 over HTTPS, as Eclipse EDC names it (`dataspace-protocol-http`
/// in `HttpMessageProtocol.java`, then `:` and the profile, in
/// `DataspaceProfileContext.java`; EDC v1.0.0-RC8).
pub const DSP_HTTP_2025_1: &str = "dataspace-protocol-http:2025-1";

/// Ask a provider for its catalog.
///
/// The body is the `CatalogRequest` of Eclipse EDC's management API v4
/// (EDC v1.0.0-RC8: `catalog-request-schema.json` in
/// `management-api-schema-validator`, `CatalogRequest.java` and
/// `CatalogRequestValidator.java`), so a client written for EDC can send
/// the same body. What Portitor does with each member:
///
/// - `counterPartyAddress`, `counterPartyId`: required, as in EDC. A
///   negotiation (`POST /v1/negotiations`) names the provider with the same
///   two members.
/// - `protocol` or `profile` (EDC's newer name for it): exactly one is
///   required, as in EDC's schema, and it must be
///   `dataspace-protocol-http:2025-1`, the only protocol this
///   connector speaks. Any other value is refused with 400.
/// - `additionalScopes`: optional, honoured as in EDC: the scopes are
///   added to the configured ones in the token for the provider.
/// - `querySpec`: refused with 400 when present. EDC sends it to the
///   provider as the `filter` of the `CatalogRequestMessage`; DSP 2025-1
///   leaves the content of `filter` to each implementation, and a
///   provider that does not support it must refuse the request, as
///   Portitor's own provider side does. Portitor always asks for the
///   whole catalog.
/// - `@context`: optional and not read. EDC requires it (its bodies are
///   JSON-LD); Portitor reads the member names as written.
/// - `@type`: optional; when present it must be `CatalogRequest`.
/// - Any other member (EDC's schema also lists `@id`, which EDC does
///   not use) is refused with 400, as on every management call.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogRequest {
    /// Accepted so that a body written for Eclipse EDC works unchanged;
    /// not read.
    #[serde(rename = "@context")]
    #[schema(value_type = Option<Vec<String>>, example = json!(["https://w3id.org/edc/connector/management/v2"]))]
    // `IgnoredAny` takes any JSON value and keeps nothing of it; the
    // leading `_` says the field is never read
    pub(super) _context: Option<serde::de::IgnoredAny>,
    /// `CatalogRequest`, when present.
    #[serde(rename = "@type")]
    #[schema(example = "CatalogRequest")]
    pub(super) kind: Option<String>,
    /// The provider's DSP endpoint, for example
    /// `https://provider.example/dsp/2025-1`: the `counterPartyAddress`
    /// of a negotiation. The catalog is read at
    /// `<counterPartyAddress>/catalog/request`.
    #[schema(example = "https://provider.example/dsp/2025-1")]
    pub(super) counter_party_address: String,
    /// The provider's participant id: a `did:web` DID, the
    /// `counterPartyId` of a negotiation. Our token is made for it, and
    /// the catalog must name it as its `participantId`.
    #[schema(example = "did:web:provider.example")]
    pub(super) counter_party_id: String,
    /// `dataspace-protocol-http:2025-1`. Exactly one of `protocol` and
    /// `profile` is required.
    #[schema(example = "dataspace-protocol-http:2025-1")]
    pub(super) protocol: Option<String>,
    /// Eclipse EDC's newer name for `protocol`; the same value.
    pub(super) profile: Option<String>,
    /// Scopes the provider may read from our Credential Service with our
    /// token, on top of the configured ones (`PORTITOR_DCP_SCOPES`): for
    /// datasets a provider shows only to holders of a credential.
    #[serde(default)]
    #[schema(example = json!(["org.eclipse.dspace.dcp.vc.type:MembershipCredential:read"]))]
    pub(super) additional_scopes: Vec<String>,
    /// Not supported: a request with a `querySpec` is refused with 400.
    /// The whole catalog is always asked for.
    #[schema(value_type = Option<Object>)]
    // only whether it is there matters (`null` reads as left out)
    pub(super) query_spec: Option<serde::de::IgnoredAny>,
}

/// Ask a provider for one dataset.
///
/// The body is the `DatasetRequest` of Eclipse EDC's management API v4
/// (EDC v1.0.0-RC8: `dataset-request-schema.json`, `DatasetRequest.java`
/// and `DatasetRequestValidator.java`). `@id` (the dataset's id),
/// `counterPartyAddress` and `counterPartyId` are required, and exactly
/// one of `protocol` and `profile`; all are read as in
/// [`CatalogRequest`]. EDC's `DatasetRequest` has no `querySpec`, so
/// none is taken here. It has no `additionalScopes` either; Portitor
/// takes it anyway, optional and read as in [`CatalogRequest`], so that
/// a dataset a provider shows only to holders of a credential can be read
/// (ADR 0019, amendment in 1.4). A body written for EDC works unchanged.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetRequest {
    /// Accepted so that a body written for Eclipse EDC works unchanged;
    /// not read.
    #[serde(rename = "@context")]
    #[schema(value_type = Option<Vec<String>>, example = json!(["https://w3id.org/edc/connector/management/v2"]))]
    // `IgnoredAny` takes any JSON value and keeps nothing of it; the
    // leading `_` says the field is never read
    pub(super) _context: Option<serde::de::IgnoredAny>,
    /// `DatasetRequest`, when present.
    #[serde(rename = "@type")]
    #[schema(example = "DatasetRequest")]
    pub(super) kind: Option<String>,
    /// The dataset's `@id` in the provider's catalog.
    #[serde(rename = "@id")]
    #[schema(example = "urn:uuid:dataset-1")]
    pub(super) id: String,
    /// The provider's DSP endpoint. The dataset is read at
    /// `<counterPartyAddress>/catalog/datasets/<@id>`.
    #[schema(example = "https://provider.example/dsp/2025-1")]
    pub(super) counter_party_address: String,
    /// The provider's participant id: a `did:web` DID.
    #[schema(example = "did:web:provider.example")]
    pub(super) counter_party_id: String,
    /// `dataspace-protocol-http:2025-1`. Exactly one of `protocol` and
    /// `profile` is required.
    #[schema(example = "dataspace-protocol-http:2025-1")]
    pub(super) protocol: Option<String>,
    /// Eclipse EDC's newer name for `protocol`; the same value.
    pub(super) profile: Option<String>,
    /// Scopes the provider may read from our Credential Service with our
    /// token, on top of the configured ones (`PORTITOR_DCP_SCOPES`): for a
    /// dataset a provider shows only to holders of a credential. Not in
    /// Eclipse EDC's `DatasetRequest`.
    #[serde(default)]
    #[schema(example = json!(["org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read"]))]
    pub(super) additional_scopes: Vec<String>,
}

/// Checks the members that say what kind of request this is: `@type`,
/// if present, is `expected_type`, and exactly one of `protocol` and
/// `profile` is there and names DSP 2025-1 over HTTPS.
///
/// # Errors
///
/// What is wrong, for the 400 answer.
pub(super) fn check_kind_and_protocol(
    kind: Option<&str>,
    expected_type: &str,
    protocol: Option<&str>,
    profile: Option<&str>,
) -> Result<(), String> {
    if let Some(kind) = kind
        && kind != expected_type
    {
        return Err(format!("@type must be {expected_type}"));
    }
    let named = match (protocol, profile) {
        (Some(protocol), None) => protocol,
        (None, Some(profile)) => profile,
        (Some(_), Some(_)) => return Err("give protocol or profile, not both".to_owned()),
        (None, None) => {
            return Err(format!(
                "protocol (or profile) is required: {DSP_HTTP_2025_1}"
            ));
        }
    };
    if named == DSP_HTTP_2025_1 {
        Ok(())
    } else {
        Err(format!(
            "protocol {named} is not supported; this connector speaks {DSP_HTTP_2025_1} only"
        ))
    }
}

/// Checks `additionalScopes`: a scope is one word, as the scopes go to
/// our STS in one field, separated by spaces (`bearer_access_scope`,
/// `auth/sts.rs`).
pub(super) fn check_scopes(scopes: &[String]) -> Result<(), String> {
    for scope in scopes {
        if scope.is_empty() || scope.contains(char::is_whitespace) {
            return Err("each of additionalScopes must be one word, without spaces".to_owned());
        }
    }
    Ok(())
}
