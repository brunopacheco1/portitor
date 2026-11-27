// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The check of one harvested entry (ADR 0020, "A harvest run", step 3):
//! the [`DatasetCheck`] port, the DSP and ODRL rules ([`DspCheck`]), and
//! [`BothChecks`] to add another check after them.

use std::sync::Arc;

use portitor_dsp::catalog::{DatasetDefinitionError, prepare_dataset};
use portitor_dsp::policy::{AccessPolicy, CredentialTypes, validate_access_policy};
use portitor_dsp::{Dataset, Offer};
use serde::Deserialize;

/// The version of the code that checks entries: this crate's version.
/// [`DspCheck::rules_version`] names it, so after an upgrade of Portitor
/// (which may change the DSP, ODRL or SHACL checks) the stored datasets
/// are checked again at start. It is named once, in [`DspCheck`]: the
/// SHACL check always runs after it ([`BothChecks`] joins both versions).
pub const CHECKS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The key, in an entry, of the dataset's access policy (ADR 0019,
/// amendment in 1.4): who may see the dataset in the catalog. The DSP
/// check reads it and takes it out of the dataset, so it never reaches
/// the catalog.
pub const ACCESS_POLICY_KEY: &str = "portitor:accessPolicy";

/// Which check rejected an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// The entry is not a JSON object.
    Json,
    /// The DSP `Dataset` form or the DSP catalog rules (including a bad
    /// `portitor:source` and an `@id` used twice).
    Dsp,
    /// An offer or the access policy is not understood: a field Portitor
    /// does not read (ADR 0009, amendment of 2026-11-18) or a constraint
    /// it cannot evaluate (ADR 0009, ADR 0019).
    Odrl,
    /// The SHACL shapes of the bundle (ADR 0005).
    Shacl,
}

impl CheckKind {
    /// Every value, for [`CheckKind::from_name`]; the names are in
    /// [`CheckKind::name`] only.
    const ALL: [Self; 4] = [Self::Json, Self::Dsp, Self::Odrl, Self::Shacl];

    /// The name stored and shown: `json`, `dsp`, `odrl` or `shacl`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Dsp => "dsp",
            Self::Odrl => "odrl",
            Self::Shacl => "shacl",
        }
    }

    /// The kind with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.name() == name)
    }
}

/// Why a check rejected an entry.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryFailure {
    /// The check that failed.
    pub kind: CheckKind,
    /// What is wrong, in one line.
    pub message: String,
    /// The check's full report, kept with the error (a SHACL validation
    /// report as JSON), if any.
    pub report: Option<serde_json::Value>,
}

impl EntryFailure {
    /// A failure of `kind` without a report.
    #[must_use]
    pub fn new(kind: CheckKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            report: None,
        }
    }
}

/// An entry that passed a check.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedDataset {
    /// The dataset as the catalog will show it.
    pub dataset: Dataset,
    /// A shape or profile the dataset conforms to (an IRI); the catalog
    /// shows it as `dct:conformsTo`.
    pub conforms_to: Option<String>,
    /// Who may see the dataset (`portitor:accessPolicy`); `None`: every
    /// member. Never shown in the catalog.
    pub access_policy: Option<AccessPolicy>,
}

/// Port: the check of one harvested entry.
pub trait DatasetCheck: Send + Sync {
    /// Names the rules in force, and the version of the code that checks
    /// them (for example `dsp(...; portitor 0.7.0)` or
    /// `dsp(...)+bundle:<version>`). When it differs from the one stored
    /// with the copy, the stored datasets are checked again at start
    /// (trigger `bundle`).
    fn rules_version(&self) -> String;

    /// Checks one entry: the dataset JSON as the source gave it, without
    /// `portitor:source`. It may hold `portitor:accessPolicy`, which the
    /// DSP check reads ([`ACCESS_POLICY_KEY`]).
    ///
    /// # Errors
    ///
    /// The entry breaks a rule; the failure says which and why.
    fn check(&self, entry: &serde_json::Value) -> Result<CheckedDataset, EntryFailure>;
}

/// The DSP 2025-1 catalog rules and the supported ODRL constraints, the
/// same as before 0.7 for the whole datasets file, now per entry
/// ([`prepare_dataset`]). Fills each distribution's `accessService`.
#[derive(Debug, Clone)]
pub struct DspCheck {
    service_id: String,
    types: CredentialTypes,
}

