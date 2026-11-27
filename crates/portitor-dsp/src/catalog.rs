// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Catalog protocol (domain). Pure logic; datasets come through the
//! [`CatalogDatasets`] port.

use crate::policy::{
    AccessPolicy, CallerCredentials, CredentialTypes, access_policy_holds, credential_scopes,
};
pub use crate::process::StoreError;
use crate::{
    Catalog, CatalogError, CatalogRequestMessage, CatalogType, Context, DataService,
    DataServiceType, Dataset,
};

/// Port: where the provider's datasets come from. Each call reads them
/// where they are kept (the connector's database, ADR 0029, D4), so every
/// call can fail.
pub trait CatalogDatasets {
    /// All datasets visible to the requester.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn all(&self) -> Result<Vec<Dataset>, StoreError>;
    /// One dataset by id, if it is in the catalog.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn get(&self, id: &str) -> Result<Option<Dataset>, StoreError>;
    /// One dataset by id, for a transfer under an agreement already made.
    /// It may be a dataset that has left the catalog since (the provider
    /// withdrew or hid it): the agreement is a contract, so its transfers
    /// keep working. Use it only for transfers; catalog, dataset and
    /// contract requests use [`CatalogDatasets::get`]. The default is `get`,
    /// for stores whose datasets never leave the catalog.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn get_for_agreement(&self, id: &str) -> Result<Option<Dataset>, StoreError> {
        self.get(id)
    }
    /// Every dataset in the catalog with its access policy, if it has one
    /// (ADR 0019, amendment in 1.4). The default: [`CatalogDatasets::all`],
    /// without access policies, for stores that have none.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn all_with_access_policies(&self) -> Result<Vec<ListedDataset>, StoreError> {
        let mut listed = Vec::new();
        for dataset in self.all()? {
            listed.push(ListedDataset {
                dataset,
                access_policy: None,
            });
        }
        Ok(listed)
    }
    /// One dataset in the catalog with its access policy. The default:
    /// [`CatalogDatasets::get`], without an access policy.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn get_with_access_policy(&self, id: &str) -> Result<Option<ListedDataset>, StoreError> {
        Ok(self.get(id)?.map(|dataset| ListedDataset {
            dataset,
            access_policy: None,
        }))
    }
    /// The access policies of the datasets in the catalog, each once: what
    /// a catalog request needs to know which credentials to ask for. The
    /// default reads them from [`CatalogDatasets::all_with_access_policies`];
    /// a store may read only them.
    ///
    /// # Errors
    /// The datasets cannot be read.
    fn access_policies(&self) -> Result<Vec<AccessPolicy>, StoreError> {
        let mut policies: Vec<AccessPolicy> = Vec::new();
        for listed in self.all_with_access_policies()? {
            if let Some(policy) = listed.access_policy
                && !policies.contains(&policy)
            {
                policies.push(policy);
            }
        }
        Ok(policies)
    }
}

/// A dataset of the catalog with its access policy (ADR 0019, amendment
/// in 1.4). The access policy decides who sees the dataset; it is never
/// shown in the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedDataset {
    /// The dataset as the catalog shows it.
    pub dataset: Dataset,
    /// Who may see it; `None`: every member.
    pub access_policy: Option<AccessPolicy>,
}

/// Who asks for the catalog or a dataset: the time of the request (an
/// `xsd:dateTime` with a time zone) and the caller's verified
/// credentials, for the access policies.
#[derive(Debug, Clone, Copy)]
pub struct Requester<'a> {
    /// The time now.
    pub now: &'a str,
    /// The caller's verified credentials.
    pub credentials: &'a CallerCredentials,
}

impl Requester<'_> {
    /// Whether the requester may see a dataset with this access policy.
    /// Without one, every member may (as before 1.4). A contract request
    /// asks this too (the provider's negotiation).
    pub(crate) fn may_see(&self, access_policy: Option<&AccessPolicy>) -> bool {
        match access_policy {
            None => true,
            Some(policy) => access_policy_holds(policy, self.now, self.credentials).is_ok(),
        }
    }
}

