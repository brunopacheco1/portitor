<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Security assurance case

Why Portitor's security requirements are met: what users can expect, the
threats considered, the trust boundaries, the design principles applied,
the common weaknesses countered, and the evidence. How to report a
vulnerability is in [SECURITY.md](../SECURITY.md); the design is in
[ARCHITECTURE.md](ARCHITECTURE.md).

Status: **0.14, the release candidate for 1.0.** This case describes the
release candidate. Items marked *(planned)* are not implemented; do not
rely on them.

## What users can and cannot expect

Portitor runs as network services (`portitor-connector`, `portitor-identity` from 0.5
and `portitor-issuer` from 0.6, [ADR 0003](adr/0003-component-split.md)) that talk to other
dataspace participants over HTTPS.

**Users can expect:**
- **Strict message parsing:** every DSP message is parsed into a dedicated
  type; a wrong `@type`, a missing DSP `@context` or an unknown structure is
  rejected with a DSP error, never half-processed.
- **Strict token checks:** DCP self-issued tokens are accepted only with
  `ES256` or `PS256` (never `none`), a key from the issuer's DID document
  with the `capabilityInvocation` relationship, matching `iss`/`sub`/`aud`,
  valid `exp`/`iat`, and a `jti` not seen before (DCP v1.0.1).
- **Small attack surface:** a static binary on an empty (`scratch`) image,
  running as a non-root user, with no shell.
- **Authentic schema and vocabulary bundles (0.7):** a connector loads
  only the newest version the authority names in `latest.json`, never one
  issued before the version in use (or one fixed version), signed (ES256, `typ`
  `portitor-bundle+jws`) with the bundle key `<authority>#bundle-key`
  that the authority's `did:web` document lists under `assertionMethod`,
  with every file's SHA-256 matching. Any other key of the authority,
  such as its credential key (also under `assertionMethod`), is refused,
  and so is PS256. A bundle that fails a check stops the start; the disk
  cache never replaces it and is itself checked again before use
  ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)). While the
  authority cannot be reached, the connector reuses the cached copy of
  the version in use that it verified before, with the key it stored
  then, without reading the DID document again: a bundle key revoked in
  the meantime is not noticed until the authority is reachable.
- **No surprise in validation:** `portitor-shacl` refuses shapes that use
  a SHACL term it does not implement, so a rule is never passed silently.
- **A bad dataset does not take the catalog down (0.7):** each dataset is
  checked on its own when it is harvested; one that fails is rejected and
  logged, and the rest of the catalog and running transfers are not
  touched ([ADR 0020](adr/0020-dataset-harvest.md)).
- **Gaia-X compliance credentials checked strictly (0.8):** a
  connector accepts a `gx:ParticipantComplianceCredential` only from a
  compliance service in `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`, signed with
  its `assertionMethod` key, valid now, and only with the
  `gx:LegalPerson` credential it names by `digestSRI`, issued and signed
  by the caller; one from a listed service that fails refuses the
  caller, one from any other service is skipped
  ([ADR 0022](adr/0022-gaia-x-compliance-in-policies.md)). The identity
  stores a compliance service's answer only if it is issued by
  `PORTITOR_GAIAX_COMPLIANCE_DID` (checked before anything is resolved),
  signed by that DID's key and lists exactly the digests of what it sent,
  and refuses a certificate chain for another key
  ([ADR 0021](adr/0021-gaia-x-onboarding.md)).
- **Telemetry off by default (0.9):** no trace or metric leaves a
  component unless the operator sets an OTLP endpoint. With it set,
  span and metric attributes are an allow-list: never a token, key,
  `Authorization` header, body or dataset contents, and no URL path or
  full URL (they hold ids and tokens). Trace context (`traceparent`)
  goes only to services inside the participant (its own STS and data
  sources); partners never receive ours, and one they send on the DSP
  port is ignored ([ADR 0023](adr/0023-observability.md)). The health
  checks answer without a key on the management port only, and their
  failure messages name a check and a reason, never a secret.
- **Authentic releases:** from 0.1.0, images are signed with cosign
  (keyless) and carry an SBOM and SLSA build provenance as attestations.

**Users cannot expect (out of scope in 0.x):**
- Production readiness: Portitor is a prototype until 1.0.
- TLS termination inside the services: it stays the operator's job
  ([ADR 0026](adr/0026-tls-termination.md), 2026-11-20). Run them behind a
  TLS-terminating proxy, with the management ports on a private network.
- Multi-instance safety: negotiations are kept in one SQLite file per
  process (ADR 0006); several replicas need the PostgreSQL adapter
  *(planned)*.
- Offers hidden by credentials: the catalog shows every offer to every
  caller with a trusted membership credential (DCP, ADR 0016); credential
  constraints (ADR 0019) are checked at negotiation and transfer only.
- Protection against server-side request forgery through the
  consumer-chosen `callbackAddress` beyond the `https://`-only rule:
  internal addresses are not blocked yet.
- Protection from a participant that is legitimately authorised: policies
  decide what a partner may do; writing them correctly is the operator's
  job.

## Threat model

