// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Portitor's metrics (ADR 0023, section 5), one function per measure.
//! The binaries call these functions; only this module knows
//! OpenTelemetry's types.
//!
//! Names follow OpenTelemetry's naming rules
//! (<https://opentelemetry.io/docs/specs/semconv/general/naming/>): a
//! `portitor.` namespace, then the thing and what happened to it, like
//! the stable `jvm.class.loaded`; counts have a `{thing}` unit and no
//! `_total`. The HTTP measures are the semantic conventions' own
//! (<https://opentelemetry.io/docs/specs/semconv/http/http-metrics/>).
//! Attribute values come from short fixed lists, never from free text,
//! ids or URLs, so their number stays small: the enums below ([`Role`],
//! [`Refusal`], [`Outbox`]), and `&'static str` values that the callers
//! pass from fixed names in their code (a DSP state such as `FINALIZED`,
//! a harvest kind, outcome or check name; `&'static` means a text fixed
//! in the program, so it cannot come from a request). Two exceptions,
//! each with a small, operator-controlled set of values: the bundle
//! version, chosen by the dataspace's authority, and the HTTP measures'
//! route templates, status codes and, for our own services only, server
//! addresses. The list is repeated in `docs/DEPLOYING.md`; keep both the
//! same.
//!
//! When export is off, the instruments come from OpenTelemetry's global
//! no-op meter: recording is an empty function call.

use std::sync::LazyLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// The bucket boundaries, in seconds, that the HTTP semantic conventions
/// advise for `http.server.request.duration` and
/// `http.client.request.duration`.
const DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// Every instrument, made once from the global meter.
struct Instruments {
    server_duration: Histogram<f64>,
    client_duration: Histogram<f64>,
    negotiations_ended: Counter<u64>,
    transfers_ended: Counter<u64>,
    requests_refused: Counter<u64>,
    outbox_resent: Counter<u64>,
    outbox_given_up: Counter<u64>,
    harvest_runs: Counter<u64>,
    datasets_rejected: Counter<u64>,
    bundle_in_use: Gauge<u64>,
    bundle_files_skipped: Gauge<u64>,
    http_requests_panicked: Counter<u64>,
    tasks_panicked: Counter<u64>,
}

/// The instruments, made the first time a measure is recorded.
///
/// `LazyLock` holds a value made the first time it is used and then
/// shared by every thread. An instrument stays bound to the meter provider
/// it was made from, so [`super::init`] must set the provider before the
/// first measure; it is the first thing each `main` does.
static INSTRUMENTS: LazyLock<Instruments> =
    LazyLock::new(|| Instruments::new(&opentelemetry::global::meter("portitor")));

/// The instruments ([`INSTRUMENTS`]).
fn instruments() -> &'static Instruments {
    &INSTRUMENTS
}

impl Instruments {
    fn new(meter: &Meter) -> Self {
        Self {
            server_duration: meter
                .f64_histogram("http.server.request.duration")
                .with_unit("s")
                .with_description("Duration of HTTP server requests.")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
            client_duration: meter
                .f64_histogram("http.client.request.duration")
                .with_unit("s")
                .with_description("Duration of HTTP client requests.")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
            negotiations_ended: meter
                .u64_counter("portitor.negotiation.ended")
                .with_unit("{negotiation}")
                .with_description("Contract negotiations that reached a final state.")
                .build(),
            transfers_ended: meter
                .u64_counter("portitor.transfer.ended")
                .with_unit("{transfer}")
                .with_description("Transfer processes that reached a final state.")
                .build(),
            requests_refused: meter
                .u64_counter("portitor.dsp.request.refused")
                .with_unit("{request}")
                .with_description("DSP requests refused before the protocol handled them.")
                .build(),
            outbox_resent: meter
                .u64_counter("portitor.outbox.message.resent")
                .with_unit("{message}")
                .with_description("Messages sent again by an outbox.")
                .build(),
            outbox_given_up: meter
                .u64_counter("portitor.outbox.message.given_up")
                .with_unit("{message}")
                .with_description("Messages an outbox stopped trying to deliver.")
                .build(),
            harvest_runs: meter
                .u64_counter("portitor.harvest.run.ended")
                .with_unit("{run}")
                .with_description("Harvest runs, by what they harvested and how they ended.")
                .build(),
            datasets_rejected: meter
                .u64_counter("portitor.harvest.dataset.rejected")
                .with_unit("{dataset}")
                .with_description("Harvested datasets rejected by a check.")
                .build(),
            bundle_in_use: meter
                .u64_gauge("portitor.bundle.in_use")
                .with_unit("{bundle}")
                .with_description(
                    "1 for the bundle version in force, 0 for one no longer in force.",
                )
                .build(),
            bundle_files_skipped: meter
                .u64_gauge("portitor.bundle.file.skipped")
                .with_unit("{file}")
                .with_description(
                    "Files of the bundle version in force that are skipped, because their role is not known to this version. What they hold is not enforced.",
                )
                .build(),
            http_requests_panicked: meter
                .u64_counter("portitor.http.request.panicked")
                .with_unit("{request}")
                .with_description("HTTP requests whose handler panicked; answered 500.")
                .build(),
            tasks_panicked: meter
                .u64_counter("portitor.task.panicked")
                .with_unit("{task}")
                .with_description("Background tasks that panicked.")
                .build(),
        }
    }
}

