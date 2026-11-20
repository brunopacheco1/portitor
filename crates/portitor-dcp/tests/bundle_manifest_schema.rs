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

// ---- Case by case: the schema and the Rust type give the same answer.

/// Whether the schema accepts `manifest`, and whether the Rust type does.
fn accepted_by_both(validator: &jsonschema::Validator, manifest: &Value) -> (bool, bool) {
    let by_schema = validator.is_valid(manifest);
    let by_rust = Manifest::parse(&serde_json::to_vec(manifest).unwrap()).is_ok();
    (by_schema, by_rust)
}

/// The full example with the field `field` of the object at `pointer`
/// set to `value`.
fn changed(pointer: &str, field: &str, value: &Value) -> Value {
    let mut manifest = full_example();
    manifest.pointer_mut(pointer).unwrap()[field] = value.clone();
    manifest
}

/// Checks each case (the value of one field, and whether it is valid):
/// the schema and the Rust type must both give that answer. Returns the
/// cases where one of them does not, one per line.
fn disagreements(pointer: &str, field: &str, cases: &[(Value, bool)]) -> Vec<String> {
    let validator = validator();
    let mut wrong = Vec::new();
    for (value, valid) in cases {
        let (by_schema, by_rust) = accepted_by_both(&validator, &changed(pointer, field, value));
        if by_schema != *valid || by_rust != *valid {
            wrong.push(format!(
                "{pointer}/{field} = {value}: expected valid={valid}, schema says {by_schema}, the Rust type says {by_rust}"
            ));
        }
    }
    wrong
}

/// Fails with every disagreement, so that one run shows them all.
fn assert_no_disagreement(wrong: &[String]) {
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[test]
fn versions_agree_case_by_case() {
    let cases = [
        (json!("1"), true),
        (json!("2026.11.0"), true),
        (json!("1."), true),
        (json!("-"), true),
        (json!("_draft"), true),
        (json!("a".repeat(64)), true),
        (json!("a".repeat(65)), false),
        (json!(""), false),
        (json!(".1"), false),
        (json!("1 2"), false),
        (json!("1\n"), false),
        (json!("v\u{e9}"), false),
        (json!("1:2"), false),
        (json!(1), false),
        (json!("latest.1"), true),
    ];
    assert_no_disagreement(&disagreements("", "version", &cases));
}

/// `latest` in any letter case means "the newest version" in a
/// connector's settings (`PORTITOR_BUNDLE_VERSION`), so no version may
/// have that name: `check_version` refuses it, and with it the issuer's
/// build tool and every reader. The schema does not say so: it accepts
/// such a manifest. A line like
/// `"not": {"pattern": "^[Ll][Aa][Tt][Ee][Ss][Tt]$"}` on `version` would.
#[test]
#[ignore = "bug: docs/schemas/bundle-manifest.schema.json accepts the version \"latest\", which the Rust type (check_version) refuses"]
fn the_version_latest_is_refused_by_both() {
    let cases = [
        (json!("latest"), false),
        (json!("Latest"), false),
        (json!("LATEST"), false),
    ];
    assert_no_disagreement(&disagreements("", "version", &cases));
}

/// The half of the case above that holds today: the Rust type refuses
/// the version `latest`, whatever the schema says.
#[test]
fn the_rust_type_refuses_the_version_latest() {
    for version in ["latest", "Latest", "LATEST"] {
        let manifest = changed("", "version", &json!(version));
        let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap());
        let error = parsed.expect_err(version).to_string();
        assert!(error.contains("the newest version"), "{error}");
    }
}

