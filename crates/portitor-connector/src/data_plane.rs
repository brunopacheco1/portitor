// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! How [`crate::build`] starts the built-in data plane (crate
//! `portitor-dataplane`, linked in; ADR 0028) and serves its route.

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use portitor_dataplane::{DATA_PATH, HttpDataPlane, serve};

use crate::flow_membership::MembershipGate;
use crate::harvest::{self, UpstreamSources};
use crate::ports::{self, Ports};
use crate::{Config, data_flows};

/// Builds the built-in HTTP data plane, if the `data-plane` feature is on
/// and `ports` has a flow store, and makes transfers use it. Transfers
/// read each dataset's source from the read copy `datasets` and name it in
/// the data flow's start message (`s3://` sources are read from
/// `ports.object_storage`); the data plane checks the consumer's
/// membership with the caller verifier on every request (ADR 0027); the
/// gate it returns still needs to know who ends transfers
/// ([`MembershipGate::ends_transfers_with`]).
pub(crate) fn start(
    config: &Config,
    ports: &mut Ports,
    datasets: &harvest::DatasetCopy,
) -> Option<(Arc<HttpDataPlane>, Arc<MembershipGate>)> {
    if !cfg!(feature = "data-plane") {
        return None;
    }
    let store = ports.flow_store.clone()?;
    let sources: Arc<dyn UpstreamSources> = Arc::new(datasets.clone());
    let gate = Arc::new(MembershipGate::new(
        store.clone(),
        Arc::clone(&ports.verifier),
    ));
    // the two steps that can fail: building the HTTP clients
    let built = HttpDataPlane::new(
        store.flows_only(),
        &config.public_url,
        config.data_token_seconds,
    )
    .and_then(|plane| plane.with_push_over_http(config.allow_http));
    let plane = match built {
        Ok(plane) => plane,
        Err(e) => {
            tracing::error!(error = %e, "no data plane: transfers will be refused");
            return None;
        }
    };
    // `as _`: the gate is turned into an `Arc<dyn AccessGate>`
    let plane = plane
        .with_object_storage(ports.object_storage.clone())
        .checking_membership_with(Arc::clone(&gate) as _);
    let flows = data_flows::SignalingDataFlows::new(
        plane.clone(),
        &config.participant_id,
        "default",
        sources,
    )
    .with_memberships(Arc::new(store.clone()));
    ports.transfers.flows = ports::Shared(Arc::new(flows));
    Some((Arc::new(plane), gate))
}

/// The route of `/data/{flowId}`, when there is a data plane.
pub(crate) fn routes(plane: Option<Arc<HttpDataPlane>>) -> Router {
    let Some(plane) = plane else {
        return Router::new();
    };
    Router::new()
        .route(&format!("{DATA_PATH}/{{id}}"), get(serve))
        .with_state(plane)
}
