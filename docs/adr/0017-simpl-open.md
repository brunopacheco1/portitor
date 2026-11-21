<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0017: Joining a Simpl-Open data space

- Status: **Accepted** (2026-11-21, by the maintainer; answers in
  [Maintainer's answers](#maintainers-answers-2026-11-21)). The second
  authentication mode of decision 2 is accepted in principle only and
  reviewed again before it is built.
- Note (2026-11-20): still Proposed at the 0.13 freeze; interoperability with Simpl-Open has not been tested ([STABILITY](../STABILITY.md)).
- Date: 2026-11-13
- Scope: the Simpl-Open spike of release 0.5 (roadmap, 0.5). Builds on
  ADR 0011 (interop targets), ADR 0015 (identity component) and ADR 0016
  (DSP caller authentication).
- Question asked: how would Portitor join a Simpl-Open data space? Does
  Portitor need an x509/mTLS authentication profile, and what else is
  missing?

## Context

Simpl is the European Commission's programme for a common middleware for
the EU data spaces. Simpl-Open is that middleware, open source. ADR 0011
names Simpl-Open as a reference counterpart next to Eclipse EDC, and the
roadmap plans an interop test against it in 0.9.

The earlier ADRs (0002, 0003) read the
[Simpl-Open Q&A](https://simpl-programme.ec.europa.eu/landing-page/simpl-open-qa)
as "inside its agent Simpl-Open uses x509 and mTLS, outside its agent it
uses DCP". This spike checked that against the code. The answer is
different in one important point: **no public Simpl-Open code uses DCP**.
The "outside the agent" part of the Q&A describes ordinary EDC
deployments that are not Simpl agents (the answer explains why Simpl is
built on EDC), not a second way into a Simpl data space.

All facts below were read on 2026-11-13. Code links point to
code.europa.eu (GitLab). The code is moving from the old
`simpl-open/development/...` tree, now archived, to new groups by
capability (`integration/`, `security/`, ...), so links to the old tree
may show an archived project.

## Facts

### What Simpl-Open is and where it lives

| Fact | Source |
|---|---|
| Releases have city names. Gdynia (4.0) was released in July 2026 (session recap of 9 July 2026; Data Provider agent v4.0.1 tagged 2026-07-22). Hamburg (4.1, target October 2026) and Innsbruck (5.0, target Q4 2026) are planned. | [Development status](https://simpl-programme.ec.europa.eu/development/development-status), [Coffee with Simpl 3 recap](https://simpl-programme.ec.europa.eu/topic/coffee-simpl-3-session-recap), [data-provider v4.0.1](https://code.europa.eu/simpl/simpl-open/development/agents/data-provider/-/releases/v4.0.1) |
| Code: [code.europa.eu/simpl/simpl-open](https://code.europa.eu/simpl/simpl-open). The `development/` tree is "legacy, being migrated" and its projects are archived (for example `gaia-x-edc/simpl-edc`, `iaa/tier1-gateway`). New code lives in groups by capability. | [About Simpl-Open](https://code.europa.eu/simpl/simpl-open/gitlab-profile), [simpl-edc (archived)](https://code.europa.eu/simpl/simpl-open/development/gaia-x-edc/simpl-edc) |
| Architecture: "D1.3.4 Functional and Technical Architecture Specifications", version 0.1, December 2025. The Installation Guide (D1.6.1) deploys the middleware on Kubernetes with Helm. | [Architecture](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture), [Installation guide](https://ssg-website-ad5a79.pages.code.europa.eu/docs/guides/installation-guide) |
| Each participant runs a Simpl-Open **agent**: "a middleware, to be deployed on each node, acting as a local gateway" (section 2.2.1). The governance authority (GA) runs its own agent. | [Architecture](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture) |

### Identity, onboarding and the two tiers

| Fact | Source |
|---|---|
| **Tier 1** is people inside one organisation: Keycloak, roles (RBAC), the Tier 1 gateway stays on the internal network. Gdynia added sign-in with an organisation's own identity provider. | [Architecture, 2.2.2 and 2.2.3](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture), [Gdynia items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-gdynia-4.0) |
| **Tier 2** is organisation to organisation (agent to agent). "Only the Tier 2 Gateway is designed to be exposed through the firewall"; it enforces attribute-based access (ABAC). | [Architecture, 2.2.3](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture) |
| Onboarding: the participant creates a key pair and sends a credential request with its public key and fields such as OU, City, CN, FQDN to the GA (requirement SIMPL-1682, status "Proposed"). The GA "relies on a x509 Certification Authority building block"; these identities are used for Tier 2 only. The authentication provider code has a `CSRService`. | [3A42 create credential request](https://simpl-programme.ec.europa.eu/book-page/3a42-create-credential-request), [Architecture, 2.4](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture), [authentication-provider-be](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/authentication-provider-federation/authentication-provider/authentication-provider-be) |
| The Tier 2 handshake (section 2.3): the two agents authenticate each other with "mutual x509 TLS Authentication", check the certificate, then the consumer gets an **ephemeral proof** of its identity attributes from the GA's identity provider federation and sends it to the provider, which checks it and applies ABAC. | [Architecture, 2.3](https://ssg-website-ad5a79.pages.code.europa.eu/resources/architecture) |
| In code, the provider's `tier-2-gateway` (Spring Cloud Gateway) requires a client certificate on every call except `OPTIONS`. A session is opened with `POST /ephemeralProof` (mTLS) carrying a JWT; `OPTIONS /ephemeralProof?hash=...` checks a session. The example proof has `transportCertsSha256` (bound to the TLS certificate) and `identityAttributes`. The gateway strips identity headers the caller set itself. The consumer side uses `tier-2-proxy` for outgoing calls. | [tier-2-gateway README](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/supporting-security-services/gateway/tier-2-gateway/-/blob/main/README.md), [tier-2 API v3](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/authentication-provider-federation/authentication-provider/authentication-provider-be/-/blob/main/openapi/authenticationprovider-tier2-v3.yaml), [tier-2-proxy README](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/supporting-security-services/gateway/tier-2-proxy/-/blob/main/README.md) |
| A second session mode, `VERIFIABLE_PRESENTATION`, exists next to the default `EPHEMERAL_PROOF`. The authentication provider issues credentials as `vc+jwt`, VC Data Model 2.0, issuer `did:web:...`, with `BitstringStatusListEntry` status. The session POST stays mTLS in both modes. Neither README mentions DCP. | [authentication-provider-be README](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/authentication-provider-federation/authentication-provider/authentication-provider-be/-/blob/main/README.md), [tier-2-proxy README](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/supporting-security-services/gateway/tier-2-proxy/-/blob/main/README.md) |
| Planned: "Self Sovereign Identification (SSI) Tier 1" in Hamburg; "SSI Tier 2" ("verifiable credentials and decentralized identifiers") and cross-data-space federation in Innsbruck. The Q&A says SSI Tier 2 "will provide both identity attributes as VC and x509 credentials as DID". | [Hamburg items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-hamburg-4.1), [Innsbruck items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-innsbruck-5.0), [Simpl-Open Q&A](https://simpl-programme.ec.europa.eu/landing-page/simpl-open-qa) |

### The connector: EDC, DSP and DCP

| Fact | Source |
|---|---|
| The current connector is `connector-be`, v1.3.0 tagged 2026-11-13. Its `pom.xml` sets `<edc>0.17.0</edc>` (EDC tag of 2026-04-20). Tag notes: "Move the cached catalogue contract to EDC 0.17.0 and Dataspace Protocol 2025-1". | [connector-be pom.xml](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/pom.xml), [connector-be tags](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/tags) |
| EDC 0.17.0's `dsp` bundle contains only `dsp-2025` (DSP **2025-1**), the version Portitor speaks. Checked in the EDC source at tag v0.17.0 (`data-protocols/dsp/build.gradle.kts`). | [EDC v0.17.0](https://github.com/eclipse-edc/Connector/tree/v0.17.0/data-protocols/dsp) |
| The archived `simpl-edc` (last tag v1.0.26, 2026-08-10) used EDC 0.10.1, which offers DSP `v0.8` and `2024/1` only (checked in the EDC source at tag v0.10.1). Portitor cannot talk to it. | [simpl-edc pom.xml at v1.0.26](https://code.europa.eu/simpl/simpl-open/development/gaia-x-edc/simpl-edc/-/blob/v1.0.26/pom.xml), [EDC v0.10.1](https://github.com/eclipse-edc/Connector/tree/v0.10.1/data-protocols/dsp) |
| **No DCP.** `connector-be`'s dependencies have no DCP, IdentityHub or `decentralized-claims` module. Its own `SimplIdentityService` (an EDC `IdentityService`) sends a **plain JSON token** (`region`, `audience`, `clientId`, `identityAttributes`), not a signed JWT, and on receipt only parses it: no signature, issuer or expiry check. It reads its own attributes from the authentication provider (`/participant/identityAttributes`). So the trust comes from the Tier 2 gateway and mTLS in front of it, not from the connector. | [SimplIdentityService.java](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/src/main/java/eu/europa/ec/simpl/iam/service/SimplIdentityService.java), [connector-be pom.xml](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/pom.xml) |
| Access policies use an ODRL left operand `https://ec.europa.eu/simpl/profile#identityAttributes` with `odrl:hasPart` (one code) or `odrl:isAllOf` (a list), for example `CONSUMER`; other operators deny. | [identity-attributes-policy.md](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/documents/identity-attributes-policy.md) |
| Simpl's EDC extensions: identity (IAM), identity-attribute policies, IONOS S3 data plane, Kafka streaming (Tractus-X extension), a contract manager hook and a transfer hook that triggers data orchestration. For local tests the connector runs without gateway, with `mocked.agent.identity.attributes`. | [connector-be pom.xml](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/pom.xml), [provider-config.properties](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/blob/main/local/provider-config.properties) |

A third-party gap analysis of the same question
([MinimumViableHealthDataspacev2](https://github.com/ma3u/MinimumViableHealthDataspacev2/blob/main/docs/simpl-ehds-gap-analysis.md),
updated 2026-11-13) also finds no DCP in Simpl-Open's code. It still
describes the archived `simpl-edc` as the current connector.

## What a non-EDC connector needs to take part

DSP itself is no longer the problem: `connector-be` and Portitor both
speak DSP 2025-1. The gaps are around it:

1. **Network identity (Tier 2).** An x509 client certificate from the
   GA's CA, mTLS to the other agent's Tier 2 gateway, and a Tier 2
   session (ephemeral proof, or a presentation in VP mode) before any
   DSP call. Inbound, a gateway that checks the same for callers.
2. **Identity attributes.** The caller's attributes (for example
   `CONSUMER`) come from the GA, and the provider's policies test them.
   Portitor's verifier (ADR 0016) knows DCP credentials, not Simpl
   attributes.
3. **The Simpl token.** `connector-be` expects its plain JSON token in
   the DSP `Authorization` header. Portitor sends a DCP self-issued
   JWT, which `connector-be` cannot parse.
4. **Participant id.** ADR 0016 makes the verified DID the participant
   id. In Simpl the participant id is the agent's own id, not a DID.
5. **Onboarding and the rest of the agent.** The GA onboarding process
   (credential request, certificate), resource descriptions in the
   federated catalogue, contracts and the transfer types Simpl uses (S3
   push, Kafka). Portitor has HTTP pull only. These are outside this
   ADR but part of "joining".

## Options

### (a) Run the Simpl agent's Tier 2 parts beside Portitor

Portitor sits behind the participant's own Simpl agent: inbound DSP
calls come through the `tier-2-gateway`, outbound ones go out through
the `tier-2-proxy`, and the authentication provider holds the
certificate and talks to the GA. Portitor adds a second caller
authentication adapter, "behind a trusted Simpl gateway": it reads the
caller's identity and attributes from what the gateway forwards, and
sends the Simpl JSON token on outgoing calls.

- Cost in Portitor: small (an adapter on the ADR 0016 port, the Simpl
  token, the identity-attribute policy operand). Cost in operations:
  high: a Java stack (gateway, proxy, authentication provider,
  Keycloak, Redis, PostgreSQL, a vault) on Kubernetes next to a small
  Rust connector. It undoes "small and simple" for Simpl users, but the
  participant runs a Simpl agent for the rest of Simpl anyway.
- Risks: the connector must trust headers from the gateway, so the
  network between them must be private; Simpl's internal APIs are not
  specifications and change often (the tier 2 API has v2 and v3 side
  by side, a gateway flag was renamed as a breaking change, the old
  repos are archived).
- Depends on: whether the agent's Tier 2 parts can run without the rest
  of the agent, and on the exact way the gateway forwards identity to
  the connector (not confirmed, see below).

### (b) An x509/mTLS profile inside Portitor

Portitor itself holds the GA certificate, does mTLS in and out, opens
Tier 2 sessions (`POST /ephemeralProof`), gets ephemeral proofs from
the GA and checks callers' proofs.

- Cost: large. A CSR and onboarding client, client certificates in the
  HTTP sender, TLS termination with client certificates in Portitor (today
  TLS ends at the operator's reverse proxy), the session protocol in both
  directions, the GA's APIs, and certificate status checks.
- Risks: high. It reimplements a protocol that exists only as Simpl's
  code and OpenAPI files, not as a specification, and that is changing
  (SSI Tier 2 in Innsbruck). No TCK. Every Simpl release can break it.
  It also adds a second, non-standard identity model next to DCP,
  against ARCHITECTURE principle 1 (standards first).
- Depends on: Simpl keeping the Tier 2 protocol stable, and access to a
  GA (test CA) to try it.

### (c) DCP only

Portitor keeps DCP (0.5) and joins wherever Simpl uses DCP.

- Cost: none beyond 0.5.
- Today it gives nothing for Simpl: no public Simpl component speaks
  DCP. It works with EDC deployments outside Simpl, which the 0.5 EDC
  test already covers.
- Worth keeping in view: Simpl's VP mode already uses the same credential
  format as Portitor's `vc20-bssl/jwt` profile (VC 2.0 `vc+jwt`,
  `did:web`, Bitstring Status List), but sends it over its own session
  API with mTLS, not over DCP. If SSI Tier 2 moves to DCP, (c) becomes
  the cheap path.

## Decision

1. **No x509/mTLS profile in Portitor** (option b), not in 0.5 and not
   planned. Revisit only if Simpl publishes the Tier 2 protocol as a
   specification, or a user needs it and option (a) is not possible.
2. **The path into a Simpl data space is option (a)**: Portitor behind
   the participant's Simpl Tier 2 gateway and proxy, with a small
   "trusted Simpl gateway" authentication adapter. It is built only
   when there is a Simpl data space (or the Simpl sandbox) to test
   against, after a short test of how the gateway forwards identity.
3. **Keep DCP as Portitor's own identity model** (option c) and watch
   each Simpl-Open release for how SSI Tier 2 is built (DCP, OpenID4VP,
   OpenID4VCI or something else). If agents talk DCP to each other, the
   adapter of point 2 is no longer needed. No questions upstream and no
   GitLab account (maintainer, 2026-11-17 and 2026-11-21).

Reasons: DSP already matches (2025-1), so the gap is only identity, and
Simpl places identity in its gateway, not in the connector. Using that
gateway costs Portitor little code; copying it costs a lot and ties
Portitor to an unspecified, moving protocol. DCP stays the standard
Portitor invests in.

### What to do in which release

- **0.5 (now):** this ADR. Correct the roadmap's reading of the Q&A.
  Keep the caller authentication in ADR 0016 behind a port, so a second
  adapter can be added later without changing the DSP code.
- **0.6 (credential-based policies):** make the policy operand for
  credentials general enough that Simpl's `identityAttributes` operand
  (`hasPart`, `isAllOf`) can be added as one more operand.
- **0.9 (interop tests; now 0.12 in the ROADMAP, and parked under
  "Later" until a Simpl-Open sandbox exists):** replace "Interoperability tests against
  Simpl-Open" with a DSP-level test against `connector-be` in its local
  mode (mocked identity attributes, no gateway). It needs only the Simpl
  JSON token on outgoing calls, in the test build. A full test through
  the Tier 2 gateway follows only with the adapter of decision 2.
- **Later (when a Simpl data space needs Portitor):** the "trusted
  Simpl gateway" adapter, the participant id mapping, and S3 push
  (already under "Later" in the roadmap).
- **1.7 (Simpl-Open tests, after the maintainer's answers of
  2026-11-21):** step 1 is the DSP-level test against `connector-be`
  above; step 2 runs Simpl-Open's governance authority locally from its
  Helm charts, to get a Tier 2 certificate and identity attributes, and
  needs the review of answer 3 first
  ([ROADMAP, 1.7](../ROADMAP.md#17-simpl-open-tests)).

## Maintainer's answers (2026-11-21)

The six questions this ADR asked, with the maintainer's answers.

| # | Question | Answer | What it changes |
|---|---|---|---|
| 1 | Is there a real Simpl-Open data space (or the Simpl sandbox) that Portitor should join, and by when? | No shared one. "We can run locally Simpl-Open governance authority with helm-charts to set it up." | Decision 2 no longer waits for a sandbox: the test runs a governance authority locally (1.7, step 2). What that needs is under [Running a governance authority locally](#running-a-governance-authority-locally). |
| 2 | Is it acceptable that a Simpl participant runs Simpl's Java Tier 2 stack next to Portitor (option a)? | Yes. | Option (a) stays the path into a Simpl data space. |
| 3 | Is a second authentication mode that trusts a gateway on a private network acceptable next to DCP? | "In principle yes, but let's review this in a later state." | **Deferred, not decided.** The "trusted Simpl gateway" adapter is not built until the maintainer reviews it again, with its own ADR, before 1.7 step 2. Until then DCP stays the only way to call Portitor. |
| 4 | Should we ask the Simpl programme whether SSI Tier 2 will use DCP, and whether non-EDC connectors are in scope? | No question upstream. "Let's keep an eye on Simpl-Open releases, I remember reading something about implementing OIDC4VC and OIDC4VP." | Decision 3 now says "watch releases" instead of "ask". What was found about OpenID4VP and OpenID4VCI is under [OpenID for Verifiable Credentials in Simpl-Open](#openid-for-verifiable-credentials-in-simpl-open). |
| 5 | Should the Simpl-Open interop test be the DSP-level test against `connector-be`? | Yes. | 1.7 step 1 is that test. |
| 6 | ADRs 0002, 0003 and ARCHITECTURE say Simpl-Open uses DCP "outside its agent". Correct them? | "Yes, fix them." | ARCHITECTURE is corrected; ADRs 0002 and 0003 keep their text and get a note that points here. |

### Running a governance authority locally

Read on 2026-11-21 in Simpl-Open's public code.

| Fact | Source |
|---|---|
| Hamburg (4.1) is out: the governance authority chart 4.1.0 is dated 2026-11-05, "Part of the Hamburg release", latest 4.1.2 of 2026-11-10. The Installation Guide D1.6.5 is at version 4.1 (5 Nov 2026). This answers the first item of "Not verified". | [governance-authority-agent-chart CHANGELOG](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/governance-authority-agent-chart/-/blob/main/CHANGELOG.md), [Installation Guide](https://code.europa.eu/simpl/simpl-open/cross-cutting/documentation/installation-guide/-/blob/main/README.md) |
| The governance authority is a master Helm chart that creates one **Argo CD `Application`**; Argo CD then installs the sub-charts `authority-iaa` (identity, authentication and authorisation, 1.9.19), `authority-gaia-x-edc` (1.2.7) and `authority-monitoring` (1.0.1) from the code.europa.eu Helm registry. So Argo CD is needed, not only Helm, and the cluster must reach code.europa.eu. | [governance-authority-agent-chart README and values.yaml](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/governance-authority-agent-chart) |
| The "Common Components" are installed first, by their own master chart: OpenBao (3 replicas by default), the Vault secrets webhook, Kafka (Strimzi), the Zalando Postgres operator, Elastic (ECK), plus optional development tools (Redpanda console, pgAdmin, Mailpit). | [common-components-chart](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/common-components-chart) |
| Tools, in install order: Git 2.47+, Helm 3.14+, Kubernetes 1.33.x (mandatory, tested on OVH), a load balancer (MetalLB, NodePort or a tunnel for a local cluster), ingress-nginx 1.13+, cert-manager 1.19+, nfs-provisioner 4.0+, Argo CD 3.2+ (last). external-dns is optional; kube-state-metrics 2.18+ is needed by the common components. | [Prerequisites](https://code.europa.eu/simpl/simpl-open/cross-cutting/documentation/installation-guide/-/blob/main/Prerequisites.md) |
| Smallest size ("experimental environment", `resourcePreset: low`, one person, idle): governance authority + common components need 2 worker nodes of 4 CPU and 16 GB RAM each, and 11 GB of ReadWriteOnce volumes. A data provider + common components need the same again. | [Prerequisites](https://code.europa.eu/simpl/simpl-open/cross-cutting/documentation/installation-guide/-/blob/main/Prerequisites.md) |
| Network: one IP for the ingress and one more per agent for its Tier 2 gateway (`tls.authority.{namespaceTag}.{domainSuffix}`), and DNS names under a domain suffix for each front end and back end. | [governance-authority-agent-chart README](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/governance-authority-agent-chart), [Prerequisites](https://code.europa.eu/simpl/simpl-open/cross-cutting/documentation/installation-guide/-/blob/main/Prerequisites.md) |
| After deployment the authority must be initialised by hand through an HTTP API; the Tier 2 proxy does not work until then. | [governance-authority-agent-chart README](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/governance-authority-agent-chart), [authority-iaa-chart user manual](https://code.europa.eu/simpl/simpl-open/cross-cutting/agents/agent-iaa/authority-iaa-chart/-/blob/v1.5.32/documents/user-manual/README.md) |
| `connector-be` is still v1.3.0 (2026-11-13) on EDC 0.17.0. | [connector-be tags](https://code.europa.eu/simpl/simpl-open/integration/resource-sharing/resource-sharing-runtime/connector/connector-be/-/tags) |

So step 1 of the 1.7 test (DSP against `connector-be` in its local
mode) needs no governance authority. Step 2 needs the governance
authority and the common components on a local Kubernetes cluster of
about 8 CPU and 32 GB RAM, plus a Simpl participant agent whose Tier 2
gateway sits in front of Portitor, which needs more (the guide gives no
figure for both agents on one cluster).

### OpenID for Verifiable Credentials in Simpl-Open

Read on 2026-11-21.

| Fact | Source |
|---|---|
| Simpl-Open has an "SSI Verifier" Keycloak extension (v0.2.0, 2026-11-20, R4.1 fixes). It adds a Keycloak identity provider of type "OpenID for Verifiable Presentation": a user signs in by presenting a credential from a wallet (QR code, `openid4vp://` request). The verifier itself is walt.id (the only supported type); accepted credentials are DCQL queries kept in the Users and Roles service. | [ssi-verifier keycloak-extension README and user manual](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/authentication-provider-federation/ssi-verifier/keycloak-extension) |
| That is **Tier 1** (people signing in to an agent), the Hamburg item "SSI Tier 1": "Enable end users to authenticate using verifiable credentials stored in digital wallets". | [Hamburg items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-hamburg-4.1) |
| The extension's test guide gets a credential from walt.id's demo issuer with an `openid-credential-offer://` link (OpenID4VCI). No Simpl-Open issuer that speaks OpenID4VCI was found: the README of Simpl's `issuer-service-be` describes an issuing and verifying API, not OpenID4VCI. | [ssi-verifier user manual](https://code.europa.eu/simpl/simpl-open/security/access-control-and-trust/authentication-provider-federation/ssi-verifier/keycloak-extension/-/blob/main/documents/user-manual/README.md), [issuer-service-be](https://code.europa.eu/simpl/simpl-open/security/credential-management/vc-issuance-verification/issuer-service/issuer-service-be) |
| Agent to agent (**Tier 2**) stays mTLS with an ephemeral proof. "SSI Tier 2" is planned for Innsbruck (5.0) and its roadmap text names verifiable credentials and decentralized identifiers, but no protocol (not DCP, not OpenID4VP). | [Innsbruck items](https://simpl-programme.ec.europa.eu/development/development-status/simpl-open-roadmap-items-innsbruck-5.0) |

So the maintainer's memory is right for OpenID4VP, but for user sign-in
(Tier 1), not for connectors. It matters for Portitor's own user
sign-in work (roadmap 1.5) more than for DSP calls. What to watch at
each Simpl-Open release: the protocol SSI Tier 2 uses between agents.

## Not verified

- Whether `connector-be` 1.3.0 belongs to a Simpl-Open release
  (Hamburg 4.1.0 was released on 2026-11-05, before that tag; see
  [Running a governance authority locally](#running-a-governance-authority-locally)).
- Whether the governance authority and the common components install on
  a local cluster (kind, k3d or similar) on a laptop: the guide is
  tested on OVH, and says other Kubernetes versions "may work".
- How the provider's gateway passes the caller's identity to
  `connector-be`: the gateway README says it removes `Authorization`
  on the session route and strips untrusted identity headers, while
  `SimplIdentityService` reads a JSON token. The exact header path was
  not found.
- The format and signature of the ephemeral proof beyond the OpenAPI
  example (the example header says `HS256`), and how a participant
  obtains one.
- What the VP-mode presentation looks like on the wire, and whether it
  follows any part of DCP. Neither README mentions DCP.
- How the GA's CA handles revocation (the code has a
  `CertificateStatusService`; the architecture document names no OCSP
  or CRL).
- Whether HTTP pull works with `connector-be` (it includes EDC's HTTP
  data plane, but Simpl's documented transfers are S3 push and Kafka).
- The rest of the architecture document after its first 100,000
  characters; Simpl's GitLab pages render most files in the browser
  only, and `git clone` from code.europa.eu was blocked here.
- The third-party claim that the DSP TCK passes 1 of 65 tests against
  `simpl-edc` 1.0.26; only its DSP versions were checked in EDC's code.

## Consequences

- No new authentication code in 0.5. Portitor's 0.5 identity stays DCP
  only. After the maintainer's answers (2026-11-21) this still holds:
  the gateway adapter waits for its review (answer 3).
- Joining a Simpl data space needs, later, a small adapter in Portitor
  and the Simpl Tier 2 stack beside it. Until then Portitor is not a
  Simpl participant.
- The roadmap's 0.5 spike line is corrected and links here; ADR 0011
  gets a short correction on what Simpl-Open's connector is.
- Simpl changes fast. Re-check these facts before any Simpl work, and
  at each Simpl release (roadmap watch item: SSI Tier 2 protocol).
- ADRs 0002 and 0003 carry a note that points here; ARCHITECTURE no
  longer says a Simpl participant can use "Simpl-Open x509 only" instead
  of `portitor-identity`.
