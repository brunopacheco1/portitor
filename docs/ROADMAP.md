<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Roadmap (draft)

Status: **draft for review**. Releases are small and frequent. Each 0.x
release is usable on its own and ends with a tag, a signed image and a
changelog entry. Version numbers are shared by every component (mono repo).

Each item lists its exit criteria, so "done" is checkable.

## 0.0 Spike: choose the technology stack (done 2026-11-11)

- Compare candidate languages: [ADR 0002](adr/0002-technology-stack.md).
- Timeboxed bake-off (3 days): version endpoint + catalog request + DSP TCK
  catalog tests + DCP self-issued token, with measured footprint.
- Exit: ADR 0002 accepted by the maintainer. Done: Rust accepted on
  2026-11-11 after the side-by-side with Quarkus native.
- Then ADR 0003: evaluate and decide how the system splits into
  components (modules and images).

## 0.1 Repository foundations (released 2026-11-12 as v0.1.0)

Mirror the set-up already proven in
[praecepta](https://github.com/brunopacheco1/praecepta).

Community and licensing files:
- `LICENSE` (Apache-2.0), `LICENSES/`, `REUSE.toml`, SPDX headers, `reuse lint` in CI.
- `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1), `CONTRIBUTING.md`,
  `GOVERNANCE.md`, `SECURITY.md` (private vulnerability reporting),
  `CHANGELOG.md` (Keep a Changelog), `CODEOWNERS`, issue forms, PR template.

Docs:
- `docs/ARCHITECTURE.md`, `docs/SECURITY-ASSURANCE.md`, `docs/RELEASING.md`,
  `docs/ROADMAP.md`, `docs/adr/`.

Repository configuration (GitHub settings, done by the owner, recorded in docs):
- **Ruleset on `main`**: changes only through pull requests; 1 approval
  from a code owner; approval of the latest push; stale approvals dismissed;
  branch up to date before merge; signed commits; required status checks
  (build and test, style, `reuse lint`, Sonar quality gate); no force push or
  deletion. Admin bypass only for merging a pull request, because a sole
  maintainer cannot approve their own (Scorecard Branch-Protection expected
  8/10 until a second maintainer joins).
- **Security**: private vulnerability reporting, secret scanning, push
  protection, Dependabot alerts and security updates, CodeQL code scanning.
- **Updates**: `dependabot.yml` for the language ecosystem, GitHub Actions
  and Docker base images, weekly, with tooling updates grouped and runtime
  dependencies kept separate.
- **Release environment** `release` with its own secrets; releases start
  from a **signed, annotated `v*` tag**.
- Repository successor set, for continuity.

CI (every action pinned to a commit SHA, `permissions: contents: read` by default):
- build and test; style check and linter as errors; coverage to Sonar.
- `reuse lint`; CodeQL; OpenSSF Scorecard (published, README badge).
- Dependency review on pull requests (fail on moderate or worse);
  OSV-Scanner on `Cargo.lock`; licence allowlist, RustSec advisories and
  crate sources with cargo-deny (`deny.toml`); ORT.
- Container image: built on every PR, scanned, not pushed.

Release (on signed tag):
- multi-arch image pushed to GHCR, signed (cosign keyless), SBOM
  (CycloneDX) and build provenance (`actions/attest-build-provenance`)
  attached; GitHub release with changelog notes and third-party notices.

Website:
- Static site in `site/`, published by GitHub Pages: what Portitor is, its
  objectives, the minimum functionality, the component list and the
  deployment order, live badges.

OpenSSF:
- Register at bestpractices.dev (done: project 16578, 91% at release).
  Reaching **passing** moved to 0.2.
- OpenSSF Baseline **level 1** (version 2026-08-28) is the target for all
  of 0.x ([ADR 0012](adr/0012-openssf-baseline.md)). The written
  self-assessment moved to the hardening release (maintainer,
  2026-11-12), renumbered 0.10 on 2026-11-14; since the split of
  2026-11-15 it is in 0.13, the release candidate.

Exit: a "hello" binary ships as a signed image through the full pipeline,
all checks green, Scorecard published.

## 0.2 DSP basics: version and catalog (released 2026-11-12 as v0.2.0)

- `/.well-known/dspace-version` (version discovery).
- Catalog protocol (provider side): catalog request, dataset request,
  datasets defined in a config file.
- DSP message validation against the DSP 2025-1 JSON schemas.
- DSP TCK in CI (catalog tests must pass).
- Raise the OpenSSF scores (decided 2026-11-12):
  - Best Practices from 91% to 100% **passing**: answer the open
    criteria with evidence (release notes, test coverage, bug and
    enhancement responses, secure design knowledge).
  - Scorecard from 6.7/10 (2026-11-12). Checks we can move now:
    CII-Best-Practices (2 to 5 at passing), Signed-Releases (v0.1.0 has
    provenance; confirm on the next scan), Fuzzing (fuzz the DSP and DCP
    parsers with cargo-fuzz, run by ClusterFuzzLite, which Scorecard
    detects), Branch-Protection (0 although a ruleset exists; find out
    why). Checks that only time or more people fix: Maintained (project
    under 90 days old), Code-Review (needs a second maintainer),
    Contributors.
- Exit: DSP TCK catalog tests green; Best Practices badge at **passing**;
  Scorecard fuzzing and branch protection no longer 0.
- Result (2026-11-12): all met. Best Practices **passing** (100%).
  Scorecard 6.7 to 7.9: Fuzzing 10, CII-Best-Practices 5,
  Signed-Releases 10, Branch-Protection 5. Branch-Protection was 0
  because the `main` ruleset targeted no branch; fixed.

## 0.3 Contract negotiation (released 2026-11-12 as v0.3.0)

- Provider and consumer state machines, persisted in a SQLite file
  (ADR 0006); a PostgreSQL adapter follows later on the same port.
- Minimal ODRL policy support: permit all, and one constraint type.
- Management API to start a negotiation (consumer side), on its own port
  with an API key; its OpenAPI description is generated from the code.
  The DSP endpoints keep the hand-written `docs/api/openapi.json`, which
  uses the official DSP schemas.
- Exit: DSP TCK negotiation tests green; two Portitor instances agree a contract.
- Result (2026-11-12): met. The DSP TCK passes the 15 provider and 16
  consumer negotiation tests (CN:01 to CN:03, CN_C:01 to CN_C:03).
  `scripts/demo.sh` has two connectors agree a contract and refuse
  changed terms; CI runs it on every pull request. Also in 0.3: outbox
  with proof of delivery (ADR 0007), minimal ODRL (ADR 0009), and the
  rename to Portitor (ADR 0008).

## 0.4 Transfer process and HTTP data plane (released 2026-11-13 as v0.4.0)

- Transfer process state machines (provider and consumer).
- Data plane in the same binary, behind a feature flag: HTTP pull with a
  short-lived token. Internal interface follows Eclipse Data Plane
  Signaling v1.0-RC4, so the data plane can later be split out.
- Report to the Dataspace Protocol project what implementing it shows,
  starting with the three transfer JSON Schemas (suspension, termination,
  error) whose `$ref` to `AbstractTransferCodeMessage` is not a valid JSON
  pointer (`#definitions/...` instead of `#/definitions/...`; tag
  2025-1-err2). Each report links back from the code that works around it.
  Filed 2026-11-16 on the existing issue
  https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/issues/265.
