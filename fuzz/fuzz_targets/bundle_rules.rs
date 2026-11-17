// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The files of a bundle as the connector reads them
//! (`portitor_connector::shacl::BundleRules`, ADR 0005), and the DSP
//! messages it checks against the bundle's message shapes
//! (`ShaclGuard`). The first byte picks what the rest of the input is in
//! a small bundle: a JSON-LD context, dataset shapes, message shapes or a
//! vocabulary. Separately, the whole input is a DSP message for the
//! guard of a fixed bundle. Nothing may panic, and a refused message
//! always has a reason.
#![no_main]

use std::collections::BTreeMap;
use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use portitor_connector::harvest::DatasetCheck;
use portitor_connector::shacl::{BundleRules, MAX_REASONS};
use portitor_dcp::bundle::{self, FileEntry, Manifest, MediaType, Role, VerifiedBundle};
use portitor_dsp::guard::{Direction, MessageGuard};
use serde_json::{Value, json};

const PUBLIC: &str = "http://publications.europa.eu/resource/authority/access-right/PUBLIC";
const FUZZ_CONTEXT: &str = "https://contexts.example/fuzz.jsonld";

/// Requires a `dct:title` on every `dcat:Dataset`, and themes of the
/// vocabulary.
const DATASET_SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix dcat: <http://www.w3.org/ns/dcat#> .
@prefix dct: <http://purl.org/dc/terms/> .
<urn:shapes:Dataset> sh:targetClass dcat:Dataset ;
  sh:property [ sh:path dct:title ; sh:minCount 1 ] ;
  sh:property [ sh:path dcat:theme ; sh:class <urn:vocabulary:Theme> ] .";

