// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Access policies (ADR 0019, amendment in 1.4, "Access policy per
//! dataset"; TD-13): who may see a dataset in the catalog. Eclipse EDC
//! builds its catalog from the contract definitions whose access policy
//! holds for the caller; Portitor does the same per dataset.
//!
//! An access policy has only permissions, read strictly as an offer's
//! (ADR 0009): `{"permission": [{"action": "use", "constraint": [...]}]}`.
//! It uses the constraints offers use, `dateTime` and credential
//! constraints. A prohibition, a duty or a `purpose` constraint is not
//! understood: a purpose is a term of an agreement, and nothing is agreed
//! when a catalog is read.

use serde::{Deserialize, Serialize};

use super::{CallerCredentials, Constraint, CredentialTypes, PolicyError, Terms, short};
use crate::Rule;

/// The access policy of a dataset (`portitor:accessPolicy` in the
/// datasets file). Read strictly: a field not listed here (a
/// `prohibition`, an `obligation`, a `duty` inside a permission) is an
/// error, never dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessPolicy {
    /// Permissions; all of them must hold.
    pub permission: Vec<Rule>,
}

impl<'a> From<&'a AccessPolicy> for Terms<'a> {
    fn from(policy: &'a AccessPolicy) -> Self {
        Self {
            permission: &policy.permission,
            prohibition: &[],
            obligation: &[],
            profile: None,
        }
    }
}

/// Checks that `policy` is understood, without evaluating it (when the
/// datasets are harvested): at least one permission, no `purpose`
/// constraint, and every constraint understood with the credential
/// `types`.
///
/// # Errors
///
/// What is not understood.
pub fn validate_access_policy(
    policy: &AccessPolicy,
    types: &CredentialTypes,
) -> Result<(), PolicyError> {
    if policy.permission.is_empty() {
        return Err(PolicyError::Unsupported(
            "an access policy without a permission".to_owned(),
        ));
    }
    for constraint in policy.permission.iter().flat_map(|rule| &rule.constraint) {
        if names_a_purpose(constraint) {
            return Err(PolicyError::Unsupported(
                "a purpose constraint in an access policy (a purpose is a term of an agreement)"
                    .to_owned(),
            ));
        }
    }
    for constraint in policy.permission.iter().flat_map(|rule| &rule.constraint) {
        super::check(constraint, &super::Mode::Validate(types))?;
    }
    Ok(())
}

/// Whether `policy` holds at `now` for a caller with `credentials`. A
/// policy that is not understood never holds (fail closed).
///
/// # Errors
///
/// The policy is not understood, or does not hold for this caller now.
pub fn access_policy_holds(
    policy: &AccessPolicy,
    now: &str,
    credentials: &CallerCredentials,
) -> Result<(), PolicyError> {
    validate_access_policy(policy, &credentials.understood)?;
    super::permissions_hold(policy, now, credentials)
}

