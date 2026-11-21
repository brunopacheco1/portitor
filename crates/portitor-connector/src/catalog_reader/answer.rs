// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The checks of a provider's answer to a catalog or dataset request,
//! before it is handed on as it came: a JSON object of the type that was
//! asked for, and, for a catalog, the catalog of the provider that was
//! asked.

use serde::Deserialize;

use super::ReadFailure;

/// The two members read from a catalog answer; every other member is
/// skipped without being kept, so a large catalog is checked without
/// building it in memory a second time.
#[derive(Deserialize)]
struct CatalogHead {
    #[serde(rename = "@type")]
    kind: Option<String>,
    #[serde(rename = "participantId")]
    participant_id: Option<String>,
}

/// Only the `@type` of a dataset answer; every other member is skipped,
/// as in [`CatalogHead`].
#[derive(Deserialize)]
struct DatasetHead {
    #[serde(rename = "@type")]
    kind: Option<String>,
}

/// How much of a provider's `participantId` an error shows, in
/// characters: the text comes from the provider and goes to the caller.
const SHOWN_CHARACTERS: usize = 200;

/// Checks a provider's answer to a catalog request, sent by the provider
/// `provider_id`: a JSON object whose `@type` is `Catalog` and whose
/// `participantId` is `provider_id`.
///
/// DSP 2025-1 requires both members of a catalog (`catalog-schema.json`:
/// `@type` is the text `Catalog` in `Catalog`, and `RootCatalog` lists
/// `participantId`, a text, under `required`). The specification does
/// not say that a consumer compares `participantId` with the provider it
/// asked; Portitor does, so that the caller is not handed the catalog of
/// another participant under the name it asked for. There is no setting
/// to turn this off.
///
/// The two ids are compared as DIDs (`portitor_dcp::same_did`): equal as
/// written, or two `did:web` DIDs that differ only in the letter case of
/// the escaped colon before the port (`%3A`, `%3a`), which the DID
/// resolver reads as one DID too. Nothing is trimmed and no letter case
/// is changed elsewhere.
pub(super) fn check_catalog(body: &[u8], provider_id: &str) -> Result<(), ReadFailure> {
    check_json_object(body)?;
    let head: CatalogHead = match serde_json::from_slice(body) {
        Ok(head) => head,
        Err(error) => {
            return Err(ReadFailure::NotDsp(format!(
                "the provider's answer is not a JSON object with a text @type and a text participantId: {error}"
            )));
        }
    };
    if head.kind.as_deref() != Some("Catalog") {
        return Err(wrong_type("Catalog"));
    }
    match head.participant_id {
        Some(participant_id) if portitor_dcp::same_did(&participant_id, provider_id) => Ok(()),
        Some(participant_id) => Err(other_participant(&participant_id, provider_id)),
        None => Err(ReadFailure::NotDsp(
            "the provider's answer is not a DSP 2025-1 Catalog (it has no participantId)"
                .to_owned(),
        )),
    }
}

/// Checks a provider's answer to a dataset request: a JSON object whose
/// `@type`, if it has one, is `Dataset`. DSP 2025-1 does not require
/// `@type` of a dataset (`dataset-schema.json`, `RootDataset`, requires
/// `@context`, `@id`, `hasPolicy` and `distribution` only), and a
/// dataset has no `participantId`.
pub(super) fn check_dataset(body: &[u8]) -> Result<(), ReadFailure> {
    check_json_object(body)?;
    let head: DatasetHead = match serde_json::from_slice(body) {
        Ok(head) => head,
        Err(error) => {
            return Err(ReadFailure::NotDsp(format!(
                "the provider's answer is not a JSON object with a text @type: {error}"
            )));
        }
    };
    match head.kind.as_deref() {
        Some("Dataset") | None => Ok(()),
        Some(_) => Err(wrong_type("Dataset")),
    }
}

