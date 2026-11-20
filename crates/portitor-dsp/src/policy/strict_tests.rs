// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Policies are read strictly (ADR 0009, amendment of 2026-11-18): a
//! field that Portitor does not read is an error that names the field,
//! for each policy object.

#![allow(clippy::unwrap_used)]

use serde_json::{Value, json};

use super::{check_parties, same_terms};
use crate::Offer;
use crate::negotiation::Agreement;
use crate::negotiation::{ContractAgreementMessage, ContractOfferMessage, ContractRequestMessage};

/// An offer with every field Portitor reads, at every level.
fn full_offer() -> Value {
    json!({
        "@id": "o", "@type": "Offer", "target": "d",
        "assigner": "did:web:p", "assignee": "did:web:c",
        "profile": ["https://example.com/profile"],
        "permission": [{"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2030-12-31"},
            {"and": [
                {"leftOperand": "dateTime", "operator": "gt", "rightOperand": "2020-01-01"},
                {"or": [{"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2031-01-01"}]},
                {"xone": []}, {"andSequence": []}
            ]}
        ]}],
        "prohibition": [{"action": "distribute"}],
        "obligation": [{"action": "delete", "constraint": []}]
    })
}

/// An agreement with every field Portitor reads.
fn full_agreement() -> Value {
    let mut agreement = full_offer();
    agreement["@type"] = json!("Agreement");
    agreement["timestamp"] = json!("2026-11-18T10:00:00Z");
    agreement
}

/// The error of reading `value` as an offer.
fn offer_error(value: &Value) -> String {
    serde_json::from_value::<Offer>(value.clone())
        .unwrap_err()
        .to_string()
}

#[test]
fn every_known_field_is_read_and_written_back() {
    let offer: Offer = serde_json::from_value(full_offer()).unwrap();
    assert_eq!(serde_json::to_value(&offer).unwrap(), {
        // an empty constraint list is left out when written
        let mut expected = full_offer();
        expected["obligation"][0]
            .as_object_mut()
            .unwrap()
            .remove("constraint");
        expected
    });
    assert_eq!(offer.assigner.as_deref(), Some("did:web:p"));
    assert_eq!(offer.assignee.as_deref(), Some("did:web:c"));

    let agreement: Agreement = serde_json::from_value(full_agreement()).unwrap();
    assert_eq!(agreement.timestamp.as_deref(), Some("2026-11-18T10:00:00Z"));
    assert!(same_terms(&offer, &agreement));
}

#[test]
fn an_unknown_field_of_an_offer_is_refused_by_name() {
    let mut offer = full_offer();
    offer["uid"] = json!("urn:x");
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `uid`"), "{error}");

    // an agreement's `timestamp` is not a field of an offer
    let mut offer = full_offer();
    offer["timestamp"] = json!("2026-11-18T10:00:00Z");
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `timestamp`"), "{error}");

    // a misspelled key
    let mut offer = full_offer();
    offer["permision"] = json!([]);
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `permision`"), "{error}");
}

#[test]
fn an_unknown_field_of_an_agreement_is_refused_by_name() {
    let mut agreement = full_agreement();
    agreement["inheritFrom"] = json!("urn:policy:1");
    let error = serde_json::from_value::<Agreement>(agreement)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `inheritFrom`"), "{error}");
}

#[test]
fn a_duty_inside_a_permission_is_refused() {
    // ODRL 2.2 defines `duty` on a permission, and the DSP context names
    // it (odrl.jsonld); Portitor cannot enforce it, so it is not read
    let mut offer = full_offer();
    offer["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `duty`"), "{error}");

    let mut agreement = full_agreement();
    agreement["permission"][0]["duty"] = json!([{"action": "attribute"}]);
    let error = serde_json::from_value::<Agreement>(agreement)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `duty`"), "{error}");
}

#[test]
fn an_unknown_field_of_a_rule_is_refused_by_name() {
    for (list, field) in [
        ("permission", "assignee"),
        ("prohibition", "remedy"),
        ("obligation", "consequence"),
    ] {
        let mut offer = full_offer();
        offer[list][0][field] = json!("x");
        let error = offer_error(&offer);
        assert!(
            error.contains(&format!("unknown field `{field}`")),
            "{list}: {error}"
        );
    }
}

#[test]
fn an_unknown_field_of_an_atomic_constraint_is_refused_by_name() {
    let mut offer = full_offer();
    offer["permission"][0]["constraint"][0]["unit"] = json!("http://example.com/unit");
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `unit`"), "{error}");

    // also inside a logical constraint, at any depth
    let mut offer = full_offer();
    offer["permission"][0]["constraint"][1]["and"][1]["or"][0]["dataType"] = json!("xsd:date");
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `dataType`"), "{error}");

    // and in a prohibition or a duty, whose constraints are not evaluated
    let mut offer = full_offer();
    offer["obligation"][0]["constraint"] = json!([{"leftOperand": "count", "operator": "lteq",
        "rightOperand": "1", "unit": "x"}]);
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `unit`"), "{error}");
}