/// Outcome of a catalog operation, independent of transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<T> {
    /// Success.
    Ok(T),
    /// The request was invalid (HTTP 400 in the HTTPS binding).
    BadRequest(CatalogError),
    /// The resource does not exist (HTTP 404 in the HTTPS binding).
    NotFound(CatalogError),
    /// The datasets could not be read (HTTP 500 in the HTTPS binding).
    StoreFailed(StoreError),
}

/// The DSP 2025-1 namespace: in expanded JSON-LD, every DSP term is this
/// IRI plus the term (for example `CatalogRequestMessage`).
pub const DSPACE_NAMESPACE: &str = "https://w3id.org/dspace/2025/1/";

/// Reads a `CatalogRequestMessage` in compacted form (what DSP 2025-1
/// sends), or in the one expanded form the DCP TCK v1.2.1 sends to start
/// its verifier tests: `@type` is the full IRI and the filter, if present,
/// is empty (`docs/TESTING.md`, "The verifier tests"; the TCK's own trigger
/// message). Both mean the same JSON-LD message. Any other expanded
/// content is refused, as Portitor does not process JSON-LD.
///
/// # Errors
///
/// Not a catalog request in either form; the text says why.
pub fn read_catalog_request(value: serde_json::Value) -> Result<CatalogRequestMessage, String> {
    let expanded_type = format!("{DSPACE_NAMESPACE}CatalogRequestMessage");
    let expanded_filter = format!("{DSPACE_NAMESPACE}filter");
    let is_expanded = value.get("@context").is_none()
        && value.get("@type").and_then(serde_json::Value::as_str) == Some(&expanded_type);
    if !is_expanded {
        return serde_json::from_value(value).map_err(|e| e.to_string());
    }
    let Some(fields) = value.as_object() else {
        return Err("not a JSON object".to_owned());
    };
    for (key, field) in fields {
        let empty_filter = *key == expanded_filter && is_empty_json(field);
        if key != "@type" && !empty_filter {
            return Err(format!("expanded field {key} is not supported"));
        }
    }
    Ok(CatalogRequestMessage {
        context: Context::default(),
        kind: crate::CatalogRequestMessageType,
        filter: None,
    })
}

/// Whether a `filter` entry asks for no filtering at all.
///
/// EDC (checked in v1.0.0-RC7, `JsonObjectFromCatalogRequestMessageTransformer`)
/// always sends its own `QuerySpec` as the filter, even when it wants the
/// whole catalog: no filter expressions, only paging (`offset`, `limit`)
/// and sorting. Such a filter, or an empty `{}`, selects nothing, so
/// Portitor answers with the whole catalog. Paging and sorting are
/// ignored. Anything else, for example a `QuerySpec` with filter
/// expressions or an unknown field, is not supported.
fn filters_nothing(entry: &serde_json::Value) -> bool {
    /// The `QuerySpec` fields that do not select datasets.
    const PAGING: [&str; 4] = ["offset", "limit", "sortOrder", "sortField"];
    let Some(fields) = entry.as_object() else {
        return false;
    };
    fields.iter().all(|(key, value)| match local_name(key) {
        "@context" => true,
        "@type" => value.as_str().map(local_name) == Some("QuerySpec"),
        "filterExpression" => is_empty_json(value),
        name => PAGING.contains(&name),
    })
}

/// The term at the end of an IRI or a prefixed name
/// (`https://w3id.org/edc/v0.0.1/ns/limit` and `edc:limit` both give `limit`).
fn local_name(key: &str) -> &str {
    key.rsplit(['/', '#', ':']).next().unwrap_or(key)
}

/// `{}` or `[]`.
fn is_empty_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.is_empty(),
        serde_json::Value::Array(list) => list.is_empty(),
        _ => false,
    }
}

