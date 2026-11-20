<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Deploying and operating Portitor

A runbook for the team that deploys and runs Portitor. It covers
`portitor-connector` and, from 0.5, `portitor-identity`, which runs next to
it ([section 4](#4-run-the-identity-next-to-the-connector)). From 0.6 it
also covers `portitor-issuer`, which only the dataspace authority runs
([section 7](#7-run-the-issuer-dataspace-authority-only)).

**Deployment order.** Once per dataspace, the authority deploys the
issuer and registers each participant. Then each participant deploys its
identity, asks the issuer for its membership credential, and deploys its
connector ([ARCHITECTURE.md](ARCHITECTURE.md#deployment-order)).

**What the dataspace decides before participants join.** DSP 2025-1
says a dataspace is based on an agreed Governance Framework and an
agreed set of Policies. DCP v1.0.1 leaves some points to that framework
or to a dataspace profile, on purpose (DCP issues
[#277](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/277)
and [#278](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278)).
The dataspace authority decides them once and tells every participant.
Every participant then sets the same values:

| Left open by DCP | What the authority decides | Portitor setting |
| --- | --- | --- |
| Trusted issuers ("trust anchors" in DCP) | Which issuer DIDs are trusted for which credential types | `PORTITOR_TRUSTED_ISSUERS` (connector and identity) |
| Scope strings | The exact scopes participants send and accept. EDC needs an access part, for example `org.eclipse.dspace.dcp.vc.type:MembershipCredential:read` | `PORTITOR_DCP_SCOPES`, `PORTITOR_POLICY_CREDENTIALS`, `PORTITOR_SCOPE_TYPE_ALIASES` |
| Holder binding | Which credential types must have the presenter as subject, and how other credentials are bound | Fixed in Portitor: the subject must be the presenter, except the Gaia-X compliance credential ([Gaia-X compliance](#gaia-x-compliance), ADR 0022) |
| Gaia-X compliance (optional) | Whether offers may require it, and which compliance services are trusted. Gaia-X lists its Trust Anchors in the Gaia-X Registry | `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` (provider) |
| Dataset metadata shapes (optional) | Whether dataset metadata is checked, and against which bundle | `PORTITOR_BUNDLE_URL`, `PORTITOR_BUNDLE_VERSION`, `PORTITOR_BUNDLE_AUTHORITY` ([bundle](#schema-and-vocabulary-bundle)) |

Portitor is a prototype (0.x). Read [Known limits](#known-limits) before you
expose it to partners.

## 1. Before the first deployment

Check each item and keep the answers in your own runbook.

- [ ] **Version.** Pick a release tag (`X.Y.Z`) from the
  [releases](https://github.com/brunopacheco1/portitor/releases). Never deploy
  `latest` or a branch build.
- [ ] **Verify the images** you deploy, `portitor-connector` and
  `portitor-identity`, both of the same version (see
  [RELEASING.md](RELEASING.md#checking-a-release)):
  ```sh
  for image in portitor-connector portitor-identity; do
    cosign verify "ghcr.io/brunopacheco1/$image:X.Y.Z" \
      --certificate-identity-regexp '^https://github.com/brunopacheco1/portitor/\.github/workflows/release\.yml@refs/tags/v' \
      --certificate-oidc-issuer https://token.actions.githubusercontent.com
    gh attestation verify "oci://ghcr.io/brunopacheco1/$image:X.Y.Z" --repo brunopacheco1/portitor
  done
  ```
  Deploy by digest (`ghcr.io/...@sha256:...`), not only by tag. The
  dataspace authority checks `portitor-issuer` the same way.
- [ ] **Participant id.** The DID your dataspace gave you
  (`PORTITOR_PARTICIPANT_ID`).
- [ ] **Public URL.** The `https://` URL partners use to reach you
  (`PORTITOR_PUBLIC_URL`). TLS ends at your reverse proxy or load balancer;
  Portitor itself serves plain HTTP.
- [ ] **Datasets file.** A JSON file with the datasets you offer (see the
  [README](../README.md#interface)). From 0.7 the connector harvests it
  into a read copy at start and every hour ([Dataset harvest](#dataset-harvest),
  ADR 0020). A dataset that breaks a DSP rule, uses a constraint the
  connector does not understand (`dateTime`, ADR 0009, and credential
  claims of the types in `PORTITOR_POLICY_CREDENTIALS`, ADR 0019; see
  [Credential constraints](#credential-constraints)) or, with a bundle,
  fails the authority's shapes is rejected and left out of the catalog;
  the connector still starts. Test the file before a rollout and read the
  harvest job (`GET /v1/harvest-jobs`) after it.
- [ ] **Schema and vocabulary bundle** (optional, from 0.7). If your
  dataspace checks dataset metadata, its authority gives you the
  bundle's URL, the version to pin and its DID
  ([Schema and vocabulary bundle](#schema-and-vocabulary-bundle), ADR 0005).
- [ ] **Data sources.** For each dataset you transfer, the URL the
  connector fetches its data from (`portitor:source` in the datasets file,
  see the README). Partners never see it. It must be reachable from the
  connector, should be internal, and must not need credentials (none are
  sent). Plain `http://` is accepted here, since you set it; prefer
  `https://`. The harvest logs a warning for each dataset it adds or
  changes without a source.
- [ ] **Clock.** Offer constraints such as a date limit are checked
  against the host clock (ADR 0009). Keep it synchronised (NTP); a
  container uses the host's clock.
- [ ] **Management API.** Needed only if this connector reads catalogs
  or starts negotiations as consumer. Create a key (`openssl rand -hex 32`), keep it
  in your secret store, and mount it as a file
  (`PORTITOR_MANAGEMENT_API_KEY_FILE`). Decide who may reach the management
  port; never route it through the public proxy.
- [ ] **Storage.** A persistent volume for `/data` (negotiations, SQLite).
- [ ] **Network.** Inbound: partners reach the DSP endpoints, including
  the consumer callbacks under `/dsp/2025-1/callback`, through your proxy.
  Inbound from your probes and monitoring only: the management port
  (health checks, [Health and logs](#health-and-logs)), never through
  the proxy. Outbound: the connector calls the callback addresses
  partners give it and, as consumer, the providers' DSP endpoints; with
  telemetry on, also your OTLP receiver
  ([Traces and metrics](#traces-and-metrics)). Allow only what it needs
  (see [Security](#5-security)).
- [ ] **Trust.** Partners' TLS certificates must chain to a CA in the
  image's bundle, or to a private CA you add (see [Private CAs](#private-cas)).
- [ ] **Identity (DCP).** Every DSP request is authenticated (ADR 0016),
  in both directions, so the connector needs its own identity component
  ([section 4](#4-run-the-identity-next-to-the-connector)) with a `MembershipCredential` from
  your dataspace's issuer in it. The connector checks at start that its
  `PORTITOR_PARTICIPANT_ID` is the identity's DID and that the STS signs
  for it, and refuses to start otherwise. Outbound it also needs: its STS,
  partners' DID documents and credential services, and the issuers'
  status list URLs.

## 2. Configuration

All configuration is by environment variable. The full table is in the
[README](../README.md#interface). In all three services, spaces around a value are dropped and an empty
value means "not set": a required setting that is empty stops the start,
and an optional one takes its default.

| Variable | Production value |
|---|---|
| `PORTITOR_PARTICIPANT_ID` | **required**: your DID |
| `PORTITOR_PUBLIC_URL` | **required**: your public `https://` URL |
| `PORTITOR_DATASETS_FILE` | path of the mounted datasets file, harvested at start and at each run |
| `PORTITOR_HARVEST_INTERVAL_SECONDS` | leave the default, `3600` (one hour); `0` harvests only at start. A value that is not a whole number, or is above `31536000` (one year), stops the start |
| `PORTITOR_BUNDLE_URL` | with a bundle: the `https://` URL of the authority's bundles folder (one folder per version, and `latest.json`), for example `https://issuer.example.com/bundles/`; plain `http://` only with `PORTITOR_ALLOW_HTTP=true` |
| `PORTITOR_BUNDLE_DIR` | unset; a local bundles folder (one folder per version, and `latest.json`), for development and tests only. Not together with `PORTITOR_BUNDLE_URL` |
| `PORTITOR_BUNDLE_VERSION` | `latest`: follow the newest version the authority names in `latest.json`; or one fixed version, never changed while the connector runs (the manifest must have exactly this version) |
| `PORTITOR_BUNDLE_REFRESH_SECONDS` | `3600`: how often the newest version is looked for; `0` only at start. A value that is not a whole number, or is above `31536000` (one year), stops the start. Not used with a fixed version |
| `PORTITOR_BUNDLE_AUTHORITY` | with a bundle, **required**: the authority's `did:web` DID; the manifest must name it, and its bundle key `<DID>#bundle-key`, listed under `assertionMethod` in its DID document, must have signed it with ES256; a bundle signed with any other key of the authority (such as its credential key) is refused |
| `PORTITOR_BUNDLE_CACHE_DIR` | leave the default, `bundle-cache` next to `PORTITOR_DB_FILE` (`/data/bundle-cache` in the image) |
| `PORTITOR_DB_FILE` | leave the image default, `/data/portitor.db` |
| `PORTITOR_BIND` | leave the default, `0.0.0.0:8080` |
| `PORTITOR_MANAGEMENT_API_KEY_FILE` | path of the mounted key file, or unset (management API off; the management port then serves only the [health checks](#health-and-logs)). The value is trimmed; an empty value counts as unset |
| `PORTITOR_MANAGEMENT_BIND` | `0.0.0.0:8081` in a container (publish it only on an internal interface); the default `127.0.0.1:8081` is unreachable from outside a container. The port is always bound: it serves the [health checks](#health-and-logs), with or without the key |
| `PORTITOR_MAX_REQUEST_BYTES` | leave the default, `2097152` (2 MiB): the largest request body the public port takes; a larger one gets HTTP 413. Some routes have a lower fixed limit ([section 5](#5-security)). A value that is not a whole number from `1` to `67108864` (64 MiB) stops the start |
| `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` | leave the default, `2097152` (2 MiB): the same for the management port |
| `PORTITOR_ALLOW_HTTP` | **unset** (never `true` in production) |
| `PORTITOR_DATA_TOKEN_SECONDS` | leave the default, `3600`, unless consumers need longer to fetch; at most `31536000` (one year) |
| `PORTITOR_STS_URL` | **required**: your identity's STS token endpoint, `<identity base URL>/sts/token` |
| `PORTITOR_STS_ALLOW_HTTP` | unset; `true` only when `PORTITOR_STS_URL` is plain `http://` on an internal network ([section 4](#endpoints)) |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | **required**: path of the mounted STS secret, the same file as the identity's |
| `PORTITOR_STS_CLIENT_ID` | leave the default, `PORTITOR_PARTICIPANT_ID` |
| `PORTITOR_TRUSTED_ISSUERS` | **required**: comma-separated DIDs trusted to issue credentials (`MembershipCredential` and those of credential constraints) |
| `PORTITOR_DCP_SCOPES` | leave the default, `org.eclipse.dspace.dcp.vc.type:MembershipCredential` (comma-separated). Each must be a DCP scope (`org.eclipse.dspace.dcp.vc.type:<Type>[:access]` or `org.eclipse.dspace.dcp.vc.id:<id>`); the type `VerifiableCredential` is refused, as it would let a partner read every credential. The connector does not start otherwise |
| `PORTITOR_POLICY_CREDENTIALS` | unset, unless your offers constrain credential claims: the credential types they use, comma-separated, each `Type` or `Type=<scope>` (see [Credential constraints](#credential-constraints)) |
| `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` | unset, unless your offers require Gaia-X compliance: comma-separated `did:web` DIDs of the compliance services you trust (see [Gaia-X compliance](#gaia-x-compliance)) |
| `PORTITOR_STATUS_LIST_CACHE_SECONDS` | leave the default, `300` (5 minutes): how long a status list credential is reused; `0` fetches it on every check. At most `3600` (one hour); a larger value, or one that is not a whole number, stops the start. See [Status list cache](#status-list-cache) |
| `PORTITOR_DID_CACHE_SECONDS` | leave the default, `300` (5 minutes): how long a resolved DID document is reused; `0` fetches it on every use. At most `3600` (one hour); a larger value, or one that is not a whole number, stops the start. See [DID document cache](#did-document-cache) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset (no traces or metrics), or your OTLP receiver's base URL; see [Traces and metrics](#traces-and-metrics) for this and the other `OTEL_*` variables |
| `RUST_LOG` | `info` (default); `debug` only while diagnosing |

### DID document cache

Every component resolves other parties' `did:web` DIDs: the connector
(each caller's, each credential issuer's, and the authority's for the
bundle key), the identity (callers' and issuers') and the issuer
(holders'). Each reuses a DID document it fetched for
`PORTITOR_DID_CACHE_SECONDS` (default 300 seconds), in memory. Set the
same value on the connector, the identity and the issuer.

- **The trade-off.** A key that its owner removed from the document,
  for example after a leak, is still accepted until the cached document
  is fetched again; a new key is refused for as long. With `0` every
  use fetches the document: a change is seen at once, at the cost of
  one fetch more per caller and per credential issuer on every DSP
  request, and no answer at all while a partner's DID host is down.
- At most `3600` (one hour). A larger value, or one that is not a whole
  number, stops the start.
- At most 1,000 documents are kept. A lookup that failed is never
  kept. A restart empties the cache.
- A partner that must drop a leaked key at once should tell you; a
  restart of your components, or a low value here, shortens the wait.

### Status list cache

The connector checks each partner's credentials against the issuer's
status list on every DSP request. It reuses a status list credential it
fetched for `PORTITOR_STATUS_LIST_CACHE_SECONDS` (default 300 seconds),
by the list's URL, in memory.

- **The trade-off.** A revocation or suspension takes up to that long to
  be seen: a partner whose credential was just revoked is still admitted
  until the cached list is fetched again. With `0` it is seen on the
  next request, at the cost of one fetch per credential status on every
  request. Eclipse EDC 1.0.0-RC7 reuses status lists for 15 minutes by
  default (`edc.iam.credential.revocation.cache.validity`).
- Only a list that was fetched and verified (signed by the credential's
  issuer, valid now) is reused. A failed fetch or check is not: it is
  tried again on the next request.
- A list is never reused past its own end (`validUntil`, or the JWT's
  `exp`), even if that comes before the configured time.
- The cache is per process: a restart empties it. Presentations and
  tokens are not cached; they are checked on every request (ADR 0016).
- The Compose demo, `scripts/demo.sh`, the EDC interop test and the DCP
  TCK run set `0`, so their revocation steps see a revocation at once.

### Credential constraints

An offer may admit only consumers with a credential claim (ADR 0019),
for example only data processors in the EU:

```json
"permission": [{"action": "use", "constraint": [
  {"leftOperand": "DataProcessorCredential.processing", "operator": "eq", "rightOperand": "eu"}
]}]
```

- List each credential type your offers use in
  `PORTITOR_POLICY_CREDENTIALS`, on the provider **and** on consumers:
  `DataProcessorCredential` stands for the scope
  `org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read`; write
  `DataProcessorCredential=<scope>` for another one. The scope must ask
  for that type. `MembershipCredential` is always there, with its scope
  from `PORTITOR_DCP_SCOPES`.
- The provider derives the scopes from the offer and asks the consumer's
  credential service for them at the contract request, and again at each
  transfer request, so a revoked credential stops new transfers. The
  consumer's token must allow them: Portitor as consumer adds them itself.
- Credentials must come from `PORTITOR_TRUSTED_ISSUERS`, as the
  membership does. Operators: `eq`, `neq` (a text), `isAnyOf`, `isNoneOf`
  (a list of texts); a claim that is missing or not text never grants
  access.
- An EDC participant needs the same mapping, per type, for example:

  ```yaml
  edc.iam.dcp.scopes.processor.id: "processor-scope"
  edc.iam.dcp.scopes.processor.type: "POLICY"
  edc.iam.dcp.scopes.processor.prefix.mapping: "DataProcessorCredential"
  edc.iam.dcp.scopes.processor.value: "org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read"
  ```

  (the key names are those of EDC v1.0.0-RC8,
  `DynamicDcpScopeConfigurationExtension`; the EDC interop run sets them
  in `interop/edc/config/edc-controlplane.properties`. The Minimum Viable
  Dataspace, commit 17bb5b8, and EDC's own decision record write
  `prefix-mapping`: RC8 reads `prefix.mapping`).

### Gaia-X compliance

An offer may admit only participants that a Gaia-X compliance service
has checked (ADR 0022). The service issues a
`gx:ParticipantComplianceCredential`. Its claim `gx:labelLevel` holds
the label, for example `SC`. A policy reads it like any credential
claim:

```json
"permission": [{"action": "use", "constraint": [
  {"leftOperand": "ParticipantComplianceCredential.gx:labelLevel", "operator": "eq", "rightOperand": "SC"}
]}]
```

Set `PORTITOR_POLICY_CREDENTIALS` on the provider **and** on consumers,
and `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` on the provider:

```sh
PORTITOR_POLICY_CREDENTIALS=ParticipantComplianceCredential
PORTITOR_GAIAX_COMPLIANCE_ISSUERS=did:web:compliance.lab.gaia-x.eu:v2
```

- `ParticipantComplianceCredential` stands for the scope
  `org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential:read`.
  Both sides add the `LegalPerson` scope with the same alias and access
  part (here `org.eclipse.dspace.dcp.vc.type:LegalPerson:read`). The
  consumer's identity then presents both credentials.
- `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` lists the compliance services you
  trust. It is separate from `PORTITOR_TRUSTED_ISSUERS`: a compliance
  service is not a dataspace issuer. A consumer does not need it. A
  provider without it admits no one to these offers; the start log
  warns.
- What the provider checks: the compliance credential comes from a
  listed service and is signed with its `assertionMethod` key (PS256 or
  ES256). It is valid now by `validFrom` and `validUntil`, and it must
  have a `validUntil` (the service writes its JWT times in milliseconds,
  so those are ignored). The same
  presentation holds a `gx:LegalPerson` credential whose digest
  (`digestSRI`) the compliance credential lists. That credential is
  issued and signed by the consumer. Its `credentialStatus`, if any, is
  not fetched: the consumer controls that list.
- A compliance credential from a service you do not list is skipped, like
  a credential from an issuer you do not trust: the consumer can still
  use offers that need only other credentials (Gaia-X has several
  clearing houses).
- A `gx:LegalPerson` issued by one of your `PORTITOR_TRUSTED_ISSUERS` is
  an ordinary dataspace credential and checked as before 0.8. Policies
  see Gaia-X credentials only by their Gaia-X type.
- These two credentials are about a credential, not about the
  consumer's DID, so the subject check is skipped for them only. Every
  other credential must still be about the consumer.
- A compliance credential from a listed service that fails a check makes
  its presentation invalid: the consumer is refused, and the log says why (`credential
  refused`). A `gx:LegalPerson` credential that no trusted compliance
  credential names is ignored.
- Only Portitor connectors meet these policies today. EDC applies the
  subject check to every credential, so it refuses the compliance
  credential (ADR 0022).

### Dataset harvest

From 0.7 the datasets come from a source outside the connector into a
read copy in its database (ADR 0020). Today the source is the datasets
file (`PORTITOR_DATASETS_FILE`), in the same format as before.

- A run happens at start, before the DSP port serves, so the catalog is
  ready at once; then every `PORTITOR_HARVEST_INTERVAL_SECONDS`. One run
  at a time: a timer tick that finds a run going on is skipped.
- The file is read again at each run. When its SHA-256 is the last run's,
  the run ends `unchanged` and nothing else is touched. To publish a
  change, replace the file; it is taken at the next run (or restart).
- Each dataset is checked on its own: JSON object, DSP `Dataset` form,
  the DSP catalog rules, offers with only the policy terms and ODRL
  constraints the connector reads ([Known limits](#known-limits)), a valid
  `portitor:source` and, with a bundle, the SHACL shapes for its
  `dct:accessRights` level. A dataset that passes is added or updated. A
  dataset that fails is rejected with its error; its previous good
  version, if any, stays in the catalog.
- Two datasets with the same `@id`: the first is used, the others are
  rejected.
- A dataset that is no longer in the file is **withdrawn**: it leaves the
  catalog and new contract requests for it are refused. Transfers under
  agreements already made keep working. If it comes back and passes, it
  is active again.
- A file that cannot be read, or is not a JSON array, makes the run
  `failed`: the copy stays as it was. On a first start with no copy, the
  catalog is empty and the connector still starts.
- When the rules change (a new bundle version, another public URL,
  another `PORTITOR_POLICY_CREDENTIALS`, or a new Portitor version, whose
  checks may differ), the stored datasets are checked
  again at start (trigger `bundle`). One that now fails is **hidden**:
  out of the catalog, kept for existing agreements, with the reason
  stored.
- All changes of a run are written in one transaction. The last 100 runs
  are kept.
- Limits: a datasets file larger than 256 MiB makes the run `failed`. A
  run stores the errors of its first 1000 rejected entries (the others are
  counted and logged), and a stored SHACL report keeps its first 50
  results (`resultsNotStored` says how many more).

Each run is a job on the management API (read only, API key):

```sh
curl -s localhost:8081/v1/harvest-jobs -H "authorization: Bearer $(cat key)"       # newest first
curl -s localhost:8081/v1/harvest-jobs/42 -H "authorization: Bearer $(cat key)"    # one job, with its errors
```

A job has `trigger` (`start`, `interval`, `bundle`), `outcome`
(`succeeded`, `partial`, `unchanged`, `failed`), `counts` (`seen`,
`added`, `updated`, `unchanged`, `withdrawn`, `rejected`) and a one-line
`summary`. Each error names its `position` in the file, the `datasetId`
if readable, the `check` that failed (`json`, `dsp`, `odrl`, `shacl`), the
`message` and, for SHACL, the `report`. Pages: `?limit=1..100`, then
`?after=<next>`. Described in
[api/management-openapi.json](api/management-openapi.json).

To keep a strict start, as before 0.7, make your start-up probe check
that the latest job's `outcome` is `succeeded`.

### Schema and vocabulary bundle

From 0.7 a dataspace authority can publish a signed bundle of SHACL
shapes, code lists and JSON-LD contexts (ADR 0005); HealthDCAT-AP
release 8 is the baseline. With a bundle the connector checks each
harvested dataset against the shapes for its `dct:accessRights` value
and, if the bundle has message shapes, DSP negotiation and transfer
messages against them. A dataset that passes is served with
`"dct:conformsTo": {"@id": "<the bundle's profile>", "@type": "dct:Standard"}`.

- Set `PORTITOR_BUNDLE_URL` and `PORTITOR_BUNDLE_AUTHORITY` from what
  the authority gives you. Without them nothing is checked against
  shapes. By default the connector follows the newest version
  (`latest.json`); set `PORTITOR_BUNDLE_VERSION` to keep one version.
- For a version, the connector downloads `manifest.json` and `manifest.jws`
  first, checks the signature (ES256) with the bundle key listed in the
  authority's `did:web` document (`<did>#bundle-key`, under
  `assertionMethod`), the version and the authority, then downloads each
  file and checks its SHA-256. Limits: 1 MiB for the manifest, 64 MiB per
  file, 256 MiB in all, 120 s per download.
- The verified bundle is kept in `PORTITOR_BUNDLE_CACHE_DIR`. The cache
  is used only when the authority's host or DID document cannot be
  reached; its files and signature are then checked again, with a
  warning. Keep the cache on the `/data` volume.
- **A file with a role this version does not know is skipped** (from
  0.13; ADR 0005). A later version of the format may add roles, for
  example policy shapes. The connector does not download such a file
  and does not use it; the other files are used as before. You see it
  in three places: one WARN line for each file,
  `bundle file skipped: ...`, with `path` and `role_type`; the gauge
  `portitor.bundle.file.skipped`; and the bundle run in
  `GET /v1/harvest-jobs`, whose summary ends with
  `N files skipped (unknown role, ...)`. **The limit: a connector that
  skips a new kind of rule does not enforce it.** Upgrade the connector
  to enforce it. A bundle in which the connector knows no role at all
  is refused like any invalid bundle (the version in use stays; with
  no version, the start stops). Everything else in a manifest is still
  read strictly: a field the format does not have is an error. Such a
  file may have any media type (`type/subtype`, RFC 6838); a file with
  a role the connector knows must be `text/turtle`,
  `application/n-triples` or `application/ld+json`.
  **After an upgrade to a version that knows the role**, the connector
  needs the authority's server once: the cached copy of the bundle does
  not hold the file that was skipped, so it cannot be used offline (the
  start stops if the server cannot be reached and there is no other
  usable copy). Upgrade while the authority's bundle address answers.
  A role type that is a known one in other letter case (`Messages` for
  `messages`) is taken as a mistake and the bundle is refused.
- A bundle that was read but fails a check (signature, key, authority,
  version or a digest) stops the start; the cache does not replace it.
  So does a bundle that cannot be loaded at all (no download and no
  usable cache), and one whose shapes, vocabulary or contexts the
  connector cannot use (for example a SHACL term the engine does not
  implement): the log line `portitor-connector stopped` says why. Start
  the connector after the authority's issuer, or let it restart.
- New versions are pulled, never pushed: at start and every
  `PORTITOR_BUNDLE_REFRESH_SECONDS` the connector reads `latest.json`.
  A newer version is checked as above and put in use without a restart;
  then the stored datasets are checked again (those that fail are
  hidden) and the datasets source is harvested again. Each check is a
  run in `GET /v1/harvest-jobs`, with `"kind": "bundle"`.
- The connector never goes back to a version issued before the one in
  use, which it remembers in the cache. A version that cannot be taken
  (unreachable, refused, older) leaves the version in use and the run
  is `failed` with the reason. At start, with `latest`, the version in
  use before is read back from the cache if the newest cannot be taken.
  The order comes from the manifest's `issued` (`dcterms:issued` in
  W3C DCAT 3), a UTC date and time to the second; `bundle build` writes
  the time of the build unless you give `--issued`.
- The catalog's `@context` is always the DSP 2025-1 context alone, with
  or without a bundle. The connector never fetches a JSON-LD context from
  the network: it has its own copy of the DSP 2025-1 contexts, and a
  bundle's `context` files serve only to read DSP messages whose
  `@context` names their URL (for the message check). A message that
  names any other context URL is refused when the bundle has message
  shapes.
- Datasets are checked as the catalog serves them: read with the DSP
  context only. Each dataset needs an absolute IRI as `@id` (JSON-LD
  drops a relative one, and with it every statement about the dataset)
  and a `dct:accessRights` IRI the bundle has shapes for; otherwise it is
  rejected. Without a bundle, neither is needed.
- Message rules: an inbound negotiation or transfer message that breaks
  them is answered 400 with the DSP error, and nothing changes; an
  outbound one is not sent and the negotiation or transfer ends (logged
  as `message breaks the dataspace's message rules; not sent, ...`).
  Catalog requests are not checked.
- What the engine checks: the SHACL Core subset of `portitor-shacl`.
  `sh:extends` and `sh:shape` are ignored with a warning, so DCAT-AP's
  "date or date and time" check on `dct:issued` and `dct:modified` is not
  done. Class targets apply only to nodes typed in the dataset itself.

Write HealthDCAT-AP datasets as DSP 2025-1 `Dataset` objects, as in
[examples/healthdcat-datasets.json](../examples/healthdcat-datasets.json).
Use the `dct:`, `dcat:`, `odrl:`, `xsd:` and `dspace:` prefixes of the
DSP context, and write every other term as a full IRI, for example
`"http://healthdataportal.eu/ns/health#healthCategory"` or
`"@type": "http://xmlns.com/foaf/0.1/Agent"`. Other prefixes are not
supported: the DSP 2025-1 JSON Schema allows only URL strings in
`@context`, and EDC 1.0.0-RC7 does not fetch remote contexts by default
(`edc.jsonld.http.enabled` and `edc.jsonld.https.enabled` are `false`),
so a consumer could not read a prefix the catalog does not define. A
prefixed name the DSP context does not know is read as an IRI of its own
(`healthdcatap:hdab` is not `http://healthdataportal.eu/ns/health#hdab`),
so the shapes do not see it. DSP 2025-1 and DCAT-AP do not agree on everything:

- The DSP context makes `dcat:endpointURL` a text, while DCAT-AP needs an
  IRI.
- `"format": "HttpData-PULL"` is not an IRI, so JSON-LD drops it when the
  dataset is turned into RDF; DCAT-AP shapes do not see it.
- DSP's examples have no `dcat:accessURL` on a `Distribution` and no
  `dct:title` on a `DataService`; DCAT-AP requires both. Give each
  distribution a `dcat:accessURL`.

### Private CAs

To trust a private CA (for example on a private network) or a
self-signed certificate in a test environment, mount the PEM file and set
`SSL_CERT_DIR=/etc/ssl/certs:/ca`. Setting `SSL_CERT_DIR` replaces the
default bundle, so keep `/etc/ssl/certs` in the list. Certificate checks
cannot be turned off.

## 3. Run

Docker:

```sh
docker run -d --name portitor-connector --restart unless-stopped \
  -p 8080:8080 \
  -v /srv/portitor/datasets.json:/datasets.json:ro \
  -v portitor-data:/data \
  -e PORTITOR_PARTICIPANT_ID=did:web:connector.example.com \
  -e PORTITOR_PUBLIC_URL=https://connector.example.com \
  -e PORTITOR_DATASETS_FILE=/datasets.json \
  -v /srv/portitor/sts-client-secret:/run/secrets/sts-client-secret:ro \
  -e PORTITOR_STS_URL=http://identity:8090/sts/token -e PORTITOR_STS_ALLOW_HTTP=true \
  -e PORTITOR_STS_CLIENT_SECRET_FILE=/run/secrets/sts-client-secret \
  -e PORTITOR_TRUSTED_ISSUERS=did:web:issuer.example.com \
  ghcr.io/brunopacheco1/portitor-connector@sha256:<digest>
```

The STS lines point at the identity component of
[section 4](#4-run-the-identity-next-to-the-connector), reached on an
internal network as `identity`; without them the connector does not start.

With the management API, add (the key file must be readable by user
`65532`; the port is published on the host's loopback only):

```sh
  -p 127.0.0.1:8081:8081 \
  -v /srv/portitor/management-key:/run/secrets/portitor-key:ro \
  -e PORTITOR_MANAGEMENT_API_KEY_FILE=/run/secrets/portitor-key \
  -e PORTITOR_MANAGEMENT_BIND=0.0.0.0:8081 \
```

Rules:

- **One container per database file.** Never run two replicas on the same
  `/data` volume. Several replicas need the PostgreSQL adapter (planned,
  ADR 0006).
- The image runs as user `65532`, has no shell and holds only the binary,
  the CA bundle and `/data`. Only `/data` needs to be writable.
- `docker stop` (SIGTERM) stops it cleanly; it stops taking requests and
  exits.
- Kubernetes: one replica (`Deployment` with `strategy: Recreate`, or a
  `StatefulSet`), a `PersistentVolumeClaim` on `/data`, and the
  [health checks](#health-and-logs) as probes (set
  `PORTITOR_MANAGEMENT_BIND=0.0.0.0:8081` so the kubelet can reach them).
  Mount the management key from a `Secret`, and do not put the management
  port in an `Ingress`. There is no Helm chart yet.

Check after start:

```sh
curl -sf https://connector.example.com/health          # "ok"
curl -s  https://connector.example.com/.well-known/dspace-version
```

The log line `portitor-connector listening` shows the bind address, the number
of datasets in the catalog after the start harvest, and the database file.
`catalog ready` comes just before it.
`management port listening` shows the management port's address; it is
always bound, for the health checks. `management API off` means no key
file is set (or the variable is empty): the port then serves only the
health checks. A missing, unreadable or too short key file stops the
start with an error. With telemetry on, `telemetry: exporting` comes
first ([Traces and metrics](#traces-and-metrics)).

The binary takes no arguments except `health`
([Health and logs](#health-and-logs)); any other argument stops it with
exit code 2.

## 4. Run the identity next to the connector

The identity component holds the participant's key and credentials and
answers for it in the Decentralized Claims Protocol v1.0.1 (ADR 0015):
its DID document, a Secure Token Service (STS) for the own connector, and
the credential service that other participants query. One process per
participant.

### Before the first deployment

- [ ] **DID.** A `did:web` DID whose document this component serves, for
  example `did:web:identity.example.com`. did:web maps it to
  `https://identity.example.com/.well-known/did.json` (or
  `/<path>/did.json` for a DID with a path), so your proxy must route that
  path to the component. The connector's `PORTITOR_PARTICIPANT_ID` is the
  same DID.
- [ ] **Key.** One P-256 private key, unencrypted PKCS#8 PEM, never
  leaving this component:
  `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out identity-key.pem`.
  Mount it read-only. A SEC1 key (`BEGIN EC PRIVATE KEY`) is refused; convert it
  with `openssl pkcs8 -topk8 -nocrypt`. The key id in the DID document is
  the key's JWK thumbprint (RFC 7638), shown in the start log.
- [ ] **STS client secret.** `openssl rand -hex 32`, mounted as a file into
  both the identity component and the connector
  (`PORTITOR_STS_CLIENT_SECRET_FILE` in both).
- [ ] **Trusted issuers.** The DIDs whose credentials you accept
  (`PORTITOR_TRUSTED_ISSUERS`). Only these may deliver credentials
  (Storage API) or offers.
- [ ] **Management API key.** As for the connector, if you ask issuers
  for credentials or manage credentials.
- [ ] **Storage.** A persistent volume for `/data` (credentials, SQLite).
- [ ] **Gaia-X** (optional, from 0.8): a certificate for the identity
  key, the notary's registration number credential and a participant
  file ([Gaia-X onboarding](#gaia-x-onboarding-optional)).
- [ ] **Network.** Inbound from other participants, through your proxy:
  the DID document and the credential service. Inbound from your own
  connector only: the STS (`/sts/token`). Outbound: other participants'
  and trusted issuers' DID documents (`https://`), and the issuers'
  `IssuerService` (metadata, credential requests, request status).

### Configuration

| Variable | Default | Production value |
|---|---|---|
| `PORTITOR_PARTICIPANT_ID` | required | your `did:web` DID |
| `PORTITOR_IDENTITY_KEY_FILE` | required | path of the mounted PEM key |
| `PORTITOR_IDENTITY_RETIRED_KEY_FILES` | unset (no retired key) | only during a key change: comma-separated paths of the old **public** keys (`-----BEGIN PUBLIC KEY-----`), listed in the DID document so that what they signed keeps verifying; they never sign. The start fails for a private key, a key that is not P-256, the current key, or a key given twice ([Rotating a key](#rotating-a-key)) |
| `PORTITOR_CREDENTIAL_SERVICE_URL` | required | public `https://` base URL of the credential service, written into the DID document; its path is where the component serves it (for example `https://identity.example.com/cs` serves `/cs/presentations/query`, `/cs/credentials`, `/cs/offers`) |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | required | path of the mounted STS secret (at least 32 characters) |
| `PORTITOR_STS_CLIENT_ID` | the participant DID | leave the default, or the client id your connector sends |
| `PORTITOR_TRUSTED_ISSUERS` | required | comma-separated issuer DIDs |
| `PORTITOR_BIND` | `0.0.0.0:8090` | leave the default |
| `PORTITOR_DB_FILE` | `/data/portitor-identity.db` | leave the default |
| `PORTITOR_MANAGEMENT_API_KEY_FILE` | unset (management API off; the port serves only the health checks; an empty value counts as unset) | path of the mounted key file |
| `PORTITOR_MANAGEMENT_BIND` | `127.0.0.1:8091` | `0.0.0.0:8091` in a container, published only on an internal interface. Always bound, for the health checks |
| `PORTITOR_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | leave the default; the largest request body the public port takes (HTTP 413 above it; `POST /sts/token` has a lower fixed limit, [section 5](#5-security)). From `1` to `67108864` (64 MiB), or the start stops |
| `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | leave the default; the same for the management port |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset (no traces or metrics) | your OTLP receiver's base URL, if you collect them ([Traces and metrics](#traces-and-metrics)) |
| `PORTITOR_SCOPE_TYPE_ALIASES` | `org.eclipse.edc.vc.type` | comma-separated scope aliases that mean "credentials of this type", besides DCP's `org.eclipse.dspace.dcp.vc.type` |
| `PORTITOR_ALLOW_HTTP` | unset | **unset** (`true` allows plain-HTTP DIDs and URLs, tests only) |
| `PORTITOR_DID_CACHE_SECONDS` | `300` | leave the default; how long a resolved DID document (a caller's, an issuer's) is reused, `0` to `3600`, `0` for no cache ([DID document cache](#did-document-cache)) |
| `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` | unset | Gaia-X only: path of the PEM chain of the identity key, the key's certificate first ([Gaia-X onboarding](#gaia-x-onboarding-optional)) |
| `PORTITOR_GAIAX_COMPLIANCE_URL` | unset (Gaia-X off) | base URL of a Gaia-X compliance service, for example `https://compliance.lab.gaia-x.eu/v2`; needs the settings around it |
| `PORTITOR_GAIAX_COMPLIANCE_DID` | unset | Gaia-X only: the `did:web` DID of that service, for example `did:web:compliance.lab.gaia-x.eu:v2`; an answer signed by any other DID is refused |
| `PORTITOR_GAIAX_PARTICIPANT_FILE` | unset | Gaia-X only: path of the participant file |

### Endpoints

| Port | Path | Who calls it |
|---|---|---|
| public | `GET /.well-known/did.json` (or the DID's path) | anyone |
| public | `POST /sts/token` | your connector only (client id and secret); route it on the internal network only, never through the public proxy |
| public | `POST <credential service>/presentations/query` | verifiers, with a self-issued ID token and an access token from your STS (DCP 5.4) |
| public | `POST <credential service>/credentials`, `/offers` | trusted issuers (DCP 6.5, 6.6) |
| public | `GET /health` | probes |
| public | `GET {base}/.well-known/x509CertificateChain.pem` | anyone, with a chain set (Gaia-X services read it through `x5u`). `{base}` is empty for a DID without a path, else the DID path (`/participants/a` for `did:web:host:participants:a`) |
| public | `GET {base}/credentials/gaiax/{legal-person,terms-and-conditions,registration-number}` | anyone, with Gaia-X on; `{base}` as above |
| management | `POST`, `GET /v1/credential-requests`, `GET /v1/credential-requests/{holderPid}`, `GET /v1/credentials`, `DELETE /v1/credentials/{id}`, `GET /v1/offers`, `POST /v1/offers/request`, `POST`, `GET /v1/gaiax/compliance`, `GET /v1/openapi.json` | operators ([OpenAPI](api/identity-management-openapi.json)) |
| management | `GET /api/check/liveness`, `/readiness`, `/startup`, `/health` | probes, no API key, also without the management API ([Health and logs](#health-and-logs)). Readiness checks `database`, `key` and `did-document` |

The public endpoints are described in
[api/identity-openapi.json](api/identity-openapi.json).

The connector then needs these settings to get tokens from the identity
and to check partners' credentials:

| Variable (connector) | Production value |
|---|---|
| `PORTITOR_PARTICIPANT_ID` | the identity's DID |
| `PORTITOR_STS_URL` | **required**: the identity's STS token endpoint on the internal network, for example `http://identity:8090/sts/token` |
| `PORTITOR_STS_ALLOW_HTTP` | `true` when `PORTITOR_STS_URL` is plain `http://`: allows plain HTTP for that URL only, nothing else (the connector refuses to start with an `http://` STS URL unless this or `PORTITOR_ALLOW_HTTP` is `true`) |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | **required**: the same secret file as the identity's |
| `PORTITOR_STS_CLIENT_ID` | leave the default, `PORTITOR_PARTICIPANT_ID` |
| `PORTITOR_TRUSTED_ISSUERS` | **required**: DIDs trusted to issue credentials, usually the dataspace's `portitor-issuer` |
| `PORTITOR_DCP_SCOPES` | leave the default, `org.eclipse.dspace.dcp.vc.type:MembershipCredential` |

### Ports

| Port | Serves | Reached by | Expose |
|---|---|---|---|
| 8090 | DID document, credential service (presentation and Storage API), STS; with Gaia-X, the certificate chain and the Gaia-X credentials | partners and issuers (DID document, credential service); Gaia-X services (DID document, certificate chain); your connector (STS) | through your HTTPS reverse proxy, except `/sts/token`: your connector reaches it on the internal network |
| 8091 | health checks; management API with its key | your operators and probes | **never publicly**: host loopback or an internal network, never the public proxy |

### Run

Docker, next to the connector of [section 3](#3-run) (keys and secrets
as read-only mounts):

```sh
docker run -d --name portitor-identity --restart unless-stopped \
  -p 8090:8090 \
  -p 127.0.0.1:8091:8091 \
  -v portitor-identity-data:/data \
  -v /srv/portitor/identity-key.pem:/run/secrets/identity-key.pem:ro \
  -v /srv/portitor/sts-client-secret:/run/secrets/sts-client-secret:ro \
  -v /srv/portitor/identity-management-key:/run/secrets/identity-management-key:ro \
  -e PORTITOR_PARTICIPANT_ID=did:web:identity.example.com \
  -e PORTITOR_IDENTITY_KEY_FILE=/run/secrets/identity-key.pem \
  -e PORTITOR_CREDENTIAL_SERVICE_URL=https://identity.example.com/cs \
  -e PORTITOR_STS_CLIENT_SECRET_FILE=/run/secrets/sts-client-secret \
  -e PORTITOR_TRUSTED_ISSUERS=did:web:issuer.example.com \
  -e PORTITOR_MANAGEMENT_API_KEY_FILE=/run/secrets/identity-management-key \
  -e PORTITOR_MANAGEMENT_BIND=0.0.0.0:8091 \
  ghcr.io/brunopacheco1/portitor-identity@sha256:<digest>
```


Mount the same STS secret file into the connector and add the connector
settings above to its `docker run`.

The same rules as for the connector apply: one container per database
file, user `65532`, only `/data` writable, `docker stop` stops it cleanly.
Kubernetes: one replica with a `PersistentVolumeClaim` on `/data`; the key
and both secrets from `Secret`s mounted as files; only port 8090 in the
`Ingress`; probes on port 8091 as for the connector
([Health and logs](#health-and-logs)).

Check after start:

```sh
curl -sf https://identity.example.com/.well-known/did.json
```

### Getting a credential

The participant asks its issuer (for example the dataspace authority's
`portitor-issuer`) for credentials through the management API. The
issuer must be in `PORTITOR_TRUSTED_ISSUERS`, or nothing is sent.

1. Find the ids of the credentials you want in the issuer's metadata
   (DCP 6.7). The issuer's DID document names its `IssuerService` base
   URL; the metadata is at `<IssuerService>/metadata`:

   ```sh
   curl -s https://issuer.example.com/issuer/metadata
   ```

2. Ask for them:

   ```sh
   curl -X POST localhost:8091/v1/credential-requests \
     -H "Authorization: Bearer $(cat /srv/portitor/identity-management-key)" \
     -H 'Content-Type: application/json' \
     -d '{"issuerDid": "did:web:issuer.example.com", "credentialIds": ["membership-vc11"]}'
   ```

   The component resolves the issuer's DID, checks the ids against its
   metadata, records the request (`holderPid`) and sends a
   `CredentialRequestMessage` to `<IssuerService>/credentials` (DCP 6.4),
   with a self-issued ID token that carries an access token. The answer
   (`201`) gives the `holderPid` and the `statusUrl` where the issuer
   reports the request. If the issuer refuses it (an HTTP 4xx, answered
   `502`), nothing is kept: fix the cause and ask again. If its answer is
   unknown (no answer in time, a network error, HTTP 408, 429 or 5xx),
   the issuer may have taken the request: the answer is `202` with the
   `holderPid` and a `sendError`, and the request stays `REQUESTED` so the
   issuer's delivery is still accepted. If no delivery comes, ask again
   with a new `holderPid`.

3. Follow it: `GET /v1/credential-requests/{holderPid}` shows the local
   state (`REQUESTED`, then `ISSUED` or `REJECTED`) and the issuer's
   status, read from the `statusUrl` (DCP 6.8), plus `sendError` while
   the issuer's answer to the request is unknown and `issuerPid` once it
   delivered. `GET /v1/credential-requests` lists all requests. An issuer
   such as `portitor-issuer` decides and delivers in the background, so
   the request stays `REQUESTED` for a moment: poll until it is `ISSUED`
   (`request_credentials` in `scripts/participants.sh` does).

The issuer then sends its `CredentialMessage` with that `holderPid` to
`<credential service>/credentials`. The component checks the issuer's
token, that the issuer is trusted and matches the request, that the token
carries back the access token sent with the request (it is valid 30
days, so a later delivery is refused and you ask again), and each
credential's signature, dates, type and subject (your DID). The issuer's
answer (`ISSUED` or `REJECTED`) closes the request. The same message
again (same `issuerPid` and status, for example when the issuer did not
get our reply) is answered `200` and stores nothing; any other delivery
for the same `holderPid` is refused, so a renewed credential needs a new
request. Credentials are stored as received (JWT); `GET
/v1/credentials` lists them and `DELETE /v1/credentials/{id}` removes one,
after which it is no longer presented.

**Revoked credentials.** The component does not read status lists, so it
keeps presenting a credential its issuer revoked (DCP v1.0.1, 5.4.2: a
presentation "SHOULD only include valid" credentials). After a reissue,
delete the revoked credential from the identity with
`DELETE /v1/credentials/{id}` once the new one is listed. Otherwise
other participants refuse you: a presentation that holds a revoked
credential is invalid (DCP v1.0.1, 5.4.3, step 8), so Portitor
verifiers refuse it even with the new credential next to it, and EDC
verifiers may too.

**Offers.** An issuer may tell you that credentials are waiting (DCP
6.6), for example before its key changes. `GET /v1/offers` lists the
offers; nothing is asked for by itself. To ask for what an issuer
offered: `POST /v1/offers/request` with `{"issuerDid": "..."}` (and
`credentialIds` to ask for only some).

**Issuers asked out of band.** For an issuer that is asked some other
way (for example the DCP TCK), leave `credentialIds` out: the request is
only recorded, so the Storage API accepts that issuer's delivery for the
`holderPid` you give, without an access token:

```sh
curl -X POST localhost:8091/v1/credential-requests \
  -H "Authorization: Bearer $(cat /srv/portitor/identity-management-key)" \
  -H 'Content-Type: application/json' \
  -d '{"issuerDid": "did:web:issuer.example", "holderPid": "membership-2026"}'
```

### Gaia-X onboarding (optional)

The identity can get a Gaia-X compliance credential for its participant
and keep it valid ([ADR 0021](adr/0021-gaia-x-onboarding.md)). It signs
a legal person credential (`gx:LegalPerson`) and a terms and conditions
credential (`gx:Issuer`) with its key, sends them with your registration
number credential to a Gaia-X compliance service, checks the answer
(`gx:ParticipantComplianceCredential`) and stores all four. Verifiers
then get them through the presentation API, for example with the scope
`org.eclipse.dspace.dcp.vc.type:ParticipantComplianceCredential` or
`org.eclipse.dspace.dcp.vc.type:LegalPerson` (a scope type `T` also
matches the Gaia-X type `gx:T`).

You need three things.

**1. A certificate for the identity key.** Gaia-X wants the key in the
DID document to come with an X.509 chain that ends at a Gaia-X trust
anchor. Keep the identity key; get a certificate for it:

- Make a certificate request (CSR) with the identity key, for the host
  of your DID:

  ```sh
  openssl req -new -key identity-key.pem -subj "/CN=identity.example.com" \
    -addext "subjectAltName=DNS:identity.example.com" -out identity.csr
  ```

- For tests (the Gaia-X Lab accepts certificates that are not EV, its
  compliance service README says): Let's Encrypt with your own key.
  `certbot certonly --csr` signs your CSR, so the key never leaves you:

  ```sh
  certbot certonly --webroot -w /var/www/html --csr identity.csr \
    --cert-path cert.pem --chain-path ca.pem --fullchain-path chain.pem
  ```

  `chain.pem` is the file to mount. certbot does not renew a `--csr`
  certificate by itself: run the same command again before it expires
  (Let's Encrypt certificates last 90 days).
- For production: a certificate from a Gaia-X trust anchor, that is an
  eIDAS trust service provider, or an EV SSL certificate where the Gaia-X
  Registry has no other option
  ([trust anchors](https://docs.gaia-x.eu/policy-rules-committee/compliance-document/25.10/Gaia-X_Trust_Anchors/)).
  Give the provider the same CSR.

Mount the chain (your certificate first, then the CA certificates) and
set `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` to it. The identity refuses to
start if the first certificate is not for its key. It serves the chain at
`https://<your host>/.well-known/x509CertificateChain.pem` (below the DID
path, for a DID with one) and points to
it from the DID document (`x5u`). Route that path to the identity, like
the DID document. It reads the file again every minute: put the renewed
chain in place and nothing else is needed. A new file for another key is
refused and logged; the old chain stays.

**2. A registration number credential.** A notary of a Gaia-X Digital
Clearing House (GXDCH) checks your registration number (VAT ID, LEI code,
EORI, tax ID or a national company number) and signs a credential for
it. Ask once, for example at the Gaia-X Lab notary
(`https://registrationnumber.notary.lab.gaia-x.eu/v2`, API description
at `/v2/docs`):

```sh
base=https://identity.example.com/credentials/gaiax/registration-number
curl -s -G "https://registrationnumber.notary.lab.gaia-x.eu/v2/registration-numbers/vat-id/BE0762747721" \
  --data-urlencode "vcId=$base" --data-urlencode "subjectId=$base#cs" \
  > registration-number.jwt
```

Use `https://<your host>/credentials/gaiax/registration-number` (below the
DID path, for a DID with one) as
`vcId`, and the same with `#cs` as `subjectId`: the identity serves the
credential at that URL. The answer is a VC-JWT; keep it as a file.

**3. The participant file.** JSON, only Gaia-X terms, copied from the
Gaia-X ontology the compliance service expects:

```json
{
  "context": ["https://w3id.org/gaia-x/development#", {"schema": "https://schema.org/"}],
  "legalPerson": {
    "schema:name": "Example S.A.",
    "gx:legalAddress": {"type": "gx:Address", "gx:countryCode": "BE",
      "vcard:street-address": "1 Rue Example", "vcard:locality": "Bruxelles",
      "vcard:postal-code": "1000"},
    "gx:headquartersAddress": {"type": "gx:Address", "gx:countryCode": "BE",
      "vcard:street-address": "1 Rue Example", "vcard:locality": "Bruxelles",
      "vcard:postal-code": "1000"}
  },
  "termsAndConditions": "4bd7554097444c960292b4726c2efa1373485e8a5565d94d41195214c5e0ceb3",
  "registrationNumberCredential": "registration-number.jwt"
}
```

- `context`: the Gaia-X JSON-LD context, one IRI or a list. Add prefix
  definitions your claims use and the Gaia-X context lacks (here
  `schema:`).
- `legalPerson`: the claims of your legal person credential, without
  `id` and `gx:registrationNumber`: the identity writes both (the
  registration number points to the subject of the notary's
  credential).
- `termsAndConditions`: the `gaiaxTermsAndConditions` hash the compliance
  service expects (the value above is the one the 0.8 spike used).
- `registrationNumberCredential`: the file from step 2, absolute or next
  to the participant file.

The identity reads the file at start (and refuses to start if it is
wrong) and again at each run, so a changed file needs no restart.

**Run it.** Set `PORTITOR_GAIAX_COMPLIANCE_URL` and the service's DID,
`PORTITOR_GAIAX_COMPLIANCE_DID` (the identity refuses an answer signed by
any other DID). At start, if there is no
valid compliance credential, the identity runs at once; then it runs
again when two thirds of the credential's validity have passed (it is
valid at most 90 days). A failed run keeps the credentials you have and
is tried again after 1, 2, 4 ... seconds, at most one hour apart. To run
now, and to see the runs:

```sh
curl -X POST localhost:8091/v1/gaiax/compliance \
  -H "Authorization: Bearer $(cat /srv/portitor/identity-management-key)"
curl -s localhost:8091/v1/gaiax/compliance \
  -H "Authorization: Bearer $(cat /srv/portitor/identity-management-key)"
```

Each run shows `started`, `finished`, `outcome` (`RUNNING`, `SUCCEEDED`
or `FAILED`) and `error`. The legal person, terms and conditions and
registration number credentials of the last good run are public at
`https://<your host>/credentials/gaiax/{legal-person,terms-and-conditions,registration-number}`,
so anyone can check the digests in the compliance credential.

Network: outbound to the compliance service (`https://`); it calls back
your DID document, the certificate chain and the notary's DID. After
a key change, run again ([Rotating a key](#rotating-a-key)): the chain
is published for the current key only.

### Logs and operation

| Log message (level) | Meaning |
|---|---|
| `portitor-identity listening` (INFO) | Started; shows the DID, the key id and the database |
| `retired key listed in the DID document; it does not sign` (INFO, at start) | One line per key in `PORTITOR_IDENTITY_RETIRED_KEY_FILES`; shows its key id |
| `management port listening` (INFO, at start) | The management port's address (`bind`); always bound, for the health checks |
| `telemetry: exporting` (INFO, at start) | Traces and metrics are exported ([Traces and metrics](#traces-and-metrics)) |
| `STS token issued` (INFO) | Your connector got a token for the named audience |
| `STS client refused` (INFO) | A wrong client id or secret at the STS |
| `request refused` (INFO) | A caller's token was refused; `reason` says why (expired, replayed, wrong audience, untrusted issuer, ...). The caller only gets HTTP 401 |
| `presentation sent` (INFO) | A verifier got a presentation; `credentials` is how many it held |
| `issuance message refused` (INFO) | A `CredentialMessage` or offer was refused (HTTP 400); `reason` says why |
| `credential request sent` (INFO) | The issuer took a credential request (`holderPid`) |
| `credential request not sent` (INFO) | The issuer refused a request (HTTP 4xx), or it could not be signed; `reason` says why. Nothing was kept |
| `credential request outcome unknown; kept open` (WARN) | No answer in time, a network error, or HTTP 408, 429 or 5xx from the issuer; the request stays open for a late delivery |
| `repeated delivery acknowledged` (INFO) | The issuer sent an answer again that closed a request before; nothing was stored twice |
| `the issuer gave no usable Location for the request` (WARN) | The issuer took the request but gave no status URL on its own host; the delivery is still accepted, only the issuer's status cannot be shown |
| `identity store failed` (ERROR) | The SQLite file could not be read or written |
| `certificate chain reloaded` (INFO) | A renewed certificate chain for the same key is in use |
| `new certificate chain refused; the old one stays` (ERROR) | The chain file changed but cannot be used: unreadable, not PEM, or for another key; `reason` says which |
| `Gaia-X compliance credential stored` (INFO) | A run succeeded; `valid_until` is when the compliance credential expires |
| `Gaia-X compliance run failed; the stored credentials stay` (ERROR) | A run failed; `error` says why (the service refused, its answer did not check out, ...). It is tried again |

Backup, restore, upgrade and rollback work as for the connector, with
`/data/portitor-identity.db*`. Used token ids (`jti`) are kept in memory
until their token expires (ADR 0016), so one replica per database file.

**Key rotation:** see [Rotating a key](#rotating-a-key). The key id
is the key's thumbprint, so a new key file gives a new key id.

### Two data spaces

One connector serves one data space
([ADR 0024](adr/0024-several-data-spaces.md), accepted for 1.0). If
your organisation is in two data spaces, run **one connector per data
space**. Each connector has its own:

- host name and `PORTITOR_PUBLIC_URL` (the connector serves
  `/dsp/2025-1` at the root of its host), and its own management port
  and key;
- database and volume (`PORTITOR_DB_FILE`; one connector per file);
- datasets file, with the description and the offers that data space
  asks for; the `portitor:source` of a dataset can be the same in both;
- `PORTITOR_TRUSTED_ISSUERS`, with the issuers of **that data space
  only**, and that data space's `PORTITOR_DCP_SCOPES`,
  `PORTITOR_POLICY_CREDENTIALS` and bundle settings.

A member of one data space is then refused by the connector of the
other, because its credential comes from an issuer that connector does
not trust.

For the identity there are two ways:

| | One identity and DID per data space (the default) | One shared identity and DID |
|---|---|---|
| Deploy | One `portitor-identity` per data space, each with its own DID, key, database and host name. Each connector uses the STS of its own identity | One `portitor-identity`. Both connectors have the same `PORTITOR_PARTICIPANT_ID`, `PORTITOR_STS_URL` and STS client secret |
| `PORTITOR_TRUSTED_ISSUERS` of the identity | That data space's issuers | The issuers of both data spaces |
| Credentials | Each identity asks its data space's issuer. With Gaia-X, one onboarding per identity | The one identity asks each issuer |
| What partners learn | Nothing about the other data space | **A partner in one data space learns that you are a member of the other,** when both use the same credential type: it asks for `MembershipCredential` and gets both (DCP has no scope by issuer). Its own check skips the credential of the issuer it does not trust |

Use a shared DID only if this disclosure is acceptable in both data
spaces. With a shared DID, also remember that a partner whose connector
trusts both issuers refuses you while one of the two credentials is
revoked or expired (section 5, "Caller authentication").

Tested by hand on 2026-11-20, both ways, with
`scripts/two-data-spaces.sh` ([TESTING](TESTING.md#two-data-spaces)):
on one machine over plain HTTP, without a reverse proxy, a bundle or
Gaia-X. One connector for several data spaces is planned after 1.0.

### Users and roles

Portitor is system to system: it knows participants, not people
([ADR 0025](adr/0025-users-and-roles.md), accepted for 1.0). Each
management API has one key, and the key grants every right.

To let people and applications of your organisation use a management
API, put your own reverse proxy or API gateway in front of the
management port. The proxy signs people in at your identity provider
(for example with OpenID Connect), decides by method and path who may
call what, logs the person, and adds the Portitor key. The proxy must be
the only way to the management port. Portitor's own logs do not name the
person; match them with the proxy's log by time and trace id.

Two limits of this pattern: the proxy cannot hide the `dataAddress`
(the data plane token) from someone who may read transfers, and the
provider's data plane does not know which person of the consumer pulls
the data. The contract is between participants; the consumer answers
for its people. This pattern is described from ADR 0025; it was not
tested with a proxy product.

## 5. Security

- **Caller authentication (DCP, ADR 0016).** Every DSP endpoint, the
  consumer callbacks included, needs a self-issued ID token in
  `Authorization: Bearer`. The connector checks it, asks the caller's
  credential service for its presentations, and accepts the caller only
  with a valid `MembershipCredential` from a DID in
  `PORTITOR_TRUSTED_ISSUERS` whose subject is the caller, not revoked or
  suspended (the status list must be signed by the same issuer; a
  verified list is reused for `PORTITOR_STATUS_LIST_CACHE_SECONDS`, see
  [Status list cache](#status-list-cache)). One
  credential from a trusted issuer that fails its checks (revoked,
  expired, bad signature, status list not readable, about someone else)
  makes its presentation invalid and refuses the caller, even next to a
  valid membership credential (DCP v1.0.1, 5.4.3, step 8); the log names
  it (`credential refused`, INFO). So a participant must delete a revoked
  credential once its reissued one arrives (section 4). Credentials from
  other issuers are skipped. When refused it
  answers with a DSP error that says only "unauthorized": HTTP 404 on
  the negotiation and transfer routes (DSP 2025-1 HTTPS bindings,
  "Unauthorized Access"; Eclipse EDC does the same), HTTP 401 on the
  catalog routes. The reason is in the log (`DSP request refused: caller
  not authenticated`, WARN). When the check cannot finish because a server
  it fetches from does not answer, or answers 408, 429 or 5xx, it
  answers HTTP 503 instead, so the partner tries again later. Each negotiation and transfer is bound to the DID
  that started it: another caller is told it does not exist. As
  consumer, an agreement must name your participant id as `assignee`.
  Only `/.well-known/dspace-version`, `/health` and the data plane are
  open (the data plane has its own tokens).
- **Limits against abuse.** Tokens valid for more than an hour are
  refused (both components), and at most 100,000 used token ids are
  remembered; when that is full, new tokens are refused until old ones
  expire (log reason `too many token ids in use`). The connector checks
  only credentials from trusted issuers, reads at most 8 presentations of
  16 credentials with 4 status entries each, and gives up on a caller
  after 20 s. On the public ports each request gets 60 s and at most 256
  are worked on at once; the others wait, and a request over time gets
  HTTP 503 (log `request took too long or waited too long`, WARN).
- **Request size limits (R-18).** Each port takes request bodies up to
  its setting, `PORTITOR_MAX_REQUEST_BYTES` (public port) and
  `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` (management port), 2 MiB by
  default. These routes, whose messages are small by specification, have
  a fixed limit of 64 KiB, or the port's limit when that is lower:
  - connector: `POST /dsp/2025-1/catalog/request`; `POST
    /dsp/2025-1/negotiations/{providerPid}/events`,
    `/agreement/verification` and `/termination`; `POST
    /dsp/2025-1/transfers/{providerPid}/suspension`, `/completion` and
    `/termination`; and the same messages on the consumer callbacks
    (`/dsp/2025-1/callback/negotiations/{consumerPid}/events` and
    `/termination`, `/dsp/2025-1/callback/transfers/{consumerPid}/suspension`,
    `/completion` and `/termination`);
  - identity: `POST /sts/token`;
  - issuer: `POST <issuer service>/credentials`.

  A larger body gets HTTP 413 with a problem body (log `request body too
  large`, INFO). A request that declares its length (`Content-Length`)
  is refused before its body is read; a body without a declared length
  is read by the handler, after the DSP caller check, and refused once
  it goes over the limit. Data plane answers are streamed and have no
  size limit.
- **Trusted issuers.** Whoever controls a DID in
  `PORTITOR_TRUSTED_ISSUERS` can admit any participant. List only your
  dataspace's issuers.
- **Bundle (ADR 0005).** The connector trusts only a bundle signed with
  the bundle key that the authority's `did:web` document lists, for the
  newest version (or the one you fixed), never one older than the version
  in use; a changed file or signature stops the start, and a newer
  version that fails a check is not taken. The
  bundle key is separate from the issuer's credential key, and the issuer
  holds only its public part. Shapes, code lists and contexts are never
  fetched from the network apart from the authority's signed bundle, and no Portitor
  binary parses RDF/XML.
- **Outbound calls.** The connector POSTs to the `callbackAddress` that the
  partner chooses. Only `https://` is allowed, but internal addresses are
  not blocked yet. Put the connector in a network zone where it cannot
  reach internal services it should not call (egress firewall or network
  policy).
- **Management API.** It can read catalogs and start negotiations and
  transfers in your name (each call sends a request, with a token of
  yours, to the address the caller gives), and it shows the data access tokens of your transfers, so treat its
  key like a password: a file readable only by the connector, mounted from
  your secret store, at least 32 characters. Only an HMAC of the key is
  kept in memory, and keys are compared in constant time. The port speaks
  plain HTTP: keep it on a private network or behind TLS, and never on the
  public proxy. To change the key, replace the file and restart. The
  key is an opaque string, and so is the data plane token of a
  transfer: your systems must not parse them, and a 1.x release may
  accept or issue other forms ([STABILITY](STABILITY.md)). This holds
  for the keys of the identity and the issuer too.
- **Health checks and telemetry (0.9).** The health checks answer
  without a key and name what failed, so keep the management port on
  an internal network, as for the management API. Traces and metrics
  are off by default. Spans name your partners' hosts and the DSP
  process ids, so send them only to a receiver you control, over
  `https://` when it is not on your internal network, with its key in
  `OTEL_EXPORTER_OTLP_HEADERS`. Your data sources receive a
  `traceparent` header with telemetry on; partners never do
  ([Traces and metrics](#traces-and-metrics)).
- **The database file holds access tokens.** As consumer, the connector
  keeps the data address of each usable transfer, token included, in
  `/data/portitor.db` until the transfer ends (ADR 0014). Protect the volume
  and its backups like the management key.
- Do not set `PORTITOR_ALLOW_HTTP=true` outside development and tests.
- Report vulnerabilities as described in [SECURITY.md](../SECURITY.md).

## 6. Operate

### Health and logs

- `GET /health` on the public port (and on the management port, with the
  management API on) answers `ok` while the process runs. It checks
  nothing.
- **Health checks** (ADR 0023), the same in all three components, on the
  **management port**, without the API key, also when the management API
  is off. The answer is Eclipse EDC's `HealthStatus`: HTTP 200 when every
  check passes, 503 otherwise
  ([OpenAPI](api/management-openapi.json), tag `Health`):

  ```json
  {"isSystemHealthy": false, "componentResults": [
    {"component": "database", "isHealthy": true, "failure": null},
    {"component": "identity", "isHealthy": false,
     "failure": {"messages": ["STS: HTTP 401 Unauthorized"], "failureDetail": "STS: HTTP 401 Unauthorized"}}]}
  ```

  | Path | Checks |
  |---|---|
  | `GET /api/check/liveness` | nothing but the process (component `process`) |
  | `GET /api/check/readiness` | the component's own dependencies, never a partner (below) |
  | `GET /api/check/startup`, `GET /api/check/health` | the same as readiness |

  Readiness of the connector:
  - `database`: the SQLite file answers.
  - `harvest` (the start harvest ran) and `bundle` (a version is in
    force; only with a bundle): start gates. The connector binds its
    ports only after both, so they cannot fail while it serves.
  - `identity`: the connector's DID document lists a credential service,
    and its STS signs a token that the document's key verifies. The
    document is read at the DID's own URL, as partners read it, so this
    check also goes through your reverse proxy, DNS and certificate. A
    background task runs it every 10 s, with a 3 s timeout; probes read
    the last result. It fails only after 2 failures in a row (about
    20 s), and passes again after one success. Before its first result
    the connector is not ready.

  Identity: `database`, `key` (the key is loaded and signs),
  `did-document` (the DID document it serves names the participant and
  lists that key). Issuer: `database`, `key`, `status-lists` (both
  status lists it serves are signed by that key). These run in the
  process, at each probe. Each check gets 0.5 s. A failure names the
  check and why, never a key or token. A check is logged only when its
  result changes: `readiness check failed` (WARN, with `component` and
  `reason`) when it starts failing, `readiness check passes again`
  (INFO) when it recovers. The health checks are neither traced nor
  measured, and the background identity check sends no `traceparent`.
- **`health` subcommand**: `portitor-connector health` (and
  `portitor-identity health`, `portitor-issuer health`) calls its own
  `GET /api/check/readiness` on `PORTITOR_MANAGEMENT_BIND` (`0.0.0.0` is
  called as `127.0.0.1`), prints the answer, and exits 0 on HTTP 200, 1
  otherwise or after 3 s. It needs the same `PORTITOR_MANAGEMENT_BIND` as
  the running process. The connector and the identity take no other
  argument (exit code 2); the issuer also takes `bundle build`.
- **Docker**: each image has a `HEALTHCHECK` with that subcommand
  (every 30 s, 5 s timeout, 3 retries; every 2 s during a 60 s start
  period). `docker ps` shows `healthy` or `unhealthy`; Docker Compose can
  start a service once another is healthy (`depends_on: condition:
  service_healthy`, as in `examples/compose/compose.yaml`). The check
  every 2 s during the start period (`--start-interval`) needs Docker
  Engine 25 or later (Docker Desktop 4.27 or later). Podman runs the
  images too, but may not read their `HEALTHCHECK`: see
  [TESTING](TESTING.md#docker-or-podman).
- **Kubernetes**: set `PORTITOR_MANAGEMENT_BIND=0.0.0.0:<port>` and probe
  the management port (8081 connector, 8091 identity, 8071 issuer); keep
  that port out of any `Service` that the public reaches:

  ```yaml
  startupProbe:
    httpGet: {path: /api/check/startup, port: 8081}
    periodSeconds: 2
    failureThreshold: 60   # the start check of the identity may take ~30 s
  livenessProbe:
    httpGet: {path: /api/check/liveness, port: 8081}
  readinessProbe:
    httpGet: {path: /api/check/readiness, port: 8081}
    periodSeconds: 10
  ```

  A failing readiness takes the pod out of its `Service`; it does not
  restart it. Only liveness restarts, and it checks only the process.
- Logs are JSON lines on stdout, one event per line. Collect them with
  your platform's log driver.

| Log message (level) | Meaning | What to do |
|---|---|---|
| `counter-party not reachable; terminating` (WARN) | A message to a partner failed and the partner's state does not show it arrived; the negotiation or transfer is terminated. The message is tried up to 4 times (0.5 s, 1 s, 2 s apart) when no answer came, or the answer was 408, 429 or 5xx; any other answer (400, 404, 409, ...) is final after one try, and `error` shows it | No answer: check the partner's address, DNS, TLS trust and your egress rules. An HTTP status: the partner refused the message; its logs say why |
| `message breaks the dataspace's message rules; not sent, terminating` (WARN) | With bundle message shapes: our own message failed them (`error` lists the SHACL reasons), so it was not sent and the negotiation or transfer is terminated; `..., process not started` for a consumer's first request, `termination breaks ...; not sent` for a termination | Check the offer or policy against the authority's rules |
| `outbound DSP message breaks the dataspace's message rules; not sent` (WARN) | The same breach, logged by the check itself with `message_type` and `reasons` | As above |
| `inbound DSP message breaks the dataspace's message rules; answered 400` (INFO) | A partner's negotiation or transfer message failed the bundle's message shapes; it got HTTP 400 and nothing changed | Nothing, unless the partner asks |
| `inbound DSP message is not valid; answered 400` (INFO) | A partner's negotiation or transfer message could not be read; `reason` says why. `unknown field ...` in an offer or agreement: a policy term the connector does not read (for example a `duty` inside a permission), refused on purpose. The partner got HTTP 400 and nothing changed | Nothing, unless you want that offer: then ask the partner for one with only the terms listed under [Known limits](#known-limits) |
| `transfer request is refused; answered 400` (INFO) | A partner asked for a transfer under an agreement that is not finalized here, is not its own, or does not exist (`reason`: `no finalized agreement with this id for the caller`). It got HTTP 400 and no transfer was made. The line names neither the agreement nor the partner. From 0.13 a request that arrives just before the partner's acknowledgement of the finalization is recorded is accepted, not refused (ADR 0007) | Nothing, unless a partner asks: its negotiation must reach `FINALIZED` on your side first (look for its `negotiation state changed` lines) |
| `message had arrived before` (INFO) | A send failed, but the partner's state shows it had received the message (a lost answer, or a resend after a restart); the negotiation goes on | Nothing |
| `sending pending negotiation messages again` (INFO, at start) | Messages saved but not acknowledged before the last stop are being sent | Nothing |
| `counter-party not reachable; process not started` (WARN) | The first request of a negotiation or transfer we started as consumer failed (tried as above); it ends without a message, as the provider never knew it | Same as above |
| `termination not acknowledged` (WARN) | The partner did not take a termination; DSP allows this, the negotiation is terminated anyway | Nothing, unless it repeats for one partner |
| `no CA certificates: https calls will fail` (WARN, at start) | The CA bundle was not found (for example `SSL_CERT_DIR` points to an empty folder) | Fix the mount or the variable |
| `negotiation store failed` (ERROR) | The SQLite file could not be read or written; partners get HTTP 500 | Check disk space and the `/data` volume |
| `DSP request refused: caller not authenticated` (WARN) | A DSP request was refused: the caller got HTTP 404 on a negotiation or transfer route, HTTP 401 on a catalog route (`status` says which). `reason` says why (token, presentation, credential or status list); the caller is not told | If a partner should be admitted: check its credential, its issuer against `PORTITOR_TRUSTED_ISSUERS`, and clock skew (60 s allowed) |
| A partner says a negotiation or transfer call got HTTP 404 `unauthorized`, or its side ended with `no such negotiation` although the process exists | From 0.12 a caller that is not authenticated gets 404 on the negotiation and transfer routes, as DSP 2025-1 asks (Eclipse EDC does the same). The 404 does not say why | Look in **your** log for `DSP request refused: caller not authenticated` at that time: its `reason` names the failed check. A 404 with reason `no such negotiation` or `no such transfer` and no such log line means the process id is unknown, or belongs to another participant |
| `counter-party not reachable; terminating` (WARN) with `HTTP 404` in `error` | The partner answered 404 to your message: it does not know the process, or it refused your credentials (a Portitor from 0.12 and Eclipse EDC answer 404 for both). The message is not sent again | Ask the partner for the reason in its log; check that your credential is valid and not revoked, and that the partner trusts its issuer |
| `DSP request not checked: a service the check needs is unavailable; answered 503` (WARN) | A server the check fetches from (the partner's DID document or Credential Service, its issuer's DID document or status list, your STS) did not answer, or answered 408, 429 or 5xx; `reason` names it. The partner tries again | If it repeats: check your STS, then ask the partner about its services |
| `no token: ...` in a send failure (WARN) | The STS did not give a token for an outgoing DSP request | Check the identity component, `PORTITOR_STS_URL` and the shared secret |
| `own identity checked: DID document and STS agree` (INFO, at start) | The connector's DID document resolves and the STS signs for it | Nothing |
| `panic` (ERROR) | A bug: code stopped at `place` (file, line and column). One of the next two lines follows | Report it with `place` and the version |
| `a request handler panicked; answered 500` (ERROR) | The request on `route` got HTTP 500; the other requests went on | Report it; if it repeats for one partner, its message triggers the bug |
| `a background task panicked; it starts again` / `...; its message is sent again` / `...; its work is left as saved` (ERROR) | The task `task` stopped at a bug. A task that runs all the time starts again after a pause (`restart_in_seconds`): 5 seconds, twice as long after each further panic in a row, at most 5 minutes. A DSP message whose sending panicked (`outbox_delivery`) is sent again after the same pauses (`again_in_seconds`). A credential decision stays saved and is tried again on the issuer's next outbox round (at most a minute); the other requests and deliveries go on | Report it. A line that repeats for the same `pid` or `issuer_pid` is a message or request that meets the bug every time: it stays as it is until a fixed version runs |
| `portitor-connector stopped` (ERROR) | The process could not start or stopped with an error; the message says why | Fix the cause; the container restarts |
| `harvesting datasets` (INFO, at start) | The datasets file and the interval in use | Nothing |
| `no datasets file (PORTITOR_DATASETS_FILE): the catalog is empty` (INFO) | No datasets file is set | Set it, if you offer datasets |
| `harvest run` (INFO) | One per run: `job`, `trigger`, `outcome` and the counts | Nothing |
| `harvest run` (WARN, `outcome=failed`) | The source could not be read; the catalog is unchanged | Check the file, its mount and its JSON |
| `harvest: dataset rejected` (WARN) | One dataset failed a check: `position`, `dataset`, `check`, `error`. With check `odrl` and `offer "..." is not understood: unknown field ...`: the offer has a policy term the connector does not read, or a misspelled key | Fix the dataset in the source; the next run takes it. For an unknown field: correct the key, or remove the term ([Known limits](#known-limits)) |
| `harvest: dataset has no portitor:source; its transfers will be refused` (WARN) | A dataset was added or changed without a data source | Add `portitor:source`, unless the dataset is not transferred |
| `harvest skipped: a run is going on` (INFO) | A timer tick came while a run was going on | Nothing, unless it repeats: then lengthen the interval |
| `harvest: cannot store the run; ...` (ERROR) | The database could not be written; the catalog keeps what it had | Check disk space and the `/data` volume |
| `looking for new bundle versions` (INFO, at start) | The bundles URL or folder and how often `latest.json` is read | Nothing |
| `bundle: no new version is looked for (...)` (INFO, at start) | A fixed `PORTITOR_BUNDLE_VERSION`, or `PORTITOR_BUNDLE_REFRESH_SECONDS=0` | Nothing |
| `bundle file skipped: this version does not know its role; what the file holds is not enforced` (WARN) | The bundle in force lists a file with a role of a later version (`path`, `role_type`, `version`). The file is not downloaded and not used; the other files are. Logged each time a version is taken | Upgrade the connector to enforce that kind of rule; until then it is not enforced here. If the role type looks misspelt, tell the authority |
| `bundle downloaded and verified` (INFO) | A bundle version was downloaded and passed every check | Nothing |
| `bundle download failed; using the cached copy, checked again` (WARN, at start) | The authority's host or DID document could not be reached; the cached copy is used | Check the network to the authority; nothing breaks meanwhile |
| `bundle: the newest version cannot be taken; keeping the version in use` (ERROR, at start) | With `latest`: the newest version could not be downloaded, failed a check or is older than the one in use; the cached version in use before is kept | Check the reason in the log line or in `GET /v1/harvest-jobs`; ask the authority |
| `bundle: no new version taken; keeping the version in use` (WARN) | A scheduled check failed for the same reasons; nothing changed | As above; the next check tries again |
| `bundle: new version in use; checking the stored datasets again` (INFO) | A newer version was put in use without a restart | Nothing; watch the next harvest jobs for hidden datasets |
| `bundle verified, but not written to the cache` (WARN) | The cache folder is not writable | Check `PORTITOR_BUNDLE_CACHE_DIR` and the volume |
| `bundle rules in force` (INFO, at start) | The bundle's `version`, `profile`, the `access_rights` it has dataset shapes for, and whether it has `message_shapes` | Nothing |
| `bundle shapes: ignored by the SHACL engine` (INFO or WARN, at start) | A SHACL term the engine ignores (`sh:extends`, `sh:shape`, `sh:property` without `sh:path` at INFO: expected for HealthDCAT-AP and DCAT-AP; anything else at WARN) | Nothing at INFO; at WARN, ask the authority |
| `bundle context: not used` (WARN, at start) | The bundle has a `context` file for a DSP 2025-1 context URL; the connector's own copy is used | Nothing |

### Traces and metrics

All three components (connector, identity, issuer) can send traces and
metrics with OpenTelemetry ([ADR 0023](adr/0023-observability.md)).
Export is **off** unless `OTEL_EXPORTER_OTLP_ENDPOINT`,
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or
`OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` is set. There is
no Prometheus scrape endpoint: the components push over OTLP, to an
OpenTelemetry Collector or any receiver that takes OTLP/HTTP (Prometheus
3 with `--web.enable-otlp-receiver`, Jaeger).
The OpenTelemetry SDK reads the `OTEL_*` values as written: a space
around an endpoint makes it invalid, and export stays off with an ERROR
line (`telemetry: export is off`).

**Switch it on.** Set the receiver's base URL; the components add
`/v1/traces` and `/v1/metrics`:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318
```

The start log then says `telemetry: exporting` (INFO), with `traces`
and `metrics` set to `true` or `false`. Each signal needs an endpoint:
the generic one counts for both, a signal-specific one only for its
signal. So with only `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` set, traces
are exported and metrics are not.

Check before you upgrade: if `OTEL_EXPORTER_OTLP_ENDPOINT` is already
in the environment (for example injected by the OpenTelemetry Operator),
export turns on with 0.9.

`http://` and `https://` are both allowed: the receiver is your own.
An `https://` receiver needs the CA bundle, as for every other call
([Private CAs](#private-cas)). Protocol: OTLP over HTTP with Protocol
Buffers only (no gRPC, no JSON), without compression. A setting the SDK
refuses is logged (`telemetry: export is off`, ERROR) and the component
runs on with logs only. A value the SDK does not know for compression or
temporality only gets an SDK warning; the default is used. A failed
export is logged by the SDK (target `opentelemetry_sdk`); its data is
dropped, and the next batch is sent as usual. At shutdown, the last
spans and metrics get about 3 s together; what is not sent by then is
lost, logged as `telemetry: shutdown took too long; the last data is
lost` (WARN; also `telemetry: the last data was not sent` or
`telemetry: shutdown failed` when the receiver refused it).

**Variables.** Only OpenTelemetry's own variables; there is no
`PORTITOR_*` setting. Read by the Rust SDK 0.33 that Portitor uses:

| Variable | Use |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Switches export on for both signals; the receiver's base URL |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | A full URL for one signal, used instead of the base URL; each also switches export on, for its signal |
| `OTEL_EXPORTER_OTLP_HEADERS` (and `_TRACES_`, `_METRICS_`) | Headers for the receiver, `key=value,key2=value2`, for example an API key |
| `OTEL_EXPORTER_OTLP_TIMEOUT` (and `_TRACES_`, `_METRICS_`) | Milliseconds an export may take (default 10000; each request is also cut off after 10 s) |
| `OTEL_SERVICE_NAME` | `service.name`; default `portitor-connector`, `portitor-identity` or `portitor-issuer` |
| `OTEL_RESOURCE_ATTRIBUTES` | More resource attributes, for example `deployment.environment.name=prod`; `service.version` is Portitor's version unless set here |
| `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG` | Which traces are kept (below); default every trace (`parentbased_always_on`) |
| `OTEL_BSP_SCHEDULE_DELAY`, `OTEL_BSP_MAX_QUEUE_SIZE`, `OTEL_BSP_MAX_EXPORT_BATCH_SIZE` | The batch of spans: milliseconds between exports (default 5000), spans kept waiting (2048; more are dropped), spans per export (512) |
| `OTEL_METRIC_EXPORT_INTERVAL` | Milliseconds between metric exports (default 60000) |
| `OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE` | `cumulative` (default), `delta` or `lowmemory`. Keep the default for Prometheus' OTLP receiver: it takes cumulative metrics, and delta only with an experimental feature flag ([Prometheus guide](https://prometheus.io/docs/guides/opentelemetry/)) |
| `OTEL_EXPORTER_OTLP_COMPRESSION` (and `_TRACES_`, `_METRICS_`) | Read, but compression is not built in: the SDK logs a warning and sends uncompressed |
| `OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT`, `OTEL_SPAN_EVENT_COUNT_LIMIT`, `OTEL_SPAN_LINK_COUNT_LIMIT` | Limits per span (default 128 each) |

No effect: `OTEL_SDK_DISABLED` (to switch export off, unset the
endpoint variables), `OTEL_EXPORTER_OTLP_PROTOCOL` (always
`http/protobuf`), `OTEL_BSP_EXPORT_TIMEOUT`,
`OTEL_BSP_MAX_CONCURRENT_EXPORTS` and `OTEL_METRIC_EXPORT_TIMEOUT` (only
the SDK's experimental async processors read them), and `OTEL_LOGS_*`
(logs stay on stdout).

**Cost and sampling.** In the 0.9 release measurements (one 4-core
machine, 50 clients, `scripts/measure.sh`, mean of three runs) a
connector served 2,074 DSP requests per second with 0.8.0, 1,909 with
0.9 and export off, 1,820 with every request traced and 1,701 with one
trace in ten kept. Runs vary by up to 15%, so these differences are
close to the noise. Sampling did **not** raise throughput: the spans
are made in the process either way. It lowers what is sent (about ten
times fewer trace exports) and the load on your backend. To keep one
trace in ten:

```sh
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1
```

Metrics are not sampled: they count every request, except the health
checks. With sampling, a log line can carry the trace id of a trace
that was not kept.

**What a trace shows.** One span per request served on the public and
the management port (`GET /dsp/2025-1/negotiations/{pid}`: the route
template, never the path with its ids), except the health checks, which
get no span; one per HTTP call sent (`POST`, with the host and port
called, never the URL); one per DSP outbox delivery
(`dsp.outbox.deliver`); and each negotiation and transfer state change
as a span event with both DSP process ids. Attributes are an allow-list:
method, route, status code, host and port, process ids and states;
never a token, an `Authorization` header, a key, a body or dataset
contents. The bridge from Rust's `tracing` also adds where in the code
the span was made (`code.*` attributes) and how long it was busy and
idle (`busy_ns`, `idle_ns`). Other log lines are not copied into
traces.

**Trace context stays inside the participant.** A `traceparent` header
(W3C Trace Context) is sent only on the connector's calls to its own
STS and to its own data sources (the dataset's `portitor:source`), and
read only on the identity component's `/sts/token`, and there only
after the client id and secret are checked. DSP and DCP calls to
other participants carry none, and one that arrives on the DSP port is
ignored: each side starts its own trace. To follow one negotiation
across two organisations, match the DSP process ids (`providerPid`,
`consumerPid`) in both sides' logs.

**Logs carry the trace id.** With export on, every JSON log line written
inside a span starts with `"trace_id":"<32 hex digits>"`, the
id of the trace it belongs to: search the logs for it. With export off,
log lines are as before.

**State changes in the log.** Each change is also one log line (INFO,
target `portitor::state`): `negotiation state changed` or `transfer
state changed`, with `portitor.dsp.process`, `portitor.dsp.role`,
`portitor.dsp.provider_pid`, `portitor.dsp.consumer_pid`,
`portitor.dsp.previous_state` (none for a new process) and
`portitor.dsp.state`.

**Metrics.** Names follow OpenTelemetry's
[naming rules](https://opentelemetry.io/docs/specs/semconv/general/naming/);
the two HTTP histograms are the
[HTTP semantic conventions'](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/),
with their advised buckets (5 ms to 10 s). A Prometheus receiver turns
dots into underscores and adds the unit, for example
`http_server_request_duration_seconds` and
`portitor_negotiation_ended_total`.

| Name | Instrument, unit | Attributes | Component |
|---|---|---|---|
| `http.server.request.duration` | histogram, `s` | `http.request.method`, `http.route` (when a route matched), `http.response.status_code`, `url.scheme` (`http`: TLS ends before the component), `network.protocol.version`, `error.type` (5xx). Health checks are not measured | all |
| `http.client.request.duration` | histogram, `s` | `http.request.method`, `server.address` and `server.port` (only for calls inside the participant: the own STS and data sources), `http.response.status_code` (when answered), `error.type` (4xx, 5xx, `timeout` or `_OTHER` when unanswered, or when the body of a 2xx answer cannot be read) | all |
| `portitor.negotiation.ended` | counter, `{negotiation}` | `portitor.dsp.role` (`provider`, `consumer`), `portitor.dsp.state` (`FINALIZED`, `TERMINATED`) | connector |
| `portitor.transfer.ended` | counter, `{transfer}` | `portitor.dsp.role`, `portitor.dsp.state` (`COMPLETED`, `TERMINATED`) | connector |
| `portitor.dsp.request.refused` | counter, `{request}` | `error.type`: `unauthenticated` (HTTP 404, or 401 on the catalog routes), `invalid_message` or `message_rules` (the bundle's message shapes; both HTTP 400) | connector |
| `portitor.outbox.message.resent` | counter, `{message}` | `portitor.outbox.name` (`dsp`, `credential_delivery`), and for `dsp` `portitor.dsp.process` (`negotiation`, `transfer`) and `portitor.dsp.role` | connector, issuer |
| `portitor.outbox.message.given_up` | counter, `{message}` | as above | connector, issuer |
| `portitor.harvest.run.ended` | counter, `{run}` | `portitor.harvest.kind` (`datasets`, `bundle`), `portitor.harvest.outcome` (`succeeded`, `partial`, `unchanged`, `failed`) | connector |
| `portitor.harvest.dataset.rejected` | counter, `{dataset}` | `portitor.harvest.check` (the check of the job log) | connector |
| `portitor.http.request.panicked` | counter, `{request}` | `http.route` (when a route matched). A request handler panicked (a bug); the request got HTTP 500 | all |
| `portitor.task.panicked` | counter, `{task}` | `portitor.task.name`: `outbox_delivery`, `dataset_harvest`, `bundle_harvest`, `identity_check` (connector), `certificate_watch`, `gaiax_schedule`, `gaiax_run` (identity), `issuer_outbox`, `credential_decision` (issuer) | all |
| `portitor.bundle.in_use` | gauge, `{bundle}` | `portitor.bundle.version`: 1 for the version in force, 0 for one replaced since the start | connector, with a bundle |
| `portitor.bundle.file.skipped` | gauge, `{file}` | none. Files of the bundle version in force that are skipped because this connector does not know their role; what they hold is not enforced. 0 when every file is used | connector, with a bundle |

A resend is a retry of a message, a message saved before a restart
and sent again, or a DSP message sent again after its sending
panicked; a give-up is a message the outbox stopped trying (the
negotiation or transfer is then terminated or not started, as the log
says; for the issuer, the delivery is `FAILED`). Every service also
reports its `service.name`, `service.version` and the SDK's resource
attributes.

### Backup and restore

The database is `/data/portitor.db` plus, while the connector runs,
`portitor.db-wal` and `portitor.db-shm`.

- **Simple and safe:** stop the container, copy all `portitor.db*` files,
  start it again.
- **Online:** from a separate container that has the `sqlite3` tool and
  mounts the same volume, run `sqlite3 /data/portitor.db ".backup '/backup/portitor-$(date +%F).db'"`.
  Do not copy only `portitor.db` while the connector runs.
- **Restore:** stop the container, replace the `portitor.db*` files with the
  backup (delete any old `-wal` and `-shm` files), start it.

Negotiations and transfers that were in progress at backup time continue
from their saved state.

### Rotating a key

The identity and the issuer each sign with one P-256 key. A new key
gets a new key id (its thumbprint). To change it without a gap, list
the old **public** key as retired for a while: what the old key signed
keeps verifying, and only the new key signs. EDC IdentityHub rotates
the same way: the old key stays listed until it is revoked
([EDC decision record 2024-09-02](https://github.com/eclipse-edc/IdentityHub/blob/main/docs/developer/decision-records/2024-09-02-resource-operations/README.md)).

1. Make the new key, as at the first deployment:
   `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out new-key.pem`.
2. Write the old public key to a file:
   `openssl pkey -in old-key.pem -pubout -out old-key.pub.pem`.
3. Mount `new-key.pem` as the key file and `old-key.pub.pem` read-only,
   and set `PORTITOR_IDENTITY_RETIRED_KEY_FILES` (or
   `PORTITOR_ISSUER_RETIRED_KEY_FILES`) to its path. With Gaia-X, also
   mount a certificate chain for the new key: the identity does not
   start with a chain for another key. Restart. The start log shows the
   new key id and one `retired key listed` line.
4. Destroy the old private key at once, with its backups. It is no
   longer needed: a retired key never signs, and the start fails if a
   retired file holds a private key.
5. Wait until everything the old key signed has expired:
   - identity: tokens and presentations (5 minutes), access tokens
     given to issuers with a credential request (30 days, or until the
     request is delivered). With Gaia-X, start a run at once
     (`POST /v1/gaiax/compliance`): Gaia-X services want the chain of
     the signing key, and the chain is published for the current key
     only. When the run succeeds, the Gaia-X credentials of the old key
     are replaced;
   - issuer: every credential it issued (their `validityDays`). Status
     lists are signed again with the new key at start. To end sooner,
     reissue the credentials with the new key.
6. Remove the file from the setting and restart. The old key id leaves
   the DID document; what the old key signed no longer verifies.

Verifiers cache DID documents (Portitor for
`PORTITOR_DID_CACHE_SECONDS`, 5 minutes by default; see
[DID document cache](#did-document-cache)), so a new key id may be
refused for that long after step 3 by a verifier that has not seen the
new document. Do step 3 when the component is quiet. For the same
reason a key removed in step 6 is still accepted for that long.

**A leaked or lost key.** Never list a leaked key as retired: anyone
with it could sign in your name while it is listed. Change the key
without a retired list; what the old key signed stops verifying, and
the issuer must reissue every credential. A lost issuer key (not
leaked) can be listed as retired, since only its public key is needed.

### Upgrade and rollback

1. Read the [CHANGELOG](../CHANGELOG.md) entries between your version and
   the new one.
2. Back up `/data`.
3. Replace the container with the new image. Database migrations run at
   start, forward only.
4. Check the start log and the readiness check on the management port
   (`GET /api/check/readiness` answers HTTP 200;
   [Health and logs](#health-and-logs)). `/health` only shows that the
   process runs.

Rollback: an older version refuses a database that a newer version
migrated (`database schema version N is newer than this Portitor`). Restore
the backup from step 2, then start the older image. What was written
after the backup is lost. This stays so in 1.x: migrations run forward
only, and no release migrates a database back (confirmed by the
maintainer at the 0.13 freeze, 2026-11-20;
[TD-22](RISKS_AND_TECHNICAL_DEBTS.md#td-22), [STABILITY](STABILITY.md#database)).

Messages the connector was sending when it stopped are saved and sent
again at start (ADR 0007), so an upgrade does not leave negotiations
waiting. A partner may receive such a message twice; that is handled.

#### From 0.4 to 0.5

0.5 authenticates every DSP request with DCP (ADR 0016). Before you
upgrade:

1. Set up the identity component and a credential for the participant
   ([section 4](#4-run-the-identity-next-to-the-connector)), and add the
   new connector settings (`PORTITOR_STS_URL`,
   `PORTITOR_STS_CLIENT_SECRET_FILE`, `PORTITOR_TRUSTED_ISSUERS`).
   Without them the 0.5 connector does not start. Your partners need the
   same, at the same time: a 0.4 connector cannot talk to a 0.5 one.
2. Finish or terminate the negotiations and transfers in progress. 0.4
   did not know who its callers were, so 0.5 cannot match them to the
   participant that started them: a provider answers that they do not
   exist, and a consumer refuses the provider's callbacks.
3. Agreements made by 0.4 name `urn:portitor:unauthenticated` as the
   consumer (`assignee`). 0.5 does not start transfers under them;
   negotiate them again after the upgrade.

The database needs no migration for this; a rollback to 0.4 works as
above.

#### From 0.5 to 0.6

0.6 adds the issuer, `portitor-issuer`
([section 7](#7-run-the-issuer-dataspace-authority-only)). Credentials
issued before stay valid. Before you upgrade:

1. Back up `/data/portitor-identity.db*`. The identity's database is
   migrated forward at start (credential requests get new columns), so a
   rollback of the identity to 0.5 needs that backup. The connector's
   database is unchanged.
2. `PORTITOR_POLICY_CREDENTIALS` is new and optional. Set it, on providers
   and consumers alike, only when offers use credential constraints
   (ADR 0019).
3. The test-only `test-issuer` tool is gone: get credentials from your
   dataspace's issuer ([section 4](#4-run-the-identity-next-to-the-connector),
   "Getting a credential").

#### From 0.6 to 0.7

0.7 harvests the datasets into a read copy (ADR 0020) and can check them
against the authority's signed bundle (ADR 0005).

1. Back up `/data/portitor.db*`. The connector's database is migrated
   forward at start (schema version 8: the read copy, the harvest
   jobs and the job kind), so a rollback to 0.6 needs that backup. The identity's and the
   issuer's databases are unchanged.
2. **Behaviour change:** a datasets file that cannot be read, or a
   dataset that breaks a rule (including an offer with a credential
   constraint on a type not in `PORTITOR_POLICY_CREDENTIALS`, or a
   `portitor:source` that is not an HTTP(S) URL), no longer stops the
   start. The dataset is rejected and logged, and the connector starts
   without it. To keep a strict start, check `GET /v1/harvest-jobs` in a
   start-up probe (latest `outcome` is `succeeded`).
   `PORTITOR_DATASETS` (removed in 0.2) still stops the start.
3. Datasets you remove from the file are withdrawn at the next run, not
   at the next restart. `PORTITOR_HARVEST_INTERVAL_SECONDS=0` keeps the
   0.6 rhythm (read only at start).
4. The bundle settings are new and optional. Without them nothing
   changes in what the catalog shows, except that a distribution now
   keeps its own metadata (`@id`, `dcat:accessURL`, other properties).
   With a bundle, each dataset needs an absolute IRI as `@id`, a
   `dct:accessRights` IRI the bundle has shapes for, and full IRIs for
   terms outside the DSP context; a dataset without them is rejected
   (logged, shown in the harvest job), the connector still starts.
   The verified bundle is kept in `/data/bundle-cache` (next to the
   database), so keep `/data` on a volume. `PORTITOR_BUNDLE_URL` is the
   authority's bundles folder, not a version folder, and new versions
   are taken without a restart (`PORTITOR_BUNDLE_VERSION` defaults to
   `latest`).
5. The issuer can serve bundles and list a bundle key
   (`PORTITOR_ISSUER_BUNDLE_DIR`, `PORTITOR_ISSUER_BUNDLE_KEY_FILE`, both
   optional; [section 7](#7-run-the-issuer-dataspace-authority-only)).

#### From 0.7 to 0.8

0.8 adds Gaia-X: the identity can get a Gaia-X compliance credential
(ADR 0021) and the connector can require one in a policy (ADR 0022).
Both are optional; without the new settings Portitor works as in 0.7.

1. Back up `/data/portitor-identity.db*`. On its first start, the 0.8
   identity migrates its database forward to schema version 3 (new
   table `gaiax_run`, the Gaia-X compliance runs). This happens with or
   without Gaia-X on. The connector's and the issuer's databases are
   unchanged.
2. Downgrade: a 0.7 identity refuses the migrated file (`database schema
   version 3 is newer than this Portitor (2)`) and does not start. To go
   back to 0.7, stop the identity, restore the backup from step 1 and
   start the 0.7 image. Credentials received after the backup are lost;
   ask the issuer again. Migrations only run forward; there is no way
   down without the backup. The connector and the issuer can go back to
   0.7 as they are.
3. **Behaviour change:** the DID documents of the identity and the issuer
   give their key `alg: ES256` (RFC 7517 section 4.4). DCP ignores it;
   nothing to do.
4. **Behaviour change:** in the presentation API a scope type `T` also
   matches a credential of type `gx:T` whose `@context` names a Gaia-X
   context. Plain types match as before.
5. New identity settings, all optional:
   `PORTITOR_IDENTITY_CERTIFICATE_CHAIN`, `PORTITOR_GAIAX_COMPLIANCE_URL`,
   `PORTITOR_GAIAX_COMPLIANCE_DID`, `PORTITOR_GAIAX_PARTICIPANT_FILE`
   ([Gaia-X onboarding](#gaia-x-onboarding-optional)). New connector
   setting, optional: `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`
   ([Gaia-X compliance](#gaia-x-compliance)). Set it on the provider
   before you publish an offer that requires Gaia-X compliance; without
   it the provider refuses every compliance credential (the start log
   warns when `PORTITOR_POLICY_CREDENTIALS` lists
   `ParticipantComplianceCredential` and no service is set).

#### From 0.8 to 0.9

0.9 adds health checks, traces and metrics
([ADR 0023](adr/0023-observability.md)). No database changes: a
rollback to 0.8 works as it is, but 0.8 has no `/api/check` routes, so
move your probes back to `/health` first.

1. **Behaviour change:** the management port is always bound (connector
   8081, identity 8091, issuer 8071; default address `127.0.0.1`), also
   without `PORTITOR_MANAGEMENT_API_KEY_FILE`. It serves the health
   checks without a key. On a shared host or pod, where that port is
   already taken, set `PORTITOR_MANAGEMENT_BIND` to a free one, or the
   component does not start. Never publish the port.
2. **Probes:** move Kubernetes probes from `GET /health` to the checks
   on the management port, with `PORTITOR_MANAGEMENT_BIND=0.0.0.0:<port>`
   ([Health and logs](#health-and-logs)).
3. **Docker `HEALTHCHECK`:** the images now check their own readiness,
   so `docker ps` shows `healthy` or `unhealthy`. Docker Swarm replaces
   an unhealthy container; plain `docker run` only shows it. A connector
   whose own identity or STS is down becomes unhealthy (after about
   20 s). The start interval of the check needs Docker Engine 25 or
   later.
4. **Telemetry:** export turns on if `OTEL_EXPORTER_OTLP_ENDPOINT` (or
   `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
   `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`) is already in the
   environment, for example set by the OpenTelemetry Operator. Unset it
   if you do not want export ([Traces and metrics](#traces-and-metrics)).
5. **More log lines:** each negotiation and transfer state change is one
   INFO line (target `portitor::state`). With export on, lines written
   inside a request carry its `trace_id`. A failed readiness check is
   logged when its result changes, not on every probe. The start line
   `management API listening` is now `management port listening`.
6. **Behaviour change:** `portitor-connector` and `portitor-identity`
   refuse any argument other than `health` (exit code 2); the issuer
   takes `bundle build` and `health`. An empty
   `PORTITOR_MANAGEMENT_API_KEY_FILE` (or one of only spaces) now
   counts as unset in all three components: the management API is off
   and the port serves only the health checks.

#### From 0.11 to 0.12

Back up the connector's database file before the upgrade: 0.12 adds
three indexes to it (schema version 11), and a 0.11 connector does not
open a file of a newer version. The identity's and the issuer's files
do not change.

1. **Issuer, breaking: `credentialSchema` in the definitions file.**
   Before you upgrade the issuer, add `"credentialSchema": "<URL>"` to
   every entry of the file `PORTITOR_ISSUER_CREDENTIALS_FILE` names: the
   absolute `http://` or `https://` URL of the JSON schema of that
   credential's `credentialSubject`. A 0.12 issuer does not start with
   an entry that lacks it; the line says which
   (`"<id>": credentialSchema is required`). The issuer only publishes
   the URL in its metadata, which DCP's schema requires; nothing fetches
   it ([section 7](#7-run-the-issuer-dataspace-authority-only),
   "Configuration"; ADR 0018, amended in 0.12). If your dataspace has no
   schema for a credential yet, agree on the URL where it will be
   published and use that. Credentials issued before stay valid, and
   participants change nothing.
2. **New setting, optional:** `PORTITOR_DID_CACHE_SECONDS` on the
   connector, the identity and the issuer. The default, 300 seconds, is
   what every version before did
   ([DID document cache](#did-document-cache)).
3. **Behaviour change, verifier and identity:** a credential is refused
   when the key that signed it is listed in its issuer's DID document
   under the id of another DID (DCP v1.0.1, 5.4.3, step 4; ADR 0016,
   amended in 0.12). Credentials from `portitor-issuer` and from EDC
   IdentityHub are signed with a key of the issuer's own DID, so they
   are not affected.
4. **Behaviour change, connector: policies are read strictly.** A
   policy term the connector does not read is refused by name, where
   0.11 dropped it without a word. At the first start of 0.12 every
   stored dataset is checked again. A dataset whose offer has a field
   the connector does not know (a `duty` inside a permission, a `unit`
   on a constraint, a misspelled key, `assigner` or `assignee`) leaves
   the catalog until its source is corrected; the harvest job says
   which field, under the check `odrl`. Check your datasets before the
   upgrade ([Known limits](#known-limits)). Agreements made before stay
   valid.
5. **Behaviour change, connector: 404 for a refused caller.** A caller
   that cannot be authenticated gets HTTP 404, not 401, on the
   negotiation and transfer routes; the catalog still answers 401, and
   503 is unchanged. If you monitor 401 answers, also watch the log
   line `DSP request refused: caller not authenticated` and its
   `status` field.
6. **Behaviour change, all three: a panic no longer stops the
   process.** The request that met the bug gets HTTP 500 and the rest
   go on. Watch the metrics `portitor.http.request.panicked` and
   `portitor.task.panicked`: each count is a bug to report. Keep the
   restart policy. The binaries are about 13% larger.
7. **Faster, nothing to do:** a transfer start no longer slows down as
   negotiations pile up, and small answers no longer wait 43 ms
   ([BENCHMARKS](BENCHMARKS.md)).

#### From 0.12 to 0.13

0.13 freezes the interface for 1.0 ([STABILITY](STABILITY.md)). Several
things changed before the freeze, because after 1.0 they could only
change in 2.0. The databases need no manual step. Change your
configuration and your management API clients **before** you start the
new images.

1. **Breaking, all three: three settings have new names.** A service
   that finds an old name set (to a value that is not empty) does not
   start; the log line `... stopped` names the new one, for example
   `PORTITOR_ALLOW_HTTP_CALLBACKS was renamed in 0.13; set
   PORTITOR_ALLOW_HTTP instead`. The meanings did not change.

   | Service | Old name | New name |
   |---|---|---|
   | connector, identity, issuer | `PORTITOR_ALLOW_HTTP_CALLBACKS` | `PORTITOR_ALLOW_HTTP` |
   | identity | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` |
   | issuer | `PORTITOR_ISSUER_CREDENTIALS_FILE` | `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` |

   `PORTITOR_STS_ALLOW_HTTP` is another setting and keeps its name.
2. **Breaking, connector management API: `counterPartyAddress` and
   `counterPartyId`.** `POST /v1/negotiations` takes them in place of
   `providerAddress` and `providerId`, and the answers of
   `/v1/negotiations` and `/v1/transfers` (start, one, list) return
   them under the new names. They are the names of Eclipse EDC and of
   the catalog calls. A request with an old name gets HTTP 400
   (`unknown field`). Change every client that starts a negotiation or
   reads these two fields. `providerPid` and `consumerPid` did not
   change.
3. **Breaking, connector: `PORTITOR_PARTICIPANT_ID` and
   `PORTITOR_PUBLIC_URL` are required.** Before, a connector without
   them started as `did:web:localhost%3A8080` at
   `http://localhost:8080`. Now it does not start
   (`PORTITOR_PARTICIPANT_ID: required ...`). A production connector
   set both already; check test and development setups.
4. **Behaviour change, all three: an empty value means "not set".**
   Spaces around a value are dropped, and a setting that is empty (or
   only spaces) counts as not set: a required one stops the start, an
   optional one takes its default. The identity and the issuer mostly
   did this already; the connector did not. What is different for a
   setting that is **set but empty**:

   | Setting | Before | From 0.13 |
   |---|---|---|
   | connector: `PORTITOR_PARTICIPANT_ID`, `PORTITOR_PUBLIC_URL` | an empty id or URL was used | the start stops (required) |
   | connector: `PORTITOR_BIND`, `PORTITOR_DB_FILE`, `PORTITOR_DATASETS_FILE` | an empty address or path was used (and failed, or gave an empty catalog) | the default (`0.0.0.0:8080`, `portitor.db`, no datasets file) |
   | connector: `PORTITOR_HARVEST_INTERVAL_SECONDS`, `PORTITOR_DATA_TOKEN_SECONDS` | the start stopped (invalid value) | the default |
   | connector: `PORTITOR_DCP_SCOPES` | the start stopped (`set but empty`) | the default scope |
   | connector: `PORTITOR_STS_CLIENT_SECRET_FILE` | the start stopped (cannot read the file) | the start stops (required) |
   | identity, issuer: `PORTITOR_BIND`, `PORTITOR_DB_FILE` | an empty address or path was used (and failed) | the default |
   | identity: `PORTITOR_SCOPE_TYPE_ALIASES` | no alias besides DCP's own | the default alias, `org.eclipse.edc.vc.type` |
   | issuer: `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` | the start stopped (invalid value) | the default, 10 |

   If you set `PORTITOR_SCOPE_TYPE_ALIASES` to an empty value to switch
   EDC's alias off, set it to `org.eclipse.dspace.dcp.vc.type` (DCP's
   own alias) instead. The content of a secret file is read as before.
   The connector also trims values now: a value with a space or a line
   end around it, which failed before, works.
5. **Bundle authorities, nothing to do:** a file whose role a connector
   does not know may now have any media type, not only the three
   ([Schema and vocabulary bundle](#schema-and-vocabulary-bundle)).
6. New in 0.13, nothing to do: the catalog calls of the management API
   ([README](../README.md#management-api)).
7. **Behaviour change, connector: `PORTITOR_PARTICIPANT_ID` and
   `PORTITOR_PUBLIC_URL` are checked at start**, as the identity's and
   the issuer's already were. The id must be a `did:web` DID; the URL an
   `http://` or `https://` URL without a query or fragment. A wrong
   value stops the start and the log names the setting. Before, such a
   connector started and its partners refused its messages.
8. **Behaviour change, all three: a switch is `true` or `false`.**
   `PORTITOR_ALLOW_HTTP` and `PORTITOR_STS_ALLOW_HTTP` with any other
   value (`1`, `yes`, `on`) stop the start; before, such a value was
   read as off without a word. Upper or lower case does not matter, and
   empty still means not set (off).
9. **Behaviour change, all three: a list that is set must name
   something.** A comma-separated setting whose value holds only commas
   and spaces (`","`) stops the start: `PORTITOR_TRUSTED_ISSUERS`,
   `PORTITOR_DCP_SCOPES`, `PORTITOR_POLICY_CREDENTIALS`,
   `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`, `PORTITOR_SCOPE_TYPE_ALIASES`,
   `PORTITOR_IDENTITY_RETIRED_KEY_FILES`,
   `PORTITOR_ISSUER_RETIRED_KEY_FILES`. Before, some of them read such a
   value as an empty list and some stopped. Leave a list unset (or
   empty) for its default. To switch EDC's scope alias off, see point 4.
10. **Behaviour change, connector, nothing to do:** a bundle file whose
    role the connector does not know is skipped; before, the whole
    bundle was refused. The connector does not enforce what such a file
    holds. Watch the new gauge `portitor.bundle.file.skipped` and the
    WARN line `bundle file skipped: ...`
    ([Schema and vocabulary bundle](#schema-and-vocabulary-bundle)).
11. **Behaviour change, connector as provider, nothing to do:** a
    transfer request from the agreement's consumer that arrives while
    the finalization event is still on its way is accepted; before, it
    got HTTP 400 (ADR 0007, amended;
    [TD-44](RISKS_AND_TECHNICAL_DEBTS.md#td-44)). A transfer request
    that is refused because it names no finalized agreement of the
    caller is now logged: `transfer request is refused; answered 400`
    (INFO).
12. The management OpenAPI files of the three services list HTTP 403 on
    every route that needs the API key. No service answers 403 in 0.13:
    it is reserved for 1.x (ADR 0025).
13. **Behaviour change, management API clients: an empty id is
    refused.** `datasetId` of `POST /v1/negotiations` (connector) and
    `holderPid` of `POST /v1/credential-requests` (identity) that are
    empty or only spaces answer 400. Leave `holderPid` out if the
    identity should make a new id; before, an empty one did that too.

Rollback to 0.12: put the old names back in the configuration and the
old field names in your clients; for the database, as above.

## 7. Run the issuer (dataspace authority only)

The issuer issues and revokes the dataspace's credentials with the
Decentralized Claims Protocol v1.0.1, section 6 (ADR 0018): participants'
identities ask it for credentials, it delivers them to their Storage API,
and verifiers read its status lists. **One issuer per dataspace, run by
the dataspace authority.** Participants do not run it; they only trust its
DID.

### Before the first deployment

- [ ] **Host.** Never on a participant's host, next to a connector or an
  identity: the issuer key admits anyone to the dataspace. Give it its
  own host or cluster and network zone, reached only through your HTTPS
  proxy.
- [ ] **DID.** A `did:web` DID for the issuer, for example
  `did:web:issuer.example.com`, whose document this component serves.
  Every participant lists it in `PORTITOR_TRUSTED_ISSUERS` (connector and
  identity).
- [ ] **Key.** One P-256 private key, unencrypted PKCS#8 PEM, as for the
  identity: `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out issuer-key.pem`.
  Mount it read-only. It signs every credential and status list of the
  dataspace: it is the dataspace's root of trust.
- [ ] **Key backup.** Keep an encrypted copy offline, apart from the
  database backups, with access for at least two people. Keep its public
  key too (`openssl pkey -in issuer-key.pem -pubout`). Without the key
  the issuer can neither issue nor revoke, so a lost key means a new key;
  list the old public key as retired and the issued credentials keep
  verifying ([Rotating a key](#rotating-a-key)). A leaked key lets anyone
  issue credentials in the dataspace's name: replace it at once, without
  listing it, and reissue every credential.
- [ ] **Credential definitions.** A JSON file listing what the issuer can
  issue (ADR 0018, "Credential definitions"), each with the URL of its
  schema (`credentialSchema`); the issuer checks it at start and refuses
  to start if it is wrong.
- [ ] **Management API key.** As for the connector. It registers
  participants and revokes credentials, so guard it like the issuer key.
- [ ] **Storage.** A persistent volume for `/data`: participants,
  requests, issued credentials, status lists and the outbox. Losing it
  loses the revocation state; back it up.
- [ ] **Network.** Inbound, through your proxy: the DID document, the
  issuer service and the status lists. Management port: internal only.
  Outbound: participants' DID documents and Storage APIs (`https://`).

### Configuration

| Variable | Default | Production value |
|---|---|---|
| `PORTITOR_PARTICIPANT_ID` | required | the issuer's `did:web` DID; its document is served at the path the DID gives |
| `PORTITOR_ISSUER_SERVICE_URL` | required | public base URL of the issuer service, for example `https://issuer.example.com/issuer`; written into the DID document (`IssuerService`). The status lists are served under it (`/status-lists/vc11`, `/status-lists/vc20`) and every credential points there, so **never change it** once credentials are issued |
| `PORTITOR_ISSUER_KEY_FILE` | required | path of the mounted PEM key |
| `PORTITOR_ISSUER_RETIRED_KEY_FILES` | unset (no retired key) | only during a key change: comma-separated paths of the old **public** keys, as for the identity ([Rotating a key](#rotating-a-key)). The start fails for a private key, a key that is not P-256, the current key, a key given twice, or the bundle key |
| `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` | required | path of the mounted credential definitions file |
| `PORTITOR_BIND` | `0.0.0.0:8070` | keep |
| `PORTITOR_DB_FILE` | `/data/portitor-issuer.db` | keep the image default |
| `PORTITOR_MANAGEMENT_API_KEY_FILE` | unset (management API off; the port serves only the health checks; an empty value counts as unset) | path of the mounted key file |
| `PORTITOR_MANAGEMENT_BIND` | `127.0.0.1:8071` | `0.0.0.0:8071` in a container, published only on an internal interface. Always bound, for the health checks |
| `PORTITOR_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | keep; the largest request body the public port takes (HTTP 413 above it; credential requests have a lower fixed limit, [section 5](#5-security)). From `1` to `67108864` (64 MiB), or the start stops |
| `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | keep; the same for the management port |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset (no traces or metrics) | your OTLP receiver's base URL, if you collect them ([Traces and metrics](#traces-and-metrics)) |
| `PORTITOR_ALLOW_HTTP` | unset | **unset**; `true` only for local tests with plain-HTTP DIDs |
| `PORTITOR_DID_CACHE_SECONDS` | `300` | keep; how long a holder's resolved DID document is reused, `0` to `3600`, `0` for no cache ([DID document cache](#did-document-cache)) |
| `PORTITOR_ISSUER_BUNDLE_KEY_FILE` | unset | with a bundle: the **public** bundle key as a JWK (as `bundle build` prints it), so the private key stays offline; a PKCS#8 PEM is accepted too. Listed in the DID document as `<did>#bundle-key` under `assertionMethod`. The start fails if it is the credential key |
| `PORTITOR_ISSUER_BUNDLE_DIR` | unset | with a bundle: the folder of built bundles, served read only at `GET /bundles/{version}/{path}` on the public port, and its `latest.json` at `GET /bundles/latest.json` |
| `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` | `10` | keep; the most credentials of one definition a participant may hold that are neither revoked nor expired. A request beyond it ends `REJECTED`. It keeps one participant from filling a status list (131,072 positions) |
| `RUST_LOG` | `info` | `info` |

The credential definitions file (ADR 0018):

```json
[
  {"id": "membership-vc20", "credentialType": "MembershipCredential",
   "credentialSchema": "https://issuer.example.com/schemas/membership-credential.json",
   "profile": "vc20-bssl/jwt", "validityDays": 365, "claims": ["role"]}
]
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | what holders ask for (it appears in the issuer metadata); must stay the same across restarts, and be unique in the file |
| `credentialType` | yes | the credential type, for example `MembershipCredential` |
| `credentialSchema` | yes, from 0.12 | the URL of the JSON schema of the credential's `credentialSubject` (DCP v1.0.1, 6.6.2): an absolute `http://` or `https://` URL. The issuer publishes it in its metadata and does nothing else with it: it does not fetch it and does not check credentials against it |
| `profile` | yes | `vc11-sl2021/jwt` or `vc20-bssl/jwt` |
| `validityDays` | yes | how long an issued credential is valid, `1` to `3650` |
| `claims` | no | the participant claims copied into `credentialSubject`; a claim the participant lacks is left out. Not `id` |

Any other field, a missing required one, or a value outside these rules
stops the start, with a line that names the entry, for example
`PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE: credential definitions:
"membership-vc20": credentialSchema is required (the URL of the
credential's schema)`.

The issuer metadata (`GET <issuer service>/metadata`) lists each entry
with `offerReason: "reissue"`. DCP's schema requires the property there;
it does not mean the issuer is offering anything (ADR 0018, amended in
0.12).

Management API (`Authorization: Bearer <key>`; described in
[api/issuer-management-openapi.json](api/issuer-management-openapi.json)):

| Call | Does |
|---|---|
| `POST /v1/participants` `{"did": ..., "claims": {...}}` | registers a participant (409 if registered) |
| `GET /v1/participants` | lists them |
| `DELETE /v1/participants/{did}` | removes a registration (the DID percent-encoded as a whole, for example `jq -rR @uri`); issued credentials stay valid until revoked |
| `POST /v1/offers` `{"participantDid": ..., "credentialIds": [...]}` | sends a `CredentialOfferMessage` to the participant's credential service |
| `GET /v1/credentials?holder={did}` | lists issued credentials, with their delivery state (`PENDING`, `DELIVERED`, `FAILED`) |
| `POST /v1/credentials/{id}/revoke` | revokes one: its bit is set and the status list signed again. Revoking again signs the list again, so after a `500` simply repeat the call |

Who gets what (ADR 0018, amended during the 0.6 review):

- Only registered participants may call the issuer service: anyone else
  gets `401`, before the issuer resolves its DID, and nothing is kept.
  A participant whose registration is deleted gets `401` from then on;
  a request it made before that is still waiting ends `REJECTED`.
- A request for an id not in the metadata, beyond
  `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER`, or when the profile's status
  list is full, ends `REJECTED`; nothing is sent to the holder, which
  reads the status.
- Each credential gets a random free position in its status list (W3C
  Bitstring Status List: "SHOULD assign indexes randomly").
- A delivery that fails for a passing reason (no answer, the holder's
  DID or credential service not found, HTTP 401, 408, 429 or 5xx) is
  retried, waiting 1, 2, 4 ... seconds up to an hour, for about 8 hours.
  Any other 4xx answer (400, 403, 404, ...) means the holder refuses the
  message: it is not retried. Either way it is then `FAILED` in the list
  and logged (`delivery given up`, or `delivery refused by the holder,
  given up`); the credentials stay issued. Send the participant an offer
  so that it asks again.
- A request whose decision failed (for example the disk was full) stays
  `RECEIVED` and is decided again within a minute.

### Schema and vocabulary bundle (optional)

The authority can publish a signed bundle of SHACL shapes and code lists
that connectors check datasets and DSP messages against (ADR 0005). A
bundle may also hold JSON-LD contexts (role `context`); connectors use them
only to read DSP messages whose `@context` names their URL, never in the
catalog.

1. Make a **bundle key**, apart from the issuer key:
   `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out bundle-key.pem`.
   Keep it offline, like the issuer key backup.
2. Put the files in a folder with a `bundle.json` that lists each file
   and its role (`datasets` with the `dct:accessRights` values it applies
   to, `messages`, `vocabulary`, `context` with its URL). The format is
   documented in `crates/portitor-issuer/src/bundle_build.rs`;
   [examples/bundle/healthdcat-ap-r8](../examples/bundle/healthdcat-ap-r8)
   is a full example. Code lists go in as N-Triples:
   `scripts/reduce_code_lists.py` keeps only the triples the shapes need.
3. Build and sign it, offline:
   ```sh
   portitor-issuer bundle build --in ./bundle-src --out ./bundles --version 2026-11 \
     --profile https://data.health.europa.eu/healthdcat-ap/releases/release-8/ \
     --authority did:web:issuer.example.com --kid 'did:web:issuer.example.com#bundle-key' \
     --key bundle-key.pem [--issued 2026-11-14T09:00:00Z]
   ```
   It refuses RDF/XML, paths that leave the folder, files over 64 MiB, and
   a version folder that exists already: a published version never
   changes. It also refuses a role it does not know (only `datasets`,
   `messages`, `vocabulary` and `context` in this version), so a
   misspelt role is never published. It checks its own result as a connector will, and prints the
   public key as a JWK. `--issued` is a UTC date and time
   (`YYYY-MM-DDThh:mm:ssZ`); without it, the time of the build is used. It also points `latest.json` at the new version,
   unless that file names a version issued later, or you pass
   `--not-latest` (to publish a version that connectors do not take
   yet).
4. Put the public JWK in `PORTITOR_ISSUER_BUNDLE_KEY_FILE` and the output
   folder in `PORTITOR_ISSUER_BUNDLE_DIR`, and restart the issuer. Check:
   `curl -sf https://issuer.example.com/bundles/latest.json`.
5. Tell participants the URL (`https://issuer.example.com/bundles/`) and
   your DID. A new version is a new folder; connectors that follow
   `latest` take it at their next check (within an hour by default),
   without a restart. They never go back to an older version, so to undo
   a version, publish a newer one.

The issuer serves only `manifest.json`, `manifest.jws` and the files the
version's manifest lists; anything else is 404.

Roles and older connectors (from 0.13; ADR 0005): a connector skips a
file whose role it does not know, with a warning, and uses the rest of
the bundle. So a later version of the issuer can add a new kind of file
without breaking the members that have not upgraded. **The limit: a
connector that skips a new kind of rule does not enforce it.** If every
member must enforce a new rule, wait until all have upgraded before you
rely on it. Keep at least one file with a role every member knows: a
connector refuses a bundle in which it knows no role. The issuer itself
is strict: it builds, signs and serves only roles it knows, so publish
a new role with the issuer version that has it.

### Ports

| Port | Serves | Reached by | Expose |
|---|---|---|---|
| 8070 | DID document, issuer metadata, credential requests and their status, status lists, bundles | participants' identities, verifiers, connectors | through your HTTPS reverse proxy |
| 8071 | health checks (`GET /api/check/...`, no key; readiness checks `database`, `key` and `status-lists`); management API (participants, offers, revocation) with its key | the authority's operators and probes | **never publicly** |

The public endpoints are described in
[api/issuer-openapi.json](api/issuer-openapi.json).

Check after start: `curl -sf https://issuer.example.com/.well-known/did.json`
and `curl -sf https://issuer.example.com/issuer/metadata`.

| Log message (level) | Meaning |
|---|---|
| `portitor-issuer listening` (INFO, at start) | Started; shows the DID, key id, issuer service URL and database |
| `retired key listed in the DID document; it does not sign` (INFO, at start) | One line per key in `PORTITOR_ISSUER_RETIRED_KEY_FILES`; shows its key id |
| `management port listening` (INFO, at start) | The management port's address (`bind`); always bound, for the health checks |
| `telemetry: exporting` (INFO, at start) | Traces and metrics are exported ([Traces and metrics](#traces-and-metrics)) |
| `credential offer sent` (INFO) / `credential offer not taken` (WARN) | An offer (`POST /v1/offers`) reached the holder, or not (the call answers 502) |
| `credential request refused` (INFO) | A credential request was malformed and answered 400; `reason` says why |
| `request refused` (INFO) | A caller's token was refused (401), for example a DID that is not registered |
| `delivery given up`, `delivery refused by the holder, given up` | See "Who gets what" above |
| `bundle key listed in the DID document` (INFO, at start) | `PORTITOR_ISSUER_BUNDLE_KEY_FILE` is set; shows the key id |
| `serving bundles at /bundles/` (INFO, at start) | `PORTITOR_ISSUER_BUNDLE_DIR` is set |
| `bundle file not readable`, `bundle manifest not readable` (WARN) | A request for a bundle file was answered 404 because the file or the version's manifest could not be read; check the folder |

### Order

1. Create the key, the credential definitions and the management key;
   start the issuer, check its readiness (`GET /api/check/readiness` on
   the management port answers HTTP 200) and that its DID document
   resolves.
2. Register each participant (its DID and claims) through the management
   API. A DID that is not registered gets `401` from the issuer service.
3. Give participants the issuer's DID; each adds it to
   `PORTITOR_TRUSTED_ISSUERS` and asks for its credentials (or you send
   it an offer).
4. When a participant leaves, revoke its credentials. Revocation cannot
   be undone.

Backup, restore, upgrade and rollback work as for the connector, with
`/data/portitor-issuer.db*`. Keep the key backup separate from them.

## Known limits

In this version (see also [SECURITY-ASSURANCE.md](SECURITY-ASSURANCE.md),
and [RISKS_AND_TECHNICAL_DEBTS.md](RISKS_AND_TECHNICAL_DEBTS.md) for every
known risk and technical debt, with its status and target release):

- Partners are admitted by one credential type (`MembershipCredential`,
  `PORTITOR_DCP_SCOPES`); offers may add credential claims (ADR 0019),
  but the catalog still shows every offer to every member. Credential
  schemas are not checked: the issuer publishes each definition's
  `credentialSchema` URL and no component fetches it.
- STS tokens are not cached: each DSP request costs one STS call, and each
  incoming one a presentation query.
- Status lists are reused for up to `PORTITOR_STATUS_LIST_CACHE_SECONDS`
  (default 300 seconds), so a revocation takes up to that long to be
  seen ([Status list cache](#status-list-cache)).
- One replica per database file; no PostgreSQL adapter yet.
- Internal callback addresses are not blocked.
- Policies: only `dateTime` and credential claims are understood;
  prohibitions and duties are carried into agreements but not enforced
  (ADR 0009). As consumer, the connector does not accept a provider's
  offer (DSP `OFFERED`) that has credential constraints; requests it
  sends itself are not affected.
- Policies are read strictly: a policy term the connector does not
  read is refused, never dropped
  ([ADR 0009](adr/0009-minimal-odrl-policies.md), amendment of
  2026-11-18). Examples: a `duty` inside a permission, `unit` on a
  constraint, a misspelled key. In your datasets file, that dataset is
  rejected (check `odrl`) and the others are harvested. In a partner's
  offer, contract request or agreement, the message is answered 400
  with the DSP error naming the field, and the negotiation does not
  move. Eclipse EDC ignores such fields, so an offer that EDC accepts
  can be refused here. The fields read are `@id`, `@type`, `target`,
  `assigner`, `assignee`, `profile`, `permission`, `prohibition` and
  `obligation` (and `timestamp` on an agreement); `action` and
  `constraint` in a rule; `leftOperand`, `operator` and `rightOperand`,
  or one of `and`, `andSequence`, `or`, `xone`, in a constraint. An
  offer in your datasets file has no `target`, `assigner` or `assignee`.
- Bundle: a file with a role this version does not know is skipped,
  so a rule of a later kind is not enforced until the connector is
  upgraded ([bundle](#schema-and-vocabulary-bundle)). The readiness
  check `bundle` does not show skipped files (it only passes or
  fails); the log, the gauge and the job log do.
- Negotiations start from the consumer only; a provider cannot start
  one with an offer ([TD-35](RISKS_AND_TECHNICAL_DEBTS.md#td-35)). As
  provider, a transfer request repeated with the same `consumerPid`
  gets the transfer it already has, not a second one (from 0.12;
  [TD-34](RISKS_AND_TECHNICAL_DEBTS.md#td-34)). As consumer, each
  `POST /v1/transfers` on the management API starts a new transfer.
- Reading a provider's catalog (`POST /v1/catalog/request` and
  `/v1/catalog/dataset/request` on the management API, from 0.13;
  [README](../README.md#management-api)): at most 16 MiB of answer are
  read, a fixed limit with no setting; a larger catalog gets HTTP 502.
  The request is sent once and waits up to 30 seconds. Of a catalog
  sent in pages (DSP 2025-1, "Pagination") only the first page is
  returned, with the provider's `Link` headers; the other pages are
  not read. No filter is sent: EDC's `querySpec` is refused with 400.
  A compressed answer (`Content-Encoding: gzip`, which the connector
  does not ask for) is not read and gets 502
  ([TD-38](RISKS_AND_TECHNICAL_DEBTS.md#td-38),
  [TD-36](RISKS_AND_TECHNICAL_DEBTS.md#td-36)).
- A panic (a bug) no longer stops the process (from 0.12): the request
  it happened in gets HTTP 500, a background task starts again after a
  pause (5 seconds, doubling up to 5 minutes while it keeps
  panicking), a DSP message whose sending panicked is sent again after
  the same pauses, and all are logged at ERROR and counted
  (`portitor.http.request.panicked`, `portitor.task.panicked`). Alert on
  these counts. Still run each container with a restart policy, as the
  `docker run` examples do: a failed start, a stopped management server
  and a kill for lack of memory end the process
  ([SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#what-a-panic-does),
  [TD-41](RISKS_AND_TECHNICAL_DEBTS.md#td-41)).
- Every request waits for its database write to reach the disk, one
  write at a time per database file. Measured for 0.12: with 100 pulls
  in flight, new negotiations did not slow the data plane beyond the
  agreed limit, at one per second or as fast as they go
  ([BENCHMARKS](BENCHMARKS.md#td-40-do-negotiations-slow-the-data-plane-down),
  [TD-40](RISKS_AND_TECHNICAL_DEBTS.md#td-40)).
- Each pull in flight uses memory in the connector and two connections
  (one from the consumer, one to your data source); see
  [BENCHMARKS](BENCHMARKS.md#data-plane) for the numbers before you
  plan for hundreds of pulls at once, and raise the open files limit
  (`ulimit -n`) above twice that number.
- No metrics scrape endpoint: metrics and traces are pushed over OTLP
  ([Traces and metrics](#traces-and-metrics)).
- Gaia-X: tested against a local Gaia-X compliance service in test mode
  (gx-compliance v2.16.0), not the Gaia-X Lab or a production clearing
  house. You get the registration number credential from a notary
  yourself; label levels 1 to 3, service offering credentials and
  OpenID4VC are not supported. The identity signs the legal person
  credential itself; another signer cannot be named. A Gaia-X compliance
  credential is checked
  between Portitor connectors only; EDC does not accept it (ADR 0022).
- Datasets come only from the datasets file; no other harvest source yet.
  No API writes datasets or policies.
- SHACL: the subset `portitor-shacl` implements (ADR 0005). Shapes that
  use anything else are refused when the bundle is loaded; `sh:extends`
  and `sh:shape` are ignored with a warning.
- TLS: the services speak plain HTTP; your reverse proxy ends TLS, and
  the management ports stay on a private network
  ([ADR 0026](adr/0026-tls-termination.md)).
- One connector serves one data space
  ([Two data spaces](#two-data-spaces)), and Portitor does not identify
  persons ([Users and roles](#users-and-roles)).
- Interoperability with Simpl-Open has not been tested
  ([ADR 0017](adr/0017-simpl-open.md)). What 1.0 keeps stable and what
  it does not do: [STABILITY](STABILITY.md).
- Transfers: pull only. A data access token ends `PORTITOR_DATA_TOKEN_SECONDS`
  after the transfer starts and is not renewed; the consumer then asks for
  a new transfer. Only `GET` of the whole source; no credentials are sent
  to it.
- Transfers as consumer: your own system fetches the data with the data
  address from the management API; the connector does not download it.
  The management API completes or terminates a transfer; suspend and start
  again are not offered yet.