/// Whether `constraint`, or a part of it, has the left operand `purpose`.
fn names_a_purpose(constraint: &Constraint) -> bool {
    match constraint {
        Constraint::Atomic(atomic) => short(&atomic.left_operand) == Some("purpose"),
        Constraint::Logical(logical) => {
            let (_name, parts) = super::logical_parts(logical);
            parts.iter().any(names_a_purpose)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::policy::{HeldCredential, MEMBERSHIP, credential_scopes};

    const NOW: &str = "2026-11-27T12:00:00Z";
    const PERMIT: &str = "DataPermitCredential";
    const PERMIT_SCOPE: &str = "org.eclipse.dspace.dcp.vc.type:DataPermitCredential:read";

    fn types() -> CredentialTypes {
        let mut types = CredentialTypes::default();
        types.add(PERMIT, PERMIT_SCOPE);
        types
    }

    fn read(value: serde_json::Value) -> Result<AccessPolicy, serde_json::Error> {
        serde_json::from_value(value)
    }

    fn researchers_only() -> AccessPolicy {
        read(json!({"permission": [{"action": "use", "constraint": [
            {"leftOperand": "DataPermitCredential.role", "operator": "eq",
             "rightOperand": "researcher"},
            {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2030-01-01"}
        ]}]}))
        .unwrap()
    }

    fn caller(role: &str) -> CallerCredentials {
        let serde_json::Value::Object(subject) = json!({"role": role}) else {
            panic!("an object")
        };
        CallerCredentials {
            understood: types(),
            held: vec![HeldCredential {
                types: vec!["VerifiableCredential".to_owned(), PERMIT.to_owned()],
                subjects: vec![subject],
            }],
        }
    }

    #[test]
    fn holds_only_for_a_caller_that_meets_it() {
        let policy = researchers_only();
        assert_eq!(validate_access_policy(&policy, &types()), Ok(()));
        assert_eq!(
            access_policy_holds(&policy, NOW, &caller("researcher")),
            Ok(())
        );
        assert!(matches!(
            access_policy_holds(&policy, NOW, &caller("visitor")),
            Err(PolicyError::NotSatisfied(_))
        ));
        assert!(matches!(
            access_policy_holds(&policy, "2031-01-01T00:00:00Z", &caller("researcher")),
            Err(PolicyError::NotSatisfied(_))
        ));
        // the scopes to ask the caller's credential service for
        assert_eq!(
            credential_scopes(&policy, &types()),
            vec![PERMIT_SCOPE.to_owned()]
        );
        // a membership constraint is a credential constraint too
        let members: AccessPolicy = read(json!({"permission": [{"action": "use", "constraint": [
            {"leftOperand": format!("{MEMBERSHIP}.role"), "operator": "eq",
             "rightOperand": "provider"}]}]}))
        .unwrap();
        assert_eq!(validate_access_policy(&members, &types()), Ok(()));
    }

    #[test]
    fn what_is_not_understood_is_refused() {
        // read strictly: prohibitions, duties and unknown fields
        for refused in [
            json!({"permission": [{"action": "use"}], "prohibition": [{"action": "use"}]}),
            json!({"permission": [{"action": "use"}], "obligation": [{"action": "use"}]}),
            json!({"permission": [{"action": "use", "duty": [{"action": "notify"}]}]}),
            json!({"permission": [{"action": "use"}], "@type": "Set"}),
            json!({"rules": []}),
            json!([]),
        ] {
            assert!(read(refused.clone()).is_err(), "{refused}");
        }
        // read, but not understood
        for refused in [
            json!({"permission": []}),
            json!({"permission": [{"action": "use", "constraint": [
                {"leftOperand": "purpose", "operator": "eq", "rightOperand": "https://example.com/p"}]}]}),
            json!({"permission": [{"action": "use", "constraint": [{"or": [
                {"leftOperand": "dateTime", "operator": "lt", "rightOperand": "2030-01-01"},
                {"leftOperand": "odrl:purpose", "operator": "eq", "rightOperand": "https://example.com/p"}]}]}]}),
            json!({"permission": [{"action": "use", "constraint": [
                {"leftOperand": "OtherCredential.role", "operator": "eq", "rightOperand": "x"}]}]}),
            json!({"permission": [{"action": "use", "constraint": [
                {"leftOperand": "spatial", "operator": "eq", "rightOperand": "LU"}]}]}),
        ] {
            let policy = read(refused.clone()).unwrap();
            assert!(
                matches!(
                    validate_access_policy(&policy, &types()),
                    Err(PolicyError::Unsupported(_))
                ),
                "{refused}"
            );
            assert!(matches!(
                access_policy_holds(&policy, NOW, &caller("researcher")),
                Err(PolicyError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn round_trips_as_json() {
        let policy = researchers_only();
        let text = serde_json::to_string(&policy).unwrap();
        assert_eq!(read(serde_json::from_str(&text).unwrap()).unwrap(), policy);
    }
}
