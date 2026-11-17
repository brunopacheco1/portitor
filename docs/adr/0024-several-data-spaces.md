<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0024: A provider in several data spaces

- Status: **Proposed** (2026-11-17; waits for the maintainer)
- Date: 2026-11-17
- Scope: research only, no code (ROADMAP 0.11, "Research, docs only").
  Answers [TD-03](../RISKS_AND_TECHNICAL_DEBTS.md#td-03). Builds on ADR
  0005 (bundle), ADR 0016 (DSP caller authentication), ADR 0017
  (Simpl-Open), ADR 0019 (credential constraints), ADR 0020 (dataset
  harvest) and ADR 0022 (Gaia-X compliance in policies).
- Decisions already taken by the maintainer: the TD-03 ADR comes in 0.11
  and is decided before the 0.13 API and configuration freeze (ROADMAP,
  "Decisions on the 0.11 plan", 2026-11-17); each format is taken from a
  specification, EDC or Simpl-Open, never invented (ROADMAP, "After
  0.13", 2026-11-15).

## Context

A data provider may belong to several data spaces at once: long-lived
ones (for example the EHDS and ICUdata4EU) and short-lived ones set up
for one use case. Each data space authority may set its own rules: how
datasets are described (schemas and vocabularies), which policies and
constraints an offer may use, which credentials a consumer needs and
from which issuers, and where data may be processed. The same dataset
may need a different description and a different offer in each data
space (TD-03).

Today one Portitor connector serves one data space:

| What | Today | Where |
|---|---|---|
| Schemas and vocabularies | One signed bundle from one authority (`PORTITOR_BUNDLE_URL`, `PORTITOR_BUNDLE_AUTHORITY`) | ADR 0005 |
| Datasets and offers | One datasets file, each dataset with its offers (`PORTITOR_DATASETS_FILE`) | ADR 0020 |
| Trusted issuers | One list for every credential type (`PORTITOR_TRUSTED_ISSUERS`) | ADR 0016, ADR 0019 |
| What a caller must present | `PORTITOR_DCP_SCOPES`, `PORTITOR_POLICY_CREDENTIALS` | ADR 0016, ADR 0019 |
| Gaia-X | One list of compliance services (`PORTITOR_GAIAX_COMPLIANCE_ISSUERS`) | ADR 0022 |
| DSP endpoint | One path, `/dsp/2025-1` | ADR 0016 |
| Identity | One DID per participant; the identity stores credentials from every issuer in its own `PORTITOR_TRUSTED_ISSUERS` | ADR 0015 |

The 0.11 threat model review found that a trusted issuer is trusted for
every credential type (SECURITY-ASSURANCE, finding F-08). With several
data spaces, the issuer of data space A could then admit a participant
to data space B.

## What the specifications say (checked 2026-11-17)

| Source | What it says | Link |
|---|---|---|
| DSP 2025-1 (errata 2), terminology | A Participant is "a member of one or more Dataspaces"; a Participant Agent "does not necessarily correspond to a single runtime process" | [terminology.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/common/terminology.md) |
| DSP 2025-1, version endpoint | `/.well-known/dspace-version` lists one entry per endpoint, each with its own `path`, `serviceId`, `identifierType` and `auth`; the example shows two 2025-1 endpoints on one root, at different paths ("another Connector on the same root URL") | [common.protocol.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/common/common.protocol.md), [protocol-version.json](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/artifacts/src/main/resources/common/example/protocol-version.json) |
| DSP 2025-1, DID services | A participant "MAY publicize" its Data Services or Catalog Services as `service` entries of type `DataService` or `CatalogService` in its DID document | [common.protocol.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/common/common.protocol.md), [did-service-schema.json](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/artifacts/src/main/resources/common/did-service-schema.json) |
| DSP 2025-1, catalog | A root catalog must have one `participantId`; a catalog "MAY be dynamically generated for a request based on the requesting Participant's credentials"; catalogs may nest (`nested-catalog.json`); "the discovery protocol adopted by a particular Dataspace defines how a Consumer discovers Catalog Services" | [catalog.protocol.md](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/specifications/catalog/catalog.protocol.md), [catalog-schema.json](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/blob/d7d810b276f5f5710ff7a53bff630e2845cc0105/artifacts/src/main/resources/catalog/catalog-schema.json) |
| DCP v1.0.1, trust model | The governance authority provides "a secure list of trusted Issuer DIDs"; "Verifiers can recognize several Dataspace Governance Authorities"; "a dataspace MAY contain multiple Issuer Services" | [trust.model.md](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/blob/27175bc63ba47c13bae37dd4a158e1a1cb50af82/specifications/trust.model.md), [dataspace.ecosystem.md](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/blob/27175bc63ba47c13bae37dd4a158e1a1cb50af82/specifications/dataspace.ecosystem.md) |
| DCP v1.0.1, scopes | A verifier asks by `org.eclipse.dspace.dcp.vc.type:<type>` or `org.eclipse.dspace.dcp.vc.id:<id>`; there is no scope by issuer or data space | [verifiable.presentation.protocol.md](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/blob/27175bc63ba47c13bae37dd4a158e1a1cb50af82/specifications/verifiable.presentation.protocol.md) |
| DCP v1.0.1, credential service | One `CredentialService` entry in the holder's DID document | same file, "Credential Service Endpoint Discovery" |
| Gaia-X Architecture Document 25.11 | Each ecosystem sets its own policy rules (3.2); "Ecosystems can agree on common Trust Service Providers, ontologies, and shared services ... for participants in multiple ecosystems" (3.2.1); "ecosystems then are free to declare their individual rules for compliance" (3.3.1) | [Gaia-X context](https://docs.gaia-x.eu/technical-committee/architecture-document/25.11/gaia-x_context/), [technical compatibility](https://docs.gaia-x.eu/technical-committee/architecture-document/25.11/gaia-x_technical_compatibility_specifications/) |
| HealthDCAT-AP release 8 | EHDS datasets are checked with their own shapes, one set per access level (public, restricted, non-public); another data space may use other shapes | [public-shapes.ttl](https://data.health.europa.eu/healthdcat-ap/releases/release-8/html/shacl/HealthDCAT-AP_validator/config/rdf-validator/ehds/shapes/public-shapes.ttl), ADR 0005 |

No specification defines how one connector tells which data space a
request belongs to, or how a dataset is assigned to a data space.

## What others do (checked 2026-11-17)

### Eclipse EDC

All of the following is in the EDC Connector at tag `v1.0.0-RC7`
(commit `3ac4837`, the version the EDC interop test runs) and on `main`
(commit `c08ac62`, 2026-11-16).

| Concept | What it is | Source |
|---|---|---|
| Dataspace profile context (decision record 2025-05-28) | "enable a connector runtime to serve requests for multiple dataspaces with potentially different protocol versions, authentication, vocabulary and policies". One profile is a set of wire protocol and version, authentication protocol, policy functions, JSON-LD vocabulary, scopes and identifier resolution. "Every profile will be represented by a different protocol endpoint, and it will be advertised in the `.well-known/dspace.version` endpoint." Assets are not tied to a profile; the contract definition's `assetsSelector` can limit them. The management API carries the profile in its `protocol` field | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2025-05-28-dataspace-profile-context/README.md) |
| A participant id per profile (2025-07-25, then 2025-10-29) | The connector may use another participant id in each profile; resolved by `ParticipantIdentityResolver(participantContextId, protocol)` | [2025-07-25](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2025-07-25-multiple-participant-identifiers/README.md), [2025-10-29](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2025-10-29-participant-identifiers/README.md) |
| Participant context, "EDC Virtual" (2025-08-26) | Several participants in one runtime; assets, policies, contract definitions, negotiations, agreements and transfers carry a `participantContextId`; "the policy engine should have the capability to register and evaluate policy functions within a dataspace profile context" | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2025-08-26-participant-context/README.md) |
| Multi-profile virtual connector (2026-05-11) | One runtime serves several profiles "on a per-participant basis"; each profile has its own JSON-LD namespace and contexts and its own DSP path: `/{participantContextId}/{profileId}/catalog/request`, `/negotiations/...`, `/transfers/...`; a participant lists its profiles in `edc.dataspace.profiles`; the version endpoint lists only that participant's profiles | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2026-05-11-multi-profile-virtual-connector/README.md), [`ParticipantProfileService`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/spi/control-plane-spi/src/main/java/org/eclipse/edc/protocol/spi/ParticipantProfileService.java) |
| Trusted issuers per profile, per type | A profile is configured under `edc.dataspace.profiles.<alias>` with `name`, `protocol.version`, `protocol.binding`, `protocol.namespace`, `jsonld.context.urls` and `trustedissuers.<alias>.id` / `.supportedtypes`. `HasValidIssuer` refuses a credential whose type is not among its issuer's `supportedtypes` in that profile (`*` allows all) | [`DataspaceProfileConfigurationExtension`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/data-protocols/dsp/dsp-virtual/dsp-http-core-virtual/src/main/java/org/eclipse/edc/protocol/dsp/http/profile/DataspaceProfileConfigurationExtension.java), [`DataspaceProfileContext`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/spi/control-plane-spi/src/main/java/org/eclipse/edc/protocol/spi/DataspaceProfileContext.java), [`HasValidIssuer`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/core/common/lib/core-lib/src/main/java/org/eclipse/edc/iam/verifiablecredentials/rules/HasValidIssuer.java) |
| Partners and partner groups (2026-09-22) | Counter-parties known per participant context, for policies (management API v5 only) | [decision record](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/docs/developer/decision-records/2026-09-22-partners-and-partner-groups/README.md) |

EDC IdentityHub ([architecture](https://github.com/eclipse-edc/IdentityHub/blob/f04300c6c5ac019eab6140b8c35d191d46c76d2d/docs/developer/architecture/decentralized-claims-protocol/identity.hub.architecture.md),
commit `f04300c`, 2026-11-16): one IdentityHub holds several
participant contexts; each has its own DID
([`ParticipantManifest`](https://github.com/eclipse-edc/IdentityHub/blob/f04300c6c5ac019eab6140b8c35d191d46c76d2d/spi/participant-context-spi/src/main/java/org/eclipse/edc/identityhub/spi/participantcontext/model/ParticipantManifest.java)),
keys and credentials, and "an access token ... is associated with a
specific context and may not be used to access resources in another
context".

Not checked: whether EDC's policy engine already binds policy functions
to a profile (the 2025-08-26 record says it "should"), and whether
IdentityHub filters a presentation by issuer.

### Simpl-Open

From [ADR 0017](0017-simpl-open.md) (facts read 2026-11-13, not read
again): each participant runs a Simpl-Open agent; its Tier 2 identity is
an x509 certificate from the governance authority's CA, and its
attributes come from that authority. So a participant's identity is
given by one data space's authority. "Cross-data-space federation" is
planned for Innsbruck (5.0)
([Innsbruck items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-innsbruck-5.0)).
Not verified: whether one agent can serve several data spaces today. On
what ADR 0017 shows, a participant in two Simpl data spaces runs one
agent per data space.

## The questions of TD-03

| Question | What the sources answer |
|---|---|
| One connector with profiles, or one connector per data space? | Both are allowed by DSP ("Participant Agent ... not necessarily a single runtime process"). EDC builds profiles; Simpl-Open gives each data space its own agent |
| How does a dataset say where it is offered, and with which description and offer? | No specification. EDC: assets are shared, contract definitions select them (`assetsSelector`), distributions come per protocol |
| How does each data space express rules such as the processing location? | Its own policy vocabulary (Gaia-X 6.8.3: ODRL "in combination with an ontology typically defined by the ecosystem"; EDC: policy functions per profile). DSP's catalog example uses the ODRL left operand `spatial`. Portitor does not support it yet ([TD-12](../RISKS_AND_TECHNICAL_DEBTS.md#td-12)) |
| How is a caller's data space known? | EDC: by the URL path (`/{participantContextId}/{profileId}/...`), advertised in `/.well-known/dspace-version`. Not by the caller's credentials: a caller may hold credentials of both data spaces |
| How do data spaces publish schemas? | Still open ([TD-01](../RISKS_AND_TECHNICAL_DEBTS.md#td-01)); each authority publishes its own. Nothing here changes the bundle format |
| How are short-lived data spaces joined and left? | EDC: profiles are rows in a store (`edc_dataspace_profiles`), managed at run time through the v5 management API ([`DataspaceProfileApiV5Controller`](https://github.com/eclipse-edc/Connector/blob/v1.0.0-RC7/extensions/control-plane/api/management-api-v5/dataspace-profile-context-api-v5/src/main/java/org/eclipse/edc/connector/controlplane/api/management/participantcontext/profile/v5/DataspaceProfileApiV5Controller.java)). Simpl-Open: a new agent. What happens to running agreements when a profile is removed was not checked |

## Options

### (a) One connector per data space (deployment only)

The participant runs one Portitor connector per data space, each with
its own settings, database, datasets file and bundle. They can share
one identity and so one DID: each connector uses the same STS client id
and secret file, and the identity trusts the issuers of every data space
in its own `PORTITOR_TRUSTED_ISSUERS`. Or each data space gets its own
identity and DID.

| | Impact |
|---|---|
| Configuration | None; one set of settings per connector |
| API | None; each connector has its own DSP URL and management port |
| DID and identity | One DID works today (not tested). Several DIDs need one identity per data space, a membership and, with Gaia-X, a compliance run each |
| Storage | One database per connector; nothing shared |
| 0.13 freeze | Nothing to change |
| Cost | One more process and database per data space; the same dataset listed in several datasets files; a short-lived data space is a new deployment |

One DID has a side effect with every option: a verifier asks for a
type (DCP has no scope by issuer), so a verifier in data space A that
asks for `MembershipCredential` also gets data space B's membership
credential, if both use that type. Portitor's verifier skips it, as it
skips every credential from an issuer it does not trust, but it learns
that the participant is in B. Separate DIDs avoid this.

### (b) One connector with several data space profiles

As EDC's dataspace profile context. A profile holds, per data space:
the bundle (URL, authority, version), the trusted issuers with the
credential types each may issue (EDC's `supportedtypes`), the DCP scopes
and the credential types policies may use, the Gaia-X compliance
services, and its datasets. The caller's data space is the profile of
the URL it calls: one DSP path per profile, each listed in
`/.well-known/dspace-version` (DSP allows several entries). The current
settings become the default profile, on today's path `/dsp/2025-1`.

Two ways to give a profile its datasets:

- **(b1) one datasets file, each dataset names its profiles.** Close to
  EDC (assets shared, offers per selector). A dataset with a different
  description per data space needs one entry per data space anyway.
- **(b2) one datasets source per profile.** Each data space gets its own
  description and offers, checked against its own bundle; the upstream
  `portitor:source` can be the same. This answers TD-03's "a different
  description and a different offer in each data space" directly. No
  specification or EDC format says which; the datasets file is
  Portitor's own format already (ADR 0020).

| | Impact |
|---|---|
| Configuration | A profile list: environment variables do not nest well, so an optional profiles file (ARCHITECTURE principle 10 allows one file), with EDC's names where they exist (`name`, `trustedissuers.<alias>.id`, `supportedtypes`). Today's variables stay as the default profile |
| API | One DSP path per profile (new paths, today's path kept). The consumer side of the management API names the profile when it starts a negotiation, as EDC's `protocol` field does (an optional field, default profile when absent) |
| DID and identity | One DID by default (with the disclosure above); a DID per profile would need the identity to hold several DIDs, as IdentityHub's participant contexts do, which is option (c) on the identity side |
| Storage | A profile column on negotiations, transfers, datasets and harvest jobs, and one bundle cache folder per profile (new migrations, forward only, TD-22) |
| 0.13 freeze | Nothing must break if the default profile keeps today's settings, path and API; every addition is optional |
| Cost | The largest code change of the three that keep one deployment: every check that reads a setting today reads the request's profile instead |

### (c) Several participants in one runtime (EDC Virtual)

As EDC's participant contexts and IdentityHub: one connector and one
identity hold several participants, each with its own DID, keys,
credentials and profiles. A provider in two data spaces could be two
participant contexts.

| | Impact |
|---|---|
| Configuration | Per participant, stored in the database and managed by API (EDC's participant context configuration), not in the environment |
| API | DSP paths with the participant (`/{participantContextId}/{profileId}/...`); a management API per participant (EDC's v5 `/v5/participants/{id}/...`) |
| DID and identity | One DID per participant context; the identity becomes multi-tenant (keys, credentials, STS clients per context) |
| Storage | A participant column on every table of the three components |
| 0.13 freeze | Breaks the management APIs and much of the configuration: before the freeze, or in 2.0 |
| Cost | A multi-tenant redesign, against "few components" and "small and simple" (ARCHITECTURE principles 3 and 4). It serves hosting providers more than one provider in several data spaces |

### (d) Simpl-Open's way

One agent per data space, its identity given by that data space's
authority. For Portitor this is option (a) with one DID per data space.
Nothing else to build.

## Decision (proposed)

1. **For 1.0: option (a).** One connector per data space, with one
   shared identity or one identity per data space. It works without new
   code; DEPLOYING says how, with the disclosure of a shared DID. The
   setup is checked once by hand before 0.13 (not tested so far).
2. **After 1.0: option (b), EDC's dataspace profile model, with (b2),
   one datasets source per profile,** behind its own release and code
   ADR. It is additive: the default profile keeps today's settings,
   path and API, so it needs no breaking change and nothing at the 0.13
   freeze beyond this ADR.
3. **Trusted issuers by credential type,** as EDC's `supportedtypes`
   (finding F-08): an optional addition to `PORTITOR_TRUSTED_ISSUERS`,
   with "all types" as the default, so it is additive too. It is useful
   with one data space already (the HDAB issuer of 1.3). Format to copy
   from EDC; the release is the maintainer's choice.
4. **Not option (c).** Several participants in one runtime is a
   hosting feature; revisit only if a user asks for it.

Reasons: DSP and DCP allow (a) and (b); (a) costs nothing now and is
what Simpl-Open does; (b) is what EDC built and ships in the RC7 that
Portitor tests against, so its names and shape can be reused; neither
needs a break of the 1.0 API or configuration.

## Open questions for the maintainer

1. Is (a) enough for 1.0, or must one connector serve two data spaces
   in 1.0?
2. One DID for all data spaces (simpler, but a verifier learns the other
   memberships when credential types are shared), or one per data space
   (private, but one identity, membership and Gaia-X run each)?
3. (b1) or (b2) for datasets?
4. Should trusted issuers by type (point 3) come before 1.0? It is
   additive, so it can wait.
5. Is there a short-lived data space to try this with (for example the
   ICUdata4EU demo of ROADMAP "The health data space demo")?

## Not verified

- That two Portitor connectors can share one identity and STS client
  (read in the code, not run).
- Whether EDC binds policy functions to a profile in RC7, and whether
  an EDC verifier in profile A refuses a presentation that holds a
  credential from profile B's issuer, or skips it.
- How EDC advertises profile paths in `/.well-known/dspace-version` on
  the wire (read in the decision record only).
- The Simpl-Open facts, taken from ADR 0017 (2026-11-13) and not read
  again.
- How the EHDS (Regulation (EU) 2025/327) and ICUdata4EU expect a data
  holder in both to describe the same dataset.

## Consequences

- TD-03 points here; it stays Open until the maintainer decides.
- With (a): DEPLOYING gets a section "Several data spaces" before 0.13.
- With (b): a code ADR and a release after 1.0; the profile names and
  settings follow EDC's.
- Finding F-08 (one trusted issuer list for every type) is handled by
  point 3, whatever is chosen for the rest.
