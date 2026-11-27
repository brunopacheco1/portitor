// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! What the DSP code ([`CatalogDatasets`]) and the transfers' signals to
//! the data plane ([`UpstreamSources`]) read: the stored datasets, read
//! from the store on each call (ADR 0029, D4). Eclipse EDC reads its
//! catalog from its asset index on each request in the same way.
//!
//! Nothing is kept in memory: a harvest committed by any process that
//! shares the database is seen by the next request, and a request never
//! sees half a run, because a run is one transaction.

use std::sync::Arc;

use portitor_dsp::Dataset;
use portitor_dsp::catalog::{CatalogDatasets, ListedDataset};
use portitor_dsp::policy::AccessPolicy;

use super::{DatasetState, DatasetStore, StoreError};

/// Port: each dataset's source, as the harvest read it from the datasets
/// file (`portitor:source`). The control plane offers transfers of the
/// datasets that have one, and names it in the data flow's start message
/// (`crate::data_flows`); the data plane keeps it from there (ADR 0028,
/// step 1).
pub trait UpstreamSources: Send + Sync {
    /// The source of dataset `dataset_id`'s data, if it has one.
    ///
    /// # Errors
    ///
    /// The datasets cannot be read now; the text says why.
    fn upstream_of(&self, dataset_id: &str) -> Result<Option<String>, String>;
}

/// A fixed map of dataset id to source (tests only, so not in the binary).
#[cfg(test)]
impl UpstreamSources for std::collections::BTreeMap<String, String> {
    fn upstream_of(&self, dataset_id: &str) -> Result<Option<String>, String> {
        Ok(self.get(dataset_id).cloned())
    }
}

/// Adapter: the catalog, read from the harvest's store on each call.
/// Cheap to clone; all clones read the same store.
#[derive(Clone)]
pub struct StoredCatalog(Arc<dyn DatasetStore>);

impl std::fmt::Debug for StoredCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoredCatalog")
    }
}

impl StoredCatalog {
    /// The catalog of the datasets in `store`.
    #[must_use]
    pub fn new(store: Arc<dyn DatasetStore>) -> Self {
        Self(store)
    }

    /// A catalog with no datasets (tests).
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::new(Arc::new(super::InMemoryDatasetStore::default()))
    }

    /// How many datasets are in the catalog now.
    ///
    /// # Errors
    ///
    /// The store cannot be read.
    pub fn active_count(&self) -> Result<usize, StoreError> {
        Ok(self.0.catalog()?.len())
    }
}

/// Short forms for the tests, without the access policies.
#[cfg(test)]
impl StoredCatalog {
    /// Active datasets only, in id order.
    pub(crate) fn all(&self) -> Result<Vec<Dataset>, StoreError> {
        let mut datasets = Vec::new();
        for listed in self.0.catalog()? {
            datasets.push(listed.dataset);
        }
        Ok(datasets)
    }

    /// Active datasets only.
    pub(crate) fn get(&self, id: &str) -> Result<Option<Dataset>, StoreError> {
        Ok(self
            .get_with_access_policy(id)?
            .map(|listed| listed.dataset))
    }
}

impl CatalogDatasets for StoredCatalog {
    /// Active, withdrawn or hidden: an agreement already made keeps its
    /// transfers (ADR 0020).
    fn get_for_agreement(&self, id: &str) -> Result<Option<Dataset>, StoreError> {
        Ok(self.0.dataset(id)?.map(|stored| stored.content))
    }

    /// Active datasets only, in id order.
    fn all_with_access_policies(&self) -> Result<Vec<ListedDataset>, StoreError> {
        self.0.catalog()
    }

    /// Active datasets only.
    fn get_with_access_policy(&self, id: &str) -> Result<Option<ListedDataset>, StoreError> {
        match self.0.dataset(id)? {
            Some(stored) if stored.state == DatasetState::Active => Ok(Some(ListedDataset {
                dataset: stored.content,
                access_policy: stored.access_policy,
            })),
            _ => Ok(None),
        }
    }

    /// Active datasets only; read from their own column.
    fn access_policies(&self) -> Result<Vec<AccessPolicy>, StoreError> {
        self.0.access_policies()
    }
}

impl UpstreamSources for StoredCatalog {
    /// Any state, like [`StoredCatalog::get_for_agreement`].
    fn upstream_of(&self, dataset_id: &str) -> Result<Option<String>, String> {
        match self.0.dataset(dataset_id) {
            Ok(found) => Ok(found.and_then(|stored| stored.upstream)),
            Err(e) => Err(format!("cannot read dataset {dataset_id}: {}", e.0)),
        }
    }
}