#[test]
fn authorities_and_profiles_agree_case_by_case() {
    let authorities = [
        (json!("did:web:a"), true),
        (json!("did:web:authority.example:dataspace:eu"), true),
        (json!("did:web:authority.example%3A8443"), true),
        (json!("did:web:"), false),
        (json!("did:web:a:"), false),
        (json!("did:web:a::b"), false),
        (json!("did:web:a/b"), false),
        (json!("did:web:a:b/c"), false),
        (json!("did:WEB:a"), false),
        (json!(" did:web:a"), false),
        (json!("did:key:z6Mk"), false),
        (json!("https://authority.example"), false),
    ];
    let mut wrong = disagreements("", "authority", &authorities);
    let profiles = [
        (json!("http://p"), true),
        (json!("https://profile.example/r\u{e9}sum\u{e9}#8"), true),
        (json!("https://"), false),
        (json!("http://"), false),
        (json!("https://a b"), false),
        (json!("https://a\tb"), false),
        (json!("https://a\n"), false),
        (json!(" https://a"), false),
        (json!("HTTPS://a"), false),
        (json!("ftp://a"), false),
        (json!("urn:profile:1"), false),
    ];
    wrong.append(&mut disagreements("", "profile", &profiles));
    // the same rule for the IRIs inside roles
    let access_rights = [
        (json!(["https://a", "http://b"]), true),
        (json!(["https://a", "b"]), false),
        (json!(["https://a b"]), false),
        (json!("https://a"), false),
        (json!([7]), false),
    ];
    wrong.append(&mut disagreements(
        "/files/0/role",
        "accessRights",
        &access_rights,
    ));
    let context_urls = [
        (json!("http://context.example/c.jsonld"), true),
        (json!("context.jsonld"), false),
        (json!(["https://context.example/c.jsonld"]), false),
    ];
    wrong.append(&mut disagreements("/files/4/role", "url", &context_urls));
    assert_no_disagreement(&wrong);
}

#[test]
fn paths_and_digests_agree_case_by_case() {
    let paths = [
        (json!("a"), true),
        (json!("a/b/c.ttl"), true),
        (json!("-a/_b"), true),
        (json!("a..b"), true),
        (json!("x".repeat(200)), true),
        (json!("x".repeat(201)), false),
        (json!(""), false),
        (json!("a//b"), false),
        (json!("a/"), false),
        (json!("/a"), false),
        (json!(".a"), false),
        (json!("a/.b"), false),
        (json!("a/.."), false),
        (json!("./a"), false),
        (json!("a b"), false),
        (json!("a\n"), false),
        (json!("\u{e4}.ttl"), false),
        (json!("C:/a"), false),
        (json!("a%2Fb"), false),
        (json!("manifest.json"), false),
        (json!("manifest.jws"), false),
        // only the two names in the bundle's own folder are reserved
        (json!("shapes/manifest.json"), true),
        (json!("latest.json"), true),
    ];
    let mut wrong = disagreements("/files/0", "path", &paths);
    let digests = [
        (json!("0".repeat(64)), true),
        (json!("0123456789abcdef".repeat(4)), true),
        (json!("0".repeat(63)), false),
        (json!("0".repeat(65)), false),
        (json!(format!("{}g", "0".repeat(63))), false),
        (json!(format!("{} ", "0".repeat(63))), false),
        (json!(format!("sha256:{}", "0".repeat(57))), false),
        (json!(""), false),
    ];
    wrong.append(&mut disagreements("/files/0", "sha256", &digests));
    assert_no_disagreement(&wrong);
}

/// A known role is closed; a role with any other `type` of the allowed
/// shape is valid whatever else it holds.
#[test]
fn roles_agree_case_by_case() {
    let public = "http://publications.europa.eu/resource/authority/access-right/PUBLIC";
    let roles = [
        (json!({"type": "datasets", "accessRights": [public]}), true),
        (json!({"type": "messages"}), true),
        (json!({"type": "vocabulary"}), true),
        (json!({"type": "datasets"}), false),
        (
            json!({"type": "datasets", "accessRights": [public], "url": "https://x"}),
            false,
        ),
        (json!({"type": "messages", "version": 2}), false),
        (
            json!({"type": "vocabulary", "accessRights": [public]}),
            false,
        ),
        // file 0 is Turtle, so it cannot be a context
        (json!({"type": "context", "url": "https://x"}), false),
        (json!({"accessRights": [public]}), false),
        (json!({"type": null}), false),
        (json!({"type": ["messages"]}), false),
        (json!({"type": {"name": "messages"}}), false),
        (json!(null), false),
        (json!(7), false),
        (json!([]), false),
        // unknown types: the shape of the name
        (json!({"type": "x"}), true),
        (json!({"type": "x-1_Y"}), true),
        (json!({"type": "x".repeat(64)}), true),
        (json!({"type": "x".repeat(65)}), false),
        (json!({"type": ""}), false),
        (json!({"type": "1x"}), false),
        (json!({"type": "-x"}), false),
        (json!({"type": "_x"}), false),
        (json!({"type": "x.y"}), false),
        (json!({"type": "x:y"}), false),
        (json!({"type": "x y"}), false),
        (json!({"type": " messages"}), false),
        (json!({"type": "messages "}), false),
        (json!({"type": "r\u{e9}gles"}), false),
        // unknown types: anything else in the object is passed over
        (
            json!({"type": "rules", "accessRights": "not a list", "url": 7, "x": null}),
            true,
        ),
    ];
    assert_no_disagreement(&disagreements("/files/0", "role", &roles));
}