#[test]
fn an_unknown_field_of_a_logical_constraint_is_refused_by_name() {
    let mut offer = full_offer();
    offer["permission"][0]["constraint"][1]["uid"] = json!("urn:c");
    let error = offer_error(&offer);
    assert!(
        error.contains("unknown field `uid` in a logical constraint"),
        "{error}"
    );

    // two operands in one logical constraint
    let mut offer = full_offer();
    offer["permission"][0]["constraint"] = json!([{"and": [], "or": []}]);
    let error = offer_error(&offer);
    assert!(error.contains("exactly one of"), "{error}");

    // an atomic constraint with a logical operand added
    let mut offer = full_offer();
    offer["permission"][0]["constraint"][0]["or"] = json!([]);
    let error = offer_error(&offer);
    assert!(error.contains("unknown field `"), "{error}");

    // not an object, a missing field
    let mut offer = full_offer();
    offer["permission"][0]["constraint"] = json!(["dateTime"]);
    let error = offer_error(&offer);
    assert!(
        error.contains("a constraint must be a JSON object"),
        "{error}"
    );
    let mut offer = full_offer();
    offer["permission"][0]["constraint"] = json!([{"leftOperand": "dateTime", "operator": "lt"}]);
    let error = offer_error(&offer);
    assert!(error.contains("missing field `rightOperand`"), "{error}");
}

#[test]
fn a_message_with_an_unknown_policy_term_is_refused_by_name() {
    let context = json!(["https://w3id.org/dspace/2025/1/context.jsonld"]);
    let mut with_duty = full_offer();
    with_duty["permission"][0]["duty"] = json!([{"action": "attribute"}]);

    let request = json!({"@context": context, "@type": "ContractRequestMessage",
        "consumerPid": "c", "callbackAddress": "https://c.example", "offer": with_duty});
    let error = serde_json::from_value::<ContractRequestMessage>(request)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `duty`"), "{error}");

    let offer = json!({"@context": context, "@type": "ContractOfferMessage",
        "providerPid": "p", "consumerPid": "c", "offer": with_duty});
    let error = serde_json::from_value::<ContractOfferMessage>(offer)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `duty`"), "{error}");

    let mut agreement = full_agreement();
    agreement["permission"][0]["constraint"][0]["unit"] = json!("x");
    let message = json!({"@context": context, "@type": "ContractAgreementMessage",
        "providerPid": "p", "consumerPid": "c", "agreement": agreement});
    let error = serde_json::from_value::<ContractAgreementMessage>(message)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `unit`"), "{error}");

    // a field of the message itself is not a policy term: Eclipse EDC
    // sends an `@id` on each message, and it is still accepted
    let message = json!({"@context": context, "@id": "urn:uuid:1",
        "@type": "ContractRequestMessage", "consumerPid": "c", "offer": full_offer()});
    assert!(serde_json::from_value::<ContractRequestMessage>(message).is_ok());
}

/// What Eclipse EDC 1.0.0-RC7 writes into a policy
/// (`JsonObjectFromPolicyTransformer`), compacted with the DSP 2025-1
/// context: empty rule lists, `assigner`, `assignee`, `target`, and on an
/// agreement the `timestamp`.
#[test]
fn the_fields_eclipse_edc_sends_are_accepted() {
    let offer = json!({"@id": "b2ZmZXI=:YXNzZXQ=:dXVpZA==", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2030-12-31"}]}],
        "prohibition": [], "obligation": [],
        "assigner": "did:web:provider", "target": "asset-1"});
    let offer: Offer = serde_json::from_value(offer).unwrap();
    assert_eq!(offer.assigner.as_deref(), Some("did:web:provider"));

    let agreement = json!({"@id": "a", "@type": "Agreement",
        "permission": [{"action": "use"}], "prohibition": [], "obligation": [],
        "assignee": "did:web:consumer", "assigner": "did:web:provider",
        "target": "asset-1", "timestamp": "2026-11-18T10:00:00Z"});
    assert!(serde_json::from_value::<Agreement>(agreement).is_ok());

    // what EDC writes only when its policy has them, and Portitor does
    // not read: refused, not dropped
    for (field, value) in [
        ("duty", json!([{"action": "use"}])),
        ("odrl:remedy", json!([{"action": "use"}])),
    ] {
        let mut permission = json!({"action": "use"});
        permission[field] = value;
        let raw = json!({"@id": "o", "@type": "Offer", "permission": [permission]});
        let error = offer_error(&raw);
        assert!(error.contains(field), "{field}: {error}");
    }
    // an action with a refinement is an object, not a name
    let raw = json!({"@id": "o", "@type": "Offer", "permission": [
        {"action": {"action": "use", "refinement": []}}]});
    assert!(serde_json::from_value::<Offer>(raw).is_err());
}