impl DspCheck {
    /// Checks with this connector's data service id (its DSP endpoint URL)
    /// and the credential types that policies may use.
    #[must_use]
    pub fn new(service_id: &str, types: &CredentialTypes) -> Self {
        Self {
            service_id: service_id.to_owned(),
            types: types.clone(),
        }
    }
}

impl DatasetCheck for DspCheck {
    /// `dsp(<service id>; <credential types>; portitor <version>)`: each
    /// changes what the check accepts or what it writes into a dataset.
    /// The version is [`CHECKS_VERSION`].
    fn rules_version(&self) -> String {
        let mut types = Vec::new();
        for (credential_type, _scope) in self.types.iter() {
            types.push(credential_type);
        }
        format!(
            "dsp({}; {}; portitor {CHECKS_VERSION})",
            self.service_id,
            types.join(",")
        )
    }

    fn check(&self, entry: &serde_json::Value) -> Result<CheckedDataset, EntryFailure> {
        if !entry.is_object() {
            return Err(EntryFailure::new(
                CheckKind::Json,
                "the entry is not a JSON object",
            ));
        }
        let access_policy = read_access_policy(entry, &self.types)?;
        let entry = without_access_policy(entry);
        read_offers(&entry)?;
        let dataset: Dataset = serde_json::from_value(entry)
            .map_err(|e| EntryFailure::new(CheckKind::Dsp, format!("not a DSP dataset: {e}")))?;
        match prepare_dataset(dataset, &self.service_id, &self.types) {
            Ok(dataset) => Ok(CheckedDataset {
                dataset,
                conforms_to: None,
                access_policy,
            }),
            Err(e) => Err(definition_failure(&e)),
        }
    }
}

/// Reads the entry's access policy, if it has one, strictly (as an
/// offer), and checks that it is understood with the credential `types`
/// (ADR 0019, amendment in 1.4). One that is not is an `odrl` failure.
fn read_access_policy(
    entry: &serde_json::Value,
    types: &CredentialTypes,
) -> Result<Option<AccessPolicy>, EntryFailure> {
    let Some(value) = entry.get(ACCESS_POLICY_KEY) else {
        return Ok(None);
    };
    let not_understood = |why: String| {
        let id = entry
            .get("@id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        EntryFailure::new(
            CheckKind::Odrl,
            format!("dataset {id:?}: {ACCESS_POLICY_KEY} is not understood: {why}"),
        )
    };
    // `&Value` can be read as a type without copying it first
    let policy = AccessPolicy::deserialize(value).map_err(|e| not_understood(e.to_string()))?;
    validate_access_policy(&policy, types).map_err(|e| not_understood(e.to_string()))?;
    Ok(Some(policy))
}

/// The entry without its access policy: what the catalog shows, and what
/// the other checks see.
fn without_access_policy(entry: &serde_json::Value) -> serde_json::Value {
    let mut entry = entry.clone();
    if let Some(fields) = entry.as_object_mut() {
        fields.remove(ACCESS_POLICY_KEY);
    }
    entry
}

/// Reads each offer of the entry (`hasPolicy`) on its own, before the
/// whole dataset is read. Offers are read strictly
/// ([`portitor_dsp::Offer`]): a policy field Portitor does not understand
/// (a `duty` inside a permission, a misspelled key) is refused. Reading
/// them here first makes that an `odrl` failure that names the offer and
/// the field, not a general "not a DSP dataset".
///
/// An entry without a `hasPolicy` list passes here; reading the dataset
/// reports it.
fn read_offers(entry: &serde_json::Value) -> Result<(), EntryFailure> {
    let Some(offers) = entry.get("hasPolicy").and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for offer in offers {
        // `&Value` can be read as a type without copying it first
        if let Err(error) = Offer::deserialize(offer) {
            let id = offer
                .get("@id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            return Err(EntryFailure::new(
                CheckKind::Odrl,
                format!("offer {id:?} is not understood: {error}"),
            ));
        }
    }
    Ok(())
}

