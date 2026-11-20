// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading a provider's catalog as consumer (DSP 2025-1, "Catalog HTTPS
//! Binding"): `POST <provider address>/catalog/request` for the catalog
//! and `GET <provider address>/catalog/datasets/{id}` for one dataset.
//! The management API uses it (`POST /v1/catalog/request`,
//! `POST /v1/catalog/dataset/request`), so an operator needs no token
//! of their own to see what a provider offers (TD-38).
//!
//! Each request carries a fresh DCP token for the provider, from the same
//! [`Prover`] the negotiation and transfer messages use (ADR 0016), and
//! goes out through the same [`Sender`], so the rule for outbound URLs is
//! the same too (`https://` only, unless `PORTITOR_ALLOW_HTTP`).
//!
//! What differs from the outbox ([`crate::outbox`]):
//!
//! - One try, no retries: a person or a script waits for the answer and
//!   can ask again. Nothing is stored.
//! - A larger answer is read ([`MAX_CATALOG_ANSWER`]) and more time is
//!   given ([`CATALOG_TIMEOUT`]).
//! - The provider's answer is handed on as it came, byte for byte. It is
//!   only checked to be a JSON object whose `@type` is `Catalog` (for a
//!   dataset: `Dataset`, or no `@type` at all, as the DSP 2025-1 schema
//!   of a dataset does not require one): reading it into Portitor's own
//!   types and writing it again would drop whatever those types do not
//!   know.
//! - A catalog must be the catalog of the provider that was asked: its
//!   `participantId` is the provider id of the request (the
//!   `counterPartyId` of the management call). DSP 2025-1 requires
//!   `participantId` of a catalog (`catalog-schema.json`, `RootCatalog`),
//!   so a catalog without it is refused too. A dataset has no
//!   `participantId`.
//! - At most [`MAX_CONCURRENT_READS`] answers are read at a time, as each
//!   may be held in memory whole.
//!
//! Pages: a provider MAY send its catalog in pages, named by `Link`
//! headers ("Catalog HTTPS Binding", "Pagination"). Portitor reads the
//! first answer only and does not follow `rel="next"`: the specification
//! does not say how the pages of a catalog are joined into one. The
//! provider's `Link` header values are returned with the answer
//! ([`ProviderAnswer::links`]), so the caller can see that there is more.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use portitor_dsp::guard::{Direction, MessageGuard};
use portitor_dsp::{CatalogRequestMessage, CatalogRequestMessageType, Context};
use serde::Deserialize;

use crate::auth::Prover;
use crate::http_client::{AnswerLimits, Bytes, HttpError, RawAnswer, Sender, outbound_url_allowed};
use crate::ports::Ports;

/// The largest catalog (or dataset) answer read from a provider: 16 MiB.
///
/// Every other answer from a counter-party is read up to 64 KiB, which
/// holds about 250 small datasets (TD-36). The 0.12 benchmark measured
/// 2.6 MB for a catalog of 10,000 small datasets (`docs/BENCHMARKS.md`),
/// and a national gateway lists about 1,100 datasets (TD-36); 16 MiB
/// leaves each of those about 15 kB of metadata. The limit is fixed, not
/// a setting: it only has to stop a provider from making the connector
/// buffer a body without end. A larger answer fails with HTTP 502 on the
/// management API.
pub const MAX_CATALOG_ANSWER: usize = 16 * 1024 * 1024;

/// How long the provider has to send its whole catalog (or dataset)
/// answer: longer than the 10 seconds of other calls, as the answer may
/// be large and the provider builds it on request. With the token
/// request before it (at most 10 seconds) this stays under the 60
/// seconds a request to the management port may take
/// (`portitor_runtime::serve::REQUEST_TIMEOUT`, set on that port in
/// `main.rs`).
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// How many provider answers are read at the same time. Each may hold
/// [`MAX_CATALOG_ANSWER`] in memory (and about twice that while its
/// parts are joined), so without a limit a script sending many catalog
/// requests at once could use up the connector's memory. A request over
/// the limit waits for a free place; the time limit of the management
/// port ends a wait that is too long with HTTP 503.
pub const MAX_CONCURRENT_READS: usize = 4;

/// A provider's answer to a catalog or dataset request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAnswer {
    /// The body as the provider sent it: a DSP 2025-1 `Catalog` or
    /// `Dataset` in JSON.
    pub body: Bytes,
    /// The provider's `Link` header values (catalog pages); empty when
    /// the answer is the whole catalog.
    pub links: Vec<String>,
}

/// Why a provider's catalog or dataset could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    /// The request cannot be sent as asked; the text says why. Nothing
    /// was sent. (HTTP 400 on the management API.)
    BadRequest(String),
    /// Our identity component gave no token for the provider. The reason
    /// is in the log. (HTTP 502.)
    NoToken,
    /// The provider did not give a usable answer: it refused, could not
    /// be reached, or sent more than [`MAX_CATALOG_ANSWER`]. (HTTP 502.)
    Provider(HttpError),
    /// The provider answered 2xx, but not with what was asked for (also:
    /// with the catalog of another participant); the text says what is
    /// wrong. (HTTP 502.)
    NotDsp(String),
}