/// Why a set of dataset definitions cannot be offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetDefinitionError {
    /// A dataset has an empty `@id`.
    EmptyId,
    /// Two datasets share an `@id`.
    DuplicateId(String),
    /// A dataset has no offer; DSP 2025-1 requires at least one `hasPolicy`.
    NoOffer(String),
    /// A dataset has no distribution; DSP 2025-1 requires at least one.
    NoDistribution(String),
    /// An offer in a catalog has a `target`; DSP 2025-1 forbids it there
    /// (the dataset is the target).
    OfferWithTarget(String),
    /// An offer in a catalog has an `assigner` or an `assignee`. The DSP
    /// 2025-1 JSON Schema defines neither for an offer in a catalog
    /// (`contract-schema.json`, `Offer`), and the consumer is not known
    /// yet; the provider is the catalog's `participantId`.
    OfferWithParty(String),
    /// A distribution has an empty `format`.
    EmptyFormat(String),
    /// An offer has a constraint this version does not understand, so the
    /// provider could never agree to it (ADR 0009).
    UnsupportedPolicy {
        /// The dataset.
        dataset: String,
        /// What is not understood.
        reason: String,
    },
    /// A distribution names a data service this connector does not run.
    UnknownAccessService {
        /// The dataset.
        dataset: String,
        /// The `accessService` it names.
        service: String,
    },
}

impl std::fmt::Display for DatasetDefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyId => write!(f, "a dataset has an empty @id"),
            Self::DuplicateId(id) => write!(f, "dataset {id:?} is defined twice"),
            Self::NoOffer(id) => write!(f, "dataset {id:?} has no offer (hasPolicy)"),
            Self::NoDistribution(id) => write!(f, "dataset {id:?} has no distribution"),
            Self::OfferWithTarget(id) => {
                write!(f, "dataset {id:?} has an offer with a target; leave it out")
            }
            Self::OfferWithParty(id) => write!(
                f,
                "dataset {id:?} has an offer with an assigner or an assignee; leave them out"
            ),
            Self::EmptyFormat(id) => write!(f, "dataset {id:?} has a distribution without format"),
            Self::UnsupportedPolicy { dataset, reason } => {
                write!(f, "dataset {dataset:?} has an offer with an {reason}")
            }
            Self::UnknownAccessService { dataset, service } => write!(
                f,
                "dataset {dataset:?} names accessService {service:?}; leave it out, the connector sets it"
            ),
        }
    }
}

impl std::error::Error for DatasetDefinitionError {}

/// Checks a provider's dataset definitions, each with
/// [`prepare_dataset`], and that no two share an `@id`.
///
/// # Errors
///
/// The first definition that breaks a rule.
pub fn prepare_datasets(
    datasets: Vec<Dataset>,
    service_id: &str,
    types: &crate::policy::CredentialTypes,
) -> Result<Vec<Dataset>, DatasetDefinitionError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut prepared = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        let ready = prepare_dataset(dataset, service_id, types)?;
        if !seen.insert(ready.id.clone()) {
            return Err(DatasetDefinitionError::DuplicateId(ready.id));
        }
        prepared.push(ready);
    }
    Ok(prepared)
}

/// Checks one dataset definition against the DSP 2025-1 rules for a
/// dataset in a catalog (a non-empty `@id`, at least one offer and one
/// distribution, each distribution with a format, no offer with a
/// target, assigner or assignee), checks that every offer's constraints are understood (ADR
/// 0009; credential constraints only on the credential types in `types`,
/// ADR 0019), and makes every distribution point to this connector's data
/// service, `service_id`.
///
/// # Errors
///
/// The first rule the definition breaks.
pub fn prepare_dataset(
    mut dataset: Dataset,
    service_id: &str,
    types: &crate::policy::CredentialTypes,
) -> Result<Dataset, DatasetDefinitionError> {
    let id = dataset.id.clone();
    if id.is_empty() {
        return Err(DatasetDefinitionError::EmptyId);
    }
    if dataset.has_policy.is_empty() {
        return Err(DatasetDefinitionError::NoOffer(id));
    }
    if dataset.distribution.is_empty() {
        return Err(DatasetDefinitionError::NoDistribution(id));
    }
    if dataset.has_policy.iter().any(|o| o.target.is_some()) {
        return Err(DatasetDefinitionError::OfferWithTarget(id));
    }
    for offer in &dataset.has_policy {
        if offer.assigner.is_some() || offer.assignee.is_some() {
            return Err(DatasetDefinitionError::OfferWithParty(id));
        }
        if let Err(e) = crate::policy::validate(offer, types) {
            return Err(DatasetDefinitionError::UnsupportedPolicy {
                dataset: id,
                reason: e.to_string(),
            });
        }
    }
    for dist in &mut dataset.distribution {
        if dist.format.is_empty() {
            return Err(DatasetDefinitionError::EmptyFormat(id));
        }
        if dist.access_service.is_empty() {
            service_id.clone_into(&mut dist.access_service);
        } else if dist.access_service != service_id {
            return Err(DatasetDefinitionError::UnknownAccessService {
                dataset: id,
                service: dist.access_service.clone(),
            });
        }
    }
    dataset.context = None;
    Ok(dataset)
}