/// The failure for a broken dataset definition: an offer that is not
/// understood is an ODRL failure, the rest DSP failures.
fn definition_failure(error: &DatasetDefinitionError) -> EntryFailure {
    let kind = match error {
        DatasetDefinitionError::UnsupportedPolicy { .. } => CheckKind::Odrl,
        _ => CheckKind::Dsp,
    };
    EntryFailure::new(kind, error.to_string())
}

/// Two checks, one after the other: an entry passes if it passes both.
/// The dataset served and its access policy are the first check's (the
/// DSP check); `conforms_to` is the second's, or else the first's. The
/// second check sees the entry without its access policy, as the
/// catalog shows it. The rules version names both, joined by `+`.
#[derive(Clone)]
pub struct BothChecks {
    first: Arc<dyn DatasetCheck>,
    second: Arc<dyn DatasetCheck>,
}

impl std::fmt::Debug for BothChecks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BothChecks")
    }
}

impl BothChecks {
    /// `first`, then `second` on the same entry.
    #[must_use]
    pub fn new(first: Arc<dyn DatasetCheck>, second: Arc<dyn DatasetCheck>) -> Self {
        Self { first, second }
    }
}

impl DatasetCheck for BothChecks {
    fn rules_version(&self) -> String {
        format!(
            "{}+{}",
            self.first.rules_version(),
            self.second.rules_version()
        )
    }