/// What to read from a provider.
enum Wanted<'a> {
    /// Its catalog.
    Catalog,
    /// One dataset, by its `@id`.
    Dataset(&'a str),
}

/// Reads catalogs and datasets from providers, with our token.
pub(crate) struct CatalogReader {
    sender: Arc<dyn Sender>,
    prover: Arc<dyn Prover>,
    guard: Arc<dyn MessageGuard + Send + Sync>,
    allow_http: bool,
    /// One permit per answer being read ([`MAX_CONCURRENT_READS`]).
    reads: tokio::sync::Semaphore,
}

impl CatalogReader {
    pub(crate) fn new(ports: &Ports, allow_http: bool) -> Self {
        Self {
            sender: Arc::clone(&ports.sender),
            prover: Arc::clone(&ports.prover),
            guard: Arc::clone(&ports.guard),
            allow_http,
            reads: tokio::sync::Semaphore::new(MAX_CONCURRENT_READS),
        }
    }

    /// The catalog of the provider `provider_id` (a `did:web` DID) whose
    /// DSP endpoint is `provider_address`. Our token lets the provider
    /// read `additional_scopes` from our Credential Service, on top of
    /// the configured scopes.
    pub(crate) async fn catalog(
        &self,
        provider_address: &str,
        provider_id: &str,
        additional_scopes: &[String],
    ) -> Result<ProviderAnswer, ReadFailure> {
        self.read(
            provider_address,
            provider_id,
            additional_scopes,
            &Wanted::Catalog,
        )
        .await
    }

    /// The dataset `dataset_id` of the provider `provider_id` whose DSP
    /// endpoint is `provider_address`.
    pub(crate) async fn dataset(
        &self,
        provider_address: &str,
        provider_id: &str,
        dataset_id: &str,
    ) -> Result<ProviderAnswer, ReadFailure> {
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
        let wanted = Wanted::Dataset(dataset_id);
        self.read(provider_address, provider_id, &[], &wanted).await
    }

