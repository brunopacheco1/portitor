<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Portitor review: spec gaps and code quality

- Date: 2026-11-18
- Reviewed: `main` at `229c70e` (release 0.11.0). Nothing in the repository was changed.
- Specs used: Dataspace Protocol 2025-1 (tag `2025-1-err2`), Decentralized Claims Protocol v1.0.1, Data Plane Signaling v1.0-RC4, Gaia-X Compliance Document 4.0.0 (2026-10-01) with ICAM 25.11.
- Method: read the spec texts and JSON schemas, read the code, ran the test suite (719 pass, 0 fail, 2 ignored), and ran two small experiments (noted where used). Each finding was compared with `docs/ROADMAP.md`, `docs/RISKS_AND_TECHNICAL_DEBTS.md` and the ADRs. "New" means I did not find it there.

Where each finding is tracked
([RISKS_AND_TECHNICAL_DEBTS](../RISKS_AND_TECHNICAL_DEBTS.md), with the
target release from the [roadmap](../ROADMAP.md)):

| Finding | Tracked as |
|---|---|
| D1 | TD-32 |
| D2 | TD-35 |
| D3 | TD-34 |
| D4, D5 | TD-36 |
| C1 | TD-33 |
| C2 | TD-37 |
| S1, S2 | TD-17, R-06 |
| G1 | TD-43 |
| G2 | TD-21 |
| Q1 | TD-38 |
| Q2, Q7 | TD-39 |
| Q3 | TD-40 |
| Q4 | TD-41 |
| Q5 | TD-42 |
| Q6 | R-01 |

This file is the report as written on 2026-11-18 and is not updated;
the list above and the linked items are.

## 1. Summary

The core is in good shape. Both DSP state machines follow the spec, the DCP verifier follows the validation steps, and the TCKs pass in CI. I found no critical gap. The new findings are about edges, and the largest gaps are ones you already list (push transfers, data plane over HTTP, Gaia-X lab run).

What I would fix first, in this order:

| # | Finding | Area | Severity | Status |
|---|---|---|---|---|
| 1 | Unknown policy fields (a `duty` inside a permission, a constraint `unit`, `assigner`) are dropped without an error, so a policy can lose terms silently | DSP / ODRL | Medium | New |
| 2 | The issuer's metadata does not pass DCP's own JSON schema (`credentialSchema` and `offerReason` are missing) | DCP | Low to medium, small fix | New |
| 3 | An operator cannot read a provider's catalog through the management API; it takes a hand-made STS call and curl | Usability | Medium | New |
| 4 | DEPLOYING explains the connector first, but the deployment order is issuer, identity, connector | Docs | Medium | New |
| 5 | Provider-initiated negotiation is missing (`POST /negotiations/offers` on the consumer side) | DSP | Medium | New |
| 6 | Database calls block the async runtime, behind one lock per database, with an fsync per write | Code | Low to medium | New (not in RISKS) |
| 7 | `panic = "abort"`: any panic stops the whole process | Code | Low to medium | New (not documented) |
| 8 | A repeated transfer request creates a second transfer and token | DSP | Low to medium | New |
| 9 | Gaia-X documents cite four different versions and call the framework "Danube" | Gaia-X docs | Low | New |
| 10 | No Data Plane Signaling over HTTP, so no outside data plane can plug in | Signaling | Medium | Known (ADR 0003, TD-17, R-06) |

Three questions for you are in section 7.

## 2. Dataspace Protocol 2025-1

### New findings

**D1. Unknown policy fields are dropped silently. Medium.**
`Rule`, `Offer` and `AtomicConstraint` (`crates/portitor-dsp/src/lib.rs:375`, `:410`, `:420`) keep only the fields they know and have no catch-all. I checked with a throwaway test: an offer with `assigner`, `uid`, a `duty` inside a permission and a constraint `unit` was read without error, and the output held only `action` and `constraint`. `same_terms` (`crates/portitor-dsp/src/policy.rs`) then compares the reduced policies.
Why it matters: a provider can send an offer whose permission carries a duty; Portitor as consumer agrees to it without seeing it. In your own datasets file, a typo or an unsupported term vanishes instead of stopping the harvest. This goes against the fail-closed rule the policy code follows elsewhere. TD-12 says duties and prohibitions are not enforced; it does not say they are dropped unseen.
Suggested direction: refuse what is not understood (as is done for unknown left operands), or keep unknown fields and refuse to agree.

