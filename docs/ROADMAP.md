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
| 0.12 | 2026-11-20 | v0.12.0 | Interoperability and performance: strict policy reading, DSP and DCP fixes checked against EDC, benchmarks published, a panic fails one request, ADR 0025 proposed | [0.12.0](../CHANGELOG.md#0120---2026-11-20) |
| 0.13 | 2026-11-20 | v0.13.0 | API and configuration freeze ([STABILITY](STABILITY.md)); management call that reads a catalog; EDC test on 1.0.0-RC8 with Gaia-X; ADRs 0024, 0025 and 0026 accepted | [0.13.0](../CHANGELOG.md#0130---2026-11-20) |
| 0.14 | 2026-11-21 | v0.14.0 | Release candidate for 1.0: shorter documentation with [OPERATING](OPERATING.md); Roman website; EDC Issuer Service in the EDC test; CI scope and gates; CodeQL gate; OpenSSF Baseline level 1 self-assessment | [0.14.0](../CHANGELOG.md#0140---2026-11-21) |
| 1.0 | 2026-11-24 | v1.0.0 | Stable: the 0.14 release candidate, tried as v1.0.0-rc.1 on 2026-11-22; routes and settings kept until 2.0 ([STABILITY](STABILITY.md)); the demos as self-contained scenarios, with two health scenarios | [1.0.0](../CHANGELOG.md#100---2026-11-24) |
| 1.1 | 2026-11-25 | v1.1.0 | Push transfers by the Data Plane Signaling HTTP profile into the consumer's object storage; `s3://` dataset sources; the provider can always end access (provider routes, membership checked on every request, running downloads cut); suspend and start as consumer; audit line per transfer; ADR 0027 | [1.1.0](../CHANGELOG.md#110---2026-11-25) |

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
  on `main`: changes only through pull requests; one approval, of the latest
  push, dismissed when new commits are pushed, not tied to a code owner
  (as read from the ruleset on 2026-11-21); the
  branch up to date before merge; signed commits; required status
  checks, by name `CI gate`, `TCK gate`, `Dependencies gate`,
  `Analyze (rust)` and `Analyze (actions)` (from 0.14; [CONTRIBUTING](../CONTRIBUTING.md#required-checks)); no force
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

## 0.14 Documentation and assurance (release candidate for 1.0; released 2026-11-21 as v0.14.0)

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
  (`scenarios/two-party/run.sh`) that the quickstart uses.
- Before 1.0: the trademark and domain check of the name
  ([R-25](RISKS_AND_TECHNICAL_DEBTS.md#r-25)). Done on 2026-11-21: no
  trademark conflict was found, and the maintainer accepted the
  remaining risk. A second maintainer ([R-02](RISKS_AND_TECHNICAL_DEBTS.md#r-02)) does not
  block 1.0.
- Exit: Scorecard at least 8/10; the quickstart runs on a clean machine;
  the EDC interop run with step 8 passes in CI; no open CodeQL alert;
  TD-39 and TD-43 addressed; R-25 done before 1.0 (done, accepted).
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

Together, 1.0 to 1.9 are what the project intends to do in the next
twelve months, to the end of 2027 (maintainer, 2026-11-21): 1.0 to 1.5
below, then the EHDS participant functions and the Simpl-Open tests of
"2027" (1.6 to 1.9). What it does not intend to do in that time is in
"2027", "What Portitor does not plan to do in this year", in "Later" and
in [STABILITY, "What 1.0 does not do"](STABILITY.md#what-10-does-not-do).

The health dataspace demo (see below) grows one step per release, from
1.1 to 1.3. It is a manual test, run with Compose and a runbook, not a
CI job (maintainer, 2026-11-15).

### 1.0 Stable (released 2026-11-24 as v1.0.0)

- The 0.14 release candidate and the "before 1.0" items of 0.14.
- Exit: upgrade notes from 0.9 to 1.0; the website says which APIs and
  settings 1.0 keeps stable.
- Main risk: a breaking change found after the freeze.
- Shipped as 1.0.0-rc.1 (2026-11-22), then 1.0.0 with no API or
  configuration change. Before the tag, all six scenarios passed on
  Linux with the candidate and `cosign verify` passed on its images
  (2026-11-24).

### 1.1 Push to object storage (released 2026-11-25 as v1.1.0)

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
- A transfer whose provider was revoked does not stay `REQUESTED`
  forever ([TD-47](RISKS_AND_TECHNICAL_DEBTS.md#td-47); found by the
  hdab-intermediary scenario, maintainer, 2026-11-24).
- An audit log line for each data pull: flow, agreement, consumer DID
  and bytes sent. It needs no user identity
  ([ADR 0025](adr/0025-users-and-roles.md), maintainer, 2026-11-20).
- Demo step 1: the authority (issuer), Lux Biobank and Lux Biomedicine
  Lab. The lab negotiates with Lux Biobank, peer to peer, and Lux Biobank
  pushes the data to RustFS in the lab's network.
- Exit: a push transfer lands in RustFS; pull still passes the DSP TCK,
  the DCP TCK and the EDC interop test.
- Main risk: no S3 data address shared by EDC and Simpl-Open.
- Shipped in 1.1.0:
  - Push, not as planned above: no S3 address crosses between
    participants. The provider pushes over HTTP, as the Data Plane
    Signaling HTTP Transfer Profile says (`HttpData-PUSH`), and the
    consumer's connector streams the data into its own bucket
    (`PORTITOR_PUSH_BUCKET`); a dataset may also live in the provider's
    object storage (`s3://`), still pulled over `/data`
    ([ADR 0027](adr/0027-push-transfers-and-object-storage.md);
    [R-38](RISKS_AND_TECHNICAL_DEBTS.md#r-38) closed,
    [TD-10](RISKS_AND_TECHNICAL_DEBTS.md#td-10) mitigated). The
    two-party scenario pushes into RustFS, and EDC v1.0.0-RC8 asks
    Portitor to push in the EDC interop test, both in CI.
  - Suspend and start again as consumer
    ([TD-11](RISKS_AND_TECHNICAL_DEBTS.md#td-11)).
  - A termination from a revoked partner is accepted
    ([TD-47](RISKS_AND_TECHNICAL_DEBTS.md#td-47)); the
    hdab-intermediary scenario is not run on 1.1 yet.
  - The audit line `data transfer done`, for each pull, push sent and
    push received ([TD-31](RISKS_AND_TECHNICAL_DEBTS.md#td-31), first
    half).
  - Added by the maintainer on 2026-11-24 (ADR 0027, section 7): the
    provider can always end access. Management routes for the
    transfers it runs (`/v1/provided-transfers`), the consumer's
    membership checked again on every `/data` request and while a
    download runs, and running downloads cut on suspend and terminate.
  - The shared process core of "Code quality in 1.x", done before push
    as planned ([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)).
  - Demo step 1: in `scenarios/hdab-intermediary`, Lux Biobank pushes
    the genotypes into the lab's own RustFS bucket. Written; to be run
    by hand with 1.1 images.

### 1.2 Purpose and data permit in policies

- An ODRL `purpose` constraint (a left operand of the ODRL 2.2
  vocabulary), with purposes named by Data Privacy Vocabulary (DPV)
  IRIs, the vocabulary HealthDCAT-AP uses for `dpv:hasPurpose`. Checked
  at negotiation and transfer, fail closed
  ([TD-12](RISKS_AND_TECHNICAL_DEBTS.md#td-12)). No ICUdata4EU or LNDS
  list of purposes exists to reuse (maintainer, 2026-11-15;
  [R-40](RISKS_AND_TECHNICAL_DEBTS.md#r-40)).
- A data permit as a verifiable credential that policies can require.
  The data-permits scenario already runs one on 1.0, as a credential
  type in the HDAB issuer's configuration, with placeholder claims. What
  it found is fixed here, because the permit brings a second issuer and
  several permits per data user (maintainer, 2026-11-24):
  - trusted issuers by credential type
    ([R-48](RISKS_AND_TECHNICAL_DEBTS.md#r-48));
  - claims per credential at the issuer, not per participant
    ([TD-46](RISKS_AND_TECHNICAL_DEBTS.md#td-46));
  - the identity reads status lists and stops presenting a revoked
    credential, so one revoked permit no longer blocks the others
    ([TD-20](RISKS_AND_TECHNICAL_DEBTS.md#td-20), moved from 1.4).
- As consumer, accept a provider's offer with credential constraints
  ([TD-11](RISKS_AND_TECHNICAL_DEBTS.md#td-11)); the catalog hides offers
  the caller cannot meet ([TD-13](RISKS_AND_TECHNICAL_DEBTS.md#td-13)).
- Demo step 2: the lab's agreement with Lux Biobank needs a matching
  purpose; a wrong purpose is refused.
- A second health scenario, the HDAB as permit authority (maintainer,
  2026-11-23): the HDAB only issues the data permit; Lux Biobank checks
  it at negotiation and sends the data where the permit says. It needs
  the permit credential (above) and push (1.1), and one more piece no
  release has yet: a check that the transfer's destination matches a
  claim in the permit. No specification for that is known; research
  first. A folder in `scenarios/` once it runs.
- Exit: purpose and permit tested granted and refused; data-permits runs
  with the R-48, TD-46 and TD-20 steps changed to refusals or passes;
  the TCKs and the EDC interop test stay green.
- Main risk: DPV purposes may not map one to one to the purposes of the
  EHDS (Article 53).

### 1.3 Data user and HDAB: the HDAB and Schuman Clinical Research

- The HDAB plays the health data access body towards Schuman Clinical
  Research, the data user; the lab and Lux Biobank stay a peer to peer
  agreement (maintainer, 2026-11-15). Schuman Clinical Research needs a
  data permit, issued by the HDAB (EHDS Art. 68), so the HDAB runs its
  own issuer next to the authority's.
- No secure processing environment is built: the HDAB runs a fixed
  pipeline on Lux Biobank's data that produces counts, and pushes that
  output to RustFS in Schuman Clinical Research's infrastructure
  (maintainer, 2026-11-15).
- Agreements that depend on an upstream agreement: the HDAB's offer to
  Schuman Clinical Research cannot be wider than the HDAB's agreement
  with Lux Biobank (purpose,
  users, duration). No specification defines this; research and ADR
  first ([R-39](RISKS_AND_TECHNICAL_DEBTS.md#r-39)).
- A third health scenario, the HDAB negotiating on a data user's behalf
  (maintainer, 2026-11-23): the HDAB asks Lux Biobank for an agreement
  whose assignee is Schuman Clinical Research. Today Lux Biobank refuses
  it (the offer's assignee must be the caller, ADR 0013), and an
  agreement the HDAB makes for itself is checked against the HDAB's
  credentials and transferable only to the HDAB. It needs a way for the
  data user to delegate to the HDAB that the holder can verify; DSP and
  DCP define none that we know of. Research and an ADR first, like
  [R-39](RISKS_AND_TECHNICAL_DEBTS.md#r-39).
- Demo step 3: the four questions (below) answered by running the demo,
  each with its evidence, and a note of where Portitor fell short.
- Exit: the four answers written in the docs.
- Main risk: our design for dependent agreements may not match what EDC
  or Simpl-Open do later.

### 1.4 Keys and credentials

- A signer port with an OpenBao transit adapter (P-256 signing, versioned
  keys; the private key never leaves OpenBao), optional
  ([TD-05](RISKS_AND_TECHNICAL_DEBTS.md#td-05)).
- Credential schemas, presentation definitions and issuance policies, if
  a target dataspace needs them
  ([TD-14](RISKS_AND_TECHNICAL_DEBTS.md#td-14)); STS token and
  presentation caching only if the 0.11 measurements show the need
  ([TD-07](RISKS_AND_TECHNICAL_DEBTS.md#td-07)).
- Exit: a key change in OpenBao with no verification gap (tested).
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
  ([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)). Done on the 1.1
  branch: `crates/portitor-dsp/src/process.rs` and
  `crates/portitor-connector/src/test_stores.rs`.
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
  the Helm chart: the demo dataspace runs from it on a local cluster,
  and DEPLOYING explains it.
- Main risk: a refactoring that changes behaviour unseen. The TCKs and
  the interop test are the guard, so no item starts while one of them
  is red.

### Research before code

Docs only: a standards check, then an ADR the maintainer accepts or
rejects. It runs beside the releases.

| Topic | Needed by |
| --- | --- |
| Several dataspaces ([TD-03](RISKS_AND_TECHNICAL_DEBTS.md#td-03)) | Accepted in 0.13 ([ADR 0024](adr/0024-several-data-spaces.md)) |
| Users and roles ([TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)) | Accepted in 0.13 ([ADR 0025](adr/0025-users-and-roles.md)); code in 1.5 |
| Who calls the data plane ([TD-02](RISKS_AND_TECHNICAL_DEBTS.md#td-02)) | 1.5 |
| S3 data address | Answered in 1.1: none is needed ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)) |
| Agreements that depend on an upstream agreement | 1.3 |
| Consent | 1.5 |
| How schemas are published ([TD-01](RISKS_AND_TECHNICAL_DEBTS.md#td-01)) | with TD-03 |
| A dataspace's policy rules as SHACL shapes in the bundle (maintainer, 2026-11-18; [TD-32](RISKS_AND_TECHNICAL_DEBTS.md#td-32)) | 1.2 |
| Spike: the contract negotiation state machine written out in DSP terms, every state and every transition, and a check that the stored data matches it (maintainer, 2026-11-21) | Done 2026-11-25 ([spike](spikes/1.x-negotiation-state-machine.md)): states and stored data match DSP; gaps G1 to G6 and their next steps are in the spike, release not chosen |

### The health dataspace demo

Four participants and an authority (maintainer, 2026-11-15), modelled on
a health data access body (HDAB) and the ICUdata4EU ICU dataspace. The
names are made up, each modelled on a Luxembourg institution
(maintainer, 2026-11-23). The authority, Digital Lux Trust, governs the
dataspace: it runs the issuer and publishes the bundle, the minimal
trust and semantics, and grants no data access; it is not the HDAB
(maintainer, 2026-11-23). Lux Biobank, a data holder, holds genotypes.
Lux Biomedicine Lab, a data user, asks Lux Biobank directly, peer to
peer (the EHDS does not exclude it); Lux Biobank pushes the data to
object storage (RustFS) in the lab's own infrastructure. The HDAB plays
the access body towards Schuman Clinical Research, another data user:
the HDAB issues its data permit, runs a fixed pipeline on Lux Biobank's
data that produces counts, and pushes that output to RustFS in Schuman
Clinical Research's infrastructure. No secure processing environment
(SPE) is built; the pipeline stands in for one (maintainer, 2026-11-15).
The demo must answer, by running it:

- can the same data be reached through several channels (Lux Biobank's
  offer and the HDAB's)?
- does the HDAB negotiate with Lux Biobank for every new cohort query,
  or does one agreement cover them?
- does the HDAB negotiate with Lux Biobank for every new access through
  its pipeline, or for every new data user?
- do Lux Biobank's and the HDAB's policies differ, and can the HDAB's be
  less strict because its environment is safer? What can never be wider
  than what Lux Biobank allowed the HDAB (purpose, users, duration)?

Step 0 runs on v1.0.0-rc.1 as it is, with pull transfers and synthetic
genotypes, and gives the first answers:
the [hdab-intermediary](../scenarios/hdab-intermediary/README.md)
scenario (maintainer, 2026-11-23). Steps 1 to 3 update it as 1.1 to 1.3 land.

Checked against the EHDS (Regulation (EU) 2025/327, 2026-11-15): for
secondary use a data user applies to the HDAB, which issues a data
permit (Art. 67, 68); pseudonymised data is processed only in an SPE,
and only non-personal data can be downloaded (Art. 73); most of it
applies from March 2029.

#### More health scenarios (maintainer, 2026-11-23)

Each one becomes a folder in `scenarios/`. Before the next ones are
built, the ways to set up the relationship between data user, HDAB and
data holder are compared, with pros and cons (maintainer, 2026-11-23).

| Scenario | What it shows | Runs on |
| --- | --- | --- |
| The HDAB as permit authority | The HDAB only issues the data permit; the holder checks it at negotiation and sends the data where the permit says | 1.1 and 1.2, plus a destination check no release has yet |
| The HDAB on a data user's behalf | The HDAB negotiates with the holder for a data user | research first: a delegation the holder can check (1.3) |
| A mixed data space | One participant runs Eclipse EDC instead of Portitor, in the health story | 1.0 |
| The holder withdraws its dataset | New negotiations fail, existing agreements keep their data (ADR 0020) | 1.0 |
| Cross-border | A data user in another Member State, with its own authority, issuer and bundle; how HealthData@EU links national nodes is read first | partly 1.0 (several trusted issuers) |
| A foreign issuer (maintainer, 2026-11-24) | Lux Biobank trusts an issuer of another Member State next to Digital Lux Trust, and what that issuer may then issue ([R-48](RISKS_AND_TECHNICAL_DEBTS.md#r-48)) | 1.0 |
| A holder without its own connector | A small holder puts its data with the HDAB, which publishes it; the legal side is read first | 1.0 |
| Opt-out | A person's opt-out of secondary use (EHDS Art. 71) changes the data before it leaves | 1.8, research first |

#### Ways to link data user, HDAB and holder (maintainer, 2026-11-23)

Six ways, compared with pros and cons in a
[design note](https://claude.ai/artifact/W9SXQmmRXXDsibZhz38c36). They are
explored one at a time, in this order; each ends with a short note of
what worked and what did not, and a demo or an ADR where it earns one
(maintainer, 2026-11-23).

| Order | Way | Who the holder checks, where the data goes | Status |
| --- | --- | --- | --- |
| 1 | A. The HDAB as intermediary | the HDAB; data to the HDAB, results to the user | done: `scenarios/hdab-intermediary` |
| 2 | F. Permit-gated peer to peer | the user's credential; data to the user (non-personal data only, EHDS Art. 73(2)) | explored on 1.0: `scenarios/data-permits`, placeholder permit claims; findings [R-48](RISKS_AND_TECHNICAL_DEBTS.md#r-48), [TD-20](RISKS_AND_TECHNICAL_DEBTS.md#td-20), [TD-46](RISKS_AND_TECHNICAL_DEBTS.md#td-46) |
| 3 | C. Permit credential, data to the SPE | the user's permit; data into the SPE the permit names | needs 1.1, 1.2 and a destination check |
| 4 | E. The trusted holder runs the SPE (EHDS Art. 72) | the user's permit; data stays, results leave | needs 1.2; the SPE is outside Portitor |
| 5 | B. Chained agreements | the HDAB; its offers held inside its agreement | 1.3, research and ADR (R-39) |
| 6 | D. The HDAB on the user's behalf | a delegation and the user's permit | research and ADR; refused today (ADR 0013) |

## 2027: EHDS participant functions and Simpl-Open tests (proposal, 2026-11-21)

The maintainer asked for the roadmap to cover what a participant of an
EHDS-interoperable dataspace may need, with tests against Simpl-Open,
as about one year of work (maintainer, 2026-11-21). This section is that
year: the releases after 1.5, to the end of 2027. Like the 1.x plan, each
release ships when its exit criteria pass, not on a date, and each
feature starts with research and an ADR.

### What the sources say (read 2026-11-21)

| Fact | Source |
| --- | --- |
| For secondary use, a data user applies to the HDAB, which issues a data permit (Art. 67, 68) or answers a data request with anonymised statistics (Art. 69). Pseudonymised data is processed only in a secure processing environment (Art. 73). Most of it applies from March 2029. | Regulation (EU) 2025/327, checked 2026-11-15 |
| The HDAB keeps a national dataset catalogue (Art. 77). Article 77(4) leads to an implementing act on the minimum metadata of a dataset description. Its staff working document, sent to the Council on 6 November 2026, names HealthDCAT-AP. | [Council ST 15505/26](https://data.consilium.europa.eu/doc/document/ST-15505-2026-INIT/en/pdf) (SWD(2026) 342) |
| Datasets can carry a data quality and utility label (Art. 78); people can opt out of secondary use (Art. 71). | Regulation (EU) 2025/327 |
| HealthData@EU, the cross-border infrastructure, connects national contact points through dispatchers and eDelivery access points: "Messaging is based on ebMS3 ... and its payload-agnostic AS4 Conformance Profile", with Domibus as reference. Metadata is HealthDCAT-AP. It does not name DSP, DCP, connectors or Simpl. | [TEHDAS2 D7.3](https://tehdas.eu/wp-content/uploads/2026/05/d7.3-technical-specification-for-health-data-access-bodies-on-the-implementation-of-the-common-it-infrastructure.pdf), sections 4.1, 5.1, 6 ([ADR 0025](adr/0025-users-and-roles.md)) |
| The HealthData@EU central platform is open source (EUPL-1.2) since release 3 (March 2025). Its Health Data Dispatcher replaced the "EHDS2 Pilot connector" and talks AS4 only. | [EC Digital Building Blocks, 2025-03-27](https://ec.europa.eu/digital-building-blocks/sites/pages/viewpage.action?pageId=887383631), [code.europa.eu/healthdataeu](https://code.europa.eu/healthdataeu/healthdataeu-eu-dataset-catalogue) |
| Simpl-Open: Gdynia (4.0) released July 2026; Hamburg (4.1.0) released 2026-11-05 ([ADR 0017](adr/0017-simpl-open.md#running-a-governance-authority-locally)); Innsbruck (5.0, target Q4 2026) planned. The development status page names no DSP version, sandbox or conformance kit. Its connector is built on EDC, behind a Tier 2 gateway with mutual TLS and an "ephemeral proof". | [Development status](https://simpl-programme.ec.europa.eu/documentation/development-status), [ADR 0017](adr/0017-simpl-open.md) |

Not verified yet: D7.3 beyond its first third (an end-to-end test
framework and an ITB conformance use case are mentioned in its annexes)
and the date of the EHDS implementing acts. Each ADR below reads them
first.

So two worlds meet in a health dataspace: participants sharing data
peer to peer (DSP and DCP, where Portitor sits) and the HDAB and
HealthData@EU (AS4, applications and permits). Portitor's place is the
first, with clean hand-overs to the second.

### 1.6 Data holder: describe datasets the EHDS way

- Dataset descriptions that meet the minimum metadata of the Art. 77(4)
  implementing act, in HealthDCAT-AP, checked with its SHACL shapes
  (ADR 0005 already checks HealthDCAT-AP release 8). The holder sends
  the same description to the HDAB's catalogue; how it is sent depends
  on the HDAB, so the ADR first asks which national catalogues take
  DCAT-AP harvest.
- The data quality and utility label (Art. 78) in the description, in
  the place HealthDCAT-AP gives it (to check), never a field of our own.
- Exit: a description of the demo's dataset passes the HealthDCAT-AP
  shapes and the EU ITB validator, and lists each minimum element.
- Main risk: the implementing act is adopted after this release; the
  ADR then follows the draft and says so.

### 1.7 Simpl-Open tests

Replaces the "Simpl-Open" item of "Later": no shared sandbox exists, so
the test runs Simpl-Open locally from its public code. The plan follows
the maintainer's answers to [ADR 0017](adr/0017-simpl-open.md#maintainers-answers-2026-11-21)
(2026-11-21).

- Step 1, the protocol: a DSP-level test against Simpl-Open's connector,
  `connector-be`, in its own local mode (mocked identity attributes, no
  Tier 2 gateway). Catalog, negotiation and transfer, both directions.
  Portitor sends Simpl's JSON token only in the test build. It needs no
  governance authority and no Kubernetes. It confirms on the wire the
  DSP version `connector-be` speaks (its 1.3.0 tag notes say 2025-1).
- Step 2, the way in: a Compose file, built in this release, runs the
  governance authority components the Tier 2 handshake needs from
  Simpl-Open's published images, plus a Tier 2 gateway in front of
  Portitor. Portitor gets an x509 certificate and identity attributes
  from it and is called through that gateway. Not Simpl-Open's Helm
  install: it needs Argo CD, Kubernetes 1.33 with several add-ons and
  about 8 CPU and 32 GB RAM for the authority alone, too much for a
  local test (maintainer, 2026-11-21;
  [ADR 0017](adr/0017-simpl-open.md#running-a-governance-authority-locally)).
  The component list comes from Simpl-Open's `authority-iaa` chart when
  the file is built. Before step 2: the maintainer reviews the "trusted
  Simpl gateway" adapter again (ADR 0017, answer 3, deferred) and it
  gets its own ADR.
- Step 3, users: the user-to-system test of 1.5 run again against the
  newest Simpl-Open release.
- A CI job only if the Simpl-Open stack fits a runner; otherwise a
  runbook, like the demo. Step 1 should fit; step 2 is checked once the
  Compose file exists.
- Exit: steps 1 and 2 pass, or each failure is written down with its
  cause; STABILITY says what was tested, with which Simpl-Open release.
  No upstream reports or questions: Portitor watches Simpl-Open's
  releases only (maintainer, 2026-11-17 and 2026-11-21).
- Main risk: Simpl-Open publishes and tests only its Helm install, so
  the Compose file can break with each Simpl-Open release, or a needed
  component may not run outside Kubernetes. Then step 2 waits, and the
  step that failed is written down.
- Watch, at each Simpl-Open release until 1.7 starts (maintainer,
  2026-11-21): which protocol "SSI Tier 2" (planned for Innsbruck, 5.0)
  uses between agents. Today Simpl-Open uses OpenID4VP only for people
  signing in (Tier 1, a Keycloak extension with a walt.id verifier).
  If agents move to DCP, the gateway adapter is no longer needed; if
  they move to OpenID4VP, ADR 0017 is reviewed.

### 1.8 Data user and HDAB: permits, requests and the SPE

Builds on 1.2 (data permit as credential) and 1.3 (the HDAB and Schuman Clinical Research).

- Data permit life cycle: validity, change and withdrawal of a permit
  (Art. 68) end the agreements and transfers that rely on it, through
  the status list Portitor already checks.
- Data request (Art. 69): the HDAB answers with anonymised statistics only;
  the policy says so and the transfer refuses anything else.
- Hand-over to a secure processing environment (Art. 73): Portitor
  pushes the data into an SPE the operator runs, and only the output
  leaves it. Portitor builds no SPE.
- Opt-out (Art. 71): the ADR finds out how Member States publish opt-out
  and whether the holder or the HDAB applies it, before any code.
- Exit: the demo's four questions run again with permits that change and
  end; the opt-out ADR is accepted or rejected.
- Main risk: national rules on opt-out differ; there may be nothing
  common to build.

### 1.9 Hand-over to HealthData@EU

Research first; code only if the ADR finds a hand-over worth building.

- How a Portitor participant meets HealthData@EU: does the HDAB need
  anything from it beyond the HealthDCAT-AP descriptions of 1.6? Read
  D7.3 in full, the central platform's code and its test framework.
- Options to weigh: none (the HDAB does it all), the HDAB harvesting
  Portitor's DSP catalog, or Portitor talking to an AS4 access point.
  AS4 is not planned: a second protocol stack is a large step and needs
  the maintainer's yes.
- Exit: the ADR, accepted or rejected; the ITB conformance tests, if
  public, run against what 1.6 publishes.
- Main risk: HealthData@EU and the dataspace protocols stay apart, so a
  bridge serves no one yet.

### What Portitor does not plan to do in this year

- Run an HDAB, a national contact point or an application portal for
  data users.
- Build a secure processing environment.
- Speak AS4 or run a Domibus access point, unless the 1.9 ADR is
  accepted.
- Define its own metadata, label or permit format: each comes from the
  EHDS acts, HealthDCAT-AP, DCP, EDC or Simpl-Open.

## Later (not planned for a release yet)

- A separate, pluggable data plane (maintainer, 2026-11-25): the
  connector drives data planes over Data Plane Signaling, operators
  register their own data planes for new sources and transfer types
  without a Portitor release, and the built-in data plane becomes one
  of them, linked in by default and also an optional image
  `portitor-dataplane`. Spike done
  ([spike](spikes/1.x-separate-data-plane.md)); decision in
  [ADR 0028](adr/0028-separate-pluggable-data-plane.md) (Accepted
  2026-11-25).
  Four steps, each its own release: untie the built-in data plane
  inside the process (no change of behaviour, closes
  [TD-48](RISKS_AND_TECHNICAL_DEBTS.md#td-48)); signaling over HTTP and
  registration; the separate image; interop with an outside data plane
  and an EDC control plane. Closes
  [TD-17](RISKS_AND_TECHNICAL_DEBTS.md#td-17). Step 1 goes in a 1.x release
  when it fits; steps 2 to 4 later in 1.x, or after Data Plane
  Signaling 1.0 is released (maintainer, 2026-11-25).
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
  Signaling or EDC, but add security or complete what a dataspace
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
- [A provider in several dataspaces, each with its own rules](RISKS_AND_TECHNICAL_DEBTS.md#td-03) (TD-03)
- [User to system authentication and authorisation, governed per dataspace](RISKS_AND_TECHNICAL_DEBTS.md#td-04) (TD-04)
- [Key custody and key rotation](RISKS_AND_TECHNICAL_DEBTS.md#td-05) (TD-05)

The other technical debts (TD-06 onwards) and the risks (R-01 onwards)
are listed there too, with their status and target release.
