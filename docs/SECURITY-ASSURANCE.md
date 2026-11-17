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

Status: **0.x, prototype.** This case grows with each release. Items marked
*(planned, 0.N)* are not implemented yet; do not rely on them.

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
- TLS termination inside the services (not on the roadmap yet); run them behind a
  TLS-terminating proxy.
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
| Credentials | A revoked, forged or someone else's credential admits a party | Only trusted issuers' credentials count, signed with an `assertionMethod` key; status lists fetched and checked, fail closed; a presentation with any failing credential is refused (DCP v1.0.1, 5.4.3); the issuer issues only to registered participants (ADR 0018) |
| Gaia-X compliance | A forged compliance credential, or someone else's, admits a party | Only the compliance services the operator lists, separate from the dataspace's issuers; the service's signature; the `digestSRI` link to a legal person credential issued and signed by the caller; the holder binding is skipped for these two credentials only (ADR 0022); the identity takes an answer only from `PORTITOR_GAIAX_COMPLIANCE_DID` (ADR 0021) |
| Private keys | Keys leak from the internet-facing connector | Keys live only in `portitor-identity` and `portitor-issuer`, not in the connector (ADR 0003) |
| Schema and vocabulary bundle | A forged or changed bundle makes connectors accept bad datasets or refuse good ones | A separate bundle key, kept offline: the issuer holds only its public part and refuses to start if it is the credential key; signature checked against the authority's DID document; the version (never older than the one in use) and authority; SHA-256 of every file; size limits (manifest 1 MiB, 64 MiB per file, 256 MiB in all) and a 120 s timeout per download; a published version is never written again |
| Validation code | Crafted shapes or JSON-LD make the connector fetch remote content, or exploit an XML parser | JSON-LD contexts come only from a map the caller fills (the bundle's and the DSP contexts), never from the network; no Portitor binary parses RDF/XML (`bundle build` refuses it, code lists travel as N-Triples); fuzz targets `shacl_load`, `shacl_jsonld` and `bundle_manifest` |
| Internal services | A consumer gives a `callbackAddress` that points at an internal service, so the connector calls it (SSRF) | Only `https://` URLs with a host, unless the operator allows `http://`; the response body is never read; a 10 s timeout; blocking internal addresses *(planned)* |
| Negotiation state | A consumer drives a negotiation out of order, or two messages race | The DSP state machine as a pure function in the domain; every change is a compare-and-update on a version number; a message that races our acknowledgement is accepted only as proof of delivery of our saved pending message (ADR 0007) |
| Service availability | Crafted or oversized messages crash or exhaust a service | Typed parsing with no `unsafe`; a request size limit per port (`PORTITOR_MAX_REQUEST_BYTES`, `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES`, 2 MiB by default) and a fixed 64 KiB limit on routes whose messages are small by specification, HTTP 413 above them ([R-18](RISKS_AND_TECHNICAL_DEBTS.md#r-18)); fuzzing of the DSP, DCP and ODRL parsers, the SHACL shapes and JSON-LD readers, the bundle manifest and the Gaia-X compliance checks and JCS digest (ClusterFuzzLite), of every parser *(planned, 0.11)*; a nesting limit in SHACL validation |
| Operational data (health answers, traces, metrics) | A caller reads health answers, or a receiver reads traces and metrics, and learns secrets, partners or process ids; a partner links our internal calls through trace context | Health checks only on the management port, which stays on the operator's network; failure messages without secrets; export off by default and only to the receiver the operator names; attribute allow-list (no tokens, keys, headers, bodies or id-bearing paths); client metrics name only hosts inside the participant (no untrusted cardinality); `traceparent` neither sent to nor taken from other participants, and taken on `/sts/token` only after the client is authenticated (ADR 0023) |
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
- **Static analysis:** Clippy pedantic with warnings as errors; CodeQL and
  SonarQube Cloud quality gate on every pull request.
- **Supply chain:** pinned workflow actions, least-privilege workflow
  tokens, REUSE compliance; OpenSSF Scorecard; cargo-vet, so a new or
  bumped crate needs a recorded audit or exemption before it merges.

## OpenSSF Baseline

Target: **level 1** (Baseline version 2026-08-28) for all of 0.x ([ADR 0012](adr/0012-openssf-baseline.md)). The
self-assessment, control by control, is added here in the 0.13 release
candidate, so it assesses the final state of 0.x.
