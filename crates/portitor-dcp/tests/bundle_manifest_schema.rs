// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The bundle manifest's JSON Schema (`docs/schemas/bundle-manifest.schema.json`)
//! and the Rust type `portitor_dcp::bundle::Manifest` agree: example
//! manifests pass both, broken ones fail both.
#![allow(clippy::unwrap_used)]

use portitor_dcp::bundle::Manifest;
use serde_json::{Value, json};

const SCHEMA: &str = include_str!("../../../docs/schemas/bundle-manifest.schema.json");

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

/// A HealthDCAT-AP-like bundle with every role.
fn full_example() -> Value {
    json!({
      "version": "2026.11.0",
      "issued": "2026-11-14T00:00:00Z",
      "authority": "did:web:authority.example",
      "profile": "https://healthdcat-ap.example/releases/release-8",
      "files": [
        {
          "path": "shapes/public-shapes.ttl",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000001",
          "mediaType": "text/turtle",
          "role": {
            "type": "datasets",
            "accessRights": ["http://publications.europa.eu/resource/authority/access-right/PUBLIC"]
          }
        },
        {
          "path": "shapes/restricted-shapes.ttl",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000002",
          "mediaType": "text/turtle",
          "role": {
            "type": "datasets",
            "accessRights": [
              "http://publications.europa.eu/resource/authority/access-right/RESTRICTED",
              "http://publications.europa.eu/resource/authority/access-right/NON_PUBLIC"
            ]
          }
        },
        {
          "path": "shapes/odrl-profile.ttl",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000003",
          "mediaType": "text/turtle",
          "role": {"type": "messages"}
        },
        {
          "path": "vocabularies/access-right.nt",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000004",
          "mediaType": "application/n-triples",
          "role": {"type": "vocabulary"}
        },
        {
          "path": "contexts/dspace-2025-1.jsonld",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000005",
          "mediaType": "application/ld+json",
          "role": {"type": "context", "url": "https://w3id.org/dspace/2025/1/context.jsonld"}
        }
      ]
    })
}

/// The smallest valid manifest.
fn minimal_example() -> Value {
    json!({
      "version": "1",
      "issued": "2026-01-01T00:00:00Z",
      "authority": "did:web:authority.example:dataspace",
      "profile": "http://profile.example/p",
      "files": [{
        "path": "messages.ttl",
        "sha256": "abcdef0000000000000000000000000000000000000000000000000000000000",
        "mediaType": "text/turtle",
        "role": {"type": "messages"}
      }]
    })
}

#[test]
fn examples_pass_the_schema_and_the_rust_type() {
    let validator = validator();
    for example in [full_example(), minimal_example()] {
        let errors: Vec<String> = validator
            .iter_errors(&example)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let manifest = Manifest::parse(&serde_json::to_vec(&example).unwrap()).unwrap();
        // written back unchanged
        assert_eq!(serde_json::to_value(&manifest).unwrap(), example);
    }
}

/// One change that breaks a manifest: its name, the JSON pointer of the
/// object to change, the field, and the new value.
type Change = (&'static str, &'static str, &'static str, Value);

#[test]
fn broken_manifests_fail_the_schema_and_the_rust_type() {
    let validator = validator();
    let digest = "ABCDEF0000000000000000000000000000000000000000000000000000000000";
    let changes: Vec<Change> = vec![
        ("unknown field", "", "signedBy", json!("x")),
        ("no files", "", "files", json!([])),
        (
            "RDF/XML",
            "/files/0",
            "mediaType",
            json!("application/rdf+xml"),
        ),
        ("path traversal", "/files/0", "path", json!("../x.ttl")),
        ("absolute path", "/files/0", "path", json!("/x.ttl")),
        ("backslash", "/files/0", "path", json!("a\\x.ttl")),
        ("reserved name", "/files/0", "path", json!("manifest.json")),
        ("upper-case digest", "/files/0", "sha256", json!(digest)),
        (
            "context as Turtle",
            "/files/4",
            "mediaType",
            json!("text/turtle"),
        ),
        (
            "shapes as JSON-LD",
            "/files/0",
            "mediaType",
            json!("application/ld+json"),
        ),
        (
            "no access rights",
            "/files/0/role",
            "accessRights",
            json!([]),
        ),
        ("role without type", "/files/0", "role", json!({})),
        (
            "role type not a string",
            "/files/0",
            "role",
            json!({"type": 7}),
        ),
        ("role not an object", "/files/0", "role", json!("messages")),
        ("role as an array", "/files/0", "role", json!(["messages"])),
        (
            "role type not a name",
            "/files/0",
            "role",
            json!({"type": "two words"}),
        ),
        (
            "extra field in a context role",
            "/files/4/role",
            "extra",
            json!(1),
        ),
        (
            "extra role field",
            "/files/2/role",
            "accessRights",
            json!(["https://x.example"]),
        ),
        ("version with a slash", "", "version", json!("1/2")),
        ("not did:web", "", "authority", json!("did:key:z6Mk")),
        ("profile not an IRI", "", "profile", json!("healthdcat")),
    ];
    for (name, pointer, field, value) in changes {
        let mut manifest = full_example();
        manifest.pointer_mut(pointer).unwrap()[field] = value;
        assert!(!validator.is_valid(&manifest), "schema accepted: {name}");
        let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap());
        assert!(parsed.is_err(), "Rust type accepted: {name}");
    }
}

