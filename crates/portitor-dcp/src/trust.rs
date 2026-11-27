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
