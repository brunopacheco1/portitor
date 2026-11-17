// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The lexical forms of XML Schema datatypes that `sh:datatype` checks
//! (`portitor-shacl`, `lexical.rs`). A harvested dataset or a DSP
//! message can give any text with any of these datatypes. The input is
//! the text of one literal of each datatype; validation must never
//! panic, `xsd:string` (and a datatype the engine has no rules for) is
//! never refused, and an integer that Rust reads in a type's range is
//! never refused for that type.
#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use portitor_shacl::oxrdf::{Graph, Literal, NamedNode, Triple};
use portitor_shacl::{Document, Format, Shapes};

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const FOCUS: &str = "urn:fuzz:focus";
const PROPERTY: &str = "urn:fuzz:property/";

/// Every datatype the engine has lexical rules for, and `gDay`, which
/// it checks by IRI only.
const DATATYPES: [&str; 28] = [
    "string",
    "boolean",
    "decimal",
    "double",
    "float",
    "date",
    "dateTime",
    "dateTimeStamp",
    "time",
    "gYear",
    "gYearMonth",
    "duration",
    "anyURI",
    "hexBinary",
    "integer",
    "nonNegativeInteger",
    "positiveInteger",
    "nonPositiveInteger",
    "negativeInteger",
    "long",
    "int",
    "short",
    "byte",
    "unsignedLong",
    "unsignedInt",
    "unsignedShort",
    "unsignedByte",
    "gDay",
];

/// One property shape per datatype, on one focus node:
/// `<urn:fuzz:property/integer>` must be an `xsd:integer`, and so on.
static SHAPES: LazyLock<Shapes> = LazyLock::new(|| {
    let mut text = String::from("@prefix sh: <http://www.w3.org/ns/shacl#> .\n");
    text.push_str(&format!("<urn:fuzz:shape> sh:targetNode <{FOCUS}> "));
    for datatype in DATATYPES {
        text.push_str(&format!(
            ";\n  sh:property [ sh:path <{PROPERTY}{datatype}> ; sh:datatype <{XSD}{datatype}> ] "
        ));
    }
    text.push_str(".\n");
    let document = Document {
        name: "datatypes.ttl",
        format: Format::Turtle,
        text: &text,
    };
    Shapes::load(&[document], &[]).expect("shapes")
});

fn named(iri: &str) -> NamedNode {
    NamedNode::new(iri).expect("IRI")
}

/// The focus node with `text` as a literal of every datatype.
fn graph_with(text: &str) -> Graph {
    let mut graph = Graph::new();
    for datatype in DATATYPES {
        let literal = Literal::new_typed_literal(text, named(&format!("{XSD}{datatype}")));
        let triple = Triple::new(
            named(FOCUS),
            named(&format!("{PROPERTY}{datatype}")),
            literal,
        );
        graph.insert(&triple);
    }
    graph
}

/// The datatypes whose literal was refused.
fn refused(text: &str) -> Vec<String> {
    let report = SHAPES.validate(&graph_with(text));
    let mut names = Vec::new();
    for result in &report.results {
        let path = result.path.as_deref().unwrap_or_default();
        if let Some(datatype) = path.strip_prefix(PROPERTY) {
            names.push(datatype.to_owned());
        }
    }
    names
}

/// The integer types Rust can read `text` into, in range.
fn integer_types_that_fit(text: &str) -> Vec<&'static str> {
    let mut fit = Vec::new();
    if text.parse::<i64>().is_ok() {
        fit.extend(["integer", "long"]);
    }
    if text.parse::<i32>().is_ok() {
        fit.push("int");
    }
    if text.parse::<i16>().is_ok() {
        fit.push("short");
    }
    if text.parse::<i8>().is_ok() {
        fit.push("byte");
    }
    if text.parse::<u64>().is_ok() {
        fit.extend(["unsignedLong", "nonNegativeInteger"]);
    }
    if text.parse::<u8>().is_ok() {
        fit.push("unsignedByte");
    }
    fit
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let refused = refused(text);
    for always_accepted in ["string", "gDay"] {
        assert!(
            !refused.iter().any(|name| name == always_accepted),
            "{text:?} refused as {always_accepted}"
        );
    }
    for datatype in integer_types_that_fit(text) {
        assert!(
            !refused.iter().any(|name| name == datatype),
            "{text:?} refused as {datatype}"
        );
    }
});