- Report the same way to the Eclipse Data Plane Signaling project what
  implementing v1.0-RC4 shows (listed in ARCHITECTURE, "Data plane"):
  examples that fail the specification's own schemas, a pull `start`
  example that breaks a MUST, who may resume, and the HTTP profile's
  `endpointType` (maintainer, 2026-11-13), plus what the consumer side
  showed: no Signaling message for a new token when the provider starts a
  transfer again, and the "Consumer Suspend/Start" diagram. Report the
  DSP TCK race (ADR 0014) to the TCK project. Filed 2026-11-16:
  https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/133,
  https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/134,
  https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/135,
  https://github.com/eclipse-dataplane-signaling/profiles/issues/8 (`endpointType`),
  https://github.com/eclipse-dataspacetck/dsp-tck/issues/338 (TCK race).
- Code quality and readability review of the whole code base before the
  release, so a maintainer who is not a Rust expert can follow it (asked
  by the maintainer, 2026-11-13): simpler code where it is hard to read,
  comments where an idiom hides the intent, and a short guide to the Rust
  idioms the code uses (closures `|x| ...`, the `?` operator, `match`,
  `Option` and `Result`). Findings are fixed or recorded before tagging.
  Done: `docs/RUST-IDIOMS.md`, and the review's fixes (2026-11-13).
- Exit: DSP TCK transfer tests green; end-to-end file transfer between two
  instances in Docker Compose. The EDC interop test moved to 0.5, so it
  runs once with real identity (DCP) instead of EDC's mock (maintainer,
  2026-11-12).