    /// Checks the address and the id, gets a token, sends the request
    /// once and checks the answer (`check_catalog`, `check_dataset`).
    async fn read(
        &self,
        provider_address: &str,
        provider_id: &str,
        scopes: &[String],
        wanted: &Wanted<'_>,
    ) -> Result<ProviderAnswer, ReadFailure> {
        if !outbound_url_allowed(provider_address, self.allow_http) {
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
        let base = provider_address.trim_end_matches('/');
        // the semaphore is never closed, so this waits for a free place
        let _reading = self.reads.acquire().await;
        let limits = AnswerLimits {
            max_bytes: MAX_CATALOG_ANSWER,
            timeout: CATALOG_TIMEOUT,
        };
        let sent = match wanted {
            Wanted::Catalog => {
                let message = catalog_request_message(self.guard.as_ref())?;
                let authorization = self.authorization(provider_id, scopes).await?;
                let url = format!("{base}/catalog/request");
                self.sender
                    .post_authorized_within(&url, &message, &authorization, limits)
                    .await
            }
            Wanted::Dataset(dataset_id) => {
                let authorization = self.authorization(provider_id, scopes).await?;
                let url = format!("{base}/catalog/datasets/{}", path_segment(dataset_id));
                self.sender
                    .get_authorized_within(&url, &authorization, limits)
                    .await
            }
        };
        let RawAnswer { body, links } = sent.map_err(ReadFailure::Provider)?;
        match wanted {
            Wanted::Catalog => check_catalog(&body, provider_id)?,
            Wanted::Dataset(_) => check_dataset(&body)?,
        }
        if !links.is_empty() {
            tracing::warn!(
                "the provider sent Link headers (catalog pages); only this first answer is read"
            );
        }
        Ok(ProviderAnswer { body, links })
    }

    /// The `Authorization` header for one request to `provider_id`.
    async fn authorization(
        &self,
        provider_id: &str,
        scopes: &[String],
    ) -> Result<String, ReadFailure> {
        match self.prover.authorization(provider_id, scopes).await {
            Ok(authorization) => Ok(authorization),
            Err(error) => {
                // the text may name our STS: for the log, not the caller
                tracing::warn!(%error, "no token to read a provider's catalog");
                Err(ReadFailure::NoToken)
            }
        }
    }
}

/// The `CatalogRequestMessage` to send (DSP 2025-1, "Catalog Protocol",
/// "Catalog Request Message"): no `filter`, so the whole catalog is asked
/// for. Checked against the dataspace's message rules first, as every
/// outgoing DSP message is (ADR 0005).
fn catalog_request_message(guard: &dyn MessageGuard) -> Result<serde_json::Value, ReadFailure> {
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
/// another participant under the name it asked for. The ids are compared
/// as written.
fn check_catalog(body: &[u8], provider_id: &str) -> Result<(), ReadFailure> {
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
        Some(participant_id) if participant_id == provider_id => Ok(()),
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
fn check_dataset(body: &[u8]) -> Result<(), ReadFailure> {
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

/// `text` as one segment of a URL path (RFC 3986, section 3.3): letters,
/// digits and `-._~!$&'()*+,;=:@` stay; every other byte becomes `%XX`. A
/// dataset id is often a URN (`urn:uuid:...`), which stays readable, but
/// it may hold a `/` or a `?`, which would change the path.
fn path_segment(text: &str) -> String {
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
#[allow(clippy::unwrap_used)]
mod tests {
    use portitor_dsp::guard::Rejection;
    use portitor_runtime::BoxFuture;
    use serde_json::Value;

    use super::*;

    #[test]
    fn a_dataset_id_becomes_one_path_segment() {
        assert_eq!(path_segment("urn:uuid:dataset-1"), "urn:uuid:dataset-1");
        assert_eq!(path_segment("a/b?c#d e"), "a%2Fb%3Fc%23d%20e");
        assert_eq!(path_segment("é%"), "%C3%A9%25");
    }

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
        // compared as written: other letters or a longer id are another id
        for different in [
            &br#"{"@type":"Catalog","participantId":"did:web:Provider.example"}"#[..],
            br#"{"@type":"Catalog","participantId":"did:web:provider.example:x"}"#,
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

    /// The id comes from the provider and goes into the answer to the
    /// caller: only its start is shown.
    #[test]
    fn a_long_participant_id_is_cut_in_the_error() {
        let long = format!(
            r#"{{"@type":"Catalog","participantId":"{}"}}"#,
            "x".repeat(5000)
        );
        let reason = not_dsp(check_catalog(long.as_bytes(), PROVIDER));
        assert!(reason.contains(&"x".repeat(200)), "{reason}");
        assert!(!reason.contains(&"x".repeat(201)), "{reason}");
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

    /// Never sends: the tests below fail before a request goes out.
    struct NeverSends;

    impl Sender for NeverSends {
        fn post<'a>(&'a self, _: &'a str, _: &'a Value) -> BoxFuture<'a, Result<Value, String>> {
            Box::pin(async { Err("sent".to_owned()) })
        }
    }

    struct RefusesAll;

    impl MessageGuard for RefusesAll {
        fn check(&self, _: Direction, _: &str, _: &Value) -> Result<(), Rejection> {
            Err(Rejection {
                reasons: vec!["no catalog requests here".to_owned()],
            })
        }
    }

    fn reader(allow_http: bool) -> CatalogReader {
        CatalogReader::new(&Ports::new(Arc::new(NeverSends)), allow_http)
    }

    #[tokio::test]
    async fn a_bad_address_or_id_is_refused_before_anything_is_sent() {
        let provider = PROVIDER;
        for (address, id, allow_http) in [
            ("http://provider.example/dsp", provider, false),
            ("ftp://provider.example/dsp", provider, true),
            ("https://provider.example/dsp?x=1", provider, false),
            ("https://provider.example/dsp#x", provider, false),
            ("https://provider.example/dsp", "provider", false),
            ("https://provider.example/dsp", "did:key:z6Mk", false),
        ] {
            let read = reader(allow_http).catalog(address, id, &[]).await;
            assert!(
                matches!(read, Err(ReadFailure::BadRequest(_))),
                "{address} {id}"
            );
        }
        let read = reader(false)
            .dataset("https://provider.example/dsp", provider, " ")
            .await;
        assert!(matches!(read, Err(ReadFailure::BadRequest(_))));
        for dots in [".", ".."] {
            let read = reader(false)
                .dataset("https://provider.example/dsp", provider, dots)
                .await;
            assert!(matches!(read, Err(ReadFailure::BadRequest(_))), "{dots}");
        }
    }

    #[tokio::test]
    async fn without_a_token_nothing_is_sent() {
        // `Ports::new` starts with a prover that has no tokens
        let read = reader(false)
            .catalog(
                "https://provider.example/dsp",
                "did:web:provider.example",
                &[],
            )
            .await;
        assert_eq!(read, Err(ReadFailure::NoToken));
    }

    #[tokio::test]
    async fn the_dataspace_rules_can_refuse_the_request() {
        let mut ports = Ports::new(Arc::new(NeverSends));
        ports.guard = Arc::new(RefusesAll);
        let read = CatalogReader::new(&ports, false)
            .catalog(
                "https://provider.example/dsp",
                "did:web:provider.example",
                &[],
            )
            .await;
        let Err(ReadFailure::BadRequest(reason)) = read else {
            panic!("not refused");
        };
        assert!(reason.contains("no catalog requests here"), "{reason}");
    }
}
