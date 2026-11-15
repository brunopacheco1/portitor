<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Risks and technical debts

This file collects every risk and technical debt raised so far, from
release 0.0 to 0.8, in one place: the ADRs, the spikes, the roadmap, the
security case, DEPLOYING, TESTING, the changelog, the commit messages,
and what the maintainer said in the project chat. For each item it says
whether it is still open, and what is in place today.

Two kinds:

- **Risk**: something that could go wrong: security, interoperability,
  specification or standard churn upstream, a dependency, or delivery.
- **Technical debt**: a known shortcut or a missing capability in the
  code or the design.

Status values:

- **Open**: nothing removes it yet.
- **Mitigated**: reduced, not removed. The item says how.
- **Addressed**: done. The item says where (file, ADR, release).
- **Accepted**: kept on purpose. The item says who decided.
- **Status unverified**: could not be confirmed from the code or the
  docs; the item says why.

"Target" is the release the roadmap names; "Later" is the roadmap's list
of work not planned for a release yet; "—" means no target.

## How to keep this file up to date

This file is the single list of risks and technical debts. The
[roadmap](ROADMAP.md) keeps the plan and only names the technical debts,
with links here. [DEPLOYING](DEPLOYING.md#known-limits) keeps the
operator's view ("Known limits") and links here.

Every release review ([RELEASING](RELEASING.md#documentation-review))
checks this file:

1. Check each open item against the code of the release. Change its
   status, and say where it was addressed (file, ADR, release).
2. Add what the release raised: new ADR consequences, spike risks,
   review findings, the maintainer's decisions, "not in this release"
   lists. Give each a new id (`R-nn`, `TD-nn`); never reuse an id.
3. Keep addressed items in the file: they show what was closed, where
   and in which release.

## Summary

| ID | Title | Kind | Raised in | Status | Target |
|---|---|---|---|---|---|
| [R-01](#r-01) | Rust is hard to read for the maintainer and slower to build | Delivery | 0.0 (ADR 0002, 2026-11-11); 0.4 (maintainer, 2026-11-13) | Mitigated | every release |
| [R-02](#r-02) | One maintainer: no two-person review | Delivery, security | 0.1 (ADR 0012, maintainer, 2026-11-11) | Accepted | until a second maintainer |
| [R-03](#r-03) | Pre-1.0 JSON-LD and VC crates | Dependency | 0.0 (ADR 0002, 2026-11-11) | Mitigated | — |
| [R-04](#r-04) | Dependencies are not audited crate by crate (cargo-vet) | Dependency, security | 0.1 (maintainer, 2026-11-11 and 2026-11-12) | Open | 0.10 |
| [R-05](#r-05) | No written OpenSSF Baseline level 1 self-assessment | Security | 0.1 (ADR 0012, maintainer, 2026-11-12) | Open | 0.10 |
| [R-06](#r-06) | Data Plane Signaling is a draft (v1.0-RC4) | Upstream churn | 0.0 (ARCHITECTURE, 2026-11-11) | Mitigated | until Signaling 1.0 |
| [R-07](#r-07) | Upstream findings not reported yet | Upstream churn, interoperability | 0.4 to 0.8 (maintainer, 2026-11-13 to 2026-11-15) | Open | — |
| [R-08](#r-08) | The DSP TCK v1.0.2 needs a test-only build | Interoperability | 0.4, 0.5 (ADR 0014, ADR 0016, 2026-11-13) | Mitigated | — |
| [R-09](#r-09) | EDC is tested at a release candidate (1.0.0-RC7) | Interoperability, dependency | 0.5 (maintainer, 2026-11-13) | Mitigated | pin 1.0.0 when it ships |
| [R-10](#r-10) | EDC refuses the Gaia-X compliance credential | Interoperability | 0.8 (ADR 0022, maintainer, 2026-11-15) | Accepted | Later |
| [R-11](#r-11) | Gaia-X does not use DCP; Portitor's binding and scope rules are its own reading | Interoperability, upstream churn | 0.8 (ADR 0021, ADR 0022, 2026-11-15) | Open | — |
| [R-12](#r-12) | Gaia-X never tested with a real trust anchor or the Gaia-X Lab | Interoperability, security | 0.8 (maintainer, 2026-11-15) | Open | Later |
| [R-13](#r-13) | The Gaia-X test depends on Gaia-X and w3id.org hosts | Delivery | 0.8 (spike, TESTING, 2026-11-15) | Mitigated | — |
| [R-14](#r-14) | Gaia-X protocols move: the Compliance Dispatcher (OpenID4VP, OpenID4VCI) is announced, not released | Upstream churn | 0.8 (ADR 0021, 2026-11-15) | Mitigated | Later |
| [R-15](#r-15) | Onboarding accepts a compliance answer from any issuer whose signature verifies | Security | 0.8 review (maintainer, 2026-11-15) | Open | 0.10 (proposed) |
| [R-16](#r-16) | A Gaia-X compliance credential cannot be revoked | Security | 0.8 (spike, ADR 0022, 2026-11-15) | Accepted | — |
| [R-17](#r-17) | Server-side request forgery through partner-chosen URLs | Security | 0.3 (ADR 0016 security notes, SECURITY-ASSURANCE) | Mitigated | — |
| [R-18](#r-18) | No request size limit of Portitor's own on the public ports | Security | 0.5 (SECURITY-ASSURANCE) | Open | 0.9 per SECURITY-ASSURANCE (not in the roadmap) |
| [R-19](#r-19) | Consumer's data access tokens in the SQLite file and the management API | Security | 0.4 (ADR 0014, 2026-11-13) | Mitigated | — |
| [R-20](#r-20) | The issuer key is the root of trust of a data space | Security | 0.6 (ADR 0018, 2026-11-14) | Mitigated | — |
| [R-21](#r-21) | Bugs in Portitor's own SHACL engine, and new SHACL components in later profiles | Security, upstream churn | 0.6 spike (ADR 0005, 2026-11-14) | Mitigated | — |
| [R-22](#r-22) | Bundle: unsigned `latest.json`, and a revoked bundle key not seen while offline | Security | 0.7 (ADR 0005, SECURITY-ASSURANCE) | Mitigated | — |
| [R-23](#r-23) | A wrong host clock gives wrong policy and token decisions | Security | 0.3 (ADR 0009, 2026-11-12) | Mitigated | — |
| [R-24](#r-24) | Simpl-Open: parked, facts not verified, and it changes fast | Interoperability, upstream churn | 0.5 (ADR 0017, maintainer, 2026-11-13) | Accepted | Later |
| [R-25](#r-25) | Name not checked for trademarks and domains | Delivery, legal | 0.3 (ADR 0008, 2026-11-12) | Open | before 1.0 |
| [R-26](#r-26) | ODRL `dateTime` definition read from memory | Interoperability | 0.3 (ADR 0009, 2026-11-12) | Open | before 1.0 |
| [R-27](#r-27) | CI downloads from Maven Central and GitHub | Delivery | 0.8 (2026-11-15) | Mitigated | — |
| [R-28](#r-28) | 0.8 Gaia-X code has no fuzz target | Security | 0.8 (sweep for this file, 2026-11-15) | Open | 0.10 |
| [R-29](#r-29) | DCP costs about 2 ms per request and 13 times fewer requests under load | Performance | 0.5 (ADR 0002 section 7, 2026-11-13) | Open | 0.10 (with TD-06) |
| [R-30](#r-30) | DSP callers were not authenticated | Security | 0.3 (ADR 0007, 2026-11-12) | Addressed | done in 0.5 |
| [R-31](#r-31) | RSA timing advisory in the pure-Rust `rsa` crate | Security, dependency | 0.0 (ADR 0002, 2026-11-11) | Addressed | done in 0.0 |
| [R-32](#r-32) | XML parser advisories through RDF/XML code lists and rudof | Security, dependency | 0.6 spike (2026-11-14) | Addressed | done in 0.7 |
| [R-33](#r-33) | Any caller, or one participant, could exhaust the issuer | Security | 0.6 review (ADR 0018, 2026-11-14) | Addressed | done in 0.6 |
| [R-34](#r-34) | Scorecard Branch-Protection and Fuzzing at 0 | Security | 0.1 (2026-11-12) | Addressed | done in 0.2 |
| [R-35](#r-35) | The DCP TCK uses VCDM 1.1, Gaia-X needs VCDM 2.0 | Interoperability | 0.5 (ADR 0015, 2026-11-13) | Addressed | done in 0.5 |
| [TD-01](#td-01) | How schemas and vocabularies are collected and stored | Technical debt | 0.7 (maintainer, 2026-11-14) | Open | — |
| [TD-02](#td-02) | Who calls the data plane, and with which credentials | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | — |
| [TD-03](#td-03) | A provider in several data spaces, each with its own rules | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | — |
| [TD-04](#td-04) | User to system authentication and authorisation, governed per data space | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | — |
| [TD-05](#td-05) | Key custody and key rotation | Technical debt | 0.5 (ADR 0015); 0.7 close-out (maintainer, 2026-11-15) | Open | 0.10 (to investigate) |
| [TD-06](#td-06) | Status lists are not cached in the connector's DCP verifier | Technical debt | 0.6 (maintainer, 2026-11-14) | Open | 0.10 |
| [TD-07](#td-07) | STS tokens and presentations are not cached | Technical debt | 0.5 (ADR 0016) | Open | — |
| [TD-08](#td-08) | One replica per SQLite file; no PostgreSQL adapter | Technical debt | 0.3 (ADR 0006, maintainer, 2026-11-12) | Accepted | Later |
| [TD-09](#td-09) | Used token ids are kept in memory only | Technical debt | 0.5 (ADR 0015, ADR 0016, 2026-11-13) | Accepted | with TD-08 |
| [TD-10](#td-10) | Transfers: pull only, no token renewal, no push to object storage | Technical debt | 0.4 (ADR 0013, maintainer, 2026-11-12 and 2026-11-13) | Open | Later |
| [TD-11](#td-11) | Management API: no suspend or start again; consumer refuses offers with credential constraints | Technical debt | 0.4 (ADR 0014); 0.6 (DEPLOYING) | Open | — |
| [TD-12](#td-12) | ODRL: few constraints, prohibitions and duties not enforced | Technical debt | 0.3 (ADR 0009, 2026-11-12) | Open | — |
| [TD-13](#td-13) | The catalog shows every offer to every member | Technical debt | 0.6 (ADR 0019, 2026-11-14) | Open | — |
| [TD-14](#td-14) | Credential schemas, presentation definitions and issuance policies are not supported | Technical debt | 0.5, 0.6 (ADR 0015, ADR 0018) | Open | — |
| [TD-15](#td-15) | Agreements are not revised when the data's scope changes | Technical debt | 0.7 (maintainer, 2026-11-14) | Open | Later |
| [TD-16](#td-16) | One harvest source; no data space registry; other catalogs not checked; no bundle expiry or bundle key rotation | Technical debt | 0.7 (ADR 0005, ADR 0020, 2026-11-14) | Open | Later |
| [TD-17](#td-17) | The data plane runs inside the connector process | Technical debt | 0.0 (maintainer, 2026-11-11) | Accepted | when Signaling 1.0 is released |
| [TD-18](#td-18) | Operations: no metrics, traces or readiness check | Technical debt | 0.1 (ARCHITECTURE principle 11) | Open | 0.9 |
| [TD-19](#td-19) | No TLS inside the services | Technical debt | 0.1 (SECURITY-ASSURANCE) | Open | — |
| [TD-20](#td-20) | A holder must delete a revoked credential by hand after a reissue | Technical debt | 0.6 (DEPLOYING, 2026-11-14) | Open | — |
| [TD-21](#td-21) | Gaia-X beyond 0.8 | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | Later |
| [TD-22](#td-22) | Migrations run forward only; a rollback needs the backup | Technical debt | 0.3 (DEPLOYING) | Accepted | — |
| [TD-23](#td-23) | ADRs 0021 and 0022 are built but still Proposed | Technical debt | 0.8 (sweep for this file, 2026-11-15) | Open | 0.8 close-out |
| [TD-24](#td-24) | A 3 second wait for acknowledgements; messages lost at a restart | Technical debt | 0.3 (ADR 0006, maintainer, 2026-11-12) | Addressed | done in 0.3 |
| [TD-25](#td-25) | A test-only tool signed the demo credentials | Technical debt | 0.5 (ADR 0015, 2026-11-13) | Addressed | done in 0.6 |
| [TD-26](#td-26) | One bad dataset stopped the connector at start | Technical debt | 0.7 plan (ADR 0020, 2026-11-14) | Addressed | done in 0.7 |
| [TD-27](#td-27) | The domain crates' I/O check named too few crates | Technical debt | 0.5 review (ADR 0004, 2026-11-14) | Addressed | done 2026-11-14 |

Totals: 35 risks (Open 11, Mitigated 14, Accepted 4, Addressed 6);
27 technical debts (Open 19, Accepted 4, Addressed 4).

## Risks

<a id="r-01"></a>
### R-01 Rust is hard to read for the maintainer and slower to build

- **What.** Rust is harder to learn and slower to compile than the
  alternatives (ADR 0002, "main risk is delivery speed"). The maintainer
  is not a Rust expert and finds Rust hard to read (maintainer,
  2026-11-13).
- **Why it matters.** Code the maintainer cannot read cannot be reviewed,
  and the project has one maintainer (R-02).
- **Current state: Mitigated.** [RUST-IDIOMS.md](RUST-IDIOMS.md) explains
  every idiom the code uses; a readability review runs before each
  release since 0.4 (ROADMAP 0.4 to 0.8), next to the documentation
  review ([RELEASING](RELEASING.md#documentation-review)). Clippy
  pedantic as errors, small crates, no `unsafe`. Build time is measured
  (ADR 0002, section 7).
- **Next.** Keep the review in every release; add each new idiom to
  RUST-IDIOMS.

<a id="r-02"></a>
### R-02 One maintainer: no two-person review

- **What.** A sole maintainer cannot approve their own pull request; the
  ruleset allows an admin bypass for merging. OpenSSF Scorecard
  Code-Review and Contributors stay low; controls that need two people
  wait (ADR 0012).
- **Why it matters.** Every change, security fixes included, is
  reviewed by one person. Bus factor of one.
- **Current state: Accepted** (maintainer, ADR 0012, 2026-11-11;
  ROADMAP 0.1). Signed commits, required CI checks, CodeQL and Sonar
  gate every merge. A repository successor is set (ROADMAP 0.1).
- **Next.** Find a second maintainer before 1.0.

<a id="r-03"></a>
### R-03 Pre-1.0 JSON-LD and VC crates

- **What.** Rust's JSON-LD and verifiable credential crates are pre-1.0
  (ADR 0002).
- **Why it matters.** Breaking changes or bugs in a security path.
- **Current state: Mitigated.** DSP and DCP messages are plain JSON with
  typed structs, checked against the official JSON Schemas, which both
  specifications allow (ARCHITECTURE principle 6). Credentials and
  tokens use Portitor's own code on `ring`. A JSON-LD processor
  (`oxjsonld`) is used only for the SHACL checks, with contexts from
  local data (ADR 0005). Each dependency passes `cargo deny`.
- **Next.** None beyond R-04.

<a id="r-04"></a>
### R-04 Dependencies are not audited crate by crate (cargo-vet)

- **What.** cargo-vet (every crate version in `Cargo.lock` audited or
  explicitly exempted) was deferred from 0.1 to the hardening release,
  because it turns each dependency bump into review work (maintainer,
  2026-11-11; moved to "0.9 hardening" on 2026-11-12, and the hardening
  release was renumbered 0.10 on 2026-11-14).
- **Why it matters.** A malicious or careless crate update could reach a
  release.
- **Current state: Open.** No cargo-vet in CI (checked: no
  `cargo-vet` in `.github/` or `Cargo.toml`). What runs today: pinned
  actions, Dependabot, dependency review, OSV-Scanner, `cargo deny`
  (RustSec, licences, sources), ORT ([TESTING](TESTING.md#what-runs-where)).
- **Next.** ROADMAP 0.10.

<a id="r-05"></a>
### R-05 No written OpenSSF Baseline level 1 self-assessment

- **What.** Portitor commits to OpenSSF Baseline level 1 for all of 0.x
  ([ADR 0012](adr/0012-openssf-baseline.md)), but the control-by-control
  self-assessment is not written (moved from 0.1, maintainer, 2026-11-12).
- **Why it matters.** Until then, level 1 rests on settings and CI, not
  on evidence anyone can check.
- **Current state: Open.** [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#openssf-baseline)
  says it comes in 0.10. ADR 0012 still says "0.9 hardening release":
  written before the renumbering of 2026-11-14.
- **Next.** ROADMAP 0.10; decide there whether 1.0 aims higher.

<a id="r-06"></a>
### R-06 Data Plane Signaling is a draft (v1.0-RC4)

- **What.** The internal interface between control plane and data plane
  follows Eclipse Data Plane Signaling v1.0-RC4, an incubating
  specification with no release ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate)).
- **Why it matters.** The final release may change messages or states.
- **Current state: Mitigated.** The data plane sits behind the ports of
  `portitor-dps` (ARCHITECTURE principle 9), in-process function calls,
  no wire format to keep. Other connectors only see DSP and the data
  endpoint.
- **Next.** Track the specification until 1.0 and record each update in
  the changelog; then split the data plane out (TD-17).

<a id="r-07"></a>
### R-07 Upstream findings not reported yet

- **What.** Implementing the specifications showed errors and gaps
  upstream. The reports are drafted but not filed:
  - DSP 2025-1: three transfer JSON Schemas whose `$ref` to
    `AbstractTransferCodeMessage` is not a valid JSON pointer (tag
    `2025-1-err2`; ROADMAP 0.4). Comments on DSP issue #265 (maintainer,
    project chat).
  - Data Plane Signaling v1.0-RC4: five examples that fail its own
    schemas, a pull `start` example that breaks a MUST, who may resume,
    the HTTP profile's `endpointType`, no message for a new token when
    the provider starts again, the "Consumer Suspend/Start" diagram
    ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate),
    ADR 0013, ADR 0014). Comments on Signaling issue #122 (maintainer,
    project chat).
  - DSP TCK v1.0.2: the transfer start race (ADR 0014, R-08).
  - DCP TCK, EDC IdentityHub and DCP schemas (ROADMAP 0.5): the schema
    `$id` mistakes and the offer example's profile listed in
    `crates/portitor-dcp/tests/spec.rs`; IdentityHub's `:read` scope
    suffix (ADR 0019).
  - DCP: a scope cannot name a compact IRI type such as
    `gx:ParticipantComplianceCredential`, because the DCP TCK's scope
    pattern `vc.type:(?<type>[^:]+)(?<suffix>:.+)?` forbids `:` in the
    type; Portitor uses the matching rule of
    [ADR 0021](adr/0021-gaia-x-onboarding.md#credential-types-and-scopes).
  - Gaia-X Trust Protocol and DCP: how a compliance credential is bound
    to its holder over DCP (ADR 0022).
- **Why it matters.** Until upstream decides, Portitor's workarounds may
  differ from what others do, and the next specification release may
  break them.
- **Current state: Open.** The roadmap says the maintainer files them and
  their links are added; no link is in the repository (checked).
- **Next.** File them; add each link to the ROADMAP and to the code that
  works around it.

<a id="r-08"></a>
### R-08 The DSP TCK v1.0.2 needs a test-only build

- **What.** (1) A race in the TCK's consumer: a start message sent at
  once can be refused and every retry gets 404. (2) The TCK sends one
  fixed `Authorization` header, which cannot be a DCP token. (3) The TCK
  itself disables TP:02-04 and TP_C:02-04.
- **Why it matters.** The conformance run does not test exactly the
  release build.
- **Current state: Mitigated.** The `tck` feature, never in release
  images: start messages 300 ms late (`tck::LateStart`, ADR 0014) and the
  fixed header mapped to the participant id `TCK_PARTICIPANT`, the id the
  TCK's own messages use (ADR 0016, amended 2026-11-13; the first text
  said "a test DID"). DCP authentication is covered by the DCP TCK and
  the EDC interop test instead.
- **Next.** Report the race (R-07); drop `LateStart` when a fixed TCK
  ships.

<a id="r-09"></a>
### R-09 EDC is tested at a release candidate (1.0.0-RC7)

- **What.** The EDC interop test runs EDC 1.0.0-RC7
  (`interop/edc/runtimes/gradle.properties`). The plan was to pin 1.0.0
  when it ships (ROADMAP 0.5). The IdentityHub Identity API the test uses
  is marked unstable in RC7 ([TESTING](TESTING.md#edc-interop-test)).
  EDC's reading of a specification can change (ADR 0011).
- **Why it matters.** Interoperability with the final EDC release is not
  proven.
- **Current state: Mitigated.** The test runs on pull requests that
  touch the code and nightly, to catch changes on EDC's side. Whether EDC
  1.0.0 has shipped was not checked for this file.
- **Next.** Pin EDC 1.0.0 when it ships and run the test again.

<a id="r-10"></a>
### R-10 EDC refuses the Gaia-X compliance credential

- **What.** EDC 1.0.0-RC7 applies its subject check
  (`HasValidSubjectIds`) to every credential of a DCP presentation. The
  Gaia-X compliance credential's subject is not the holder's DID, so EDC
  refuses it ([ADR 0022](adr/0022-gaia-x-compliance-in-policies.md)).
- **Why it matters.** Gaia-X policies work only between Portitor
  connectors. An EDC provider cannot ask Portitor for them, and EDC
  cannot meet them.
- **Current state: Accepted** (maintainer, 2026-11-15, 0.8 plan). The EDC
  interop test runs without Gaia-X; DEPLOYING "Known limits" says so.
- **Next.** Later: ask EDC how it would accept Gaia-X compliance
  credentials (ROADMAP "Later", Gaia-X beyond 0.8).

<a id="r-11"></a>
### R-11 Gaia-X does not use DCP; Portitor's binding and scope rules are its own reading

- **What.** Gaia-X supports VC-API style REST and OpenID4VC, not DCP
  (ADR 0021). No specification says how to present a Gaia-X compliance
  credential over DCP. Portitor ties it to the caller through the
  `digestSRI` of the caller's `gx:LegalPerson` (ADR 0022) and matches a
  scope type `T` to `gx:T` (ADR 0021).
- **Why it matters.** Other implementations may choose differently, and
  a later Gaia-X or DCP text may contradict ours.
- **Current state: Open.** Both rules are in ADRs that are still
  Proposed (TD-23), tested between Portitor connectors only.
- **Next.** The upstream reports (R-07).

<a id="r-12"></a>
### R-12 Gaia-X never tested with a real trust anchor or the Gaia-X Lab

- **What.** The Gaia-X end-to-end test uses a local compliance service in
  test mode: `DISABLE_SIGNATURE_CHECK=true` (certificate chains not
  checked against Gaia-X trust anchors) and `production=false` (any
  registration number issuer accepted), with a test CA and a test notary
  (`interop/gaiax/compose.yaml`). No run against the Gaia-X Lab or a
  production clearing house: Portitor runs only on the maintainer's
  laptop, and that needs a public `did:web` and a CA certificate
  (maintainer, 2026-11-15).
- **Why it matters.** The real trust chain and a real notary are not
  proven; the Lab may expect another ontology version (spike 0.8, "Not
  checked").
- **Current state: Open.** [TESTING](TESTING.md#gaia-x-end-to-end-test)
  lists what the test does not prove.
- **Next.** ROADMAP "Later": a test against the Lab once Portitor runs
  beyond a laptop.

<a id="r-13"></a>
### R-13 The Gaia-X test depends on Gaia-X and w3id.org hosts

- **What.** The local compliance service loads shapes, ontology and
  contexts from `registry.lab.gaia-x.eu` (through `w3id.org`) and
  `www.w3.org`, with Node.js clients that ignore `HTTPS_PROXY` (spike
  0.8, finding 6).
- **Why it matters.** A change or outage there breaks the test with no
  change in Portitor.
- **Current state: Mitigated.** The images are pinned by digest; the
  workflow runs nightly (`.github/workflows/gaiax.yml`) so a break shows
  apart from code changes.
- **Next.** None planned.

<a id="r-14"></a>
### R-14 Gaia-X protocols move: the Compliance Dispatcher is announced, not released

- **What.** The Gaia-X Compliance Dispatcher's wallet flow (OpenID4VP and
  OpenID4VCI) is announced (Gaia-X, 2026-07-24), not released. Portitor
  talks to the clearing house with its REST API (ADR 0021). The service
  writes `iat` and `exp` in milliseconds (spike 0.8).
- **Why it matters.** The REST API may be replaced.
- **Current state: Mitigated.** The service is a port
  (`GaiaxCompliance`) with an HTTP adapter, so OpenID4VCI can be a
  second adapter. Validity is read from `validFrom` and `validUntil`
  only.
- **Next.** ROADMAP "Later" (TD-21).

<a id="r-15"></a>
### R-15 Onboarding accepts a compliance answer from any issuer whose signature verifies

- **What.** During Gaia-X onboarding the identity resolves the DID named
  as `issuer` in the compliance service's answer and checks the signature
  with it (`crates/portitor-identity/src/gaiax.rs`, checked). It does not
  check that this issuer is the compliance service it called.
- **Why it matters.** Whoever answers at the configured URL could return
  a credential signed by any DID. The risk is limited: the URL is set by
  the operator and called over HTTPS, the answer must list exactly the
  digests sent, and connectors trust only the services in
  `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` (ADR 0022), so a credential from
  another issuer is skipped by verifiers.
- **Current state: Open.** Raised and deferred in the 0.8 review
  (maintainer, 2026-11-15, project chat).
- **Next.** Proposed for 0.10: pin the expected compliance issuer DID in
  the identity's settings.

<a id="r-16"></a>
### R-16 A Gaia-X compliance credential cannot be revoked

- **What.** The compliance credential has no `credentialStatus`; it is
  valid up to 90 days (spike 0.8). The caller's `gx:LegalPerson` status
  is not fetched, because the caller controls it (ADR 0022).
- **Why it matters.** A participant that loses its compliance keeps it
  until it expires.
- **Current state: Accepted** (ADR 0022, maintainer, 2026-11-15): the
  compliance credential's validity bounds it, and `validUntil` is
  required.
- **Next.** Follow Gaia-X if it adds a status mechanism.

<a id="r-17"></a>
### R-17 Server-side request forgery through partner-chosen URLs

- **What.** The connector POSTs to the partner's `callbackAddress` and
  fetches DID documents and credential services the caller chooses
  (ADR 0016).
- **Why it matters.** A partner could make the connector call an internal
  service.
- **Current state: Mitigated.** Only `https://` (unless
  `PORTITOR_ALLOW_HTTP_CALLBACKS` for tests), no redirects, time and size
  limits, the callback's answer body is never read, a 10 s timeout;
  DEPLOYING asks for an egress firewall. Internal addresses are not
  blocked (checked: no such check in the code).
- **Next.** Block internal addresses with an operator allow list (ADR
  0016, "roadmap, later"); not in the roadmap's lists yet.

<a id="r-18"></a>
### R-18 No request size limit of Portitor's own on the public ports

- **What.** SECURITY-ASSURANCE lists request size limits as "planned,
  0.9".
- **Why it matters.** Oversized requests can exhaust memory.
- **Current state: Open.** The public ports have a 60 s limit per request
  and at most 256 requests at once (DEPLOYING, section 5); presentations,
  status lists (16 MiB after decompression) and bundles have their own
  limits. Portitor's code sets no body limit of its own (checked); what
  the HTTP framework's defaults cover was not checked.
- **Next.** Add it to ROADMAP 0.9 or 0.10, which do not list it today.

<a id="r-19"></a>
### R-19 Consumer's data access tokens in the SQLite file and the management API

- **What.** As consumer, the connector keeps each usable transfer's data
  address, token included, in its SQLite file, and shows it on the
  management API (ADR 0014).
- **Why it matters.** Whoever reads the file, its backups, or holds the
  API key can fetch the data until the transfer ends or the token
  expires.
- **Current state: Mitigated.** The token is removed when the transfer
  ends; tokens expire (`PORTITOR_DATA_TOKEN_SECONDS`, default one hour);
  only an HMAC of the API key is kept, compared in constant time;
  DEPLOYING (section 5) says to protect the volume and the key alike.
- **Next.** Encrypting the token in the file is later work (ADR 0014); not
  in the roadmap's lists yet.

<a id="r-20"></a>
### R-20 The issuer key is the root of trust of a data space

- **What.** The issuer key signs every credential and status list of the
  data space (ADR 0018). Whoever controls a DID in
  `PORTITOR_TRUSTED_ISSUERS` can admit any participant.
- **Why it matters.** A stolen key admits anyone; a lost key means
  every participant needs new credentials (TD-05).
- **Current state: Mitigated.** The key lives only in `portitor-issuer`,
  never on a participant's host (ADR 0003, DEPLOYING section 7, with
  backup advice). The bundle has its own key, kept offline (ADR 0005).
- **Next.** Key custody (TD-05).

<a id="r-21"></a>
### R-21 Bugs in Portitor's own SHACL engine, and new SHACL components in later profiles

- **What.** Portitor uses its own SHACL Core subset, `portitor-shacl`,
  not rudof, which fails `cargo deny` (spike 0.6, ADR 0005).
  HealthDCAT-AP release 8 already adds a SPARQL rule.
- **Why it matters.** A bug passes a bad dataset or refuses a good one; a
  shape the engine does not support cannot be enforced.
- **Current state: Mitigated.** The W3C SHACL tests for the implemented
  components and a comparison with the EU ITB validator on the spike
  samples (`crates/portitor-shacl/tests`); shapes with a term the engine
  does not implement are refused (`tests/refusal.rs`); a work limit
  (`MAX_CHECKS`) ends in a violation, never a pass; fuzz targets
  `shacl_load` and `shacl_jsonld`.
- **Next.** Per profile release, the maintainer decides whether to add a
  component or leave the rule out of the bundle. Look at rudof again if
  its HTTP and CLI dependencies become optional.

<a id="r-22"></a>
### R-22 Bundle: unsigned `latest.json`, and a revoked bundle key not seen while offline

- **What.** `latest.json` is not signed. While the authority cannot be
  reached, the connector reuses the cached bundle with the key it stored,
  so a bundle key revoked meanwhile is not noticed (SECURITY-ASSURANCE).
- **Why it matters.** An attacker on the path could hold back a newer
  bundle; a compromised bundle key keeps working on offline connectors.
- **Current state: Mitigated.** The version `latest.json` names is
  verified (signature by `<authority>#bundle-key`, every SHA-256); the
  connector never takes a version issued before the one in use; a
  version that cannot be taken leaves the one in use (ADR 0005).
- **Next.** Bundle expiry and bundle key rotation (TD-16).

<a id="r-23"></a>
### R-23 A wrong host clock gives wrong policy and token decisions

- **What.** `dateTime` constraints, token expiry and credential validity
  use the host clock (ADR 0009).
- **Why it matters.** Wrong decisions near a limit.
- **Current state: Mitigated.** DEPLOYING asks hosts to run NTP; 60 s
  clock skew is allowed for tokens.
- **Next.** None.

<a id="r-24"></a>
### R-24 Simpl-Open: parked, facts not verified, and it changes fast

- **What.** Simpl-Open's connector uses no DCP; agents authenticate with
  x509 and mTLS at their Tier 2 gateway
  ([ADR 0017](adr/0017-simpl-open.md), Proposed). Several facts could not
  be verified (ADR 0017, "Not verified"). Simpl-Open's schemas are not
  signed (ADR 0005).
- **Why it matters.** Portitor is not a Simpl participant; reference
  counterpart number two (ADR 0011) is untested.
- **Current state: Accepted** (parked by the maintainer, 2026-11-13,
  until a Simpl-Open sandbox exists). ROADMAP 0.10 still lists
  "Interoperability tests against ... Simpl-Open (with Gaia-X)", which
  the parking and R-10 contradict.
- **Next.** ROADMAP "Later"; re-check the facts before any Simpl work;
  update the 0.10 line.

<a id="r-25"></a>
### R-25 Name not checked for trademarks and domains

- **What.** `portitor` was free on crates.io (2026-11-12); domain names
  and a formal trademark search (EUIPO, WIPO, BOIP) are not done
  ([ADR 0008](adr/0008-project-name.md)).
- **Why it matters.** A rename after 1.0 is expensive.
- **Current state: Open.**
- **Next.** Before 1.0.

<a id="r-26"></a>
### R-26 ODRL `dateTime` definition read from memory

- **What.** ADR 0009 could not read the ODRL 2.2 definition of `dateTime`
  and used it from memory: "check before 1.0".
- **Why it matters.** Another connector may read it differently.
- **Current state: Open.**
- **Next.** Before 1.0.

<a id="r-27"></a>
### R-27 CI downloads from Maven Central and GitHub

- **What.** Maven Central answered HTTP 429 on 2026-11-15 when CI fetched
  the TCKs.
- **Why it matters.** Red CI with no code change.
- **Current state: Mitigated.** The DCP TCK jar is cached, keyed on its
  SHA-256 and checked after every restore; the DSP TCK build's Gradle
  dependencies are cached; downloads are retried (CHANGELOG 0.8.0).
- **Next.** None.

<a id="r-28"></a>
### R-28 0.8 Gaia-X code has no fuzz target

- **What.** TESTING says "a new parser of untrusted input gets a target".
  The 0.8 code that reads caller data (the Gaia-X checks and the RFC 8785
  JCS digest of the caller's credential, which parses numbers again)
  has none: no target in `fuzz/fuzz_targets` names Gaia-X or JCS
  (checked). The X.509 reader reads only the operator's own file.
- **Why it matters.** A crash or wrong digest on crafted input.
- **Current state: Open.** Unit tests cover the RFC 8785 examples and the
  spike's digests.
- **Next.** ROADMAP 0.10, "fuzzing on every parser".

<a id="r-29"></a>
### R-29 DCP costs about 2 ms per request and 13 times fewer requests under load

- **What.** Measured for 0.5 (ADR 0002, section 7): DCP adds about 2 ms
  per DSP request when idle, and under load the provider answers about
  13 times fewer requests. Each request costs a DID resolution, an STS
  token and a presentation query, and the issuer's status list is
  fetched every time.
- **Why it matters.** Capacity, and load on partners' identities and
  issuers.
- **Current state: Open.** DID documents are cached 5 minutes; nothing
  else (TD-06, TD-07).
- **Next.** Status list caching in 0.10 (TD-06); measure again.

<a id="r-30"></a>
### R-30 DSP callers were not authenticated

- **What.** Until 0.4 the connector believed any caller: the participant
  id, and so the agreement's `assignee`, was whatever the caller wrote
  (ADR 0007 consequences, ADR 0013).
- **Current state: Addressed** in 0.5 by
  [ADR 0016](adr/0016-dsp-caller-authentication.md): every DSP request
  carries a DCP token, the caller's DID is proven and needs a
  `MembershipCredential` from a trusted issuer, and each process is bound
  to the DID that started it.

<a id="r-31"></a>
### R-31 RSA timing advisory in the pure-Rust `rsa` crate

- **What.** Gaia-X signs with PS256; the pure-Rust `rsa` crate has a
  timing advisory (RUSTSEC-2023-0071) (ADR 0002).
- **Current state: Addressed** in the 0.0 bake-off: PS256 is verified
  with `ring`; no `rsa` crate in `Cargo.lock` (checked).

<a id="r-32"></a>
### R-32 XML parser advisories through RDF/XML code lists and rudof

- **What.** Code lists are published as RDF/XML, whose only Rust parser
  (`oxrdfxml`, through `quick-xml`) had open advisories; rudof brought
  them in too (spike 0.6).
- **Current state: Addressed** in 0.7: no Portitor binary parses RDF/XML;
  `bundle build` refuses it and code lists travel as reduced N-Triples
  ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)). No
  `quick-xml` or `oxrdfxml` in `Cargo.lock` (checked).

<a id="r-33"></a>
### R-33 Any caller, or one participant, could exhaust the issuer

- **What.** The 0.6 review found that any valid self-issued token made
  the issuer fetch a DID document and store rows, and one participant
  could fill a status list and stop issuance for everyone.
- **Current state: Addressed** in 0.6
  ([ADR 0018](adr/0018-issuer-component.md), "Amended during the 0.6
  review"): 401 for DIDs that are not registered, before anything is
  fetched; at most `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` active
  credentials per definition; random status list positions.

<a id="r-34"></a>
### R-34 Scorecard Branch-Protection and Fuzzing at 0

- **What.** OpenSSF Scorecard 6.7/10 at 0.1, with Branch-Protection and
  Fuzzing at 0.
- **Current state: Addressed** in 0.2 (ROADMAP 0.2 result): the `main`
  ruleset targeted no branch, fixed; cargo-fuzz with ClusterFuzzLite.
  Scorecard 7.9.

<a id="r-35"></a>
### R-35 The DCP TCK uses VCDM 1.1, Gaia-X needs VCDM 2.0

- **What.** The DCP TCK v1.2.1 uses only VCDM 1.1 credentials, so a
  `vc20-bssl/jwt`-only identity could not pass it, while Gaia-X needs
  VCDM 2.0 (ADR 0015).
- **Current state: Addressed** in 0.5: both DCP profiles,
  `vc11-sl2021/jwt` and `vc20-bssl/jwt` (maintainer, 2026-11-13).

## Technical debt

<a id="td-01"></a>
### TD-01 How schemas and vocabularies are collected and stored

Raised by the maintainer, 2026-11-14 (to be investigated further).
Status: **Open**, no target.

In 0.7 the authority publishes a signed bundle in a format Portitor
defines (`manifest.json`, `manifest.jws`, `latest.json`;
[ADR 0005](adr/0005-schema-and-vocabulary-registry.md)), and the
connector pulls it into a disk cache (`portitor-bundle`). Only its
version fields come from a standard (W3C DCAT 3 `dcat:version` and
`dcterms:issued`). No data space specification we follow defines how an
authority publishes schemas: Simpl-Open has its own schema-manager API
(versions numbered by the authority, pulled by a sync adapter, not
signed), and its versioning requirements are still Proposed. Portitor
must not invent a new standard, so the format may change when this is
decided. To investigate: how Simpl-Open, Gaia-X and HealthData@EU
authorities publish schemas, whether a DCAT catalogue of schema
distributions can replace `latest.json`, and where the connector should
keep them.

Mitigation today: the source and the store are their own crate,
`portitor-bundle`, so they can be replaced as a whole (ADR 0005).

<a id="td-02"></a>
### TD-02 Who calls the data plane, and with which credentials

Raised by the maintainer, 2026-11-15 (to be investigated, for a future
release). Status: **Open**, no target.

Today every credential is system to system (see "Who holds the
credentials" in [ARCHITECTURE](ARCHITECTURE.md#who-holds-the-credentials)).
Only participants are identified, and only in the control plane: every
DSP request carries a DCP token, and the connector checks the caller's
DID and credentials ([ADR 0016](adr/0016-dsp-caller-authentication.md),
[ADR 0019](adr/0019-credential-constraints.md)). The data plane checks
only the bearer token handed out when the transfer started
([ADR 0013](adr/0013-transfer-and-data-plane.md)): whoever holds the
token gets the data until the transfer ends. No identity or credential
is checked on each data request, and the person or application behind
the consumer is never identified. To investigate: how a consumer's users
and applications prove who they are and which credentials they hold when
they call the data plane (for example a DCP token or a verifiable
presentation per request, OpenID4VP as Gaia-X Danube and the EU Digital
Identity Wallet use it, Simpl-Open's mTLS at its Tier 2 gateway, what
the EDC data plane does); whether the policy is checked again on each
request; and how this fits the consent and API gateway items under
"Later" in the [roadmap](ROADMAP.md#later-not-planned-for-a-release-yet).
Standards check and an ADR before any code.

<a id="td-03"></a>
### TD-03 A provider in several data spaces, each with its own rules

Raised by the maintainer, 2026-11-15 (to be investigated). Status:
**Open**, no target.

A data provider may belong to several data spaces at once: long-lived
ones that may or may not interoperate with each other (for example EHDS
and ICUDATA4EU), and short-lived ones set up for one use case. Each data
space authority may set its own rules, not only for dataset descriptions
(schemas and vocabularies) but also for offers: which policy
requirements and constraints apply, which credentials a consumer must
hold, and where data may be processed (for example the data processing
address or location). The same dataset may need a different description
and a different offer in each data space. Today one connector serves one
data space: it pulls one signed bundle from one authority
(`PORTITOR_BUNDLE_URL`, [ADR 0005](adr/0005-schema-and-vocabulary-registry.md)),
checks every dataset against that one bundle
([ADR 0020](adr/0020-dataset-harvest.md)), and trusts one list of
credential issuers ([ADR 0016](adr/0016-dsp-caller-authentication.md)).
Offers take their policies from the datasets file, with the left
operands of ADR 0009 and ADR 0019 only. To investigate: one connector
with several data space profiles (bundle, trusted issuers, policy
vocabulary and offer rules per data space) or one connector per data
space; how a dataset says which data spaces it is offered in, and which
schema and offer it has in each; how each data space expresses
constraints such as the processing location (ODRL profiles, the data
space's rulebook); how a caller's data space is known (its credentials or
the catalog it asks); how EHDS and other data spaces publish their
schemas; how short-lived data spaces are joined and left. Standards
check and an ADR before any code.

<a id="td-04"></a>
### TD-04 User to system authentication and authorisation, governed per data space

Raised by the maintainer, 2026-11-15 (to be investigated). Status:
**Open**, no target.

Today every credential is system to system: held by a participant (an
organisation with one DID) and checked between connectors (see "Who
holds the credentials" in [ARCHITECTURE](ARCHITECTURE.md#who-holds-the-credentials)).
The people and applications inside an organisation are not identified:
the management APIs use one API key per component, and the data plane
only checks the transfer's bearer token. To investigate:

- authentication: how a person or an application of a participant proves
  who it is to the connector, the identity and the data plane (for
  example OpenID Connect with the organisation's own identity provider,
  OpenID4VP with the EU Digital Identity Wallet, what Simpl-Open and EDC
  do), and how that links to the participant's DID;
- authorisation: who may do what (read the catalog, negotiate, start a
  transfer, read data, manage the connector), whether a contract agreed
  between participants can be narrowed to some users or roles, and
  whether ODRL policies may name them (party collections, constraints on
  the user's credentials);
- governance: which of these rules each data space authority sets (the
  accepted identity providers or wallets, the roles, the credentials a
  user needs, audit and consent, for example EHDS rules on who may see
  health data) and how the authority distributes them, likely with the
  bundle of [ADR 0005](adr/0005-schema-and-vocabulary-registry.md);
- several data spaces: a participant in several data spaces must meet
  different user rules in each (see [TD-03](#td-03)), so the rules are
  per data space, not per connector.

Linked to [TD-02](#td-02). Standards check and an ADR before any code.

<a id="td-05"></a>
### TD-05 Key custody and key rotation

Raised in ADR 0015 (0.5: "no key rotation API", rotation manual) and
ADR 0018 (0.6); recorded by the maintainer at the 0.7 close-out,
2026-11-15 (a key custody service such as OpenBao is probably better).
Status: **Open**; whether it lands with the 0.10 hardening work is part
of the investigation (ROADMAP; ADR 0021 says "key rotation stays in
0.10").

Today each component (identity, issuer, bundle signing) reads one P-256
private key from a PEM file and lists one key in its `did:web` document,
with the key's thumbprint as its id. Rotation is manual and leaves a
gap: the old key leaves the DID document at once, so anything it signed
stops verifying. For the identity that is tokens and presentations until
verifiers reload the document (Portitor caches it 5 minutes); for the
issuer it is every credential and status list it ever signed, so every
participant needs new credentials.
[ADR 0015](adr/0015-dcp-identity-component.md) describes "add the new
key, switch, remove the old key", but the code cannot list two keys, so
that overlap is not possible yet. Since 0.8 a key change also means a new
Gaia-X compliance run, because the self-signed Gaia-X credentials stop
verifying ([ADR 0021](adr/0021-gaia-x-onboarding.md)); renewing the
certificate on the same key causes no gap. To investigate: a signer
port with a key custody adapter (OpenBao transit engine: P-256 signing,
versioned keys, the private key never leaves it), listing retired public
keys next to the current one in the DID document (DID Core allows
several verification methods), how EDC IdentityHub rotates keys, and
whether this lands with the 0.10 hardening work.

<a id="td-06"></a>
### TD-06 Status lists are not cached in the connector's DCP verifier

- **What.** The verifier fetches the issuer's status list on every
  request ([ADR 0016](adr/0016-dsp-caller-authentication.md):
  "Presentations are not cached").
- **Why it matters.** Cost per request (R-29) and load on the issuer.
- **Current state: Open.** No status list cache in the code (checked: no
  `PORTITOR_STATUS_LIST_CACHE_SECONDS`).
- **Next.** ROADMAP 0.10 (maintainer, 2026-11-14):
  `PORTITOR_STATUS_LIST_CACHE_SECONDS`, default 300, 0 checks on every
  request, at most 3600; only lists that were fetched and verified are
  cached, never longer than the list's own expiry; the trade-off (a
  revocation takes up to that long to be seen) goes in DEPLOYING. EDC
  RC7 caches 15 minutes by default. Demos, the EDC test and the TCK runs
  use 0. Amends ADR 0016.

<a id="td-07"></a>
### TD-07 STS tokens and presentations are not cached

- **What.** Each outgoing DSP request costs one STS call; each incoming
  one a presentation query (DEPLOYING, "Known limits").
- **Why it matters.** Cost per request (R-29).
- **Current state: Open.** ADR 0016 chose "measure before adding more
  caching" so a revocation takes effect at once.
- **Next.** Decide after TD-06 is measured.

<a id="td-08"></a>
### TD-08 One replica per SQLite file; no PostgreSQL adapter

- **What.** Each image keeps its state in one SQLite file; several
  replicas need the PostgreSQL adapter, which comes later on the same
  ports and migrations ([ADR 0006](adr/0006-negotiation-storage.md)).
- **Why it matters.** No horizontal scaling or failover; ARCHITECTURE
  principle 8 is not met yet.
- **Current state: Accepted** for 0.x (maintainer, ADR 0006,
  2026-11-12). Migrations are plain, portable SQL; ports are ready
  (`NegotiationStore` and the others). No PostgreSQL code exists.
- **Next.** Later; no release named.

<a id="td-09"></a>
### TD-09 Used token ids are kept in memory only

- **What.** The `jti` replay list lives in memory (ADR 0015 and ADR 0016,
  amended 2026-11-13), at most 100,000 entries.
- **Why it matters.** A restart forgets them; it only works with one
  replica.
- **Current state: Accepted** (maintainer, 0.5 review): tokens are
  short-lived (at most an hour), so the window is small.
- **Next.** Revisit with TD-08.

<a id="td-10"></a>
### TD-10 Transfers: pull only, no token renewal, no push to object storage

- **What.** HTTP pull only; the data token is not renewed (DSP 2025-1 has
  no message for it), the consumer asks for a new transfer; only `GET`
  of the whole source, without credentials
  ([ADR 0013](adr/0013-transfer-and-data-plane.md)). Push to
  S3-compatible object storage is in the roadmap's "Later" (maintainer,
  2026-11-13). The consumer side has no data plane (ADR 0014).
- **Why it matters.** Long or large transfers, and data spaces that use
  push (Simpl-Open documents S3 push and Kafka).
- **Current state: Open.**
- **Next.** Later: push as a second data plane behind the Signaling
  port.

<a id="td-11"></a>
### TD-11 Management API: no suspend or start again; consumer refuses offers with credential constraints

- **What.** The consumer's management API completes or terminates a
  transfer, but does not suspend or start it again; the provider cannot
  suspend or complete from the management API (ADR 0013, ADR 0014). As
  consumer, the connector does not accept a provider's offer (DSP
  `OFFERED`) that has credential constraints (DEPLOYING "Known limits").
- **Why it matters.** The domain supports these steps, but operators
  cannot use them.
- **Current state: Open.**
- **Next.** No release named.

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
- **Next.** No release named; see also TD-03 (processing location).

<a id="td-13"></a>
### TD-13 The catalog shows every offer to every member

- **What.** Credential constraints are checked at negotiation and
  transfer, not when the catalog is built; EDC's "access policy" is
  later work ([ADR 0019](adr/0019-credential-constraints.md)).
- **Why it matters.** Members see offers they cannot meet.
- **Current state: Open.**
- **Next.** No release named.

<a id="td-14"></a>
### TD-14 Credential schemas, presentation definitions and issuance policies are not supported

- **What.** `credentialSchema` is not checked (DEPLOYING "Known limits");
  a presentation query with only a `presentationDefinition` gets an error
  (ADR 0015; the DCP TCK does not test it yet); the issuer has no
  approval queue and no "present another credential first"
  (`issuancePolicy`, [ADR 0018](adr/0018-issuer-component.md)).
- **Why it matters.** Data spaces that use them cannot use Portitor as
  is.
- **Current state: Open.**
- **Next.** No release named; the issuer's decision port allows it
  later.

<a id="td-15"></a>
### TD-15 Agreements are not revised when the data's scope changes

- **What.** An agreement keeps working after its dataset leaves the
  source; there is no way to revise it when a data subject objects or
  the consumer asks for a new scope (maintainer, 2026-11-14;
  [ADR 0020](adr/0020-dataset-harvest.md)).
- **Why it matters.** Consent withdrawal and scope changes.
- **Current state: Open.**
- **Next.** ROADMAP "Later".

<a id="td-16"></a>
### TD-16 One harvest source; no data space registry; other catalogs not checked; no bundle expiry or bundle key rotation

- **What.** Datasets come only from the datasets file
  ([ADR 0020](adr/0020-dataset-harvest.md)); a data space wide registry
  or federated catalogue is "only if needed"; other participants'
  catalogs are not checked against the bundle; the bundle has no expiry
  and its key no rotation ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)).
- **Why it matters.** Organisations keep datasets in DCAT-AP endpoints
  or CKAN; see also R-22.
- **Current state: Open.** The `DatasetSource` port takes new adapters
  without other changes.
- **Next.** ROADMAP "Later" (maintainer, 2026-11-14).

<a id="td-17"></a>
### TD-17 The data plane runs inside the connector process

- **What.** Data traffic and protocol logic share CPU, memory and network
  exposure; they cannot be scaled apart, and a data handling bug can take
  negotiation down
  ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate),
  [ADR 0003](adr/0003-component-split.md)).
- **Current state: Accepted** (maintainer, 2026-11-11, option A for
  0.x), behind the Signaling ports (R-06).
- **Next.** Split it out as an optional component once Data Plane
  Signaling is released, or when measured load needs it.

<a id="td-18"></a>
### TD-18 Operations: no metrics, traces or readiness check

- **What.** `GET /health` answers while the process runs and does not
  check the database or partners; there is no metrics endpoint,
  OpenTelemetry or readiness endpoint (checked in the code). ARCHITECTURE
  principle 11 asks for them.
- **Current state: Open.** Structured JSON logs exist.
- **Next.** ROADMAP 0.9 (Operations).

<a id="td-19"></a>
### TD-19 No TLS inside the services

- **What.** The services speak plain HTTP; operators run them behind a
  TLS-terminating proxy. The management ports must stay on a private
  network (SECURITY-ASSURANCE, DEPLOYING).
- **Current state: Open.** "Not on the roadmap yet"
  (SECURITY-ASSURANCE).
- **Next.** Decide before 1.0 whether it stays the operator's job.

<a id="td-20"></a>
### TD-20 A holder must delete a revoked credential by hand after a reissue

- **What.** Verifiers refuse a presentation that holds any revoked
  credential (DCP v1.0.1, 5.4.3, step 8). The identity does not read
  status lists, so it keeps presenting a revoked credential next to its
  reissued one (DEPLOYING, sections 4 and 5).
- **Why it matters.** A participant is refused until its operator
  deletes the revoked credential.
- **Current state: Open.** Documented in DEPLOYING.
- **Next.** No release named.

<a id="td-21"></a>
### TD-21 Gaia-X beyond 0.8

- **What.** Not in 0.8 (ROADMAP 0.8 "Not in 0.8", ADR 0021): the Gaia-X
  Lab or a production clearing house (R-12); Gaia-X DCAT and service
  offering credentials with a federated catalogue; OpenID4VCI and
  OpenID4VP (R-14); label levels 1 to 3; calling the registration number
  notary (the operator gets that credential by hand); trust anchors and
  compliance issuers read from the Gaia-X Registry; Gaia-X with EDC
  (R-10).
- **Current state: Open.**
- **Next.** ROADMAP "Later", Gaia-X beyond 0.8 (maintainer, 2026-11-15).

<a id="td-22"></a>
### TD-22 Migrations run forward only; a rollback needs the backup

- **What.** An older version refuses a database a newer one migrated;
  going back means restoring the backup, and data written since is lost
  (DEPLOYING, "Upgrade and rollback").
- **Current state: Accepted** (DEPLOYING, since 0.3; each upgrade note
  says which database moves).
- **Next.** Revisit at the 0.10 API and config freeze.

<a id="td-23"></a>
### TD-23 ADRs 0021 and 0022 are built but still Proposed

- **What.** The 0.8 code implements
  [ADR 0021](adr/0021-gaia-x-onboarding.md) and
  [ADR 0022](adr/0022-gaia-x-compliance-in-policies.md), but both say
  "Status: Proposed". ADR 0017 (Simpl-Open) is Proposed too, on purpose
  (R-24).
- **Why it matters.** The rules in force are not yet accepted decisions.
- **Current state: Open.** Earlier releases accepted their ADRs at the
  close-out (0.6: ADRs 0018 and 0019; 0.7: ADRs 0005 and 0020).
- **Next.** The maintainer accepts or amends them at the 0.8 close-out.

<a id="td-24"></a>
### TD-24 A 3 second wait for acknowledgements; messages lost at a restart

- **What.** In 0.3 a partner message that raced our acknowledgement
  waited up to 3 s; a message being sent when the process stopped was
  lost; a failed send ended the negotiation even if the partner had the
  message (ADR 0006 consequences; "a delay is a remediation, not a fix",
  maintainer, 2026-11-12).
- **Current state: Addressed** in 0.3 by
  [ADR 0007](adr/0007-outbox-and-proof-of-delivery.md): outbox, version
  check, proof of delivery, state check after a lost answer, resend at
  start. Transfers follow it (ADR 0013, ADR 0014).

<a id="td-25"></a>
### TD-25 A test-only tool signed the demo credentials

- **What.** Until the issuer existed, the `test-issuer` example signed
  credentials for the demos and the EDC test (ADR 0015).
- **Current state: Addressed** in 0.6: `portitor-issuer`
  ([ADR 0018](adr/0018-issuer-component.md)); the example is removed
  (checked: no `crates/portitor-identity/examples`).

<a id="td-26"></a>
### TD-26 One bad dataset stopped the connector at start

- **What.** Until 0.6 a dataset that broke a rule stopped the start, so
  one wrong dataset took the whole catalog down.
- **Current state: Addressed** in 0.7 by
  [ADR 0020](adr/0020-dataset-harvest.md): each dataset is checked on its
  own, a bad one is rejected and logged in the harvest job.

<a id="td-27"></a>
### TD-27 The domain crates' I/O check named too few crates

- **What.** The CI check refused only axum, hyper, tokio, sqlx and
  reqwest in domain crates.
- **Current state: Addressed** on 2026-11-14
  ([ADR 0004](adr/0004-hexagonal-architecture.md), amended): it also
  refuses hyper-util, hyper-rustls, mio, socket2, ureq, rusqlite,
  libsqlite3-sys, postgres and rustls (`.github/workflows/ci.yml`).