- Result (2026-11-13): met. The DSP TCK passes the 15 provider and 15
  consumer transfer tests (TP:01 to TP:03, TP_C:01 to TP_C:03; TP:02-04
  and TP_C:02-04 are disabled in the TCK itself), 65 tests in all with
  metadata, catalog and negotiation. `scripts/demo-compose.sh` has two
  connectors in Docker Compose negotiate a contract and fetch the data
  over HTTP pull; CI runs it on every pull request. The upstream reports
  are drafted; the maintainer files them, and their links are added here
  (filed 2026-11-16, linked above and in
  [R-07](RISKS_AND_TECHNICAL_DEBTS.md#r-07)).

## 0.5 DCP: identity and DSP authentication (released 2026-11-13 as v0.5.0)

Plan approved by the maintainer on 2026-11-13. DCP v1.0.1 and the DCP
TCK v1.2.1 are the targets (latest releases, checked 2026-11-13).

- Identity component `portitor-identity` ([ADR 0015](adr/0015-dcp-identity-component.md)):
  did:web document, one P-256 key from a mounted file, Secure Token
  Service (the DCP TCK and EDC need one), presentation API, storage API
  and credential offers (holder side), credentials in SQLite. Profile
  both DCP profiles, `vc11-sl2021/jwt` (what the DCP TCK uses) and
  `vc20-bssl/jwt` (what Gaia-X needs, now in 0.8). Its own signed image.
- The connector authenticates every DSP request with DCP
  ([ADR 0016](adr/0016-dsp-caller-authentication.md)): the caller's DID
  is proven, it must hold a `MembershipCredential` from a trusted issuer,
  not revoked (BitstringStatusList). Every outgoing DSP request carries a
  token from the own identity's STS. `/.well-known/dspace-version` stays
  open. Credential rules in ODRL stay in 0.6.
- Until the issuer exists (0.6), a test-only tool, not shipped in any
  image, signs credentials for the demos and the EDC test.
- **First interop test against EDC**: catalog, negotiation and pull
  transfer, with DCP against EDC's IdentityHub. Target the EDC 1.0
  release candidates and pin 1.0.0 when it ships (fallback 0.18.2). To
  save CI minutes it runs on pull requests that change DSP or DCP code
  and nightly (maintainer, 2026-11-13).
- Spike: how to join a **Simpl-Open** dataspace
  ([ADR 0017](adr/0017-simpl-open.md), proposed). Simpl-Open's connector
  is EDC 0.17.0 with DSP 2025-1 but no DCP; agents authenticate each
  other with GA-issued x509 certificates over mTLS at their Tier 2
  gateway, with attribute-based access. Proposed: no x509/mTLS profile
  in Portitor; later, run Portitor behind the Simpl Tier 2 gateway with
  a small adapter, and ask Simpl whether SSI Tier 2 will use DCP.
- Code quality and readability review before the release, as in 0.4.
- Pull requests: (1) ADRs 0015 and 0016; (2) DCP domain types;
  (3) identity component; (4) storage API and DCP TCK credential service
  tests; (5) connector as verifier, DCP TCK verifier tests, DSP TCK and
  demos with DCP; (6) EDC interop test; (7) Simpl-Open spike;
  (8) readability review and 0.5.0.
- Exit: DCP TCK credential service tests (presentation and holder-side
  issuance) and verifier tests green; DSP flows from 0.2 to 0.4 run with
  DCP authentication; the EDC interop test is green in CI.
- Result (2026-11-13): met. The DCP TCK v1.2.1 passes all 54 credential
  service tests and all 22 verifier tests, locally and in CI. The DSP TCK
  runs green with DCP authentication on every request. The EDC interop
  test (EDC 1.0.0-RC7 with its IdentityHub) runs catalog, negotiation and
  pull transfer with DCP in both directions, checks that each side
  refuses the other once its credential is revoked, and is green in CI. The
  Simpl-Open spike stays at ADR 0017 (proposed) until the maintainer sets
  up a Simpl-Open sandbox (maintainer, 2026-11-13). Upstream reports
  (DCP TCK, EDC IdentityHub, DCP schemas) are still to be filed. They
  were filed on 2026-11-16; the links are in
  [R-07](RISKS_AND_TECHNICAL_DEBTS.md#r-07).

## 0.6 DCP: issuance and credential-based policies (released 2026-11-14 as v0.6.0)

Plan approved by the maintainer on 2026-11-14. The schema and vocabulary
registry moved to 0.7, so each release has one main risk (maintainer,
2026-11-14).

- Issuer component `portitor-issuer` ([ADR 0018](adr/0018-issuer-component.md)):
  optional per participant, one per dataspace, run by the authority.
  DCP issuer metadata, credential request and status, delivery to the
  holder's Storage API, offers; issues only to participants the authority
  registered; credential types from a configuration file; both DCP
  profiles; BitstringStatusList revocation. Its own signed image.
- The identity component asks an issuer for credentials (holder side of
  DCP issuance) and accepts their delivery.
- ODRL constraints that check credentials
  ([ADR 0019](adr/0019-credential-constraints.md)): left operand
  `<CredentialType>.<claim path>`, as EDC does, with scopes derived from
  the policy. Checked at negotiation and again at transfer start.
- The test-only `test-issuer` tool is removed; demos and the EDC
  interop test use `portitor-issuer`.
- Spike (3 days): SHACL engine for ADR 0005 (rudof, a small SHACL Core
  subset on Oxigraph's RDF crates, or a sidecar), on HealthDCAT-AP
  release 7 samples. Ends with ADR 0005 updated for the maintainer's
  decision.
- Reviews before the release: engineering and testing, readability
  (`docs/RUST-IDIOMS.md`), documentation (C4 diagrams, OpenAPI, website,
  DEPLOYING).
- Exit: DCP TCK v1.2.1 issuer tests (41) green against `portitor-issuer`,
  and the credential service, verifier and DSP TCK runs still green; a
  negotiation under a credential constraint succeeds for a holder with
  the claim and is terminated for one without it; a revoked credential
  blocks a new negotiation; EDC IdentityHub gets its membership credential
  from `portitor-issuer` and the EDC interop test stays green.
- Result (2026-11-14): met. The DCP TCK v1.2.1 passes all 41 issuer
  tests against `portitor-issuer` (a new CI job), all 54 credential
  service tests and all 22 verifier tests, run locally. The DSP TCK passes
  its 65 required tests with the connector of this release, run locally.
  `scripts/demo.sh` and the Compose demo pass locally: a negotiation under
  a `DataProcessorCredential` constraint is agreed, one for a missing role
  claim is terminated, and once the issuer revokes the credential a new
  transfer is refused. The EDC interop test (EDC v1.0.0-RC7) passes
  locally, with the IdentityHub asking `portitor-issuer` for its
  credential. Verifiers follow DCP 5.4.3, step 8: a presentation that
  holds a revoked credential is refused, so after a reissue the holder
  deletes the revoked one (DEPLOYING). The SHACL spike is done; the
  maintainer chose an own SHACL Core subset for 0.7 (ADR 0005).

## 0.7 Schema and vocabulary registry, dataset harvest (released 2026-11-15 as v0.7.0)

Plan approved by the maintainer on 2026-11-14.

- Schema and vocabulary registry ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)):
  the authority builds and signs a bundle of SHACL shapes, vocabularies
  and JSON-LD contexts with `portitor-issuer bundle build`, signed with a
  separate bundle key under its `did:web`; the issuer can serve it. The
  connector pulls the newest version (or a fixed one), verifies it,
  caches it on disk and puts a new version in use without a restart.
  How schemas are collected and stored stays open: see
  [TD-01](RISKS_AND_TECHNICAL_DEBTS.md#td-01).
  HealthDCAT-AP release 8 is the baseline.
- Our own SHACL Core engine, `portitor-shacl` (maintainer, 2026-11-14):
  shapes that use anything it does not implement are refused.
- Datasets are harvested into a read copy ([ADR 0020](adr/0020-dataset-harvest.md)):
  the source of truth lives outside the connector; the first source is
  the existing datasets file; harvest at start and every hour
  (`PORTITOR_HARVEST_INTERVAL_SECONDS`); each dataset is checked on its
  own (DSP, ODRL, SHACL for its `dct:accessRights` level); a bad dataset
  is rejected and never stops the connector; every run is stored with
  its errors and a summary, readable at `GET /v1/harvest-jobs`. Policies
  live with the datasets; no API writes datasets or policies.
- The SHACL `MessageGuard` adapter checks DSP messages in and out.
- Datasets that passed carry `dct:conformsTo`. The catalog `@context`
  stays the DSP context alone; metadata terms outside it are full IRIs
  (maintainer, 2026-11-14).
- Reviews before the release: engineering and testing, readability
  (`docs/RUST-IDIOMS.md`), documentation (C4 diagrams, OpenAPI, website,
  DEPLOYING).
- Exit: a dataset that breaks the authority's shapes is not published,
  its error is in the harvest job, and the connector keeps running; a
  dataset removed from the source stops new negotiations while a
  transfer under an existing agreement still works; a DSP message that
  breaks the shapes is refused; a changed or wrongly signed bundle is
  refused; the engine agrees with the EU ITB validator on our release 8
  samples; the DSP TCK, the DCP TCK and the EDC interop test stay green.

- Result (2026-11-14): met, run locally. A HealthDCAT-AP release 8 bundle
  (with the DCAT-AP 3.0.0 shapes and reduced code lists) is built and
  signed with the issuer's bundle key and pinned by the provider; in the
  demo two valid datasets are served with `dct:conformsTo` and one is
  rejected with its 7 SHACL violations in `GET /v1/harvest-jobs`, while
  the connector keeps running. A dataset removed from the source stops new
  negotiations while a transfer under an existing agreement still works
  (tests). A DSP message that breaks the shapes is refused (400 inbound,
  not sent outbound). A changed or wrongly signed bundle, or one signed
  with any key other than the bundle key, is refused. The engine gives
  the same 7 results as the EU ITB validator on the spike samples and
  passes the vendored W3C SHACL tests for its components. The DSP TCK
  passes its 65 required tests, the DCP TCK 41 issuer (also with a bundle
  key), 54 credential service and 22 verifier tests, and the EDC interop
  test (EDC v1.0.0-RC7) passes with the bundle on. The catalog `@context`
  stays the DSP context alone; datasets use full IRIs (maintainer,
  2026-11-14).

## 0.8 Gaia-X Trust Framework (Danube) (released 2026-11-15 as v0.8.0)

Plan approved by the maintainer on 2026-11-15. Gaia-X does not use DCP,
and EDC 1.0.0-RC7 refuses a credential that is not about the holder's
DID, which the Gaia-X compliance credential is not. So Gaia-X policies
are tested between Portitor connectors, and the EDC interop test keeps
running without Gaia-X (maintainer, 2026-11-15).

- Spike (go/no-go): a local Gaia-X compliance service accepts a
  presentation signed with a Portitor ES256 key. Result: go
  ([spike report](spikes/0.8-gaia-x-compliance.md)).
- The identity component publishes the X.509 chain of its key (`x5u`)
  and onboards to Gaia-X: it signs the LegalPerson and terms and
  conditions credentials, sends them with the notary's registration
  number credential to a compliance service, stores the compliance
  credential and renews it ([ADR 0021](adr/0021-gaia-x-onboarding.md)).
- The connector checks a Gaia-X compliance credential from a trusted
  compliance service, tied to the caller through the digest of its
  LegalPerson credential, and policies can require it
  ([ADR 0022](adr/0022-gaia-x-compliance-in-policies.md)).
- CI: a Gaia-X job with a local compliance service (pinned image, Gaia-X
  test mode); the DCP TCK jar is cached (Maven Central answered HTTP 429
  on 2026-11-15).
- Reviews before the release: engineering and testing, readability
  (`docs/RUST-IDIOMS.md`), documentation (C4 diagrams, OpenAPI, website,
  DEPLOYING).
- Exit: a participant set up from scratch gets a
  `ParticipantComplianceCredential` from the local compliance service in
  CI, and a Portitor consumer negotiates with a Portitor provider under a
  policy that requires it; a consumer without it is refused; a tampered
  LegalPerson credential and an untrusted compliance service are refused.
  The DCP TCK (issuer, credential service, verifier), the DSP TCK and the
  EDC interop test stay green.
- Not in 0.8 (see "Later"): the real Gaia-X Lab, Gaia-X DCAT and service
  offering credentials, OpenID4VC, label levels 1 to 3, calling the
  notary, Gaia-X with EDC.

## 0.9 Operations (released 2026-11-15 as v0.9.0)

Plan approved by the maintainer on 2026-11-15. The two-participant demo
in Docker Compose exists since 0.4; 0.9 adds what an operator needs to
run Portitor and what a new user needs to try it
([ADR 0023](adr/0023-observability.md)).

- Spike (go/no-go): OpenTelemetry in the connector within the footprint
  budget (at most +25% binary size and idle memory against 0.8.0, start
  time unchanged), and `cargo deny` still green
  ([spike report](spikes/0.9-opentelemetry.md)).
- Liveness and readiness in the connector, the identity component and
  the issuer, on the management port without the API key, with the paths,
  status codes and body of EDC's Observability API
  (`/api/check/liveness`, `/readiness`, `/startup`, `/health`). Readiness
  checks only the component's own dependencies, never other
  participants. `GET /health` on the public port stays.
- A `health` subcommand in each binary for Docker `HEALTHCHECK`, as the
  images hold no shell; the Compose demo waits for each component to be
  ready, so the file itself shows the deployment order.
- Traces and metrics with OpenTelemetry over OTLP/HTTP, off unless
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set, configured with OpenTelemetry's
  own variables. Trace context stays inside one participant: DSP calls
  to other participants carry no `traceparent`. HTTP metrics follow the
  OpenTelemetry semantic conventions; a short list of Portitor metrics
  covers negotiations, transfers, refused DSP requests, the outbox, the
  harvest and the bundle. Log lines carry the `trace_id`.
- An optional Compose file with an OpenTelemetry Collector, Jaeger and
  Prometheus, started by hand and described in TESTING (not run in CI).
- A quickstart on the website.
- Reviews before the release: engineering and testing, readability
  (`docs/RUST-IDIOMS.md`), documentation (C4 diagrams, OpenAPI, website,
  DEPLOYING, `_http`).
- Exit: readiness fails when a component's own dependency is down
  (tested) and the Compose demo starts in order on it; with an OTLP
  endpoint set, a negotiation and a transfer show as traces and metrics;
  without it nothing is exported and the footprint stays in budget; a new
  user runs the demo from the website in under 15 minutes (timed by the
  maintainer on a clean machine). The DSP TCK, the DCP TCK, the EDC
  interop test and the Gaia-X test stay green.
- Result (2026-11-15): met, except the timed quickstart, which waits for
  the maintainer's run on a clean machine. Spike: go. Readiness fails
  when a component's own dependency is down (unit and integration
  tests), and the Compose demo starts each component once the ones it
  needs are healthy. With the observability stack, Jaeger shows traces
  of all three components with the negotiation and transfer state
  changes, and Prometheus the metrics; without an endpoint nothing is
  exported (in-process OTLP receiver test). Footprint against 0.8.0:
  binaries +14% (connector), +20% (identity, issuer); idle memory +1.2
  MB per process with export off. At saturation the connector serves
  8% fewer requests with export off and 12% to 18% fewer with export on
  (R-37). The DSP TCK (65 of 65), the DCP TCK (issuer 41, credential
  service 54, verifier 22), the EDC interop test (RC7) and the Gaia-X
  test pass.

The hardening release planned as 0.10 is split into four smaller
releases, each with one main risk (maintainer, 2026-11-15). 1.0 follows
0.13.

## 0.10 Supply chain

- Dependency review with **cargo-vet**: every crate version in `Cargo.lock`
  audited (own audits or imported sets, such as Mozilla's and Google's) or
  explicitly exempted; CI fails on an unaudited change. Deferred from 0.1
  (decided 2026-11-11) because it turns each dependency bump into review
  work ([R-04](RISKS_AND_TECHNICAL_DEBTS.md#r-04)).
- Website trust values (maintainer, 2026-11-15): the site shows "not
  available" for the SonarQube Cloud quality gate and for REUSE. Register
  the repository with the REUSE API (open since 0.1), check that the
  quality gate can be read without a token
  (`api/qualitygates/project_status`), and check that the site shows
  both after its next build.
- File the upstream reports drafted from 0.4 to 0.8 and link them
  (maintainer, 2026-11-15; earlier if an answer is needed to go on;
  [R-07](RISKS_AND_TECHNICAL_DEBTS.md#r-07)).
- Exit: cargo-vet runs in CI with every crate audited or exempted; the
  site shows the SonarQube and REUSE values; each upstream report filed
  and linked.
- Main risk: the audit load of cargo-vet.
- Result (2026-11-16): cargo-vet runs in CI: 45 of 204 crate versions
  are audited by imported sets from Mozilla, Google, Bytecode Alliance,
  ISRG and Zcash, and 159 are exempted. The SonarQube quality gate is
  fixed: the site asks for `branch=main`, because SonarQube Cloud still
  names its main branch `master`. The REUSE value is pending: the
  maintainer registers the repository with the REUSE API. Upstream
  reports: 12 filed and linked in
  [R-07](RISKS_AND_TECHNICAL_DEBTS.md#r-07) (11 issues and 1 comment), 1
  not filed (Gaia-X, no GitLab account), 1 on hold (R-10), 1 dropped
  (already filed by a third party).

## 0.11 Security hardening

- Threat model review; fuzzing on every parser (ClusterFuzzLite).
- Explicit request size limits per port and route, set by Portitor
  (today axum's default of 2 MB on every parsed body; data plane bodies
  are streamed), with tests ([R-18](RISKS_AND_TECHNICAL_DEBTS.md#r-18)).
- Cache status list credentials in the connector's DCP verifier
  (maintainer, 2026-11-14): `PORTITOR_STATUS_LIST_CACHE_SECONDS`, default
  300 (5 minutes), 0 checks on every request, at most 3600; only lists
  that were fetched and verified are cached, never longer than the list's
  own expiry. Trade-off documented in DEPLOYING: a revocation takes up to
  that long to be seen. EDC v1.0.0-RC7 caches for 15 minutes by default
  (`edc.iam.credential.revocation.cache.validity`). Demos, the EDC test
  and the TCK runs use 0. Amends ADR 0016 ("Presentations are not
  cached") ([TD-06](RISKS_AND_TECHNICAL_DEBTS.md#td-06)).
- Investigate key custody and key rotation
  ([TD-05](RISKS_AND_TECHNICAL_DEBTS.md#td-05)), and list a retired key
  next to the current one in each DID document, so a key change leaves
  no gap: what the old key signed keeps verifying until it expires.
  EDC IdentityHub rotates this way (maintainer, 2026-11-15, plan after
  0.13). Key custody in OpenBao comes in 1.4.
- The outbox retries a DSP message only on network errors, 408, 429 and
  5xx, and gives up at once on other 4xx
  ([TD-30](RISKS_AND_TECHNICAL_DEBTS.md#td-30); maintainer, 2026-11-15).
- Research, docs only, in 0.11 and 0.12: an ADR on a provider in several
  data spaces ([TD-03](RISKS_AND_TECHNICAL_DEBTS.md#td-03)) and one on
  user to system authentication and authorisation
  ([TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)). Their answers may change
  the API and the configuration, so they must be known before the 0.13
  freeze (maintainer, 2026-11-15).
- Exit: every parser has a fuzz target; the request limits per port and
  route are tested; the status list cache is tested at 0 and 300 s; the
  threat model review is recorded in SECURITY-ASSURANCE; a key change
  with two keys listed leaves no verification gap (tested); the outbox
  stops at the first 4xx it may not retry (tested).
- Main risk: a finding that needs a design change.

## 0.12 Interoperability and performance

- Interoperability tests against Eclipse EDC, without Gaia-X: EDC
  1.0.0-RC7 refuses the Gaia-X compliance credential (ADR 0022). Simpl-Open
  stays under "Later" until a sandbox exists (ADR 0017).
- Benchmarks published.
- Exit: the EDC interop test passes against the newest EDC release;
  the benchmarks are published in `docs/`.
- Main risk: EDC changing under us.

## 0.13 Release candidate for 1.0

- API and config freeze, upgrade notes, documentation review.
- OpenSSF Best Practices **silver**.
- OpenSSF Baseline **level 1** self-assessment, control by control, in
  `docs/SECURITY-ASSURANCE.md` (moved from 0.1, maintainer, 2026-11-12),
  done last so it assesses the final state; decide whether 1.0 aims at a
  higher level ([R-05](RISKS_AND_TECHNICAL_DEBTS.md#r-05)).
- The TD-03 and TD-04 ADRs decided; any API or configuration change
  they need is made before the freeze.
- Before 1.0 (maintainer, 2026-11-15): read the ODRL 2.2 `dateTime`
  definition and compare it with ADR 0009
  ([R-26](RISKS_AND_TECHNICAL_DEBTS.md#r-26)); decide in an ADR whether
  TLS stays the operator's job
  ([TD-19](RISKS_AND_TECHNICAL_DEBTS.md#td-19)); confirm forward-only
  migrations in the upgrade notes
  ([TD-22](RISKS_AND_TECHNICAL_DEBTS.md#td-22)); the formal trademark and
  domain check of the name ([R-25](RISKS_AND_TECHNICAL_DEBTS.md#r-25)),
  which blocks 1.0. A second maintainer
  ([R-02](RISKS_AND_TECHNICAL_DEBTS.md#r-02)) does not block 1.0.
- Usable as it is (maintainer, 2026-11-15: 1.0 ships the release
  candidate only if what is built until 0.13 can be shown and used): a
  new user runs the website quickstart on a clean machine, and the docs
  say what 1.0 does and does not do (for example pull transfers only).
- Exit: Scorecard at least 8/10; no open high findings in Sonar or
  CodeQL; the quickstart runs on a clean machine; R-25 done.
- Main risk: freezing too early or too late.

## After 0.13 (plan approved by the maintainer, 2026-11-15)

1.0 is the 0.13 release candidate, with no new features. After it, the
1.x releases add features without breaking the 1.0 API or configuration
(semantic versioning: a breaking change waits for 2.0). As now, each
minor release is one pull request. Each feature gets an ADR before code,
and each format is taken from a specification, EDC or Simpl-Open, never
invented. Each 1.x release ships when its exit criteria pass, not on a
date.

The health data space demo (see below) grows one step per release, from
1.1 to 1.3. It is a manual test, run with Compose and a runbook, not a
CI job (maintainer, 2026-11-15).

### 1.0 Stable

- The 0.13 release candidate and the "before 1.0" items of 0.13.
- Exit: upgrade notes from 0.9 to 1.0; the website says which APIs and
  settings 1.0 keeps stable.
- Main risk: a breaking change found after the freeze.

### 1.1 Push to object storage

- Provider push to S3-compatible object storage, a second data plane
  behind the Data Plane Signaling port
  ([TD-10](RISKS_AND_TECHNICAL_DEBTS.md#td-10)). The consumer gives the
  bucket and short-lived credentials in the `dataAddress` of its transfer
  request (DSP 2025-1; Signaling v1.0-RC4 covers provider push). Neither
  specification names an S3 endpoint type, so the ADR reuses EDC's or
  Simpl-Open's ([R-38](RISKS_AND_TECHNICAL_DEBTS.md#r-38)). Each transfer
  method is its own distribution and `format` in the catalog
  (maintainer, 2026-11-13).
- Management API: suspend and start a transfer again
  ([TD-11](RISKS_AND_TECHNICAL_DEBTS.md#td-11)).
- Demo step 1: the authority (issuer), Alice and Bob. Bob negotiates with
  Alice, peer to peer, and Alice pushes the data to RustFS in Bob's
  network.
- Exit: a push transfer lands in RustFS; pull still passes the DSP TCK,
  the DCP TCK and the EDC interop test.
- Main risk: no S3 data address shared by EDC and Simpl-Open.

### 1.2 Purpose and data permit in policies

- An ODRL `purpose` constraint (a left operand of the ODRL 2.2
  vocabulary), with purposes named by Data Privacy Vocabulary (DPV)
  IRIs, the vocabulary HealthDCAT-AP uses for `dpv:hasPurpose`. Checked
  at negotiation and transfer, fail closed
  ([TD-12](RISKS_AND_TECHNICAL_DEBTS.md#td-12)). No ICUdata4EU or LNDS
  list of purposes exists to reuse (maintainer, 2026-11-15;
  [R-40](RISKS_AND_TECHNICAL_DEBTS.md#r-40)).
- A data permit as a verifiable credential that policies can require.
- As consumer, accept a provider's offer with credential constraints
  ([TD-11](RISKS_AND_TECHNICAL_DEBTS.md#td-11)); the catalog hides offers
  the caller cannot meet ([TD-13](RISKS_AND_TECHNICAL_DEBTS.md#td-13)).
- Demo step 2: Bob's agreement with Alice needs a matching purpose; a
  wrong purpose is refused.
- Exit: purpose and permit tested granted and refused; the TCKs and the
  EDC interop test stay green.
- Main risk: DPV purposes may not map one to one to the purposes of the
  EHDS (Article 53).

### 1.3 Data user and HDAB: Cooper and Marley

- Cooper plays the health data access body (HDAB) towards Marley, the
  data user; Bob and Alice stay a peer to peer agreement (maintainer,
  2026-11-15). Marley needs a data permit, issued by Cooper as the HDAB
  (EHDS Art. 68), so Cooper runs his own issuer next to the authority's.
- No secure processing environment is built: Cooper runs a fixed
  pipeline on Alice's data that produces counts, and pushes that output
  to RustFS in Marley's infrastructure (maintainer, 2026-11-15).
- Agreements that depend on an upstream agreement: Cooper's offer to
  Marley cannot be wider than Cooper's agreement with Alice (purpose,
  users, duration). No specification defines this; research and ADR
  first ([R-39](RISKS_AND_TECHNICAL_DEBTS.md#r-39)).
- Demo step 3: the four questions (below) answered by running the demo,
  each with its evidence, and a note of where Portitor fell short.
- Exit: the four answers written in the docs.
- Main risk: our design for dependent agreements may not match what EDC
  or Simpl-Open do later.

### 1.4 Keys and credentials

- A signer port with an OpenBao transit adapter (P-256 signing, versioned
  keys; the private key never leaves OpenBao), optional
  ([TD-05](RISKS_AND_TECHNICAL_DEBTS.md#td-05)).
- The identity reads status lists and stops presenting a revoked
  credential ([TD-20](RISKS_AND_TECHNICAL_DEBTS.md#td-20)).
- Credential schemas, presentation definitions and issuance policies, if
  a target data space needs them
  ([TD-14](RISKS_AND_TECHNICAL_DEBTS.md#td-14)); STS token and
  presentation caching only if the 0.11 measurements show the need
  ([TD-07](RISKS_AND_TECHNICAL_DEBTS.md#td-07)).
- Exit: a key change in OpenBao with no verification gap (tested); a
  revoked credential leaves the presentations by itself.
- Main risk: OpenBao must stay optional.

### 1.5 Users and the data plane

- Code from the TD-02 and TD-04 ADRs: how a person or an application of
  a participant proves who it is, and whether the data plane checks
  identity or credentials on each request
  ([TD-02](RISKS_AND_TECHNICAL_DEBTS.md#td-02),
  [TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)).
- Consent enforcement and agreement revision
  ([TD-15](RISKS_AND_TECHNICAL_DEBTS.md#td-15)), if the consent ADR finds
  a standard to follow (ISO/IEC TS 27560 and DPV to read first).
- Exit: set by the ADRs.
- Main risk: the largest unknown of the plan; the ADRs may split it.

### Research before code

Docs only: a standards check, then an ADR the maintainer accepts or
rejects. It runs beside the releases.

| Topic | Needed by |
| --- | --- |
| Several data spaces ([TD-03](RISKS_AND_TECHNICAL_DEBTS.md#td-03)) | 0.13 freeze |
| Users and roles ([TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)) | 0.13 freeze; code in 1.5 |
| Who calls the data plane ([TD-02](RISKS_AND_TECHNICAL_DEBTS.md#td-02)) | 1.5 |
| S3 data address | 1.1 |
| Agreements that depend on an upstream agreement | 1.3 |
| Consent | 1.5 |
| How schemas are published ([TD-01](RISKS_AND_TECHNICAL_DEBTS.md#td-01)) | with TD-03 |

### The health data space demo

Four participants and an authority (maintainer, 2026-11-15), modelled on
a health data access body (HDAB) and the ICUdata4EU ICU data space. The
authority governs the data space and runs the issuer; it plays the
national contact point for secondary use (EHDS Art. 75, which the
coordinator HDAB of Art. 55(1) may also be). Alice, a data
provider, holds patient records. Bob, a data user, asks Alice directly,
peer to peer (the EHDS does not exclude it); Alice pushes the data to
object storage (RustFS) in Bob's own infrastructure. Cooper plays the
HDAB towards Marley, another data user: Cooper issues Marley's data
permit, runs a fixed pipeline on Alice's data that produces counts, and
pushes that output to RustFS in Marley's infrastructure. No secure processing
environment (SPE) is built; the pipeline stands in for one (maintainer,
2026-11-15). The demo must answer, by running it:

- can the same data be reached through several channels (Alice's offer
  and Cooper's)?
- does Cooper negotiate with Alice for every new cohort query, or does
  one agreement cover them?
- does Cooper negotiate with Alice for every new access through his
  pipeline, or for every new data user?
- do Alice's and Cooper's policies differ, and can Cooper's be less
  strict because his environment is safer? What can never be wider than
  what Alice allowed Cooper (purpose, users, duration)?

Checked against the EHDS (Regulation (EU) 2025/327, 2026-11-15): for
secondary use a data user applies to the HDAB, which issues a data
permit (Art. 67, 68); pseudonymised data is processed only in an SPE,
and only non-personal data can be downloaded (Art. 73); most of it
applies from March 2029.

## Later (not planned for a release yet)

- Join a Simpl-Open data space behind the participant's Simpl Tier 2
  gateway, with a small login adapter ([ADR 0017](adr/0017-simpl-open.md)).
  Parked until the maintainer sets up a Simpl-Open sandbox to test
  against (maintainer, 2026-11-13).
- Revise agreements when the scope of the data changes, for example a
  data subject objects to a use, or the consumer asks to change the
  scope of the first agreement (maintainer, 2026-11-14). Until then an
  agreement keeps working after its dataset leaves the source. Planned
  with consent in 1.5, if the consent ADR finds a standard.
- More harvest sources for datasets ([ADR 0020](adr/0020-dataset-harvest.md)),
  such as a DCAT-AP endpoint or CKAN; which catalogues to support is
  decided later (maintainer, 2026-11-14).
- A dataspace-wide registry or federated catalogue, run by the
  authority, only if needed (maintainer, 2026-11-14): it would pull the
  participants' DSP catalogs, check them with the same bundle and offer
  search (SPARQL), with its own ADR.
- Check other participants' catalogs against the bundle (warnings only,
  ADR 0005); bundle expiry and bundle key rotation.
- Gaia-X beyond 0.8 (maintainer, 2026-11-15): a test against the real
  Gaia-X Lab or a production clearing house, once Portitor runs beyond a
  laptop (needs a public `did:web` and a CA certificate); Gaia-X-compatible
  DCAT descriptions and service offering credentials, with a federated
  catalogue; OpenID4VCI and OpenID4VP to the Gaia-X Compliance Dispatcher
  when its wallet flow ships; label levels 1 to 3; calling the
  registration number notary; trust anchors and compliance issuers read
  from the Gaia-X Registry; asking EDC how it would accept Gaia-X
  compliance credentials.
- Explore Simpl-Open features that are not part of DSP, DCP, Data Plane
  Signaling or EDC, but add security or complete what a data space
  participant can do (maintainer, 2026-11-15). First candidates:
  - mutual TLS between participants: Simpl-Open's Tier 2 gateway asks
    every caller for an x509 client certificate from the governance
    authority's CA and binds the caller's identity attributes to it
    ([ADR 0017](adr/0017-simpl-open.md));
  - consent enforcement: honouring a data subject's consent (and its
    withdrawal) when data is shared (researched for 1.5, above);
  - a configurable API gateway in front of the participant's services,
    like Simpl-Open's Tier 1 and Tier 2 gateways (attribute-based access).

  Most of the work sits in the data plane, but not all of it: mutual
  TLS protects the link for the DSP messages (control plane) as well as
  the transfer; for consent, the rule belongs to the policy (control
  plane) but the check runs in the data plane on every request, because
  consent can be withdrawn after the agreement; a gateway sits mainly in
  front of the data source, but could also front the management API.
  So: data plane first, not data plane only.

  For each: what it does in Simpl-Open, whether a standard defines it,
  which plane it touches, and whether Portitor should offer it as its
  own component, an adapter or a deployment guide. The result is an ADR
  before any code; consent comes first, as it is not researched yet.

## Technical debt

Known open questions and shortcuts, kept with the risks in
[RISKS_AND_TECHNICAL_DEBTS.md](RISKS_AND_TECHNICAL_DEBTS.md), the single
list: what each is, what is in place now and what comes next. The open
questions the maintainer asked to investigate:

- [How schemas and vocabularies are collected and stored](RISKS_AND_TECHNICAL_DEBTS.md#td-01) (TD-01)
- [Who calls the data plane, and with which credentials](RISKS_AND_TECHNICAL_DEBTS.md#td-02) (TD-02)
- [A provider in several data spaces, each with its own rules](RISKS_AND_TECHNICAL_DEBTS.md#td-03) (TD-03)
- [User to system authentication and authorisation, governed per data space](RISKS_AND_TECHNICAL_DEBTS.md#td-04) (TD-04)
- [Key custody and key rotation](RISKS_AND_TECHNICAL_DEBTS.md#td-05) (TD-05)

The other technical debts (TD-06 onwards) and the risks (R-01 onwards)
are listed there too, with their status and target release.