**D2. Provider-initiated negotiation is missing. Medium.**
DSP negotiation HTTPS binding, "Contract Offer Endpoint": a consumer exposes `POST /negotiations/offers` so a provider can start with an offer. The consumer callback routes (`crates/portitor-connector/src/lib.rs:621-636`) have only `{pid}`-based paths, and Portitor as provider never starts a negotiation. The DSP TCK passes, so check if the TCK build in use skips this case (R-08). EDC also starts negotiations from the consumer, so interop risk today is small.

**D3. A transfer request is not idempotent. Low to medium.**
Transfer protocol: "Providers SHOULD implement idempotent behavior" for a request with the same `consumerPid`. `request` (`crates/portitor-dsp/src/transfer/provider.rs:316-347`) has no lookup by `consumerPid` (the table key is `provider_pid`, `crates/portitor-connector/src/store.rs:60`). A consumer that retries after a timeout gets a second transfer and a second data token.

**D4. Who may terminate a negotiation. Low, a decision to record.**
The state machine figure allows termination from ACCEPTED and VERIFIED by the provider only, and from AGREED by the consumer only. The text says the message "MAY be sent at any state". Portitor accepts either party in every non-final state (`crates/portitor-dsp/src/negotiation.rs:192`), while a test comment says it follows the figure. Pick one reading and write it down; the lenient one is safer for interop.

**D5. Smaller points. Low.**
- No catalog pagination (`Link` header is a MAY); the whole catalog goes in one answer (`crates/portitor-dsp/src/catalog.rs:285`). EDC's `limit`/`offset` are ignored. Fine for a small catalog, a limit for a large one.
- A counter-request on an existing negotiation does not check the `consumerPid` against the stored one (`crates/portitor-dsp/src/negotiation/provider.rs:259`).
- As consumer, an agreement without `timestamp` is accepted (`crates/portitor-dsp/src/negotiation/consumer.rs:354-388`); the schema requires it.
- A call without valid credentials gets 401. The negotiation and transfer bindings say an unauthorized client gets 404. I did not verify what EDC answers; 401 is more useful for troubleshooting, so I would keep it and record the choice.
- Not implemented and optional: the DSP `service` entry in the DID document, the proof metadata endpoint, gzip.

### Known and listed
TD-10 (pull only, no token renewal), TD-11 (consumer refuses offers with credential constraints; suspend and start by hand), TD-12 (few ODRL constraints; counter-offers end the negotiation), TD-13 (catalog shows every offer to every caller), R-08 (TCK build), R-17 (callback addresses).

### Conformant
Version endpoint, catalog shape, both state machines apart from D4, the 200/201/400/404 answers, callback paths, error bodies.

## 3. Decentralized Claims Protocol v1.0.1

### New findings

**C1. Issuer metadata fails the spec's schema. Low to medium, small fix.**
`issuer-metadata-schema.json` requires `credentialType`, `credentialSchema`, `offerReason`, `bindingMethods` and `profile` for each credential. `metadata()` (`crates/portitor-issuer/src/definitions.rs:116-139`) leaves out `credentialSchema` and `offerReason`. I validated Portitor's output against the schema: both are reported as missing. The existing schema test (`crates/portitor-dcp/tests/spec.rs:157`) only checks the spec's example, not Portitor's output. A strict client could refuse the metadata. TD-14 lists `credentialSchema` as not supported, but not that the answer is invalid.