/// The catalog service of one provider.
#[derive(Debug, Clone)]
pub struct CatalogService<S> {
    store: S,
    catalog_id: String,
    participant_id: String,
    service: DataService,
}

impl<S: CatalogDatasets> CatalogService<S> {
    /// Creates the service.
    pub fn new(
        store: S,
        catalog_id: String,
        participant_id: String,
        service_id: String,
        endpoint_url: String,
    ) -> Self {
        Self {
            store,
            catalog_id,
            participant_id,
            service: DataService {
                id: service_id,
                kind: DataServiceType,
                endpoint_url,
            },
        }
    }

    /// The DCP scopes to ask the caller's credential service for, to
    /// evaluate the access policies of the catalog (ADR 0019, amendment in
    /// 1.4): those of the credential types the policies name, each once.
    ///
    /// # Errors
    ///
    /// The datasets cannot be read.
    pub fn access_scopes(&self, types: &CredentialTypes) -> Result<Vec<String>, StoreError> {
        let mut scopes: Vec<String> = Vec::new();
        for policy in self.store.access_policies()? {
            for scope in credential_scopes(&policy, types) {
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
        Ok(scopes)
    }

    /// As [`CatalogService::access_scopes`], for the access policy of
    /// dataset `id` only (a dataset request).
    ///
    /// # Errors
    ///
    /// The datasets cannot be read.
    pub fn dataset_access_scopes(
        &self,
        id: &str,
        types: &CredentialTypes,
    ) -> Result<Vec<String>, StoreError> {
        let policy = self
            .store
            .get_with_access_policy(id)?
            .and_then(|listed| listed.access_policy);
        Ok(match policy {
            Some(policy) => credential_scopes(&policy, types),
            None => Vec::new(),
        })
    }

    /// Answers a `CatalogRequestMessage` from `requester`. Filters are not
    /// supported, so a filter that would select something is a bad
    /// request, as DSP 2025-1 requires. A filter that selects nothing is
    /// ignored (see [`filters_nothing`]). A dataset whose access policy
    /// does not hold for the requester is left out.
    pub fn request(
        &self,
        msg: &CatalogRequestMessage,
        requester: &Requester<'_>,
    ) -> Outcome<Catalog> {
        let filter = msg.filter.as_deref().unwrap_or_default();
        if !filter.iter().all(filters_nothing) {
            return Outcome::BadRequest(CatalogError::new(
                "400",
                "filter expressions are not supported",
            ));
        }
        let listed = match self.store.all_with_access_policies() {
            Ok(listed) => listed,
            Err(e) => return Outcome::StoreFailed(e),
        };
        let mut dataset = Vec::new();
        for entry in listed {
            if requester.may_see(entry.access_policy.as_ref()) {
                dataset.push(entry.dataset);
            }
        }
        Outcome::Ok(Catalog {
            context: Context::default(),
            id: self.catalog_id.clone(),
            kind: CatalogType,
            participant_id: self.participant_id.clone(),
            service: vec![self.service.clone()],
            dataset,
        })
    }

    /// Answers a dataset request from `requester`. A dataset whose access
    /// policy does not hold for the requester gets the same answer as an
    /// unknown dataset.
    pub fn dataset(&self, id: &str, requester: &Requester<'_>) -> Outcome<Dataset> {
        match self.store.get_with_access_policy(id) {
            Ok(Some(listed)) if requester.may_see(listed.access_policy.as_ref()) => {
                let mut d = listed.dataset;
                d.context = Some(Context::default());
                Outcome::Ok(d)
            }
            Ok(_) => Outcome::NotFound(CatalogError::new("404", "dataset not found")),
            Err(e) => Outcome::StoreFailed(e),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::policy::CredentialTypes;
    use crate::{DatasetType, Distribution, DistributionType, Offer, OfferType, Rule};

    fn dataset(id: &str) -> Dataset {
        Dataset {
            context: None,
            id: id.to_owned(),
            kind: DatasetType,
            has_policy: vec![Offer {
                id: format!("{id}:offer"),
                kind: OfferType,
                permission: vec![Rule {
                    action: "use".to_owned(),
                    constraint: vec![],
                }],
                prohibition: vec![],
                obligation: vec![],
                target: None,
                assigner: None,
                assignee: None,
                profile: None,
            }],
            distribution: vec![Distribution {
                kind: DistributionType,
                format: "HttpData-PULL".to_owned(),
                access_service: String::new(),
                metadata: crate::Metadata::default(),
            }],
            metadata: crate::Metadata::default(),
        }
    }

    #[test]
    fn fills_the_access_service() {
        let out = prepare_datasets(vec![dataset("a")], "svc", &CredentialTypes::default()).unwrap();
        assert_eq!(out[0].distribution[0].access_service, "svc");
    }

    #[test]
    fn rejects_broken_definitions() {
        let mut no_offer = dataset("a");
        no_offer.has_policy.clear();
        let mut no_dist = dataset("a");
        no_dist.distribution.clear();
        let mut no_format = dataset("a");
        no_format.distribution[0].format.clear();
        let mut other_service = dataset("a");
        other_service.distribution[0].access_service = "elsewhere".to_owned();
        let mut targeted = dataset("a");
        targeted.has_policy[0].target = Some("a".to_owned());
        // a party on an offer in a catalog (0.12): the consumer is not
        // known yet, and the provider is the catalog's `participantId`
        let mut names_an_assigner = dataset("a");
        names_an_assigner.has_policy[0].assigner = Some("did:web:provider".to_owned());
        let mut names_the_receiver = dataset("a");
        names_the_receiver.has_policy[0].assignee = Some("did:web:consumer".to_owned());
        let mut spatial = dataset("a");
        spatial.has_policy[0].permission[0].constraint = vec![
            serde_json::from_value(serde_json::json!(
                {"leftOperand": "spatial", "operator": "eq", "rightOperand": "LU"}
            ))
            .unwrap(),
        ];
        let cases = [
            (vec![dataset("")], DatasetDefinitionError::EmptyId),
            (
                vec![dataset("a"), dataset("a")],
                DatasetDefinitionError::DuplicateId("a".to_owned()),
            ),
            (
                vec![no_offer],
                DatasetDefinitionError::NoOffer("a".to_owned()),
            ),
            (
                vec![no_dist],
                DatasetDefinitionError::NoDistribution("a".to_owned()),
            ),
            (
                vec![no_format],
                DatasetDefinitionError::EmptyFormat("a".to_owned()),
            ),
            (
                vec![targeted],
                DatasetDefinitionError::OfferWithTarget("a".to_owned()),
            ),
            (
                vec![names_an_assigner],
                DatasetDefinitionError::OfferWithParty("a".to_owned()),
            ),
            (
                vec![names_the_receiver],
                DatasetDefinitionError::OfferWithParty("a".to_owned()),
            ),
            (
                vec![spatial],
                DatasetDefinitionError::UnsupportedPolicy {
                    dataset: "a".to_owned(),
                    reason: "unsupported policy: spatial eq \"LU\"".to_owned(),
                },
            ),
            (
                vec![other_service],
                DatasetDefinitionError::UnknownAccessService {
                    dataset: "a".to_owned(),
                    service: "elsewhere".to_owned(),
                },
            ),
        ];
        for (input, want) in cases {
            let got = prepare_datasets(input, "svc", &CredentialTypes::default()).unwrap_err();
            assert!(got.to_string().contains("dataset"), "{got}");
            assert_eq!(got, want);
        }
    }

    #[test]
    fn keeps_a_matching_access_service() {
        let mut d = dataset("a");
        d.distribution[0].access_service = "svc".to_owned();
        assert!(prepare_datasets(vec![d], "svc", &CredentialTypes::default()).is_ok());
    }

    #[test]
    fn a_filter_that_selects_nothing_is_ignored() {
        // what EDC v1.0.0-RC7 sends when it asks for the whole catalog
        let edc = serde_json::json!({
            "@type": "https://w3id.org/edc/v0.0.1/ns/QuerySpec",
            "https://w3id.org/edc/v0.0.1/ns/offset": 0,
            "https://w3id.org/edc/v0.0.1/ns/limit": 50,
            "https://w3id.org/edc/v0.0.1/ns/sortOrder": "ASC",
            "https://w3id.org/edc/v0.0.1/ns/filterExpression": []
        });
        let compacted = serde_json::json!({"@type": "QuerySpec", "offset": 0, "limit": 50});
        for nothing in [edc, compacted, serde_json::json!({})] {
            assert!(filters_nothing(&nothing), "{nothing}");
        }
        for something in [
            serde_json::json!({"@type": "QuerySpec", "filterExpression": [
                {"operandLeft": "id", "operator": "=", "operandRight": "a"}]}),
            serde_json::json!({"@type": "Other"}),
            serde_json::json!({"keyword": "x"}),
            serde_json::json!("id = a"),
        ] {
            assert!(!filters_nothing(&something), "{something}");
        }
    }

    #[test]
    fn reads_compacted_and_the_tck_expanded_catalog_request() {
        let compact = serde_json::json!({"@context": [crate::CONTEXT],
            "@type": "CatalogRequestMessage"});
        assert!(read_catalog_request(compact).is_ok());
        // the DCP TCK v1.2.1 trigger message
        let expanded = serde_json::json!({
            "@type": "https://w3id.org/dspace/2025/1/CatalogRequestMessage",
            "https://w3id.org/dspace/2025/1/filter": {}
        });
        let msg = read_catalog_request(expanded).unwrap();
        assert_eq!(msg.filter, None);
        for refused in [
            serde_json::json!({"@type": "https://w3id.org/dspace/2025/1/CatalogRequestMessage",
                "https://w3id.org/dspace/2025/1/filter": {"x": 1}}),
            serde_json::json!({"@type": "https://w3id.org/dspace/2025/1/CatalogRequestMessage",
                "other": []}),
            serde_json::json!({"@type": "https://w3id.org/dspace/2025/1/Other"}),
            serde_json::json!({"@type": "CatalogRequestMessage"}),
        ] {
            assert!(read_catalog_request(refused.clone()).is_err(), "{refused}");
        }
    }

    /// Datasets that cannot be read now (their database is down).
    struct Unreadable;

    impl CatalogDatasets for Unreadable {
        fn all(&self) -> Result<Vec<Dataset>, StoreError> {
            Err(StoreError("database down".to_owned()))
        }
        fn get(&self, _: &str) -> Result<Option<Dataset>, StoreError> {
            Err(StoreError("database down".to_owned()))
        }
    }

    #[test]
    fn datasets_that_cannot_be_read_are_a_store_failure_not_an_empty_catalog() {
        let service = CatalogService::new(
            Unreadable,
            "cat".to_owned(),
            "did:web:p".to_owned(),
            "svc".to_owned(),
            "https://p.example/dsp".to_owned(),
        );
        let request: CatalogRequestMessage = serde_json::from_value(serde_json::json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "CatalogRequestMessage"
        }))
        .unwrap();
        let credentials = CallerCredentials::default();
        let requester = Requester {
            now: "2026-11-27T12:00:00Z",
            credentials: &credentials,
        };
        assert!(matches!(
            service.request(&request, &requester),
            Outcome::StoreFailed(_)
        ));
        assert!(matches!(
            service.dataset("a", &requester),
            Outcome::StoreFailed(_)
        ));
    }

    /// Two datasets, `open` without an access policy and `permits` for
    /// holders of a data permit (ADR 0019, amendment in 1.4).
    struct WithAccessPolicies;

    impl WithAccessPolicies {
        fn listed() -> Vec<ListedDataset> {
            let policy: AccessPolicy = serde_json::from_value(serde_json::json!({
                "permission": [{"action": "use", "constraint": [{
                    "leftOperand": "DataPermitCredential.purpose", "operator": "eq",
                    "rightOperand": "research"}]}]}))
            .unwrap();
            vec![
                ListedDataset {
                    dataset: dataset("open"),
                    access_policy: None,
                },
                ListedDataset {
                    dataset: dataset("permits"),
                    access_policy: Some(policy),
                },
            ]
        }
    }

    impl CatalogDatasets for WithAccessPolicies {
        fn all(&self) -> Result<Vec<Dataset>, StoreError> {
            Ok(Self::listed().into_iter().map(|l| l.dataset).collect())
        }
        fn get(&self, id: &str) -> Result<Option<Dataset>, StoreError> {
            Ok(self.all()?.into_iter().find(|d| d.id == id))
        }
        fn all_with_access_policies(&self) -> Result<Vec<ListedDataset>, StoreError> {
            Ok(Self::listed())
        }
        fn get_with_access_policy(&self, id: &str) -> Result<Option<ListedDataset>, StoreError> {
            Ok(Self::listed().into_iter().find(|l| l.dataset.id == id))
        }
    }

    #[test]
    fn a_dataset_is_shown_only_to_callers_its_access_policy_holds_for() {
        use crate::policy::HeldCredential;
        let service = CatalogService::new(
            WithAccessPolicies,
            "cat".to_owned(),
            "did:web:p".to_owned(),
            "svc".to_owned(),
            "https://p.example/dsp".to_owned(),
        );
        let mut types = CredentialTypes::default();
        types.add("DataPermitCredential", "scope:permit");
        assert_eq!(service.access_scopes(&types).unwrap(), vec!["scope:permit"]);
        assert_eq!(
            service.dataset_access_scopes("permits", &types).unwrap(),
            vec!["scope:permit"]
        );
        assert!(
            service
                .dataset_access_scopes("open", &types)
                .unwrap()
                .is_empty()
        );

        let request: CatalogRequestMessage = serde_json::from_value(serde_json::json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "CatalogRequestMessage"
        }))
        .unwrap();
        let holder = |purpose: &str| {
            let serde_json::Value::Object(subject) = serde_json::json!({"purpose": purpose}) else {
                panic!("an object")
            };
            CallerCredentials {
                understood: types.clone(),
                held: vec![HeldCredential {
                    types: vec!["DataPermitCredential".to_owned()],
                    subjects: vec![subject],
                }],
            }
        };
        for (credentials, seen) in [
            (holder("research"), vec!["open", "permits"]),
            (holder("marketing"), vec!["open"]),
            (CallerCredentials::default(), vec!["open"]),
        ] {
            let requester = Requester {
                now: "2026-11-27T12:00:00Z",
                credentials: &credentials,
            };
            let Outcome::Ok(catalog) = service.request(&request, &requester) else {
                panic!("a catalog")
            };
            let ids: Vec<&str> = catalog.dataset.iter().map(|d| d.id.as_str()).collect();
            assert_eq!(ids, seen);
            // the access policy is never shown
            let text = serde_json::to_string(&catalog).unwrap();
            assert!(!text.contains("DataPermitCredential"), "{text}");
            let permits = service.dataset("permits", &requester);
            if seen.len() == 2 {
                assert!(matches!(permits, Outcome::Ok(_)));
            } else {
                // the same answer as an unknown dataset
                assert_eq!(permits, service.dataset("unknown", &requester));
            }
            assert!(matches!(
                service.dataset("open", &requester),
                Outcome::Ok(_)
            ));
        }
    }
}