/// Which media type fits which known role, file by file of the example.
#[test]
fn media_types_of_known_roles_agree_case_by_case() {
    let turtle = json!("text/turtle");
    let n_triples = json!("application/n-triples");
    let json_ld = json!("application/ld+json");
    let mut wrong = Vec::new();
    // files 0 to 3: datasets, datasets, messages, vocabulary; 4: context
    for file in 0..5 {
        let is_context = file == 4;
        let cases = [
            (turtle.clone(), !is_context),
            (n_triples.clone(), !is_context),
            (json_ld.clone(), is_context),
            (json!("text/turtle; charset=utf-8"), false),
            (json!("TEXT/TURTLE"), false),
            (json!(" text/turtle"), false),
            (json!("application/json"), false),
        ];
        wrong.append(&mut disagreements(
            &format!("/files/{file}"),
            "mediaType",
            &cases,
        ));
    }
    assert_no_disagreement(&wrong);
}

/// A field that is missing, at each level.
#[test]
fn a_manifest_without_one_of_its_fields_fails_both() {
    let validator = validator();
    let fields = [
        ("", "version"),
        ("", "issued"),
        ("", "authority"),
        ("", "profile"),
        ("", "files"),
        ("/files/0", "path"),
        ("/files/0", "sha256"),
        ("/files/0", "mediaType"),
        ("/files/0", "role"),
        ("/files/0/role", "type"),
        ("/files/0/role", "accessRights"),
        ("/files/4/role", "url"),
    ];
    for (pointer, field) in fields {
        let mut manifest = full_example();
        let object = manifest.pointer_mut(pointer).unwrap();
        object.as_object_mut().unwrap().remove(field).unwrap();
        let both = accepted_by_both(&validator, &manifest);
        assert_eq!(both, (false, false), "without {pointer}/{field}");
    }
}

