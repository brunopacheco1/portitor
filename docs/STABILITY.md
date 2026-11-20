<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# What 1.0 keeps stable

Release 0.13 freezes the interface of Portitor. 1.0 is the 0.13 release
candidate. From 1.0 on, Portitor follows
[Semantic Versioning](https://semver.org/): a 1.x release may add to what
is listed here, and only 2.0 may remove or rename it.

This file is the list. Tests compare it with the code
(`tests/frozen_interface.rs` in the three binaries' crates): a route or
a setting cannot leave the code while it is listed here, and a new one
cannot be added to the code without a line here. What each name means is
not repeated here; follow the links.

Between 0.13 and 1.0 the lists can still change if the release candidate
shows a mistake. The [CHANGELOG](../CHANGELOG.md) will say so.

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

A 1.x release may refuse internal network addresses on URLs that a
partner chooses, by default, with a setting to allow them (security
finding F-01 in [SECURITY-ASSURANCE](SECURITY-ASSURANCE.md),
[R-17](RISKS_AND_TECHNICAL_DEBTS.md#r-17)). 1.0 does not refuse them.

Each service needs its own host name (or its own port): the connector
serves `/dsp/2025-1`, `/data` and `/.well-known/dspace-version` at the
root of its host. The path of `PORTITOR_PUBLIC_URL` is not used for
routing; it is only put in front of the addresses the connector gives
to partners.

A management route may answer **HTTP 403** in 1.x, for a token that does
not grant the action. 1.0 never answers it: the API key grants every
right (ADR 0025, freeze point 2). `dataAddress` in the transfer view is
optional and stays optional: a 1.x release may leave it out for a caller
without the right to read data (freeze point 3).

## HTTP routes

The public routes are described in [docs/api/](api/) (`openapi.json`,
`identity-openapi.json`, `issuer-openapi.json`), the management routes in
the three `*management-openapi.json` files there. The README and
[DEPLOYING](DEPLOYING.md) say what each route does.

### portitor-connector, public port

`PORTITOR_BIND`. DSP 2025-1, the data plane and one open check.

```text
GET /.well-known/dspace-version
GET /health
GET /data/{flowId}
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
GET /v1/harvest-jobs
GET /v1/harvest-jobs/{id}
POST /v1/catalog/request
POST /v1/catalog/dataset/request
```

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
DELETE /v1/participants/{did}
POST /v1/offers
GET /v1/credentials
POST /v1/credentials/{id}/revoke
```

### Every binary, management port

The health checks and the two open routes. They need no key
([DEPLOYING, health](DEPLOYING.md#health-and-logs)).

```text
GET /api/check/liveness
GET /api/check/readiness
GET /api/check/startup
GET /api/check/health
GET /health
GET /v1/openapi.json
```

## Environment variables

Meaning and defaults: the [README](../README.md#interface) for the
connector, [DEPLOYING](DEPLOYING.md) section 4 for the identity and
section 7 for the issuer,
[Traces and metrics](DEPLOYING.md#traces-and-metrics) for `OTEL_*`.

One rule for every setting: spaces around a value are dropped, and an
empty value means "not set". A required setting that is empty stops the
start; an optional one takes its default.

### Settings of every binary

```text
PORTITOR_BIND
PORTITOR_PARTICIPANT_ID
PORTITOR_DB_FILE
PORTITOR_ALLOW_HTTP
PORTITOR_MANAGEMENT_BIND
PORTITOR_MANAGEMENT_API_KEY_FILE
PORTITOR_MAX_REQUEST_BYTES
PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES
PORTITOR_DID_CACHE_SECONDS
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
[Traces and metrics](DEPLOYING.md#traces-and-metrics) is for the SDK
version of each release), `RUST_LOG` (the log level) and `SSL_CERT_FILE`
and `SSL_CERT_DIR` (the CA certificates,
[Private CAs](DEPLOYING.md#private-cas)). A change to one of them is
named in the CHANGELOG.

## Files

| File | Set with | Format |
|---|---|---|
| Datasets file | `PORTITOR_DATASETS_FILE` | A JSON list of DCAT datasets with their ODRL offers and `portitor:source` ([ADR 0020](adr/0020-dataset-harvest.md), [DEPLOYING](DEPLOYING.md#dataset-harvest)). Policies are read strictly ([ADR 0009](adr/0009-minimal-odrl-policies.md)); a 1.x release may understand more terms |
| Credential definitions | `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` | A JSON list ([DEPLOYING, issuer configuration](DEPLOYING.md#configuration-1), `examples/issuer-credentials.json`) |
| Bundle | `PORTITOR_BUNDLE_URL`, `portitor-issuer bundle build` | A folder per version with `manifest.json` and `manifest.jws`, and `latest.json` ([ADR 0005](adr/0005-schema-and-vocabulary-registry.md)); the manifest follows [bundle-manifest.schema.json](schemas/bundle-manifest.schema.json) |
| Gaia-X participant file | `PORTITOR_GAIAX_PARTICIPANT_FILE` | JSON ([DEPLOYING, Gaia-X onboarding](DEPLOYING.md#gaia-x-onboarding-optional)) |
| Private keys | `PORTITOR_IDENTITY_KEY_FILE`, `PORTITOR_ISSUER_KEY_FILE` | One P-256 key, unencrypted PKCS#8 PEM |
| Retired keys | `PORTITOR_IDENTITY_RETIRED_KEY_FILES`, `PORTITOR_ISSUER_RETIRED_KEY_FILES` | Public keys, PEM |
| Certificate chain | `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` | PEM, the key's certificate first |
| Bundle key | `PORTITOR_ISSUER_BUNDLE_KEY_FILE` | The public key as a JWK, or a PKCS#8 PEM |
| Secrets | `PORTITOR_MANAGEMENT_API_KEY_FILE`, `PORTITOR_STS_CLIENT_SECRET_FILE` | Text; the API key has at least 32 characters |

## Database

Each component keeps its state in one SQLite file (`PORTITOR_DB_FILE`).
The tables are not an interface: read the state through the management
APIs.

Migrations run at start and **forward only**. An older release refuses a
database that a newer one migrated. To go back, restore the backup taken
before the upgrade; what was written since is lost. The maintainer
confirmed this for 1.x at the freeze (2026-11-20,
[TD-22](RISKS_AND_TECHNICAL_DEBTS.md#td-22);
[DEPLOYING, upgrade and rollback](DEPLOYING.md#upgrade-and-rollback)).

## Not part of the stable interface

- **Metric, span and log names.** [ADR 0023](adr/0023-observability.md)
  promises none of them. The metric names, their attributes, the span
  names, and the texts and fields of log lines can change in a 1.x
  release. Each change is named in the CHANGELOG, so check your alerts
  and dashboards when you upgrade. The two HTTP histograms follow
  OpenTelemetry's semantic conventions and change only with them.
- **The Rust crates.** They are not published as libraries
  (`publish = false` in every `Cargo.toml`); their APIs change without
  notice.
- **Names removed before 1.0.** A service that finds one of them set
  refuses to start and names what to set instead; the names themselves
  are not settings: `PORTITOR_DATASETS` (removed in 0.2),
  `PORTITOR_ALLOW_HTTP_CALLBACKS`, `PORTITOR_IDENTITY_CERTIFICATE_CHAIN`
  and `PORTITOR_ISSUER_CREDENTIALS_FILE` (renamed in 0.13;
  [DEPLOYING, from 0.12 to 0.13](DEPLOYING.md#from-012-to-013)). The
  freeze guard keeps them on its "not frozen" lists.
- **Development aids:** the `tck` build feature and its
  `PORTITOR_TCK_AUTHORIZATION` setting and `/tck/...` routes, `scripts/`,
  the `_http/` request collection, `examples/`, and any API browser
  (Swagger UI) of a development build.
- **Texts for people:** error messages, the `detail` of a problem
  answer, and the reasons in the harvest job log.
- **Limits and timings** that are not a setting (for example the number
  of retries, the 64 KiB limit of small messages, the request timeout).

## What 1.0 does not do

- **Pull transfers only** (`HttpData-PULL`). No push
  ([ADR 0013](adr/0013-transfer-and-data-plane.md)).
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