**C2. Smaller points. Low.**
- The verifier refuses a caller whose ID token has no `token` claim (`crates/portitor-connector/src/auth/dcp.rs:370`). In DCP that claim is optional. Refusing is the safe choice and matches the DSP profile; record it.
- Only presentations sent as JWT strings are accepted (`dcp.rs:981`); the schema also allows objects. Both supported profiles use JWT, so this is fine today.
- `/.well-known/dspace-trust` from the DCP profile for DSP is not served.
- Not checked: that the issuer's base URL equals the `issuer` field of the credentials it signs.

### Known and listed
Credentials from an untrusted issuer are skipped, not refused (ADR 0016, ADR 0022), TD-14 (`presentationDefinition` answers 501, `issuancePolicy`), TD-09 (`jti` kept in memory), TD-05 (key custody and rotation), TD-07, TD-20, R-41, R-42.

### Conformant
ID token checks, the DID verification relationships, `vc.type` and `vc.id` scopes, 201 with `Location` on credential requests, the `CredentialService` and `IssuerService` entries, both profiles (`vc11-sl2021/jwt`, `vc20-bssl/jwt`), revocation.

## 4. Data Plane Signaling v1.0-RC4

**S1. No HTTP binding. Medium. Known (ADR 0003, ADR 0013, TD-17, R-06).**
The message shapes in `crates/portitor-dps` match the RC4 schemas, but `DataPlane` and `ControlPlane` are in-process traits. None of the endpoints exist: `/dataflows/prepare`, `/dataflows/start`, `/dataflows/{id}/...`, the `/transfers/{id}/dataflow/...` callbacks, `PUT /dataplanes`. So Portitor "follows the interface" but cannot use an outside data plane, and its data plane cannot serve an outside control plane. This is your accepted design; I only want the wording in README and website to say "message model", not "interface", if it does not already.

**S2. Not verified.** I could not confirm the profile name `https://w3id.org/dspace-sig/profile/http-pull` against an online registry. The spec is still a release candidate, so names may change.

## 5. Gaia-X Trust Framework

**G1. Mixed versions and naming in the documents. Low. New.**
ADR 0021 cites Compliance Document 3.1.0 and links 25.10 pages; ADR 0022 cites 4.0.0; AGENTS.md, ROADMAP and ADR 0011 say "Trust Framework (Danube)"; ADR 0002 says "Danube, 3.0". The current rule book is Compliance Document 4.0.0 (2026-10-01). Its changelog adds criteria for digital services; the participant criterion and the onboarding steps Portitor uses did not change, so this is a wording fix, not a code fix. One line saying "checked against Compliance Document 4.0.0, ICAM 25.11" in one place would end the confusion.

**G2. Signing by power of attorney is not supported or mentioned. Low. New.**
The legal person credential can be signed by another party the participant chooses. Portitor always self-signs. Worth one line in "Known limits".

**Known and listed:** TD-21, R-10 to R-16: no run against the real Gaia-X Lab, no service offering credentials or labels, the notary credential is fetched by hand, trusted compliance issuers come from configuration and not the Gaia-X Registry, EDC does not accept the compliance credential in DCP, the type-to-scope rule is Portitor's own (reported upstream).

The largest real risk here stays the missing lab run: everything is tested against a local service only.

## 6. Code quality, deployment and troubleshooting

Facts: 48,400 lines of production Rust and 22,000 lines of tests in 10 crates; 214 packages in `Cargo.lock`; about 48 settings over three components, all documented; about 10,700 lines of documentation. The full test run takes about 20 seconds after the build.

### What is good
- Start-up fails early with a clear message when a setting is wrong.
- DEPLOYING has a table that maps log messages to causes and fixes (lines 1058-1093). This is the best troubleshooting aid in the repo.
- Health checks, typed errors, doc comments that cite spec sections, `unsafe` forbidden, `unwrap` denied, TCKs in CI.

### Findings

**Q1. No management call to read a provider's catalog. Medium.**
To start a negotiation an operator must get an STS token with the client secret, curl the provider, and copy the offer by hand (`scripts/demo-steps.sh:119-146`). This is the first thing a new user does, and it is the hardest step. A `POST` on the management API that takes the provider's address and returns its catalog would remove it. EDC has the same call, so nothing new is invented.