/// Finding D1 of the 2026-11-18 review: an offer with `assigner`, `uid`,
/// a `duty` inside a permission and a constraint `unit` was read without
/// error, only `action` and `constraint` were left, and `same_terms`
/// found it equal to the plain offer. It can no longer be read, so there
/// is nothing reduced to compare.
#[test]
fn same_terms_cannot_be_fooled_by_dropped_terms() {
    let plain = json!({"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2030-12-31"}]}]});
    let plain: Offer = serde_json::from_value(plain).unwrap();

    let d1 = json!({"@id": "o", "@type": "Offer", "assigner": "did:web:p", "uid": "urn:o",
        "permission": [{"action": "use",
            "duty": [{"action": "compensate"}],
            "constraint": [{"leftOperand": "dateTime", "operator": "lteq",
                "rightOperand": "2030-12-31", "unit": "http://example.com/unit"}]}]});
    assert!(serde_json::from_value::<Offer>(d1.clone()).is_err());

    // each extra term alone is refused too, so none reaches `same_terms`
    for pointer in [
        "/uid",
        "/permission/0/duty",
        "/permission/0/constraint/0/unit",
    ] {
        let mut only_one = serde_json::to_value(&plain).unwrap();
        let extra = d1.pointer(pointer).unwrap().clone();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        only_one.pointer_mut(parent).unwrap()[field] = extra;
        let error = offer_error(&only_one);
        assert!(
            error.contains(&format!("unknown field `{field}`")),
            "{error}"
        );
    }

    // `assigner` is read and kept; the negotiation checks it
    let mut with_assigner = plain.clone();
    with_assigner.assigner = Some("did:web:p".to_owned());
    assert_eq!(
        serde_json::to_value(&with_assigner).unwrap()["assigner"],
        "did:web:p"
    );
    assert_eq!(
        check_parties(&with_assigner, "did:web:p", "did:web:c"),
        Ok(())
    );
    assert_eq!(
        check_parties(&with_assigner, "did:web:other", "did:web:c"),
        Err("the offer's assigner is not the provider")
    );
    with_assigner.assignee = Some("did:web:eve".to_owned());
    assert_eq!(
        check_parties(&with_assigner, "did:web:p", "did:web:c"),
        Err("the offer's assignee is not the consumer")
    );
    // an offer that names no party passes
    assert_eq!(check_parties(&plain, "did:web:p", "did:web:c"), Ok(()));
}

/// The error of reading the JSON text `raw` as an offer; `None` when it
/// is read. Text, not `json!`: only text can hold one key twice.
fn offer_text_error(raw: &str) -> Option<String> {
    match serde_json::from_str::<Offer>(raw) {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}

/// A field given twice would have one of its two values dropped unseen
/// (JSON readers disagree on which one). On an offer and on a rule it is
/// refused by name.
#[test]
fn a_field_given_twice_in_an_offer_or_a_rule_is_refused() {
    let twice_in_the_offer = r#"{"@id": "o", "@type": "Offer",
        "permission": [{"action": "use"}],
        "permission": [{"action": "distribute"}]}"#;
    let error = offer_text_error(twice_in_the_offer).unwrap();
    assert!(error.contains("duplicate field `permission`"), "{error}");

    let twice_in_the_rule = r#"{"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "action": "distribute"}]}"#;
    let error = offer_text_error(twice_in_the_rule).unwrap();
    assert!(error.contains("duplicate field `action`"), "{error}");
}

/// The same inside a constraint: the constraint is read through a
/// `serde_json::Value` (`read_constraint`), which keeps the last value of
/// a repeated key and says nothing.
#[test]
fn a_field_given_twice_in_a_constraint_is_refused() {
    let twice_in_an_atomic_constraint = r#"{"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [
            {"leftOperand": "dateTime", "operator": "lteq",
             "rightOperand": "2020-12-31", "rightOperand": "2099-12-31"}]}]}"#;
    let error = offer_text_error(twice_in_an_atomic_constraint);
    assert!(error.is_some(), "read, and one `rightOperand` was dropped");

    let twice_in_a_logical_constraint = r#"{"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": [{
            "and": [{"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2020-12-31"}],
            "and": []}]}]}"#;
    let error = offer_text_error(twice_in_a_logical_constraint);
    assert!(error.is_some(), "read, and one `and` was dropped");
}

