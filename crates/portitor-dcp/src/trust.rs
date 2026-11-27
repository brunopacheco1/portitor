// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Which credential types a trusted issuer may issue (R-48, from 1.4).
//!
//! `PORTITOR_TRUSTED_ISSUERS` lists the issuers a component trusts. The
//! optional setting `PORTITOR_TRUSTED_ISSUER_TYPES` limits some of them
//! to a list of credential types, as Eclipse EDC does with
//! `edc.iam.trustedissuer.<alias>.supportedtypes`. An issuer the setting
//! does not name keeps every type, as before 1.4 and as EDC's `"*"`
//! (TD-74 in `docs/RISKS_AND_TECHNICAL_DEBTS.md`: the default changes in
//! 2.0).
//!
//! The setting is a comma-separated list of `<issuer DID>=<credential
//! type>` items. An issuer trusted for two types is named twice:
//! `did:web:hdab.example=DataPermitCredential,did:web:hdab.example=ResearcherCredential`.

use std::collections::BTreeMap;

/// The base type of every credential, short (JWT) and as the expanded
/// JSON-LD IRI. It never limits an issuer ([`IssuerTypes::allows`]).
const BASE_TYPES: [&str; 2] = [
    "VerifiableCredential",
    "https://www.w3.org/2018/credentials#VerifiableCredential",
];

/// The credential types each limited issuer may issue. Read from
/// `PORTITOR_TRUSTED_ISSUER_TYPES` with [`IssuerTypes::parse`]; the
/// default limits no issuer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssuerTypes {
    /// Issuer DID to its types, in the order they were first named.
    limits: BTreeMap<String, Vec<String>>,
}

impl IssuerTypes {
    /// Reads the items of `PORTITOR_TRUSTED_ISSUER_TYPES` (already split
    /// at the commas and trimmed). Each issuer must be one of `trusted`
    /// (`PORTITOR_TRUSTED_ISSUERS`): a limit on an issuer that is not
    /// trusted is most likely a typing error.
    ///
    /// # Errors
    ///
    /// An item is not `<DID>=<type>`, names an issuer that is not in
    /// `trusted`, or names a type with a space, a `=` or a `:` in it.
    pub fn parse(items: &[String], trusted: &[String]) -> Result<Self, String> {
        let mut limits: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for item in items {
            let Some((issuer, credential_type)) = item.split_once('=') else {
                return Err(format!("{item:?} is not <issuer DID>=<credential type>"));
            };
            let issuer = issuer.trim();
            let credential_type = credential_type.trim();
            if !trusted.iter().any(|known| known == issuer) {
                return Err(format!("{issuer:?} is not in PORTITOR_TRUSTED_ISSUERS"));
            }
            let bad_type = credential_type.is_empty()
                || credential_type.contains(|c: char| c.is_whitespace() || c == '=' || c == ':');
            if bad_type {
                return Err(format!(
                    "{credential_type:?} is not a credential type, as in {issuer}=MembershipCredential"
                ));
            }
            let types = limits.entry(issuer.to_owned()).or_default();
            if !types.iter().any(|known| known == credential_type) {
                types.push(credential_type.to_owned());
            }
        }
        Ok(Self { limits })
    }

    /// The types `issuer` is limited to; `None` when the setting does not
    /// name it, so it keeps every type.
    #[must_use]
    pub fn limit_of(&self, issuer: &str) -> Option<&[String]> {
        self.limits.get(issuer).map(Vec::as_slice)
    }