/// Checks that `body` is UTF-8 text that starts as a JSON object.
fn check_json_object(body: &[u8]) -> Result<(), ReadFailure> {
    // JSON is UTF-8 (RFC 8259, section 8.1). Checked here for the whole
    // body: the reader passes over the members it does not keep without
    // looking at the bytes of their texts, and the body is handed on as
    // `application/json`.
    if std::str::from_utf8(body).is_err() {
        return Err(ReadFailure::NotDsp(
            "the provider's answer is not JSON: it is not UTF-8 text".to_owned(),
        ));
    }
    // serde reads a struct from a JSON array too (its fields in order),
    // so `["Catalog"]` would pass as a catalog: only an object is read
    let first = body.iter().find(|byte| !byte.is_ascii_whitespace());
    if first != Some(&b'{') {
        return Err(ReadFailure::NotDsp(
            "the provider's answer is not a JSON object".to_owned(),
        ));
    }
    Ok(())
}

/// The failure for an answer whose `@type` is not `expected`.
fn wrong_type(expected: &str) -> ReadFailure {
    ReadFailure::NotDsp(format!(
        "the provider's answer is not a DSP 2025-1 {expected} (its @type is not \"{expected}\")"
    ))
}

/// The failure for a catalog whose `participantId` is not the provider
/// that was asked.
fn other_participant(participant_id: &str, provider_id: &str) -> ReadFailure {
    let shown: String = participant_id.chars().take(SHOWN_CHARACTERS).collect();
    ReadFailure::NotDsp(format!(
        "the provider's answer is the catalog of another participant: its participantId is {shown:?}, not the counterPartyId {provider_id:?}"
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const PROVIDER: &str = "did:web:provider.example";

    /// The reason of an answer that was refused as not what was asked.
    fn not_dsp(checked: Result<(), ReadFailure>) -> String {
        match checked {
            Err(ReadFailure::NotDsp(reason)) => reason,
            other => panic!("not refused as NotDsp: {other:?}"),
        }
    }

    #[test]
    fn a_catalog_answer_must_have_the_type_catalog() {
        let catalog =
            br#" {"@type":"Catalog","participantId":"did:web:provider.example","x":[1,{}]}"#;
        assert_eq!(check_catalog(catalog, PROVIDER), Ok(()));
        for wrong in [
            &br#"{"@type":"Dataset","participantId":"did:web:provider.example"}"#[..],
            br#"{"x":1,"participantId":"did:web:provider.example"}"#,
            b"[]",
            br#"["Catalog","did:web:provider.example"]"#,
            br#""Catalog""#,
            b"",
            b"<html>",
            br#"{"@type":"Catalog","participantId":"did:web:provider.example""#,
            // not UTF-8, in a member that is not read
            b"{\"@type\":\"Catalog\",\"participantId\":\"did:web:provider.example\",\"x\":\"\xff\"}",
        ] {
            let reason = not_dsp(check_catalog(wrong, PROVIDER));
            assert!(
                reason.starts_with("the provider's answer is not "),
                "{reason}"
            );
        }
    }

    /// DSP 2025-1 `catalog-schema.json`, `RootCatalog`: `participantId`
    /// is required and a text. It must name the provider that was asked.
    #[test]
    fn a_catalog_answer_must_name_the_provider_that_was_asked() {
        let other = br#"{"@type":"Catalog","participantId":"did:web:someone-else.example"}"#;
        assert_eq!(
            not_dsp(check_catalog(other, PROVIDER)),
            "the provider's answer is the catalog of another participant: its participantId is \"did:web:someone-else.example\", not the counterPartyId \"did:web:provider.example\""
        );
        // compared as written: other letters, a longer id or a space
        // around it are another id
        for different in [
            &br#"{"@type":"Catalog","participantId":"did:web:Provider.example"}"#[..],
            br#"{"@type":"Catalog","participantId":"did:web:provider.example:x"}"#,
            br#"{"@type":"Catalog","participantId":"did:web:provider.example "}"#,
            br#"{"@type":"Catalog","participantId":" did:web:provider.example"}"#,
            br#"{"@type":"Catalog","participantId":""}"#,
        ] {
            let reason = not_dsp(check_catalog(different, PROVIDER));
            assert!(reason.contains("another participant"), "{reason}");
        }
        let missing =
            br#"{"@type":"Catalog","dataset":[{"participantId":"did:web:provider.example"}]}"#;
        assert_eq!(
            not_dsp(check_catalog(missing, PROVIDER)),
            "the provider's answer is not a DSP 2025-1 Catalog (it has no participantId)"
        );
        for not_one_text in [
            &br#"{"@type":"Catalog","participantId":{"@id":"did:web:provider.example"}}"#[..],
            br#"{"@type":"Catalog","participantId":["did:web:provider.example"]}"#,
            br#"{"@type":"Catalog","participantId":"did:web:provider.example","participantId":"did:web:someone-else.example"}"#,
        ] {
            let reason = not_dsp(check_catalog(not_one_text, PROVIDER));
            assert!(reason.contains("a text participantId"), "{reason}");
        }
    }

    /// The one difference that is not another id: the letter case of the
    /// escaped colon before the port of a `did:web`. The DID resolver
    /// reads `%3A` and `%3a` as the same DID (`portitor_dcp::did_web_url`),
    /// so the catalog check does too, in both directions.
    #[test]
    fn a_did_that_differs_only_in_the_case_of_the_port_escape_is_the_same_provider() {
        let upper = "did:web:provider.example%3A8443";
        let lower = "did:web:provider.example%3a8443";
        let names_upper =
            br#"{"@type":"Catalog","participantId":"did:web:provider.example%3A8443"}"#;
        let names_lower =
            br#"{"@type":"Catalog","participantId":"did:web:provider.example%3a8443"}"#;
        assert_eq!(check_catalog(names_upper, lower), Ok(()));
        assert_eq!(check_catalog(names_lower, upper), Ok(()));
        // another port is another participant, and so is the same number
        // written as a path segment
        for different in [
            &br#"{"@type":"Catalog","participantId":"did:web:provider.example%3a8444"}"#[..],
            br#"{"@type":"Catalog","participantId":"did:web:provider.example:8443"}"#,
        ] {
            let reason = not_dsp(check_catalog(different, upper));
            assert!(reason.contains("another participant"), "{reason}");
        }
    }

    /// The id is the provider's text and is shown escaped (`{:?}`): a
    /// line break in it does not start a new line in the caller's
    /// problem detail or in a log.
    #[test]
    fn the_participant_id_in_the_error_is_escaped() {
        let with_line_break = br#"{"@type":"Catalog","participantId":"a\nb\u001b[31m"}"#;
        let reason = not_dsp(check_catalog(with_line_break, PROVIDER));
        assert!(reason.contains(r#""a\nb\u{1b}[31m""#), "{reason}");
        assert!(!reason.contains('\n'), "{reason}");
        assert!(!reason.contains('\u{1b}'), "{reason}");
    }

    /// The id comes from the provider and goes into the answer to the
    /// caller: only its start is shown, cut between characters.
    #[test]
    fn a_long_participant_id_is_cut_in_the_error() {
        let long = format!(
            r#"{{"@type":"Catalog","participantId":"{}"}}"#,
            "x".repeat(5000)
        );
        let reason = not_dsp(check_catalog(long.as_bytes(), PROVIDER));
        assert!(reason.contains(&"x".repeat(200)), "{reason}");
        assert!(!reason.contains(&"x".repeat(201)), "{reason}");
        // a character of several bytes across the 200th byte: the cut is
        // by characters, so this neither panics nor shows half of one
        let wide = format!(
            r#"{{"@type":"Catalog","participantId":"a{}"}}"#,
            "€".repeat(300)
        );
        let reason = not_dsp(check_catalog(wide.as_bytes(), PROVIDER));
        assert!(
            reason.contains(&format!("\"a{}\"", "€".repeat(199))),
            "{reason}"
        );
    }

    /// DSP 2025-1 `dataset-schema.json` does not require `@type`, and a
    /// dataset has no `participantId`: one that is there is not read.
    #[test]
    fn a_dataset_may_leave_its_type_out_but_not_name_another() {
        let dataset = br#"{"@id":"urn:uuid:1","hasPolicy":[],"distribution":[]}"#;
        assert_eq!(check_dataset(dataset), Ok(()));
        assert_eq!(check_dataset(br#"{"@type":"Dataset"}"#), Ok(()));
        let with_participant = br#"{"@type":"Dataset","participantId":{"any":"thing"}}"#;
        assert_eq!(check_dataset(with_participant), Ok(()));
        for wrong in [&br#"{"@type":"Catalog"}"#[..], br#"["Dataset"]"#, b"[]"] {
            not_dsp(check_dataset(wrong));
        }
    }
}