/// Requires a `providerPid` that starts with `urn:` on every contract
/// request.
const MESSAGE_SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
<urn:shapes:Request> sh:targetClass <https://w3id.org/dspace/2025/1/ContractRequestMessage> ;
  sh:property [ sh:path <https://w3id.org/dspace/2025/1/providerPid> ; sh:minCount 1 ;
                sh:pattern \"^urn:\" ] .";

/// One code of a code list.
const VOCABULARY: &str = "<urn:vocabulary:health> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <urn:vocabulary:Theme> .\n";

/// A context that names one term.
const CONTEXT: &str = r#"{"@context": {"fuzz": "urn:fuzz:", "note": "urn:fuzz:note"}}"#;

/// One file of a bundle: path, media type, role and bytes.
struct File {
    path: &'static str,
    media_type: MediaType,
    role: Role,
    bytes: Vec<u8>,
}

fn datasets_file(path: &'static str, bytes: &[u8]) -> File {
    let role = Role::Datasets {
        access_rights: vec![PUBLIC.to_owned()],
    };
    File {
        path,
        media_type: MediaType::Turtle,
        role,
        bytes: bytes.to_vec(),
    }
}

fn messages_file(path: &'static str, bytes: &[u8]) -> File {
    File {
        path,
        media_type: MediaType::Turtle,
        role: Role::Messages {},
        bytes: bytes.to_vec(),
    }
}

fn vocabulary_file(path: &'static str, bytes: &[u8]) -> File {
    let media_type = MediaType::NTriples;
    File {
        path,
        media_type,
        role: Role::Vocabulary {},
        bytes: bytes.to_vec(),
    }
}

fn context_file(path: &'static str, url: &str, bytes: &[u8]) -> File {
    let role = Role::Context {
        url: url.to_owned(),
    };
    File {
        path,
        media_type: MediaType::JsonLd,
        role,
        bytes: bytes.to_vec(),
    }
}

/// A bundle of `files`, with a manifest that passes its checks and
/// digests that match (as after the signature and digest checks).
fn bundle_of(files: Vec<File>) -> Option<VerifiedBundle> {
    let mut entries = Vec::new();
    let mut bytes = BTreeMap::new();
    for file in files {
        entries.push(FileEntry {
            path: file.path.to_owned(),
            sha256: bundle::sha256_hex(&file.bytes),
            media_type: file.media_type,
            role: file.role,
        });
        bytes.insert(file.path.to_owned(), file.bytes);
    }
    let manifest = Manifest {
        version: "2026.11.0".to_owned(),
        issued: "2026-11-17T00:00:00Z".to_owned(),
        authority: "did:web:authority.example".to_owned(),
        profile: "https://profile.example/1".to_owned(),
        files: entries,
    };
    manifest.check().ok()?;
    bundle::check_files(manifest, bytes).ok()
}

/// The fixed shapes and vocabulary, with `context` for [`FUZZ_CONTEXT`]
/// and `extra`.
fn bundle_with(context: &[u8], extra: Option<File>) -> Option<VerifiedBundle> {
    let mut files = vec![
        datasets_file("datasets.ttl", DATASET_SHAPES.as_bytes()),
        messages_file("messages.ttl", MESSAGE_SHAPES.as_bytes()),
        vocabulary_file("vocabulary.nt", VOCABULARY.as_bytes()),
        context_file("fuzz.jsonld", FUZZ_CONTEXT, context),
    ];
    if let Some(extra) = extra {
        files.push(extra);
    }
    bundle_of(files)
}

/// The rules of the fixed bundle, for the guard.
static RULES: LazyLock<BundleRules> = LazyLock::new(|| {
    let bundle = bundle_with(CONTEXT.as_bytes(), None).expect("bundle");
    BundleRules::from_bundle(&bundle).expect("rules")
});

/// The guard of `rules` on `message`: a refusal has between one reason
/// and the most it lists, plus one line for the rest.
fn check_message(rules: &BundleRules, message: &Value) {
    let Some(guard) = rules.guard() else {
        return;
    };
    for direction in [Direction::Inbound, Direction::Outbound] {
        if let Err(rejection) = guard.check(direction, "ContractRequestMessage", message) {
            assert!(!rejection.reasons.is_empty());
            assert!(rejection.reasons.len() <= MAX_REASONS + 1);
        }
    }
}

/// A contract request that uses the fuzzed context, and a dataset entry.
fn use_rules(rules: &BundleRules) {
    let message = json!({
        "@context": ["https://w3id.org/dspace/2025/1/context.jsonld", FUZZ_CONTEXT],
        "@type": "ContractRequestMessage",
        "consumerPid": "urn:consumer:1",
        "providerPid": "urn:provider:1",
        "note": "a note",
        "callbackAddress": "https://consumer.example/callback",
        "offer": {"@id": "urn:offer:1", "@type": "Offer", "target": "urn:dataset:1"}
    });
    check_message(rules, &message);
    if let Some(check) = rules.dataset_check() {
        let entry = json!({
            "@id": "urn:dataset:1", "@type": "Dataset",
            "dct:title": "A title",
            "dct:accessRights": {"@id": PUBLIC},
            "dcat:theme": {"@id": "urn:vocabulary:health"},
            "hasPolicy": [{"@id": "urn:offer:1", "@type": "Offer", "permission": [{"action": "use"}]}],
            "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}]
        });
        let _ = check.check(&entry);
    }
}

fuzz_target!(|data: &[u8]| {
    // the whole input as a DSP message, for the guard of the fixed bundle
    if let Ok(message) = serde_json::from_slice::<Value>(data) {
        check_message(&RULES, &message);
    }
    // the rest of the input as one more file of a bundle
    let Some((&kind, rest)) = data.split_first() else {
        return;
    };
    let fixed_context = CONTEXT.as_bytes();
    let bundle = match kind % 5 {
        // the input in place of the context the request uses
        0 => bundle_with(rest, None),
        1 => {
            let url = "https://contexts.example/extra.jsonld";
            bundle_with(fixed_context, Some(context_file("extra.jsonld", url, rest)))
        }
        2 => bundle_with(fixed_context, Some(datasets_file("extra.ttl", rest))),
        3 => bundle_with(fixed_context, Some(messages_file("extra.ttl", rest))),
        _ => bundle_with(fixed_context, Some(vocabulary_file("extra.nt", rest))),
    };
    let Some(bundle) = bundle else {
        return;
    };
    if let Ok(rules) = BundleRules::from_bundle(&bundle) {
        use_rules(&rules);
    }
});