/// The side of a DSP process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// We offer the dataset.
    Provider,
    /// We asked for it.
    Consumer,
}

impl Role {
    /// The attribute value: `provider` or `consumer`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Provider => "provider",
            Self::Consumer => "consumer",
        }
    }
}

/// Why a DSP request was refused before the protocol handled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The caller is not authenticated (no valid DCP token): HTTP 404
    /// on the negotiation and transfer routes, 401 on the catalog routes.
    Unauthenticated,
    /// The body is not the message the endpoint takes (not JSON, a field
    /// missing or of the wrong type): HTTP 400.
    InvalidMessage,
    /// The dataspace's message rules (the bundle's SHACL shapes) refused
    /// it: HTTP 400.
    MessageRules,
}

impl Refusal {
    /// The `error.type` attribute value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidMessage => "invalid_message",
            Self::MessageRules => "message_rules",
        }
    }
}

/// Which outbox a message is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outbox {
    /// The connector's DSP negotiation messages (ADR 0007), on our `Role`'s side.
    Negotiation(Role),
    /// The connector's DSP transfer messages, on our `Role`'s side.
    Transfer(Role),
    /// The issuer's credential deliveries (DCP).
    CredentialDelivery,
}

impl Outbox {
    /// What the outbox delivers: `negotiation`, `transfer` or
    /// `credential_delivery`.
    #[must_use]
    pub fn process_name(self) -> &'static str {
        match self {
            Self::Negotiation(_) => "negotiation",
            Self::Transfer(_) => "transfer",
            Self::CredentialDelivery => "credential_delivery",
        }
    }

    /// Our side of the DSP process (`provider` or `consumer`); `None` for
    /// the issuer's credential deliveries, which are not DSP.
    #[must_use]
    pub fn role_name(self) -> Option<&'static str> {
        match self {
            Self::Negotiation(role) | Self::Transfer(role) => Some(role.name()),
            Self::CredentialDelivery => None,
        }
    }

    /// The attributes of the outbox measures: `portitor.outbox.name`
    /// (`dsp` or `credential_delivery`), and for DSP the process and our
    /// role.
    fn attributes(self) -> Vec<KeyValue> {
        match self.role_name() {
            Some(role) => vec![
                KeyValue::new("portitor.outbox.name", "dsp"),
                KeyValue::new("portitor.dsp.process", self.process_name()),
                KeyValue::new("portitor.dsp.role", role),
            ],
            None => vec![KeyValue::new("portitor.outbox.name", self.process_name())],
        }
    }
}

/// `http.server.request.duration`: one served request. `route` is the
/// route template (`/dsp/2025-1/negotiations/{pid}`), `None` when no
/// route matched; `status` the answer's status code; `version` the HTTP
/// version (`1.1`). The servers speak plain HTTP (TLS, if any, ends in
/// front of them), so `url.scheme` is `http`.
pub(crate) fn server_request(
    seconds: f64,
    method: &'static str,
    route: Option<&str>,
    status: u16,
    version: &'static str,
) {
    let mut attributes = vec![
        KeyValue::new("http.request.method", method),
        KeyValue::new("url.scheme", "http"),
        KeyValue::new("network.protocol.version", version),
        KeyValue::new("http.response.status_code", i64::from(status)),
    ];
    if let Some(route) = route {
        attributes.push(KeyValue::new("http.route", route.to_owned()));
    }
    if status >= 500 {
        attributes.push(KeyValue::new("error.type", status.to_string()));
    }
    instruments().server_duration.record(seconds, &attributes);
}

/// `http.client.request.duration`: one sent request, with what the
/// semantic conventions ask for (`server.address`, `server.port`, the
/// status, or `error.type` when there was no answer or a 4xx/5xx).
pub(crate) fn client_request(seconds: f64, attributes: &[KeyValue]) {
    instruments().client_duration.record(seconds, attributes);
}

/// A negotiation reached `state` (`FINALIZED` or `TERMINATED`) on our
/// `role`'s side.
pub fn negotiation_ended(role: Role, state: &'static str) {
    let attributes = [
        KeyValue::new("portitor.dsp.role", role.name()),
        KeyValue::new("portitor.dsp.state", state),
    ];
    instruments().negotiations_ended.add(1, &attributes);
}

/// A transfer reached `state` (`COMPLETED` or `TERMINATED`) on our
/// `role`'s side.
pub fn transfer_ended(role: Role, state: &'static str) {
    let attributes = [
        KeyValue::new("portitor.dsp.role", role.name()),
        KeyValue::new("portitor.dsp.state", state),
    ];
    instruments().transfers_ended.add(1, &attributes);
}