    /// Whether `issuer`, a trusted issuer, may issue a credential with
    /// the types `credential_types` (its `type` list).
    ///
    /// The rule is Eclipse EDC's `HasValidIssuer` (default mode, read in
    /// `core/common/lib/core-lib/src/main/java/org/eclipse/edc/iam/verifiablecredentials/rules/HasValidIssuer.java`
    /// at EDC Connector commit `4215744`, 2026-11-27):
    ///
    /// - an issuer the setting does not name may issue every type;
    /// - otherwise **every** type of the credential must be in the
    ///   issuer's list, except the base type `VerifiableCredential`,
    ///   which never limits. So a credential typed
    ///   `["VerifiableCredential", "DataPermitCredential"]` needs
    ///   `DataPermitCredential`, and one that also says
    ///   `MembershipCredential` needs both;
    /// - a credential with only the base type needs the base type in the
    ///   list, and one with no type at all is never allowed.
    #[must_use]
    pub fn allows(&self, issuer: &str, credential_types: &[String]) -> bool {
        let Some(limit) = self.limit_of(issuer) else {
            return true;
        };
        let mut to_check: Vec<&String> = Vec::new();
        for credential_type in credential_types {
            if !BASE_TYPES.contains(&credential_type.as_str()) {
                to_check.push(credential_type);
            }
        }
        // only the base type: then the base type is what is checked, as
        // in EDC
        if to_check.is_empty() {
            to_check = credential_types.iter().collect();
        }
        if to_check.is_empty() {
            return false;
        }
        to_check
            .iter()
            .all(|credential_type| limit.contains(*credential_type))
    }

    /// Whether no issuer is limited (the setting is not set).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.limits.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn items(text: &[&str]) -> Vec<String> {
        text.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn an_issuer_named_twice_gets_both_types_and_others_keep_every_type() {
        let trusted = items(&["did:web:authority", "did:web:hdab"]);
        let types = IssuerTypes::parse(
            &items(&[
                "did:web:hdab=DataPermitCredential",
                "did:web:hdab = ResearcherCredential",
                "did:web:hdab=DataPermitCredential",
            ]),
            &trusted,
        )
        .unwrap();
        assert_eq!(
            types.limit_of("did:web:hdab").unwrap(),
            ["DataPermitCredential", "ResearcherCredential"]
        );
        assert_eq!(types.limit_of("did:web:authority"), None);
        assert!(!types.is_empty());
        assert!(IssuerTypes::parse(&[], &trusted).unwrap().is_empty());
    }

    #[test]
    fn a_limited_issuer_needs_every_type_but_the_base_type_as_in_edc() {
        let trusted = items(&["did:web:authority", "did:web:hdab"]);
        let types =
            IssuerTypes::parse(&items(&["did:web:hdab=DataPermitCredential"]), &trusted).unwrap();
        let permit = items(&["VerifiableCredential", "DataPermitCredential"]);
        let membership = items(&["VerifiableCredential", "MembershipCredential"]);
        let both = items(&[
            "VerifiableCredential",
            "DataPermitCredential",
            "MembershipCredential",
        ]);
        assert!(types.allows("did:web:hdab", &permit));
        assert!(!types.allows("did:web:hdab", &membership));
        assert!(!types.allows("did:web:hdab", &both));
        assert!(!types.allows("did:web:hdab", &items(&["VerifiableCredential"])));
        assert!(!types.allows("did:web:hdab", &[]));
        // an issuer the setting does not name keeps every type
        assert!(types.allows("did:web:authority", &membership));
        assert!(types.allows("did:web:authority", &both));
        let base =
            IssuerTypes::parse(&items(&["did:web:hdab=VerifiableCredential"]), &trusted).unwrap();
        assert!(base.allows("did:web:hdab", &items(&["VerifiableCredential"])));
        assert!(!base.allows("did:web:hdab", &permit));
    }

    #[test]
    fn a_wrong_item_is_refused_with_the_reason() {
        let trusted = items(&["did:web:hdab"]);
        for (item, reason) in [
            ("did:web:hdab", "is not <issuer DID>=<credential type>"),
            (
                "did:web:other=DataPermitCredential",
                "is not in PORTITOR_TRUSTED_ISSUERS",
            ),
            ("did:web:hdab=", "is not a credential type"),
            ("did:web:hdab=Data Permit", "is not a credential type"),
            (
                "did:web:hdab=org.eclipse.dspace.dcp.vc.type:X",
                "is not a credential type",
            ),
        ] {
            let error = IssuerTypes::parse(&items(&[item]), &trusted).unwrap_err();
            assert!(error.contains(reason), "{item}: {error}");
        }
    }
}