| Asset | Threat | Countered by |
|---|---|---|
| Datasets and transfers | A party that is not authorised reads data or starts a transfer | DCP verification of every request (ADR 0016); ODRL policy checks (ADR 0009); credential constraints, checked at negotiation and again at each transfer request (ADR 0019) |
| Participant identity | Forged or replayed self-issued tokens | Algorithm allow-list, key relationship check, audience check, expiry with bounded leeway, `jti` replay cache |
| Credentials | A revoked, forged or someone else's credential admits a party | Only trusted issuers' credentials count, signed with an `assertionMethod` key; status lists fetched and checked, fail closed, a verified list reused for at most `PORTITOR_STATUS_LIST_CACHE_SECONDS` (default 300 s, so a revocation takes up to that long to be seen; ADR 0016, amended in 0.11); a presentation with any failing credential is refused (DCP v1.0.1, 5.4.3); the issuer issues only to registered participants (ADR 0018) |
| Gaia-X compliance | A forged compliance credential, or someone else's, admits a party | Only the compliance services the operator lists, separate from the dataspace's issuers; the service's signature; the `digestSRI` link to a legal person credential issued and signed by the caller; the holder binding is skipped for these two credentials only (ADR 0022); the identity takes an answer only from `PORTITOR_GAIAX_COMPLIANCE_DID` (ADR 0021) |
| Private keys | Keys leak from the internet-facing connector; an old key is kept after a key change | Keys live only in `portitor-identity` and `portitor-issuer`, not in the connector (ADR 0003); a retired key is listed by its public key only, and a private key in the retired list stops the start, so the old private key can be destroyed at once (ADR 0015, ADR 0018, TD-05) |
| Schema and vocabulary bundle | A forged or changed bundle makes connectors accept bad datasets or refuse good ones | A separate bundle key, kept offline: the issuer holds only its public part and refuses to start if it is the credential key; signature checked against the authority's DID document; the version (never older than the one in use) and authority; SHA-256 of every file; size limits (manifest 1 MiB, 64 MiB per file, 256 MiB in all) and a 120 s timeout per download; a published version is never written again |
| Validation code | Crafted shapes or JSON-LD make the connector fetch remote content, or exploit an XML parser | JSON-LD contexts come only from a map the caller fills (the bundle's and the DSP contexts), never from the network; no Portitor binary parses RDF/XML (`bundle build` refuses it, code lists travel as N-Triples); fuzz targets `shacl_load`, `shacl_jsonld` and `bundle_manifest` |
| Internal services | A consumer gives a `callbackAddress` that points at an internal service, so the connector calls it (SSRF) | Only `https://` URLs with a host, unless the operator allows `http://`; the response body is never read; a 10 s timeout; blocking internal addresses *(planned)* |
| Negotiation state | A consumer drives a negotiation out of order, or two messages race | The DSP state machine as a pure function in the domain; every change is a compare-and-update on a version number; a message that races our acknowledgement is accepted only as proof of delivery of our saved pending message (ADR 0007) |
| Service availability | Crafted or oversized messages crash or exhaust a service | Typed parsing with no `unsafe`; a request size limit per port (`PORTITOR_MAX_REQUEST_BYTES`, `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES`, 2 MiB by default) and a fixed 64 KiB limit on routes whose messages are small by specification, HTTP 413 above them ([R-18](RISKS_AND_TECHNICAL_DEBTS.md#r-18)); fuzzing of every parser of input from outside the process (ClusterFuzzLite; list in [Parsers and fuzz targets](#parsers-and-fuzz-targets)); a nesting limit in SHACL validation |
| Operational data (health answers, traces, metrics) | A caller reads health answers, or a receiver reads traces and metrics, and learns secrets, partners or process ids; a partner links our internal calls through trace context | Health checks only on the management port, which stays on the operator's network; failure messages without secrets; export off by default and only to the receiver the operator names; attribute allow-list (no tokens, keys, headers, bodies or id-bearing paths); client metrics name only hosts inside the participant (no untrusted cardinality); `traceparent` neither sent to nor taken from other participants, and taken on `/sts/token` only after the client is authenticated (ADR 0023) |
| Evidence of who did what | A partner denies an agreement or a data pull; an operator's action cannot be traced to a person | State changes of negotiations and transfers logged with their process ids; agreements kept with the counter-party's DID; the issuer logs registrations and revocations. No audit trail of data pulls or management calls, and one key per management API: findings F-02 and F-03 of the [0.11 review](#threat-model-review-011-2026-11-17) ([TD-31](RISKS_AND_TECHNICAL_DEBTS.md#td-31), [TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)) |
| Users' supply chain | A tampered release or dependency | Pinned CI actions; signed commits; Dependabot, dependency review, OSV, cargo-deny (RustSec advisories, licence allowlist, crate sources), cargo-vet (every crate version audited or exempted) and ORT; signed images with SBOM and provenance attestations |

## Trust boundaries

1. **Remote participants** reach the DSP and DCP endpoints. Everything that
   crosses is parsed into types and validated before the domain sees it.
   With a bundle that has message shapes, DSP negotiation and transfer
   messages are also checked against them, in and out (0.7); catalog
   requests are not.
2. **The DID web** (`did:web` documents fetched over HTTPS) is resolved
   through a port; the domain checks the keys it returns.
   From 0.8 the identity also calls a Gaia-X compliance service through a
   port (`GaiaxCompliance`) and the shared HTTP client (HTTPS only,
   limits, no redirects), and checks its answer before storing it.
3. **The data plane** receives signals only from its own control plane
   (in the same process for 0.x).
4. **Operators** configure the services through environment variables
   and give the connector its datasets source; harvested datasets are
   checked one by one before they reach the catalog.
5. **The dataspace authority** signs the schema and vocabulary bundle
   offline; connectors trust it only through the bundle key in its
   `did:web` document.
6. **The operator's monitoring (0.9).** Probes call the health checks on
   the management port without a key. With telemetry on, the components
   push traces and metrics to the OTLP receiver the operator names; that
   receiver sees partners' hosts and DSP process ids, so it must be the
   operator's own (DEPLOYING, "Security").

### Threat model review 0.11 (2026-11-17)

**Method.** The maintainer kept the table above and asked for a STRIDE
pass over each trust boundary (maintainer, 2026-11-17; the 0.11 plan,
now one row of [ROADMAP, "Released"](ROADMAP.md#released)).
For each boundary and each STRIDE category (Spoofing, Tampering,
Repudiation, Information disclosure, Denial of service, Elevation of
privilege) the tables below name the threat in one line and how it is
countered, with the file or ADR where the counter was checked in the
code. Where nothing counters it, or only partly, the cell names a
finding (F-nn, listed at the end). "Not checked" means the claim was
not checked in the code for this review. Severity: **high** (an outside
party can break a security goal now), **medium** (a goal is missing or
rests on the operator alone), **low** (defence in depth, or the cost is
bounded).

**Scope.** The code of commit `3f36409` (version 0.11.0, before the 0.11
changes), all three components, with the work added since 0.5: identity
and STS (ADR 0015), issuer and delivery outbox (ADR 0018), credential
requests, ODRL credential constraints (ADR 0019), bundle and dataset
harvest (ADR 0005, ADR 0020), Gaia-X onboarding and compliance (ADR
0021, ADR 0022), health checks and telemetry (ADR 0023), cargo-vet
(0.10). Not in scope: the 0.11 changes made at the same time (request
limits per port, status list cache, outbox retries, retired keys), the
reverse proxy and TLS (operator's job, [TD-19](RISKS_AND_TECHNICAL_DEBTS.md#td-19)),
and the host. The rows of the table above that 0.11 changes were left
as they are.

Paths below are under `crates/`.

#### 1. Remote participants (DSP, DCP endpoints)

The public ports: the connector's DSP port and data plane, the
identity's DID document, credential service and STS, the issuer's DID
document, issuer service, status lists and bundles.

| | Threat | Countered by, or finding |
|---|---|---|
| S | A caller forges or replays a self-issued token, or uses another participant's process | `validate_self_issued` (key with `capabilityInvocation`, `aud`, `iss` = `sub`, expiry, `jti`) in `portitor-dcp/src/id_token.rs`; run by `require_caller` on every DSP route (`portitor-connector/src/lib.rs`, `auth.rs`) and by `portitor-runtime/src/caller.rs` for the identity and the issuer; a `MembershipCredential` from a trusted issuer (`auth/dcp.rs`, ADR 0016); each process bound to the DID that started it (`portitor-dsp/src/negotiation/provider.rs`, `transfer/provider.rs`); the issuer admits only registered DIDs and the Storage API only trusted issuers, both before resolving the DID (`portitor-issuer/src/requests.rs`, `portitor-identity/src/caller.rs`) |
| T | A message is changed on the way | TLS at the operator's proxy (not checked, TD-19). DSP messages are not signed (DSP 2025-1 has no message signature); tokens, credentials and presentations are, and are verified (`auth/dcp.rs`); typed parsing and, with a bundle, the SHACL message guard (`portitor-connector/src/shacl.rs`) |
| R | A partner denies it agreed, or denies it pulled data | Partly: state changes logged with process ids (`portitor-connector/src/state_events.rs`); the agreement and the counter-party's DID stored (`portitor-connector/src/store.rs`). The caller's token and presentation are not kept, and data pulls are not logged: **F-02** |
| I | A caller learns offers, flows or reasons it should not | A refusal without a reason: on DSP routes 404 for negotiations and transfers and 401 for the catalog (`auth.rs` `unauthorized`, `refusal_status`; ADR 0016, amended in 0.12), 401 on the identity and the issuer (`portitor-runtime/src/caller.rs` `refused`); the data plane answers the same 401 for an unknown flow, a wrong or expired token and a stopped flow (`portitor-connector/src/data_plane.rs`); a presentation holds only what the scopes and our access token allow (`portitor-identity/src/presentation.rs`); a request status only for the holder that made it (`portitor-issuer/src/requests.rs`). The catalog shows every offer to every member ([TD-13](RISKS_AND_TECHNICAL_DEBTS.md#td-13), known) |
| D | A caller exhausts a component | 256 requests at once and 60 s per request on every public port (`portitor-runtime/src/serve.rs`, applied in each `main.rs`); a 20 s deadline, at most 8 presentations of 16 credentials and 2 MiB status lists per check (`auth/dcp.rs`); 10 s and 64 KiB per outgoing call (`portitor-runtime/src/http.rs`); body limits ([R-18](RISKS_AND_TECHNICAL_DEBTS.md#r-18), changed in 0.11). No limit per partner: **F-10**. The identity fetches any caller's DID before cheaper checks: **F-06**. One slow holder delays every issuer delivery: **F-09** |
| E | A caller gets more than its credentials allow | A policy not understood fails closed (`portitor-dsp/src/policy.rs`, ADR 0009); credential constraints checked again at each transfer request (ADR 0019); credentials from untrusted issuers skipped (`auth/dcp.rs`). `neq` and `isNoneOf` can be met with another credential of the type: **F-07**. A trusted issuer is trusted for every credential type: **F-08** |

#### 2. The DID web and the Gaia-X compliance service

| | Threat | Countered by, or finding |
|---|---|---|
| S | A fake DID document, or a fake compliance service | HTTPS with the system's CA certificates, no redirects, and the document's `id` must be the DID (`portitor-runtime/src/did.rs`, `http.rs`). Trust in a `did:web` rests on the DNS and web PKI of its host, by the method's design (not countered by Portitor). The identity takes a compliance answer only from `PORTITOR_GAIAX_COMPLIANCE_DID` (`portitor-identity/tests/gaiax.rs`, `an_answer_from_another_service_is_refused`) |
| T | The document is changed, or a key its owner removed is still used | TLS; the key relationship is checked for each use (`capabilityInvocation` for tokens, `assertionMethod` for credentials, `authentication` for presentations, `auth/dcp.rs`). A removed key stays trusted until the cached document expires: `PORTITOR_DID_CACHE_SECONDS`, 5 minutes by default, `0` for no cache (from 0.12): **F-04** |
| R | The compliance service denies what it answered | Its answer is a signed credential, checked and stored with the run (`portitor-identity/src/gaiax.rs`, table `gaiax_run`) |
| I | The hosts we fetch from learn whom we talk to | By design of `did:web` (each verifier fetches the caller's document); not countered. The compliance service gets the participant's Gaia-X credentials, by design (ADR 0021) |
| D | A DID host or the compliance service is slow or down | 10 s and 64 KiB per fetch; documents cached `PORTITOR_DID_CACHE_SECONDS` (5 minutes by default), at most 1,000 (`portitor-runtime/src/did.rs`); a compliance run repeats at two thirds of the credential's validity and keeps the credential it has (ADR 0021; the keeping not checked in code) |
| E | The caller chooses the URL we fetch (its DID, its credential service) to reach an internal service | `https://` only, no redirects, time and size limits; internal addresses are not blocked: **F-01** |

#### 3. The data plane

The built-in data plane: the signals from the control plane (in the
same process) and the consumer's pulls at `GET /data/{flowId}`.

| | Threat | Countered by, or finding |
|---|---|---|
| S | A caller guesses or reuses a data token; something else sends signals | 32 random bytes, only the SHA-256 kept, an expiry, served only while the flow is STARTED (`portitor-connector/src/data_plane.rs`, ADR 0013). Signals are function calls in the process, no network ([TD-17](RISKS_AND_TECHNICAL_DEBTS.md#td-17)) |
| T | The data is changed on the way | TLS at the proxy (not checked, TD-19); the upstream URL comes only from the operator's datasets file, never from the consumer (`data_plane.rs`, ADR 0020) |
| R | Nobody can show who pulled which data, when | Not logged on success (only a failed upstream fetch is logged in `data_plane.rs`): **F-02** |
| I | The token leaks and someone else pulls the data | A bearer token: whoever holds it gets the data until the flow ends or the token expires ([TD-02](RISKS_AND_TECHNICAL_DEBTS.md#td-02)); the consumer's headers are not passed upstream; the consumer drops its token when the transfer ends ([R-19](RISKS_AND_TECHNICAL_DEBTS.md#r-19)) |
| D | Many or slow pulls | 30 s for the upstream answer (`data_plane.rs`); bodies streamed, not buffered; the 60 s limit ends when the answer starts (`serve.rs`), so long downloads are not cut, and nothing limits them per partner: **F-10** |
| E | A consumer reaches another dataset or another upstream | A transfer is for the agreed dataset (ADR 0013); the flow keeps its dataset id (table `data_flow`, `portitor-connector/src/store.rs`), and the URL is looked up for that id in the harvest's read copy (`data_plane.rs`) |

#### 4. Operators

Settings, the datasets file, the volumes and the management ports.

| | Threat | Countered by, or finding |
|---|---|---|
| S | Someone else calls the management API or the STS | An API key of at least 32 characters, only an HMAC of it kept, compared in constant time (`portitor-runtime/src/api_key.rs`); management ports bind to `127.0.0.1` by default (`DEFAULT_MANAGEMENT_BIND` in the connector's `management.rs`, the identity's and the issuer's `config.rs`); the API is off without a key file. The STS client secret is checked the same way (`portitor-identity/src/sts.rs`), but the STS shares the identity's public port: **F-05** |
| T | The datasets file or a volume is changed | Each harvested dataset is checked before it reaches the catalog (ADR 0020). The volumes are trusted: the SQLite files, and the bundle cache with the bundle key stored next to it (`portitor-bundle/src/store.rs`); DEPLOYING asks to protect them (not countered in code) |
| R | An operator's action cannot be traced to a person | One key per component; failed key checks and some changes (for example `DELETE /v1/credentials/{id}` on the identity) are not logged (`portitor-identity/src/management.rs`): **F-02**, **F-03** |
| I | The management API or a health answer shows secrets | The management API shows consumer data tokens ([R-19](RISKS_AND_TECHNICAL_DEBTS.md#r-19), known); health checks name a check and a reason, never a secret (ADR 0023; `tests/health.rs` of the connector and the identity, the issuer's `tests/issuer.rs`) |
| D | Load on the management port | No request limits there (`with_limits` is applied to the public ports only); the port is on the operator's network. Accepted, no finding |
| E | Whoever holds the key does everything | One key, every right: **F-03** ([TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04)) |

#### 5. The dataspace authority

The authority's bundle and its issuer, as seen by participants.

| | Threat | Countered by, or finding |
|---|---|---|
| S | A forged bundle or a forged credential | Only `<authority>#bundle-key` under `assertionMethod`, ES256 only (ADR 0005; `portitor-connector/tests/bundle_source.rs`); credentials only from `PORTITOR_TRUSTED_ISSUERS`, with an `assertionMethod` key (`auth/dcp.rs`) |
| T | A changed bundle or an older one | Signature over the exact manifest bytes, SHA-256 of every file, never a version older than the one in use (ADR 0005); the issuer serves no path outside its folder (`portitor-issuer/tests/bundle.rs`). `latest.json` is not signed ([R-22](RISKS_AND_TECHNICAL_DEBTS.md#r-22), known) |
| R | The authority denies a bundle or a credential | Both are signed; the connector keeps the signed manifest and the key it used (`portitor-bundle/src/store.rs`); the issuer logs registrations, deletions and revocations (`portitor-issuer/src/management.rs`) |
| I | The bundle or the issuer shows more than it should | The bundle is public by design; issuance answers go only to the holder (`portitor-issuer/src/requests.rs`) |
| D | Huge bundles, or the authority is down | 1 MiB manifest, 64 MiB per file, 256 MiB in all, 120 s per download (`portitor-bundle/src/source.rs`); the cached copy is used while the authority is down (ADR 0005) |
| E | Shapes run code, or the issuer key admits anyone | `portitor-shacl` refuses terms it does not implement, SPARQL constraints among them (`tests/refusal.rs`), and stops after `MAX_CHECKS` with a violation (`src/validate.rs`); the issuer key is the root of trust ([R-20](RISKS_AND_TECHNICAL_DEBTS.md#r-20), known); every trusted issuer may issue every type: **F-08** |

#### 6. The operator's monitoring

| | Threat | Countered by, or finding |
|---|---|---|
| S | Someone poses as the OTLP receiver | The operator names the endpoint (ADR 0023); whether TLS to it is required was not checked |
| T | Exported data is changed | Not relevant: Portitor never reads it back |
| R | Traces or logs are used as evidence | They are not an audit trail (no user, data pulls not logged): **F-02** |
| I | The receiver or a probe learns secrets | Attribute allow-list and no export without an endpoint (`portitor-runtime/tests/telemetry_export.rs`); `traceparent` only inside the participant (`portitor-connector/tests/trace_context.rs`, `portitor-identity/tests/sts_trace.rs`) |
| D | A receiver that fails stops the component | `a_failing_receiver_does_not_stop_the_process` in `telemetry_export.rs`; throughput cost at saturation ([R-37](RISKS_AND_TECHNICAL_DEBTS.md#r-37), known) |
| E | A health probe changes state | The health checks only read (ADR 0023); every other management route needs the key (`tests/health.rs`) |

The supply chain is not a runtime boundary; the row "Users' supply
chain" above was checked: `cargo vet --locked` runs in
`.github/workflows/dependencies.yml`, with 159 exemptions in
`supply-chain/config.toml` ([R-04](RISKS_AND_TECHNICAL_DEBTS.md#r-04)).

#### Findings

No high finding. None needs a design change before 0.13, but F-01 and
F-05 would add settings, so they are best decided before the 0.13
freeze.

| Id | Boundary | Finding | Severity | Proposed action | Tracked in |
|---|---|---|---|---|---|
| F-01 | 1, 2 | Internal addresses are not blocked on calls whose URL a partner chooses: the `callbackAddress`, the caller's DID document and credential service, and, for the issuer, a registered holder's credential service (`portitor-issuer/src/delivery.rs`). The table row "Internal services" says *(planned)*, but no release has the work | medium | The maintainer names a release; before the 0.13 freeze if the block is on by default, since turning it on later changes behaviour. Until then DEPLOYING's egress firewall advice stands | [R-17](RISKS_AND_TECHNICAL_DEBTS.md#r-17) |
| F-02 | 1, 3, 4, 6 | No audit trail: data pulls are not logged, the caller's token is not kept with an agreement, failed management key checks and some management changes are not logged, and a management call has no person behind it | medium | Research with the TD-04 ADR (0.12): what an audit record holds, where it goes, how long it is kept (EHDS rules for health data) | [TD-31](RISKS_AND_TECHNICAL_DEBTS.md#td-31) |
| F-03 | 4 | One API key per management API, with every right | medium | The TD-04 ADR (0.12) | [TD-04](RISKS_AND_TECHNICAL_DEBTS.md#td-04) |
| F-04 | 2 | DID documents are cached 5 minutes, fixed (ADR 0016 says "default"; there is no setting): a key its owner removed after a leak is still accepted for up to 5 minutes. The cache is bounded (1,000) and caches no failures. **From 0.12 the time is a setting**, `PORTITOR_DID_CACHE_SECONDS` (default 300, at most 3600, `0` for no cache; ADR 0016, amended in 0.12; DEPLOYING, "DID document cache"). With the default the 5 minutes remain | low | Say it in this case and DEPLOYING; decide with the retired keys work (0.11) whether the time becomes a setting | [R-41](RISKS_AND_TECHNICAL_DEBTS.md#r-41) |
| F-05 | 4 | The STS (`/sts/token`) is served on the identity's public port; keeping it internal depends on the proxy's routing (DEPLOYING). The secret is long and compared in constant time, so guessing is not practical | low | A separate listen address for the STS, or a check in the deployment tests that the proxy does not route it | [R-42](RISKS_AND_TECHNICAL_DEBTS.md#r-42) |
| F-06 | 1 | The identity's presentation API resolves the caller's DID before it checks `aud` and the access token that only our STS signs (`portitor-runtime/src/caller.rs`, `Callers::Anyone`), so anyone can make it fetch a `did.json` from a host of their choice | low | Check `aud` and the access token's signature before resolving | [R-17](RISKS_AND_TECHNICAL_DEBTS.md#r-17) |
| F-07 | 1 | `neq` and `isNoneOf` hold if one credential of the type has another value (ADR 0019), and the holder chooses what it presents, so they cannot exclude a holder that has a second credential of that type | low | Say in ADR 0019 and DEPLOYING that negative operators are not deny lists | [R-43](RISKS_AND_TECHNICAL_DEBTS.md#r-43) |
| F-08 | 1, 5 | `PORTITOR_TRUSTED_ISSUERS` is one list for every credential type (ADR 0019): any trusted issuer may issue a `MembershipCredential`. EDC limits each trusted issuer to its `supportedtypes` | low (one issuer today); grows with several data spaces and with the HDAB issuer of 1.3 | Part of [ADR 0024](adr/0024-several-data-spaces.md) (several data spaces) | [TD-03](RISKS_AND_TECHNICAL_DEBTS.md#td-03) |
| F-09 | 1 | The issuer sends deliveries one after the other, up to 10 s each; a registered holder whose credential service hangs delays every other holder (bounded by `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` and the retry waits) | low | Send to different holders side by side, a few at a time | [R-44](RISKS_AND_TECHNICAL_DEBTS.md#r-44) |
| F-10 | 1, 3 | No limit per partner: one authenticated partner can use the 256 request slots, and downloads, once started, are not limited | low | Document rate limits per client at the reverse proxy in DEPLOYING; a limit in Portitor only if measured | [R-45](RISKS_AND_TECHNICAL_DEBTS.md#r-45) |

The open items the maintainer asked to check are F-01 (SSRF: no
release has it), F-02 (repudiation), F-03 (the single management key)
and F-04 (the DID document cache).

## Secure design principles applied

- **Economy of mechanism:** messages are processed as plain JSON with typed
  structs, which DSP and DCP allow. From 0.7 a JSON-LD processor
  (`oxjsonld`) is used only to turn datasets and messages into RDF for the
  SHACL checks, with contexts from local data; there is still no remote
  context loading.
- **Fail-safe defaults:** unknown types, algorithms and fields that matter
  are rejected, not ignored.
- **Complete mediation:** every request goes through the same parser and,
  from 0.5, the same token check.
- **Least privilege:** non-root container, no shell; keys only where
  needed (ADR 0003); CI tokens are read-only by default.
- **Open design:** code, threat model and decisions are public.

## What a panic does

A panic is a bug caught at run time (an index out of range, a failed
`expect`). `unwrap` is denied by Clippy and every parser is fuzzed, so
it should be rare. Until 0.11 the release build set `panic = "abort"`:
any panic ended the process and every request in flight, so one request
that found such a bug was a denial of service until the container
started again
([TD-41](RISKS_AND_TECHNICAL_DEBTS.md#td-41)). From 0.12 the setting is
gone (maintainer, 2026-11-18) and a panic ends only the work it
happened in (`crates/portitor-runtime/src/panics.rs`):

- **A request handler:** the caller gets HTTP 500 with a problem
  document (`application/problem+json`, detail `internal error`), also
  on DSP routes; the other requests go on. This holds for every
  HTTP server of the three components, public and management.
- **A background task that runs for as long as the process** (the
  dataset, bundle and identity schedules of the connector, the
  certificate watch and Gaia-X schedule of the identity, the issuer's
  outbox): it starts again after a pause of 5 seconds, twice as long
  after each further panic in a row, at most 5 minutes; a task that ran
  for more than 5 minutes starts at 5 seconds again. So a task that
  panics on every run does not call other services (the Gaia-X
  compliance service) every few seconds. A dataset harvest run that
  panics is skipped; the next one runs on schedule.
- **A background task for one piece of work** (sending one DSP message,
  deciding one credential request, one Gaia-X run): the work stays as
  it was saved. A DSP message is sent again after the same pauses (5
  seconds, doubling up to 5 minutes), for as long as the process runs;
  sending a saved message again is safe (ADR 0007). A credential
  request stays `RECEIVED` and is tried again on each round of the
  issuer's outbox (at most a minute apart); a request that panics every
  time does not hold up the other requests or the deliveries.
- **The management server's task:** if it ends, the process stops with
  an error, as a component without health checks must not go on.

Every case is one log line at ERROR and a count
(`portitor.http.request.panicked`, `portitor.task.panicked`;
OPERATING, "Traces and metrics"). The panic's message is never logged
or sent, only the place in the code: the message can quote the value
that caused it, which may come from a request.

Data stays consistent: a database write of several statements is one
transaction, which `rusqlite` rolls back when the panic drops it, and
every other write is one statement. A lock held during a panic is taken
again (Rust marks it "poisoned"); each place says why that is safe, and
each store has a test for it (`store/poison_tests.rs`; the checks are
in `portitor-runtime/src/sqlite.rs`, `testing`). Two caches (DID
documents, status lists) switch themselves off instead after a panic
inside them, until the next start: slower, never wrong.

What it costs: the release binaries are about 13% larger (the code that
unwinds; [BENCHMARKS](BENCHMARKS.md), "Footprint"). What stays: a bug
that panics on every start, or in a task that starts again, shows as
repeated errors, so a restart policy and an alert on the two counts are
still needed. A panic while a download's body is streamed closes that
one connection.

## Common weaknesses countered

| Weakness | How |
|---|---|
| CWE-20, improper input validation | Typed parsing; DSP JSON Schemas checked in tests |
| CWE-347, improper verification of a signature | `ring` only; algorithm allow-list; `alg: none` rejected; the bundle manifest is verified over its exact bytes |
| CWE-611, XML external entities | No RDF/XML or other XML parser in any Portitor binary |
| CWE-494, download of code or data without an integrity check | The bundle: signature and SHA-256 of every file before use |
| CWE-294, replay | `jti` cache with expiry |
| CWE-787 / CWE-416, memory safety | Rust with `unsafe` forbidden |
| CWE-1104, unmaintained components | Few dependencies; Dependabot, OSV, and cargo-deny failing on unmaintained or yanked crates |

## Evidence

- **Tests:** unit tests per crate; DSP and DCP JSON Schema tests; in CI,
  the DSP TCK (metadata, catalog, negotiation and transfer groups), the
  DCP TCK (credential service, verifier and issuer tests), the EDC
  interoperability test and, from 0.8, the Gaia-X end-to-end test
  against a local compliance service in test mode. From 0.9,
  `crates/portitor-runtime/tests/telemetry_export.rs` checks that
  nothing is exported without an endpoint, that no secret or
  `Authorization` value appears in any export, and that no trace context
  crosses a DSP call; the health tests (`tests/health.rs` in the
  connector and the identity, and the issuer's `tests/issuer.rs`) check
  that the checks are open while the management API keeps its key.
  From 0.11, tests check the request size limits of every limited port
  and route, the status list cache (a stale list is fetched again, and
  a failed fetch refuses the caller), and that a key change with the
  old key retired leaves no verification gap (TESTING, "Unit and
  integration tests").
- **Static analysis:** Clippy pedantic with warnings as errors; CodeQL and
  SonarQube Cloud quality gate on every pull request. A CodeQL finding
  that is not on the list of accepted findings fails the CodeQL job, on
  a pull request and on `main` ([TESTING](TESTING.md#codeql)).
- **Supply chain:** pinned workflow actions, least-privilege workflow
  tokens, REUSE compliance; OpenSSF Scorecard; cargo-vet, so a new or
  bumped crate needs a recorded audit or exemption before it merges.
- **Fuzzing:** every parser below has a fuzz target. ClusterFuzzLite
  builds every target in `fuzz/fuzz_targets` on each pull request and
  each week (`docs/TESTING.md`, "Fuzzing").

### Parsers and fuzz targets

A parser is any code that reads bytes from outside the process that the
operator did not write: network messages, DID documents, certificates,
tokens, the bundle and harvested datasets. The operator's own
configuration is not a parser (maintainer, 2026-11-17). Each target
also checks rules that must hold for what it reads, not only that
nothing panics.

| Input | Parser | Fuzz target |
|---|---|---|
| DSP catalog request (compacted or expanded JSON-LD) | `portitor_dsp::catalog::read_catalog_request`, `CatalogRequestMessage` | `dsp_messages` |
| DSP negotiation messages, and the other party's `ContractNegotiation` and `ContractNegotiationError` answers | `portitor_dsp::negotiation` | `dsp_negotiation` |
| DSP transfer messages, and the other party's `TransferProcess` and `TransferError` answers | `portitor_dsp::transfer` | `dsp_transfer` |
| ODRL policies in offers and agreements | `portitor_dsp::policy` | `odrl_policy` |
| `xsd:date` and `xsd:dateTime` in policies and credentials | `portitor_time` | `time_parse`, `odrl_policy` |
| DSP messages checked against the bundle's message shapes | `portitor_connector::shacl` (`ShaclGuard`) | `bundle_rules` |
| JSON-LD from DSP messages and harvested datasets | `portitor_shacl::jsonld_to_graph` | `shacl_jsonld` |
| Literals checked with `sh:datatype` (lexical forms) | `portitor-shacl`, `lexical.rs` | `shacl_datatypes` |
| Self-issued ID tokens (JWS header and claims, claims read before the check) | `portitor_dcp::Jws`, `unverified_claims`, `validate_self_issued` | `dcp_token_parse`, `dcp_token_validate` |
| Access tokens sent to the credential service | `portitor_dcp::holder::AccessToken` | `dcp_access_token` |
| DCP messages: presentation query and response, credential, offer, credential request, request status, issuer metadata; scopes | `portitor_dcp::message`, `portitor_dcp::scope` | `dcp_messages` |
| Credentials and presentations (VC 1.1 and 2.0 JWT) | `portitor_dcp::credential` | `dcp_credentials` |
| Status lists (base64 and GZIP) | `portitor_dcp::status_list` | `dcp_status_list`, `dcp_credentials` |
| Gaia-X credentials and compliance answers; JCS | `portitor_dcp::gaiax`, `gaiax_sign::check_answer` | `gaiax_compliance` |
| DID documents, and their keys (JWK) used to check signatures | `portitor_dcp::DidDocument`, `Jws::verify`, `bundle::key_from_document` | `dcp_did_document` |
| `did:web` DIDs | `portitor_dcp::did_web_url` | `dcp_token_parse` |
| X.509 certificate chains (PEM and DER) | `portitor_dcp::x509` | `dcp_x509` |
| `Authorization: Bearer` headers, and URLs that other participants choose (`callbackAddress`, Credential Service) | `portitor_runtime::http::bearer_token`, `outbound_url_allowed` | `http_inputs` |
| The bundle: `latest.json`, `manifest.json` and `manifest.jws` | `portitor_dcp::bundle` | `bundle_manifest` |
| The bundle: shapes and vocabularies (Turtle, N-Triples) | `portitor_shacl::Shapes::load` | `shacl_load`, `bundle_rules` |
| The bundle: JSON-LD contexts | `portitor_connector::shacl::BundleRules` | `bundle_rules` |
| Harvested datasets file | `portitor_connector::harvest` (`FileSource`, `DspCheck`, the bundle's dataset shapes) | `harvest_datasets`, `dsp_messages` |
| Data Plane Signaling messages | `portitor_dps::message` | `dps_messages` |

Notes:

- Data Plane Signaling messages are function calls inside the connector
  in 0.x. They are fuzzed because they keep the shape they have on HTTP,
  for a data plane in its own process later.
- Not parsers by the definition above: settings, key files, the issuer's
  credential definitions, the Gaia-X participant file, the management
  APIs (called with the operator's key), and the files each process
  writes for itself (SQLite stores, the bundle cache). The TCK routes are
  only in test builds.
- The STS form (`/sts/token`) is read into plain strings by axum; its
  scopes go through `Scope::parse` (`dcp_messages`).
- Bytes that are passed on but not read, such as the data a data plane
  serves, are not parsed.
- serde_json and Oxigraph's Turtle and N-Triples readers are reached
  through these targets. The HTTP and TLS libraries (hyper, axum,
  rustls) are not fuzzed here.

## OpenSSF Baseline

Target: **level 1** of the OpenSSF Open Source Project Security Baseline,
version 2026-08-28 ([ADR 0012](adr/0012-openssf-baseline.md)). The target
stays level 1 for 1.0 (maintainer, 2026-11-20;
[R-05](RISKS_AND_TECHNICAL_DEBTS.md#r-05)).

### Level 1 self-assessment (2026-11-20)

**Source.** The control list is the Baseline's own source, tag
[`v2026.08.28`](https://github.com/ossf/security-baseline/tree/v2026.08.28/baseline)
(commit `a26a796`): every requirement that applies to maturity level 1.
The requirement column is our short paraphrase; the Baseline's text
decides.

**Scope.** The repository at version 0.14.0, the release candidate.

**Status.** *Met*: the evidence is in the repository, or is a GitHub
setting read through the GitHub API on 2026-11-20, and was checked
there. *Met, to confirm*: the evidence is a GitHub setting that could
not be read; the maintainer confirms it in the
[settings checklist](#settings-to-confirm) below. *Not met* and *Not
applicable* mean what they say.

**Result.** 24 requirements apply (one more is retired): 19 met, 4 met,
to confirm, 1 not applicable.
Level 1 is reached when the four settings are confirmed. Private
vulnerability reporting, the one requirement that was not met when this
was written, was turned on on 2026-11-21.

| Control | Requirement (paraphrase) | Status | Evidence |
|---|---|---|---|
| OSPS-AC-01.01 | Reading or changing a sensitive resource of the repository needs multi-factor authentication | Met, to confirm | The only account with such access is the maintainer's ([GOVERNANCE, "Roles"](../GOVERNANCE.md#roles)). The repository belongs to a personal account, so no organisation rule can show it. Checklist 1 |
| OSPS-AC-02.01 | A new collaborator gets permissions by hand, or the lowest ones by default | Met, to confirm | Only the owner can add a collaborator, by hand; none is planned without a record in [GOVERNANCE](../GOVERNANCE.md#roles). Checklist 2 |
| OSPS-AC-03.01 | A direct commit to the primary branch is prevented | Met | The ruleset `main`, as read: active, on the default branch, with the rule "pull request" (one approval) and required status checks; an admin may bypass it only to merge a pull request ([R-02](RISKS_AND_TECHNICAL_DEBTS.md#r-02)). Described in [ROADMAP, "Repository settings"](ROADMAP.md#decisions-recorded-here) |
| OSPS-AC-03.02 | Deleting the primary branch is prevented or needs explicit confirmation | Met | The same ruleset, as read: the rules "deletion" and "non fast forward" (no deletion, no force push) |
| OSPS-BR-01.01 | Untrusted metadata (branch names, titles, tags) is validated before the pipeline uses it | Met | No `run:` step holds an untrusted `${{ }}` expression: values reach scripts as environment variables ([`changes.yml`](../.github/workflows/changes.yml), step "Which jobs does this change need?"); the tag name is matched against a pattern before use ([`release.yml`](../.github/workflows/release.yml), job `verify`); values fetched for the website are reduced to plain characters ([`pages.yml`](../.github/workflows/pages.yml), `clean`). CodeQL analyses the workflows ([`codeql.yml`](../.github/workflows/codeql.yml), language `actions`) |
| OSPS-BR-01.02 | Retired by the Baseline | Retired | |
| OSPS-BR-01.03 | Untrusted code (a pull request) cannot reach privileged credentials | Met, to confirm | No workflow uses `pull_request_target`. Every workflow starts with `permissions: contents: read`. The only repository secret a pull request job reads, `SONAR_TOKEN`, goes to the scan step alone, and GitHub gives no secrets to a pull request from a fork ([`ci.yml`](../.github/workflows/ci.yml), job `sonar`). Pushing, signing and releasing run only for a tag, in the `release` environment ([`release.yml`](../.github/workflows/release.yml), jobs `images` and `release`). Checklist 3 |
| OSPS-BR-03.01 | Every official project address is served over an encrypted channel only | Met, to confirm | The project's addresses are `https://github.com/brunopacheco1/portitor` and `https://bruno.pacheco.lu/portitor/` ([README](../README.md)). The website is on GitHub Pages with its own domain. Checklist 4 |
| OSPS-BR-03.02 | Distribution channels are protected against an attacker in the middle | Met | Images come from `ghcr.io` and release files from GitHub Releases, both over HTTPS; each image is signed and has provenance ([RELEASING, "Checking a release"](RELEASING.md#checking-a-release)) |
| OSPS-BR-07.01 | Secrets are kept out of version control | Met | Secret scanning and push protection are on, as read ([ROADMAP, "Repository settings"](ROADMAP.md#decisions-recorded-here)); the rule and the naming of test keys ([AGENTS, "Rules"](../AGENTS.md#rules)); checkouts do not keep the workflow token (`persist-credentials: false`), except the tag check of the release |
| OSPS-DO-01.01 | User guides for all basic functionality | Met | [README](../README.md), [DEPLOYING](DEPLOYING.md), [OPERATING](OPERATING.md), the OpenAPI files in [`docs/api/`](api/), and the quickstart on the [website](https://bruno.pacheco.lu/portitor/) |
| OSPS-DO-02.01 | A guide for reporting defects | Met | [CONTRIBUTING](../CONTRIBUTING.md#before-you-start); the issue forms in [`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/); vulnerabilities in [SECURITY](../SECURITY.md#reporting-a-vulnerability) |
| OSPS-GV-02.01 | A public place to discuss proposed changes and problems of use | Met | GitHub issues (on, as read) and pull requests ([CONTRIBUTING](../CONTRIBUTING.md#before-you-start); [GOVERNANCE, "Decisions"](../GOVERNANCE.md#decisions)) |
| OSPS-GV-03.01 | The contribution process is explained | Met | [CONTRIBUTING](../CONTRIBUTING.md#pull-requests) |
| OSPS-LE-02.01 | The source code licence is an open source licence (OSI or FSF) | Met | Apache-2.0 ([LICENSE](../LICENSE)); every file carries its licence, checked by `reuse lint` in [`ci.yml`](../.github/workflows/ci.yml) (job `reuse`) |
| OSPS-LE-02.02 | The licence of the released software is an open source licence | Met | The images are Apache-2.0 and hold one third-party file, the CA certificate bundle, MPL-2.0 ([NOTICE](../NOTICE)). Crates linked into the binaries must have a licence on the allowlist in [`deny.toml`](../deny.toml) (cargo-deny in [`dependencies.yml`](../.github/workflows/dependencies.yml)) |
| OSPS-LE-03.01 | The source licence is in `LICENSE`, `COPYING` or `LICENSES/` | Met | [LICENSE](../LICENSE) and [`LICENSES/`](../LICENSES/) |
| OSPS-LE-03.02 | The licence of the released software is in the released source, or in a licence file next to the release assets | Met | The released source (the tag) holds `LICENSE` and `LICENSES/`; each GitHub release also has `THIRD-PARTY-NOTICES.txt` ([`release.yml`](../.github/workflows/release.yml), job `release`). The images hold no licence file: they name the licence in the label `org.opencontainers.image.licenses` ([Dockerfile](../Dockerfile)) |
| OSPS-QA-01.01 | The source repository is public, at a fixed address | Met | `https://github.com/brunopacheco1/portitor` (public, as read) |
| OSPS-QA-01.02 | A public record of every change: what, who and when | Met | The Git history, with signed commits ([CONTRIBUTING, "Pull requests"](../CONTRIBUTING.md#pull-requests), step 6) |
| OSPS-QA-02.01 | A dependency list for the direct language dependencies | Met | [`Cargo.toml`](../Cargo.toml) and [`Cargo.lock`](../Cargo.lock) (also `fuzz/Cargo.lock`) |
| OSPS-QA-04.01 | A project with several repositories lists its codebases | Not applicable | One repository ([AGENTS, "Repository layout"](../AGENTS.md#repository-layout)) |
| OSPS-QA-05.01 | No generated executable in version control | Met | Checked on 2026-11-20: the files Git marks executable are shell and Python scripts. Scorecard's Binary-Artifacts check runs weekly ([`scorecard.yml`](../.github/workflows/scorecard.yml)) |
| OSPS-QA-05.02 | No binary in version control that cannot be reviewed | Met | Checked on 2026-11-20: two binary files, neither executable. A font for the website (`site/fonts/cinzel-regular.woff2`, source and licence in [NOTICE](../NOTICE)) and a test-only RSA key in DER (`crates/portitor-dcp/tests/fixtures/test-only-rsa-2048.der`) |
| OSPS-VM-02.01 | The documentation names security contacts | Met | [SECURITY, "Reporting a vulnerability"](../SECURITY.md#reporting-a-vulnerability) names one contact: a private report through GitHub. Private vulnerability reporting is on for the repository (the API answered `enabled: true` on 2026-11-21; it was off the day before) |

### Settings to confirm

Read through the GitHub API on 2026-11-20, so not in this list: the
ruleset `main`, secret scanning and push protection, the repository's
visibility and its issue tracker. The maintainer checks the rest in
GitHub and writes the date here.

| # | Control | Where in GitHub | What must be true |
|---|---|---|---|
| 1 | OSPS-AC-01.01 | The maintainer's account: Settings, Password and authentication, Two-factor authentication | Two-factor authentication is on |
| 2 | OSPS-AC-02.01 | Repository: Settings, Collaborators | No collaborator, or only people named in GOVERNANCE |
| 3 | OSPS-BR-01.03 | Repository: Settings, Actions, General (approval for fork pull request workflows); Settings, Environments, `release`; Settings, Secrets and variables, Actions | Workflows of outside contributors wait for approval; `release` deploys only from tags `v*`; `SONAR_TOKEN` is the only repository secret |
| 4 | OSPS-BR-03.01 | Repository: Settings, Pages | "Enforce HTTPS" is on for the custom domain |
| 5 | OSPS-VM-02.01 | Repository: Settings, Advanced Security, Private vulnerability reporting | Done on 2026-11-21: the API answers `enabled: true` |
| 6 | OSPS-AC-04.01 (level 2) | Repository: Settings, Actions, General, Workflow permissions | "Read repository contents and packages permissions" is selected |

### What level 2 would need

Level 2 is written for projects with at least two maintainers. Read
from the same source, these level 2 requirements are not met today; the
other level 2 requirements are met, or are a setting in the checklist
above (row 6).

- **OSPS-LE-01.01**, every commit asserts the right to contribute: not
  met. No sign-off is asked for
  ([CONTRIBUTING, "Licensing of contributions"](../CONTRIBUTING.md#licensing-of-contributions)).
  A Developer Certificate of Origin with a sign-off check would meet it.
- **OSPS-BR-06.01**, every release asset is signed or listed with its
  hash in a signed manifest: met for the images only. The binaries, the
  SBOM files and `THIRD-PARTY-NOTICES.txt` attached to a GitHub release
  have no signature of their own. An attestation for these files, or a
  signed list of their SHA-256 hashes, would meet it.