/// A DSP request was refused for `reason`.
pub fn dsp_request_refused(reason: Refusal) {
    let attributes = [KeyValue::new("error.type", reason.name())];
    instruments().requests_refused.add(1, &attributes);
}

/// `outbox` sends a message again: a retry, a message saved before a
/// restart, or a message whose sending panicked.
pub fn outbox_resent(outbox: Outbox) {
    instruments().outbox_resent.add(1, &outbox.attributes());
}

/// `outbox` stopped trying to deliver a message.
pub fn outbox_given_up(outbox: Outbox) {
    instruments().outbox_given_up.add(1, &outbox.attributes());
}

/// A harvest run of `kind` (`datasets` or `bundle`) ended with `outcome`
/// (`succeeded`, `partial`, `unchanged`, `failed`; the names of the job
/// log, ADR 0020).
pub fn harvest_run_ended(kind: &'static str, outcome: &'static str) {
    let attributes = [
        KeyValue::new("portitor.harvest.kind", kind),
        KeyValue::new("portitor.harvest.outcome", outcome),
    ];
    instruments().harvest_runs.add(1, &attributes);
}

/// `count` harvested datasets were rejected by the check `check` (the
/// check names of the job log).
pub fn datasets_rejected(check: &'static str, count: u64) {
    let attributes = [KeyValue::new("portitor.harvest.check", check)];
    instruments().datasets_rejected.add(count, &attributes);
}

/// The bundle `version` is in force (`true`), or no longer (`false`).
/// The version is chosen by the dataspace's authority and changes
/// rarely, so it is an attribute here.
pub fn bundle_in_use(version: &str, in_use: bool) {
    let attributes = [KeyValue::new("portitor.bundle.version", version.to_owned())];
    instruments()
        .bundle_in_use
        .record(u64::from(in_use), &attributes);
}

/// `count` files of the bundle version in force are skipped: their role
/// is not known to this version, so what they hold is not enforced
/// (ADR 0005, amended in 0.13). 0 when every file is used. No attribute:
/// the paths and role names are the authority's text, not a fixed set.
pub fn bundle_files_skipped(count: usize) {
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    instruments().bundle_files_skipped.record(count, &[]);
}

/// A request handler panicked and the request was answered with HTTP 500
/// ([`crate::panics::answer_500_on_panic`]). `route` is the route
/// template, `None` when no route matched.
pub(crate) fn http_request_panicked(route: Option<&str>) {
    let mut attributes = Vec::new();
    if let Some(route) = route {
        attributes.push(KeyValue::new("http.route", route.to_owned()));
    }
    instruments().http_requests_panicked.add(1, &attributes);
}

/// The background task `name` panicked ([`crate::panics`]). `name` is one
/// of the fixed task names in the code (`outbox_delivery`,
/// `bundle_harvest`, ...).
pub fn task_panicked(name: &'static str) {
    let attributes = [KeyValue::new("portitor.task.name", name)];
    instruments().tasks_panicked.add(1, &attributes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without `init`, every measure goes to the no-op meter: nothing
    /// fails, nothing is kept.
    #[test]
    fn outbox_names() {
        let negotiation = Outbox::Negotiation(Role::Consumer);
        assert_eq!(negotiation.process_name(), "negotiation");
        assert_eq!(negotiation.role_name(), Some("consumer"));
        assert_eq!(
            format!("{:?}", negotiation.attributes()),
            format!(
                "{:?}",
                [
                    KeyValue::new("portitor.outbox.name", "dsp"),
                    KeyValue::new("portitor.dsp.process", "negotiation"),
                    KeyValue::new("portitor.dsp.role", "consumer"),
                ]
            )
        );
        let delivery = Outbox::CredentialDelivery;
        assert_eq!(delivery.role_name(), None);
        assert_eq!(
            format!("{:?}", delivery.attributes()),
            format!(
                "{:?}",
                [KeyValue::new("portitor.outbox.name", "credential_delivery")]
            )
        );
    }

    #[test]
    fn recording_without_export_does_nothing() {
        negotiation_ended(Role::Provider, "FINALIZED");
        transfer_ended(Role::Consumer, "TERMINATED");
        dsp_request_refused(Refusal::Unauthenticated);
        outbox_resent(Outbox::Negotiation(Role::Consumer));
        outbox_given_up(Outbox::CredentialDelivery);
        harvest_run_ended("datasets", "succeeded");
        datasets_rejected("dsp", 2);
        bundle_in_use("1.0.0", true);
        bundle_files_skipped(2);
        http_request_panicked(Some("/data/{flowId}"));
        task_panicked("outbox_delivery");
        server_request(0.01, "GET", Some("/health"), 200, "1.1");
        client_request(0.01, &[]);
    }
}
