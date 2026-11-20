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

> **Top risks for 1.0** (chosen from the items that are not closed, by
> what they can cost at or after 1.0)
>
> 1. [R-25](#r-25) The name is not checked for trademarks and domains.
>    It blocks 1.0, and a rename after 1.0 is expensive.
> 2. [R-02](#r-02) One maintainer, who finds Rust hard to read
>    ([R-01](#r-01)): every change, security fixes included, has one
>    reviewer.
> 3. [R-12](#r-12) Gaia-X was never tested with a real trust anchor.
>    Every Gaia-X claim rests on a local service in test mode.
> 4. [R-17](#r-17) Server-side request forgery: internal addresses are
>    not blocked on URLs a partner chooses, and no release is named for
>    the fix.
> 5. [R-09](#r-09) EDC is tested at a release candidate. Interoperability
>    with EDC 1.0.0, the first reference counterpart, is not proven.
>
> Next in line: [TD-05](#td-05) (private keys are PEM files),
> [TD-31](#td-31) (no audit trail of data pulls) and [R-24](#r-24)
> (Simpl-Open is untested).

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

## Items that are not closed

"Target" is the release the roadmap names. "Later" is the roadmap's list
of work with no release yet.

| ID | Title | Kind | Status | Target |
|---|---|---|---|---|
| [R-01](#r-01) | Rust is hard to read for the maintainer and slower to build | Delivery | Mitigated | every release |
| [R-02](#r-02) | One maintainer: no two-person review | Delivery, security | Accepted | until a second maintainer |
| [R-03](#r-03) | Pre-1.0 JSON-LD and VC crates | Dependency | Mitigated | none |
| [R-04](#r-04) | Most dependencies are exempted, not audited (cargo-vet) | Dependency, security | Mitigated | every release |
| [R-05](#r-05) | No written OpenSSF Baseline level 1 self-assessment | Security | Open | 0.14 |
| [R-06](#r-06) | Data Plane Signaling is a draft (v1.0-RC4) | Upstream | Mitigated | Signaling 1.0 |
| [R-08](#r-08) | The DSP TCK v1.0.2 needs a test-only build | Interoperability | Mitigated | a fixed TCK release |
| [R-09](#r-09) | EDC is tested at a release candidate (1.0.0-RC8) | Interoperability | Mitigated | EDC 1.0.0 |
| [R-10](#r-10) | EDC as provider refuses the Gaia-X compliance credential | Interoperability | Accepted | Later |
| [R-11](#r-11) | Gaia-X does not use DCP; the binding and scope rules are Portitor's own reading | Interoperability, upstream | Open | none |
| [R-12](#r-12) | Gaia-X never tested with a real trust anchor or the Gaia-X Lab | Interoperability, security | Open | Later |
| [R-13](#r-13) | The Gaia-X test depends on Gaia-X and w3id.org hosts | Delivery | Mitigated | none |
| [R-14](#r-14) | Gaia-X protocols move: the Compliance Dispatcher is announced, not released | Upstream | Mitigated | Later |
| [R-16](#r-16) | A Gaia-X compliance credential cannot be revoked | Security | Accepted | none |
| [R-17](#r-17) | Server-side request forgery through partner-chosen URLs | Security | Mitigated | not named |
| [R-19](#r-19) | Consumer's data access tokens in the SQLite file and the management API | Security | Mitigated | none |
| [R-20](#r-20) | The issuer key is the root of trust of a dataspace | Security | Mitigated | 1.4 (TD-05) |
| [R-21](#r-21) | Bugs in Portitor's own SHACL engine; new SHACL components in later profiles | Security, upstream | Mitigated | none |
| [R-22](#r-22) | Bundle: unsigned `latest.json`; a revoked bundle key not seen while offline | Security | Mitigated | Later (TD-16) |
| [R-23](#r-23) | A wrong host clock gives wrong policy and token decisions | Security | Mitigated | none |
| [R-24](#r-24) | Simpl-Open: parked, facts not verified, and it changes fast | Interoperability, upstream | Accepted | Later |
| [R-25](#r-25) | Name not checked for trademarks and domains | Delivery, legal | Open | 0.14 (blocks 1.0) |
| [R-27](#r-27) | CI downloads from Maven Central and GitHub | Delivery | Mitigated | none |
| [R-29](#r-29) | DCP costs about 2 ms per request and 8 times fewer requests under load | Performance | Open | with TD-07 |
| [R-36](#r-36) | The OpenTelemetry Rust crates are 0.x and change their API often | Dependency, upstream | Mitigated | none |
| [R-37](#r-37) | Telemetry costs some throughput at saturation | Performance | Mitigated | none |
| [R-38](#r-38) | No specification names an S3 data address | Interoperability | Open | 1.1 |
| [R-39](#r-39) | No specification defines agreements that depend on an upstream agreement | Interoperability | Open | 1.3 |
| [R-40](#r-40) | No shared list of EHDS purposes | Interoperability | Open | 1.2 |
| [R-41](#r-41) | A key removed from a DID document is still trusted for up to 5 minutes | Security | Mitigated | none |
| [R-42](#r-42) | The STS shares the identity's public port | Security | Open | not named |
| [R-43](#r-43) | Negative credential constraints can be met with another credential | Security | Open | not named |
| [R-44](#r-44) | One slow holder delays every issuer delivery | Security | Open | not named |
| [R-45](#r-45) | No request limit per partner | Security | Open | not named |
| [R-46](#r-46) | The EDC Issuer Service names a request status address that is not found | Interoperability, upstream | Open | report upstream; the next EDC release |
| [R-47](#r-47) | The EDC Issuer Service's admin API may change with the next release candidate | Interoperability | Accepted | EDC 1.0.0 |
| [TD-01](#td-01) | How schemas and vocabularies are collected and stored | Technical debt | Open | research, with TD-03 |
| [TD-02](#td-02) | Who calls the data plane, and with which credentials | Technical debt | Open | research; code 1.5 |
| [TD-03](#td-03) | A provider in several dataspaces, each with its own rules | Technical debt | Accepted | profiles in one connector: after 1.0 |
| [TD-04](#td-04) | User to system authentication and authorisation | Technical debt | Accepted | code 1.5 |
| [TD-05](#td-05) | Key custody and key rotation | Technical debt | Open | 1.4 (OpenBao) |
| [TD-07](#td-07) | STS tokens and presentations are not cached | Technical debt | Open | 1.4, if needed |
| [TD-08](#td-08) | One replica per SQLite file; no PostgreSQL adapter | Technical debt | Accepted | Later |
| [TD-09](#td-09) | Used token ids are kept in memory only | Technical debt | Accepted | with TD-08 |
| [TD-10](#td-10) | Transfers: pull only, no token renewal, no push to object storage | Technical debt | Open | 1.1 |
| [TD-11](#td-11) | Management API: no suspend or start again; consumer refuses offers with credential constraints | Technical debt | Open | 1.1 and 1.2 |
| [TD-12](#td-12) | ODRL: few constraints; prohibitions and duties not enforced | Technical debt | Open | 1.2 (purpose) |
| [TD-13](#td-13) | The catalog shows every offer to every member | Technical debt | Open | 1.2 |
| [TD-14](#td-14) | Credential schemas, presentation definitions and issuance policies are not supported | Technical debt | Open | 1.4, if needed |
| [TD-15](#td-15) | Agreements are not revised when the data's scope changes | Technical debt | Open | 1.5 |
| [TD-16](#td-16) | One harvest source; no dataspace registry; other catalogs not checked; no bundle expiry or bundle key rotation | Technical debt | Open | Later |
| [TD-17](#td-17) | The data plane runs inside the connector process | Technical debt | Accepted | after 1.0, when Signaling 1.0 is released |
| [TD-19](#td-19) | No TLS inside the services | Technical debt | Accepted | none |
| [TD-20](#td-20) | A holder must delete a revoked credential by hand after a reissue | Technical debt | Open | 1.4 |
| [TD-21](#td-21) | Gaia-X beyond 0.8 | Technical debt | Open | Later |
| [TD-22](#td-22) | Migrations run forward only; a rollback needs the backup | Technical debt | Accepted | none |
| [TD-28](#td-28) | No trace across participants | Technical debt | Accepted | none |
| [TD-29](#td-29) | No OTLP logs export, no gRPC, no compression | Technical debt | Open | Later |
| [TD-31](#td-31) | No audit trail of data pulls and management actions | Technical debt | Open | 1.1 (data pulls); 1.5 |
| [TD-35](#td-35) | No negotiation started by the provider | Technical debt, interoperability | Open | after 1.0 |
| [TD-39](#td-39) | DEPLOYING is not in deployment order; the docs are long | Technical debt, usability | Mitigated | 1.x (Helm chart) |
| [TD-40](#td-40) | Database calls block the async runtime | Technical debt | Accepted | measure each release |
| [TD-42](#td-42) | Four near-copies of the negotiation and transfer logic | Technical debt | Open | 1.x (merge) |
| [TD-45](#td-45) | Each running pull holds about 0.8 MB of memory | Technical debt | Open | none |

Counts on 2026-11-20: 36 risks and 28 technical debts are not closed
(Open 31, Mitigated 19, Accepted 14); 11 risks and 17 technical debts
are closed.

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
  in 1.x".

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

<a id="r-05"></a>
### R-05 No written OpenSSF Baseline level 1 self-assessment

- **What.** Portitor commits to OpenSSF Baseline level 1
  ([ADR 0012](adr/0012-openssf-baseline.md)), but the control-by-control
  self-assessment is not written.
- **Why it matters.** Until then, level 1 rests on settings and CI, not
  on evidence anyone can check.
- **Current state: Open.** Decided (maintainer, 2026-11-20): the target
  stays level 1 for 1.0.
- **Next.** ROADMAP 0.14, done last so it assesses the final state, in
  [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#openssf-baseline).

<a id="r-06"></a>
### R-06 Data Plane Signaling is a draft (v1.0-RC4)

- **What.** The interface between control plane and data plane follows
  Eclipse Data Plane Signaling v1.0-RC4, an incubating specification
  with no release
  ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate)).
- **Why it matters.** The final release may change messages or states.
- **Current state: Mitigated.** The data plane sits behind the ports of
  `portitor-dps`: function calls in one process, no wire format to
  keep. Other connectors see only DSP and the data endpoint.
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
  (`interop/edc/runtimes/gradle.properties`). The IdentityHub Identity
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
  (report 13).

<a id="r-12"></a>
### R-12 Gaia-X never tested with a real trust anchor or the Gaia-X Lab

- **What.** The Gaia-X end-to-end test uses a local compliance service
  in test mode: `DISABLE_SIGNATURE_CHECK=true` (certificate chains not
  checked against Gaia-X trust anchors) and `production=false` (any
  registration number issuer accepted), with a test CA and a test notary
  (`interop/gaiax/compose.yaml`). A run against the Gaia-X Lab needs a
  public `did:web` and a CA certificate; Portitor runs only on the
  maintainer's laptop (maintainer, 2026-11-15).
- **Why it matters.** The real trust chain and a real notary are not
  proven; the Lab may expect another ontology version
  ([spike 0.8](spikes/0.8-gaia-x-compliance.md), "Not checked").
- **Current state: Open.** [TESTING](TESTING.md#gaia-x-end-to-end-test)
  lists what the test does not prove.
- **Next.** ROADMAP "Later": a test against the Lab once Portitor runs
  beyond a laptop.

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
- **Next.** ROADMAP "Later" (TD-21).

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
- **Why it matters.** A partner could make a component call an internal
  service.
- **Current state: Mitigated.** Only `https://` (unless
  `PORTITOR_ALLOW_HTTP`, for tests), no redirects, time and size
  limits, the callback's answer body is never read; DEPLOYING asks for
  an egress firewall. Internal addresses are not blocked.
- **Next.** Block internal addresses with an operator allow list
  (F-01), and check `aud` and the access token before resolving a
  caller (F-06). No release is named. The 0.13 freeze passed without
  it, so a block that is on by default would now be a behaviour change
  to announce.

<a id="r-19"></a>
### R-19 Consumer's data access tokens in the SQLite file and the management API

- **What.** As consumer, the connector keeps each usable transfer's
  data address, token included, in its SQLite file, and shows it on the
  management API ([ADR 0014](adr/0014-transfers-as-consumer.md)).
- **Why it matters.** Whoever reads the file or its backups, or holds
  the API key, can fetch the data until the transfer ends or the token
  expires.
- **Current state: Mitigated.** The token is removed when the transfer
  ends; tokens expire (`PORTITOR_DATA_TOKEN_SECONDS`, default one
  hour); only an HMAC of the API key is kept; DEPLOYING ("Security")
  says to protect the volume and the key alike.
- **Next.** Encrypting the token in the file is later work (ADR 0014);
  no release named.

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
  ([ADR 0017](adr/0017-simpl-open.md), Proposed). Several facts could
  not be verified (ADR 0017, "Not verified"). Simpl-Open's schemas are
  not signed (ADR 0005).
- **Why it matters.** Portitor is not a Simpl participant; reference
  counterpart number two
  ([ADR 0011](adr/0011-interoperability-targets.md)) is untested.
- **Current state: Accepted** (parked by the maintainer, 2026-11-13,
  until a Simpl-Open sandbox exists).
  [STABILITY](STABILITY.md#what-10-does-not-do) says it is not tested.
- **Next.** ROADMAP "Later"; check the facts again before any Simpl
  work.

<a id="r-25"></a>
### R-25 Name not checked for trademarks and domains

- **What.** `portitor` was free on crates.io (2026-11-12); domain names
  and a formal trademark search (EUIPO, WIPO, BOIP) are not done
  ([ADR 0008](adr/0008-project-name.md)).
- **Why it matters.** A rename after 1.0 is expensive.
- **Current state: Open.**
- **Next.** ROADMAP 0.14; it blocks 1.0 (maintainer, 2026-11-15).

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
  (DEPLOYING, "Traces and metrics"); runs vary by up to 15%. Sampling
  lowers the data sent, not the cost in the process.
- **Why it matters.** A busy connector with full tracing reaches its
  limit sooner.
- **Current state: Mitigated.** Export is off by default; the
  differences are close to the noise of one machine.
- **Next.** Measure again with the benchmarks
  ([BENCHMARKS](BENCHMARKS.md)); if the cost is real, look at what the
  spans cost when export is off.

<a id="r-38"></a>
### R-38 No specification names an S3 data address

- **What.** DSP 2025-1 lets the consumer give a `dataAddress` in its
  transfer request, and Data Plane Signaling v1.0-RC4 covers provider
  push, but neither names an endpoint type or the properties for
  S3-compatible storage (checked in their schemas, 2026-11-15).
- **Why it matters.** Push to object storage (ROADMAP 1.1) works with
  another connector only if both read the address the same way.
- **Current state: Open.**
- **Next.** ROADMAP 1.1: the ADR reuses the address EDC or Simpl-Open
  uses; Portitor does not invent one.

<a id="r-39"></a>
### R-39 No specification defines agreements that depend on an upstream agreement

- **What.** In the health dataspace demo, Cooper's offer to Marley
  rests on Cooper's agreement with Alice and must not be wider than it.
  DSP and ODRL have no link between two agreements.
- **Why it matters.** Portitor's design may differ from what EDC or
  Simpl-Open choose later.
- **Current state: Open.**
- **Next.** ROADMAP 1.3: research and an ADR before code.

<a id="r-40"></a>
### R-40 No shared list of EHDS purposes

- **What.** The EHDS lists the purposes of secondary use (Article 53),
  but no list with identifiers exists to reuse; the maintainer knows of
  none in ICUdata4EU or LNDS (2026-11-15). HealthDCAT-AP release 8
  names purposes with `dpv:hasPurpose`.
- **Why it matters.** A `purpose` constraint is useful only if both
  sides name purposes the same way.
- **Current state: Open.**
- **Next.** ROADMAP 1.2: purposes as Data Privacy Vocabulary (DPV)
  IRIs; check how they map to Article 53.

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
- **Current state: Open.**
- **Next.** Say in ADR 0019 and DEPLOYING that negative operators are
  not deny lists.

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
  `scripts/edc-interop.sh` when EDC is updated, as with the Identity API
  ([R-09](#r-09); TESTING, "From RC7 to RC8").
- **Next.** Read the admin API again at each EDC update. Suspension is
  not tested until EDC implements it.

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

<a id="td-02"></a>
### TD-02 Who calls the data plane, and with which credentials

- **What.** Only participants are identified, and only in the control
  plane ([ARCHITECTURE](ARCHITECTURE.md#who-holds-the-credentials)). The
  data plane checks only the bearer token handed out when the transfer
  started ([ADR 0013](adr/0013-transfer-and-data-plane.md)): whoever
  holds the token gets the data until the transfer ends. No identity or
  credential is checked on each data request (maintainer, 2026-11-15).
- **Current state: Open.**
  [ADR 0025](adr/0025-users-and-roles.md) decides that the provider's
  data plane does not identify persons in 1.x. What it checks again
  about the participant on each pull is not decided.
- **Next.** Research, then an ADR; code in ROADMAP 1.5. To compare: a
  DCP token or a presentation per request, OpenID4VP, Simpl-Open's mTLS
  at its Tier 2 gateway, what the EDC data plane does; whether the
  policy is checked again on each request; how this fits the consent
  and gateway items under
  [Later](ROADMAP.md#later-not-planned-for-a-release-yet).

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
  `scripts/two-data-spaces.sh`).
- **Next.** After 1.0, no release named: one connector with several
  dataspace profiles, behind its own ADR; trusted issuers by
  credential type (threat model finding F-08), which is additive.

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
- **Next.** In 1.5: access tokens of the organisation's identity
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
  (ADR 0015 and ADR 0018, "Amended in 0.11: retired keys"; DEPLOYING,
  "Rotating a key").
- **Still open.** Key custody: the private key is a PEM file. The
  bundle key cannot be rotated (one key, TD-16). With Gaia-X, each key
  change needs a new certificate and a new compliance run
  ([ADR 0021](adr/0021-gaia-x-onboarding.md)).
- **Next.** ROADMAP 1.4: a signer port with an OpenBao transit adapter.

<a id="td-07"></a>
### TD-07 STS tokens and presentations are not cached

- **What.** Each outgoing DSP request costs one STS call; each incoming
  one a presentation query (DEPLOYING, "Known limits").
- **Why it matters.** Cost per request (R-29).
- **Current state: Open.** ADR 0016 chose "measure before adding more
  caching", so a revocation takes effect at once. The 0.12 numbers are
  in R-29.
- **Next.** Decide from those numbers; code in ROADMAP 1.4 if needed.

<a id="td-08"></a>
### TD-08 One replica per SQLite file; no PostgreSQL adapter

- **What.** Each image keeps its state in one SQLite file; several
  replicas need the PostgreSQL adapter, which comes later on the same
  ports and migrations ([ADR 0006](adr/0006-negotiation-storage.md)).
- **Why it matters.** No horizontal scaling or failover; ARCHITECTURE
  principle 8 is not fully met.
- **Current state: Accepted** (maintainer, 2026-11-12). Migrations are
  plain, portable SQL; the store ports are ready. No PostgreSQL code
  exists.
- **Next.** Later; no release named.

<a id="td-09"></a>
### TD-09 Used token ids are kept in memory only

- **What.** The `jti` replay list lives in memory, at most 100,000
  entries (ADR 0015, ADR 0016).
- **Why it matters.** A restart forgets them; it works with one replica
  only.
- **Current state: Accepted** (maintainer, 0.5 review): tokens live at
  most an hour, so the window is small.
- **Next.** Revisit with TD-08.

<a id="td-10"></a>
### TD-10 Transfers: pull only, no token renewal, no push to object storage

- **What.** HTTP pull only; the data token is not renewed (DSP 2025-1
  has no message for it), the consumer asks for a new transfer; only
  `GET` of the whole source
  ([ADR 0013](adr/0013-transfer-and-data-plane.md)). The consumer side
  has no data plane (ADR 0014).
- **Why it matters.** Long or large transfers, and dataspaces that use
  push (Simpl-Open documents S3 push and Kafka).
- **Current state: Open.** Measured in 0.12
  ([BENCHMARKS](BENCHMARKS.md), "Data plane"): a download still running
  when its token ends is not cut; a new request with the old token gets
  401. So a long pull works, and a pull that must start again after the
  token's life does not.
- **Next.** ROADMAP 1.1: push as a second data plane behind the
  Signaling port.

<a id="td-11"></a>
### TD-11 Management API: no suspend or start again; consumer refuses offers with credential constraints

- **What.** The consumer's management API completes or terminates a
  transfer, but does not suspend or start it again; the provider cannot
  suspend or complete from the management API (ADR 0013, ADR 0014). As
  consumer, the connector does not accept a provider's offer (DSP
  `OFFERED`) that has credential constraints (DEPLOYING, "Known
  limits").
- **Why it matters.** The domain supports these steps, but operators
  cannot use them.
- **Current state: Open.**
- **Next.** ROADMAP 1.1 (suspend and start again) and 1.2 (offers with
  credential constraints).

<a id="td-12"></a>
### TD-12 ODRL: few constraints, prohibitions and duties not enforced

- **What.** Only `dateTime` and credential claims are understood; other
  left operands fail closed and stop the offer. Prohibitions and duties
  are carried into agreements but not enforced. Terms are compared as
  written, so a counter-offer is terminated
  ([ADR 0009](adr/0009-minimal-odrl-policies.md),
  [ADR 0019](adr/0019-credential-constraints.md)).
- **Why it matters.** Offers with `purpose`, `spatial` and similar
  cannot be published; usage control stops at the agreement.
- **Current state: Open.** Fail closed: nothing not understood is
  granted.
- **Next.** ROADMAP 1.2: `purpose`; others when needed.

<a id="td-13"></a>
### TD-13 The catalog shows every offer to every member

- **What.** Credential constraints are checked at negotiation and
  transfer, not when the catalog is built; EDC's "access policy" is
  later work ([ADR 0019](adr/0019-credential-constraints.md)).
- **Why it matters.** Members see offers they cannot meet.
- **Current state: Open.**
- **Next.** ROADMAP 1.2.

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
- **Next.** ROADMAP 1.4, if a target dataspace needs it; the issuer's
  decision port allows it.

<a id="td-15"></a>
### TD-15 Agreements are not revised when the data's scope changes

- **What.** An agreement keeps working after its dataset leaves the
  source; there is no way to revise it when a data subject objects or
  the consumer asks for a new scope (maintainer, 2026-11-14;
  [ADR 0020](adr/0020-dataset-harvest.md)).
- **Why it matters.** Consent withdrawal and scope changes.
- **Current state: Open.**
- **Next.** ROADMAP 1.5, with consent, if the consent ADR finds a
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
- **Next.** ROADMAP "Later".

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
- **Next.** Split it out as an optional component once Data Plane
  Signaling is released (R-06), or when measured load needs it.

<a id="td-19"></a>
### TD-19 No TLS inside the services

- **What.** The services speak plain HTTP behind a TLS-terminating
  reverse proxy; the management ports stay on a private network.
- **Current state: Accepted** (maintainer, 2026-11-20,
  [ADR 0026](adr/0026-tls-termination.md)): TLS stays the operator's
  job.
- **Next.** None. Optional TLS in the services would be additive; it is
  not planned.

<a id="td-20"></a>
### TD-20 A holder must delete a revoked credential by hand after a reissue

- **What.** Verifiers refuse a presentation that holds any revoked
  credential (DCP v1.0.1, 5.4.3, step 8). The identity does not read
  status lists, so it keeps presenting a revoked credential next to its
  reissued one.
- **Why it matters.** A participant is refused until its operator
  deletes the revoked credential.
- **Current state: Open.** Documented in DEPLOYING.
- **Next.** ROADMAP 1.4.

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
- **Next.** ROADMAP "Later", Gaia-X beyond 0.8.

<a id="td-22"></a>
### TD-22 Migrations run forward only; a rollback needs the backup

- **What.** An older version refuses a database a newer one migrated;
  going back means restoring the backup, and data written since is lost
  (DEPLOYING, "Upgrade and rollback").
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
  warning (ADR 0023; DEPLOYING, "Traces and metrics").
- **Why it matters.** Operators whose platform takes only OTLP logs or
  gRPC need a collector in between.
- **Current state: Open.** Kept out to save binary size and
  dependencies.
- **Next.** Later; no release named.

<a id="td-31"></a>
### TD-31 No audit trail of data pulls and management actions

- **What.** The connector logs every state change of a negotiation or
  transfer with its process ids
  (`crates/portitor-connector/src/state_events.rs`) and keeps each
  agreement with the counter-party's DID. It does not log a successful
  data pull, does not keep the caller's token or presentation with an
  agreement, and does not log failed management key checks or some
  management changes (for example the identity's
  `DELETE /v1/credentials/{id}`). A management call has no person
  behind it ([TD-04](#td-04); SECURITY-ASSURANCE, F-02).
- **Why it matters.** A provider cannot show who pulled which data and
  when. Health dataspaces are likely to require this (EHDS; not
  checked which article).
- **Current state: Open.** DSP 2025-1 does not sign its messages, so
  evidence between participants rests on the DCP tokens and on each
  side's own records.
- **Next.** [ADR 0025](adr/0025-users-and-roles.md) decides: a log line
  for each data pull (flow, agreement, consumer DID, bytes) in ROADMAP
  1.1; audit records that name a person in 1.5.

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
- **Next.** After 1.0; no release named.

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
- **Next.** A Helm chart in a 1.x release (ROADMAP "Code quality in
  1.x"). Until then there is the Compose example.

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
  100 ms, or with [TD-08](#td-08).

<a id="td-42"></a>
### TD-42 Four near-copies of the negotiation and transfer logic

- **What.** Negotiation and transfer, each as provider and as consumer,
  have the same shape (receive, check, store, outbox, deliver, abort)
  in four services of about 770 lines each
  (`crates/portitor-dsp/src/negotiation/`,
  `crates/portitor-dsp/src/transfer/`) and four adapters in the
  connector. The in-memory stores live in production code
  ([review](reviews/2026-11-18-specifications-and-code-quality.md),
  Q5).
- **Why it matters.** A fix in one may be needed in the other three.
- **Current state: Open.**
  [READING-THE-CODE](READING-THE-CODE.md#the-three-siblings) names the
  four files side by side, and from 0.14 TESTING has the rule "changed
  one, check the other three"
  ([Unit and integration tests](TESTING.md#unit-and-integration-tests)).
- **Next.** ROADMAP "Code quality in 1.x": one shared core, best
  before 1.1 adds push; the in-memory stores move into the tests.

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
  release.

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
| <a id="td-06"></a>TD-06 | Status lists were not cached in the connector's DCP verifier | 0.11 | [ADR 0016](adr/0016-dsp-caller-authentication.md), amended; DEPLOYING, "Status list cache" |
| <a id="td-18"></a>TD-18 | Operations: no metrics, traces or readiness check | 0.9 | [ADR 0023](adr/0023-observability.md) |
| <a id="td-23"></a>TD-23 | ADRs 0021 and 0022 were built but still Proposed | 0.8 | Both accepted on 2026-11-15 ([ADR index](adr/README.md)) |
| <a id="td-24"></a>TD-24 | A 3 second wait for acknowledgements; messages lost at a restart | 0.3 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md) |
| <a id="td-25"></a>TD-25 | A test-only tool signed the demo credentials | 0.6 | [ADR 0018](adr/0018-issuer-component.md): `portitor-issuer` replaced it |
| <a id="td-26"></a>TD-26 | One bad dataset stopped the connector at start | 0.7 | [ADR 0020](adr/0020-dataset-harvest.md) |
| <a id="td-27"></a>TD-27 | The domain crates' I/O check named too few crates | 0.6 (2026-11-14) | [ADR 0004](adr/0004-hexagonal-architecture.md), amended; `.github/workflows/ci.yml` |
| <a id="td-30"></a>TD-30 | The outbox resent a message the counter-party refused | 0.11 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), amended in 0.11 |
| <a id="td-32"></a>TD-32 | Policy terms that were not understood were dropped without an error | 0.12 | [ADR 0009](adr/0009-minimal-odrl-policies.md), amended 2026-11-18 |
| <a id="td-33"></a>TD-33 | The issuer's metadata did not pass the DCP JSON schema | 0.12 | [ADR 0018](adr/0018-issuer-component.md), amended 2026-11-18; DEPLOYING, "From 0.11 to 0.12" |
| <a id="td-34"></a>TD-34 | A repeated transfer request started a second transfer | 0.12 | [ADR 0013](adr/0013-transfer-and-data-plane.md), amended 2026-11-18 |
| <a id="td-36"></a>TD-36 | DSP details: who may terminate, 401 or 404, agreement timestamp, `consumerPid` check, catalog pages | 0.12; the answer limit in 0.13 | [ADR 0006](adr/0006-negotiation-storage.md) and [ADR 0016](adr/0016-dsp-caller-authentication.md), amended 2026-11-18; CHANGELOG 0.13.0 |
| <a id="td-37"></a>TD-37 | DCP details: `token` claim required, presentations as JWT strings only, no `/.well-known/dspace-trust` | 0.12 | [ADR 0016](adr/0016-dsp-caller-authentication.md), amended 2026-11-18 |
| <a id="td-38"></a>TD-38 | No management call to read a provider's catalog | 0.13; the `participantId` check in 0.14 | CHANGELOG 0.13.0 and 0.14.0; `docs/api/management-openapi.json`; [ARCHITECTURE](ARCHITECTURE.md#diagrams-c4-model) |
| <a id="td-41"></a>TD-41 | A panic stopped the whole process (`panic = "abort"`) | 0.12 | [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md), "What a panic does"; [ADR 0002](adr/0002-technology-stack.md), amended in 0.12 |
| <a id="td-43"></a>TD-43 | The Gaia-X documents name several versions of the rules | 0.14 | [ARCHITECTURE](ARCHITECTURE.md#specification-versions) |
| <a id="td-44"></a>TD-44 | A transfer request sent at once after the agreement could be refused | 0.13; the metric in 0.14 | [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), "Amendment for 0.13" and "Note added in 0.14"; CHANGELOG 0.14.0 |

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
- **TD-43.** The ADRs keep the document versions they were written
  against; they are records. ARCHITECTURE is the place that says what
  was checked.
- **TD-44.** The benchmark that found it was not run again after the
  fix. Until 0.14 a finalization event settled this way was still
  counted in `portitor.outbox.message.given_up`; from 0.14 it is not.

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
