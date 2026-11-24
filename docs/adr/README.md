<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Architecture decision records

Each ADR records one decision: the context, the options and what was
chosen ([ADR 0001](0001-record-architecture-decisions.md)). An accepted
ADR is a record. It is not rewritten; a later decision is added as a
dated note or amendment, or as a new ADR. So an ADR may name a release
number or a setting that changed since. For the system as it is now,
read [ARCHITECTURE](../ARCHITECTURE.md).

The status in this table is copied from each file. Change it there
first, then here.

| ADR | Title | Status | Decision in one line |
|---|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted | Significant decisions are written as ADRs in `docs/adr/` |
| [0002](0002-technology-stack.md) | Technology stack (spike) | Accepted (2026-11-11) | Rust, after a bake-off measured against Java on Quarkus native |
| [0003](0003-component-split.md) | Component split (modules and images) | Accepted (2026-11-11) | One mono repo; three images (connector, identity, issuer), split by who runs them and which keys they hold |
| [0004](0004-hexagonal-architecture.md) | Hexagonal architecture (ports and adapters) | Accepted | Pure domain crates with no I/O; everything external is a port (trait) with an adapter |
| [0005](0005-schema-and-vocabulary-registry.md) | Schema and vocabulary registry | Accepted (2026-11-15) | The authority signs a bundle of SHACL shapes, vocabularies and contexts; connectors pull and verify it and check datasets and messages with Portitor's own SHACL Core subset |
| [0006](0006-negotiation-storage.md) | Negotiation storage | Accepted (2026-11-12) | A SQLite file behind the `NegotiationStore` port, one process per file; PostgreSQL later on the same port |
| [0007](0007-outbox-and-proof-of-delivery.md) | Save each message before sending it (outbox), and proof of delivery | Accepted (2026-11-12) | A message is saved with the state change that caused it, then sent; the state moves when the partner acknowledges or proves it has the message |
| [0008](0008-project-name.md) | Project name: Portitor | Accepted | The project, crates, images and settings are named Portitor |
| [0009](0009-minimal-odrl-policies.md) | Minimal ODRL policies | Accepted (2026-11-12) | One constraint type, `dateTime`; anything not understood fails closed; policies are read strictly |
| [0010](0010-licence.md) | Licence | Accepted (2026-11-11) | Apache-2.0 only, SPDX headers checked by REUSE, no contributor agreement |
| [0011](0011-interoperability-targets.md) | Interoperability targets | Accepted (2026-11-11) | Eclipse EDC and Simpl-Open are the reference counterparts; Gaia-X is in scope; interoperability is tested in CI |
| [0012](0012-openssf-baseline.md) | OpenSSF Baseline level | Accepted (2026-11-11) | OpenSSF Baseline level 1, with the Best Practices badge and Scorecard tracked |
| [0013](0013-transfer-and-data-plane.md) | Transfer process and the built-in data plane | Accepted (2026-11-13) | Provider transfers with HTTP pull only, a built-in data plane behind the Data Plane Signaling ports, a short-lived bearer token per flow |
| [0014](0014-transfers-as-consumer.md) | Transfers as consumer | Accepted (2026-11-13) | The consumer requests transfers only under its own finalized agreements and keeps the data address for the operator; it has no data plane |
| [0015](0015-dcp-identity-component.md) | DCP identity component (`portitor-identity`) | Accepted (2026-11-13) | The participant's wallet as its own image: DID document, STS, presentation and storage APIs, both DCP profiles |
| [0016](0016-dsp-caller-authentication.md) | DSP caller authentication with DCP | Accepted (2026-11-13) | Every DSP request needs a DCP token and a `MembershipCredential` from a trusted issuer; each process is bound to the DID that started it |
| [0017](0017-simpl-open.md) | Joining a Simpl-Open data space | Accepted (2026-11-21) | No x509/mTLS profile in Portitor; join behind the participant's Simpl Tier 2 gateway, with a gateway adapter accepted in principle and reviewed again before it is built; tested in 1.7 against `connector-be` and a locally run governance authority |
| [0018](0018-issuer-component.md) | Issuer component (`portitor-issuer`) | Accepted (2026-11-14) | One issuer per data space, run by its authority: DCP issuance to registered participants, status lists, its own image |
| [0019](0019-credential-constraints.md) | Credential constraints in ODRL policies | Accepted (2026-11-14) | A left operand `<CredentialType>.<claim path>`, as EDC does; checked at negotiation and again at transfer start |
| [0020](0020-dataset-harvest.md) | Datasets are harvested into a read copy | Accepted (2026-11-15) | Datasets and their policies live at a source outside the connector; each is checked on its own and a bad one never stops the connector |
| [0021](0021-gaia-x-onboarding.md) | Gaia-X onboarding in the identity component | Accepted (2026-11-15) | The identity publishes the X.509 chain of its key, signs its Gaia-X credentials, and gets and renews the compliance credential over the clearing house's REST API |
| [0022](0022-gaia-x-compliance-in-policies.md) | Gaia-X compliance in policies | Accepted (2026-11-15) | A compliance credential counts only from a trusted compliance service and only with the caller's own legal person credential it names by digest |
| [0023](0023-observability.md) | Health checks, traces and metrics | Accepted (2026-11-16) | EDC's health check paths on the management port; OpenTelemetry over OTLP/HTTP, off unless an endpoint is set; no trace context across participants |
| [0024](0024-several-data-spaces.md) | A provider in several data spaces | Accepted (2026-11-20) | For 1.0, one connector per data space; data space profiles in one connector after 1.0 |
| [0025](0025-users-and-roles.md) | Users and roles | Accepted (2026-11-20) | 1.0 stays system to system and users stop at the organisation's reverse proxy; identity provider tokens on the management APIs in 1.5 |
| [0026](0026-tls-termination.md) | TLS ends at the operator's reverse proxy | Accepted (2026-11-20) | The services speak plain HTTP; the operator ends TLS at a proxy and keeps the management ports private |
| [0027](0027-push-transfers-and-object-storage.md) | Push transfers, object storage and revoking access | Proposed (2026-11-24) | Push by the Data Plane Signaling HTTP profile into a receiving route of the consumer; provider data may sit in its own object storage behind `/data`; suspend and terminate cut running downloads; no storage keys cross between participants |

## Adding an ADR

Use the next number and the layout of ADR 0001, start with
`Status: Proposed`, and add a row here in the same pull request. Which
document versions the Gaia-X ADRs were written against, and what is
checked today, is in
[ARCHITECTURE, "Specification versions"](../ARCHITECTURE.md#specification-versions).
