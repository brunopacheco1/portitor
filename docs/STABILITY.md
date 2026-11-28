<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# What 1.0 keeps stable

Release 0.13 freezes the interface of Portitor. 1.0 is the 0.14 release
candidate, tried as 1.0.0-rc.1 and released as 1.0.0 on 2026-11-24
([ROADMAP](ROADMAP.md#10-stable-released-2026-11-24-as-v100)). From 1.0 on, Portitor follows
[Semantic Versioning](https://semver.org/): a 1.x release may add to what
is listed here, and only 2.0 may remove or rename it.

This file is the list. Tests compare it with the code
(`tests/frozen_interface.rs` in the three binaries' crates): a route or
a setting cannot leave the code while it is listed here, and a new one
cannot be added to the code without a line here. What each name means is
not repeated here; follow the links.

Between 0.13 and 1.0 the lists could still change if the release candidate
showed a mistake. None did: the lists of 1.0.0 are those of 0.13.0 and
0.14.0.

## Stable in 1.x

| What | Promise |
|---|---|
| HTTP routes | Each route below keeps its method and path, and what the OpenAPI files in [docs/api/](api/) say about it. A 1.x release may add routes and optional fields, so a client must ignore fields it does not know |
| Environment variables | Each variable below keeps its name and its meaning. A configuration that works in 1.0 works in 1.x |
| Files you write | The formats in [Files](#files) keep working |
| Database | A 1.x release migrates the database of an older release at start. See [Database](#database) |
| Command line | No arguments runs the service. `health` asks the running service's readiness check (all three binaries). `portitor-issuer bundle build` keeps its options ([DEPLOYING, bundle](DEPLOYING.md#schema-and-vocabulary-bundle-optional)) |
| Protocols | DSP 2025-1 and DCP v1.0.1 on the public ports, at the paths below |

Two values are **opaque**: the management API key and the data plane
token (the `authorization` value of a transfer's `dataAddress`). A client
must not parse them or rely on their length or alphabet; a 1.x release
may accept or issue other forms, for example a JWT
([ADR 0025](adr/0025-users-and-roles.md), freeze point 1).

Since 1.4 the three components can refuse internal network addresses
on URLs that a partner chooses (`PORTITOR_BLOCK_INTERNAL_ADDRESSES`,
with `PORTITOR_INTERNAL_ADDRESSES_ALLOWED` for the exceptions; security
finding F-01 in [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md),
[R-17](RISKS_AND_TECHNICAL_DEBTS.md#r-17)). The block is off by default
in 1.x, so a 1.0 configuration behaves the same; 2.0 turns it on by
default ([ADR 0016](adr/0016-dsp-caller-authentication.md), amended in
1.4).

Each service needs its own host name (or its own port): the connector
serves `/dsp/2025-1`, `/data`, `/push` (since 1.1) and
`/.well-known/dspace-version` at the root of its host. The path of `PORTITOR_PUBLIC_URL` is not used for
routing; it is only put in front of the addresses the connector gives
to partners.

A management route may answer **HTTP 403** in 1.x, for a token that does
not grant the action. 1.0 never answers it: the API key grants every
right (ADR 0025, freeze point 2). `dataAddress` in the transfer view is
optional and stays optional: a 1.x release may leave it out for a caller
without the right to read data (freeze point 3).

From 1.2 a route that changes a provider's transfer (the DSP transfer
messages, and `POST /v1/provided-transfers/{providerPid}/termination`)
may answer **HTTP 503** with `Retry-After` while another call is
changing that transfer; nothing was written, and the call may be made
again ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md), note on
the lease).

## HTTP routes

The public routes are described in [docs/api/](api/) (`openapi.json`,
`identity-openapi.json`, `issuer-openapi.json`), the management routes in
the three `*management-openapi.json` files there.
[DEPLOYING](DEPLOYING.md) says what each route does.

### portitor-connector, public port

`PORTITOR_BIND`. DSP 2025-1, the data plane and one open check.

```text
GET /.well-known/dspace-version
GET /health
GET /data/{flowId}
POST /push/{consumerPid}
PUT /push/{consumerPid}
POST /dsp/2025-1/catalog/request
GET /dsp/2025-1/catalog/datasets/{id}
POST /dsp/2025-1/negotiations/request
GET /dsp/2025-1/negotiations/{providerPid}
POST /dsp/2025-1/negotiations/{providerPid}/request
POST /dsp/2025-1/negotiations/{providerPid}/events
POST /dsp/2025-1/negotiations/{providerPid}/agreement/verification
POST /dsp/2025-1/negotiations/{providerPid}/termination
GET /dsp/2025-1/callback/negotiations/{consumerPid}
POST /dsp/2025-1/callback/negotiations/{consumerPid}/offers
POST /dsp/2025-1/callback/negotiations/{consumerPid}/agreement
POST /dsp/2025-1/callback/negotiations/{consumerPid}/events
POST /dsp/2025-1/callback/negotiations/{consumerPid}/termination
POST /dsp/2025-1/transfers/request
GET /dsp/2025-1/transfers/{providerPid}
POST /dsp/2025-1/transfers/{providerPid}/start
POST /dsp/2025-1/transfers/{providerPid}/suspension
POST /dsp/2025-1/transfers/{providerPid}/completion
POST /dsp/2025-1/transfers/{providerPid}/termination
GET /dsp/2025-1/callback/transfers/{consumerPid}
POST /dsp/2025-1/callback/transfers/{consumerPid}/start
POST /dsp/2025-1/callback/transfers/{consumerPid}/suspension
POST /dsp/2025-1/callback/transfers/{consumerPid}/completion
POST /dsp/2025-1/callback/transfers/{consumerPid}/termination
```

### portitor-connector, management port

`PORTITOR_MANAGEMENT_BIND`. These and the routes of
[every binary](#every-binary-management-port).

```text
GET /v1/negotiations
POST /v1/negotiations
GET /v1/negotiations/{consumerPid}
GET /v1/transfers
POST /v1/transfers
GET /v1/transfers/{consumerPid}
POST /v1/transfers/{consumerPid}/completion
POST /v1/transfers/{consumerPid}/termination
POST /v1/transfers/{consumerPid}/suspension
POST /v1/transfers/{consumerPid}/start
GET /v1/provided-transfers
GET /v1/provided-transfers/{providerPid}
POST /v1/provided-transfers/{providerPid}/termination
GET /v1/harvest-jobs
GET /v1/harvest-jobs/{id}
POST /v1/catalog/request
POST /v1/catalog/dataset/request
```

`POST /v1/catalog/request` returns the first page of the provider's
catalog and copies the provider's `Link` headers; it follows no link to
a next page. This stays so in 1.x: reading further pages for the caller
would be a new field or a new route, never a change to what this call
returns.

### portitor-identity, public port

`{credentialService}` is the path of `PORTITOR_CREDENTIAL_SERVICE_URL`.
The DID document, the certificate chain and the Gaia-X credentials are
under the path of the DID when the DID has one; the paths below are
those of a DID without a path.

```text
GET /.well-known/did.json
GET /.well-known/x509CertificateChain.pem
GET /credentials/gaiax/{name}
GET /health
POST /sts/token
POST {credentialService}/presentations/query
POST {credentialService}/credentials
POST {credentialService}/offers
```

### portitor-identity, management port

```text
GET /v1/credential-requests
POST /v1/credential-requests
GET /v1/credential-requests/{holderPid}
GET /v1/credentials
DELETE /v1/credentials/{id}
GET /v1/offers
POST /v1/offers/request
GET /v1/gaiax/compliance
POST /v1/gaiax/compliance
```

### portitor-issuer, public port

`{issuerService}` is the path of `PORTITOR_ISSUER_SERVICE_URL`. The two
`/bundles` routes are served only with `PORTITOR_ISSUER_BUNDLE_DIR`.

```text
GET /.well-known/did.json
GET /health
GET /bundles/latest.json
GET /bundles/{version}/{path}
GET {issuerService}/metadata
POST {issuerService}/credentials
GET {issuerService}/requests/{issuerPid}
GET {issuerService}/status-lists/{name}
```

### portitor-issuer, management port

```text
GET /v1/participants
POST /v1/participants
PUT /v1/participants/{did}
DELETE /v1/participants/{did}
POST /v1/offers
GET /v1/credentials
POST /v1/credentials/{id}/revoke
```

### Every binary, management port

The health checks and the two open routes. They need no key
([OPERATING, health](OPERATING.md#health-and-logs)).

```text
GET /api/check/liveness
GET /api/check/readiness
GET /api/check/startup
GET /api/check/health
GET /health
GET /v1/openapi.json
```

## Environment variables

Meaning and defaults, all in DEPLOYING:
[Settings of every component](DEPLOYING.md#settings-of-every-component),
[Connector settings](DEPLOYING.md#connector-settings),
[Identity settings](DEPLOYING.md#identity-settings) and
[Issuer settings](DEPLOYING.md#issuer-settings). The `OTEL_*` settings
are in [OPERATING, Traces and metrics](OPERATING.md#traces-and-metrics).

One rule for every setting: spaces around a value are dropped, and an
empty value means "not set". A required setting that is empty stops the
start; an optional one takes its default. The `OTEL_*` values are read
by the OpenTelemetry SDK as written: an empty endpoint also means "not
set", but spaces around a value are not dropped.

### Settings of every binary

```text
PORTITOR_BIND
PORTITOR_PARTICIPANT_ID
PORTITOR_DB_FILE
PORTITOR_DATABASE_URL
PORTITOR_DATABASE_PASSWORD_FILE
PORTITOR_ALLOW_HTTP
PORTITOR_MANAGEMENT_BIND
PORTITOR_MANAGEMENT_API_KEY_FILE
PORTITOR_MAX_REQUEST_BYTES
PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES
PORTITOR_DID_CACHE_SECONDS
PORTITOR_BLOCK_INTERNAL_ADDRESSES
PORTITOR_INTERNAL_ADDRESSES_ALLOWED
OTEL_EXPORTER_OTLP_ENDPOINT
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT
OTEL_EXPORTER_OTLP_METRICS_ENDPOINT
OTEL_SERVICE_NAME
```

### Settings of portitor-connector

```text
PORTITOR_PUBLIC_URL
PORTITOR_DATASETS_FILE
PORTITOR_HARVEST_INTERVAL_SECONDS
PORTITOR_DATA_TOKEN_SECONDS
PORTITOR_STS_URL
PORTITOR_STS_ALLOW_HTTP
PORTITOR_STS_CLIENT_ID
PORTITOR_STS_CLIENT_SECRET_FILE
PORTITOR_TRUSTED_ISSUERS
PORTITOR_TRUSTED_ISSUER_TYPES
PORTITOR_DCP_SCOPES
PORTITOR_POLICY_CREDENTIALS
PORTITOR_STATUS_LIST_CACHE_SECONDS
PORTITOR_GAIAX_COMPLIANCE_ISSUERS
PORTITOR_BUNDLE_URL
PORTITOR_BUNDLE_DIR
PORTITOR_BUNDLE_VERSION
PORTITOR_BUNDLE_REFRESH_SECONDS
PORTITOR_BUNDLE_AUTHORITY
PORTITOR_BUNDLE_CACHE_DIR
PORTITOR_PUSH_BUCKET
AWS_ENDPOINT_URL_S3
AWS_REGION
AWS_ACCESS_KEY_ID
AWS_SECRET_ACCESS_KEY
AWS_SESSION_TOKEN
```

### Settings of portitor-identity

```text
PORTITOR_CREDENTIAL_SERVICE_URL
PORTITOR_IDENTITY_KEY_FILE
PORTITOR_IDENTITY_RETIRED_KEY_FILES
PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE
PORTITOR_STS_CLIENT_ID
PORTITOR_STS_CLIENT_SECRET_FILE
PORTITOR_TRUSTED_ISSUERS
PORTITOR_TRUSTED_ISSUER_TYPES
PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS
PORTITOR_SCOPE_TYPE_ALIASES
PORTITOR_GAIAX_COMPLIANCE_URL
PORTITOR_GAIAX_COMPLIANCE_DID
PORTITOR_GAIAX_PARTICIPANT_FILE
```

### Settings of portitor-issuer

```text
PORTITOR_ISSUER_SERVICE_URL
PORTITOR_ISSUER_KEY_FILE
PORTITOR_ISSUER_RETIRED_KEY_FILES
PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE
PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER
PORTITOR_ISSUER_BUNDLE_KEY_FILE
PORTITOR_ISSUER_BUNDLE_DIR
```

### Variables read by libraries

Not in the lists above, because a library reads them and its release
decides their names: the other `OTEL_*` variables (the OpenTelemetry Rust
SDK; the table in
[Traces and metrics](OPERATING.md#traces-and-metrics) is for the SDK
version of each release), `RUST_LOG` (the log level) and `SSL_CERT_FILE`
and `SSL_CERT_DIR` (the CA certificates,
[Private CAs](DEPLOYING.md#private-cas)). A change to one of them is
named in the CHANGELOG.

## Files

| File | Set with | Format |
|---|---|---|
| Datasets file | `PORTITOR_DATASETS_FILE` | A JSON list of DCAT datasets with their ODRL offers and `portitor:source` ([ADR 0020](adr/0020-dataset-harvest.md), [DEPLOYING](DEPLOYING.md#dataset-harvest)). `portitor:source` is an `http://` or `https://` URL, or, since 1.1, an object of the connector's object storage written `s3://{bucket}/{key}` ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)). Since 1.4 a dataset may have the optional `portitor:accessPolicy`, who may see it in the catalog ([ADR 0019](adr/0019-credential-constraints.md), amendment in 1.4). Policies are read strictly ([ADR 0009](adr/0009-minimal-odrl-policies.md)); a 1.x release may understand more terms |
| Credential definitions | `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` | A JSON list ([DEPLOYING, credential definitions file](DEPLOYING.md#credential-definitions-file), `examples/issuer-credentials.json`) |
| Bundle | `PORTITOR_BUNDLE_URL`, `portitor-issuer bundle build` | A folder per version with `manifest.json` and `manifest.jws`, and `latest.json` ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)); the manifest follows [bundle-manifest.schema.json](schemas/bundle-manifest.schema.json) |
| Gaia-X participant file | `PORTITOR_GAIAX_PARTICIPANT_FILE` | JSON ([DEPLOYING, Gaia-X onboarding](DEPLOYING.md#gaia-x-onboarding-optional)) |
| Private keys | `PORTITOR_IDENTITY_KEY_FILE`, `PORTITOR_ISSUER_KEY_FILE` | One P-256 key, unencrypted PKCS#8 PEM |
| Retired keys | `PORTITOR_IDENTITY_RETIRED_KEY_FILES`, `PORTITOR_ISSUER_RETIRED_KEY_FILES` | Public keys, PEM |
| Certificate chain | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` | PEM, the key's certificate first |
| Bundle key | `PORTITOR_ISSUER_BUNDLE_KEY_FILE` | The public key as a JWK, or a PKCS#8 PEM |
| Secrets | `PORTITOR_MANAGEMENT_API_KEY_FILE`, `PORTITOR_STS_CLIENT_SECRET_FILE` | Text; the API key has at least 32 characters |

## Database

Each component keeps its state in a database: PostgreSQL (since 1.3),
or a SQLite file (`PORTITOR_DB_FILE`). The tables are not an
interface: read the state through the management APIs. Which
PostgreSQL versions are supported is in
[DEPLOYING, Database](DEPLOYING.md#database).

**SQLite is deprecated since 1.3** (maintainer, 2026-11-26;
[ADR 0029](adr/0029-several-replicas.md)). It keeps working in 1.x,
for one process per file, and `PORTITOR_DB_FILE` stays in the list
above until 2.0. 2.0 removes SQLite and `PORTITOR_DB_FILE`. No tool
copies a SQLite file into PostgreSQL (maintainer, 2026-11-27;
[OPERATING](OPERATING.md#from-sqlite-to-postgresql)).

Migrations run at start and **forward only**. An older release refuses a
database that a newer one migrated. What each release adds is in
[OPERATING](OPERATING.md#upgrade-and-rollback), for example
[From 1.3 to 1.4](OPERATING.md#from-13-to-14). To go back, restore the backup taken
before the upgrade; what was written since is lost. The maintainer
confirmed this for 1.x at the freeze (2026-11-20,
[TD-22](RISKS_AND_TECHNICAL_DEBTS.md#td-22);
[OPERATING, upgrade and rollback](OPERATING.md#upgrade-and-rollback)).

## Audit line

Since 1.1 (maintainer, 2026-11-25). Each data transfer writes one log
line at INFO with the message `data transfer done` and the target
`portitor::audit`
([OPERATING, Audit line](OPERATING.md#health-and-logs)). In 1.x the
message, the target, and these fields keep their names and meaning:
`flow` (the transfer's `providerPid`), `agreement`, `counterParty`,
`direction` (`sent` or `received`), `bytes` and `outcome` (`complete`,
`cut` or `failed`). A 1.x release may add fields and add values to
`outcome`; a reader must ignore fields it does not know.

## Not part of the stable interface

- **Metric, span and log names.** [ADR 0023](adr/0023-observability.md)
  promises none of them. The metric names, their attributes, the span
  names, and the texts and fields of log lines can change in a 1.x
  release. Each change is named in the CHANGELOG, so check your alerts
  and dashboards when you upgrade. One exception: the audit line
  (below). The two HTTP histograms follow
  OpenTelemetry's semantic conventions and change only with them.
- **The Rust crates.** They are not published as libraries
  (`publish = false` in every `Cargo.toml`); their APIs change without
  notice.
- **Names removed before 1.0.** A service that finds one of them set
  refuses to start and names what to set instead; the names themselves
  are not settings: `PORTITOR_DATASETS` (removed in 0.2),
  `PORTITOR_ALLOW_HTTP_CALLBACKS`, `PORTITOR_IDENTITY_CERTIFICATE_CHAIN`
  and `PORTITOR_ISSUER_CREDENTIALS_FILE` (renamed in 0.13;
  [OPERATING, from 0.12 to 0.13](OPERATING.md#from-012-to-013)). The
  freeze guard keeps them on its "not frozen" lists.
- **Development aids:** the `tck` build feature and its
  `PORTITOR_TCK_AUTHORIZATION` setting and `/tck/...` routes, `scripts/`,
  the `_http/` request collection, `examples/`, and any API browser
  (Swagger UI) of a development build.
- **Texts for people:** error messages, the `detail` of a problem
  answer, and the reasons in the harvest job log.
- **Limits and timings** that are not a setting (for example the number
  of retries, the 64 KiB limit of small messages, the request timeout,
  and for the catalog calls of the management API the 16 MiB limit of
  an answer, the 30 seconds a read may take and the number of reads at
  once).

## What 1.0 does not do

- **Pull transfers only** (`HttpData-PULL`) in 1.0. Push
  (`HttpData-PUSH`) exists since 1.1, as provider and as consumer, with
  S3-compatible object storage
  ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)).
- **No negotiation started by the provider.** The consumer starts
  ([TD-35](RISKS_AND_TECHNICAL_DEBTS.md#td-35)). It can be added in 1.x.
- **No Data Plane Signaling over HTTP.** The data plane runs inside the
  connector ([TD-17](RISKS_AND_TECHNICAL_DEBTS.md#td-17)). It can be
  added in 1.x.
- **One connector per data space**
  ([ADR 0024](adr/0024-several-data-spaces.md),
  [DEPLOYING, two data spaces](DEPLOYING.md#two-data-spaces)).
- **System to system only.** Portitor does not identify persons; users
  stop at your own reverse proxy
  ([ADR 0025](adr/0025-users-and-roles.md)).
- **No TLS in the services.** A reverse proxy ends TLS; the management
  ports stay on a private network
  ([ADR 0026](adr/0026-tls-termination.md)).
- **Interoperability with Simpl-Open has not been tested**
  ([ADR 0017](adr/0017-simpl-open.md)).

The other limits are in
[DEPLOYING, known limits](DEPLOYING.md#known-limits).