    fn check(&self, entry: &serde_json::Value) -> Result<CheckedDataset, EntryFailure> {
        let first = self.first.check(entry)?;
        let second = self.second.check(&without_access_policy(entry))?;
        Ok(CheckedDataset {
            dataset: first.dataset,
            conforms_to: second.conforms_to.or(first.conforms_to),
            access_policy: first.access_policy,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn entry() -> serde_json::Value {
        json!({
            "@id": "d", "@type": "Dataset",
            "hasPolicy": [{"@id": "d:offer", "@type": "Offer", "permission": [{"action": "use"}]}],
            "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}]
        })
    }

    fn dsp() -> DspCheck {
        DspCheck::new("https://p.example/dsp/2025-1", &CredentialTypes::default())
    }

    #[test]
    fn every_check_name_reads_back() {
        for kind in CheckKind::ALL {
            assert_eq!(CheckKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(CheckKind::from_name("SHACL"), None);
    }

    #[test]
    fn the_dsp_check_fills_the_service_and_names_its_kinds() {
        let passed = dsp().check(&entry()).unwrap();
        assert_eq!(
            passed.dataset.distribution[0].access_service,
            "https://p.example/dsp/2025-1"
        );
        assert_eq!(passed.conforms_to, None);

        let failed = dsp().check(&json!([1])).unwrap_err();
        assert_eq!(failed.kind, CheckKind::Json);
        let failed = dsp()
            .check(&json!({"@id": "x", "@type": "Catalog"}))
            .unwrap_err();
        assert_eq!(failed.kind, CheckKind::Dsp);
        let mut no_offer = entry();
        no_offer["hasPolicy"] = json!([]);
        let failed = dsp().check(&no_offer).unwrap_err();
        assert_eq!(
            (failed.kind, failed.message.as_str()),
            (CheckKind::Dsp, "dataset \"d\" has no offer (hasPolicy)")
        );
        let mut spatial = entry();
        spatial["hasPolicy"][0]["permission"][0]["constraint"] =
            json!([{"leftOperand": "spatial", "operator": "eq", "rightOperand": "LU"}]);
        let failed = dsp().check(&spatial).unwrap_err();
        assert_eq!(failed.kind, CheckKind::Odrl);
        assert!(failed.message.contains("spatial"), "{}", failed.message);

        // a purpose (ADR 0009, amendment in 1.4): `eq` with a text, or
        // `isAnyOf` with texts; any other form is not understood
        let research = "https://w3id.org/dpv/legal/eu/ehds#HealthcareScientificResearch";
        let mut purpose = entry();
        purpose["hasPolicy"][0]["permission"][0]["constraint"] =
            json!([{"leftOperand": "purpose", "operator": "eq", "rightOperand": research}]);
        assert!(dsp().check(&purpose).is_ok());
        for (operator, right) in [
            ("neq", json!(research)),
            ("eq", json!([research])),
            ("isAnyOf", json!(research)),
        ] {
            purpose["hasPolicy"][0]["permission"][0]["constraint"] =
                json!([{"leftOperand": "purpose", "operator": operator, "rightOperand": right}]);
            let failed = dsp().check(&purpose).unwrap_err();
            assert_eq!(failed.kind, CheckKind::Odrl, "{operator}");
            assert!(failed.message.contains("purpose"), "{}", failed.message);
        }
    }

    #[test]
    fn the_dsp_check_reads_the_access_policy_and_keeps_it_out_of_the_dataset() {
        let mut with_policy = entry();
        with_policy[ACCESS_POLICY_KEY] = json!({"permission": [{"action": "use", "constraint": [
            {"leftOperand": "MembershipCredential.role", "operator": "eq", "rightOperand": "lab"}]}]});
        let passed = dsp().check(&with_policy).unwrap();
        assert_eq!(
            passed.access_policy.unwrap().permission[0].constraint.len(),
            1
        );
        let shown = serde_json::to_string(&passed.dataset).unwrap();
        assert!(!shown.contains("accessPolicy"), "{shown}");
        assert_eq!(dsp().check(&entry()).unwrap().access_policy, None);

        // a purpose, a prohibition, a duty, a type that is not configured
        for bad in [
            json!({"permission": [{"action": "use", "constraint": [{"leftOperand": "purpose",
                "operator": "eq", "rightOperand": "https://example.com/p"}]}]}),
            json!({"permission": [{"action": "use"}], "prohibition": [{"action": "use"}]}),
            json!({"permission": [{"action": "use", "duty": [{"action": "notify"}]}]}),
            json!({"permission": [{"action": "use", "constraint": [{
                "leftOperand": "OtherCredential.x", "operator": "eq", "rightOperand": "y"}]}]}),
            json!("members"),
        ] {
            let mut entry = entry();
            entry[ACCESS_POLICY_KEY] = bad.clone();
            let failed = dsp().check(&entry).unwrap_err();
            assert_eq!(failed.kind, CheckKind::Odrl, "{bad}");
            assert!(
                failed.message.contains(ACCESS_POLICY_KEY),
                "{}",
                failed.message
            );
        }
    }

    /// Stands for a SHACL check: refuses entries with `"bad": true`.
    struct Shapes;

    impl DatasetCheck for Shapes {
        fn rules_version(&self) -> String {
            "bundle:1".to_owned()
        }
        fn check(&self, entry: &serde_json::Value) -> Result<CheckedDataset, EntryFailure> {
            if entry.get("bad").is_some() {
                return Err(EntryFailure {
                    kind: CheckKind::Shacl,
                    message: "bad".to_owned(),
                    report: Some(json!({"conforms": false})),
                });
            }
            let dataset = serde_json::from_value(entry.clone()).unwrap();
            assert!(
                entry.get(ACCESS_POLICY_KEY).is_none(),
                "the second check sees no access policy"
            );
            Ok(CheckedDataset {
                dataset,
                conforms_to: Some("https://shapes.example/1".to_owned()),
                access_policy: None,
            })
        }
    }

    #[test]
    fn both_checks_must_pass() {
        let both = BothChecks::new(Arc::new(dsp()), Arc::new(Shapes));
        assert_eq!(
            both.rules_version(),
            format!(
                "dsp(https://p.example/dsp/2025-1; MembershipCredential; portitor {})+bundle:1",
                env!("CARGO_PKG_VERSION")
            )
        );
        let mut with_policy = entry();
        with_policy[ACCESS_POLICY_KEY] = json!({"permission": [{"action": "use"}]});
        let passed = both.check(&with_policy).unwrap();
        assert!(passed.access_policy.is_some());
        let passed = both.check(&entry()).unwrap();
        // the first check's dataset, with its access service
        assert!(!passed.dataset.distribution[0].access_service.is_empty());
        assert_eq!(
            passed.conforms_to.as_deref(),
            Some("https://shapes.example/1")
        );
        let mut bad = entry();
        bad["bad"] = json!(true);
        assert_eq!(both.check(&bad).unwrap_err().kind, CheckKind::Shacl);
        let mut no_offer = entry();
        no_offer["hasPolicy"] = json!([]);
        assert_eq!(both.check(&no_offer).unwrap_err().kind, CheckKind::Dsp);
    }
}
