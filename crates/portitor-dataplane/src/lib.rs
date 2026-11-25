// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! The built-in HTTP data plane (ADR 0013, "The data"; ADR 0027; ADR
//! 0028). It implements the Data Plane Signaling [`DataPlane`] port,
//! serves the data of pull transfers at `GET /data/{flowId}`, and sends
//! the data of push transfers to the consumer's address. The connector
//! links it in and talks to it only through that port and the ports
//! below, so it can later run in its own process (ADR 0028, step 2).
//!
//! Where to look:
//!
//! | File | What it holds |
//! |---|---|
//! | `flows.rs` | [`Flow`], the [`FlowStore`] port, access tokens ([`hash`], [`new_token`]) |
//! | `plane.rs` | [`HttpDataPlane`]: what it holds and how it is built |
//! | `signaling.rs` | the [`DataPlane`] signals: start (with the source from `metadata`, [`SOURCE_METADATA_KEY`]), suspend, resume, complete, terminate, a new token |
//! | `serving.rs` | `GET /data/{flowId}` ([`serve`]): token, membership check, the stream, the audit line |
//! | `sources.rs` | reading a source: an upstream URL or an `s3://` object ([`SourceData`]) |
//! | `push.rs` | sending a push ([`PushJob`], [`push::send`]) |
//! | `download.rs` | cutting a running download, and its membership checks |
//! | `running_flows.rs` | the switches that cut what runs for a flow ([`RunningFlows`]) |
//! | `access.rs` | the [`AccessGate`] port: the control plane's membership check |
//! | `audit.rs` | the audit line `data transfer done` |
//! | `object_storage.rs` | the S3-compatible object storage client |
//! | `test_store.rs` | a flow store in memory, for tests |
//!
//! [`DataPlane`]: portitor_dps::port::DataPlane
//! [`RunningFlows`]: running_flows::RunningFlows
//! [`AccessGate`]: access::AccessGate

pub mod access;
pub mod audit;
mod download;
mod flows;
pub mod object_storage;
mod plane;
pub mod push;
pub mod running_flows;
mod serving;
mod signaling;
mod sources;
#[cfg(any(test, feature = "test-support"))]
pub mod test_store;

pub use flows::{Flow, FlowStore, FlowStoreError, hash, new_token};
pub use plane::HttpDataPlane;
pub use push::PushJob;
pub use serving::{DATA_PATH, Denied, serve, unauthorized};
pub use signaling::{HTTP_ENDPOINT_TYPE, SOURCE_METADATA_KEY};
pub use sources::SourceData;