**Q2. DEPLOYING is in the wrong order for a first deployment. Medium.**
The file is 1,723 lines. It explains the connector in sections 1 to 3, the identity in section 4 and the issuer in section 7, and only the last part ("Order", line 1664) says the issuer and identity come first. The configuration section also sends the reader to the README for the full table. A newcomer reads 1,600 lines before learning where to start. Suggested: a one-page "first deployment" path at the top (order, the minimum settings per component, one check per step), and one file per component or at least sections in deployment order. There is a compose example but no Helm chart; say so plainly if Kubernetes users are a target.

**Q3. Blocking database calls in async handlers. Low to medium.**
Each store is one `Mutex<Connection>` with `synchronous=FULL` (`crates/portitor-runtime/src/sqlite.rs:27-31`), called directly from async handlers. Only harvest and health use `spawn_blocking`. Under load, every request thread can wait on one fsync. For a small connector this is acceptable, but it is a limit that is not in RISKS, and it shows up as "slow, no error", which is hard to troubleshoot. Cheapest step: list it as a known limit with a number from a load test.

**Q4. `panic = "abort"` in the release profile. Low to medium.**
`Cargo.toml:79`. Any panic ends the process and all requests in flight. The code that recovers poisoned locks (`store.rs:184` and others) never runs in release. With a restart policy and the durable outbox the damage is small, but one request that triggers a panic is a denial of service. I did not find this choice in RISKS or DEPLOYING. Either document it with "run with a restart policy", or drop the setting.

**Q5. Four near-copies of the process logic. Low to medium, maintenance.**
Negotiation and transfer, each for provider and consumer: 3,100 lines in `portitor-dsp` and 1,900 in the connector adapters, with the same shape (receive, check, store, outbox, deliver, abort). A fix in one has to be repeated in up to four places. In-memory stores also live in production code. I would not merge them before 1.0 unless bugs show the copies drifting; a checklist in TESTING ("changed one, check the other three") costs less.

**Q6. Readability for someone new to Rust. Low to medium.**
- Types with four type parameters (`ProviderService<S, D, C, E>`, `crates/portitor-dsp/src/negotiation/provider.rs:197`).
- Four macros that generate types (`type_tag!`, `pid_message!`, `process_ack!`, `type_name!`). RUST-IDIOMS covers them in one paragraph near the end (line 846).
- Large files: `auth/dcp.rs` (1,003 lines plus 1,897 of tests), `issuer/store.rs` (1,248), `dcp/bundle.rs` (1,280), `identity/store.rs` (1,091), `runtime/http.rs` (1,077).
- `Negotiator` also handles transfers and harvest; the name misleads.
A short "read the code in this order" page (one request traced from route to store) would help more than refactoring.

**Q7. Documentation volume. Low.**
10,700 lines for a "small" connector is itself a learning cost. RISKS (1,394 lines, 76 entries) is thorough, but a reader cannot tell which five matter. A "top risks" box at the top would help.

## 7. Questions for you

1. D1: should an unknown policy term stop the harvest and end a negotiation (strict), or be kept and shown but never agreed to? Strict is simpler and matches the current fail-closed rule.
2. D2 and S1: are provider-initiated negotiation and Signaling over HTTP in scope before 1.0, or should both be stated as "not planned"?
3. Q2: who is the main reader of DEPLOYING, someone running the demo on a laptop or an operations team? The right structure depends on it.

Answers (maintainer, 2026-11-18), recorded in the linked items: 1.
strict, the harvest of that dataset stops (TD-32); 2. both before 1.0
if not expensive (TD-35, TD-17); 3. an operations team, and the laptop
demo belongs in TESTING (TD-39).

## 8. What I did not check

- No run against EDC, Simpl-Open or the Gaia-X Lab; interop statements rest on the specs and the repo's own tests.
- The TCK runs themselves (I relied on the CI results recorded in the repo).
- Security review of cryptography and dependencies (0.10 and 0.11 covered these).
- Performance: Q3 is from reading the code, not from a load test.
- The website text and the OpenAPI files, line by line.
