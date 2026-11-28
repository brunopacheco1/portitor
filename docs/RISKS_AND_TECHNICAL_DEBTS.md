<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Risks and technical debts

The single list of risks and technical debts of Portitor. Other files
link here and do not repeat it: the [roadmap](ROADMAP.md) keeps the plan,
[DEPLOYING](DEPLOYING.md#known-limits) keeps the operator's view ("Known
limits").

- **Risk** (`R-nn`): something that could go wrong: security,
  interoperability, a specification that changes upstream, a dependency,
  or delivery.
- **Technical debt** (`TD-nn`): a known shortcut or a missing capability
  in the code or the design.

Status values:

- **Open**: nothing removes it yet.
- **Mitigated**: reduced, not removed. The item says how.
- **Accepted**: kept on purpose. The item says who decided.
- **Addressed**: done. The item is one line in the [Closed](#closed)
  table, which says where the fix is described.

An item whose fix is being built on a release branch keeps its status
until the release; its "Target" and "Current state" say "in progress"
with the release, and its "Next" says what is being built. It is moved
to [Closed](#closed) when that release ships.

> **Top risks for 1.0** (chosen from the items that are not closed, by
> what they can cost at or after 1.0)
>
> 1. [R-02](#r-02) One maintainer, who finds Rust hard to read
>    ([R-01](#r-01)): every change, security fixes included, has one
>    reviewer.
> 2. [R-12](#r-12) Gaia-X was never tested with a real trust anchor.
>    Every Gaia-X claim rests on a local service in test mode.
> 3. [R-17](#r-17) Server-side request forgery: the block on internal
>    addresses on URLs a partner chooses is built (1.4), but off by
>    default in 1.x; it is on by default in 2.0.
> 4. [R-09](#r-09) EDC is tested at a release candidate. Interoperability
>    with EDC 1.0.0, the first reference counterpart, is not proven.
> 5. [TD-05](#td-05) Private keys are PEM files.
>
> Next in line: [TD-31](#td-31) (no audit trail of management
> actions; each data transfer writes an audit line since 1.1),
> [R-24](#r-24) (Simpl-Open is untested) and [R-25](#r-25) (other
> projects and a company use the name Portitor).

## How to keep this file up to date

Every release review ([RELEASING](RELEASING.md#documentation-review))
checks this file:

1. Check each item below against the code of the release and update its
   status.
2. Add what the release raised (ADR consequences, spike risks, review
   findings, the maintainer's decisions). Give each a new id; never
   reuse an id.
3. When an item is addressed, move it to the [Closed](#closed) table as
   one line, with its anchor (`<a id="td-nn"></a>`), so links to it keep
   working. If it leaves something behind, add a line to
   [Left over from closed items](#left-over-from-closed-items).
4. Check the "Top risks" box.
5. Every item that needs work has a release in its "Target" column
   (maintainer, 2026-11-26). An item with no work to plan (accepted,
   waiting on someone else, or a standing check) says why in "Target".

Once a year, with the first release after 26 November, a **yearly risk
review** re-reads every item with no release and asks: is the reason
still true, did the upstream event happen, should it get a release
now? The first one is in [1.14](ROADMAP.md#114-operations-hardening-added-2026-11-26).

## Items that are not closed

"Target" is the release the roadmap names. An item with no work to
plan says why instead (a standing check, an upstream event, or "none"
for an accepted residual risk); the yearly risk review re-reads those.

| ID | Title | Kind | Status | Target |
|---|---|---|---|---|
| [R-01](#r-01) | Rust is hard to read for the maintainer and slower to build | Delivery | Mitigated | every release |
| [R-02](#r-02) | One maintainer: no two-person review | Delivery, security | Accepted | until a second maintainer |
| [R-03](#r-03) | Pre-1.0 JSON-LD and VC crates | Dependency | Mitigated | none |
| [R-04](#r-04) | Most dependencies are exempted, not audited (cargo-vet) | Dependency, security | Mitigated | every release |
| [R-05](#r-05) | OpenSSF Baseline level 1: one requirement not met, one to confirm | Security | Open | the maintainer: turn on OSPS-BR-03.01 (Pages: Enforce HTTPS), confirm OSPS-AC-01.01 (2FA) |
| [R-06](#r-06) | Data Plane Signaling is a draft (v1.0-RC4) | Upstream | Mitigated | Signaling 1.0 |
| [R-08](#r-08) | The DSP TCK v1.0.2 needs a test-only build | Interoperability | Mitigated | a fixed TCK release |
| [R-09](#r-09) | EDC is tested at a release candidate (1.0.0-RC8) | Interoperability | Mitigated | EDC 1.0.0 |
| [R-10](#r-10) | EDC as provider refuses the Gaia-X compliance credential | Interoperability | Accepted | 1.13 |
| [R-11](#r-11) | Gaia-X does not use DCP; the binding and scope rules are Portitor's own reading | Interoperability, upstream | Open | 1.13 |
| [R-12](#r-12) | Gaia-X never tested with a real trust anchor or the Gaia-X Lab | Interoperability, security | Open | 1.13 |
| [R-13](#r-13) | The Gaia-X test depends on Gaia-X and w3id.org hosts | Delivery | Mitigated | none |
| [R-14](#r-14) | Gaia-X protocols move: the Compliance Dispatcher is announced, not released | Upstream | Mitigated | 1.13 |
| [R-16](#r-16) | A Gaia-X compliance credential cannot be revoked | Security | Accepted | none |
| [R-17](#r-17) | Server-side request forgery through partner-chosen URLs | Security | Mitigated | 2.0 (the block on by default) |
| [R-19](#r-19) | Consumer's data access tokens in the database and the management API | Security | Mitigated | none |
| [R-20](#r-20) | The issuer key is the root of trust of a dataspace | Security | Mitigated | 1.6 (TD-05) |
| [R-21](#r-21) | Bugs in Portitor's own SHACL engine; new SHACL components in later profiles | Security, upstream | Mitigated | none |
| [R-22](#r-22) | Bundle: unsigned `latest.json`; a revoked bundle key not seen while offline | Security | Mitigated | 1.8 |
| [R-23](#r-23) | A wrong host clock gives wrong policy and token decisions | Security | Mitigated | none |
| [R-24](#r-24) | Simpl-Open: parked, facts not verified, and it changes fast | Interoperability, upstream | Accepted | 1.9 |
| [R-25](#r-25) | The name Portitor is not exclusive: other projects and a company use it | Delivery, legal | Accepted | none |
| [R-27](#r-27) | CI downloads from Maven Central and GitHub | Delivery | Mitigated | none |
| [R-29](#r-29) | DCP costs about 2 ms per request and 8 times fewer requests under load | Performance | Open | 1.6 |
| [R-36](#r-36) | The OpenTelemetry Rust crates are 0.x and change their API often | Dependency, upstream | Mitigated | none |
| [R-37](#r-37) | Telemetry costs some throughput at saturation | Performance | Mitigated | none |
| [R-39](#r-39) | No specification defines agreements that depend on an upstream agreement | Interoperability | Open | 1.5 (research ADR); code only if the ADR finds a way to follow |
| [R-40](#r-40) | No shared list of EHDS purposes | Interoperability | Open | 1.5 (map the DPV purposes to Article 53) |
| [R-41](#r-41) | A key removed from a DID document is still trusted for up to 5 minutes | Security | Mitigated | none |
| [R-42](#r-42) | The STS shares the identity's public port | Security | Open | 1.6 |
| [R-43](#r-43) | Negative credential constraints can be met with another credential | Security | Accepted | none (documented in 1.4) |
| [R-44](#r-44) | One slow holder delays every issuer delivery | Security | Open | 1.6 |
| [R-45](#r-45) | No request limit per partner | Security | Open | 1.7 |
| [R-46](#r-46) | The EDC Issuer Service names a request status address that is not found | Interoperability, upstream | Open | report upstream; the next EDC release |
| [R-47](#r-47) | The EDC Issuer Service's admin API may change with the next release candidate | Interoperability | Accepted | EDC 1.0.0 |
| [R-49](#r-49) | Leases in the database depend on the hosts' clocks | Operations | Mitigated | none |
| [R-50](#r-50) | The catalog asks every caller for the scopes of all access policies, so members learn which credential types hidden datasets need | Security | Accepted | 1.5 (check written into the entry) |
| [R-51](#r-51) | No way for a data user to delegate a negotiation to the HDAB that the holder can verify | Interoperability | Open | 1.5 (research ADR); code 1.10, if the ADR is accepted |
| [TD-01](#td-01) | How schemas and vocabularies are collected and stored | Technical debt | Open | 1.8 |
| [TD-02](#td-02) | Who calls the data plane, and with which credentials | Technical debt | Open | research; code 1.7 |
| [TD-03](#td-03) | A provider in several dataspaces, each with its own rules | Technical debt | Accepted | 1.9 |
| [TD-04](#td-04) | User to system authentication and authorisation | Technical debt | Accepted | code 1.7 |
| [TD-05](#td-05) | Key custody and key rotation | Technical debt | Open | 1.6 (OpenBao) |
| [TD-07](#td-07) | STS tokens and presentations are not cached | Technical debt | Open | 1.6, if needed |
| [TD-10](#td-10) | Transfers: no token renewal; whole sources and whole pushes only | Technical debt | Mitigated | 1.7 |
| [TD-11](#td-11) | The provider cannot suspend a transfer from the management API | Technical debt | Open | 1.6 |
| [TD-12](#td-12) | ODRL: few left operands; prohibitions and duties not enforced | Technical debt | Open | 1.10 (after a check of what the EHDS permit and HealthData@EU use) |
| [TD-14](#td-14) | Credential schemas, presentation definitions and issuance policies are not supported | Technical debt | Open | 1.6, if needed |
| [TD-15](#td-15) | Agreements are not revised when the data's scope changes | Technical debt | Open | 1.7 |
| [TD-16](#td-16) | One harvest source; no dataspace registry; other catalogs not checked; no bundle expiry or bundle key rotation | Technical debt | Open | 1.8 |
| [TD-17](#td-17) | The data plane runs inside the connector process | Technical debt | Accepted | 1.12 (steps 2 to 4; step 1 done in 1.2) |
| [TD-19](#td-19) | No TLS inside the services | Technical debt | Accepted | 1.9 |
| [TD-21](#td-21) | Gaia-X beyond 0.8 | Technical debt | Open | 1.13 |
| [TD-22](#td-22) | Migrations run forward only; a rollback needs the backup | Technical debt | Accepted | none |
| [TD-28](#td-28) | No trace across participants | Technical debt | Accepted | none |
| [TD-29](#td-29) | No OTLP logs export, no gRPC, no compression | Technical debt | Open | 1.14 |
| [TD-31](#td-31) | No audit trail of management actions | Technical debt | Mitigated | 1.7 |
| [TD-35](#td-35) | No negotiation started by the provider | Technical debt, interoperability | Open | 1.9 (with the Simpl-Open tests) |
| [TD-39](#td-39) | DEPLOYING is not in deployment order; the docs are long | Technical debt, usability | Mitigated | 1.14 (Helm chart) |
| [TD-40](#td-40) | Database calls block the async runtime | Technical debt | Accepted | 1.12 |
| [TD-45](#td-45) | Each running pull holds about 0.8 MB of memory | Technical debt | Open | 1.7 |
| [TD-49](#td-49) | After a failed message, the two parties of a negotiation can stay in different states | Technical debt, interoperability | Accepted | none |
| [TD-59](#td-59) | SQLite is deprecated; no tool copies a SQLite file into PostgreSQL | Technical debt | Accepted | removal: 2.0 (no copy tool) |
| [TD-62](#td-62) | Why and when a negotiation ended is not stored | Technical debt | Open | 1.5 |
| [TD-63](#td-63) | Source files above 1,000 lines | Technical debt, readability | Open | 1.5 |
| [TD-64](#td-64) | Too many type parameters; macros that write types | Technical debt, readability | Open | 1.6 |
| [TD-65](#td-65) | `Negotiator` does more than negotiate | Technical debt, readability | Open | 1.7 |
| [TD-66](#td-66) | A data plane step runs inside the request that asks for it | Technical debt | Open | 1.12 |
| [TD-67](#td-67) | Datasets are harvested from one kind of source only | Technical debt | Open | 1.8 |
| [TD-68](#td-68) | No dataspace-wide registry or federated catalogue | Technical debt | Open | 1.8, only if needed |
| [TD-69](#td-69) | No Simpl-Open features beyond the protocols (mutual TLS, consent, API gateway) | Technical debt | Open | consent code: 1.7; ADR: 1.9 |
| [TD-72](#td-72) | Two error types for the database, converted both ways | Technical debt, readability | Open | 1.5 (with TD-63) |
| [TD-73](#td-73) | A negotiation step is about 3.5 ms slower on SQLite in 1.3 | Technical debt, performance | Open | 1.12 (with TD-40; cause found in 1.4) |
| [TD-74](#td-74) | An issuer not limited by type is trusted for every credential type | Technical debt, security | Accepted | 2.0 |
| [TD-75](#td-75) | A client constructor called `new` does not apply the block on internal addresses | Technical debt, readability, security | Open | 1.5 (with TD-63) |
| [TD-76](#td-76) | A refused DSP message is logged as "counter-party not reachable" and its reason is lost; a refused transfer request is not logged by the provider | Technical debt, operations | Open | 1.5 |
| [TD-77](#td-77) | The identity answers 400 when an issuer's metadata cannot be read | Technical debt | Open | 1.5 |
| [TD-78](#td-78) | The catalog holds every dataset's access policy during a request | Technical debt, performance | Open | 1.5 (time-boxed to one day; else 1.14) |
| [TD-79](#td-79) | The identity records a set status bit of an unknown purpose as SUSPENDED, and checks a REVOKED credential again | Technical debt | Open | 1.5 |

Counts on 2026-11-27, at the 1.4.0 release: 37 risks and 40 technical
debts are not closed (Open 37, Mitigated 22, Accepted 18); 13 risks and
38 technical debts are closed (9 of them at 1.4: R-48, TD-13, TD-20,
TD-46, TD-60, TD-61, TD-70 and TD-71, and TD-47, fixed in 1.1).

## Risks

<a id="r-01"></a>
### R-01 Rust is hard to read for the maintainer and slower to build

- **What.** Rust is harder to learn and slower to compile than the
  alternatives ([ADR 0002](adr/0002-technology-stack.md)). The
  maintainer is not a Rust expert and finds Rust hard to read
  (maintainer, 2026-11-13).
- **Why it matters.** Code the maintainer cannot read cannot be
  reviewed, and there is one maintainer (R-02).
- **Current state: Mitigated.** [RUST-IDIOMS](RUST-IDIOMS.md) explains
  every idiom the code uses. From 0.14,
  [READING-THE-CODE](READING-THE-CODE.md) gives a reading order and
  follows one request from the route to the outbox. A readability
  review runs before each release since 0.4
  ([RELEASING](RELEASING.md#documentation-review)). Clippy pedantic as
  errors, small crates, no `unsafe`.
- **Next.** Keep the review; add each new idiom to RUST-IDIOMS. The
  [2026-11-18 review](reviews/2026-11-18-specifications-and-code-quality.md)
  (Q6) found services with four type parameters
  (`ProviderService<S, D, C, E>`), four macros that write types, five
  files above 1,000 lines, and `Negotiator`, which also handles
  transfers and the harvest. All are planned in ROADMAP "Code quality
  in 1.x". In 1.2 the data plane left the connector as its own crate,
  split into one file per topic, none above 1,000 lines (tests not
  counted).

<a id="r-02"></a>
### R-02 One maintainer: no two-person review

- **What.** A sole maintainer cannot approve their own pull request, so
  the ruleset allows an admin bypass for merging. Scorecard Code-Review
  and Contributors stay low; controls that need two people wait
  ([ADR 0012](adr/0012-openssf-baseline.md)).
- **Why it matters.** Every change is reviewed by one person. Bus
  factor of one.
- **Current state: Accepted** (maintainer, 2026-11-11). Signed commits,
  required CI checks, CodeQL and Sonar gate every merge. An account
  successor is set, who can take over the repositories; the signing
  key and the container registry go with the maintainer's account
  ([GOVERNANCE, "Continuity"](../GOVERNANCE.md#continuity)).
- **Next.** Find a second maintainer; not a 1.0 blocker (maintainer,
  2026-11-15).

<a id="r-03"></a>
### R-03 Pre-1.0 JSON-LD and VC crates

- **What.** Rust's JSON-LD and verifiable credential crates are pre-1.0
  (ADR 0002).
- **Why it matters.** Breaking changes or bugs in a security path.
- **Current state: Mitigated.** DSP and DCP messages are plain JSON with
  typed structs, checked against the official JSON Schemas
  (ARCHITECTURE principle 6). Credentials and tokens use Portitor's own
  code on `ring`. A JSON-LD processor (`oxjsonld`) is used only for the
  SHACL checks, with local contexts (ADR 0005).
- **Next.** None beyond R-04.

<a id="r-04"></a>
### R-04 Most dependencies are exempted, not audited (cargo-vet)

- **What.** From 0.10 CI runs `cargo vet --locked`, so a new or bumped
  crate cannot merge without an audit or a recorded exemption
  ([CONTRIBUTING](../CONTRIBUTING.md#dependency-audits-cargo-vet)).
  Imported audits cover 45 of the 204 crate versions; the other 159 are
  exempted at the versions locked on 2026-11-16: trusted, not reviewed.
- **Why it matters.** A malicious or careless crate update could reach a
  release.
- **Current state: Mitigated.** Also running: pinned actions,
  Dependabot, dependency review, OSV-Scanner, `cargo deny`, ORT
  ([TESTING](TESTING.md#what-runs-where)).
- **Next.** Review or exempt again each bump of an exempted crate, so
  the list shrinks. Count the exemptions at each release.
- **1.3.** `tokio-postgres` (ADR 0029) brings 32 crate versions that
  the imported audits do not cover; they are exempted (maintainer,
  2026-11-26), and so are the six versions audited before, because an
  audit must mean a person read the code. On 2026-11-26: 57 crate
  versions fully audited by the imported sets, 1 partly, 185 exempted
  in `supply-chain/config.toml`; `audits.toml` is empty.
- **1.4.** No new crate version (`tower-service`, now a direct
  dependency of `portitor-runtime`, was already in the tree). Counted
  on 2026-11-27: 185 `[[exemptions]]` entries in
  `supply-chain/config.toml`, the same as at the 1.3.0 release.

<a id="r-05"></a>
### R-05 OpenSSF Baseline level 1: one requirement not met, one to confirm

- **What.** Portitor commits to OpenSSF Baseline level 1
  ([ADR 0012](adr/0012-openssf-baseline.md)). The control-by-control
  self-assessment is written; two GitHub settings are left. (Until 1.4
  the title was "No written OpenSSF Baseline level 1
  self-assessment".)
- **Why it matters.** Until they are done, level 1 is not fully met.
- **Current state: Open; written in 1.4.** Decided
  (maintainer, 2026-11-20): the target stays level 1 for 1.0. The
  self-assessment, first written for 0.14, was checked again control by
  control on 2026-11-27 against Baseline v2026.08.28, still the current
  version that day, in
  [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#level-1-self-assessment):
  of 24 level 1 requirements, 21 met, 1 not applicable, 1 to confirm
  (two-factor authentication on the maintainer's account) and 1 not
  met: the website answers over plain HTTP ("Enforce HTTPS" is off for
  the repository's Pages, OSPS-BR-03.01).
- **Next.** The maintainer turns on "Enforce HTTPS" (OSPS-BR-03.01)
  and confirms two-factor authentication (OSPS-AC-01.01)
  ([Settings to confirm](SECURITY-ASSURANCE.md#settings-to-confirm));
  then the item moves to Closed. No code or release is needed.

<a id="r-06"></a>
### R-06 Data Plane Signaling is a draft (v1.0-RC4)

- **What.** The interface between control plane and data plane follows
  Eclipse Data Plane Signaling v1.0-RC4, an incubating specification
  with no release
  ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate)).
  Since 1.1, push follows the Signaling HTTP Transfer Profile, a draft
  that is not tagged ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)).
- **Why it matters.** The final release may change messages or states.
  A change to the HTTP Transfer Profile changes what other connectors
  see of a push.
- **Current state: Mitigated.** The data plane sits behind the ports of
  `portitor-dps`: function calls in one process, no wire format to
  keep. Other connectors see only DSP, the data endpoint and, for a
  push, the profile's `endpointType` and bearer token.
- **Next.** Track the specification until 1.0 and record each update in
  the changelog; then split the data plane out (TD-17).

<a id="r-08"></a>
### R-08 The DSP TCK v1.0.2 needs a test-only build

- **What.** (1) A race in the TCK's consumer: a start message sent at
  once can be refused, and every retry gets 404. (2) The TCK sends one
  fixed `Authorization` header, which cannot be a DCP token. (3) The TCK
  itself disables TP:02-04 and TP_C:02-04.
- **Why it matters.** The conformance run does not test exactly the
  release build.
- **Current state: Mitigated.** The `tck` feature, never in release
  images: start messages go out 300 ms late (`tck::LateStart`,
  [ADR 0014](adr/0014-transfers-as-consumer.md)); the outbox tries a 404
  again; the fixed header maps to the participant id `TCK_PARTICIPANT`
  ([ADR 0016](adr/0016-dsp-caller-authentication.md)). DCP
  authentication is covered by the DCP TCK and the EDC interop test.
- **Next.** The race is reported ([upstream report 6](#upstream-reports)).
  Its fix was merged upstream on 2026-11-17 (dsp-tck pull request 339)
  and is not released. Drop `LateStart` when a TCK release has it
  (maintainer, 2026-11-17).

<a id="r-09"></a>
### R-09 EDC is tested at a release candidate (1.0.0-RC8)

- **What.** The EDC interop test runs EDC 1.0.0-RC8
  (`scenarios/edc-interop/runtimes/gradle.properties`). The IdentityHub Identity
  API the test uses is in the package `v1.unstable` in RC8
  ([TESTING](TESTING.md#edc-interop-test)); for the Issuer Service's
  admin API see [R-47](#r-47). EDC's reading of a
  specification can change
  ([ADR 0011](adr/0011-interoperability-targets.md)).
- **Why it matters.** Interoperability with the final EDC release is not
  proven.
- **Current state: Mitigated.** The test runs on pull requests that
  touch the code and nightly. Whether EDC 1.0.0 has shipped was not
  checked for this file.
- **Next.** Pin EDC 1.0.0 when it ships and run the test again.

<a id="r-10"></a>
### R-10 EDC as provider refuses the Gaia-X compliance credential

- **What.** EDC applies its subject check (`HasValidSubjectIds`,
  unchanged in 1.0.0-RC8) to every credential of a DCP presentation it
  receives. The Gaia-X compliance credential's subject is not the
  holder's DID, so EDC refuses it
  ([ADR 0022](adr/0022-gaia-x-compliance-in-policies.md)).
- **Why it matters.** An EDC provider cannot ask Portitor for Gaia-X
  compliance. The EDC IdentityHub has no Gaia-X onboarding either.
- **Current state: Accepted** (maintainer, 2026-11-15). From 0.13 the
  EDC interop test shows the other direction: an EDC consumer holds a
  compliance credential, presents it, and a Portitor provider admits it
  ([TESTING](TESTING.md#gaia-x-with-edc-what-the-test-shows-and-what-it-does-not)).
  DEPLOYING "Known limits" names the limit.
- **Next.** Later: ask EDC how it would accept Gaia-X compliance
  credentials ([upstream report 14](#upstream-reports), on hold).
  Assigned to 1.13 (maintainer, 2026-11-26).

<a id="r-11"></a>
### R-11 Gaia-X does not use DCP; the binding and scope rules are Portitor's own reading

- **What.** Gaia-X supports VC-API style REST and OpenID4VC, not DCP. No
  specification says how to present a Gaia-X compliance credential over
  DCP. Portitor ties it to the caller through the `digestSRI` of the
  caller's `gx:LegalPerson`
  ([ADR 0022](adr/0022-gaia-x-compliance-in-policies.md)) and matches a
  scope type `T` to `gx:T`
  ([ADR 0021](adr/0021-gaia-x-onboarding.md)).
- **Why it matters.** Other implementations may choose differently, and
  a later Gaia-X or DCP text may contradict ours.
- **Current state: Open.** Both rules are tested between Portitor
  connectors only.
- **Next.** Follow [upstream reports](#upstream-reports) 9 (scope
  grammar) and 12 (holder binding). The Gaia-X side is not asked yet
  (report 13). Assigned to 1.13 (maintainer, 2026-11-26).

<a id="r-12"></a>
### R-12 Gaia-X never tested with a real trust anchor or the Gaia-X Lab

- **What.** The Gaia-X end-to-end test uses a local compliance service
  in test mode: `DISABLE_SIGNATURE_CHECK=true` (certificate chains not
  checked against Gaia-X trust anchors) and `production=false` (any
  registration number issuer accepted), with a test CA and a test notary
  (`examples/gaiax/compliance.yaml`). A run against the Gaia-X Lab needs a
  public `did:web` and a CA certificate; Portitor runs only on the
  maintainer's laptop (maintainer, 2026-11-15).
- **Why it matters.** The real trust chain and a real notary are not
  proven; the Lab may expect another ontology version
  ([spike 0.8](spikes/0.8-gaia-x-compliance.md), "Not checked").
- **Current state: Open.** [TESTING](TESTING.md#gaia-x-end-to-end-test)
  lists what the test does not prove.
- **Next.** ROADMAP 1.13: a test against the Lab once Portitor runs
  beyond a laptop. Assigned to 1.13 (maintainer, 2026-11-26).

<a id="r-13"></a>
### R-13 The Gaia-X test depends on Gaia-X and w3id.org hosts

- **What.** The local compliance service loads shapes, ontology and
  contexts from `registry.lab.gaia-x.eu` (through `w3id.org`) and
  `www.w3.org`, with Node.js clients that ignore `HTTPS_PROXY`
  (spike 0.8, finding 6).
- **Why it matters.** A change or outage there breaks the test with no
  change in Portitor.
- **Current state: Mitigated.** The images are pinned by digest; the
  workflow runs nightly (`.github/workflows/gaiax.yml`), so a break
  shows apart from code changes.
- **Next.** None planned.

<a id="r-14"></a>
### R-14 Gaia-X protocols move: the Compliance Dispatcher is announced, not released

- **What.** The Compliance Dispatcher's wallet flow (OpenID4VP and
  OpenID4VCI) is announced (Gaia-X, 2026-07-24), not released. Portitor
  talks to the clearing house with its REST API (ADR 0021). The service
  writes `iat` and `exp` in milliseconds (spike 0.8).
- **Why it matters.** The REST API may be replaced.
- **Current state: Mitigated.** The service is a port
  (`GaiaxCompliance`) with an HTTP adapter, so OpenID4VCI can be a
  second adapter. Validity is read from `validFrom` and `validUntil`
  only.
- **Next.** ROADMAP 1.13, Gaia-X beyond 0.8 (TD-21).
  Assigned to 1.13 (maintainer, 2026-11-26).

<a id="r-16"></a>
### R-16 A Gaia-X compliance credential cannot be revoked

- **What.** The compliance credential has no `credentialStatus`; it is
  valid up to 90 days (spike 0.8). The caller's `gx:LegalPerson` status
  is not fetched, because the caller controls it (ADR 0022).
- **Why it matters.** A participant that loses its compliance keeps it
  until it expires.
- **Current state: Accepted** (ADR 0022, maintainer, 2026-11-15): the
  credential's validity bounds it, and `validUntil` is required.
- **Next.** Follow Gaia-X if it adds a status mechanism.

<a id="r-17"></a>
### R-17 Server-side request forgery through partner-chosen URLs

- **What.** A partner chooses URLs that Portitor calls: the connector
  POSTs to the `callbackAddress` and fetches the caller's DID document
  and credential service
  ([ADR 0016](adr/0016-dsp-caller-authentication.md)); the issuer POSTs
  credentials to the credential service a registered holder's DID
  document names (`crates/portitor-issuer/src/delivery.rs`); the
  identity resolves the DID of any caller of its presentation API before
  it checks the caller's `aud` and access token
  ([SECURITY-ASSURANCE](SECURITY-ASSURANCE.md), findings F-01 and F-06).
  Since 1.1 ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)):
  as provider, the data plane POSTs a push to the `endpoint` of the
  consumer's `dataAddress` (`crates/portitor-dataplane/src/push.rs`),
  with the dataset as the body. The connector also calls the object
  storage at `AWS_ENDPOINT_URL_S3`
  (`crates/portitor-dataplane/src/object_storage.rs`); the operator
  chooses that address, not a partner.
- **Why it matters.** A partner could make a component call an internal
  service. A push endpoint that points inside the provider's network
  would receive the provider's data.
- **Current state: Mitigated.** Only `https://` (unless
  `PORTITOR_ALLOW_HTTP`, for tests), no redirects, time and size
  limits, the callback's answer body is never read; DEPLOYING asks for
  an egress firewall. The same holds for the push endpoint: it is
  checked when the transfer request arrives (400 otherwise), redirects
  are not followed and the answer's body is not read; a push has no
  time limit, since a dataset can be large. Since 1.4 internal addresses
  can be blocked, but the block is off by default in 1.x.
- **1.4.** The block on internal addresses (F-01), off by default: `PORTITOR_BLOCK_INTERNAL_ADDRESSES` and
  `PORTITOR_INTERNAL_ADDRESSES_ALLOWED`, read at start by all three
  components. The HTTP clients for partner-chosen URLs resolve the host
  themselves and connect only to an address the rule allows, so no
  second DNS lookup can change it (`portitor-runtime/src/http/connect.rs`);
  clients for URLs the operator chooses do not apply it
  ([DEPLOYING](DEPLOYING.md#internal-addresses) has both lists). The
  identity checks `aud` and the access token before it resolves a
  caller of its presentation API (F-06), always on. Engineering review
  of 1.4: "internal" is every address IANA's special-purpose registries
  mark as not globally reachable, multicast and broadcast, with the
  IPv4 inside NAT64, 6to4, Teredo and other IPv6 forms; the connector's
  DSP check and the DCP services check `aud`, `iss` and the times
  before they resolve a caller's DID; a refusal sent back to a partner
  (a push's termination) names no address.
- **Next.** The block on by default in 2.0 ([ROADMAP](ROADMAP.md#for-20)).
  The 0.13 freeze passed without it, so a block that is on by default
  would be a behaviour change in 1.x ([ADR 0016](adr/0016-dsp-caller-authentication.md),
  amended in 1.4). Until then DEPLOYING asks for an egress firewall and
  says how to turn the block on ([Internal addresses](DEPLOYING.md#internal-addresses)).

<a id="r-19"></a>
### R-19 Consumer's data access tokens in the database and the management API

- **What.** As consumer, the connector keeps each usable transfer's
  data address, token included, in its database (a SQLite file before
  1.3), and shows it on the management API ([ADR 0014](adr/0014-transfers-as-consumer.md)). Push
  transfers (1.1, [ADR 0027](adr/0027-push-transfers-and-object-storage.md))
  add two more tokens to the file, never to the management API:
  - as consumer, the push token our receiving route takes is in the
    pending transfer request (or start message) in plain text, until
    the provider has it (the outbox, `pending` of `consumer_transfer`);
    after that only its SHA-256 is kept (`push_receipt`);
  - as provider, the consumer's push token, with its endpoint, is kept
    with the transfer and its data flow until the transfer ends
    (`transfer.data_address`, `data_flow.push_address`), so that a
    restart can still send the push.
- **Why it matters.** Whoever reads the database or its backups, or
  holds the API key, can fetch the data until the transfer ends or the token
  expires; with a push token, they can write into the consumer's push
  bucket (as consumer) or into the consumer's endpoint (as provider).
- **Current state: Mitigated.** The token is removed when the transfer
  ends; tokens expire (`PORTITOR_DATA_TOKEN_SECONDS`, default one
  hour); only an HMAC of the API key is kept; DEPLOYING ("Security")
  says to protect the database, its backups and the key alike. The provider empties
  the push columns when the transfer ends; our push token works only
  while the transfer is `REQUESTED` or `STARTED`.
- **Next.** Encrypting the token in the database is later work
  (ADR 0014); no release named.

<a id="r-20"></a>
### R-20 The issuer key is the root of trust of a dataspace

- **What.** The issuer key signs every credential and status list of
  the dataspace ([ADR 0018](adr/0018-issuer-component.md)). Whoever
  controls a DID in `PORTITOR_TRUSTED_ISSUERS` can admit any
  participant.
- **Why it matters.** A stolen key admits anyone; a lost key means
  every participant needs new credentials (TD-05).
- **Current state: Mitigated.** The key lives only in `portitor-issuer`,
  never on a participant's host (ADR 0003; DEPLOYING has backup
  advice). The bundle has its own key, kept offline (ADR 0005).
- **Next.** Key custody (TD-05).

<a id="r-21"></a>
### R-21 Bugs in Portitor's own SHACL engine, and new SHACL components in later profiles

- **What.** Portitor uses its own SHACL Core subset, `portitor-shacl`,
  not rudof, which fails `cargo deny`
  ([spike 0.6](spikes/0.6-shacl-engine.md),
  [ADR 0005](adr/0005-schema-and-vocabulary-registry.md)).
  HealthDCAT-AP release 8 already adds a SPARQL rule.
- **Why it matters.** A bug passes a bad dataset or refuses a good one;
  a shape the engine does not support cannot be enforced.
- **Current state: Mitigated.** The W3C SHACL tests for the implemented
  components and a comparison with the EU ITB validator
  (`crates/portitor-shacl/tests`); shapes with a term the engine does
  not implement are refused; a work limit (`MAX_CHECKS`) ends in a
  violation, never a pass; fuzz targets `shacl_load` and
  `shacl_jsonld`.
- **Next.** Per profile release, the maintainer decides whether to add
  a component or leave the rule out of the bundle. Look at rudof again
  if its HTTP and CLI dependencies become optional.

<a id="r-22"></a>
### R-22 Bundle: unsigned `latest.json`, and a revoked bundle key not seen while offline

- **What.** `latest.json` is not signed. While the authority cannot be
  reached, the connector reuses the cached bundle with the key it
  stored, so a bundle key revoked meanwhile is not noticed.
- **Why it matters.** An attacker on the path could hold back a newer
  bundle; a compromised bundle key keeps working on offline connectors.
- **Current state: Mitigated.** The version `latest.json` names is
  verified (signature by `<authority>#bundle-key`, every SHA-256); the
  connector never takes a version issued before the one in use
  ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)).
- **Next.** Bundle expiry and bundle key rotation (TD-16).
  Assigned to 1.8 (maintainer, 2026-11-26).

<a id="r-23"></a>
### R-23 A wrong host clock gives wrong policy and token decisions

- **What.** `dateTime` constraints, token expiry and credential validity
  use the host clock ([ADR 0009](adr/0009-minimal-odrl-policies.md)).
- **Why it matters.** Wrong decisions near a limit.
- **Current state: Mitigated.** DEPLOYING asks hosts to run NTP; 60 s
  of clock skew is allowed for tokens.
- **Next.** None.

<a id="r-24"></a>
### R-24 Simpl-Open: parked, facts not verified, and it changes fast

- **What.** Simpl-Open's connector uses no DCP; agents authenticate
  with x509 and mTLS at their Tier 2 gateway
  ([ADR 0017](adr/0017-simpl-open.md), Accepted 2026-11-21). Several facts could
  not be verified (ADR 0017, "Not verified"). Simpl-Open's schemas are
  not signed (ADR 0005).
- **Why it matters.** Portitor is not a Simpl participant; reference
  counterpart number two
  ([ADR 0011](adr/0011-interoperability-targets.md)) is untested.
- **Current state: Accepted** (parked by the maintainer, 2026-11-13,
  until a Simpl-Open sandbox exists; planned again for 1.9 without a
  shared sandbox, 2026-11-21).
  [STABILITY](STABILITY.md#what-10-does-not-do) says it is not tested.
- **Next.** ROADMAP 1.9, "Simpl-Open tests" (maintainer, 2026-11-21):
  a Simpl-Open agent run locally from its public code; check the facts
  again first. Assigned to 1.9 (maintainer, 2026-11-26).

<a id="r-25"></a>
### R-25 The name Portitor is not exclusive

- **What.** A preliminary search on 2026-11-21 (public databases only,
  not legal advice) found no live trademark for "Portitor" or a close
  spelling in Nice classes 9 or 42. It searched TMview (EUIPO, BOIP,
  WIPO, USPTO and the EU national offices) and the WIPO Global Brand
  Database, exact and fuzzy. The only live "PORTITOR" registration is a
  Portuguese logo with no Nice class, held by a retail company. The name
  is used without a trademark by a Dutch health-data conversion company
  (portitor.nl), by an open-source e-commerce project started in March
  2026 (portitor.org, portitor.io), and by small GitHub projects.
  portitor.com is taken. `portitor` is free on crates.io, npm and PyPI
  ([ADR 0008](adr/0008-project-name.md)).
- **Why it matters.** Someone can still register the name in classes 9
  or 42, and a rename after 1.0 is expensive. Search results and the
  .com, .org and .io domains belong to others.
- **Current state: Accepted** (maintainer, 2026-11-21). The legal risk is
  low; the risk of confusion is medium. No trademark is filed and no
  domain is bought.
- **Next.** Search again before a trademark filing or a commercial
  launch.

<a id="r-27"></a>
### R-27 CI downloads from Maven Central and GitHub

- **What.** Maven Central answered HTTP 429 on 2026-11-15 when CI
  fetched the TCKs.
- **Why it matters.** Red CI with no code change.
- **Current state: Mitigated.** The DCP TCK jar is cached, keyed on its
  SHA-256 and checked after every restore; the DSP TCK build's Gradle
  dependencies are cached; downloads are retried
  (`.github/workflows/tck.yml`).
- **Next.** None.

<a id="r-29"></a>
### R-29 DCP costs about 2 ms per request and 8 times fewer requests under load

- **What.** Each DSP request costs a DID resolution, an STS token and a
  presentation query. Measured in 0.12
  ([BENCHMARKS](BENCHMARKS.md), "DCP cost"), with the status list
  cache: 1.6 ms per request for one client, and 8 times fewer requests
  under load with 50 clients (2,935 against 23,751 per second). 0.5
  measured about 2 ms and 13 times
  ([ADR 0002](adr/0002-technology-stack.md), section 7).
- **Why it matters.** Capacity, and load on partners' identities and
  issuers.
- **Current state: Open.** DID documents and status lists are cached
  (`PORTITOR_DID_CACHE_SECONDS`, `PORTITOR_STATUS_LIST_CACHE_SECONDS`,
  default 5 minutes each); tokens and presentations are not.
- **Next.** What is left is the STS token and the presentation query of
  each request ([TD-07](#td-07)).
  Assigned to 1.6 (maintainer, 2026-11-26).

<a id="r-36"></a>
### R-36 The OpenTelemetry Rust crates are 0.x and change their API often

- **What.** Traces and metrics use `opentelemetry`,
  `opentelemetry_sdk`, `opentelemetry-otlp` and `tracing-opentelemetry`.
  They are 0.x and change their API with most minor releases
  ([ADR 0023](adr/0023-observability.md)).
- **Why it matters.** Each update can need code changes, and an update
  that is put off can leave a security fix behind.
- **Current state: Mitigated.** All OpenTelemetry code is in one
  module, `crates/portitor-runtime/src/telemetry.rs` and `telemetry/`,
  and Dependabot updates the crates together in one group.
- **Next.** None.

<a id="r-37"></a>
### R-37 Telemetry costs some throughput at saturation

- **What.** In the 0.9 measurements a connector at saturation served 8%
  fewer DSP requests per second than 0.8.0 with export off, 12% fewer
  with every request traced and 18% fewer with one trace in ten
  (OPERATING, "Traces and metrics"); runs vary by up to 15%. Sampling
  lowers the data sent, not the cost in the process.
- **Why it matters.** A busy connector with full tracing reaches its
  limit sooner.
- **Current state: Mitigated.** Export is off by default; the
  differences are close to the noise of one machine.
- **Next.** Measure again with the benchmarks
  ([BENCHMARKS](BENCHMARKS.md)); if the cost is real, look at what the
  spans cost when export is off.

<a id="r-39"></a>
### R-39 No specification defines agreements that depend on an upstream agreement

- **What.** In the health dataspace demo, the HDAB's offer to Schuman
  Clinical Research rests on the HDAB's agreement with Lux Biobank and
  must not be wider than it. DSP and ODRL have no link between two
  agreements. Step 0 of the demo keeps the HDAB's end date inside its
  agreement by hand ([scenarios/hdab-intermediary](../scenarios/hdab-intermediary/README.md)).
- **Why it matters.** Portitor's design may differ from what EDC or
  Simpl-Open choose later.
- **Current state: Open.** In progress for 1.5:
  [ADR 0031](adr/0031-agreements-that-depend-on-an-upstream-agreement.md)
  (Proposed, 2026-11-28) found no existing way to follow. DSP 2025-1
  has no link between agreements. ODRL 2.2 has `inheritFrom` and
  `nextPolicy`, but DSP does not carry them and ODRL has no rule that a
  downstream policy stays inside an upstream one. EDC and Simpl-Open
  have no link; Tractus-X states onward sharing as legal terms that
  always hold. Under the EHDS the limit comes from the data permit the
  HDAB issues (Art. 61(2), 68). So the scenario keeps the limit by
  hand, as in step 0; no code.
- **Next.** The maintainer accepts or rejects ADR 0031. Look again when
  DSP, EDC or Simpl-Open add a link between agreements, or with the
  EHDS permit (1.10).

<a id="r-40"></a>
### R-40 No shared list of EHDS purposes

- **What.** The EHDS lists the purposes of secondary use (Article 53),
  but no list with identifiers exists to reuse; the maintainer knows of
  none in ICUdata4EU or LNDS (2026-11-15). HealthDCAT-AP release 8
  names purposes with `dpv:hasPurpose`.
- **Why it matters.** A `purpose` constraint is useful only if both
  sides name purposes the same way.
- **Current state: Open.** Since 1.4 offers may
  state a `purpose` as any IRI ([ADR 0009](adr/0009-minimal-odrl-policies.md),
  amendment in 1.4); the docs and the hdab-intermediary demo
  use the DPV EHDS extension's draft IRIs
  (`https://w3id.org/dpv/legal/eu/ehds#`). Portitor ships no list.
  In progress for 1.5: the research note
  [1.5-dpv-ehds-purposes](spikes/1.5-dpv-ehds-purposes.md) (2026-11-28)
  found that DPV EHDS 2.3 names the point of Article 53(1) for each of
  its secondary use purposes, and covers every point, (a) to (f) with
  (e)(i) and (e)(ii). The link is a free-text `dct:source`, the
  extension is a draft Community Group report, and HealthDCAT-AP
  release 8 uses no controlled list for `dpv:hasPurpose`. The two IRIs
  the docs and scenarios use exist and fit their points:
  `HealthcareScientificResearch` (e) and `ImproveHealthcareEducation`
  (d). It stays Open: the list is a draft and nothing else uses it yet.
- **Next.** Keep accepting any IRI. Look again when the EHDS permit
  template is adopted (1.10) or DPV EHDS leaves draft.

<a id="r-41"></a>
### R-41 A key removed from a DID document is still trusted for up to 5 minutes

- **What.** Every component caches each resolved DID document
  (`crates/portitor-runtime/src/did.rs`): at most 1,000 documents, no
  failed lookups (SECURITY-ASSURANCE, F-04).
- **Why it matters.** A participant that removes a leaked key from its
  DID document is still accepted with that key until the cached
  document expires.
- **Current state: Mitigated (0.12).** The time is a setting,
  `PORTITOR_DID_CACHE_SECONDS`: default 300, at most 3600, `0` turns
  the cache off ([ADR 0016](adr/0016-dsp-caller-authentication.md),
  amended 2026-11-18). The window is still there at the default.
- **Next.** None.

<a id="r-42"></a>
### R-42 The STS shares the identity's public port

- **What.** `/sts/token` is a route of the identity's public router
  (`crates/portitor-identity/src/lib.rs`), next to the DID document and
  the credential service. DEPLOYING says to route it on the internal
  network only (SECURITY-ASSURANCE, F-05).
- **Why it matters.** A proxy that forwards every path exposes the STS
  to the internet. The client secret (at least 32 characters, compared
  in constant time) still protects it, so the risk is low.
- **Current state: Open.** The 0.13 freeze passed without a separate
  listen address.
- **Next.** A separate listen address for the STS (a new setting, which
  1.x can add), or a deployment check that the proxy does not route it.
  Assigned to 1.6 (maintainer, 2026-11-26).

<a id="r-43"></a>
### R-43 Negative credential constraints can be met with another credential

- **What.** A credential constraint holds if at least one valid
  credential of the type has the claim and the comparison is true
  ([ADR 0019](adr/0019-credential-constraints.md)). For `neq` and
  `isNoneOf`, a holder with two credentials of the type, one with the
  excluded value and one without, meets the constraint. The holder also
  chooses what it presents (SECURITY-ASSURANCE, F-07).
- **Why it matters.** An operator may read `neq` as "refuse holders
  with this value". It is not.
- **Current state: Accepted** (maintainer, 2026-11-27: docs only, the
  behaviour stays). ADR 0019,
  [amended in 1.4](adr/0019-credential-constraints.md#negative-operators-are-not-deny-lists-r-43),
  and [DEPLOYING, "Credential constraints"](DEPLOYING.md#credential-constraints)
  say that `neq` and `isNoneOf` are not deny lists, and how to keep
  someone out instead. The behaviour does not change.
- **Next.** None. To keep someone out, revoke the credential at the
  issuer, or limit the issuer by type (`PORTITOR_TRUSTED_ISSUER_TYPES`).

<a id="r-44"></a>
### R-44 One slow holder delays every issuer delivery

- **What.** The issuer's outbox sends due deliveries one after the
  other (`send_due_at` in `crates/portitor-issuer/src/delivery.rs`),
  and each call may take 10 s. A registered holder whose credential
  service hangs delays every other holder (SECURITY-ASSURANCE, F-09).
- **Why it matters.** New members wait for their credentials.
- **Current state: Open.** Bounded: a holder has at most
  `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` active credentials per
  definition, rejections are not delivered, and retries wait longer
  each time.
- **Next.** Send to different holders side by side, a few at a time.
  Assigned to 1.6 (maintainer, 2026-11-26).

<a id="r-45"></a>
### R-45 No request limit per partner

- **What.** The public ports limit requests in all (256 at once, 60 s
  each, `crates/portitor-runtime/src/serve.rs`), not per partner. A
  download is not limited once its answer has started
  (SECURITY-ASSURANCE, F-10).
- **Why it matters.** One authenticated partner can take the capacity
  the others need.
- **Current state: Open.** DEPLOYING says nothing about rate limits.
- **Next.** Document rate limits per client at the reverse proxy; a
  limit in Portitor only if measurements show the need.
  Assigned to 1.7 (maintainer, 2026-11-26).

<a id="r-46"></a>
### R-46 The EDC Issuer Service names a request status address that is not found

- **What.** The Issuer Service of EDC IdentityHub 1.0.0-RC8 answers a
  credential request with a `Location` header that leaves out the path
  of its issuance API (`CredentialRequestApiController.java`, line 100,
  builds `/v1/participants/{context}/requests/{id}` without
  `web.http.issuance.path`). `portitor-identity` asks for the request
  status there (DCP v1.0.1, 6.8) and gets 404. Found in 0.14 by step 8
  of the EDC interop test
  ([TESTING](TESTING.md#a-second-issuer-the-edc-issuer-service)).
- **Why it matters.** `GET /v1/credential-requests/{holderPid}` on the
  identity's management API shows `issuerStatusError` for a request that
  went well. An operator may read it as a failure.
- **Current state: Open.** The credential arrives all the same, as the
  issuer delivers it to the Storage API. Portitor has no workaround:
  guessing the path of another issuer's API would be wrong for the next
  issuer. The test prints the gap as known and does not fail on it;
  [DEPLOYING](DEPLOYING.md#an-edc-issuer-service-as-the-issuer) tells
  the operator.
- **Next.** The maintainer reports it to EDC IdentityHub
  ([Upstream reports](#upstream-reports), 15). Check again with the
  next EDC release ([R-09](#r-09)).

<a id="r-47"></a>
### R-47 The EDC Issuer Service's admin API may change with the next release candidate

- **What.** Step 8 of the EDC interop test sets the Issuer Service up
  through its admin API (holders, an attestation, a credential
  definition, revocation). In RC8 that API is in a package EDC names
  `unstable`, and its calls to suspend and resume a credential answer 501
  ([TESTING](TESTING.md#a-second-issuer-the-edc-issuer-service)).
- **Why it matters.** A new EDC release candidate can break the test
  script. Only the test is affected: Portitor's components speak DCP to
  an issuer and never call that API.
- **Current state: Accepted.** The cost is a change to
  `scenarios/edc-interop/run.sh` when EDC is updated, as with the Identity API
  ([R-09](#r-09); TESTING, "From RC7 to RC8").
- **Next.** Read the admin API again at each EDC update. Suspension is
  not tested until EDC implements it.

<a id="r-49"></a>
### R-49 Leases in the database depend on the hosts' clocks

- **What.** From 1.3, processes that share a database take leases of
  30 seconds on the work they run (outbox messages, pushes, uploads,
  scheduled jobs, deliveries, a transfer's data plane step). Each
  process writes and reads the end of a lease with its own clock, as
  Eclipse EDC does ([ADR 0029](adr/0029-several-replicas.md)).
- **Why it matters.** A host whose clock runs ahead sees the others'
  leases end too early, and a host whose clock runs behind writes
  leases that end too early for the others. Either way a process can
  take over work that is still running: a message is sent twice, or a
  push starts twice. A host whose clock runs ahead also writes leases
  that last longer after its process died.
- **Also.** Used token ids (TD-09) are deleted once their token has
  expired by the clock of the process that deletes them: a host whose
  clock runs ahead by some seconds deletes them that much early, and a
  token replayed in that window to a process whose clock is right is
  accepted again.
- **Current state: Mitigated.** DEPLOYING asks for NTP on every host
  ([Database](DEPLOYING.md#database)). Each lease is held by one claim
  (the process id and a random id), and a process whose lease was taken
  over, or that could not renew it for about 20 seconds (a database
  failover), stops that work (a push, an upload, a message, a Gaia-X
  run), so that it does not run beside the process that took it over;
  what already went out may go out twice. Work done twice is safe in
  most places: a partner that gets a DSP message twice answers with an
  error and the state check goes on
  ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md)); a data plane
  step may run twice. Dataset harvest runs do not depend on clocks: a
  run is written only if no other run was written since it read the
  store. Related: [R-23](#r-23).
- **Next.** None. Leases from the database's own clock (`now()`) would
  remove it; not planned.

<a id="r-50"></a>
### R-50 The catalog asks every caller for the scopes of all access policies, so members learn which credential types hidden datasets need

- **What.** Since 1.4 a dataset may have an access policy
  ([TD-13](#td-13), closed). On a catalog request the provider asks the
  caller's credential service for the scopes of the credential types of
  **every** access policy, also of datasets hidden from that caller
  ([ADR 0019](adr/0019-credential-constraints.md), amendment in 1.4).
  Eclipse EDC differs: on a catalog request its provider verifies the
  token with an empty policy (`CatalogProtocolServiceImpl.getCatalog`,
  read at EDC Connector commit `4215744` on 2026-11-27), so it asks only
  for its default scopes, the same for every caller.
- **Why it matters.** A member, and its credential service, which sees
  the query, learn which credential types the provider's access
  policies use, though not their values nor which dataset uses which.
- **Current state: Accepted** (engineering review of 1.4, 2026-11-27,
  in the ADR 0019 amendment): the types are named in the dataspace's
  credential definitions anyway, and asking for fewer scopes would hide
  a dataset from a caller that meets its policy.
- **Next.** In 1.5, no code (maintainer, 2026-11-28): the 1.5 check
  is written into this entry, and it stays Accepted.

<a id="r-51"></a>
### R-51 No way for a data user to delegate a negotiation to the HDAB that the holder can verify

- **What.** Way D of the health demo
  ([ROADMAP](ROADMAP.md#ways-to-link-data-user-hdab-and-holder-maintainer-2026-11-23)):
  the HDAB asks Lux Biobank for an agreement whose assignee is Schuman
  Clinical Research. Lux Biobank refuses it today: the offer's assignee
  must be the caller ([ADR 0013](adr/0013-transfer-and-data-plane.md)), and an
  agreement the HDAB makes for itself is checked against the HDAB's
  credentials and can be transferred only to the HDAB. DSP and DCP
  define no delegation that we know of.
- **Why it matters.** A data user that wants the HDAB to negotiate for
  it cannot use Portitor; a delegation of our own would not
  interoperate with EDC or Simpl-Open.
- **Current state: Open.**
- **Next.** In progress for 1.5: a research ADR, no code (maintainer,
  2026-11-28). Code in 1.10 only if the ADR is accepted and finds a way
  to follow.

## Technical debt

<a id="td-01"></a>
### TD-01 How schemas and vocabularies are collected and stored

- **What.** The authority publishes a signed bundle in a format Portitor
  defines (`manifest.json`, `manifest.jws`, `latest.json`;
  [ADR 0005](adr/0005-schema-and-vocabulary-registry.md)), and the
  connector pulls it into a disk cache. Only its version fields come
  from a standard (DCAT 3 `dcat:version`, `dcterms:issued`). No
  dataspace specification we follow defines how an authority publishes
  schemas: Simpl-Open has its own schema-manager API, not signed.
  Portitor must not invent a standard, so the format may change
  (maintainer, 2026-11-14).
- **Current state: Open.** The source and the store are their own
  crate, `portitor-bundle`, so they can be replaced as a whole. The
  manifest format is frozen for 1.0 with one opening: a connector skips
  a file whose role it does not know, with a warning and a metric, and
  refuses a bundle in which it knows no role (ADR 0005, "Amendment for
  0.13"). The limit: a connector that skips a new kind of rule does not
  enforce it.
- **Next.** Research with TD-03 (ROADMAP "Research before code"): how
  Simpl-Open, Gaia-X and HealthData@EU authorities publish schemas,
  whether a DCAT catalogue of schema distributions can replace
  `latest.json`, and where the connector should keep them.
  Assigned to 1.8 (maintainer, 2026-11-26).

<a id="td-02"></a>
### TD-02 Who calls the data plane, and with which credentials

- **What.** Only participants are identified, and only in the control
  plane ([ARCHITECTURE](ARCHITECTURE.md#who-holds-the-credentials)). The
  data plane checks only the bearer token handed out when the transfer
  started ([ADR 0013](adr/0013-transfer-and-data-plane.md)): whoever
  holds the token gets the data until the transfer ends. No identity or
  credential is presented on each data request (maintainer, 2026-11-15).
- **Current state: Open.**
  [ADR 0025](adr/0025-users-and-roles.md) decides that the provider's
  data plane does not identify persons in 1.x. Since 1.1 the provider
  checks again, on each data request and while a download runs, the
  `MembershipCredential` the consumer presented when it asked for the
  transfer (issuer still trusted, not expired, not revoked;
  [ADR 0027](adr/0027-push-transfers-and-object-storage.md), section 7).
  The caller itself still shows nothing but the token.
- **Next.** Research, then an ADR; code in ROADMAP 1.7. To compare: a
  DCP token or a presentation per request, OpenID4VP, Simpl-Open's mTLS
  at its Tier 2 gateway, what the EDC data plane does; whether the
  policy is checked again on each request; how this fits the consent
  and gateway items of [TD-69](#td-69).

<a id="td-03"></a>
### TD-03 A provider in several dataspaces, each with its own rules

- **What.** A provider may belong to several dataspaces, each with its
  own rules for dataset descriptions, offers, credentials and where
  data may be processed. One connector serves one dataspace: one
  bundle, one list of trusted issuers, one datasets file (maintainer,
  2026-11-15).
- **Current state: Accepted** for 1.0 (maintainer, 2026-11-20,
  [ADR 0024](adr/0024-several-data-spaces.md)): one connector per
  dataspace. One DID per dataspace is the documented default; a shared DID
  is allowed, and DEPLOYING names what it discloses
  ([Two data spaces](DEPLOYING.md#two-data-spaces);
  `scenarios/two-data-spaces/run.sh`).
- **Next.** After 1.0, no release named: one connector with several
  dataspace profiles, behind its own ADR; trusted issuers by
  credential type (threat model finding F-08), which is additive.
  Assigned to 1.9 (maintainer, 2026-11-26).

<a id="td-04"></a>
### TD-04 User to system authentication and authorisation, governed per dataspace

- **What.** The people and applications inside an organisation are not
  identified: the management APIs use one API key per component, and
  the data plane checks only the transfer's bearer token
  ([ARCHITECTURE](ARCHITECTURE.md#who-holds-the-credentials);
  maintainer, 2026-11-15).
- **Current state: Accepted** for 1.0 (maintainer, 2026-11-20,
  [ADR 0025](adr/0025-users-and-roles.md)): 1.0 stays system to system.
  Users stop at the organisation's own reverse proxy
  ([DEPLOYING](DEPLOYING.md#users-and-roles)). At the 0.13 freeze the
  management API key and the data token were stated as opaque, HTTP 403
  as reserved on the management routes, and `dataAddress` as optional
  ([STABILITY](STABILITY.md)).
- **Next.** In 1.7: access tokens of the organisation's identity
  provider on the management APIs, with EDC's scope names, behind its
  own code ADR. The data pull audit line: [TD-31](#td-31). Linked to
  [TD-02](#td-02).

<a id="td-05"></a>
### TD-05 Key custody and key rotation

- **What.** Each component (identity, issuer, bundle signing) reads one
  P-256 private key from a PEM file (ADR 0015, ADR 0018; maintainer,
  2026-11-15: a key custody service such as OpenBao is probably
  better).
- **Current state: Open.** Rotation without a gap is done for the
  identity and the issuer since 0.11: old public keys stay listed as
  retired keys (`PORTITOR_IDENTITY_RETIRED_KEY_FILES`,
  `PORTITOR_ISSUER_RETIRED_KEY_FILES`), so what the old key signed
  keeps verifying and the old private key can be destroyed at once
  (ADR 0015 and ADR 0018, "Amended in 0.11: retired keys"; OPERATING,
  "Rotating a key").
- **Still open.** Key custody: the private key is a PEM file. The
  bundle key cannot be rotated (one key, TD-16). With Gaia-X, each key
  change needs a new certificate and a new compliance run
  ([ADR 0021](adr/0021-gaia-x-onboarding.md)).
- **Next.** ROADMAP 1.6: a signer port with an OpenBao transit adapter.

<a id="td-07"></a>
### TD-07 STS tokens and presentations are not cached

- **What.** Each outgoing DSP request costs one STS call; each incoming
  one a presentation query (DEPLOYING, "Known limits").
- **Why it matters.** Cost per request (R-29).
- **Current state: Open.** ADR 0016 chose "measure before adding more
  caching", so a revocation takes effect at once. The 0.12 numbers are
  in R-29.
- **Next.** Decide from those numbers; code in ROADMAP 1.6 if needed.

<a id="td-10"></a>
### TD-10 Transfers: no token renewal; whole sources and whole pushes only

- **What.** The data token of a pull is not renewed (DSP 2025-1 has no
  message for it): the consumer asks for a new transfer. A start again
  by the consumer keeps the same token (ADR 0014). Only `GET` of the
  whole source ([ADR 0013](adr/0013-transfer-and-data-plane.md)); a
  push is sent whole, in one request, and again from the beginning
  after a start again. No other way to move data, such as Kafka.
- **Why it matters.** Long or large transfers, and dataspaces that use
  other kinds of transfer (Simpl-Open documents S3 push and Kafka).
- **Current state: Mitigated (1.1).** Push over HTTP into the
  consumer's S3-compatible object storage, and pull from the provider's
  object storage (`s3://` sources), are built
  ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)): push
  follows the Data Plane Signaling HTTP Transfer Profile
  (`HttpData-PUSH`, `push.rs`), and as consumer the connector receives
  it at `/push/{consumerPid}` (`receiving.rs`), so the consumer side
  now has a data plane when push is used. Measured in 0.12
  ([BENCHMARKS](BENCHMARKS.md), "Data plane"): a download still running
  when its token ends is not cut; a new request with the old token gets
  401. So a long pull works, and a pull that must start again after the
  token's life does not.
- **Next.** No release named. Token renewal waits for a DSP or Signaling
  message for it. Assigned to 1.7 (maintainer, 2026-11-26).

<a id="td-11"></a>
### TD-11 The provider cannot suspend a transfer from the management API

- **What.** As provider, the management API lists, reads and
  terminates transfers (`/v1/provided-transfers`, 1.1), but does not
  suspend, start again or complete them (ADR 0013; suspend left out of
  1.1 by the maintainer, ADR 0027, section 7). Until 1.4 the title also
  named the consumer's refusal of offers with credential constraints;
  1.4 fixed that part.
- **Why it matters.** The domain supports these steps, but operators
  cannot use them.
- **Current state: Open.** Done in 1.1: the consumer
  suspends a transfer and starts it again from the management API
  (`POST /v1/transfers/{consumerPid}/suspension` and `/start`,
  [ADR 0014](adr/0014-transfers-as-consumer.md), amended in 1.1). Done
  in 1.4: the consumer accepts an offer with credential constraints it
  understands, without evaluating them, as Eclipse EDC
  (`portitor_dsp::policy::consumer_may_accept`); its `dateTime`
  constraints are still evaluated, and its acceptance's token allows the
  scopes the provider needs.
- **Next.** Suspend as provider, in 1.6 (maintainer, 2026-11-28): the
  domain already supports it.

<a id="td-12"></a>
### TD-12 ODRL: few left operands; prohibitions and duties not enforced

- **What.** Only `dateTime`, credential claims and, since 1.4,
  `purpose` are understood; other left operands fail closed and stop the
  offer. Prohibitions and duties
  are carried into agreements but not enforced. Terms are compared as
  written, so a counter-offer is terminated
  ([ADR 0009](adr/0009-minimal-odrl-policies.md),
  [ADR 0019](adr/0019-credential-constraints.md)).
- **Why it matters.** Offers with `spatial` and similar left operands
  cannot be published; usage control stops at the agreement.
- **Current state: Open.** Fail closed: nothing not
  understood is granted. Done in 1.4: `purpose` with `eq` or `isAnyOf`,
  a term of the agreement that always holds; a request with another
  purpose is refused as other terms
  ([ADR 0009](adr/0009-minimal-odrl-policies.md), amendment in 1.4).
  Prohibitions and duties are still not enforced.
- **Next.** Other left operands (such as `spatial`), prohibitions and
  duties, in 1.10 (maintainer, 2026-11-28), after a check of what the
  EHDS data permit and HealthData@EU use; they matter only once a permit
  or a dataspace needs a duty.

<a id="td-14"></a>
### TD-14 Credential schemas, presentation definitions and issuance policies are not supported

- **What.** `credentialSchema` is published but not checked; a
  presentation query with only a `presentationDefinition` gets an error
  (ADR 0015; the DCP TCK does not test it); the issuer has no approval
  queue and no "present another credential first" (`issuancePolicy`,
  [ADR 0018](adr/0018-issuer-component.md)).
- **Why it matters.** Dataspaces that use them cannot use Portitor as
  it is.
- **Current state: Open.**
- **Next.** ROADMAP 1.6, if a target dataspace needs it; the issuer's
  decision port allows it.

<a id="td-15"></a>
### TD-15 Agreements are not revised when the data's scope changes

- **What.** An agreement keeps working after its dataset leaves the
  source; there is no way to revise it when a data subject objects or
  the consumer asks for a new scope (maintainer, 2026-11-14;
  [ADR 0020](adr/0020-dataset-harvest.md)).
- **Why it matters.** Consent withdrawal and scope changes.
- **Current state: Open.**
- **Next.** ROADMAP 1.7, with consent, if the consent ADR finds a
  standard.

<a id="td-16"></a>
### TD-16 One harvest source; no dataspace registry; other catalogs not checked; no bundle expiry or bundle key rotation

- **What.** Datasets come only from the datasets file
  ([ADR 0020](adr/0020-dataset-harvest.md)); a dataspace wide registry
  or federated catalogue is "only if needed"; other participants'
  catalogs are not checked against the bundle; the bundle has no expiry
  and its key no rotation
  ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)).
- **Why it matters.** Organisations keep datasets in DCAT-AP endpoints
  or CKAN; see also R-22.
- **Current state: Open.** The `DatasetSource` port takes new adapters
  without other changes.
- **Next.** ROADMAP 1.8. Assigned to 1.8 (maintainer, 2026-11-26).

<a id="td-17"></a>
### TD-17 The data plane runs inside the connector process

- **What.** Data traffic and protocol logic share CPU, memory and
  network exposure; they cannot be scaled apart, and a data handling
  bug can take negotiation down
  ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate)).
  The Signaling messages match the v1.0-RC4 schemas, but the ports are
  Rust traits inside the process: no Signaling endpoint is served over
  HTTP (`/dataflows/...`, `/transfers/{id}/dataflow/...`,
  `PUT /dataplanes`), so no outside data plane can be plugged in
  ([review](reviews/2026-11-18-specifications-and-code-quality.md),
  S1). The profile name
  `https://w3id.org/dspace-sig/profile/http-pull` was not confirmed
  against a registry.
- **Current state: Accepted** (maintainer, 2026-11-11). Signaling over
  HTTP is not in 1.0 (maintainer, 2026-11-20): the work is large, and
  it can be added later without a breaking change
  ([STABILITY](STABILITY.md#what-10-does-not-do)).
- **Next.** [ADR 0028](adr/0028-separate-pluggable-data-plane.md)
  (Accepted 2026-11-25), in four steps. Step 1, untie the built-in data
  plane inside the process, is done in 1.2: the data plane is the crate
  `portitor-dataplane`, still linked into the connector; it reads none
  of the control plane's stores, and asks it through ports (`AccessGate`
  for the membership check). Steps 2 to 4 (signaling over
  HTTP and registration, the separate image, interop with an outside
  data plane): later in 1.x, or after Data Plane Signaling 1.0 is
  released (R-06). Assigned to 1.12 (maintainer, 2026-11-26).

<a id="td-19"></a>
### TD-19 No TLS inside the services

- **What.** The services speak plain HTTP behind a TLS-terminating
  reverse proxy; the management ports stay on a private network.
- **Current state: Accepted** (maintainer, 2026-11-20,
  [ADR 0026](adr/0026-tls-termination.md)): TLS stays the operator's
  job.
- **Next.** None. Optional TLS in the services would be additive; it is
  not planned. Assigned to 1.9 (maintainer, 2026-11-26).

<a id="td-21"></a>
### TD-21 Gaia-X beyond 0.8

- **What.** Not built (ADR 0021; maintainer, 2026-11-15): a run against
  the Gaia-X Lab or a production clearing house (R-12); Gaia-X DCAT and
  service offering credentials with a federated catalogue; OpenID4VCI
  and OpenID4VP (R-14); label levels 1 to 3; calling the registration
  number notary (the operator gets that credential by hand); trust
  anchors and compliance issuers read from the Gaia-X Registry; Gaia-X
  with EDC (R-10); a legal person credential signed by another signer
  the participant chooses
  ([review](reviews/2026-11-18-specifications-and-code-quality.md),
  G2).
- **Current state: Open.**
- **Next.** ROADMAP 1.13, Gaia-X beyond 0.8.
  Assigned to 1.13 (maintainer, 2026-11-26).

<a id="td-22"></a>
### TD-22 Migrations run forward only; a rollback needs the backup

- **What.** An older version refuses a database a newer one migrated;
  going back means restoring the backup, and data written since is lost
  (OPERATING, "Upgrade and rollback").
- **Current state: Accepted.** Confirmed at the 0.13 freeze
  (maintainer, 2026-11-20): migrations stay forward only in 1.x
  ([STABILITY](STABILITY.md#database)).
- **Next.** None.

<a id="td-28"></a>
### TD-28 No trace across participants

- **What.** Trace context stays inside one participant: DSP and DCP
  calls to other participants carry no `traceparent`, and one that
  arrives on the DSP port is ignored
  ([ADR 0023](adr/0023-observability.md), section 4).
- **Why it matters.** One negotiation between two organisations shows
  as two traces; an operator matches them by hand.
- **Current state: Accepted**, by design: a trace id from another
  organisation is untrusted input, and ours would link our internal
  calls to theirs. Both sides log the DSP process ids, which is enough
  to match their logs.
- **Next.** None.

<a id="td-29"></a>
### TD-29 No OTLP logs export, no gRPC, no compression

- **What.** Portitor exports traces and metrics over OTLP/HTTP with
  Protocol Buffers only. Logs stay JSON lines on stdout, there is no
  gRPC exporter, and `OTEL_EXPORTER_OTLP_COMPRESSION` only gets an SDK
  warning (ADR 0023; OPERATING, "Traces and metrics").
- **Why it matters.** Operators whose platform takes only OTLP logs or
  gRPC need a collector in between.
- **Current state: Open.** Kept out to save binary size and
  dependencies.
- **Next.** OTLP logs export, gRPC and compression.
  Assigned to 1.14 (maintainer, 2026-11-26).

<a id="td-31"></a>
### TD-31 No audit trail of management actions

- **What.** The connector logs every state change of a negotiation or
  transfer with its process ids
  (`crates/portitor-connector/src/state_events.rs`) and keeps each
  agreement with the counter-party's DID. It does not keep the caller's
  token or presentation with an agreement, and does not log failed
  management key checks or some management changes (for example the
  identity's `DELETE /v1/credentials/{id}`). A management call has no
  person behind it ([TD-04](#td-04); SECURITY-ASSURANCE, F-02).
- **Why it matters.** A provider cannot show who pulled which data and
  when. Health dataspaces are likely to require this (EHDS; not
  checked which article).
- **Current state: Mitigated (1.1).** The first half of
  [ADR 0025](adr/0025-users-and-roles.md)'s plan is built: each pull,
  push sent and push received writes one INFO line, `data transfer
  done` (target `portitor::audit`), with `flow`, `agreement`,
  `counterParty`, `bytes` and `outcome`
  (`crates/portitor-dataplane/src/audit.rs`;
  [ADR 0027](adr/0027-push-transfers-and-object-storage.md), section 6;
  [OPERATING](OPERATING.md#traces-and-metrics)). Portitor keeps no audit
  store: the operator ships and keeps the log. DSP 2025-1 does not sign
  its messages, so evidence between participants rests on the DCP
  tokens and on each side's own records.
- **Next.** Audit records that name a person, in ROADMAP 1.7 (ADR 0025).

<a id="td-35"></a>
### TD-35 No negotiation started by the provider

- **What.** DSP 2025-1 lets a provider start a negotiation with an
  offer; the consumer serves `POST /negotiations/offers` for it.
  Portitor's consumer callbacks have only the paths with a
  `consumerPid` (`crates/portitor-connector/src/lib.rs`), and Portitor
  as provider never starts with an offer
  ([review](reviews/2026-11-18-specifications-and-code-quality.md),
  D2). The DSP TCK passes.
- **Why it matters.** A provider that works this way cannot negotiate
  with Portitor. EDC starts negotiations from the consumer, so the risk
  today is small.
- **Current state: Open.** Not in 1.0 (maintainer, 2026-11-20): the
  work is large, and it can be added later without a breaking change
  ([STABILITY](STABILITY.md#what-10-does-not-do); DEPLOYING, "Known
  limits").
- **Next.** A negotiation started by the provider, in 1.9 with the
  Simpl-Open tests (maintainer, 2026-11-28; it was assigned to 1.5 on
  2026-11-26): no partner needs it today, and 1.9 shows whether
  Simpl-Open does.

<a id="td-39"></a>
### TD-39 DEPLOYING is not in deployment order; the docs are long

- **What.** DEPLOYING explains the connector first, though the
  deployment order is issuer, identity, connector. The docs were about
  10,700 lines, and this file listed more than 80 items without saying
  which few matter most
  ([review](reviews/2026-11-18-specifications-and-code-quality.md), Q2,
  Q7). There is a Compose example and no Helm chart.
- **Why it matters.** The learning curve to deploy and troubleshoot
  should be as small as possible (maintainer, 2026-11-18).
- **Current state: Mitigated.** Done in 0.14: DEPLOYING is in
  deployment order (issuer, identity, connector) and starts with a
  ["First deployment"](DEPLOYING.md#first-deployment) path; what an
  operator does after the first day (health, logs, metrics, backup, key
  rotation, upgrades) is its own guide, [OPERATING](OPERATING.md); the
  README no longer repeats the routes and settings. This file has a
  "Top risks" box and closed items are one line each; the roadmap's
  released sections are one table; ARCHITECTURE describes the system as
  it is; [docs/adr/README.md](adr/README.md) indexes the ADRs.
- **Decision (maintainer, 2026-11-18).** DEPLOYING is written for an
  operations team; running the demo on a laptop belongs in TESTING.
- **Next.** A Helm chart, in
  [1.14](ROADMAP.md#114-operations-hardening-added-2026-11-26) (moved
  from 1.4 by the maintainer, 2026-11-27). Until then there is the
  Compose example.

<a id="td-40"></a>
### TD-40 Database calls block the async runtime

- **What.** Each store is one SQLite connection behind one lock, with
  `synchronous=FULL` (`crates/portitor-runtime/src/sqlite.rs`), and the
  HTTP handlers call it directly. Only the harvest and the health
  checks run on blocking threads
  ([review](reviews/2026-11-18-specifications-and-code-quality.md),
  Q3).
- **Why it matters.** Under load every request thread can wait for one
  write to reach the disk. It shows as "slow, no error". Related:
  [TD-08](#td-08).
- **Current state: Accepted (0.12, measured).** The rule was set before
  measuring (maintainer, 2026-11-18): move the calls to blocking
  threads if, with 100 pulls running and 1 negotiation per second, the
  data plane's p99 time to first byte more than doubles or passes
  100 ms. Measured ([BENCHMARKS](BENCHMARKS.md), "TD-40"): 9.2 ms
  alone, 8.5 ms with 1 negotiation per second, 14.5 ms with 260 per
  second. The rule was not met. The lock and the disk limit
  negotiations (p99 92 ms at 260 per second), not pulls.
- **Next.** Measure again in each release; look again if the p99 passes
  100 ms. From 1.3 the store ports stay synchronous on PostgreSQL too:
  a call waits on its thread while a database thread drives the
  connection, so the wait includes a network round trip
  ([ADR 0029](adr/0029-several-replicas.md), decision 3). Asynchronous
  store ports are in ROADMAP 1.12.
  Assigned to 1.12 (maintainer, 2026-11-26).

<a id="td-45"></a>
### TD-45 Each running pull holds about 0.8 MB of memory

- **What.** With 1,000 pulls running at once the provider used 923 MB
  ([BENCHMARKS](BENCHMARKS.md), "Data plane"). One pull of 10 GiB stays
  flat at 19 MB, so the memory goes with the number of pulls, not their
  size. Each pull also needs two open files.
- **Why it matters.** The data plane is where the load is expected
  (maintainer, 2026-11-18). An operator must size memory and the open
  files limit for the number of pulls at once.
- **Current state: Open.** DEPLOYING "Known limits" gives the numbers.
- **Next.** Find where the memory goes (buffers of the proxy, of the
  HTTP client, of TLS) before changing anything; measure again in each
  release. Assigned to 1.7 (maintainer, 2026-11-26).

<a id="td-49"></a>
### TD-49 After a failed message, the two parties of a negotiation can stay in different states

- **What.** When the other party cannot be reached through all tries,
  Portitor ends the negotiation on its side; when its termination gets
  no answer, it stops trying (DSP: "MAY choose to ignore the error").
  The other party may never hear of it and wait for ever, for example a
  consumer in `VERIFIED` while the provider is `TERMINATED`. DSP 2025-1
  says: "In case the states differ, the Contract Negotiation MUST be
  terminated." Nothing finds such a difference later
  ([negotiation spike](spikes/1.x-negotiation-state-machine.md), G3).
- **Why it matters.** Negotiations that will never move stay in the
  file and in the management API. No data flows: the provider serves
  data only for a `FINALIZED` negotiation.
- **Current state: Accepted** (maintainer, 2026-11-26): keep the
  behaviour as in Eclipse EDC 1.0.0-RC8, which gives up the same way
  and never asks the other party for a negotiation's state.
- **Next.** None. An operator who sees a negotiation that does not
  move can ask the partner, and terminate it from its side.

<a id="td-59"></a>
### TD-59 SQLite is deprecated; no tool copies a SQLite file into PostgreSQL

- **What.** From 1.3 each image runs on PostgreSQL or on a SQLite file
  ([ADR 0029](adr/0029-several-replicas.md)). SQLite is deprecated and
  removed in 2.0, with `PORTITOR_DB_FILE`. Until then each image keeps
  two migration lists, one per database, and every store test runs on
  both.
- **Why it matters.** A deployment on SQLite cannot move to PostgreSQL
  without losing its state: no tool copies the data
  ([OPERATING](OPERATING.md#from-sqlite-to-postgresql)). Two schemas
  can drift apart.
- **Current state: Accepted** (maintainer, 2026-11-26: deprecated in
  1.3, not removed, because [STABILITY](STABILITY.md#database) keeps
  `PORTITOR_DB_FILE` for 1.x). A test compares the two schemas.
  The copy tool planned for 1.4 is dropped (maintainer, 2026-11-27):
  no deployment needs one yet, so no data has to be copied.
- **Next.** SQLite and `PORTITOR_DB_FILE` removed in 2.0
  ([ROADMAP, For 2.0](ROADMAP.md#for-20)).

<a id="td-62"></a>
### TD-62 Why and when a negotiation ended is not stored

- **What.** The `code` and `reason` of a termination, sent or received,
  are only in the log; negotiations have no creation time and no time
  of the last change ([spike](spikes/1.x-negotiation-state-machine.md), G5 and G6).
- **Why it matters.** An operator cannot see why a negotiation ended,
  find negotiations stuck for days, or remove terminated ones by age.
  EDC keeps `error_detail`, `created_at`, `updated_at` and
  `state_timestamp`.
- **Current state: Open.**
- **Next.** One migration: the termination's `code` and `reason`, shown
  in the management API, and two time columns set by the adapter.
  Assigned to 1.5 (maintainer, 2026-11-26).

<a id="td-63"></a>
### TD-63 Source files above 1,000 lines

- **What.** `auth/dcp.rs`, the issuer's and the identity's `store.rs`,
  `dcp/bundle.rs` and `runtime/http.rs` are above 1,000 lines, tests
  not counted ([2026-11-18 review](reviews/2026-11-18-specifications-and-code-quality.md)).
  The 1.3 readability review (2026-11-26) adds two: the connector's
  `lib.rs` (about 1,100 lines) and the issuer's `store.rs`, which grew
  from about 940 to about 1,260 lines with PostgreSQL. It also found
  two long functions, to split with them: `BundleHarvester::start`
  (`crates/portitor-connector/src/bundle_harvest.rs`, about 90 lines)
  and `Pool::migrate`
  (`crates/portitor-runtime/src/database/postgres.rs`, one block of
  about 60 lines). The 1.4 readability review (2026-11-27) measured
  again, tests not counted: the connector's `lib.rs` about 1,220
  lines, `auth/dcp.rs` about 1,050, the issuer's `store.rs` about
  1,250, `dcp/bundle.rs` about 1,150, `dsp/transfer/provider.rs` about
  1,130 and `database/postgres.rs` 1,000; the connector's `auth.rs`,
  `management.rs` and `store.rs`, and the identity's `store.rs`, are
  just under 1,000. In the split, the catalog handlers of `lib.rs`
  (`catalog_request`, `dataset`, `caller_credentials`) could move to a
  catalog module of the connector.
- **Why it matters.** Long files and functions are hard to read for the
  maintainer ([R-01](#r-01)).
- **Current state: Open.**
- **Next.** Split each by topic, no change of behaviour. Assigned to
  1.5 (maintainer, 2026-11-26).

<a id="td-64"></a>
### TD-64 Too many type parameters; macros that write types

- **What.** Services carry many type parameters (today
  `ProviderService<S, D, C, E>`), and some macros write types
  ([2026-11-18 review](reviews/2026-11-18-specifications-and-code-quality.md)).
- **Why it matters.** Both make the code hard to read ([R-01](#r-01)).
- **Current state: Open.**
- **Next.** Fewer type parameters; each macro that writes types
  replaced by plain code or explained with its output in RUST-IDIOMS.
  Assigned to 1.6 (maintainer, 2026-11-26).

<a id="td-65"></a>
### TD-65 `Negotiator` does more than negotiate

- **What.** `Negotiator` runs negotiations, transfers and the harvest
  ([2026-11-18 review](reviews/2026-11-18-specifications-and-code-quality.md)).
- **Why it matters.** The name misleads a reader ([R-01](#r-01)).
- **Current state: Open.**
- **Next.** A name that fits, or a split. Assigned to 1.7 (maintainer,
  2026-11-26).

<a id="td-66"></a>
### TD-66 A data plane step runs inside the request that asks for it

- **What.** When the connector starts, suspends or ends a flow, it
  calls the data plane inside the request and waits for it. The
  "fully asynchronous data plane step" option of 2026-11-26 is not
  built: the connector would record the step and a worker would run it;
  a refusal would become a termination of the transfer.
- **Why it matters.** [ADR 0028](adr/0028-separate-pluggable-data-plane.md)'s
  data plane over HTTP (step 2) waits on the network, which a request
  should not do.
- **Current state: Open.**
- **Next.** Built with steps 2 to 4 of ADR 0028 ([TD-17](#td-17)).
  Assigned to 1.12 (maintainer, 2026-11-26).

<a id="td-67"></a>
### TD-67 Datasets are harvested from one kind of source only

- **What.** The harvest reads one datasets file
  ([ADR 0020](adr/0020-dataset-harvest.md)); a DCAT-AP endpoint or
  CKAN cannot be a source.
- **Why it matters.** A data holder with a catalogue must convert it
  first.
- **Current state: Open.** Which catalogues to support is to be decided
  (maintainer, 2026-11-14).
- **Next.** Assigned to 1.8, the EHDS data holder release (maintainer,
  2026-11-26).

<a id="td-68"></a>
### TD-68 No dataspace-wide registry or federated catalogue

- **What.** No component pulls the participants' DSP catalogs, checks
  them with the bundle and offers search.
- **Why it matters.** A consumer must know each provider to find data.
- **Current state: Open.** Only if needed (maintainer, 2026-11-14),
  with its own ADR.
- **Next.** Assigned to 1.8, only if that release needs it (maintainer,
  2026-11-26).

<a id="td-69"></a>
### TD-69 No Simpl-Open features beyond the protocols

- **What.** Simpl-Open offers mutual TLS between participants, consent
  enforcement and a configurable API gateway; none is part of DSP, DCP
  or Data Plane Signaling, and Portitor has none
  ([ADR 0017](adr/0017-simpl-open.md)).
- **Why it matters.** They add security, and consent is needed for
  health data.
- **Current state: Open.**
- **Next.** Consent first, with [TD-15](#td-15) in 1.7; an ADR on
  mutual TLS and the gateway in 1.9 with the Simpl-Open tests
  (maintainer, 2026-11-26).

<a id="td-72"></a>
### TD-72 Two error types for the database, converted both ways

- **What.** The database returns `DbError`
  (`crates/portitor-runtime/src/database.rs`); each store port returns
  its `StoreError`. A store converts `DbError` to `StoreError` (with
  `From`, or `map_err(err)` in the connector), and inside a transaction
  it converts its own `StoreError` back to `DbError`
  (`map_err(|e| DbError::new(e.0))`), because the transaction's closure
  must return `DbError` so that a collision on PostgreSQL is seen and
  the transaction run again (1.3 readability review, 2026-11-26).
- **Why it matters.** A reader must follow the conversions to see which
  error comes out, and the SQLSTATE code of a `DbError` is lost in a
  `StoreError` ([R-01](#r-01)).
- **Current state: Open.**
- **Next.** One error type that keeps the code, or a transaction that
  takes the store's error type; no change of behaviour. It touches
  every store file, so it is done with their split in 1.5
  ([TD-63](#td-63)) and is not a large item of its own (maintainer,
  2026-11-28).

<a id="td-73"></a>
### TD-73 A negotiation step is about 3.5 ms slower on SQLite in 1.3

- **What.** The 1.3 benchmark ([BENCHMARKS](BENCHMARKS.md#13-compared-with-12-on-sqlite-and-on-postgresql),
  1.2.0 against the 1.3 branch on one machine, 2026-11-27) shows a new
  negotiation at p50 14.7 ms on SQLite against 11.3 ms in 1.2 (+30 % to
  +46 % over three rounds), and filling 2,000 negotiations 38 % slower.
  The cause is not measured yet; the likely one is the extra writes of
  1.3 for each message (the outbox claim and the lease rows, for several
  replicas).
- **Why it matters.** It is a few milliseconds for each step, on a
  storage that is deprecated; data throughput and the TD-40 check are
  unchanged. On PostgreSQL the same steps take about twice as long
  again, because each write is a network round trip.
- **Current state: Open; cause found in 1.4, moved to 1.12 with
  [TD-40](#td-40).** Counted on 2026-11-27 (1.4 branch, SQLite, the
  connector test `negotiate_transfer_fetch_and_complete` with every
  statement logged by a temporary counter, not committed), and compared
  with the 1.2.0 store code (`git show v1.2.0:...`). For one whole
  negotiation, the provider's connector ran 8 writes and 8 reads, and
  no explicit transaction: every write is its own commit, so one sync
  of the WAL with `synchronous=FULL`.

  | Provider, one negotiation (request, verification) | 1.2.0 | 1.3 / 1.4 |
  |---|---|---|
  | Negotiation row: insert or update when a message arrives, update when ours was sent | 4 writes | 4 writes (the same statements) |
  | Lease on each message sent (TD-53): claim, then release | none | 4 writes |
  | Used token id of each DCP token received (TD-09): delete expired, insert | none (in memory) | up to 4 writes |
  | Reads of the negotiation | 4 | 4 |
  | Reads of the dataset (ADR 0020: from the database since 1.3) | none (copy in memory) | 2 |

  So a provider step went from 2 commits to up to 6 (4 without DCP);
  each costs about one disk sync, which fits the 3.5 ms. The consumer's
  side and the identities (used token ids of presentation queries)
  gained the same kinds of writes. None of the new writes can go
  without changing what is stored or the replica rules: the lease and
  the used token id are what make two replicas safe (ADR 0029), and
  deleting expired token ids less often keeps old rows longer. Ways to
  make them cheaper, each a design change for the maintainer: write
  the lease claim in the same transaction as the step that saves the
  message (one commit instead of two); on SQLite, where one process
  per file is enforced (ADR 0029, D3), keep message leases in memory;
  delete expired token ids in the scheduled loop instead of before
  each insert.
- **Next.** ROADMAP 1.12, with [TD-40](#td-40) (asynchronous store
  ports): choose one of the ways above and measure again with
  `scripts/bench.sh` (decided by the maintainer on 2026-11-27: not
  fixed in 1.4).

<a id="td-74"></a>
### TD-74 An issuer not limited by type is trusted for every credential type

- **What.** 1.4 adds `PORTITOR_TRUSTED_ISSUER_TYPES`, which limits a
  trusted issuer to a list of credential types ([R-48](#r-48)). An
  issuer of `PORTITOR_TRUSTED_ISSUERS` that the new setting does not
  name keeps every type, as in 1.3 and as EDC's `supportedtypes` with
  `"*"`. The safer default trusts such an issuer for
  `MembershipCredential` only (option B of the 1.4 plan, maintainer,
  2026-11-27).
- **Why it matters.** An operator who adds a second issuer and forgets
  the new setting trusts it for memberships too.
- **Current state: Accepted.** The default cannot change in 1.x: a 1.3
  deployment must start and behave unchanged ([STABILITY](STABILITY.md)).
- **Next.** Change the default in 2.0 ([ROADMAP, For 2.0](ROADMAP.md#for-20)).

<a id="td-75"></a>
### TD-75 A client constructor called `new` does not apply the block on internal addresses

- **What.** The outbound clients have two constructors: `new`, for
  URLs the operator chose, without the block on internal addresses,
  and `for_partners`, with it ([R-17](#r-17)): `HttpSender`
  (`portitor-runtime/src/http.rs`), `DidWebResolver` (`did.rs`),
  `WebDirectory` (`directory.rs`), `HttpIssuerClient`
  (`portitor-identity/src/issuance.rs`) and `DcpVerifier`
  (`portitor-connector/src/auth/dcp.rs`). Found by the 1.4 readability
  review (2026-11-27).
- **Why it matters.** `new` reads as the usual way to build a value. A
  future client for URLs a partner chooses, built with `::new`, gets no
  block, and nothing in the name warns the reader ([R-01](#r-01)).
- **Current state: Open.** Every partner client of 1.4 is built with
  `for_partners`.
- **Next.** Rename to `for_operator_urls` and `for_partner_urls`, with
  one private build function each. No change of behaviour. In 1.5,
  with [TD-63](#td-63) (maintainer, 2026-11-28).

<a id="td-76"></a>
### TD-76 A refused DSP message is logged as "counter-party not reachable" and its reason is lost; a refused transfer request is not logged by the provider

- **What.** When a partner answers a message of our outbox with a 4xx,
  the outbox logs it with the message for a partner that did not answer
  ("counter-party not reachable; terminating", `log_unanswered` in
  `crates/portitor-connector/src/outbox.rs`). The answer's body, which
  holds the partner's reason, is not read (`error_body_bytes: 0` in
  `portitor-runtime/src/http.rs`). As provider, a transfer request
  refused because the caller's credentials no longer meet the
  agreement's policy writes no log line; only a request that names no
  finalized agreement is logged (`log_no_finalized_agreement` in
  `crates/portitor-connector/src/transfer.rs`).
  Found in the 1.4 run of the data-permits scenario, step 4
  ([scenario changelog](../scenarios/data-permits/CHANGELOG.md)).
- **Why it matters.** An operator who reads the log looks for a network
  problem when the partner refused, and neither side's log says why.
- **Current state: Open.**
- **Next.** Log a 4xx answer as a refusal, with the start of its body;
  log the provider's refusal of a transfer request. In 1.5
  (maintainer, 2026-11-28).

<a id="td-77"></a>
### TD-77 The identity answers 400 when an issuer's metadata cannot be read

- **What.** `POST /v1/credential-requests` on the identity's management
  API answers 400 when the issuer's DID document or metadata cannot be
  read: `check_request` maps that error to `AskError::Refused`
  (`crates/portitor-identity/src/issuance.rs`), which `ask_error` answers
  with 400 (`management.rs`). Seen in the 1.4 run of the scenarios,
  when a container could not resolve the issuer's name
  ([TESTING](TESTING.md#docker-or-podman)).
- **Why it matters.** 400 says the request is wrong, so a script or an
  operator does not try again; the issuer could not be reached. 502 may
  fit, as for `AskError::Issuer`.
- **Current state: Open.** The detail of the answer says what failed.
- **Next.** Answer 502 when the issuer cannot be reached or read. A
  change of a status code on a frozen route: check
  [STABILITY](STABILITY.md) first: 502 is already listed for
  `POST /v1/credential-requests` in
  `docs/api/identity-management-openapi.json`. In 1.5 (maintainer,
  2026-11-28).

<a id="td-78"></a>
### TD-78 The catalog holds every dataset's access policy during a request

- **What.** On a catalog request the connector holds the access policy
  of every dataset in memory while it builds the answer. The 1.4
  benchmark measured the provider's peak memory at 203 MB against
  108 MB for 10,000 datasets, each with an access policy
  ([BENCHMARKS](BENCHMARKS.md#14-the-catalog-with-access-policies)).
- **Why it matters.** Memory grows with the number of datasets that
  have an access policy, for each catalog request at once.
- **Current state: Open.** Without access policies the catalog uses
  what it used in 1.3.0.
- **Next.** Check the policies one by one while the catalog is read
  (streaming), or share identical policies. In 1.5, time-boxed to one
  day; if it does not fit, it moves to 1.14 (maintainer, 2026-11-28).

<a id="td-79"></a>
### TD-79 The identity records a set status bit of an unknown purpose as SUSPENDED, and checks a REVOKED credential again

- **What.** The identity's status check
  (`crates/portitor-identity/src/status_check.rs`, [TD-20](#td-20))
  records a credential as SUSPENDED when a set bit is in a list whose
  purpose is neither `revocation` nor `suspension`, and its log says
  "suspended". It also checks a credential recorded REVOKED again on
  each round, so it becomes valid again if the issuer clears the bit.
- **Why it matters.** Not presenting such a credential is right: our
  verifier, EDC's verifier (`BaseRevocationListService.checkStatus`)
  and the W3C Bitstring Status List validation (step 13) all refuse a
  credential with any set bit, and DCP then refuses the whole
  presentation. But the stored name and the log mislead the operator.
  W3C says revocation "is not reversible", and EDC IdentityHub never
  checks a REVOKED credential again (`CredentialWatchdog.ALLOWED_STATES`,
  read at commit `aeb8a7d` on 2026-11-28).
- **Current state: Open.**
- **Next.** In 1.5 (maintainer, 2026-11-28, D6 and D7 of the 1.5
  plan): keep not presenting the credential, name the state and the
  log honestly, and never check a REVOKED credential again.

## Closed

One line each. "Where" is the record that describes the fix. Changelog
entries are in [CHANGELOG](../CHANGELOG.md) under the release named.

| ID | Title | Closed in | Where the fix is described |
|---|---|---|---|
| <a id="r-07"></a>R-07 | Upstream findings not reported yet | 0.10 | [Upstream reports](#upstream-reports) below: 12 filed, 1 not filed, 1 on hold; one more finding was dropped because a third party had already filed it |
| <a id="r-15"></a>R-15 | Onboarding accepted a compliance answer from any issuer whose signature verified | 0.8 | [ADR 0021](adr/0021-gaia-x-onboarding.md) (`PORTITOR_GAIAX_COMPLIANCE_DID`); test `an_answer_from_another_service_is_refused` in `crates/portitor-identity/tests/gaiax.rs` |
| <a id="r-18"></a>R-18 | No request size limit of Portitor's own on the public ports | 0.11 | CHANGELOG 0.11.0; [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md); `crates/portitor-runtime/src/request_size.rs` |
| <a id="r-26"></a>R-26 | ODRL `dateTime` definition read from memory | 0.13 | [ADR 0009](adr/0009-minimal-odrl-policies.md), note of 2026-11-20 |
| <a id="r-28"></a>R-28 | 0.8 Gaia-X code had no fuzz target | 0.8 | Fuzz target `gaiax_compliance`; [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md), "Parsers and fuzz targets" |
| <a id="r-30"></a>R-30 | DSP callers were not authenticated | 0.5 | [ADR 0016](adr/0016-dsp-caller-authentication.md) |
| <a id="r-31"></a>R-31 | RSA timing advisory in the pure-Rust `rsa` crate | 0.0 | [ADR 0002](adr/0002-technology-stack.md): PS256 is verified with `ring`; no `rsa` crate in `Cargo.lock` |
| <a id="r-32"></a>R-32 | XML parser advisories through RDF/XML code lists and rudof | 0.7 | [ADR 0005](adr/0005-schema-and-vocabulary-registry.md): no binary parses RDF/XML; code lists travel as N-Triples |
| <a id="r-33"></a>R-33 | Any caller, or one participant, could exhaust the issuer | 0.6 | [ADR 0018](adr/0018-issuer-component.md), "Amended during the 0.6 review" |
| <a id="r-34"></a>R-34 | Scorecard Branch-Protection and Fuzzing at 0 | 0.2 | The `main` ruleset targeted no branch (fixed); cargo-fuzz with ClusterFuzzLite; Scorecard 7.9 ([ROADMAP](ROADMAP.md#released), row 0.2) |
| <a id="r-35"></a>R-35 | The DCP TCK uses VCDM 1.1, Gaia-X needs VCDM 2.0 | 0.5 | [ADR 0015](adr/0015-dcp-identity-component.md): both DCP profiles |
| <a id="r-38"></a>R-38 | No specification names an S3 data address | 1.1 | [ADR 0027](adr/0027-push-transfers-and-object-storage.md): no S3 address crosses between participants. Push follows the Data Plane Signaling HTTP Transfer Profile (`endpointType` `https://w3id.org/dspace-sig/profile/http-push`, a bearer token, `HttpData-PUSH`); each participant reaches its own object storage with the AWS SDK settings |
| <a id="td-06"></a>TD-06 | Status lists were not cached in the connector's DCP verifier | 0.11 | [ADR 0016](adr/0016-dsp-caller-authentication.md), amended; DEPLOYING, "Status list cache" |
| <a id="td-18"></a>TD-18 | Operations: no metrics, traces or readiness check | 0.9 | [ADR 0023](adr/0023-observability.md) |
| <a id="td-23"></a>TD-23 | ADRs 0021 and 0022 were built but still Proposed | 0.8 | Both accepted on 2026-11-15 ([ADR index](adr/README.md)) |
| <a id="td-24"></a>TD-24 | A 3 second wait for acknowledgements; messages lost at a restart | 0.3 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md) |
| <a id="td-25"></a>TD-25 | A test-only tool signed the demo credentials | 0.6 | [ADR 0018](adr/0018-issuer-component.md): `portitor-issuer` replaced it |
| <a id="td-26"></a>TD-26 | One bad dataset stopped the connector at start | 0.7 | [ADR 0020](adr/0020-dataset-harvest.md) |
| <a id="td-27"></a>TD-27 | The domain crates' I/O check named too few crates | 0.6 (2026-11-14) | [ADR 0004](adr/0004-hexagonal-architecture.md), amended; `.github/workflows/ci.yml` |
| <a id="td-30"></a>TD-30 | The outbox resent a message the counter-party refused | 0.11 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), amended in 0.11 |
| <a id="td-32"></a>TD-32 | Policy terms that were not understood were dropped without an error | 0.12 | [ADR 0009](adr/0009-minimal-odrl-policies.md), amended 2026-11-18 |
| <a id="td-33"></a>TD-33 | The issuer's metadata did not pass the DCP JSON schema | 0.12 | [ADR 0018](adr/0018-issuer-component.md), amended 2026-11-18; OPERATING, "From 0.11 to 0.12" |
| <a id="td-34"></a>TD-34 | A repeated transfer request started a second transfer | 0.12 | [ADR 0013](adr/0013-transfer-and-data-plane.md), amended 2026-11-18 |
| <a id="td-36"></a>TD-36 | DSP details: who may terminate, 401 or 404, agreement timestamp, `consumerPid` check, catalog pages | 0.12; the answer limit in 0.13 | [ADR 0006](adr/0006-negotiation-storage.md) and [ADR 0016](adr/0016-dsp-caller-authentication.md), amended 2026-11-18; CHANGELOG 0.13.0 |
| <a id="td-37"></a>TD-37 | DCP details: `token` claim required, presentations as JWT strings only, no `/.well-known/dspace-trust` | 0.12 | [ADR 0016](adr/0016-dsp-caller-authentication.md), amended 2026-11-18 |
| <a id="td-38"></a>TD-38 | No management call to read a provider's catalog | 0.13; the `participantId` check in 0.14 | CHANGELOG 0.13.0 and 0.14.0; `docs/api/management-openapi.json`; [ARCHITECTURE](ARCHITECTURE.md#diagrams-c4-model) |
| <a id="td-41"></a>TD-41 | A panic stopped the whole process (`panic = "abort"`) | 0.12 | [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md), "What a panic does"; [ADR 0002](adr/0002-technology-stack.md), amended in 0.12 |
| <a id="td-42"></a>TD-42 | Four near-copies of the negotiation and transfer logic | 1.1 | The shared process core, `crates/portitor-dsp/src/process.rs` (`Record`, `Records`, `receive`, `update`); the in-memory stores moved to `crates/portitor-connector/src/test_stores.rs`, compiled for tests only; [READING-THE-CODE](READING-THE-CODE.md#the-three-siblings) |
| <a id="td-43"></a>TD-43 | The Gaia-X documents name several versions of the rules | 0.14 | [ARCHITECTURE](ARCHITECTURE.md#specification-versions) |
| <a id="td-44"></a>TD-44 | A transfer request sent at once after the agreement could be refused | 0.13; the metric in 0.14 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), "Amendment for 0.13" and "Note added in 0.14"; CHANGELOG 0.14.0 |
| <a id="td-48"></a>TD-48 | Data plane steps ran inside the retried update of a transfer | 1.2 | The write records the step (`transfer.flow_step`) and takes a lease on the transfer in the database in the same compare-and-update; the step runs once, right after the write wins; any other write while the lease holds gets HTTP 503 (`Retry-After: 1`); a step whose lease ended is taken over by the next call, a sweep or the start; a refused step takes the write back or ends the transfer: `crates/portitor-dsp/src/transfer/provider/flow_steps.rs`; [ADR 0028](adr/0028-separate-pluggable-data-plane.md), decision 6 and note added in 1.2; [ADR 0013](adr/0013-transfer-and-data-plane.md), note added in 1.2 |
| <a id="td-08"></a>TD-08 | One replica per SQLite file; no PostgreSQL adapter | 1.3 | [ADR 0029](adr/0029-several-replicas.md); the database layer `crates/portitor-runtime/src/database.rs` and `database/` (SQLite and PostgreSQL 16 to 19); `ConnectorStore` and the `DatabaseStore` of the identity and the issuer; a second process on a SQLite file refuses to start (`database/sqlite_lease.rs`); `scenarios/two-party/run-replicas.sh` |
| <a id="td-09"></a>TD-09 | Used token ids are kept in memory only | 1.3 | Table `used_token`, `crates/portitor-runtime/src/used_tokens.rs`; [ADR 0029](adr/0029-several-replicas.md), decision 6 |
| <a id="td-50"></a>TD-50 | Each connector process holds its own copy of the datasets | 1.3 | The catalog and each dataset are read from the database on each request: `StoredCatalog`, `crates/portitor-connector/src/harvest/catalog.rs`; [ADR 0029](adr/0029-several-replicas.md), decision 6 (D4, option B) |
| <a id="td-51"></a>TD-51 | The issuer's signed status lists live in the memory of one process | 1.3 | Table `status_list`, signed in the revoking transaction (`sign_status_list_if_needed`, `crates/portitor-issuer/src/store.rs`); [ADR 0029](adr/0029-several-replicas.md), decision 6 (D5) |
| <a id="td-52"></a>TD-52 | A cut of a running download or push reaches only its own process | 1.3 | `state_watchdog` in `crates/portitor-dataplane/src/download.rs` (every 5 s); [ADR 0029](adr/0029-several-replicas.md), decision 6 (D6) |
| <a id="td-53"></a>TD-53 | The DSP outbox is taken up again only at start, and retried in memory | 1.3 | Leases in the database (`portitor_runtime::leases`) and the outbox loop in every process (`Negotiator::schedule_outbox`); [ADR 0029](adr/0029-several-replicas.md), decision 6 |
| <a id="td-54"></a>TD-54 | Pushes are resumed only at start; one push or upload per transfer is guarded in one process | 1.3 | Leases `push` and `upload` of a transfer (`PUSH_LEASE`, `crates/portitor-connector/src/transfer.rs`; `HeldLease::lost`); [ADR 0029](adr/0029-several-replicas.md), decision 6 |
| <a id="td-55"></a>TD-55 | Each connector process holds its own copy of the rules bundle and its own cache folder | 1.3 | Table `bundle_in_use`, `crates/portitor-connector/src/store/bundle_in_use.rs`; `BundleHarvester::follow`; [ADR 0029](adr/0029-several-replicas.md), decision 6 (S3) |
| <a id="td-56"></a>TD-56 | Scheduled jobs run once in every container | 1.3 | Leases `job`/`dataset_harvest` and `job`/`gaiax_compliance`; job ids from the database; [ADR 0029](adr/0029-several-replicas.md), decision 6 |
| <a id="td-57"></a>TD-57 | The issuer's credential outbox does not claim a delivery before it sends it | 1.3 | `claim_due_delivery` in `crates/portitor-issuer/src/store.rs` (lease columns on the outbox row); [ADR 0029](adr/0029-several-replicas.md), decision 6 |
| <a id="td-58"></a>TD-58 | The DID document is built at start | 1.3 | No code: the two-step rolling key rotation in [OPERATING](OPERATING.md#rotating-a-key-with-several-replicas); [ADR 0029](adr/0029-several-replicas.md), decision 6 (D7) |
| <a id="td-47"></a>TD-47 | A transfer stayed REQUESTED when its provider was revoked | 1.1 | [ADR 0016](adr/0016-dsp-caller-authentication.md), amended in 1.1: a termination is accepted from the transfer's counter-party with a valid DCP token, even without a valid membership; tests in `crates/portitor-connector/tests/provider_control.rs`; the [hdab-intermediary](../scenarios/hdab-intermediary/README.md) scenario, step 6, expects `TERMINATED` and passed on 1.3 ([ROADMAP 1.3](ROADMAP.md#13-several-replicas-and-postgresql-released-2026-11-27-as-v130), "Tried before the tag"). Moved here on 2026-11-27 |
| <a id="r-48"></a>R-48 | An issuer trusted for data permits could also issue memberships | 1.4 | `PORTITOR_TRUSTED_ISSUER_TYPES` on the connector and the identity ([ADR 0019](adr/0019-credential-constraints.md), amendment in 1.4; `crates/portitor-dcp/src/trust.rs`); the [data-permits](../scenarios/data-permits/README.md#the-steps) scenario, step 5, is a refusal. The safer default waits for 2.0 ([TD-74](#td-74)) |
| <a id="td-13"></a>TD-13 | The catalog showed every offer to every member | 1.4 | `portitor:accessPolicy` per dataset, checked on catalog, dataset and contract requests ([ADR 0019](adr/0019-credential-constraints.md), amendment in 1.4; `crates/portitor-dsp/src/policy/access.rs`, `check_offer` in `negotiation/provider.rs`) |
| <a id="td-20"></a>TD-20 | A holder had to delete a revoked credential by hand after a reissue | 1.4 | The identity's status check, `crates/portitor-identity/src/status_check.rs`, with `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS` ([ADR 0029](adr/0029-several-replicas.md), note added in 1.4; [DEPLOYING](DEPLOYING.md#getting-a-credential)); tests in `crates/portitor-identity/tests/status_check.rs`; the [data-permits](../scenarios/data-permits/README.md#the-steps) scenario, step 4, passed on 1.4 without a deletion |
| <a id="td-46"></a>TD-46 | The issuer kept one set of claims per participant | 1.4 | `PUT /v1/participants/{did}` on the issuer's management API ([DEPLOYING](DEPLOYING.md#registering-participants-and-issuing)); the [data-permits](../scenarios/data-permits/README.md#the-steps) scenario, step 3 |
| <a id="td-60"></a>TD-60 | A repeated first contract request started a second negotiation | 1.4 | The index `negotiation_request` and `NegotiationStore::negotiation_requested_by` (`crates/portitor-dsp/src/negotiation/provider.rs`); CHANGELOG 1.4.0 |
| <a id="td-61"></a>TD-61 | The outbox's "it arrived before" check compared the state only | 1.4 | `same_process` in `crates/portitor-connector/src/outbox.rs`; CHANGELOG 1.4.0 |
| <a id="td-70"></a>TD-70 | Four loops that tick each wrote their own timer | 1.4 | `spawn_every` in `crates/portitor-runtime/src/panics.rs` |
| <a id="td-71"></a>TD-71 | The identity and the issuer each wrapped the database the same way | 1.4 | `ComponentStore` in `crates/portitor-runtime/src/component_store.rs` |

### Left over from closed items

What a closed item did not remove. Each line is a known limit, not a
plan.

- **R-07.** Follow the upstream answers and remove each workaround when
  upstream fixes it.
- **R-18.** Contract and transfer requests, offers, agreements,
  presentations and credentials take the port's limit (2 MiB by
  default): the specifications give them no size bound. The largest
  example of a small message in DSP 2025-1 and DCP v1.0.1 is 329 bytes;
  those routes are limited to 64 KiB.
- **R-26.** Portitor does not read a date and time without a time zone
  (ODRL: optional, "strongly recommended"); the maintainer decides
  whether it should. EDC has no built-in `odrl:dateTime` (its term is
  `edc:inForceDate`), so a time limit is not interoperable with EDC out
  of the box. The definition was read from the W3C working group's
  repository; compare it with the copy at w3.org when it can be
  reached.
- **R-38.** The HTTP Transfer Profile is a draft, not tagged, like Data
  Plane Signaling itself ([R-06](#r-06)). Portitor as consumer cannot
  receive a push from EDC, which has no production push data plane
  (ADR 0027). Pull keeps the `endpointType`
  `https://w3id.org/idsa/v4.1/HTTP`, although the profile now names
  `.../http-pull`: changing it would break 1.0 clients; to look at again
  for 2.0.
- **TD-30.** The issuer's delivery outbox was not changed: since 0.6 it
  gives up at once on a 4xx other than 401, 408 and 429
  (`is_permanent` in `crates/portitor-issuer/src/delivery.rs`).
- **TD-32.** Portitor is stricter than EDC, which ignores a field it
  does not know: a partner whose policies carry `duty`, `remedy` or
  `consequence` cannot negotiate with Portitor. The SHACL shapes could
  not do this check: `portitor-shacl` refuses `sh:closed`, and a
  misspelled JSON key is gone before SHACL sees the data. As consumer,
  Portitor does not check that a provider's offer keeps the
  negotiation's `target`. A logical constraint (`and`, `or`, `xone`)
  with one member written as an object, not a list, is refused. Policy
  shapes in the bundle are research (ROADMAP "Research before code").
- **TD-33.** DCP 6.7.1 says each metadata entry "MUST contain all
  OPTIONAL properties", which read word for word includes
  `issuancePolicy`. The JSON schema does not require it, and Portitor
  leaves it out ([TD-14](#td-14)).
- **TD-34.** A file written before 0.12 may hold two transfers for one
  repeated request. Both stay, and the one stored first answers.
- **TD-36.** Catalog pages are neither sent nor followed. As provider
  Portitor sends its catalog in one answer (measured with 10,000
  datasets: 2.6 MB in 13 ms, [BENCHMARKS](BENCHMARKS.md)). The
  management call that reads a catalog takes at most 16 MiB in 30
  seconds and returns the first page with its `Link` headers; it does
  not follow `rel="next"`. EDC 1.0.0-RC8 does not either. Following
  pages waits for a partner that sends them (DEPLOYING, "Known
  limits").
- **TD-37.** No check that the issuer's base URL equals the `issuer` of
  its credentials: DCP 6.3 gives no such check to a holder or a
  verifier, and EDC compares no URL (ADR 0016).
- **TD-38.** The catalog call refuses `querySpec` with 400: DSP 2025-1
  leaves the content of a `filter` to each implementation. From 0.14 it
  answers 502 when the catalog has no `participantId` or one that
  differs from the request's `counterPartyId`
  ([OPERATING](OPERATING.md#from-013-to-014)); a provider that uses
  another participant id per profile
  ([ADR 0024](adr/0024-several-data-spaces.md)) is then refused.
- **TD-41.** A message or a credential request that meets the same bug
  on every try is tried again for as long as the process runs, with
  pauses up to 5 minutes. Each try is logged and counted. A restart
  policy is still advised (DEPLOYING, "Known limits"). The release
  binaries are about 13% larger.
- **TD-42.** The four services keep their own checks, deciders and
  messages: a change to one service's own rules may still be needed in
  its siblings ("check the other three",
  [TESTING](TESTING.md#unit-and-integration-tests)).
- **TD-43.** The ADRs keep the document versions they were written
  against; they are records. ARCHITECTURE is the place that says what
  was checked.
- **TD-44.** The benchmark that found it was not run again after the
  fix. Until 0.14 a finalization event settled this way was still
  counted in `portitor.outbox.message.given_up`; from 0.14 it is not.
- **TD-48.** No lock in the process any more (1.2, maintainer,
  2026-11-26): the transfer is leased in the database, so several
  processes on one database keep the steps one at a time, and no call
  waits. Left over: a partner's message that comes while a step runs
  gets 503, which DSP 2025-1 does not name (it names 4xx for a refused
  message); Eclipse EDC answers the same and retries 5xx
  ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), note added in
  1.2). The lease is 30 seconds, a constant: a step of a remote data
  plane (ADR 0028, step 2) must stay well under it. A message that a
  step left over makes ready, when a call (not the sweep) ran that step
  and its own write then failed, waits for the next restart.
- **TD-08.** SQLite is deprecated and removed in 2.0
  ([TD-59](#td-59)). A process that stopped without ending its SQLite
  lease makes a new start on that file refuse to start for up to 30
  seconds; start it again after that.
- **TD-52.** A suspension and a start again on another container within
  the same 5 seconds are not seen by a running download or push (the
  flow reads `STARTED` again); a counter of state changes on the flow
  would close it.
- **TD-53.** The outbox loop reads every pending message of the four
  process tables each round, not only the free ones as EDC's
  `nextNotLeased` does; fine while few messages are pending at once. The
  loop interval (1 s) and the lease (30 s) are constants.
- **TD-55.** Another process checks with the bundle version before for
  up to 5 seconds and the time it takes to load the new one; the rules
  stay in each process's memory.
- **TD-56.** The retry pauses after a failed Gaia-X run are counted per
  process, so with two replicas a failing service is asked about twice
  as often; the bundle harvest and the start harvest run in each
  container.
- **TD-57.** A delivery try that outlasts its 30-second lease may be
  sent twice; the holder takes it (same `issuerPid`).
- **TD-58.** The DID document is still built at start: a key change
  with several replicas takes the two-step rolling rotation.
- **R-48.** An issuer that `PORTITOR_TRUSTED_ISSUER_TYPES` does not
  name keeps every type, as in 1.3 ([TD-74](#td-74), 2.0).
- **TD-13.** The offers of a shown dataset are shown to all who see it.
  A catalog request asks every caller for the scopes of all access
  policies ([R-50](#r-50)). Portitor as consumer grants, in a
  negotiation's token, the scopes of the offer only: an access policy
  on a credential type the offer does not name holds at negotiation
  only if that type's scope is configured (`PORTITOR_DCP_SCOPES`); the
  datasets file should name such a type in the offer too.
- **TD-20.** A revocation reaches the identity's presentations within
  `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS` (60 s by default); a list
  that cannot be read now keeps the last status.
- **TD-46.** Claims per credential request are not built: a second
  permit still means new claims, then a new request.
- **TD-60.** `negotiation_request` is not a UNIQUE index: a 1.3
  database may already hold repeats.
- **TD-70.** The bundle's `follow_database` loop keeps its own timer:
  its first round is at once, not one period later.

<a id="upstream-reports"></a>
## Upstream reports

What implementing the specifications showed, and where it was reported
(R-07; filed by the maintainer on 2026-11-16). Until upstream decides,
Portitor's workarounds may differ from what others do. The code that
works around a finding links its report.

| # | Project | Finding | Report |
|---|---|---|---|
| 1 | DSP 2025-1 | Three transfer JSON Schemas (suspension, termination, error) whose `$ref` to `AbstractTransferCodeMessage` is not a valid JSON pointer (`#definitions/...`; tag `2025-1-err2`) | [DataspaceProtocol 265](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/issues/265) (existing issue; the maintainer opens the fix as a pull request) |
| 2 | Data Plane Signaling v1.0-RC4 | Five examples (start, started, resume, status, base data address) fail the specification's own schemas: no `@type` in their data addresses, and two are an empty `{}` | [dataplane-signaling 133](https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/133) |
| 3 | Data Plane Signaling | The `DataFlowStartMessage` example is a pull with a `dataAddress`, while the text says that "MUST result in a HTTP 4xx Client Error" | [dataplane-signaling 134](https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/134) |
| 4 | Data Plane Signaling | Only the party that suspended may resume, while DSP lets either party start again; no message to ask the data plane for a new data address when the provider starts again (Portitor's data plane gives a new token, ADR 0014); the "Consumer Suspend/Start" diagram shows a start message the DSP TCK refuses (ADR 0013) | [dataplane-signaling 135](https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/135) |
| 5 | Signaling HTTP profile | Its `endpointType` (`https://w3id.org/dspace-sig/profile/http-pull`, HTTPS required) against DSP and the DSP TCK (`https://w3id.org/idsa/v4.1/HTTP`, format `HTTP-PULL`). What Portitor sends: ADR 0013 | [profiles 8](https://github.com/eclipse-dataplane-signaling/profiles/issues/8) |
| 6 | DSP TCK v1.0.2 | The transfer start race: its consumer moves to REQUESTED only after it reads the provider's answer, and its start handler removes itself after the first start message, even one it refused (ADR 0014, R-08) | [dsp-tck 338](https://github.com/eclipse-dataspacetck/dsp-tck/issues/338) |
| 7 | DCP v1.0.1 | The `$id` mistakes of `credential-message-schema.json` and `context-schema.json` (`crates/portitor-dcp/tests/spec.rs`) | comment on [DCP 270](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/270) |
| 8 | DCP | `credentialType` in `CredentialMessage`, `type` required on sparse `CredentialObject`s, the offer example's profile | [DCP 276](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/276) |
| 9 | DCP | The scope grammar: a scope cannot name a compact IRI type such as `gx:ParticipantComplianceCredential`, because the DCP TCK's pattern forbids `:` in the type. Portitor's rule: [ADR 0021](adr/0021-gaia-x-onboarding.md#credential-types-and-scopes) | [DCP 277](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/277) |
| 10 | EDC IdentityHub | The default scope transformer needs the `:read` suffix (ADR 0019). Answered 2026-11-17, won't fix: DCP leaves the shape to each implementation. Portitor keeps sending `:read` and accepts both forms | [IdentityHub 1278](https://github.com/eclipse-edc/IdentityHub/issues/1278) |
| 11 | DCP TCK v1.2.1 | Test credentials whose `jti` differs from the credential `id`, 2 KB status lists, BitstringStatusList on a VC 1.1 credential (ADR 0015) | [dcp-tck 243](https://github.com/eclipse-dataspacetck/dcp-tck/issues/243) |
| 12 | DCP | How a credential whose subject is not the holder, such as the Gaia-X compliance credential, is bound to its holder (ADR 0022) | [DCP 278](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278) |
| 13 | Gaia-X | The same binding question | Not filed: the place is the ICAM working group on GitLab, where the maintainer has no account (2026-11-16) |
| 14 | EDC Connector | The subject check on every credential | On hold under [R-10](#r-10) |
| 15 | EDC IdentityHub 1.0.0-RC8, Issuer Service | The `Location` of a credential request leaves out the issuance API path, so the request status (DCP 6.8) answers 404 ([R-46](#r-46)) | Not filed yet (found 2026-11-20) |