/// ADR 0005, amended in 0.13: a role object with a type this version
/// does not know is valid, with any other fields and any media type of
/// the shape of RFC 6838 (one of the three or another); the reader skips
/// the file.
#[test]
fn a_file_with_an_unknown_role_passes_the_schema_and_the_rust_type() {
    let validator = validator();
    let roles = [
        json!({"type": "rules"}),
        json!({"type": "policy-shapes", "appliesTo": ["offers"], "strict": {"level": 2}}),
    ];
    for role in roles {
        for media_type in [
            "text/turtle",
            "application/n-triples",
            "application/ld+json",
            // outside today's three: a later role may need them
            "application/json",
            "application/odrl+json",
            "application/vnd.example.rules-v2+xml",
            "Text/X-Rules_1",
        ] {
            let mut manifest = full_example();
            manifest["files"][0]["role"] = role.clone();
            manifest["files"][0]["mediaType"] = json!(media_type);
            let errors: Vec<String> = validator
                .iter_errors(&manifest)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{role} as {media_type}: {errors:?}");
            let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).unwrap();
            let skipped = parsed.files_of_unknown_role();
            assert_eq!(skipped.len(), 1, "{role}");
            assert_eq!(skipped[0].role_type, role["type"].as_str().unwrap());
        }
    }
}

/// The media type of a file with an unknown role is open, but it must
/// have the shape of a media type (RFC 6838 section 4.2, no parameters);
/// for a file with a known role only the three are valid. The schema and
/// the Rust type agree on each case.
#[test]
fn media_types_are_open_only_for_unknown_roles_and_only_in_shape() {
    let validator = validator();
    let long_name = "a".repeat(128);
    let not_media_types = [
        json!(""),
        json!("turtle"),
        json!("text/"),
        json!("/turtle"),
        json!("text/turtle; charset=utf-8"),
        json!("text/tur tle"),
        json!("text/a/b"),
        json!("-text/turtle"),
        json!("text/\u{e9}"),
        json!(format!("text/{long_name}")),
        json!(7),
        json!(null),
    ];
    for media_type in not_media_types {
        let mut manifest = full_example();
        manifest["files"][0]["role"] = json!({"type": "rules"});
        manifest["files"][0]["mediaType"] = media_type.clone();
        assert!(!validator.is_valid(&manifest), "schema: {media_type}");
        let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap());
        assert!(parsed.is_err(), "Rust type: {media_type}");
    }
    // the longest names pass both
    let longest = format!("{}/{}", "a".repeat(127), "b".repeat(127));
    let mut manifest = full_example();
    manifest["files"][0]["role"] = json!({"type": "rules"});
    manifest["files"][0]["mediaType"] = json!(longest);
    assert!(validator.is_valid(&manifest));
    assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_ok());
    // a known role keeps the closed list: every file of the example, with
    // a media type that an unknown role may have
    let files = full_example()["files"].as_array().unwrap().len();
    for index in 0..files {
        for media_type in ["application/json", "application/rdf+xml", "Text/Turtle"] {
            let mut manifest = full_example();
            manifest["files"][index]["mediaType"] = json!(media_type);
            assert!(
                !validator.is_valid(&manifest),
                "schema: file {index} as {media_type}"
            );
            let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap());
            assert!(parsed.is_err(), "Rust type: file {index} as {media_type}");
        }
    }
}

/// A manifest in which no file has a known role gives no rules: both
/// refuse it.
#[test]
fn a_manifest_with_only_unknown_roles_fails_the_schema_and_the_rust_type() {
    let mut manifest = minimal_example();
    manifest["files"][0]["role"] = json!({"type": "rules"});
    assert!(!validator().is_valid(&manifest));
    assert!(Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).is_err());
}