/// What the schema cannot say and the Rust type checks, as the schema's
/// own `$comment` lists it: a date and time that exists, paths that
/// differ in more than letter case, and one file per context URL.
#[test]
fn what_only_the_rust_type_checks_is_what_the_schema_says_it_cannot() {
    let validator = validator();
    let mut no_such_day = full_example();
    no_such_day["issued"] = json!("2026-02-30T00:00:00Z");
    let mut no_such_hour = full_example();
    no_such_hour["issued"] = json!("2026-11-14T25:00:00Z");
    let mut same_path_in_other_letters = full_example();
    same_path_in_other_letters["files"][1]["path"] = json!("Shapes/Public-Shapes.ttl");
    let mut same_path = full_example();
    same_path["files"][1]["path"] = same_path["files"][0]["path"].clone();
    let mut two_files_for_one_context = full_example();
    let mut second = two_files_for_one_context["files"][4].clone();
    second["path"] = json!("contexts/again.jsonld");
    two_files_for_one_context["files"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let cases = [
        ("no such day", no_such_day),
        ("no such hour", no_such_hour),
        ("one path in other letters", same_path_in_other_letters),
        ("one path twice", same_path),
        ("two files for one context", two_files_for_one_context),
    ];
    for (name, manifest) in cases {
        assert_eq!(
            accepted_by_both(&validator, &manifest),
            (true, false),
            "{name}"
        );
    }
}

/// The name of a role is compared letter for letter. A known role
/// written in other letters (`Datasets`) is therefore an unknown role:
/// the manifest is valid, by the schema and by the Rust type, and the
/// file is skipped, so its shapes are not enforced. The connector says
/// so (a WARN line and a gauge); nothing refuses the bundle. Written
/// down here because it is easy to do by mistake.
#[test]
fn a_known_role_in_other_letters_is_an_unknown_role_and_its_file_is_skipped() {
    let validator = validator();
    for type_name in ["Datasets", "DATASETS", "Messages", "Vocabulary", "Context"] {
        let mut manifest = full_example();
        // the role of file 0 keeps its other field, `accessRights`
        manifest["files"][0]["role"]["type"] = json!(type_name);
        assert_eq!(
            accepted_by_both(&validator, &manifest),
            (true, true),
            "{type_name}"
        );
        let parsed = Manifest::parse(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        let skipped = parsed.files_of_unknown_role();
        assert_eq!(skipped.len(), 1, "{type_name}");
        assert_eq!(skipped[0].path, "shapes/public-shapes.ttl");
        assert_eq!(skipped[0].role_type, type_name);
        assert_eq!(parsed.files_to_fetch().len(), 4);
    }
}

/// The limit of 1,000 files counts every file, also those a reader
/// skips.
#[test]
fn the_limit_of_1000_files_counts_skipped_files_too() {
    let validator = validator();
    let with_skipped_files = |count: usize| {
        let mut manifest = minimal_example();
        let files = manifest["files"].as_array_mut().unwrap();
        for number in 0..count {
            files.push(json!({
                "path": format!("later/{number}.json"),
                "sha256": "0".repeat(64),
                "mediaType": "application/json",
                "role": {"type": "rules"}
            }));
        }
        manifest
    };
    // 1 file with a known role and 999 skipped ones: 1,000
    let at_the_limit = with_skipped_files(999);
    assert_eq!(accepted_by_both(&validator, &at_the_limit), (true, true));
    let parsed = Manifest::parse(&serde_json::to_vec(&at_the_limit).unwrap()).unwrap();
    assert_eq!(parsed.files_to_fetch().len(), 1);
    assert_eq!(parsed.files_of_unknown_role().len(), 999);

    let one_more = with_skipped_files(1_000);
    assert_eq!(accepted_by_both(&validator, &one_more), (false, false));

    // 1,000 files, none with a known role
    let mut none_known = with_skipped_files(1_000);
    none_known["files"].as_array_mut().unwrap().remove(0);
    assert_eq!(accepted_by_both(&validator, &none_known), (false, false));
}

// ---- Many made-up values: every text put together from a few pieces
// ---- that matter (a letter, a digit, each allowed sign, signs that are
// ---- not allowed, the longest length and one more). No random numbers,
// ---- so a failure is the same on every run.

/// Every text made of one or two of `pieces`.
fn one_or_two_of(pieces: &[&str]) -> Vec<String> {
    let mut texts = Vec::new();
    for first in pieces {
        texts.push((*first).to_owned());
        for second in pieces {
            texts.push(format!("{first}{second}"));
        }
    }
    texts
}

/// For every made-up role name: the schema and the Rust type agree with
/// `is_role_type_name` (a known name is valid here only with what its
/// role needs, so the four are left out).
#[test]
fn made_up_role_names_are_judged_the_same_by_the_schema_and_the_rust_type() {
    use portitor_dcp::bundle::is_role_type_name;

    let validator = validator();
    let pieces = [
        "a", "Z", "7", "-", "_", ".", " ", "/", "+", ":", "\u{e9}", "\n", "",
    ];
    let mut names = one_or_two_of(&pieces);
    for length in [63, 64, 65] {
        names.push("r".repeat(length));
        names.push(format!("{}-", "r".repeat(length - 1)));
        names.push(format!("9{}", "r".repeat(length - 1)));
    }
    let mut valid_names = 0;
    for name in &names {
        let manifest = changed("/files/0", "role", &json!({"type": name}));
        let expected = is_role_type_name(name);
        let both = accepted_by_both(&validator, &manifest);
        assert_eq!(both, (expected, expected), "role type {name:?}");
        valid_names += usize::from(expected);
    }
    // the test is about both kinds
    assert!(valid_names > 10, "{valid_names}");
    assert!(names.len() - valid_names > 100, "{}", names.len());
}

/// For every made-up media type of a file with an unknown role: the
/// schema and the Rust type agree with `is_media_type_shape`.
#[test]
fn made_up_media_types_are_judged_the_same_by_the_schema_and_the_rust_type() {
    use portitor_dcp::bundle::is_media_type_shape;

    let validator = validator();
    let longest = "n".repeat(127);
    let too_long = "n".repeat(128);
    // names for the part before and the part after the `/`
    let parts = [
        "a", "Z", "7", "a.b", "a+b", "a-b", "a_b", "a!b", "a#b", "a$b", "a&b", "a^b", ".a", "+a",
        "-a", "a*", "a b", "a;b", "a=b", "a/b", "\u{e9}", " a", "a ", "", &longest, &too_long,
    ];
    let mut media_types = Vec::new();
    for type_name in parts {
        media_types.push(type_name.to_owned());
        for subtype_name in parts {
            media_types.push(format!("{type_name}/{subtype_name}"));
        }
    }
    let mut valid_types = 0;
    for media_type in &media_types {
        let mut manifest = changed("/files/0", "role", &json!({"type": "rules"}));
        manifest["files"][0]["mediaType"] = json!(media_type);
        let expected = is_media_type_shape(media_type);
        let both = accepted_by_both(&validator, &manifest);
        assert_eq!(both, (expected, expected), "media type {media_type:?}");
        valid_types += usize::from(expected);
    }
    assert!(valid_types > 100, "{valid_types}");
    assert!(
        media_types.len() - valid_types > 100,
        "{}",
        media_types.len()
    );
}

// ---- A field given twice. JSON Schema cannot see it (a JSON reader
// ---- hands it one value), so only the Rust type is asked. As for DSP
// ---- messages (0.12), a field given twice is refused: which of the two
// ---- values counts depends on the reader, and the manifest is signed.

/// The minimal example as text, with `"role": {"type": "messages"}`
/// replaced by `role`.
fn minimal_with_role(role: &str) -> String {
    let text = serde_json::to_string(&minimal_example()).unwrap();
    let original = r#"{"type":"messages"}"#;
    assert!(text.contains(original), "{text}");
    text.replace(original, role)
}

#[test]
fn a_field_given_twice_is_refused_at_every_level() {
    let good = serde_json::to_string(&minimal_example()).unwrap();
    assert!(Manifest::parse(good.as_bytes()).is_ok());
    let twice = [
        // the manifest
        good.replacen('{', r#"{"version":"0","#, 1),
        good.replacen('{', r#"{"files":[],"#, 1),
        // a file
        good.replace(r#"{"mediaType""#, r#"{"path":"other.ttl","mediaType""#),
        good.replace(
            r#"{"mediaType""#,
            r#"{"role":{"type":"vocabulary"},"mediaType""#,
        ),
        // a role: its type, with the same value, with another known
        // value, and with an unknown one first or last
        minimal_with_role(r#"{"type":"messages","type":"messages"}"#),
        minimal_with_role(r#"{"type":"messages","type":"vocabulary"}"#),
        minimal_with_role(r#"{"type":"rules","type":"messages"}"#),
        minimal_with_role(r#"{"type":"messages","type":"rules"}"#),
        // a known role's other field
        minimal_with_role(
            r#"{"type":"datasets","accessRights":["https://a"],"accessRights":["https://b"]}"#,
        ),
    ];
    for text in twice {
        assert_ne!(text, good, "the change was not made");
        let parsed = Manifest::parse(text.as_bytes());
        let error = parsed.expect_err(&text).to_string();
        assert!(error.contains("duplicate field"), "{text}: {error}");
    }
}

/// The one place a field may be given twice: inside a role this version
/// does not know, apart from its `type`. Nothing of such a role is read
/// (the file is skipped), so nothing is checked either.
#[test]
fn inside_an_unknown_role_only_the_type_is_looked_at() {
    let mut manifest: Value = serde_json::from_str(&minimal_with_role(
        r#"{"type":"rules","level":1,"level":2}"#,
    ))
    .unwrap();
    // a second file, so that one file has a known role
    let known = minimal_example()["files"][0].clone();
    manifest["files"].as_array_mut().unwrap().push(known);
    manifest["files"][0]["path"] = json!("rules.json");
    let text = serde_json::to_string(&manifest)
        .unwrap()
        .replace(r#""level":2"#, r#""level":1,"level":2"#);
    let parsed = Manifest::parse(text.as_bytes()).unwrap();
    assert_eq!(parsed.files_of_unknown_role().len(), 1);
    // but the type itself may not be given twice
    let type_twice = text.replace(r#""type":"rules""#, r#""type":"rules","type":"rules""#);
    assert_ne!(type_twice, text);
    assert!(Manifest::parse(type_twice.as_bytes()).is_err());
}
