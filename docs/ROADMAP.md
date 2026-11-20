<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Roadmap (draft)

Status: **draft for review**. Releases are small and frequent. Each
release is usable on its own and ends with a tag, signed images and a
changelog entry. Version numbers are shared by every component (mono
repo). Each planned item lists its exit criteria, so "done" is
checkable.

## Released

What each release delivered, in one line. The details are in the
[changelog](../CHANGELOG.md) and the [ADRs](adr/README.md); the risks
and debts each release raised or closed are in
[RISKS_AND_TECHNICAL_DEBTS](RISKS_AND_TECHNICAL_DEBTS.md).

| Release | Date | Tag | What it delivered | Changelog |
|---|---|---|---|---|
| 0.0 | 2026-11-11 | none | Spike: Rust chosen after a 3-day bake-off against Quarkus native ([ADR 0002](adr/0002-technology-stack.md)); then the component split ([ADR 0003](adr/0003-component-split.md)) | none |
| 0.1 | 2026-11-12 | v0.1.0 | Repository foundations: licence and REUSE, community files, CI and security scans, signed multi-arch image with SBOM and provenance, the website, OpenSSF Best Practices registered (project 16578) | [0.1.0](../CHANGELOG.md#010---2026-11-12) |
| 0.2 | 2026-11-12 | v0.2.0 | DSP version discovery and catalog (provider), schema-checked messages, DSP TCK in CI, fuzzing; Best Practices **passing**; Scorecard from 6.7 to 7.9 | [0.2.0](../CHANGELOG.md#020---2026-11-12) |
| 0.3 | 2026-11-12 | v0.3.0 | Contract negotiation as provider and consumer in SQLite (ADR 0006), outbox with proof of delivery (ADR 0007), minimal ODRL (ADR 0009), management API; renamed to Portitor (ADR 0008) | [0.3.0](../CHANGELOG.md#030---2026-11-12) |
| 0.4 | 2026-11-13 | v0.4.0 | Transfer process and the built-in HTTP pull data plane behind Data Plane Signaling v1.0-RC4 (ADRs 0013, 0014); Compose demo; RUST-IDIOMS | [0.4.0](../CHANGELOG.md#040---2026-11-13) |
| 0.5 | 2026-11-13 | v0.5.0 | `portitor-identity` (ADR 0015); every DSP request authenticated with DCP (ADR 0016); DCP TCK; first EDC interop test; Simpl-Open spike (ADR 0017, proposed) | [0.5.0](../CHANGELOG.md#050---2026-11-13) |
| 0.6 | 2026-11-14 | v0.6.0 | `portitor-issuer` (ADR 0018); the identity asks an issuer for credentials; credential constraints in policies (ADR 0019); SHACL engine spike | [0.6.0](../CHANGELOG.md#060---2026-11-14) |
| 0.7 | 2026-11-15 | v0.7.0 | Signed schema and vocabulary bundle and our own SHACL Core engine (ADR 0005); datasets harvested into a read copy (ADR 0020); DSP messages checked against shapes | [0.7.0](../CHANGELOG.md#070---2026-11-15) |
| 0.8 | 2026-11-15 | v0.8.0 | Gaia-X: onboarding in the identity (ADR 0021) and compliance credentials in policies (ADR 0022); a Gaia-X job in CI with a local compliance service | [0.8.0](../CHANGELOG.md#080---2026-11-15) |
| 0.9 | 2026-11-15 | v0.9.0 | Operations: liveness and readiness, a `health` subcommand, traces and metrics over OTLP (ADR 0023), the website quickstart | [0.9.0](../CHANGELOG.md#090---2026-11-15) |
| 0.10 | 2026-11-16 | v0.10.0 | Supply chain: cargo-vet in CI, website trust values, upstream reports filed | [0.10.0](../CHANGELOG.md#0100---2026-11-16) |
| 0.11 | 2026-11-17 | v0.11.0 | Security hardening: a fuzz target per parser, request size limits, status list cache, retired keys for rotation, outbox retry rules, threat model review, ADR 0024 proposed | [0.11.0](../CHANGELOG.md#0110---2026-11-17) |
| 0.12 | 2026-11-20 | v0.12.0 | Interoperability and performance: strict policy reading, DSP and DCP fixes checked against EDC, benchmarks published, a panic fails one request, ADR 0025 proposed | [0.12.0](../CHANGELOG.md#0120---2026-11-19) |
| 0.13 | 2026-11-20 | v0.13.0 | API and configuration freeze ([STABILITY](STABILITY.md)); management call that reads a catalog; EDC test on 1.0.0-RC8 with Gaia-X; ADRs 0024, 0025 and 0026 accepted | [0.13.0](../CHANGELOG.md#0130---2026-11-20) |

Older documents cite sections of this file that the table replaced, for
example "ROADMAP 0.11, Decisions on the 0.11 plan". Their full text is in
the Git history of this file, last at commit `0637b72` (2026-11-20).

### Decisions recorded here

Decisions about the plan itself that no ADR or changelog entry holds.

- **One main risk per release** (maintainer, 2026-11-14). For this
  reason the schema registry moved from 0.6 to 0.7 (2026-11-14), the
  hardening release planned as 0.10 was split into 0.10 to 0.13
  (2026-11-15), and the release candidate was split into 0.13 (code
  freeze) and 0.14 (documentation and assurance) (2026-11-20). A
  document that names "0.9 hardening" or "0.10" for key rotation or the
  Baseline self-assessment was written before these moves.
- **Repository settings** (0.1, set by the owner in GitHub). A ruleset
  on `main`: changes only through pull requests; one approval from a
  code owner, also of the latest push; stale approvals dismissed; the
  branch up to date before merge; signed commits; required status
  checks, by name `CI gate`, `TCK gate` and `Dependencies gate` (from
  0.14; [CONTRIBUTING](../CONTRIBUTING.md#required-checks)); no force
  push or deletion. Admin bypass only for merging a
  pull request, because a sole maintainer cannot approve their own
  ([R-02](RISKS_AND_TECHNICAL_DEBTS.md#r-02)). Private vulnerability
  reporting, secret scanning with push protection, Dependabot (weekly;
  tooling updates grouped, runtime dependencies separate), CodeQL. A
  `release` environment with its own secrets; releases start from a
  signed, annotated `v*` tag.
- **The EDC interop test** moved from 0.4 to 0.5, so it first ran with
  real identity (DCP) and not EDC's mock (maintainer, 2026-11-12). To
  save CI minutes it runs on pull requests that change the code and
  nightly (maintainer, 2026-11-13; [TESTING](TESTING.md#what-runs-where)).
- **Upstream reports** are part of the work: what implementing a
  specification shows is reported to its project, and the code that
  works around it links the report (maintainer, 2026-11-13). The list
  is in [RISKS_AND_TECHNICAL_DEBTS](RISKS_AND_TECHNICAL_DEBTS.md#upstream-reports).
- **Reviews before each release** since 0.4: engineering and testing,
  readability, documentation ([RELEASING](RELEASING.md#documentation-review)).

### Still open from released versions

- 0.9: the quickstart timed by the maintainer on a clean machine
  (target: under 15 minutes). Now an exit criterion of 0.14: one timed
  run, no new CI job.
- 0.10 and 0.11: the README badges for the SonarQube Cloud quality gate
  and coverage. SonarQube Cloud still names its main branch `master`
  and analyses `main` as a short-lived branch, so the badges read "not
  computed". They wait until the maintainer renames the branch in
  SonarQube Cloud. The website already asks for `branch=main`.

## 0.14 Documentation and assurance (release candidate for 1.0)

- Documentation simplification (maintainer, 2026-11-20): each fact has
  one home and the other files link to it. The README loses its route
  and settings tables (the OpenAPI files and DEPLOYING hold them);
  DEPLOYING is in deployment order (issuer, identity, connector) with a
  short "first deployment" path at the top
  ([TD-39](RISKS_AND_TECHNICAL_DEBTS.md#td-39)); the released sections
  of this roadmap become one table; closed risks become one line each,
  with a "top risks" box; ARCHITECTURE describes the system as it is,
  not release by release; the ADRs get an [index](adr/README.md).
  What an operator does after the first day (health, logs, metrics,
  backup, key rotation, upgrades) is its own guide,
  [OPERATING](OPERATING.md).
  DEPLOYING is for an operations team; running the demo on a laptop is
  in TESTING (maintainer, 2026-11-18). ADRs, the changelog and the
  spikes are records and are not shortened.
- A "read the code in this order" page,
  [READING-THE-CODE](READING-THE-CODE.md)
  ([R-01](RISKS_AND_TECHNICAL_DEBTS.md#r-01)), and a note in TESTING for
  the four near-copies of the negotiation and transfer logic
  ([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)).
- One place that says which specification versions Portitor was checked
  against, the Gaia-X Compliance Document and ICAM version among them:
  [ARCHITECTURE, "Specification versions"](ARCHITECTURE.md#specification-versions)
  ([TD-43](RISKS_AND_TECHNICAL_DEBTS.md#td-43)).
- The EDC Issuer Service in the EDC interop test (DCP issuance). Until
  now the test had one issuer, `portitor-issuer`. Its new step 8 adds
  the Issuer Service of EDC IdentityHub 1.0.0-RC8 as a second issuer:
  `portitor-identity` gets a credential from it over DCP; the Portitor
  connector verifies that credential and reads its Bitstring Status
  List; EDC accepts the credential when Portitor's identity presents
  it; after the Issuer Service revokes a credential, its holder is
  refused. The revocation by `portitor-issuer` is now step 9
  ([TESTING](TESTING.md#a-second-issuer-the-edc-issuer-service)). No
  change to Portitor was needed. What the test showed in EDC is
  [R-46](RISKS_AND_TECHNICAL_DEBTS.md#r-46) and
  [R-47](RISKS_AND_TECHNICAL_DEBTS.md#r-47).
- A review of the CI pipeline (maintainer, 2026-11-20), and its
  changes: a pull request that changes only prose no longer builds or
  runs the tests, the TCKs and the dependency checks
  (`scripts/ci_changes.sh`); each of `CI`, `TCK` and `Dependencies`
  ends with one gate job, and only gate jobs are required checks
  ([CONTRIBUTING](../CONTRIBUTING.md#required-checks)); the path
  filters of the EDC, Gaia-X and fuzzing workflows name every file
  those tests read; the release dry run runs on the release pull
  request; a release tag must be on a commit of `main`
  ([RELEASING](RELEASING.md#releasing-a-version)); a CodeQL finding
  fails the CodeQL job.
- The four leftovers of the 0.13 release review: an item of a list
  setting may be at most 4096 bytes; the management call that reads a
  catalog answers 502 when the catalog has no `participantId` or one
  that differs from `counterPartyId`; a finalization event that a
  transfer request settled is no longer counted in
  `portitor.outbox.message.given_up`
  ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), note of 0.14);
  the code of the catalog calls is split into smaller modules, with no
  change in behaviour. What an operator must know:
  [OPERATING, "From 0.13 to 0.14"](OPERATING.md#from-013-to-014).
- The website gets an ancient Roman theme built on the two meanings of
  *portitor*, the customs officer and the ferryman (maintainer,
  2026-11-20). An open-licence font may be stored in the repository,
  with credit.
- OpenSSF Best Practices **silver**. Its access continuity criterion is
  covered by the successor set on the maintainer's GitHub account
  (maintainer, 2026-11-20).
- OpenSSF Baseline **level 1** self-assessment, control by control, in
  `docs/SECURITY-ASSURANCE.md`, done last so it assesses the final
  state. The target stays level 1 for 1.0 (maintainer, 2026-11-20;
  [R-05](RISKS_AND_TECHNICAL_DEBTS.md#r-05)).
- Usable as it is (maintainer, 2026-11-15: 1.0 ships the release
  candidate only if what is built can be shown and used): a new user
  runs the website quickstart on a clean machine, and the docs say what
  1.0 does and does not do. The clean-machine run is one timed run by
  the maintainer ([RELEASING](RELEASING.md#releasing-a-version), step
  4). It gets no CI job of its own: `CI` already runs the Compose demo
  (`scripts/demo-compose.sh`) that the quickstart uses.
- Before 1.0: the formal trademark and domain check of the name
  ([R-25](RISKS_AND_TECHNICAL_DEBTS.md#r-25)), which blocks 1.0. A
  second maintainer ([R-02](RISKS_AND_TECHNICAL_DEBTS.md#r-02)) does not
  block 1.0.
- Exit: Scorecard at least 8/10; the quickstart runs on a clean machine;
  the EDC interop run with step 8 passes in CI; no open CodeQL alert;
  TD-39 and TD-43 addressed; R-25 done before 1.0.
- Main risk: a fact lost while cutting; each removed paragraph names
  where its facts still live.

## After 0.14 (plan approved by the maintainer, 2026-11-15)

1.0 is the 0.14 release candidate, with no new features. After it, the
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

- The 0.14 release candidate and the "before 1.0" items of 0.14.
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
- An audit log line for each data pull: flow, agreement, consumer DID
  and bytes sent. It needs no user identity
  ([ADR 0025](adr/0025-users-and-roles.md), maintainer, 2026-11-20).
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
- User to system access is tested against Simpl-Open, which has
  implemented user-to-system actions (maintainer, 2026-11-20).
- Exit: set by the ADRs.
- Main risk: the largest unknown of the plan; the ADRs may split it.

### Code quality in 1.x

From the [2026-11-18 review](reviews/2026-11-18-specifications-and-code-quality.md)
(maintainer, 2026-11-18: all of them, in 1.x). They change no behaviour,
API or setting. Each goes into the plan of a 1.x release, at most one
large item per release, so a regression is easy to trace. Which release
takes which item is decided when that release is planned; the first
item is best done before 1.1 adds push to the transfer code, so the new
code is written once.

- One shared core for the negotiation and transfer logic, as provider
  and as consumer, in place of the four near-copies; the in-memory
  stores move out of the production code into the tests
  ([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)).
- No source file above 1,000 lines, tests not counted: `auth/dcp.rs`,
  the issuer's and the identity's `store.rs`, `dcp/bundle.rs` and
  `runtime/http.rs` are split by topic ([R-01](RISKS_AND_TECHNICAL_DEBTS.md#r-01)).
- Simpler types: fewer type parameters on the services (today
  `ProviderService<S, D, C, E>`), and each macro that writes types
  either replaced by plain code or explained with its output in
  RUST-IDIOMS ([R-01](RISKS_AND_TECHNICAL_DEBTS.md#r-01)).
- `Negotiator` gets a name that fits what it does (negotiations,
  transfers and the harvest), or is split ([R-01](RISKS_AND_TECHNICAL_DEBTS.md#r-01)).
- A Helm chart for the three images, next to the Compose example
  ([TD-39](RISKS_AND_TECHNICAL_DEBTS.md#td-39)).
- Exit, for each item: the unit tests, the DSP and DCP TCKs and the EDC
  interop test pass unchanged; the C4 diagrams, RUST-IDIOMS and the
  "read the code in this order" page name the new files and types. For
  the Helm chart: the demo data space runs from it on a local cluster,
  and DEPLOYING explains it.
- Main risk: a refactoring that changes behaviour unseen. The TCKs and
  the interop test are the guard, so no item starts while one of them
  is red.

### Research before code

Docs only: a standards check, then an ADR the maintainer accepts or
rejects. It runs beside the releases.

| Topic | Needed by |
| --- | --- |
| Several data spaces ([TD-03](RISKS_AND_TECHNICAL_DEBTS.md#td-03)) | Accepted in 0.13 ([ADR 0024](adr/0024-several-data-spaces.md)) |
| Users and roles ([TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)) | Accepted in 0.13 ([ADR 0025](adr/0025-users-and-roles.md)); code in 1.5 |
| Who calls the data plane ([TD-02](RISKS_AND_TECHNICAL_DEBTS.md#td-02)) | 1.5 |
| S3 data address | 1.1 |
| Agreements that depend on an upstream agreement | 1.3 |
| Consent | 1.5 |
| How schemas are published ([TD-01](RISKS_AND_TECHNICAL_DEBTS.md#td-01)) | with TD-03 |
| A data space's policy rules as SHACL shapes in the bundle (maintainer, 2026-11-18; [TD-32](RISKS_AND_TECHNICAL_DEBTS.md#td-32)) | 1.2 |

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
  ADR 0005); bundle expiry and bundle key rotation (two bundle keys
  listed need another key id than the fixed `<DID>#bundle-key`;
  maintainer, 2026-11-17).
- Gaia-X beyond 0.8 (maintainer, 2026-11-15): a test against the real
  Gaia-X Lab or a production clearing house, once Portitor runs beyond a
  laptop (needs a public `did:web` and a CA certificate); Gaia-X-compatible
  DCAT descriptions and service offering credentials, with a federated
  catalogue; OpenID4VCI and OpenID4VP to the Gaia-X Compliance Dispatcher
  when its wallet flow ships; label levels 1 to 3; calling the
  registration number notary; trust anchors and compliance issuers read
  from the Gaia-X Registry; asking EDC how it would accept Gaia-X
  compliance credentials; a legal person credential signed by another
  signer the participant chooses ([TD-21](RISKS_AND_TECHNICAL_DEBTS.md#td-21), 2026-11-18 review).
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
