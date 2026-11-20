<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Portitor

A small and fast connector that implements the
Eclipse Dataspace Protocol (DSP) and the Decentralized Claims Protocol (DCP).
From 0.8 it can also get a participant's Gaia-X compliance credential and
require one in a policy (Gaia-X Trust Framework).

Credentials are system to system: each one is held by a participant, an
organisation with one DID, and checked between connectors. Portitor does
not identify the people or applications inside an organisation
([who holds the credentials](docs/ARCHITECTURE.md#who-holds-the-credentials)).

The name: in Latin a *portitor* is both the customs officer at a port, who
checks goods against the rules, and the ferryman who carries them across.
A connector does both: it checks every exchange against agreed policies and
carries the data to the other side. The project was called LDSC until 0.3
([ADR 0008](docs/adr/0008-project-name.md)).

[![Release](https://img.shields.io/github/v/release/brunopacheco1/portitor)](https://github.com/brunopacheco1/portitor/releases/latest)
[![CI](https://github.com/brunopacheco1/portitor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/ci.yml)
[![TCK](https://github.com/brunopacheco1/portitor/actions/workflows/tck.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/tck.yml)
[![CodeQL](https://github.com/brunopacheco1/portitor/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/brunopacheco1/portitor/badge)](https://scorecard.dev/viewer/?uri=github.com/brunopacheco1/portitor)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/16578/badge)](https://www.bestpractices.dev/projects/16578)
[![OpenSSF Baseline](https://www.bestpractices.dev/projects/16578/baseline)](https://www.bestpractices.dev/projects/16578)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![REUSE status](https://api.reuse.software/badge/github.com/brunopacheco1/portitor)](https://api.reuse.software/info/github.com/brunopacheco1/portitor)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](CODE_OF_CONDUCT.md)

Status: early prototype (0.x). Not for production use.

Website: <https://bruno.pacheco.lu/portitor/> (objectives, minimum functionality, components, roadmap).

- What it is and how it is built: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Plan: [docs/ROADMAP.md](docs/ROADMAP.md); decisions: [docs/adr/](docs/adr/)
- Known risks and technical debts: [docs/RISKS_AND_TECHNICAL_DEBTS.md](docs/RISKS_AND_TECHNICAL_DEBTS.md)
- Deploying and operating: [docs/DEPLOYING.md](docs/DEPLOYING.md); testing: [docs/TESTING.md](docs/TESTING.md)
- Trying Gaia-X compliance: [docs/TESTING.md](docs/TESTING.md#how-to-test-gaia-x-compliance)
- Reading the Rust code: [docs/RUST-IDIOMS.md](docs/RUST-IDIOMS.md)
- HTTP interfaces: [docs/api/](docs/api/) (OpenAPI 3.1: connector, management, identity, issuer)
- What 1.0 keeps stable, and what it does not do: [docs/STABILITY.md](docs/STABILITY.md)
- Security: [SECURITY.md](SECURITY.md), [docs/SECURITY-ASSURANCE.md](docs/SECURITY-ASSURANCE.md)
- Measured numbers (data plane, stored history, catalog, cost of DCP): [docs/BENCHMARKS.md](docs/BENCHMARKS.md)
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md), [GOVERNANCE.md](GOVERNANCE.md),
  [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)

## Interface

`portitor-connector` serves HTTP on `PORTITOR_BIND` (default `0.0.0.0:8080`).
The DSP endpoints follow the HTTPS bindings of
[DSP 2025-1](https://eclipse-dataspace-protocol-base.github.io/DataspaceProtocol/2025-1-err2/).

| Method and path | What it does |
|---|---|
| `GET /.well-known/dspace-version` | DSP versions this connector supports |
| `POST /dsp/2025-1/catalog/request` | Returns the catalog for a `CatalogRequestMessage` |
| `GET /dsp/2025-1/catalog/datasets/{id}` | Returns one dataset |
| `POST /dsp/2025-1/negotiations/request` | Starts a contract negotiation (provider side) |
| `GET /dsp/2025-1/negotiations/{providerPid}` | Returns the state of a negotiation |
| `POST /dsp/2025-1/negotiations/{providerPid}/request` | Counter-offer from the consumer |
| `POST /dsp/2025-1/negotiations/{providerPid}/events` | Consumer accepts an offer |
| `POST /dsp/2025-1/negotiations/{providerPid}/agreement/verification` | Consumer verifies the agreement |
| `POST /dsp/2025-1/negotiations/{providerPid}/termination` | Consumer ends a negotiation |
| `GET /dsp/2025-1/callback/negotiations/{consumerPid}` | Returns the state of a negotiation this connector started as consumer |
| `POST /dsp/2025-1/callback/negotiations/{consumerPid}/offers` | Provider sends an offer |
| `POST /dsp/2025-1/callback/negotiations/{consumerPid}/agreement` | Provider sends the agreement |
| `POST /dsp/2025-1/callback/negotiations/{consumerPid}/events` | Provider finalizes |
| `POST /dsp/2025-1/callback/negotiations/{consumerPid}/termination` | Provider ends a negotiation |
| `POST /dsp/2025-1/transfers/request` | Starts a transfer process (provider side) |
| `GET /dsp/2025-1/transfers/{providerPid}` | Returns the state of a transfer |
| `POST /dsp/2025-1/transfers/{providerPid}/start` | Consumer starts a suspended transfer again |
| `POST /dsp/2025-1/transfers/{providerPid}/suspension` | Consumer suspends a transfer |
| `POST /dsp/2025-1/transfers/{providerPid}/completion` | Consumer completes a transfer |
| `POST /dsp/2025-1/transfers/{providerPid}/termination` | Consumer ends a transfer |
| `GET /dsp/2025-1/callback/transfers/{consumerPid}` | Returns the state of a transfer this connector requested as consumer |
| `POST /dsp/2025-1/callback/transfers/{consumerPid}/start` | Provider starts a transfer (with the data address) or starts it again |
| `POST /dsp/2025-1/callback/transfers/{consumerPid}/suspension` | Provider suspends a transfer |
| `POST /dsp/2025-1/callback/transfers/{consumerPid}/completion` | Provider completes a transfer |
| `POST /dsp/2025-1/callback/transfers/{consumerPid}/termination` | Provider ends a transfer |
| `GET /data/{flowId}` | The data of a started transfer, for the bearer token in its data address |
| `GET /health` | Returns `ok` while the process runs |

A caller whose DCP token or credentials are refused gets a DSP error
without a reason: HTTP 404 on the negotiation and transfer routes, HTTP
401 on the catalog routes (from 0.12,
[ADR 0016](docs/adr/0016-dsp-caller-authentication.md)).

Configuration (environment variables). In all three services, spaces around a value are dropped and an empty
value means "not set": a required setting that is empty stops the start,
and an optional one takes its default.

| Variable | Default | Meaning |
|---|---|---|
| `PORTITOR_BIND` | `0.0.0.0:8080` | Address to listen on |
| `PORTITOR_PARTICIPANT_ID` | none (**required**) | Participant id (a DID), for example `did:web:connector.example.com`. The connector does not start without it |
| `PORTITOR_PUBLIC_URL` | none (**required**) | Public base URL, for example `https://connector.example.com`. The connector does not start without it; the DSP endpoint is this plus `/dsp/2025-1`, the consumer callback address this plus `/dsp/2025-1/callback` |
| `PORTITOR_DATASETS_FILE` | none (empty catalog) | JSON file with the datasets to offer, harvested into a read copy at start and at each run ([ADR 0020](docs/adr/0020-dataset-harvest.md)) |
| `PORTITOR_HARVEST_INTERVAL_SECONDS` | `3600` | Seconds between two harvest runs; `0` harvests only at start; at most `31536000` (one year) |
| `PORTITOR_BUNDLE_URL` | none (no SHACL checks) | `https://` URL of the authority's folder of signed schema and vocabulary bundles, one folder per version and `latest.json` ([ADR 0005](docs/adr/0005-schema-and-vocabulary-registry.md)); `http://` only with `PORTITOR_ALLOW_HTTP` |
| `PORTITOR_BUNDLE_DIR` | none | A local bundles folder instead of the URL (development and tests) |
| `PORTITOR_BUNDLE_VERSION` | `latest` | `latest` follows the newest version without a restart (never an older one); or one fixed version |
| `PORTITOR_BUNDLE_REFRESH_SECONDS` | `3600` | Seconds between two looks at `latest.json`; `0` only at start; at most `31536000` (one year) |
| `PORTITOR_BUNDLE_AUTHORITY` | none (**required** with a bundle) | The authority's `did:web` DID; its DID document lists the bundle key |
| `PORTITOR_BUNDLE_CACHE_DIR` | `bundle-cache` next to `PORTITOR_DB_FILE` (`/data/bundle-cache` in the image) | Where the verified bundle and the version in use are kept; the bundle is used only when the authority's host or DID document cannot be reached |
| `PORTITOR_DB_FILE` | `portitor.db` (`/data/portitor.db` in the image) | SQLite file that keeps negotiations; one connector per file |
| `PORTITOR_MANAGEMENT_API_KEY_FILE` | none (management API off) | File with the management API key (at least 32 characters, for example `openssl rand -hex 32`); an empty value counts as unset |
| `PORTITOR_MANAGEMENT_BIND` | `127.0.0.1:8081` | Address of the management port: always the health checks (`/api/check/...`, no key, [DEPLOYING](docs/DEPLOYING.md#health-and-logs)), and the management API with its key |
| `PORTITOR_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | Largest request body on the public port, `1` to `67108864` (64 MiB); larger gets HTTP 413. Small DSP messages have a lower fixed limit ([DEPLOYING](docs/DEPLOYING.md#5-security)) |
| `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | Largest request body on the management port, `1` to `67108864` (64 MiB) |
| `PORTITOR_ALLOW_HTTP` | `false` | `true` allows plain `http://` for consumers' callbacks, providers, partners' `did:web` DIDs and credential services, status lists and the STS; development and tests only |
| `PORTITOR_DATA_TOKEN_SECONDS` | `3600` | How long a data access token is valid after the transfer starts; `1` to `31536000` (one year) |
| `PORTITOR_STS_URL` | none (**required**) | Token endpoint of the participant's STS (`portitor-identity`: `<base URL>/sts/token`); every DSP request is authenticated with DCP (ADR 0016) |
| `PORTITOR_STS_ALLOW_HTTP` | unset | `true` allows a plain-HTTP `PORTITOR_STS_URL` (the STS on an internal network), and nothing else |
| `PORTITOR_STS_CLIENT_ID` | `PORTITOR_PARTICIPANT_ID` | Client id at the STS |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | none (**required**) | File with the STS client secret |
| `PORTITOR_TRUSTED_ISSUERS` | none (**required**) | Comma-separated DIDs trusted to issue credentials: the `MembershipCredential` and those of credential constraints |
| `PORTITOR_DCP_SCOPES` | `org.eclipse.dspace.dcp.vc.type:MembershipCredential` | Comma-separated scopes asked from callers' credential services, and granted to the counter-party in our tokens |
| `PORTITOR_POLICY_CREDENTIALS` | none (`MembershipCredential` only) | Comma-separated credential types that offers may constrain, each `Type` (scope `org.eclipse.dspace.dcp.vc.type:<Type>:read`) or `Type=<scope>` ([ADR 0019](docs/adr/0019-credential-constraints.md)) |
| `PORTITOR_STATUS_LIST_CACHE_SECONDS` | `300` | How long a verified status list credential is reused, in seconds; `0` fetches it on every check, at most `3600`. A revocation takes up to that long to be seen ([DEPLOYING](docs/DEPLOYING.md#status-list-cache)) |
| `PORTITOR_DID_CACHE_SECONDS` | `300` | How long a resolved DID document is reused, in seconds; `0` fetches it on every use, at most `3600`. A key removed from a document is still accepted for up to that long. The identity and the issuer read the same setting ([DEPLOYING](docs/DEPLOYING.md#did-document-cache)) |
| `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` | none (no Gaia-X compliance credential accepted) | Comma-separated `did:web` DIDs of the Gaia-X compliance services you trust, for example `did:web:compliance.lab.gaia-x.eu:v2`; separate from `PORTITOR_TRUSTED_ISSUERS` ([ADR 0022](docs/adr/0022-gaia-x-compliance-in-policies.md)) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | none (no traces or metrics) | Base URL of an OTLP/HTTP receiver: traces and metrics are sent there with OpenTelemetry. The other `OTEL_*` variables are in [DEPLOYING](docs/DEPLOYING.md#traces-and-metrics) ([ADR 0023](docs/adr/0023-observability.md)) |

As provider, the connector agrees to a request for an offer in its
catalog, then finalizes once the consumer verifies. It sends its messages
to the consumer's `callbackAddress`. It tries again up to 3 times when no
answer came, or the answer was 408, 429 or 5xx; any other answer ends the
tries. If the message did not arrive, it terminates the negotiation.

As consumer, the connector accepts the provider's offer and verifies its
agreement. Providers send their messages to the callback endpoints above.

Transfers ([ADR 0013](docs/adr/0013-transfer-and-data-plane.md)): pull
only (`HttpData-PULL`). Give each dataset you want to transfer its data
with `portitor:source`, an HTTP URL that the connector fetches for the
consumer. It is taken out of the dataset and never shown in the catalog:

```json
{ "@id": "weather-stations", "@type": "Dataset", "portitor:source": "https://data.internal.example/weather.csv", ... }
```

The connector accepts a transfer request under a finalized agreement held
by the caller, starts it at once and sends the consumer a data address:
`{PORTITOR_PUBLIC_URL}/data/{flowId}` with a bearer token. The token works
while the transfer is started and for `PORTITOR_DATA_TOKEN_SECONDS` after
the start; the connector then streams the source's answer (2xx only, with
its `Content-Type`). A request for a dataset without a source is refused.
A transfer request sent again by the same consumer with the same
`consumerPid` starts nothing new: the answer is the transfer it already
has (from 0.12).

Policies (minimal ODRL, [ADR 0009](docs/adr/0009-minimal-odrl-policies.md),
[ADR 0019](docs/adr/0019-credential-constraints.md)):
- A permission without constraints permits. Two kinds of constraint are
  understood:
  - `dateTime` (when the action is exercised) with `eq`, `neq`, `lt`,
    `lteq`, `gt` or `gteq`, and a date (`2030-12-31`, the whole day) or a
    date and time with a time zone (`2030-12-31T23:59:59Z`);
  - a credential claim, `<CredentialType>.<claim path>` (for example
    `MembershipCredential.role` or `DataProcessorCredential.level.name`),
    with `eq` or `neq` and a text, or `isAnyOf` or `isNoneOf` and a list
    of texts. The type must be in `PORTITOR_POLICY_CREDENTIALS` (or be
    `MembershipCredential`). It holds if one of the consumer's verified
    credentials of that type has the claim, as text, and the comparison is
    true; a missing claim never grants access.
  - Gaia-X compliance ([ADR 0022](docs/adr/0022-gaia-x-compliance-in-policies.md)):
    `ParticipantComplianceCredential.gx:labelLevel` with `eq` `SC` admits
    only consumers with a Gaia-X compliance credential of that label.
    List `ParticipantComplianceCredential` in
    `PORTITOR_POLICY_CREDENTIALS` and its service in
    `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`.

  `and`, `andSequence`, `or` and `xone` combine them.
- Anything else is refused (fail closed): a dataset whose offer uses it
  is rejected by the harvest and left out of the catalog.
- Policies are read strictly (from 0.12): a field Portitor does not
  read, for example a `duty` inside a permission or `unit` on a
  constraint, is an error that names it, never dropped. The harvest
  rejects such a dataset; a partner's message with such an offer or
  agreement gets HTTP 400. In a message, an offer's `assigner` must be
  the provider and its `assignee` the consumer, when present.
- As provider, the connector agrees only if the requested offer has the
  same terms as the catalog offer with that `@id`, and its constraints
  hold now for the consumer's credentials. For credential constraints it
  asks the consumer's credential service for the types the offer names.
  Otherwise it terminates the negotiation, saying why. It checks the
  agreement again, with fresh credentials, at each transfer request.
- As consumer, it grants the provider, in its token, the scopes the
  offer's credential constraints need. It accepts a provider's offer only
  if its constraints hold now (an offer with credential constraints is not
  accepted: the consumer cannot check its own credentials as the provider
  will), and verifies an agreement only if it has the terms it asked for
  or accepted.
- Prohibitions and duties are carried into the agreement but not enforced
  yet.

On both sides each outgoing message is saved before it is sent, and sent
again after a restart. A partner's message that arrives before its
acknowledgement of ours is recorded proves ours arrived, so it is
accepted at once. If a send fails, the connector asks the partner for its
state before giving up. See
[ADR 0007](docs/adr/0007-outbox-and-proof-of-delivery.md).

### Management API

Your own systems read a provider's catalog, and start and follow
negotiations and transfers as consumer, through the management API. It is not part of DSP, so it has its own port
(`PORTITOR_MANAGEMENT_BIND`) and is off unless `PORTITOR_MANAGEMENT_API_KEY_FILE`
is set. Every call needs `Authorization: Bearer <key>`, except
`/v1/openapi.json`, `/health` and the health checks. The port itself is
always bound, for the health checks.

The key is an opaque string, and so is the data plane token in a
transfer's `dataAddress`: a client must not parse them, and a 1.x
release may accept or issue other forms
([docs/STABILITY.md](docs/STABILITY.md)).

| Method and path | What it does |
|---|---|
| `POST /v1/catalog/request` | Returns a provider's catalog (DSP `Catalog`): the connector asks the provider with its own DCP token. The body is the `CatalogRequest` of Eclipse EDC's management API |
| `POST /v1/catalog/dataset/request` | Returns one dataset of a provider (DSP `Dataset`), by its `@id`; the body is EDC's `DatasetRequest` |
| `POST /v1/negotiations` | Starts a negotiation with a provider for an offer from its catalog |
| `GET /v1/negotiations/{consumerPid}` | Returns a negotiation; it holds the agreement once `state` is `FINALIZED` |
| `GET /v1/negotiations?after=&limit=` | Lists negotiations, up to 100 at a time |
| `POST /v1/transfers` | Requests a transfer under the agreement of one of our `FINALIZED` negotiations (`{"agreementId": ...}`; `format` defaults to `HttpData-PULL`) |
| `GET /v1/transfers/{consumerPid}` | Returns a transfer; once `state` is `STARTED` it holds the `dataAddress` to fetch the data from |
| `GET /v1/transfers?after=&limit=` | Lists transfers, up to 100 at a time |
| `POST /v1/transfers/{consumerPid}/completion` | Completes a started transfer (you have the data) |
| `POST /v1/transfers/{consumerPid}/termination` | Ends a transfer |
| `GET /v1/harvest-jobs?after=&limit=` | Lists the dataset harvest runs, newest first, up to 100 at a time (read only) |
| `GET /v1/harvest-jobs/{id}` | Returns one harvest run with the error of each rejected dataset |
| `GET /v1/openapi.json` | The OpenAPI description (no key needed); also in [docs/api/management-openapi.json](docs/api/management-openapi.json) |
| `GET /health` | Answers `ok` (no key needed) |
| `GET /api/check/liveness`, `/readiness`, `/startup`, `/health` | Health checks in Eclipse EDC's format (no key needed, also with the management API off; [DEPLOYING](docs/DEPLOYING.md#health-and-logs)) |

First read the provider's catalog. You need no token of your own: the
connector gets one from its STS, as for every DSP request it sends.

```sh
curl -s -X POST localhost:8081/v1/catalog/request \
  -H "authorization: Bearer $(cat key)" -H 'content-type: application/json' -d '{
  "counterPartyAddress": "https://provider.example/dsp/2025-1",
  "counterPartyId": "did:web:provider.example",
  "protocol": "dataspace-protocol-http:2025-1"}'
```

The answer is the provider's catalog as it came; each dataset lists its
offers in `hasPolicy`. The request body follows Eclipse EDC's management
API (`/v4/catalog/request`, EDC v1.0.0-RC8), so a client written for EDC
can send the same body:

| Member | What Portitor does with it |
|---|---|
| `counterPartyAddress`, `counterPartyId` | Required. The provider's DSP endpoint and its `did:web` DID, as in a negotiation |
| `protocol`, or `profile` (EDC's newer name) | Exactly one is required, and it must be `dataspace-protocol-http:2025-1`, the only protocol the connector speaks; anything else gets 400 |
| `additionalScopes` | Optional. Scopes added to the configured ones (`PORTITOR_DCP_SCOPES`) in the token for the provider, for datasets it shows only to holders of a credential |
| `querySpec` | Refused with 400. EDC sends it as the `filter` of the DSP request; DSP leaves its content to each implementation, and the connector always asks for the whole catalog |
| `@context`, `@type` | Optional. `@context` is not read; `@type`, if present, must be `CatalogRequest` (`DatasetRequest` for a dataset) |
| `@id` | For a dataset: required, the dataset's id. For a catalog: refused, like any other member not listed here |

If the provider refuses, cannot be reached or sends something else, the
answer is 502 and its `detail` says which (for example `the provider
answered HTTP 401 Unauthorized`). The request is sent once and waits up
to 30 seconds. At most 16 MiB of answer are read. If the provider sends
its catalog in pages (DSP 2025-1, catalog HTTPS binding, "Pagination"),
you get the first page only, with the provider's `Link` headers copied
to the answer; the other pages are not read.

Then negotiate, with an offer copied unchanged from the catalog (the
provider agrees only to the catalog's terms, see above):

```sh
curl -s -X POST localhost:8081/v1/negotiations \
  -H "authorization: Bearer $(cat key)" -H 'content-type: application/json' -d '{
  "counterPartyAddress": "https://provider.example/dsp/2025-1",
  "counterPartyId": "did:web:provider.example",
  "datasetId": "air-quality",
  "offer": {"@id": "air-quality:offer:until-2030", "@type": "Offer",
            "permission": [{"action": "use", "constraint": [{"leftOperand": "dateTime",
              "operator": "lteq", "rightOperand": "2030-12-31"}]}]}}'
```

Once the negotiation is `FINALIZED`, request a transfer under its
agreement, wait for `STARTED`, then fetch the data yourself with the token
from the data address (pull transfer,
[ADR 0014](docs/adr/0014-transfers-as-consumer.md)):

```sh
curl -s -X POST localhost:8081/v1/transfers \
  -H "authorization: Bearer $(cat key)" -H 'content-type: application/json' \
  -d '{"agreementId": "urn:uuid:..."}'
curl -s localhost:8081/v1/transfers/urn:uuid:... -H "authorization: Bearer $(cat key)"
# dataAddress.endpoint, and the "authorization" endpoint property as a bearer token:
curl -s https://provider.example/data/... -H "authorization: Bearer <token>"
curl -s -X POST localhost:8081/v1/transfers/urn:uuid:.../completion \
  -H "authorization: Bearer $(cat key)"
```

The image carries
Mozilla's CA certificate bundle (from Debian, MPL-2.0) to verify `https`
callback addresses.

To also trust a private CA (a private network, or a self-signed test
certificate), mount its PEM file into a folder and set `SSL_CERT_DIR` to
the image's folder plus yours, for example
`-v "$PWD/my-ca:/ca:ro" -e SSL_CERT_DIR=/etc/ssl/certs:/ca`. When
`SSL_CERT_DIR` or `SSL_CERT_FILE` is set, only those locations are used, so
keep `/etc/ssl/certs` in the list. The peer's certificate must still name
the host in the callback address; certificate checks cannot be turned off.

Known gaps in this version:
- Callers are admitted by one credential type, `MembershipCredential` from
  a trusted issuer (ADR 0016). Offers may also ask for credential claims
  (ADR 0019), compared as text only, but the catalog still shows every
  offer to every member.
- The callback address is chosen by the consumer. Only `https://` is
  allowed by default, but internal addresses are not blocked yet; put the
  connector where it cannot reach services it should not call.

The datasets file is a JSON array of DSP 2025-1 `Dataset` objects, as the
catalog serves them, without `@context`. Each dataset needs at least one
offer (`hasPolicy`) and one distribution with a `format`. Leave
`accessService` out: the connector sets it to its own data service. Other
properties, such as `dct:title` or `dcat:keyword`, are kept and served as
written (the DSP context defines the `dct`, `dcat` and `odrl` prefixes).
See [examples/datasets.json](examples/datasets.json).

The connector harvests the file into a read copy at start and every
`PORTITOR_HARVEST_INTERVAL_SECONDS` ([ADR 0020](docs/adr/0020-dataset-harvest.md)).
Each dataset is checked on its own; one that breaks a rule is rejected
and logged, its previous good version stays, and the connector keeps
running. A dataset removed from the file leaves the catalog, while
transfers under agreements already made still work. Each run is stored
and shown at `GET /v1/harvest-jobs`.

With the authority's signed schema and vocabulary bundle
(`PORTITOR_BUNDLE_*`, [ADR 0005](docs/adr/0005-schema-and-vocabulary-registry.md)),
each dataset is also checked against the SHACL shapes for its
`dct:accessRights` value, with Portitor's own SHACL Core engine
(`portitor-shacl`); a dataset that passes is served with
`"dct:conformsTo": {"@id": "<the bundle's profile>", "@type": "dct:Standard"}`.
Its `@id` must then be an absolute IRI, and every term outside the DSP
context's prefixes must be a full IRI (for example
`http://healthdataportal.eu/ns/health#healthCategory`): the catalog's
`@context` is always the DSP context alone. If the bundle has message
shapes, DSP negotiation and transfer messages are checked against them
too. HealthDCAT-AP release 8 is the baseline:
[examples/bundle/healthdcat-ap-r8](examples/bundle/healthdcat-ap-r8) holds
the sources of a demo bundle, `scripts/demo-bundle.sh` builds and signs
it, and [examples/healthdcat-datasets.json](examples/healthdcat-datasets.json)
has datasets written for it. See [DEPLOYING](docs/DEPLOYING.md#schema-and-vocabulary-bundle).

To see two participants agree a contract and transfer a file, run the demo
in Docker Compose (needs Docker with the Compose plugin, `curl`, `jq` and
`openssl`). The [quickstart](https://bruno.pacheco.lu/portitor/#try) on the
website walks through it: what to run, where to look, how to clean up.

```sh
scripts/demo-compose.sh
```

It starts the data space's issuer (`portitor-issuer`), a provider and a
consumer, each a connector with its identity component, and a small web
server that holds the provider's data ([examples/compose](examples/compose)).
The issuer also serves the HealthDCAT-AP demo bundle, which the script
builds and signs first; the provider pins it and offers
[examples/healthdcat-datasets.json](examples/healthdcat-datasets.json).
The script registers both participants at the issuer, and each identity
asks it for its credentials. Then it checks that the provider refuses a catalog request
without a DCP token, reads the catalog with a token from the consumer's
STS (the two valid datasets with `dct:conformsTo`; `bed-occupancy` is
missing, and the provider's harvest job shows its SHACL report), agrees a
contract, checks that changed terms are refused, checks offers that ask
for credential claims, transfers
[examples/data/hospital-admissions.csv](examples/data/hospital-admissions.csv),
fetches it with the token and completes the transfer, transfers the
restricted dataset to a member; last, once the issuer revokes the
consumer's `DataProcessorCredential`, a new transfer that needs it is
refused. It stops everything at the end;
`KEEP=1` leaves it running. `scripts/demo.sh` does the same without Docker
(needs Python 3 as well). To see the components' traces and metrics, add
the optional observability stack (OpenTelemetry Collector, Jaeger,
Prometheus): `EXTRA_COMPOSE=examples/compose/observability.yaml`, see
[TESTING](docs/TESTING.md#observability-stack-by-hand).

A connector does not run alone: it needs its participant's identity
component for DCP (`PORTITOR_STS_URL`, `PORTITOR_STS_CLIENT_SECRET_FILE`,
`PORTITOR_TRUSTED_ISSUERS`), and refuses to start without it.
[examples/compose/compose.yaml](examples/compose/compose.yaml) shows both
wired together; [docs/DEPLOYING.md](docs/DEPLOYING.md) is the runbook. The
identity component and the issuer (`portitor-issuer`, run once per
dataspace by its authority) have their own settings and APIs: see
DEPLOYING sections 4 and 7, and their OpenAPI files in
[docs/api/](docs/api/).

Gaia-X onboarding ([ADR 0021](docs/adr/0021-gaia-x-onboarding.md)) is
set on the identity component, not the connector:

| Variable (identity) | Default | Meaning |
|---------------------|---------|---------|
| `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` | none | PEM chain of the identity key, its certificate first; served at `/.well-known/x509CertificateChain.pem` (below the DID path, for a DID with one) and named by `x5u` in the DID document. The identity refuses to start if the first certificate is for another key |
| `PORTITOR_GAIAX_COMPLIANCE_URL` | none (Gaia-X off) | Base URL of a Gaia-X compliance service; the identity gets and renews its participant's compliance credential. Needs the chain, the participant file and `PORTITOR_GAIAX_COMPLIANCE_DID` |
| `PORTITOR_GAIAX_COMPLIANCE_DID` | none | The compliance service's `did:web` DID, for example `did:web:compliance.lab.gaia-x.eu:v2`. An answer signed by any other DID is refused |
| `PORTITOR_GAIAX_PARTICIPANT_FILE` | none | JSON file: Gaia-X context, legal person claims, terms and conditions hash, and the path of the notary's registration number credential (DEPLOYING, "Gaia-X onboarding") |

Licensed under [Apache-2.0](LICENSE) ([ADR 0010](docs/adr/0010-licence.md)).