/// `null` is not an empty list: a rule list or a constraint list that is
/// `null` is refused. An empty list is read, and means the same as a
/// list that is left out.
#[test]
fn a_null_list_is_refused_and_an_empty_list_is_read() {
    for pointer in [
        "/permission",
        "/prohibition",
        "/obligation",
        "/permission/0/constraint",
        "/permission/0/constraint/1/and",
    ] {
        let mut offer = full_offer();
        *offer.pointer_mut(pointer).unwrap() = Value::Null;
        let error = offer_error(&offer);
        assert!(error.contains("invalid type: null"), "{pointer}: {error}");
    }

    let with_empty_lists = json!({"@id": "o", "@type": "Offer",
        "permission": [{"action": "use", "constraint": []}],
        "prohibition": [], "obligation": []});
    let left_out = json!({"@id": "o", "@type": "Offer", "permission": [{"action": "use"}]});
    let with_empty_lists: Offer = serde_json::from_value(with_empty_lists).unwrap();
    let left_out: Offer = serde_json::from_value(left_out).unwrap();
    assert_eq!(with_empty_lists, left_out);
}

/// A text field that is `null` is read as left out, so nothing is
/// dropped: the offer names no party and no target.
#[test]
fn a_null_text_field_is_read_as_left_out() {
    let mut offer = full_offer();
    for field in ["target", "assigner", "assignee"] {
        offer[field] = Value::Null;
    }
    let offer: Offer = serde_json::from_value(offer).unwrap();
    assert_eq!(offer.target, None);
    assert_eq!(offer.assigner, None);
    assert_eq!(offer.assignee, None);
}

/// DSP 2025-1 `contract-schema.json`: `permission`, `constraint` and the
/// operands of a logical constraint are arrays. One object in their
/// place (as JSON-LD compaction writes a list of one without the DSP
/// context) is refused, not read as a list of one and not dropped.
#[test]
fn one_object_in_place_of_a_list_is_refused() {
    let constraint =
        json!({"leftOperand": "dateTime", "operator": "lteq", "rightOperand": "2030-12-31"});
    for (pointer, one_object) in [
        ("/permission", json!({"action": "use"})),
        ("/permission/0/constraint", constraint.clone()),
        ("/permission/0/constraint/1/and", constraint),
    ] {
        let mut offer = full_offer();
        *offer.pointer_mut(pointer).unwrap() = one_object;
        let error = offer_error(&offer);
        assert!(error.contains("expected a sequence"), "{pointer}: {error}");
    }
}

/// `@type` must be exactly `Offer` on an offer and `Agreement` on an
/// agreement (DSP 2025-1 `contract-schema.json`, `const`): no prefix, no
/// other ODRL policy type, no list, and it cannot be left out.
#[test]
fn only_the_exact_policy_type_is_read() {
    for other_type in [
        json!("odrl:Offer"),
        json!("http://www.w3.org/ns/odrl/2/Offer"),
        json!("offer"),
        json!("Set"),
        json!("Agreement"),
        json!(["Offer"]),
        Value::Null,
    ] {
        let mut offer = full_offer();
        offer["@type"] = other_type.clone();
        assert!(
            serde_json::from_value::<Offer>(offer).is_err(),
            "{other_type}"
        );
    }
    let mut without_type = full_offer();
    without_type.as_object_mut().unwrap().remove("@type");
    assert!(offer_error(&without_type).contains("missing field `@type`"));

    let mut agreement = full_agreement();
    agreement["@type"] = json!("Offer");
    assert!(serde_json::from_value::<Agreement>(agreement).is_err());
}

/// JSON-LD keywords are fields like any other: on a rule or a
/// constraint, where the DSP schema defines none, they are refused by
/// name.
#[test]
fn a_json_ld_keyword_on_a_rule_or_a_constraint_is_refused_by_name() {
    for (pointer, keyword) in [
        ("/permission/0", "@type"),
        ("/prohibition/0", "@id"),
        ("/permission/0/constraint/0", "@type"),
        ("/permission/0/constraint/1", "@id"),
        ("", "@context"),
    ] {
        let mut offer = full_offer();
        offer.pointer_mut(pointer).unwrap()[keyword] = json!("x");
        let error = offer_error(&offer);
        assert!(
            error.contains(&format!("unknown field `{keyword}`")),
            "{pointer}: {error}"
        );
    }
}

/// An agreement is read with the same rule and constraint types as an
/// offer: an unknown field is refused by name at every level of it.
#[test]
fn an_unknown_field_at_any_level_of_an_agreement_is_refused_by_name() {
    for (pointer, field) in [
        ("/permission/0", "assignee"),
        ("/prohibition/0", "remedy"),
        ("/obligation/0", "consequence"),
        ("/permission/0/constraint/0", "unit"),
        ("/permission/0/constraint/1", "uid"),
        ("/permission/0/constraint/1/and/1/or/0", "dataType"),
    ] {
        let mut agreement = full_agreement();
        agreement.pointer_mut(pointer).unwrap()[field] = json!("x");
        let error = serde_json::from_value::<Agreement>(agreement)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("unknown field `{field}`")),
            "{pointer}: {error}"
        );
    }
}
