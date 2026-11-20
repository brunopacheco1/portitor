<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Risks and technical debts

This file collects every risk and technical debt raised so far, from
release 0.0 on, in one place: the ADRs, the spikes, the roadmap, the
security case, DEPLOYING, TESTING, the changelog, the commit messages,
the [reviews](reviews/), and what the maintainer said in the project chat. For each item it says
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
| [R-02](#r-02) | One maintainer: no two-person review | Delivery, security | 0.1 (ADR 0012, maintainer, 2026-11-11) | Accepted | until a second maintainer (not a 1.0 blocker) |
| [R-03](#r-03) | Pre-1.0 JSON-LD and VC crates | Dependency | 0.0 (ADR 0002, 2026-11-11) | Mitigated | — |
| [R-04](#r-04) | Dependencies are not audited crate by crate (cargo-vet) | Dependency, security | 0.1 (maintainer, 2026-11-11 and 2026-11-12) | Mitigated | 0.10 |
| [R-05](#r-05) | No written OpenSSF Baseline level 1 self-assessment | Security | 0.1 (ADR 0012, maintainer, 2026-11-12) | Open | 0.13 |
| [R-06](#r-06) | Data Plane Signaling is a draft (v1.0-RC4) | Upstream churn | 0.0 (ARCHITECTURE, 2026-11-11) | Mitigated | until Signaling 1.0 |
| [R-07](#r-07) | Upstream findings not reported yet | Upstream churn, interoperability | 0.4 to 0.8 (maintainer, 2026-11-13 to 2026-11-15) | Addressed | 0.10 |
| [R-08](#r-08) | The DSP TCK v1.0.2 needs a test-only build | Interoperability | 0.4, 0.5 (ADR 0014, ADR 0016, 2026-11-13) | Mitigated | — |
| [R-09](#r-09) | EDC is tested at a release candidate (1.0.0-RC7) | Interoperability, dependency | 0.5 (maintainer, 2026-11-13) | Mitigated | pin 1.0.0 when it ships |
| [R-10](#r-10) | EDC refuses the Gaia-X compliance credential | Interoperability | 0.8 (ADR 0022, maintainer, 2026-11-15) | Accepted | Later |
| [R-11](#r-11) | Gaia-X does not use DCP; Portitor's binding and scope rules are its own reading | Interoperability, upstream churn | 0.8 (ADR 0021, ADR 0022, 2026-11-15) | Open | — |
| [R-12](#r-12) | Gaia-X never tested with a real trust anchor or the Gaia-X Lab | Interoperability, security | 0.8 (maintainer, 2026-11-15) | Open | Later |
| [R-13](#r-13) | The Gaia-X test depends on Gaia-X and w3id.org hosts | Delivery | 0.8 (spike, TESTING, 2026-11-15) | Mitigated | — |
| [R-14](#r-14) | Gaia-X protocols move: the Compliance Dispatcher (OpenID4VP, OpenID4VCI) is announced, not released | Upstream churn | 0.8 (ADR 0021, 2026-11-15) | Mitigated | Later |
| [R-15](#r-15) | Onboarding accepts a compliance answer from any issuer whose signature verifies | Security | 0.8 review (maintainer, 2026-11-15) | Addressed | 0.8 |
| [R-16](#r-16) | A Gaia-X compliance credential cannot be revoked | Security | 0.8 (spike, ADR 0022, 2026-11-15) | Accepted | — |
| [R-17](#r-17) | Server-side request forgery through partner-chosen URLs | Security | 0.3 (ADR 0016 security notes, SECURITY-ASSURANCE) | Mitigated | — |
| [R-18](#r-18) | No request size limit of Portitor's own on the public ports | Security | 0.5 (SECURITY-ASSURANCE) | Addressed | done in 0.11 |
| [R-19](#r-19) | Consumer's data access tokens in the SQLite file and the management API | Security | 0.4 (ADR 0014, 2026-11-13) | Mitigated | — |
| [R-20](#r-20) | The issuer key is the root of trust of a data space | Security | 0.6 (ADR 0018, 2026-11-14) | Mitigated | — |
| [R-21](#r-21) | Bugs in Portitor's own SHACL engine, and new SHACL components in later profiles | Security, upstream churn | 0.6 spike (ADR 0005, 2026-11-14) | Mitigated | — |
| [R-22](#r-22) | Bundle: unsigned `latest.json`, and a revoked bundle key not seen while offline | Security | 0.7 (ADR 0005, SECURITY-ASSURANCE) | Mitigated | — |
| [R-23](#r-23) | A wrong host clock gives wrong policy and token decisions | Security | 0.3 (ADR 0009, 2026-11-12) | Mitigated | — |
| [R-24](#r-24) | Simpl-Open: parked, facts not verified, and it changes fast | Interoperability, upstream churn | 0.5 (ADR 0017, maintainer, 2026-11-13) | Accepted | Later |
| [R-25](#r-25) | Name not checked for trademarks and domains | Delivery, legal | 0.3 (ADR 0008, 2026-11-12) | Open | 0.13 (blocks 1.0) |
| [R-26](#r-26) | ODRL `dateTime` definition read from memory | Interoperability | 0.3 (ADR 0009, 2026-11-12) | Open | 0.13 |
| [R-27](#r-27) | CI downloads from Maven Central and GitHub | Delivery | 0.8 (2026-11-15) | Mitigated | — |
| [R-28](#r-28) | 0.8 Gaia-X code had no fuzz target | Security | 0.8 (sweep for this file, 2026-11-15) | Addressed | 0.8 |
| [R-29](#r-29) | DCP costs about 2 ms per request and 13 times fewer requests under load | Performance | 0.5 (ADR 0002 section 7, 2026-11-13) | Open (measured again in 0.12) | with TD-07 |
| [R-30](#r-30) | DSP callers were not authenticated | Security | 0.3 (ADR 0007, 2026-11-12) | Addressed | done in 0.5 |
| [R-31](#r-31) | RSA timing advisory in the pure-Rust `rsa` crate | Security, dependency | 0.0 (ADR 0002, 2026-11-11) | Addressed | done in 0.0 |
| [R-32](#r-32) | XML parser advisories through RDF/XML code lists and rudof | Security, dependency | 0.6 spike (2026-11-14) | Addressed | done in 0.7 |
| [R-33](#r-33) | Any caller, or one participant, could exhaust the issuer | Security | 0.6 review (ADR 0018, 2026-11-14) | Addressed | done in 0.6 |
| [R-34](#r-34) | Scorecard Branch-Protection and Fuzzing at 0 | Security | 0.1 (2026-11-12) | Addressed | done in 0.2 |
| [R-35](#r-35) | The DCP TCK uses VCDM 1.1, Gaia-X needs VCDM 2.0 | Interoperability | 0.5 (ADR 0015, 2026-11-13) | Addressed | done in 0.5 |
| [R-36](#r-36) | The OpenTelemetry Rust crates are 0.x and change their API often | Dependency, upstream churn | 0.9 (ADR 0023, 2026-11-15) | Mitigated | — |
| [R-37](#r-37) | Telemetry costs some throughput at saturation | Performance | 0.9 spike (ADR 0023, 2026-11-15) | Mitigated | — |
| [R-38](#r-38) | No specification names an S3 data address | Interoperability | plan after 0.13 (maintainer, 2026-11-15) | Open | 1.1 |
| [R-39](#r-39) | No specification defines agreements that depend on an upstream agreement | Interoperability | plan after 0.13 (maintainer, 2026-11-15) | Open | 1.3 |
| [R-40](#r-40) | No shared list of EHDS purposes | Interoperability | plan after 0.13 (maintainer, 2026-11-15) | Open | 1.2 |
| [R-41](#r-41) | A key removed from a DID document is still trusted for up to 5 minutes | Security | 0.11 threat model review (F-04, 2026-11-17) | Mitigated (0.12) | — |
| [R-42](#r-42) | The STS shares the identity's public port | Security | 0.11 threat model review (F-05, 2026-11-17) | Open | — |
| [R-43](#r-43) | Negative credential constraints can be met with another credential | Security | 0.11 threat model review (F-07, 2026-11-17) | Open | — |
| [R-44](#r-44) | One slow holder delays every issuer delivery | Security | 0.11 threat model review (F-09, 2026-11-17) | Open | — |
| [R-45](#r-45) | No request limit per partner | Security | 0.11 threat model review (F-10, 2026-11-17) | Open | — |
| [TD-01](#td-01) | How schemas and vocabularies are collected and stored | Technical debt | 0.7 (maintainer, 2026-11-14) | Open | research, with TD-03 |
| [TD-02](#td-02) | Who calls the data plane, and with which credentials | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | research; code 1.5 |
| [TD-03](#td-03) | A provider in several data spaces, each with its own rules | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | research, before the 0.13 freeze |
| [TD-04](#td-04) | User to system authentication and authorisation, governed per data space | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | research before the 0.13 freeze; code 1.5 |
| [TD-05](#td-05) | Key custody and key rotation | Technical debt | 0.5 (ADR 0015); 0.7 close-out (maintainer, 2026-11-15) | Open | two keys listed done in 0.11; 1.4 (OpenBao) |
| [TD-06](#td-06) | Status lists are not cached in the connector's DCP verifier | Technical debt | 0.6 (maintainer, 2026-11-14) | Addressed | done in 0.11 |
| [TD-07](#td-07) | STS tokens and presentations are not cached | Technical debt | 0.5 (ADR 0016) | Open | 1.4, if 0.11 measurements need it |
| [TD-08](#td-08) | One replica per SQLite file; no PostgreSQL adapter | Technical debt | 0.3 (ADR 0006, maintainer, 2026-11-12) | Accepted | Later |
| [TD-09](#td-09) | Used token ids are kept in memory only | Technical debt | 0.5 (ADR 0015, ADR 0016, 2026-11-13) | Accepted | with TD-08 |
| [TD-10](#td-10) | Transfers: pull only, no token renewal, no push to object storage | Technical debt | 0.4 (ADR 0013, maintainer, 2026-11-12 and 2026-11-13) | Open | 1.1 |
| [TD-11](#td-11) | Management API: no suspend or start again; consumer refuses offers with credential constraints | Technical debt | 0.4 (ADR 0014); 0.6 (DEPLOYING) | Open | 1.1 and 1.2 |
| [TD-12](#td-12) | ODRL: few constraints, prohibitions and duties not enforced | Technical debt | 0.3 (ADR 0009, 2026-11-12) | Open | 1.2 (purpose) |
| [TD-13](#td-13) | The catalog shows every offer to every member | Technical debt | 0.6 (ADR 0019, 2026-11-14) | Open | 1.2 |
| [TD-14](#td-14) | Credential schemas, presentation definitions and issuance policies are not supported | Technical debt | 0.5, 0.6 (ADR 0015, ADR 0018) | Open | 1.4, if a target data space needs it |
| [TD-15](#td-15) | Agreements are not revised when the data's scope changes | Technical debt | 0.7 (maintainer, 2026-11-14) | Open | 1.5 |
| [TD-16](#td-16) | One harvest source; no data space registry; other catalogs not checked; no bundle expiry or bundle key rotation | Technical debt | 0.7 (ADR 0005, ADR 0020, 2026-11-14) | Open | Later |
| [TD-17](#td-17) | The data plane runs inside the connector process | Technical debt | 0.0 (maintainer, 2026-11-11) | Accepted | when Signaling 1.0 is released |
| [TD-18](#td-18) | Operations: no metrics, traces or readiness check | Technical debt | 0.1 (ARCHITECTURE principle 11) | Addressed | done in 0.9 |
| [TD-19](#td-19) | No TLS inside the services | Technical debt | 0.1 (SECURITY-ASSURANCE) | Open | 0.13 (decision) |
| [TD-20](#td-20) | A holder must delete a revoked credential by hand after a reissue | Technical debt | 0.6 (DEPLOYING, 2026-11-14) | Open | 1.4 |
| [TD-21](#td-21) | Gaia-X beyond 0.8 | Technical debt | 0.8 (maintainer, 2026-11-15) | Open | Later |
| [TD-22](#td-22) | Migrations run forward only; a rollback needs the backup | Technical debt | 0.3 (DEPLOYING) | Accepted | — |
| [TD-23](#td-23) | ADRs 0021 and 0022 were built but still Proposed | Technical debt | 0.8 (sweep for this file, 2026-11-15) | Addressed | 0.8 |
| [TD-24](#td-24) | A 3 second wait for acknowledgements; messages lost at a restart | Technical debt | 0.3 (ADR 0006, maintainer, 2026-11-12) | Addressed | done in 0.3 |
| [TD-25](#td-25) | A test-only tool signed the demo credentials | Technical debt | 0.5 (ADR 0015, 2026-11-13) | Addressed | done in 0.6 |
| [TD-26](#td-26) | One bad dataset stopped the connector at start | Technical debt | 0.7 plan (ADR 0020, 2026-11-14) | Addressed | done in 0.7 |
| [TD-27](#td-27) | The domain crates' I/O check named too few crates | Technical debt | 0.5 review (ADR 0004, 2026-11-14) | Addressed | done 2026-11-14 |
| [TD-28](#td-28) | No trace across participants | Technical debt | 0.9 (ADR 0023, 2026-11-15) | Accepted | — |
| [TD-29](#td-29) | No OTLP logs export, no gRPC, no compression | Technical debt | 0.9 (ADR 0023, DEPLOYING, 2026-11-15) | Open | Later |
| [TD-30](#td-30) | The outbox resends a message the counter-party refused | Technical debt | 0.9 (outbox metrics, 2026-11-15) | Addressed | done in 0.11 |
| [TD-31](#td-31) | No audit trail of data pulls and management actions | Technical debt | 0.11 threat model review (F-02, 2026-11-17) | Open | research with TD-04 (0.12) |
| [TD-32](#td-32) | Policy terms that are not understood are dropped without an error | Technical debt, security | 2026-11-18 review (D1) | Addressed (0.12) | — |
| [TD-33](#td-33) | The issuer's metadata does not pass the DCP JSON schema | Technical debt, interoperability | 2026-11-18 review (C1) | Addressed (0.12) | — |
| [TD-34](#td-34) | A repeated transfer request starts a second transfer | Technical debt, interoperability | 2026-11-18 review (D3) | Addressed (0.12) | — |
| [TD-35](#td-35) | No negotiation started by the provider | Technical debt, interoperability | 2026-11-18 review (D2) | Open | 0.13, if not expensive |
| [TD-36](#td-36) | DSP details: who may terminate, 401 or 404, agreement timestamp, `consumerPid` check, no catalog pages | Technical debt, interoperability | 2026-11-18 review (D4, D5) | Addressed (0.12); catalog pages open | 0.13 (catalog pages, with TD-38) |
| [TD-37](#td-37) | DCP details: `token` claim required, presentations as JWT strings only, no `/.well-known/dspace-trust` | Technical debt, interoperability | 2026-11-18 review (C2) | Addressed (0.12) | — |
| [TD-38](#td-38) | No management call to read a provider's catalog | Technical debt, usability | 2026-11-18 review (Q1) | Open | 0.13 |
| [TD-39](#td-39) | DEPLOYING is not in deployment order; the docs are long | Technical debt, usability | 2026-11-18 review (Q2, Q7) | Open | 0.13; 1.x (Helm chart) |
| [TD-40](#td-40) | Database calls block the async runtime | Technical debt | 2026-11-18 review (Q3) | Accepted (0.12, measured) | measure again each release |
| [TD-41](#td-41) | A panic stopped the whole process (`panic = "abort"`) | Technical debt, security | 2026-11-18 review (Q4) | Addressed (0.12) | — |
| [TD-42](#td-42) | Four near-copies of the negotiation and transfer logic | Technical debt | 2026-11-18 review (Q5) | Open | 0.13 (note); 1.x (merge) |
| [TD-43](#td-43) | The Gaia-X documents name several versions of the rules | Technical debt | 2026-11-18 review (G1) | Open | 0.13 |
| [TD-44](#td-44) | A transfer request sent at once after the agreement can be refused | Technical debt, interoperability | 0.12 benchmark (2026-11-19) | Addressed (0.13) | — |
| [TD-45](#td-45) | Each running pull holds about 0.8 MB of memory | Technical debt | 0.12 benchmark (2026-11-19) | Open | — |

Totals: 45 risks (Open 14, Mitigated 17, Accepted 4, Addressed 10);
31 technical debts (Open 18, Accepted 5, Addressed 8).

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
  RUST-IDIOMS. From the [review](reviews/2026-11-18-specifications-and-code-quality.md) (Q6, 2026-11-18): services with four type
  parameters (`ProviderService<S, D, C, E>`), four macros that write
  types (RUST-IDIOMS covers them in one paragraph), five files above
  1,000 lines, and `Negotiator`, which also handles transfers and the
  harvest. ROADMAP 0.13: a "read the code in this order" page that
  follows one request from the route to the store; see also
  [TD-42](#td-42). ROADMAP "Code quality in 1.x" (maintainer,
  2026-11-18): split the files above 1,000 lines, simplify the types
  and macros, rename or split `Negotiator`.

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
- **Next.** Find a second maintainer; not a 1.0 blocker (maintainer,
  2026-11-15).

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
- **Current state: Mitigated (0.10).** CI runs `cargo vet --locked`
  (workflow `Dependencies`), so a new or bumped crate cannot merge
  without an audit or a recorded exemption
  ([CONTRIBUTING](../CONTRIBUTING.md#dependency-audits-cargo-vet)).
  Imported audits from Mozilla, Google, Bytecode Alliance, ISRG and
  Zcash cover 45 of the 204 crate versions; the other 159 are exempted
  at the versions locked on 2026-11-16, so they are trusted, not
  reviewed. Also running: pinned actions, Dependabot, dependency review,
  OSV-Scanner, `cargo deny` (RustSec, licences, sources), ORT
  ([TESTING](TESTING.md#what-runs-where)).
- **Next.** Every bump of an exempted crate is reviewed (delta audit) or
  exempted again with a reason, so the exemption list shrinks over time.
  Count the exemptions at each release.

<a id="r-05"></a>
### R-05 No written OpenSSF Baseline level 1 self-assessment

- **What.** Portitor commits to OpenSSF Baseline level 1 for all of 0.x
  ([ADR 0012](adr/0012-openssf-baseline.md)), but the control-by-control
  self-assessment is not written (moved from 0.1, maintainer, 2026-11-12).
- **Why it matters.** Until then, level 1 rests on settings and CI, not
  on evidence anyone can check.
- **Current state: Open.** [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#openssf-baseline)
  says it comes in the release candidate, 0.13. ADR 0012 says "0.9
  hardening release": written before the renumbering of 2026-11-14 and
  the split of 0.10 into 0.10 to 0.13 (maintainer, 2026-11-15).
- **Next.** ROADMAP 0.13, last, so it assesses the final state; decide
  there whether 1.0 aims higher.

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
  upstream. The maintainer filed the reports on 2026-11-16:
  1. DSP 2025-1: three transfer JSON Schemas whose `$ref` to
     `AbstractTransferCodeMessage` is not a valid JSON pointer (tag
     `2025-1-err2`; ROADMAP 0.4). Existing issue
     https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/issues/265;
     the maintainer opens the fix as a pull request.
  2. Data Plane Signaling v1.0-RC4: five examples that fail its own
     schemas
     (https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/133).
  3. Data Plane Signaling: a pull `start` example that breaks a MUST
     (https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/134).
  4. Data Plane Signaling: who may resume, no message for a new data
     address when the provider starts again, the "Consumer Suspend/Start"
     diagram
     ([ARCHITECTURE](ARCHITECTURE.md#data-plane-inside-the-connector-or-separate),
     ADR 0013, ADR 0014;
     https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/135).
  5. Signaling HTTP profile: its `endpointType` versus DSP and the DSP
     TCK (ADR 0013;
     https://github.com/eclipse-dataplane-signaling/profiles/issues/8). Signaling issue
     #122 is about `authorizationType`, not this.
  6. DSP TCK v1.0.2: the transfer start race (ADR 0014, R-08;
     https://github.com/eclipse-dataspacetck/dsp-tck/issues/338).
  7. DCP v1.0.1: the `$id` mistakes of `credential-message-schema.json`
     and `context-schema.json` (`crates/portitor-dcp/tests/spec.rs`;
     comment on
     https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/270).
  8. DCP: `credentialType` in `CredentialMessage`, `type` required on
     sparse `CredentialObject`s, the offer example's profile
     (https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/276).
  9. DCP: the scope grammar; a scope cannot name a compact IRI type such
     as `gx:ParticipantComplianceCredential`, because the DCP TCK's scope
     pattern `vc.type:(?<type>[^:]+)(?<suffix>:.+)?` forbids `:` in the
     type; Portitor uses the matching rule of
     [ADR 0021](adr/0021-gaia-x-onboarding.md#credential-types-and-scopes)
     (https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/277).
  10. EDC IdentityHub: the default scope transformer needs the `:read`
      suffix (ADR 0019;
      https://github.com/eclipse-edc/IdentityHub/issues/1278). Answered
      2026-11-17, won't fix: DCP leaves the discriminator's shape to each
      implementation, `type:operation` is EDC's chosen default, and the
      transformer can be replaced. Portitor keeps sending `:read` and
      accepts both forms; the grammar question stays with DCP #277.
  11. DCP TCK v1.2.1: test credentials whose `jti` differs from the
      credential `id`, 2 KB status lists, and BitstringStatusList on a
      VC 1.1 credential (ADR 0015;
      https://github.com/eclipse-dataspacetck/dcp-tck/issues/243).
  12. DCP: how a credential whose subject is not the holder, such as the
      Gaia-X compliance credential, is bound to its holder (ADR 0022;
      https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278).
  13. Gaia-X: the same binding question. Not filed: the place for it is
      the ICAM working group on GitLab, and the maintainer has no GitLab
      account (2026-11-16). DCP issue #278 asks the question on the DCP
      side.
  14. EDC Connector: the subject check on every credential. On hold
      under R-10 (ROADMAP "Later").
- **Why it matters.** Until upstream decides, Portitor's workarounds may
  differ from what others do, and the next specification release may
  break them.
- **Current state: Addressed (0.10).** Twelve reports are filed and
  linked here and in the code that works around them (eleven issues and
  one comment). Report 13 is not filed (no GitLab account); report 14
  is on hold under R-10.
- **Next.** Follow the upstream answers, and remove each workaround when
  upstream fixes it.

<a id="r-08"></a>
### R-08 The DSP TCK v1.0.2 needs a test-only build

- **What.** (1) A race in the TCK's consumer: a start message sent at
  once can be refused and every retry gets 404. (2) The TCK sends one
  fixed `Authorization` header, which cannot be a DCP token. (3) The TCK
  itself disables TP:02-04 and TP_C:02-04.
- **Why it matters.** The conformance run does not test exactly the
  release build.
- **Current state: Mitigated.** The `tck` feature, never in release
  images: start messages 300 ms late (`tck::LateStart`, ADR 0014), the
  outbox tries a 404 again (from 0.11, when the release build stopped
  retrying it: the TCK registers a handler only after it has seen the
  state before, `TP_C:02-03` and `CN:02-01`), and the
  fixed header mapped to the participant id `TCK_PARTICIPANT`, the id the
  TCK's own messages use (ADR 0016, amended 2026-11-13; the first text
  said "a test DID"). DCP authentication is covered by the DCP TCK and
  the EDC interop test instead.
- **Next.** The race is reported
  (https://github.com/eclipse-dataspacetck/dsp-tck/issues/338, R-07); drop `LateStart` when a fixed TCK
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
  Accepted (TD-23), tested between Portitor connectors only.
- **Next.** Follow the upstream reports (R-07): the scope grammar
  (https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/277)
  and the holder binding
  (https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278).
  The Gaia-X side is not asked yet (R-07, report 13).

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

- **What.** During Gaia-X onboarding the identity resolved the DID named
  as `issuer` in the compliance service's answer and checked the
  signature with it, without checking that this issuer was the
  compliance service it called.
- **Why it matters.** Whoever answers at the configured URL could return
  a credential signed by any DID. The risk is limited: the URL is set by
  the operator and called over HTTPS, the answer must list exactly the
  digests sent, and connectors trust only the services in
  `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` (ADR 0022), so a credential from
  another issuer is skipped by verifiers.
- **Current state: Addressed in 0.8.** Raised in the 0.8 review; the
  maintainer asked to fix it before the release (2026-11-15). The
  identity now needs `PORTITOR_GAIAX_COMPLIANCE_DID` with
  `PORTITOR_GAIAX_COMPLIANCE_URL` and refuses an answer issued by any
  other DID before resolving it (ADR 0021; test
  `an_answer_from_another_service_is_refused` in
  `crates/portitor-identity/tests/gaiax.rs`).
- **Next.** None.

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
  (ADR 0016). The 0.11 threat model review (SECURITY-ASSURANCE, F-01 and
  F-06) found two more: the issuer POSTs credentials to the credential
  service that a registered holder's DID document names
  (`crates/portitor-issuer/src/delivery.rs`), and the identity resolves
  the DID of any caller of its presentation API before checking the
  caller's `aud` and access token.
- **Why it matters.** A partner could make the connector call an internal
  service.
- **Current state: Mitigated.** Only `https://` (unless
  `PORTITOR_ALLOW_HTTP_CALLBACKS` for tests), no redirects, time and size
  limits, the callback's answer body is never read, a 10 s timeout;
  DEPLOYING asks for an egress firewall. Internal addresses are not
  blocked (checked: no such check in the code).
- **Next.** Block internal addresses with an operator allow list (ADR
  0016, "roadmap, later"); not in the roadmap's lists yet. The 0.11
  review proposes that the maintainer names a release, before the 0.13
  freeze if the block is on by default (F-01), and that the identity
  checks `aud` and the access token before it resolves a caller (F-06).

<a id="r-18"></a>
### R-18 No request size limit of Portitor's own on the public ports

- **What.** Portitor sets no request size limit of its own; the
  security case planned one for 0.9, which the roadmap did not list.
- **Why it matters.** Oversized requests can exhaust memory.
- **Current state: Addressed in 0.11.** Each port of each binary has its
  own limit, a setting: `PORTITOR_MAX_REQUEST_BYTES` (public port) and
  `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` (management port), 2 MiB by
  default (the value axum applied before), at most 64 MiB. The routes
  whose messages are small by specification have a fixed limit of
  64 KiB, or the port's limit when that is lower: the DSP catalog
  request, negotiation event, verification and termination, transfer
  suspension, completion and termination (provider routes and consumer
  callbacks), the identity's STS and the issuer's credential request.
  The largest example of these messages in the DSP 2025-1 and DCP
  v1.0.1 specifications is 329 bytes. A larger body gets HTTP 413; a
  declared `Content-Length` over the limit is refused before the body is
  read (`portitor-runtime`, `request_size.rs`). Tests check each port of
  each binary and each fixed route, one byte over and at the limit. Data
  plane answers are streamed and not limited. The public ports have a
  60 s limit per request and at most 256 requests at once (DEPLOYING,
  section 5); presentations, status lists (16 MiB after decompression)
  and bundles have their own limits.
- **Next.** None planned. Contract and transfer requests, offers and
  agreements, presentations and credentials take the port's limit, as
  their policies and credentials have no size bound in the
  specifications.

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
  until a Simpl-Open sandbox exists). The ROADMAP 0.12 line keeps EDC
  only, without Gaia-X, and points Simpl-Open to "Later" (2026-11-15).
- **Next.** ROADMAP "Later"; re-check the facts before any Simpl work.

<a id="r-25"></a>
### R-25 Name not checked for trademarks and domains

- **What.** `portitor` was free on crates.io (2026-11-12); domain names
  and a formal trademark search (EUIPO, WIPO, BOIP) are not done
  ([ADR 0008](adr/0008-project-name.md)).
- **Why it matters.** A rename after 1.0 is expensive.
- **Current state: Open.**
- **Next.** ROADMAP 0.13; it blocks 1.0 (maintainer, 2026-11-15).

<a id="r-26"></a>
### R-26 ODRL `dateTime` definition read from memory

- **What.** ADR 0009 could not read the ODRL 2.2 definition of `dateTime`
  and used it from memory: "check before 1.0".
- **Why it matters.** Another connector may read it differently.
- **Current state: Open.**
- **Next.** ROADMAP 0.13, before 1.0.

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
### R-28 0.8 Gaia-X code had no fuzz target

- **What.** TESTING says "a new parser of untrusted input gets a target".
  The 0.8 code that reads caller data (the Gaia-X checks and the RFC 8785
  JCS digest of the caller's credential, which parses numbers again)
  has none: no target in `fuzz/fuzz_targets` names Gaia-X or JCS
  (checked). The X.509 reader reads only the operator's own file.
- **Why it matters.** A crash or wrong digest on crafted input.
- **Current state: Addressed in 0.8** (maintainer, 2026-11-15: "fuzz it
  if possible"). Fuzz target `gaiax_compliance`: any JSON in JCS form
  must be JSON and stay the same when put in JCS form again, every
  `digestSRI` is `sha256-` and 64 hex digits, and the compliance answer
  check and `gx:compliantCredentials` reader see any JSON object as a
  credential. 3.9 million runs found nothing (2026-11-15); ClusterFuzzLite
  runs it on every PR. Unit tests cover the RFC 8785 examples and the
  spike's digests.
- **Next.** None for these checks; the X.509 reader reads only the
  operator's file.

<a id="r-29"></a>
### R-29 DCP costs about 2 ms per request and 13 times fewer requests under load

- **What.** Measured for 0.5 (ADR 0002, section 7): DCP adds about 2 ms
  per DSP request when idle, and under load the provider answers about
  13 times fewer requests. Each request costs a DID resolution, an STS
  token and a presentation query, and the issuer's status list is
  fetched every time.
- **Why it matters.** Capacity, and load on partners' identities and
  issuers.
- **Current state: Open.** DID documents are cached 5 minutes, and from
  0.11 status lists too (TD-06, `PORTITOR_STATUS_LIST_CACHE_SECONDS`,
  default 5 minutes); tokens and presentations are not (TD-07).
- **Measured again in 0.12** ([BENCHMARKS](BENCHMARKS.md), "DCP cost"),
  with the status list cache: 1.6 ms per request for one client, and 8
  times fewer requests under load with 50 clients (2,935 against 23,751
  per second), where 0.5 measured 13 times.
- **Next.** What is left is the STS token and the presentation query of
  each request ([TD-07](#td-07)).

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

<a id="r-36"></a>
### R-36 The OpenTelemetry Rust crates are 0.x and change their API often

- **What.** Traces and metrics use `opentelemetry`, `opentelemetry_sdk`,
  `opentelemetry-otlp` (0.33) and `tracing-opentelemetry` (0.34). They
  are 0.x and change their API with most minor releases
  ([ADR 0023](adr/0023-observability.md), Consequences).
- **Why it matters.** Each update can need code changes, and an update
  that is put off can leave a security fix behind.
- **Current state: Mitigated.** All OpenTelemetry code is in one module,
  `crates/portitor-runtime/src/telemetry.rs` and `telemetry/`, and
  Dependabot updates the crates together in one group
  (`opentelemetry` in `.github/dependabot.yml`).
- **Next.** None.

<a id="r-37"></a>
### R-37 Telemetry costs some throughput at saturation

- **What.** In the 0.9 release measurements a connector at saturation
  served 8% fewer DSP requests per second than 0.8.0 with export off,
  12% fewer with every request traced and 18% fewer with one trace in
  ten (DEPLOYING, "Traces and metrics"); runs vary by up to 15%. The
  spike had measured 20% with every request traced. Sampling lowers the
  data sent, not the cost in the process.
- **Why it matters.** A busy connector with full tracing reaches its
  limit sooner.
- **Current state: Mitigated.** Export is off by default; the
  differences measured are close to the noise of one machine.
- **Next.** Measure again with the published benchmarks (ROADMAP 0.12);
  if the cost is real, look at what the spans cost when export is off.


<a id="r-38"></a>
### R-38 No specification names an S3 data address

- **What.** DSP 2025-1 lets the consumer give a `dataAddress` in its
  transfer request (`endpointType`, `endpoint`, `endpointProperties`),
  and Data Plane Signaling v1.0-RC4 covers provider push, but neither
  names an endpoint type or the properties for S3-compatible storage
  (checked in their schemas and OpenAPI files, 2026-11-15).
- **Why it matters.** Push to object storage (ROADMAP 1.1) only works
  with another connector if both read the address the same way.
- **Current state: Open.**
- **Next.** ROADMAP 1.1: the ADR reuses the address EDC or Simpl-Open
  uses; Portitor does not invent one.

<a id="r-39"></a>
### R-39 No specification defines agreements that depend on an upstream agreement

- **What.** In the health data space demo, Cooper's offer to Marley
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
  none in ICUdata4EU or LNDS (2026-11-15). HealthDCAT-AP release 8 names
  purposes with `dpv:hasPurpose` (range `dpv:Purpose`).
- **Why it matters.** A `purpose` constraint is only useful if both
  sides name purposes the same way.
- **Current state: Open.**
- **Next.** ROADMAP 1.2: purposes as Data Privacy Vocabulary (DPV) IRIs,
  as HealthDCAT-AP does; check how they map to Article 53.

<a id="r-41"></a>
### R-41 A key removed from a DID document is still trusted for up to 5 minutes

- **What.** Every component caches each resolved DID document for 5
  minutes (`DEFAULT_TTL` in `crates/portitor-runtime/src/did.rs`). There
  is no setting, although ADR 0016 says "default 5 minutes". The cache
  holds at most 1,000 documents and keeps no failed lookups
  (SECURITY-ASSURANCE, 0.11 review, F-04).
- **Why it matters.** A participant that removes a leaked key from its
  DID document is still accepted with that key by Portitor verifiers for
  up to 5 minutes.
- **Current state: Mitigated (0.12).** The time is a setting,
  `PORTITOR_DID_CACHE_SECONDS`, in the three components: default 300, at
  most 3600, `0` turns the cache off (ADR 0016, amended 2026-11-18). The
  window is still there at the default.

<a id="r-42"></a>
### R-42 The STS shares the identity's public port

- **What.** `/sts/token` is a route of the identity's public router
  (`crates/portitor-identity/src/lib.rs`), next to the DID document and
  the credential service. DEPLOYING says to route it on the internal
  network only, never through the public proxy (0.11 review, F-05).
- **Why it matters.** A proxy that forwards every path exposes the STS
  to the internet. The client secret (at least 32 characters, compared
  in constant time) still protects it, so the risk is low.
- **Current state: Open.**
- **Next.** A separate listen address for the STS, or a deployment check
  that the proxy does not route it; to decide before the 0.13 freeze,
  as it adds a setting.

<a id="r-43"></a>
### R-43 Negative credential constraints can be met with another credential

- **What.** A credential constraint holds if at least one valid
  credential of the type has the claim and the comparison is true
  ([ADR 0019](adr/0019-credential-constraints.md)). For `neq` and
  `isNoneOf` this means a holder with two credentials of the type, one
  with the excluded value and one without, meets the constraint. The
  holder also chooses what its credential service presents (0.11
  review, F-07).
- **Why it matters.** An operator may read `neq` as "refuse holders
  with this value". It is not.
- **Current state: Open.**
- **Next.** Say in ADR 0019 and DEPLOYING that negative operators are not
  deny lists.

<a id="r-44"></a>
### R-44 One slow holder delays every issuer delivery

- **What.** The issuer's outbox sends due deliveries one after the
  other (`send_due_at` in `crates/portitor-issuer/src/delivery.rs`), and
  each call may take 10 s. A registered holder whose credential service
  hangs delays the deliveries of every other holder (0.11 review, F-09).
- **Why it matters.** New members wait for their credentials.
- **Current state: Open.** Bounded: a holder has at most
  `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` active credentials per
  definition, rejections are not delivered, and retries wait longer
  each time (R-33).
- **Next.** Send to different holders side by side, a few at a time.

<a id="r-45"></a>
### R-45 No request limit per partner

- **What.** The public ports limit requests in all (256 at once, 60 s
  each, `crates/portitor-runtime/src/serve.rs`), not per partner. A
  download is not limited once its answer has started (0.11 review,
  F-10).
- **Why it matters.** One authenticated partner can take the capacity
  that the others need.
- **Current state: Open.** DEPLOYING says nothing about rate limits.
- **Next.** Document rate limits per client at the reverse proxy; a
  limit in Portitor only if measurements show the need.

## Technical debt

<a id="td-01"></a>
### TD-01 How schemas and vocabularies are collected and stored

Raised by the maintainer, 2026-11-14 (to be investigated further).
Status: **Open**; research with TD-03 (ROADMAP "Research before code").

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
release). Status: **Open**; research, code in ROADMAP 1.5.

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
**Open**; research ADR before the 0.13 freeze (ROADMAP 0.11).

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

Next: [ADR 0024](adr/0024-several-data-spaces.md) (Proposed, 2026-11-17)
compares the options with what EDC, Simpl-Open, DSP, DCP and Gaia-X do;
the maintainer decides before the 0.13 freeze. It also covers the
0.11 threat model finding F-08: today any trusted issuer is trusted for
every credential type.

<a id="td-04"></a>
### TD-04 User to system authentication and authorisation, governed per data space

Raised by the maintainer, 2026-11-15 (to be investigated). Status:
**Open**; research ADR before the 0.13 freeze (ROADMAP 0.11), code in
1.5.

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
Status: **Open**; the investigation is in ROADMAP 0.11 (security
hardening; ADR 0021 said "key rotation stays in 0.10", written before
0.10 was split on 2026-11-15, and now carries an update note).

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
whether the change itself lands in 0.11 or later. Plan after 0.13
(maintainer, 2026-11-15): two keys listed in each DID document in 0.11;
a signer port with an OpenBao adapter in ROADMAP 1.4.

Update (2026-11-17, 0.11): the rotation gap is closed for the identity
and the issuer. `PORTITOR_IDENTITY_RETIRED_KEY_FILES` and
`PORTITOR_ISSUER_RETIRED_KEY_FILES` list old public keys (PEM, never a
private key) as their own verification methods, in the same
relationships as the current key, so what the old key signed keeps
verifying until the operator removes it; the old private key can be
destroyed at once, as EDC IdentityHub does (ADR 0015 and ADR 0018,
"Amended in 0.11: retired keys"; DEPLOYING, "Rotating a key"). Tested:
`crates/portitor-identity/tests/key_rotation.rs` and
`a_key_change_with_the_old_key_retired_leaves_no_gap` in
`crates/portitor-issuer/tests/issuer.rs`. Still open: key custody (the
private key is a PEM file; OpenBao in ROADMAP 1.4), the bundle key's
rotation (one key, ROADMAP "Later", TD-16), and with Gaia-X a new
certificate and compliance run at each key change (ADR 0021).

<a id="td-06"></a>
### TD-06 Status lists are not cached in the connector's DCP verifier

- **What.** Until 0.11 the verifier fetched the issuer's status list on
  every request ([ADR 0016](adr/0016-dsp-caller-authentication.md):
  "Presentations are not cached").
- **Why it matters.** Cost per request (R-29) and load on the issuer.
- **Current state: Addressed** in 0.11, as planned (ROADMAP 0.11,
  maintainer, 2026-11-14): `PORTITOR_STATUS_LIST_CACHE_SECONDS`, default
  300, 0 checks on every request, at most 3600 (a larger value stops the
  start). Only lists that were fetched and verified are cached, keyed by
  URL, in memory, never past the list's own `validUntil` or `exp`
  (`crates/portitor-connector/src/auth/dcp/status_list_cache.rs`). The
  trade-off (a revocation takes up to that long to be seen) is in
  DEPLOYING, "Status list cache". The Compose demo, `scripts/demo.sh`,
  the EDC interop test and the DCP TCK run set 0. ADR 0016 is amended.
  Tested at 0 and 300 s (`src/auth/dcp/tests.rs`).
- **Next.** Measure again (R-29), then decide TD-07.

<a id="td-07"></a>
### TD-07 STS tokens and presentations are not cached

- **What.** Each outgoing DSP request costs one STS call; each incoming
  one a presentation query (DEPLOYING, "Known limits").
- **Why it matters.** Cost per request (R-29).
- **Current state: Open.** ADR 0016 chose "measure before adding more
  caching" so a revocation takes effect at once.
- **Next.** Decide after TD-06 is measured (0.11); code in ROADMAP 1.4
  if needed.

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
- **Measured in 0.12** ([BENCHMARKS](BENCHMARKS.md), "Data plane"): a
  download that is still running when its token ends is not cut; all
  bytes arrive. A new request with the old token gets 401. So a long
  pull works, and a pull that must start again after the token's life
  does not.
- **Next.** ROADMAP 1.1: push as a second data plane behind the
  Signaling port.

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
- **Next.** ROADMAP 1.2: `purpose`; others when needed; see also TD-03
  (processing location).

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

- **What.** `credentialSchema` is not checked (DEPLOYING "Known limits");
  a presentation query with only a `presentationDefinition` gets an error
  (ADR 0015; the DCP TCK does not test it yet); the issuer has no
  approval queue and no "present another credential first"
  (`issuancePolicy`, [ADR 0018](adr/0018-issuer-component.md)).
- **Why it matters.** Data spaces that use them cannot use Portitor as
  is.
- **Current state: Open.**
- **Next.** ROADMAP 1.4, if a target data space needs it; the issuer's
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
- **Update (2026-11-18, [review](reviews/2026-11-18-specifications-and-code-quality.md), S1).** The Signaling messages match the
  v1.0-RC4 schemas, but the ports are Rust traits inside the process:
  no Signaling endpoint is served over HTTP (`/dataflows/...`,
  `/transfers/{id}/dataflow/...`, `PUT /dataplanes`). So no outside
  data plane can be plugged in, and Portitor's data plane cannot serve
  another control plane. The profile name
  `https://w3id.org/dspace-sig/profile/http-pull` was not confirmed
  against a registry. Decision (maintainer, 2026-11-18): Signaling
  over HTTP before 1.0 if it is not expensive. ROADMAP 0.13: estimate
  it first, with the state of the Signaling specification (R-06); build
  it if the estimate is small, otherwise state it as not in 1.0.

<a id="td-18"></a>
### TD-18 Operations: no metrics, traces or readiness check

- **What.** `GET /health` answers while the process runs and does not
  check the database or partners; there is no metrics endpoint,
  OpenTelemetry or readiness endpoint (checked in the code). ARCHITECTURE
  principle 11 asks for them.
- **Current state: Addressed** in 0.9 ([ADR 0023](adr/0023-observability.md)).
  Each component has liveness and readiness checks on its management
  port (DEPLOYING "Health and logs"), a `health` subcommand for Docker,
  and OpenTelemetry traces and metrics over OTLP/HTTP when
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set (DEPLOYING "Traces and metrics").
  JSON log lines carry the `trace_id`, and every negotiation and transfer
  state change is logged. The maintainer accepted ADR 0023 on
  2026-11-16.
- **Next.** None.

<a id="td-19"></a>
### TD-19 No TLS inside the services

- **What.** The services speak plain HTTP; operators run them behind a
  TLS-terminating proxy. The management ports must stay on a private
  network (SECURITY-ASSURANCE, DEPLOYING).
- **Current state: Open.** "Not on the roadmap yet"
  (SECURITY-ASSURANCE).
- **Next.** ROADMAP 0.13: an ADR decides whether it stays the
  operator's job.

<a id="td-20"></a>
### TD-20 A holder must delete a revoked credential by hand after a reissue

- **What.** Verifiers refuse a presentation that holds any revoked
  credential (DCP v1.0.1, 5.4.3, step 8). The identity does not read
  status lists, so it keeps presenting a revoked credential next to its
  reissued one (DEPLOYING, sections 4 and 5).
- **Why it matters.** A participant is refused until its operator
  deletes the revoked credential.
- **Current state: Open.** Documented in DEPLOYING.
- **Next.** ROADMAP 1.4.

<a id="td-21"></a>
### TD-21 Gaia-X beyond 0.8

- **What.** Not in 0.8 (ROADMAP 0.8 "Not in 0.8", ADR 0021): the Gaia-X
  Lab or a production clearing house (R-12); Gaia-X DCAT and service
  offering credentials with a federated catalogue; OpenID4VCI and
  OpenID4VP (R-14); label levels 1 to 3; calling the registration number
  notary (the operator gets that credential by hand); trust anchors and
  compliance issuers read from the Gaia-X Registry; Gaia-X with EDC
  (R-10). Also not built ([review](reviews/2026-11-18-specifications-and-code-quality.md), G2, 2026-11-18): a legal person
  credential signed by another signer the participant chooses; the
  identity always signs it itself.
- **Current state: Open.**
- **Next.** ROADMAP "Later", Gaia-X beyond 0.8 (maintainer, 2026-11-15).

<a id="td-22"></a>
### TD-22 Migrations run forward only; a rollback needs the backup

- **What.** An older version refuses a database a newer one migrated;
  going back means restoring the backup, and data written since is lost
  (DEPLOYING, "Upgrade and rollback").
- **Current state: Accepted** (DEPLOYING, since 0.3; each upgrade note
  says which database moves).
- **Next.** Revisit at the 0.13 API and config freeze.

<a id="td-23"></a>
### TD-23 ADRs 0021 and 0022 were built but still Proposed

- **What.** The 0.8 code implements
  [ADR 0021](adr/0021-gaia-x-onboarding.md) and
  [ADR 0022](adr/0022-gaia-x-compliance-in-policies.md), but both say
  "Status: Proposed". ADR 0017 (Simpl-Open) is Proposed too, on purpose
  (R-24).
- **Why it matters.** The rules in force are not yet accepted decisions.
- **Current state: Addressed.** The maintainer accepted both on
  2026-11-15, before the 0.8 release.
- **Next.** None.

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
  calls to theirs. Both sides log the DSP process ids (`providerPid`,
  `consumerPid`), which is enough to match their logs (DEPLOYING,
  "Traces and metrics").
- **Next.** None.

<a id="td-29"></a>
### TD-29 No OTLP logs export, no gRPC, no compression

- **What.** Portitor exports traces and metrics over OTLP/HTTP with
  Protocol Buffers only. Logs stay JSON lines on stdout (no OTLP logs
  signal), there is no gRPC exporter, and compression is not built in:
  `OTEL_EXPORTER_OTLP_COMPRESSION` only gets an SDK warning (ADR 0023,
  DEPLOYING "Traces and metrics").
- **Why it matters.** Operators whose platform takes only OTLP logs or
  gRPC need a collector in between; exports use more bandwidth.
- **Current state: Open.** Kept out to save binary size and
  dependencies (spike, ADR 0023).
- **Next.** Later; no release named.

<a id="td-30"></a>
### TD-30 The outbox resends a message the counter-party refused

- **What.** Until 0.11 the connector's outbox retried a DSP message on
  any error, also when the counter-party answered with a 4xx status (for
  example 400, a message it will never take). With the default settings
  it sent the message three more times, then gave up
  (`crates/portitor-connector/src/outbox.rs`, `send`). Seen in 0.9 with
  the new `portitor.outbox.message.resent` metric during the Compose demo.
- **Why it matters.** Wasted calls and a later failure: the process
  ends only after the last retry instead of at once, and the partner
  sees the same bad request several times.
- **Current state: Addressed** in 0.11, as planned (ROADMAP 0.11,
  maintainer, 2026-11-15): the outbox retries only when no answer came,
  or on 408, 429 and 5xx; any other answer ends the tries at once
  (`worth_retrying` in `outbox.rs`, tested per class). The state check
  after a failed send is unchanged; ADR 0007 has a dated note. The
  issuer's delivery outbox already gave up at once on a 4xx other than
  401, 408 and 429 (`is_permanent` in
  `crates/portitor-issuer/src/delivery.rs`, since 0.6), so it is not
  changed.
- **Next.** None.

<a id="td-31"></a>
### TD-31 No audit trail of data pulls and management actions

- **What.** Found in the 0.11 threat model review (SECURITY-ASSURANCE,
  F-02). The connector logs every state change of a negotiation or
  transfer with its process ids (`crates/portitor-connector/src/state_events.rs`)
  and keeps each agreement with the counter-party's DID. It does not log
  a successful data pull (`data_plane.rs` logs only a failed upstream
  fetch), does not keep the caller's token or presentation with an
  agreement, and does not log failed management API key checks or some
  management changes (for example the identity's
  `DELETE /v1/credentials/{id}`). A management call has no person
  behind it: one key per component ([TD-04](#td-04)).
- **Why it matters.** A provider cannot show who pulled which data and
  when, and an operator's action cannot be traced. Health data spaces
  are likely to require this (EHDS; not checked which article).
- **Current state: Open.**
- **Next.** Research with the TD-04 ADR (ROADMAP 0.12): what an audit
  record holds, where it goes and how long it is kept. DSP 2025-1 does
  not sign its messages, so evidence between participants rests on the
  DCP tokens and on each side's own records.

<a id="td-32"></a>
### TD-32 Policy terms that are not understood are dropped without an error

- **What.** `Offer`, `Rule` and `AtomicConstraint`
  (`crates/portitor-dsp/src/lib.rs`) keep only the fields they know. A
  field they do not know is dropped when the JSON is read: a `duty`
  inside a permission, `assigner` or `assignee` on a rule, `unit` on a
  constraint, `uid`. A test in the [review](reviews/2026-11-18-specifications-and-code-quality.md) (D1) showed it. `same_terms`
  (`crates/portitor-dsp/src/policy.rs`) then compares what is left.
  Unknown left operands are refused ([TD-12](#td-12)); unknown fields
  are not.
- **Why it matters.** As consumer, Portitor can agree to an offer whose
  permission carries a duty it never saw. In the datasets file, a typing
  error or a term Portitor does not support vanishes instead of stopping
  the harvest. Both go against the fail closed rule of ADR 0009.
- **Current state: Addressed (0.12).** The policy reader refuses a field
  it does not read, and names it (`deny_unknown_fields` on `Offer`,
  `Rule`, `AtomicConstraint` and `Agreement`, and a constraint reader
  written by hand, `crates/portitor-dsp/src/lib.rs`; ADR 0009, amended
  2026-11-18). In the datasets file the harvest of that dataset stops
  (check `odrl`); a partner's message gets HTTP 400 with the DSP error
  and changes no state. A `duty` inside a permission is refused. An
  offer's `assigner` and `assignee` are read in a message and must be
  the provider and the consumer; they are refused in the datasets file.
- **Decision (maintainer, 2026-11-18).** Strict, for the datasets file
  and for partners' messages.
- **How Eclipse EDC differs (checked in 1.0.0-RC7).** EDC ignores a
  field it does not know and reads `duty`
  (`JsonObjectToPermissionTransformer`). Portitor is stricter on
  purpose. What is left of the risk: a partner whose policies carry
  `duty`, `remedy` or `consequence` cannot negotiate with Portitor.
- **Why the SHACL shapes could not do it** (checked 2026-11-18): shapes
  are optional, SHACL lets extra properties pass unless a shape is
  closed and `portitor-shacl` refuses `sh:closed` (ADR 0005), a `duty`
  is a valid ODRL term, and a misspelled JSON key is gone before SHACL
  sees the data.
- **Seen, not changed.** As consumer, Portitor does not check that a
  provider's offer keeps the negotiation's `target`. A logical
  constraint (`and`, `or`, `xone`) with one member, written as an object
  and not a list, is refused, as before.
- **Second layer, research (maintainer, 2026-11-18).** The bundle could
  also carry shapes for policies, so a data space can say which
  actions, left operands and values its offers may use. This adds to
  the check above and does not replace it (ROADMAP "Research before
  code", needed by 1.2): whether published ODRL shapes exist to reuse;
  whether shapes in a bundle already reach the policy inside a dataset
  and inside a message; and which SHACL core parts `portitor-shacl`
  must add, as it refuses `sh:in` and `sh:closed` today.

<a id="td-33"></a>
### TD-33 The issuer's metadata does not pass the DCP JSON schema

- **What.** DCP v1.0.1 `issuer-metadata-schema.json` requires
  `credentialType`, `credentialSchema`, `offerReason`, `bindingMethods`
  and `profile` for each credential (issuance, 6.7). `metadata()`
  (`crates/portitor-issuer/src/definitions.rs`) leaves out
  `credentialSchema` and `offerReason`; validating Portitor's answer
  against the schema reports both ([review](reviews/2026-11-18-specifications-and-code-quality.md), C1). The schema test
  (`crates/portitor-dcp/tests/spec.rs`) checks the specification's
  example, not Portitor's answer. The DCP TCK passes.
- **Why it matters.** A holder that validates the metadata can refuse
  it.
- **Current state: Addressed (0.12).** The metadata carries
  `credentialSchema` and `offerReason`, and a test validates Portitor's
  own answer and its offers against the DCP schemas
  (`crates/portitor-issuer/tests/issuer.rs`). `credentialSchema` is a
  new required URL in each entry of the issuer's definitions file, as
  EDC IdentityHub takes it from the credential definition; the issuer
  publishes it and does not fetch it ([TD-14](#td-14)). `offerReason` is
  `reissue`, the value of the DCP example and of EDC (ADR 0018, amended
  2026-11-18). Breaking for the issuer's configuration: DEPLOYING, "From
  0.11 to 0.12".
- **Left open.** DCP 6.7.1 says each entry "MUST contain all OPTIONAL
  properties", which read word for word includes `issuancePolicy`. The
  JSON schema does not require it, and Portitor leaves it out: a policy
  would say a presentation is needed before issuance ([TD-14](#td-14)).

<a id="td-34"></a>
### TD-34 A repeated transfer request starts a second transfer

- **What.** DSP 2025-1, transfer process: "Providers SHOULD implement
  idempotent behavior" for a request with the same `consumerPid`. The
  provider (`crates/portitor-dsp/src/transfer/provider.rs`, `request`)
  does not look a transfer up by `consumerPid`; the table's key is the
  `providerPid` ([review](reviews/2026-11-18-specifications-and-code-quality.md), D3).
- **Why it matters.** A consumer that sends the request again after a
  timeout gets a second transfer and a second data access token.
- **Current state: Addressed (0.12).** A request with the `consumerPid`
  of a transfer the same caller already asked for gets that transfer
  again: no new data flow, token or message
  (`crates/portitor-dsp/src/transfer/provider.rs`; ADR 0013, amended
  2026-11-18). Eclipse EDC does the same (`findForCorrelationId`). A
  repeat that names another agreement, format or callback address gets
  HTTP 400. The lookup uses the caller's checked DID, so one consumer
  never reaches another's transfer.
- **Left.** A file written before 0.12 may hold two transfers for one
  repeated request. Both stay, and the one stored first answers
  (migration 9 is an index, not a unique one, so such a file still
  opens).

<a id="td-35"></a>
### TD-35 No negotiation started by the provider

- **What.** DSP 2025-1 lets a provider start a negotiation with an
  offer; the consumer serves `POST /negotiations/offers` for it
  (negotiation HTTPS binding, "Contract Offer Endpoint"). Portitor's
  consumer callbacks have only the paths with a `consumerPid`
  (`crates/portitor-connector/src/lib.rs`), and Portitor as provider
  never starts with an offer ([review](reviews/2026-11-18-specifications-and-code-quality.md), D2). The DSP TCK passes in the
  build used ([R-08](#r-08)).
- **Why it matters.** A provider that works this way cannot negotiate
  with Portitor. EDC starts negotiations from the consumer, so the risk
  today is small.
- **Current state: Open.** DEPLOYING "Known limits" names it.
- **Decision (maintainer, 2026-11-18).** Before 1.0 if it is not
  expensive.
- **Next.** ROADMAP 0.13, before the API freeze: estimate it first;
  build it if the estimate is small, otherwise state it as not in 1.0.

<a id="td-36"></a>
### TD-36 DSP details: who may terminate, 401 or 404, agreement timestamp, `consumerPid` check, no catalog pages

- **What.** Five small points from the [review](reviews/2026-11-18-specifications-and-code-quality.md) (D4, D5):
  - The state machine figure of DSP 2025-1 lets only the provider
    terminate from ACCEPTED and VERIFIED, and only the consumer from
    AGREED; the text says the message "MAY be sent at any state".
    Portitor accepts either party in every state that is not final
    (`crates/portitor-dsp/src/negotiation.rs`, `next_state`), while a
    test comment says it follows the figure.
  - A call without valid credentials gets 401. The negotiation and
    transfer bindings say a client that is not authorized gets 404.
    What EDC answers is not checked.
  - As consumer, an agreement without `timestamp` is accepted; the
    schema requires it.
  - A new request on a running negotiation is not checked against the
    stored `consumerPid`.
  - The catalog goes out in one answer: no pages (`Link` header, a MAY),
    and EDC's `limit` and `offset` are ignored.
- **Why it matters.** None breaks a test today; each is a place where
  another connector may behave differently, and the catalog answer grows
  with the number of datasets.
- **Current state: Addressed (0.12), but for catalog pages.** What
  Eclipse EDC 1.0.0-RC7 does was read first:
  - Who may terminate: unchanged, either party in every state that is
    not final. EDC does the same (`canBeTerminated`), and the text of
    the specification allows it. The test comment is corrected (ADR
    0006, amended 2026-11-18).
  - 401 or 404: a refused caller now gets 404 on the negotiation and
    transfer routes, callbacks included, and 401 on the catalog, as the
    DSP bindings say and as EDC answers (ADR 0016, amended 2026-11-18).
    The reason is in Portitor's log only. A partner cannot tell "refused"
    from "unknown process" from its own side: that is the rule's aim,
    and it makes troubleshooting need both logs (DEPLOYING,
    troubleshooting table).
  - As consumer, an agreement without a `timestamp`, or with one that is
    not an `xsd:dateTime`, is refused. The requirement is in the text of
    the specification, not in its JSON schema.
  - A new request on a running negotiation must name the stored
    `consumerPid`.
- **Open: catalog pages.** The benchmark of 0.12
  ([BENCHMARKS](BENCHMARKS.md)) measured a catalog of 10,000 datasets:
  2.6 MB in 13 ms, 150 MB of memory on the provider. A national gateway
  lists about 1,100 datasets, so pages are not needed for the provider.
  But Portitor's own HTTP client reads at most 64 kB of an answer
  (`MAX_ANSWER`), about 250 datasets: this limits the management call
  that reads a partner's catalog ([TD-38](#td-38)), and is decided with
  it.

<a id="td-37"></a>
### TD-37 DCP details: `token` claim required, presentations as JWT strings only, no `/.well-known/dspace-trust`

- **What.** From the [review](reviews/2026-11-18-specifications-and-code-quality.md) (C2):
  - The verifier refuses a caller whose ID token has no `token` claim
    (`crates/portitor-connector/src/auth/dcp.rs`, `check_token`); in
    DCP v1.0.1 the claim is optional.
  - A presentation answer is read only when each presentation is a JWT
    string; the schema also allows objects. Both profiles Portitor
    supports use JWT.
  - `/.well-known/dspace-trust` of the DCP profile for DSP is not
    served.
  - Not checked: that the issuer's base URL equals the `issuer` of the
    credentials it signs.
- **Why it matters.** Refusing is the safe side, but the choices are not
  written down, and the last point is unknown.
- **Current state: Addressed (0.12).** Recorded in ADR 0016 (amended
  2026-11-18), each with its reason and with what Eclipse EDC 1.0.0-RC7
  does: the `token` claim stays required (EDC: `TokenNotNullRule`);
  presentations stay JWT strings; `/.well-known/dspace-trust` is not
  served (not in the published DCP v1.0.1, and not in EDC).
- **The last point, checked.** DCP 6.3 says the issuer "will use the
  base URL for the `issuer` field"; it gives no check to a holder or a
  verifier, and everywhere else the issuer is a DID. EDC compares no
  URL. No URL check was added. One real gap was found and closed: DCP
  5.4.3, step 4, asks that the key that signed a credential belongs to
  the DID of its `issuer`. Until 0.11 a key listed in the issuer's DID
  document under another DID's id was accepted; from 0.12 it is refused
  (`Credential::verify`, `crates/portitor-dcp/src/credential.rs`), as
  EDC does (`IssuerKeyIdValidationRule`).

<a id="td-38"></a>
### TD-38 No management call to read a provider's catalog

- **What.** To see what a provider offers, the operator asks the STS
  for a token with the client secret, calls the provider's catalog with
  curl and copies the offer into the negotiation request by hand
  (`scripts/demo-steps.sh`; [review](reviews/2026-11-18-specifications-and-code-quality.md), Q1).
- **Why it matters.** It is the first step of every new user and the
  hardest one; it also puts the STS secret in the operator's hands.
- **Current state: Open.**
- **Next.** ROADMAP 0.13, before the API freeze: a management call that
  takes the provider's address and returns its catalog. The request
  shape follows EDC's management API, not a new one.

<a id="td-39"></a>
### TD-39 DEPLOYING is not in deployment order; the docs are long

- **What.** DEPLOYING explains the connector first (sections 1 to 3),
  the identity in section 4 and the issuer in section 7; only "Order",
  near the end, says the issuer and the identity come first. Its
  configuration section sends the reader to the README for the full
  table. The docs are about 10,700 lines; this file alone lists more
  than 80 items without saying which few matter most ([review](reviews/2026-11-18-specifications-and-code-quality.md), Q2, Q7).
  There is a Compose example and no Helm chart.
- **Why it matters.** The learning curve to deploy and troubleshoot
  should be as small as possible (maintainer, 2026-11-18).
- **Current state: Open.**
- **Next.** ROADMAP 0.13, documentation review: a short "first
  deployment" path at the top of DEPLOYING, sections in deployment
  order, a "top risks" box here.
- **Helm chart (maintainer, 2026-11-18).** In a 1.x release (ROADMAP
  "Code quality in 1.x").
- **Decision (maintainer, 2026-11-18).** DEPLOYING is written for an
  operations team; running the demo on a laptop belongs in TESTING
  (it has the Compose demo already), so DEPLOYING loses what only the
  demo needs.

<a id="td-40"></a>
### TD-40 Database calls block the async runtime

- **What.** Each store is one SQLite connection behind one lock, with
  `synchronous=FULL` (`crates/portitor-runtime/src/sqlite.rs`), and the
  HTTP handlers call it directly. Only the harvest and the health checks
  run on blocking threads ([review](reviews/2026-11-18-specifications-and-code-quality.md), Q3).
- **Why it matters.** Under load every request thread can wait for one
  write to reach the disk. It shows as "slow, no error", which is hard
  to troubleshoot. Related: [TD-08](#td-08).
- **Current state: Accepted (0.12, measured).** The rule was set before
  measuring (maintainer, 2026-11-18): move the calls to blocking threads
  if, with 100 pulls running and 1 negotiation per second, the data
  plane's p99 time to first byte more than doubles or passes 100 ms.
  Measured ([BENCHMARKS](BENCHMARKS.md), "TD-40"): 9.2 ms alone, 8.5 ms
  with 1 negotiation per second, 14.5 ms with 260 per second, the most
  the test machine produced. The rule was not met, so the calls stay
  where they are. The lock and the disk limit negotiations (p99 92 ms at
  260 per second), not pulls. Real platforms see a few negotiations per
  day (BENCHMARKS, "Reference load").
- **Next.** Measure again in each release; look again if the p99 passes
  100 ms, or with [TD-08](#td-08).

<a id="td-41"></a>
### TD-41 A panic stopped the whole process (`panic = "abort"`)

- **What.** The release profile sets `panic = "abort"` (`Cargo.toml`).
  A panic in one request ends the process and every request in flight;
  the code that recovers a poisoned lock never runs in a release build.
  `unwrap` is denied by Clippy and the parsers are fuzzed (0.11), so a
  panic should be rare ([review](reviews/2026-11-18-specifications-and-code-quality.md), Q4).
- **Why it matters.** One request that finds a panic is a denial of
  service until the container is started again. The outbox is durable,
  so no message is lost.
- **Current state: Addressed (0.12)** (maintainer, 2026-11-18: drop the
  setting). A panic in a request handler answers HTTP 500 for that
  request and the others go on; one ERROR log line names the place in
  the code, never the message; the metric
  `portitor.http.request.panicked` counts it
  (`crates/portitor-runtime/src/panics.rs`; SECURITY-ASSURANCE, "What a
  panic does"). Background tasks are watched: schedules start again
  after a pause (5 seconds, doubling up to 5 minutes while they keep
  panicking), and the process stops if the management server ends
  (`portitor.task.panicked`). The code that recovers a poisoned lock now
  runs in release builds and has a test per store.
- **Cost.** The release binaries are about 13% larger (connector 12.9 to
  14.7 MB; ADR 0002, amended in 0.12).
- **Left.** A message or a credential request that meets the same bug
  on every try is tried again for as long as the process runs: a DSP
  message after a pause (5 seconds, doubling up to 5 minutes), a
  credential request on each round of the issuer's outbox. Each try is
  logged and counted, and the other work goes on. A restart policy is
  still advised (DEPLOYING, "Known limits").

<a id="td-42"></a>
### TD-42 Four near-copies of the negotiation and transfer logic

- **What.** Negotiation and transfer, each as provider and as consumer,
  have the same shape (receive, check, store, outbox, deliver, abort) in
  four services of about 770 lines each (`crates/portitor-dsp/src/negotiation/`,
  `crates/portitor-dsp/src/transfer/`) and four adapters in the
  connector. The in-memory stores live in production code ([review](reviews/2026-11-18-specifications-and-code-quality.md), Q5).
- **Why it matters.** A fix in one may be needed in the other three.
- **Current state: Open.**
- **Next.** No merge before 1.0. ROADMAP 0.13: TESTING gets a note,
  "changed one, check the other three". ROADMAP "Code quality in 1.x"
  (maintainer, 2026-11-18): one shared core in a 1.x release, best
  before 1.1 adds push; the in-memory stores move into the tests.

<a id="td-43"></a>
### TD-43 The Gaia-X documents name several versions of the rules

- **What.** ADR 0021 cites Compliance Document 3.1.0 and links 25.10
  pages; ADR 0022 cites 4.0.0; AGENTS, the roadmap and ADR 0011 say
  "Trust Framework (Danube)"; ADR 0002 says "Danube, 3.0". The newest
  Compliance Document is 4.0.0 (2026-10-01). Its changelog adds
  criteria for digital services; the participant criterion and the
  onboarding steps Portitor uses did not change ([review](reviews/2026-11-18-specifications-and-code-quality.md), G1; checked
  2026-11-18).
- **Why it matters.** A reader cannot tell which version Portitor was
  checked against.
- **Current state: Open.** ADR 0021 has an update note (2026-11-18).
- **Next.** ROADMAP 0.13, documentation review: one place that says
  which Compliance Document and ICAM version were checked, and the same
  name everywhere.

<a id="td-44"></a>
### TD-44 A transfer request sent at once after the agreement can be refused

- **What.** The provider marks a negotiation `FINALIZED` only when the
  consumer has acknowledged the finalization event (outbox with proof of
  delivery, ADR 0007). The consumer marks it `FINALIZED` when it
  receives the event, a moment earlier. A transfer request that arrives
  in between finds no finalized agreement: the provider answers HTTP 400
  and the consumer's transfer ends `TERMINATED`. The benchmark of 0.12
  saw it 2 to 4 times in 6,000 to 10,000 transfers started without a
  pause, 16 at a time ([BENCHMARKS](BENCHMARKS.md)). The provider logs
  nothing for this refusal.
- **Why it matters.** A person who starts a transfer by hand never meets
  it. A program that starts the transfer in the same second can, and
  sees a transfer refused for no visible reason.
- **Options considered.** (1) The provider treats a transfer request
  under an agreement whose finalization is on its way as proof that the
  event arrived, as a consumer's next negotiation message already is.
  The cost: a consumer that asks before it received the event is served
  too. (2) The provider accepts the request while the finalization is
  on its way and leaves the state to the outbox. The cost: if the
  delivery fails in the end, a transfer exists under a terminated
  negotiation. (3) Leave it, log the refusal, and say in DEPLOYING that
  a client tries again.
- **Current state: Addressed (0.13).** The maintainer chose option (1)
  on 2026-11-20
  ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), "Amendment for
  0.13"). Such a request, from the agreement's consumer, settles the
  negotiation as `FINALIZED` and is then checked as any other; any
  other state is refused as before. The rule is in `portitor-dsp`
  (`negotiation::provider::finalization_proved`,
  `transfer::provider::TransferService::agreement`), the SQL in
  `crates/portitor-connector/src/store/transfers.rs`. The provider now
  logs the refusal `transfer request is refused; answered 400`
  ([DEPLOYING](DEPLOYING.md), troubleshooting). Tested in the domain
  and over HTTP with the SQLite store, also with 16 requests at once;
  the benchmark was not run again.

<a id="td-45"></a>
### TD-45 Each running pull holds about 0.8 MB of memory

- **What.** With 1,000 pulls running at once the provider used 923 MB,
  about 0.7 to 0.9 MB for each ([BENCHMARKS](BENCHMARKS.md), "Data
  plane"). One pull of 10 GiB stays flat at 19 MB, so the memory goes
  with the number of pulls, not with their size. Each pull also needs
  two open files (the caller's connection and the one to the source).
- **Why it matters.** The data plane is where the load is expected
  (maintainer, 2026-11-18). An operator must size memory and the open
  files limit for the number of pulls at once.
- **Current state: Open.** DEPLOYING "Known limits" gives the numbers.
- **Next.** Find where the memory goes (buffers of the proxy, of the
  HTTP client, of TLS) before changing anything; measure again in each
  release.
