<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Deploying Portitor

A runbook for the operations team that deploys Portitor. Running it
afterwards is in [OPERATING](OPERATING.md). This file follows the order
of a deployment:

| Step | Component | Who runs it | Needed? |
|---|---|---|---|
| 1 | [`portitor-issuer`](#step-1-issuer-dataspace-authority-only) | Only the dataspace authority, once per dataspace | A participant never runs it. It only trusts the issuer's DID |
| 2 | [`portitor-identity`](#step-2-identity-every-participant) | Every participant | Mandatory, unless the participant uses another DCP wallet ([ARCHITECTURE](ARCHITECTURE.md#components)). It holds the participant's key and credentials |
| 3 | [`portitor-connector`](#step-3-connector-every-participant) | Every participant | Mandatory. It does not start without its identity |

Each component has a management port for its operator. The health
checks on it need no key; every other call needs the management API
key. Set the key on all three: the connector needs it to show its
harvest jobs (provider) and to read catalogs, negotiate and transfer
(consumer). Never expose a management port to the Internet.

Optional inside a deployment: the authority's
[schema and vocabulary bundle](#schema-and-vocabulary-bundle),
Gaia-X ([onboarding](#gaia-x-onboarding-optional),
[compliance in policies](#gaia-x-compliance)) and
[traces and metrics](OPERATING.md#traces-and-metrics).

Portitor is at 0.x, the release candidate for 1.0. Read
[Known limits](#known-limits) before you expose it to partners. How to
run the demos on one machine is in [TESTING](TESTING.md#manual-tests),
not here.

## First deployment

The shortest path, in order. Each step links to its details. Example
host names: `issuer.example.com`, `identity.example.com`,
`connector.example.com`. TLS ends at your reverse proxy. Verify the
images and deploy them by digest
([rules for all three components](#rules-for-all-three-components)).
The "management key" of a component is the API key of its management
port; its readiness check is `GET /api/check/readiness` on that port.

**The dataspace authority, once.** It runs the dataspace's issuer and
decides its rules.

1. Decide the [values every participant must share](#what-the-dataspace-decides-first):
   the trusted issuer DIDs and the scope strings.
2. Deploy the [issuer](#step-1-issuer-dataspace-authority-only). Make
   its key, its [credential definitions file](#credential-definitions-file)
   and a management key ([checklist](#before-you-deploy-the-issuer)).
   Set, with values as in [Run the issuer](#run-the-issuer):
   `PORTITOR_PARTICIPANT_ID`, `PORTITOR_ISSUER_SERVICE_URL`,
   `PORTITOR_ISSUER_KEY_FILE`,
   `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE`,
   `PORTITOR_MANAGEMENT_API_KEY_FILE`, `PORTITOR_MANAGEMENT_BIND`.
3. Route `https://issuer.example.com` to port 8070. Check that
   `/.well-known/did.json` answers and that the readiness check passes.
4. Give participants the issuer's DID and the values of step 1.

**Each participant:**

5. Deploy the [identity](#step-2-identity-every-participant). Make its
   key, a management key and an STS client secret
   ([checklist](#before-you-deploy-the-identity)); the STS is the
   identity's token service, which only your connector calls. Set,
   with values as in [Run the identity](#run-the-identity):
   `PORTITOR_PARTICIPANT_ID`, `PORTITOR_IDENTITY_KEY_FILE`,
   `PORTITOR_CREDENTIAL_SERVICE_URL`, `PORTITOR_STS_CLIENT_SECRET_FILE`,
   `PORTITOR_TRUSTED_ISSUERS`, `PORTITOR_MANAGEMENT_API_KEY_FILE`,
   `PORTITOR_MANAGEMENT_BIND`.
6. Route `https://identity.example.com` to port 8090, except
   `/sts/token`, which stays on the internal network. Check that
   `/.well-known/did.json` answers.
7. Send the authority your DID and your claims, the values your
   credentials will carry (for example a role), out of band. The
   authority registers them: `POST /v1/participants` on the issuer's
   management port
   ([Registering participants and issuing](#registering-participants-and-issuing)).
8. Ask the issuer for your membership credential:
   `POST /v1/credential-requests` on the identity's management port
   ([Getting a credential](#getting-a-credential)). Wait until the
   request is `ISSUED`.
9. Write your [datasets file](#datasets-file), if you offer data.
10. Deploy the [connector](#step-3-connector-every-participant)
    ([checklist](#before-you-deploy-the-connector)). It must reach
    `https://identity.example.com`: it reads its own DID document
    there. Set, with values as in [Run the connector](#run-the-connector):
    `PORTITOR_PARTICIPANT_ID` (the identity's DID), `PORTITOR_PUBLIC_URL`,
    `PORTITOR_STS_URL` (with `PORTITOR_STS_ALLOW_HTTP=true` for an
    internal `http://` address), `PORTITOR_STS_CLIENT_SECRET_FILE` (the
    identity's file), `PORTITOR_TRUSTED_ISSUERS`,
    `PORTITOR_DATASETS_FILE`, `PORTITOR_MANAGEMENT_API_KEY_FILE`,
    `PORTITOR_MANAGEMENT_BIND`. If the dataspace decided other scopes
    than the default, `PORTITOR_DCP_SCOPES`.
11. Route `https://connector.example.com` to port 8080. Check
    `/.well-known/dspace-version`, the readiness check, and with the
    management key the first harvest job (`GET /v1/harvest-jobs`), the
    run that loads your datasets file into the catalog.

Every other setting has a default that works. Before you go live, read
[Security](#security) and set up [backups](OPERATING.md#backup-and-restore) and
[probes](OPERATING.md#health-and-logs).

## What the dataspace decides first

DSP 2025-1 says a dataspace is based on an agreed Governance Framework
and an agreed set of Policies. DCP v1.0.1 leaves some points to that
framework or to a dataspace profile, on purpose (DCP issues
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

## Rules for all three components

**Images.** Pick a release tag (`X.Y.Z`) from the
[releases](https://github.com/brunopacheco1/portitor/releases). Never
deploy `latest` or a branch build. Use the same version for all the
images you run. Verify each image
([RELEASING.md](RELEASING.md#checking-a-release)), then deploy by digest
(`ghcr.io/...@sha256:...`), not only by tag:

```sh
for image in portitor-issuer portitor-identity portitor-connector; do
  cosign verify "ghcr.io/brunopacheco1/$image:X.Y.Z" \
    --certificate-identity-regexp '^https://github.com/brunopacheco1/portitor/\.github/workflows/release\.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
  gh attestation verify "oci://ghcr.io/brunopacheco1/$image:X.Y.Z" --repo brunopacheco1/portitor
done
```

**Containers.**

- **One container per database file.** Never run two replicas on the
  same `/data` volume. Several replicas need the PostgreSQL adapter
  (planned, ADR 0006).
- Each image runs as user `65532`, has no shell and holds only the
  binary, the CA bundle and `/data`. Only `/data` needs to be writable:
  give it a persistent volume. Files you mount (keys, secrets, the
  datasets file) must be readable by user `65532`.
- `docker stop` (SIGTERM) stops a component cleanly; it stops taking
  requests and exits.
- Run each container with a restart policy, as the `docker run`
  examples do.
- Each binary takes no arguments, except `health`
  ([Health and logs](OPERATING.md#health-and-logs)) and, for the issuer,
  `bundle build`. Any other argument stops it with exit code 2.
- Kubernetes: one replica (`Deployment` with `strategy: Recreate`, or a
  `StatefulSet`), a `PersistentVolumeClaim` on `/data`, keys and secrets
  from `Secret`s mounted as files, and the
  [health checks](OPERATING.md#health-and-logs) as probes. Put only the public port
  in an `Ingress`, never the management port. There is no Helm chart
  yet.

**Ports and TLS.** Each component has a public port and a management
port. Both speak plain HTTP: TLS ends at your reverse proxy or load
balancer ([ADR 0026](adr/0026-tls-termination.md)). Each service needs
its own host name, or its own port ([STABILITY](STABILITY.md)).

| Component | Public port (through your HTTPS proxy) | Management port (internal only) |
|---|---|---|
| issuer | 8070 | 8071 |
| identity | 8090, except `/sts/token` (internal) | 8091 |
| connector | 8080 | 8081 |

The management port is always bound. It serves the health checks
without a key, and the management API with its key. **Never publish
it**: keep it on the host's loopback or an internal network, never on
the public proxy.

**Clock.** Tokens, credentials and offer constraints such as a date
limit are checked against the host clock (ADR 0009). Keep it
synchronised (NTP); a container uses the host's clock.

### Settings of every component

All configuration is by environment variable. For every `PORTITOR_*`
setting: spaces around a value are dropped, and an empty value means
"not set" (a required setting then stops the start, an optional one
takes its default). A switch is `true` or `false`, in any letter case. A
comma-separated list that is set must name something, and an item may
be at most 4096 bytes. A value that
breaks these rules, or is outside a setting's range, stops the start.

All three components read the settings below. Each component's own
settings are in its step.

| Variable | Default | Meaning |
|---|---|---|
| `PORTITOR_PARTICIPANT_ID` | none (**required**) | The component's `did:web` DID. The identity and the connector of one participant share it; the issuer has its own |
| `PORTITOR_BIND` | `0.0.0.0:8070` issuer, `0.0.0.0:8090` identity, `0.0.0.0:8080` connector | Address of the public port |
| `PORTITOR_MANAGEMENT_BIND` | `127.0.0.1:8071` issuer, `127.0.0.1:8091` identity, `127.0.0.1:8081` connector | Address of the management port |
| `PORTITOR_MANAGEMENT_API_KEY_FILE` | none (management API off) | File with the management API key, at least 32 characters (`openssl rand -hex 32`) |
| `PORTITOR_DB_FILE` | `/data/portitor-issuer.db`, `/data/portitor-identity.db`, `portitor.db` (connector; `/data/portitor.db` in its image) | The SQLite file; one process per file |
| `PORTITOR_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | Largest request body on the public port, `1` to `67108864` (64 MiB) |
| `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` | `2097152` (2 MiB) | The same for the management port |
| `PORTITOR_ALLOW_HTTP` | `false` | `true` allows plain `http://` for the addresses partners give. **Never in production** |
| `PORTITOR_DID_CACHE_SECONDS` | `300` | How long a resolved DID document is reused, `0` to `3600` ([DID document cache](#did-document-cache)) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | none (no traces or metrics) | Base URL of your OTLP/HTTP receiver ([OPERATING](OPERATING.md#traces-and-metrics), with the other `OTEL_*` variables) |
| `RUST_LOG` | `info` | Log level; `debug` only while diagnosing |
| `SSL_CERT_DIR`, `SSL_CERT_FILE` | the image's CA bundle | Only to trust a private CA ([Private CAs](#private-cas)) |

- Leave `PORTITOR_BIND`, `PORTITOR_DB_FILE` and the two size limits at
  their defaults.
- Management port: in a container set `0.0.0.0:<port>` and publish it
  only on an internal interface; the default is unreachable from outside
  the container. If the port is taken, the component does not start. A
  missing, unreadable or too short key file stops the start. Without a
  key the port serves only the health checks.
- A request body over the limit gets HTTP 413. Some routes have a lower
  fixed limit ([Security](#security)).
- `PORTITOR_ALLOW_HTTP` covers consumers' callbacks, providers,
  `did:web` DIDs, credential services, status lists, the STS and the
  bundle URL. It is for development and tests.

### Private CAs

Each image carries Mozilla's CA certificate bundle (from Debian,
MPL-2.0) to verify the `https://` addresses it calls. Partners' TLS
certificates must chain to a CA in that bundle, or to a private CA you
add.

To trust a private CA (for example on a private network) or a
self-signed certificate in a test environment, mount its PEM file into
a folder and set `SSL_CERT_DIR` to the image's folder plus yours, for
example `-v "$PWD/my-ca:/ca:ro" -e SSL_CERT_DIR=/etc/ssl/certs:/ca`.
When `SSL_CERT_DIR` or `SSL_CERT_FILE` is set, only those locations are
used, so keep `/etc/ssl/certs` in the list. The peer's certificate must
still name the host that is called. Certificate checks cannot be turned
off.

### DID document cache

Every component resolves other parties' `did:web` DIDs (the connector:
callers, credential issuers and the bundle authority; the identity:
callers and issuers; the issuer: holders). It reuses a fetched document
for `PORTITOR_DID_CACHE_SECONDS`, in memory. Set the same value on all
your components.

- **The trade-off.** A key that its owner removed, for example after a
  leak, is still accepted until the document is fetched again; a new
  key is refused for as long. With `0` a change is seen at once, at the
  cost of one fetch more per caller and per credential issuer on every
  DSP request, and no answer at all while a partner's DID host is down.
- At most 1,000 documents are kept. A lookup that failed is never
  kept. A restart empties the cache.
- A partner that must drop a leaked key at once should tell you; then
  restart your components, or set a low value.

## Step 1: issuer (dataspace authority only)

The issuer issues and revokes the dataspace's credentials with the
Decentralized Claims Protocol v1.0.1, section 6 (ADR 0018): participants'
identities ask it for credentials, it delivers them to their Storage API,
and verifiers read its status lists. **One issuer per dataspace, run by
the dataspace authority.** Participants do not run it; they only trust its
DID. A participant's operations team can skip to
[step 2](#step-2-identity-every-participant).

### Before you deploy the issuer

- [ ] **Host.** Never on a participant's host, next to a connector or an
  identity: the issuer key admits anyone to the dataspace. Give it its
  own host or cluster and network zone, reached only through your HTTPS
  proxy.
- [ ] **DID.** A `did:web` DID, for example
  `did:web:issuer.example.com`, whose document this component serves.
- [ ] **Key.** One P-256 private key, unencrypted PKCS#8 PEM, mounted
  read-only:
  `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out issuer-key.pem`.
  It signs every credential and status list of the dataspace: it is the
  dataspace's root of trust.
- [ ] **Key backup.** An encrypted copy offline, apart from the database
  backups, with access for at least two people. Keep the public key too
  (`openssl pkey -in issuer-key.pem -pubout`). Without the key the
  issuer can neither issue nor revoke. What to do with a lost or a
  leaked key is in
  [OPERATING](OPERATING.md#rotating-a-key).
- [ ] **Credential definitions.** The
  [file](#credential-definitions-file) that lists what the issuer can
  issue.
- [ ] **Management API key.** `openssl rand -hex 32`, mounted as a
  file. It registers participants and revokes credentials, so guard it
  like the issuer key.
- [ ] **Storage.** A persistent volume for `/data`: participants,
  requests, issued credentials, status lists and the outbox. Losing it
  loses the revocation state; back it up, apart from the key backup.
- [ ] **Network.** Inbound, through your proxy: the DID document, the
  issuer service, the status lists and the bundles. Outbound:
  participants' DID documents and Storage APIs (`https://`).

### Issuer settings

Besides the [settings of every component](#settings-of-every-component):

| Variable | Default | Meaning |
|---|---|---|
| `PORTITOR_ISSUER_SERVICE_URL` | none (**required**) | Public base URL of the issuer service, for example `https://issuer.example.com/issuer` |
| `PORTITOR_ISSUER_KEY_FILE` | none (**required**) | The PEM key |
| `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE` | none (**required**) | The [credential definitions file](#credential-definitions-file) |
| `PORTITOR_ISSUER_RETIRED_KEY_FILES` | none | Only during a key change: the old **public** keys, comma-separated paths ([Rotating a key](OPERATING.md#rotating-a-key)) |
| `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` | `10` | The most credentials of one definition a participant may hold that are neither revoked nor expired |
| `PORTITOR_ISSUER_BUNDLE_KEY_FILE` | none | With a bundle: the **public** bundle key, as a JWK or a PKCS#8 PEM |
| `PORTITOR_ISSUER_BUNDLE_DIR` | none | With a bundle: the folder of built bundles |

- The issuer service URL is written into the DID document
  (`IssuerService`). The status lists are served under it
  (`/status-lists/vc11`, `/status-lists/vc20`) and every credential
  points there, so **never change it** once credentials are issued.
- The DID document is served at the path the issuer's DID gives.
- A request beyond `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` ends
  `REJECTED`. The limit keeps one participant from filling a status
  list (131,072 positions).
- The bundle key is listed in the DID document as `<did>#bundle-key`
  under `assertionMethod`. Give the public JWK that `bundle build`
  prints, so the private key stays offline. The start fails if it is
  the credential key. The bundle folder is served read only at
  `GET /bundles/{version}/{path}`, and its `latest.json` at
  `GET /bundles/latest.json`.

### Credential definitions file

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
| `credentialSchema` | yes | the URL of the JSON schema of the credential's `credentialSubject` (DCP v1.0.1, 6.6.2): an absolute `http://` or `https://` URL. The issuer publishes it in its metadata and does nothing else with it: it does not fetch it and does not check credentials against it |
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
it does not mean the issuer is offering anything (ADR 0018).

### Run the issuer

Docker (keys and secrets as read-only mounts; the management port is
published on the host's loopback only):

```sh
docker run -d --name portitor-issuer --restart unless-stopped \
  -p 8070:8070 \
  -p 127.0.0.1:8071:8071 \
  -v portitor-issuer-data:/data \
  -v /srv/portitor/issuer-key.pem:/run/secrets/issuer-key.pem:ro \
  -v /srv/portitor/issuer-credentials.json:/issuer-credentials.json:ro \
  -v /srv/portitor/issuer-management-key:/run/secrets/issuer-management-key:ro \
  -e PORTITOR_PARTICIPANT_ID=did:web:issuer.example.com \
  -e PORTITOR_ISSUER_SERVICE_URL=https://issuer.example.com/issuer \
  -e PORTITOR_ISSUER_KEY_FILE=/run/secrets/issuer-key.pem \
  -e PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE=/issuer-credentials.json \
  -e PORTITOR_MANAGEMENT_API_KEY_FILE=/run/secrets/issuer-management-key \
  -e PORTITOR_MANAGEMENT_BIND=0.0.0.0:8071 \
  ghcr.io/brunopacheco1/portitor-issuer@sha256:<digest>
```

The public port (8070) serves the DID document, the issuer metadata,
credential requests and their status, the status lists and the bundles,
to participants' identities, verifiers and connectors
([api/issuer-openapi.json](api/issuer-openapi.json)). The
[rules for all three components](#rules-for-all-three-components) apply.

Check after start: `curl -sf https://issuer.example.com/.well-known/did.json`
and `curl -sf https://issuer.example.com/issuer/metadata`. The readiness
check (`GET /api/check/readiness` on the management port) answers HTTP
200; it checks `database`, `key` and `status-lists`.

### Registering participants and issuing

1. Register each participant (its DID and claims) through the management
   API. A DID that is not registered gets `401` from the issuer service.
2. Give participants the issuer's DID; each adds it to
   `PORTITOR_TRUSTED_ISSUERS` (identity and connector) and asks for its
   credentials (or you send it an offer).
3. When a participant leaves, revoke its credentials. Revocation cannot
   be undone.

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

Who gets what (ADR 0018):

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

Roles and older connectors (ADR 0005): a connector skips a
file whose role it does not know, with a warning, and uses the rest of
the bundle. So a later version of the issuer can add a new kind of file
without breaking the members that have not upgraded. **The limit: a
connector that skips a new kind of rule does not enforce it.** If every
member must enforce a new rule, wait until all have upgraded before you
rely on it. Keep at least one file with a role every member knows: a
connector refuses a bundle in which it knows no role. The issuer itself
is strict: it builds, signs and serves only roles it knows, so publish
a new role with the issuer version that has it.

### An EDC Issuer Service as the issuer

A dataspace can use the Issuer Service of EDC IdentityHub in place of,
or next to, `portitor-issuer`. The EDC interop test does this with
v1.0.0-RC8 ([TESTING](TESTING.md#a-second-issuer-the-edc-issuer-service)).
What to know:

- **Trust.** Each participant lists the Issuer Service's DID in
  `PORTITOR_TRUSTED_ISSUERS`, on the identity and the connector. That
  trusts it for every credential type. Its DID document must name an
  `IssuerService` endpoint: Portitor's identity finds it there.
- **Key.** Give the Issuer Service a P-256 key (ES256), or an RSA key
  for PS256. An EDC participant context that makes its own key gets
  Ed25519, which Portitor does not verify.
- **Request status.** With EDC v1.0.0-RC8,
  `GET /v1/credential-requests/{holderPid}` on Portitor's identity shows
  `issuerStatusError`: EDC answers the request with a `Location` that
  leaves out the path of its issuance API. This is expected. The
  credential arrives all the same.
- **Validity.** Issue credentials that are valid for much longer than
  seven days if EDC participants hold them: an EDC IdentityHub asks for
  a new credential every minute once the one it holds has less than
  seven days left.

## Step 2: identity (every participant)

**Mandatory for every participant.** The identity component holds the
participant's key and credentials and answers for it in the
Decentralized Claims Protocol v1.0.1 (ADR 0015): its DID document, a
Secure Token Service (STS) for the own connector, and the credential
service that other participants query. One process per participant.
Deploy it before the connector: the connector checks it at start.

### Before you deploy the identity

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
- [ ] **Management API key.** `openssl rand -hex 32`, kept in your
  secret store and mounted as a file. You need it to ask issuers for
  credentials and to manage credentials.
- [ ] **Storage.** A persistent volume for `/data` (credentials, SQLite).
- [ ] **Gaia-X** (optional): a certificate for the identity
  key, the notary's registration number credential and a participant
  file ([Gaia-X onboarding](#gaia-x-onboarding-optional)).
- [ ] **Network.** Inbound from other participants, through your proxy:
  the DID document and the credential service. Inbound from your own
  connector only: the STS (`/sts/token`). Outbound: other participants'
  and trusted issuers' DID documents (`https://`), and the issuers'
  `IssuerService` (metadata, credential requests, request status).

### Identity settings

Besides the [settings of every component](#settings-of-every-component):

| Variable | Default | Meaning |
|---|---|---|
| `PORTITOR_IDENTITY_KEY_FILE` | none (**required**) | The PEM key |
| `PORTITOR_CREDENTIAL_SERVICE_URL` | none (**required**) | Public `https://` base URL of the credential service, for example `https://identity.example.com/cs` |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | none (**required**) | The STS secret, at least 32 characters; the connector mounts the same file |
| `PORTITOR_STS_CLIENT_ID` | `PORTITOR_PARTICIPANT_ID` | The client id your connector sends |
| `PORTITOR_TRUSTED_ISSUERS` | none (**required**) | Comma-separated issuer DIDs |
| `PORTITOR_SCOPE_TYPE_ALIASES` | `org.eclipse.edc.vc.type` | Comma-separated scope aliases that mean "credentials of this type", besides DCP's `org.eclipse.dspace.dcp.vc.type` |
| `PORTITOR_IDENTITY_RETIRED_KEY_FILES` | none | Only during a key change: the old **public** keys, comma-separated paths ([Rotating a key](OPERATING.md#rotating-a-key)) |
| `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE` | none | Gaia-X only: the PEM chain of the identity key, the key's certificate first |
| `PORTITOR_GAIAX_COMPLIANCE_URL` | none (Gaia-X off) | Base URL of a Gaia-X compliance service, for example `https://compliance.lab.gaia-x.eu/v2` |
| `PORTITOR_GAIAX_COMPLIANCE_DID` | none | Gaia-X only: the `did:web` DID of that service, for example `did:web:compliance.lab.gaia-x.eu:v2` |
| `PORTITOR_GAIAX_PARTICIPANT_FILE` | none | Gaia-X only: the participant file |

- The credential service URL is written into the DID document; its path
  is where the component serves it (the example serves
  `/cs/presentations/query`, `/cs/credentials`, `/cs/offers`).
- Only trusted issuers may deliver credentials (Storage API) or offers,
  and nothing is sent to any other issuer.
- To switch EDC's scope alias off, set `PORTITOR_SCOPE_TYPE_ALIASES` to
  `org.eclipse.dspace.dcp.vc.type`.
- The Gaia-X settings go together
  ([Gaia-X onboarding](#gaia-x-onboarding-optional)):
  `PORTITOR_GAIAX_COMPLIANCE_URL` needs the chain, the participant file
  and `PORTITOR_GAIAX_COMPLIANCE_DID`.

### Identity endpoints

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
| management | `GET /api/check/liveness`, `/readiness`, `/startup`, `/health` | probes, no API key, also without the management API ([Health and logs](OPERATING.md#health-and-logs)). Readiness checks `database`, `key` and `did-document` |

The public endpoints are described in
[api/identity-openapi.json](api/identity-openapi.json).

### Run the identity

Docker (keys and secrets as read-only mounts):

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

The [rules for all three components](#rules-for-all-three-components)
apply. Kubernetes: only port 8090 in the `Ingress`; probes on port 8091
([Health and logs](OPERATING.md#health-and-logs)). The connector mounts the same STS
secret file.

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
     -d '{"issuerDid": "did:web:issuer.example.com", "credentialIds": ["membership-vc20"]}'
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
  -d '{"issuerDid": "did:web:issuer.example.com", "holderPid": "membership-2026"}'
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
a key change, run again ([Rotating a key](OPERATING.md#rotating-a-key)): the chain
is published for the current key only.

## Step 3: connector (every participant)

**Mandatory for every participant.** The connector serves the catalog,
negotiates contracts and transfers data (DSP 2025-1). It does not run
alone: every DSP request is authenticated with DCP, in both directions
(ADR 0016), so it needs its participant's identity component
([step 2](#step-2-identity-every-participant)) with a
`MembershipCredential` from your dataspace's issuer in it. The connector
checks at start that its `PORTITOR_PARTICIPANT_ID` is the identity's DID
and that the STS signs for it, and refuses to start otherwise.

### Before you deploy the connector

Check each item and keep the answers in your own runbook.

- [ ] **Participant id.** The identity's DID (`PORTITOR_PARTICIPANT_ID`).
- [ ] **Public URL.** The `https://` URL partners use to reach you
  (`PORTITOR_PUBLIC_URL`).
- [ ] **STS secret.** The file you made for the identity, mounted into
  the connector too.
- [ ] **Datasets file.** A JSON file with the datasets you offer
  ([Datasets file](#datasets-file)). A dataset that breaks a rule is
  rejected and left out of the catalog; the connector still starts.
  Test the file before a rollout and read the harvest job
  (`GET /v1/harvest-jobs`) after it.
- [ ] **Data sources.** For each dataset you transfer, the URL the
  connector fetches its data from (`portitor:source` in the datasets
  file). Partners never see it. It must be reachable from the connector,
  should be internal, and must not need credentials (none are sent).
  Plain `http://` is accepted here, since you set it; prefer `https://`.
  The harvest logs a warning for each dataset it adds or changes
  without a source.
- [ ] **Schema and vocabulary bundle** (optional). If your dataspace
  checks dataset metadata, its authority gives you the bundle's URL, the
  version to pin (if any) and its DID
  ([Schema and vocabulary bundle](#schema-and-vocabulary-bundle), ADR 0005).
- [ ] **Management API key.** Create a key (`openssl rand -hex 32`),
  keep it in your secret store, and mount it as a file
  (`PORTITOR_MANAGEMENT_API_KEY_FILE`). You need it to read the harvest
  jobs and, as consumer, to read catalogs, negotiate and transfer.
  Decide who may reach the management port; never expose it to the
  Internet.
- [ ] **Storage.** A persistent volume for `/data` (negotiations,
  transfers, the dataset copy, the bundle cache; SQLite).
- [ ] **Network.** Inbound: partners reach the DSP endpoints, including
  the consumer callbacks under `/dsp/2025-1/callback`, and the data
  plane (`/data`), through your proxy. Inbound from your probes and
  monitoring only: the management port. Outbound: its STS; its own
  identity's public URL (it reads its own DID document as partners do,
  at start and for the readiness check); partners'
  DID documents and credential services; the issuers' status list URLs;
  the callback addresses partners give it and, as consumer, the
  providers' DSP endpoints; your data sources; with a bundle, the
  authority's host; with telemetry on, your OTLP receiver. Allow only
  what it needs ([Security](#security)).

### Connector settings

Besides the [settings of every component](#settings-of-every-component):

| Variable | Default | Meaning |
|---|---|---|
| `PORTITOR_PUBLIC_URL` | none (**required**) | Your public base URL, for example `https://connector.example.com` |
| `PORTITOR_STS_URL` | none (**required**) | Your identity's STS token endpoint, for example `http://identity:8090/sts/token` |
| `PORTITOR_STS_ALLOW_HTTP` | `false` | `true` allows a plain `http://` `PORTITOR_STS_URL`, and nothing else |
| `PORTITOR_STS_CLIENT_SECRET_FILE` | none (**required**) | The STS secret, the same file as the identity's |
| `PORTITOR_STS_CLIENT_ID` | `PORTITOR_PARTICIPANT_ID` | Client id at the STS |
| `PORTITOR_TRUSTED_ISSUERS` | none (**required**) | Comma-separated DIDs trusted to issue credentials, usually the dataspace's `portitor-issuer` |
| `PORTITOR_DCP_SCOPES` | `org.eclipse.dspace.dcp.vc.type:MembershipCredential` | Comma-separated scopes asked from callers' credential services, and granted to the counter-party in our tokens |
| `PORTITOR_POLICY_CREDENTIALS` | none (`MembershipCredential` only) | Credential types that offers may constrain ([Credential constraints](#credential-constraints)) |
| `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` | none (no Gaia-X compliance credential accepted) | `did:web` DIDs of the Gaia-X compliance services you trust ([Gaia-X compliance](#gaia-x-compliance)) |
| `PORTITOR_STATUS_LIST_CACHE_SECONDS` | `300` | How long a verified status list is reused, `0` to `3600` ([Status list cache](#status-list-cache)) |
| `PORTITOR_DATASETS_FILE` | none (empty catalog) | The [datasets file](#datasets-file) |
| `PORTITOR_HARVEST_INTERVAL_SECONDS` | `3600` | Seconds between two harvest runs, `0` (only at start) to `31536000` (one year) |
| `PORTITOR_DATA_TOKEN_SECONDS` | `3600` | How long a data access token is valid after the transfer starts, `1` to `31536000` |
| `PORTITOR_BUNDLE_URL` | none (no SHACL checks) | `https://` URL of the authority's bundles folder, for example `https://issuer.example.com/bundles/` |
| `PORTITOR_BUNDLE_AUTHORITY` | none (**required** with a bundle) | The authority's `did:web` DID |
| `PORTITOR_BUNDLE_VERSION` | `latest` | `latest` follows the newest version; or one fixed version |
| `PORTITOR_BUNDLE_REFRESH_SECONDS` | `3600` | Seconds between two looks at `latest.json`, `0` (only at start) to `31536000`; not used with a fixed version |
| `PORTITOR_BUNDLE_CACHE_DIR` | `bundle-cache` next to `PORTITOR_DB_FILE` (`/data/bundle-cache` in the image) | Where the verified bundle and the version in use are kept |
| `PORTITOR_BUNDLE_DIR` | none | A local bundles folder instead of the URL, not together with `PORTITOR_BUNDLE_URL`; development and tests only |
| `AWS_ENDPOINT_URL_S3` | AWS S3 of `AWS_REGION` | Since 1.1. Base URL of your S3-compatible object storage, for example `http://rustfs:9000` ([Object storage and push](#object-storage-and-push-transfers)) |
| `AWS_REGION` | `us-east-1` | Since 1.1. Region in the request signature |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | none (no object storage) | Since 1.1. The object storage keys; both or neither. Checked only with `PORTITOR_PUSH_BUCKET` set |
| `AWS_SESSION_TOKEN` | none | Since 1.1. The session token of temporary keys (AWS STS), sent and signed as `x-amz-security-token` |
| `PORTITOR_PUSH_BUCKET` | none (push transfers as consumer refused) | Since 1.1. Bucket where data pushed to you lands; needs the keys |

- `PORTITOR_PUBLIC_URL` has no query or fragment. The DSP endpoint is
  this plus `/dsp/2025-1`, the consumer callback address this plus
  `/dsp/2025-1/callback`.
- `PORTITOR_STS_URL` is `<identity base URL>/sts/token`, on the internal
  network. The connector refuses to start with an `http://` STS URL
  unless `PORTITOR_STS_ALLOW_HTTP` or `PORTITOR_ALLOW_HTTP` is `true`.
- Trusted issuers issue the `MembershipCredential` and the credentials
  of credential constraints.
- Each entry of `PORTITOR_DCP_SCOPES` must be a DCP scope
  (`org.eclipse.dspace.dcp.vc.type:<Type>[:access]` or
  `org.eclipse.dspace.dcp.vc.id:<id>`). The type `VerifiableCredential`
  is refused, as it would let a partner read every credential.
- A bundle setting without `PORTITOR_BUNDLE_URL` or
  `PORTITOR_BUNDLE_DIR` stops the start.
- Leave the three time settings and the cache folder at their defaults,
  unless consumers need longer than an hour to fetch.

### Run the connector

Docker (the [rules for all three components](#rules-for-all-three-components)
apply; the management port is published on the host's loopback only):

```sh
docker run -d --name portitor-connector --restart unless-stopped \
  -p 8080:8080 \
  -p 127.0.0.1:8081:8081 \
  -v /srv/portitor/datasets.json:/datasets.json:ro \
  -v portitor-data:/data \
  -e PORTITOR_PARTICIPANT_ID=did:web:identity.example.com \
  -e PORTITOR_PUBLIC_URL=https://connector.example.com \
  -e PORTITOR_DATASETS_FILE=/datasets.json \
  -v /srv/portitor/sts-client-secret:/run/secrets/sts-client-secret:ro \
  -e PORTITOR_STS_URL=http://identity:8090/sts/token -e PORTITOR_STS_ALLOW_HTTP=true \
  -e PORTITOR_STS_CLIENT_SECRET_FILE=/run/secrets/sts-client-secret \
  -e PORTITOR_TRUSTED_ISSUERS=did:web:issuer.example.com \
  -v /srv/portitor/management-key:/run/secrets/portitor-key:ro \
  -e PORTITOR_MANAGEMENT_API_KEY_FILE=/run/secrets/portitor-key \
  -e PORTITOR_MANAGEMENT_BIND=0.0.0.0:8081 \
  ghcr.io/brunopacheco1/portitor-connector@sha256:<digest>
```

The STS lines point at the identity component of
[step 2](#step-2-identity-every-participant), reached on an internal
network as `identity`; without them the connector does not start.

The management lines: without `PORTITOR_MANAGEMENT_BIND` the port
cannot be reached from outside the container, and without the key it
serves only the health checks.

Check after start, the last two on the host:

```sh
curl -sf https://connector.example.com/health          # "ok"
curl -s  https://connector.example.com/.well-known/dspace-version
curl -sf localhost:8081/api/check/readiness
curl -s  localhost:8081/v1/harvest-jobs \
  -H "authorization: Bearer $(cat /srv/portitor/management-key)"
```

The log line `portitor-connector listening` shows the bind address, the number
of datasets in the catalog after the start harvest, and the database file.
`catalog ready` comes just before it. `management API off` means no key
file is set (or the variable is empty): the management port then serves
only the health checks. A missing, unreadable or too short key file
stops the start with an error.

### What the connector does

The DSP endpoints follow the HTTPS bindings of
[DSP 2025-1](https://eclipse-dataspace-protocol-base.github.io/DataspaceProtocol/2025-1-err2/).
Every route is described in [api/openapi.json](api/openapi.json) (public
port) and [api/management-openapi.json](api/management-openapi.json)
(management port).

- **As provider**, the connector agrees to a request for an offer in
  its catalog, then finalizes once the consumer verifies. It sends its
  messages to the consumer's `callbackAddress`.
- **As consumer**, it accepts the provider's offer and verifies its
  agreement. Providers send their messages to its callback endpoints
  (`/dsp/2025-1/callback/...`). Your own systems start negotiations and
  transfers through the [management API](#management-api).
- **Messages are not lost.** Each outgoing message is saved before it is
  sent, and sent again after a restart
  ([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md)). A send is
  tried up to 4 times when no answer came, or the answer was 408, 429
  or 5xx; any other answer ends the tries. If a send fails, the
  connector asks the partner for its state before giving up. If the
  message did not arrive, it terminates the negotiation or transfer. A
  partner's message that arrives before its acknowledgement of ours is
  recorded proves ours arrived, so it is accepted at once.
- **Pull transfers** (`HttpData-PULL`,
  [ADR 0013](adr/0013-transfer-and-data-plane.md)). The connector
  accepts a transfer request under a finalized agreement held by the
  caller, starts it at once and sends the consumer a data address:
  `{PORTITOR_PUBLIC_URL}/data/{flowId}` with a bearer token. The token
  works while the transfer is started and for
  `PORTITOR_DATA_TOKEN_SECONDS` after the start. The connector then
  fetches the dataset's `portitor:source` and streams its answer (2xx
  only, with its `Content-Type`). A request for a dataset without a
  source is refused. A transfer request sent again by the same consumer
  with the same `consumerPid` starts nothing new: the answer is the
  transfer it already has.
- **Push transfers** (`HttpData-PUSH`, since 1.1,
  [ADR 0027](adr/0027-push-transfers-and-object-storage.md)). The
  connector sends the data itself, as provider, or receives it into its
  object storage, as consumer. See
  [Object storage and push transfers](#object-storage-and-push-transfers).

### Datasets file

The datasets file is a JSON array of DSP 2025-1 `Dataset` objects, as
the catalog serves them, without `@context`. See
[examples/datasets.json](../examples/datasets.json).

- Each dataset needs at least one offer (`hasPolicy`) and one
  distribution with a `format`. Leave `accessService` out: the connector
  sets it to its own data service.
- Other properties, such as `dct:title` or `dcat:keyword`, are kept and
  served as written (the DSP context defines the `dct`, `dcat` and
  `odrl` prefixes).
- Give each dataset you want to transfer its data with
  `portitor:source`, an HTTP(S) URL that the connector fetches for the
  consumer. It is taken out of the dataset and never shown in the
  catalog:

  ```json
  { "@id": "weather-stations", "@type": "Dataset", "portitor:source": "https://data.internal.example/weather.csv", ... }
  ```
- Since 1.1 `portitor:source` may also be an object in your own object
  storage, written `s3://{bucket}/{key}` (bucket in lower case, the key
  as stored). The connector reads it with its own keys
  ([Object storage and push transfers](#object-storage-and-push-transfers));
  the consumer sees an ordinary pull, and the keys never leave your
  connector.
- A dataset offers push when one of its distributions has the format
  `HttpData-PUSH`, next to (or instead of) `HttpData-PULL`.
- An offer in the datasets file has no `target`, `assigner` or
  `assignee`.

### Policies

Offers are minimal ODRL. The full rules are in
[ADR 0009](adr/0009-minimal-odrl-policies.md) and
[ADR 0019](adr/0019-credential-constraints.md); what you need to write
and read offers:

- A permission without constraints permits. Two kinds of constraint are
  understood, and `and`, `andSequence`, `or` and `xone` combine them:
  - `dateTime` (when the action is exercised) with `eq`, `neq`, `lt`,
    `lteq`, `gt` or `gteq`, and a date (`2030-12-31`, the whole day) or a
    date and time with a time zone (`2030-12-31T23:59:59Z`);
  - a credential claim, `<CredentialType>.<claim path>` (for example
    `MembershipCredential.role` or `DataProcessorCredential.level.name`),
    with `eq` or `neq` and a text, or `isAnyOf` or `isNoneOf` and a list
    of texts ([Credential constraints](#credential-constraints),
    [Gaia-X compliance](#gaia-x-compliance)).
- **Anything else is refused, never dropped** (fail closed, strict
  reading): for example a `duty` inside a permission, `unit` on a
  constraint or a misspelled key. In your datasets file, that dataset is
  rejected by the harvest (check `odrl`) and the others are harvested.
  In a partner's offer, contract request or agreement, the message is
  answered 400 with the DSP error naming the field, and the negotiation
  does not move. Eclipse EDC ignores such fields, so an offer that EDC
  accepts can be refused here.
- The fields read are `@id`, `@type`, `target`, `assigner`, `assignee`,
  `profile`, `permission`, `prohibition` and `obligation` (and
  `timestamp` on an agreement); `action` and `constraint` in a rule;
  `leftOperand`, `operator` and `rightOperand`, or one of `and`,
  `andSequence`, `or`, `xone`, in a constraint. In a message, an
  offer's `assigner` must be the provider and its `assignee` the
  consumer, when present.
- As provider, the connector agrees only if the requested offer has the
  same terms as the catalog offer with that `@id`, and its constraints
  hold now for the consumer's credentials. Otherwise it terminates the
  negotiation, saying why. It checks the agreement again, with fresh
  credentials, at each transfer request.
- As consumer, it does not accept a provider's offer (DSP `OFFERED`)
  that has credential constraints: it cannot check its own credentials
  as the provider will. Requests it sends itself are not affected. It
  verifies an agreement only if it has the terms it asked for or
  accepted and names your participant id as `assignee`.
- Prohibitions and duties are carried into the agreement but not
  enforced.

### Dataset harvest

The datasets come from a source outside the connector into a read copy
in its database (ADR 0020). Today the source is the
[datasets file](#datasets-file) (`PORTITOR_DATASETS_FILE`).

- A run happens at start, before the DSP port serves, so the catalog is
  ready at once; then every `PORTITOR_HARVEST_INTERVAL_SECONDS`. One run
  at a time: a timer tick that finds a run going on is skipped.
- The file is read again at each run. When its SHA-256 is the last run's,
  the run ends `unchanged` and nothing else is touched. To publish a
  change, replace the file; it is taken at the next run (or restart).
  A changed `portitor:source` reaches only transfers that start after
  that run: since 1.2 a transfer keeps the source it started with until
  it ends ([ADR 0028](adr/0028-separate-pluggable-data-plane.md),
  note added in 1.2).
- Each dataset is checked on its own: JSON object, DSP `Dataset` form,
  the DSP catalog rules, offers with only the policy terms and ODRL
  constraints the connector reads ([Policies](#policies)), a valid
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

For a strict start, make your start-up probe check that the latest
job's `outcome` is `succeeded`.

### Schema and vocabulary bundle

A dataspace authority can publish a signed bundle of SHACL shapes, code
lists and JSON-LD contexts (ADR 0005); HealthDCAT-AP release 8 is the
baseline. With a bundle the connector checks each harvested dataset
against the shapes for its `dct:accessRights` value and, if the bundle
has message shapes, DSP negotiation and transfer messages against them,
with Portitor's own SHACL Core engine (`portitor-shacl`). A dataset that
passes is served with
`"dct:conformsTo": {"@id": "<the bundle's profile>", "@type": "dct:Standard"}`.
Without `PORTITOR_BUNDLE_URL` and `PORTITOR_BUNDLE_AUTHORITY` nothing is
checked against shapes.

**Taking a version**

- The connector downloads `manifest.json` and `manifest.jws` first. It
  checks the signature (ES256) with the bundle key listed in the
  authority's `did:web` document (`<did>#bundle-key`, under
  `assertionMethod`), the version and the authority, then downloads each
  file and checks its SHA-256. A bundle signed with any other key of the
  authority, such as its credential key, is refused. Limits: 1 MiB for
  the manifest, 64 MiB per file, 256 MiB in all, 120 s per download.
- New versions are pulled, never pushed: at start and every
  `PORTITOR_BUNDLE_REFRESH_SECONDS` the connector reads `latest.json`.
  A newer version is checked as above and put in use without a restart;
  then the stored datasets are checked again (those that fail are
  hidden) and the datasets source is harvested again. Each check is a
  run in `GET /v1/harvest-jobs`, with `"kind": "bundle"`. A fixed
  `PORTITOR_BUNDLE_VERSION` never changes while the connector runs; the
  manifest must have exactly that version.
- The connector never goes back to a version issued before the one in
  use, which it remembers in the cache. A version that cannot be taken
  (unreachable, refused, older) leaves the version in use and the run
  is `failed` with the reason. At start, with `latest`, the version in
  use before is read back from the cache if the newest cannot be taken.
  The order comes from the manifest's `issued` (`dcterms:issued` in
  W3C DCAT 3), a UTC date and time to the second.
- The verified bundle is kept in `PORTITOR_BUNDLE_CACHE_DIR`; keep it on
  the `/data` volume. The cache is used only when the authority's host
  or DID document cannot be reached; its files and signature are then
  checked again, with a warning.
- These stop the start: a bundle that was read but fails a check
  (signature, key, authority, version or a digest; the cache does not
  replace it); a bundle that cannot be loaded at all (no download and no
  usable cache); and one whose shapes, vocabulary or contexts the
  connector cannot use (for example a SHACL term the engine does not
  implement). The log line `portitor-connector stopped` says why. Start
  the connector after the authority's issuer, or let it restart.

**A file with a role this version does not know is skipped** (ADR 0005).
A later version of the format may add roles, for example policy shapes.
The connector does not download such a file and does not use it; the
other files are used as before.

- You see it in three places: one WARN line for each file
  (`bundle file skipped: ...`, with `path` and `role_type`), the gauge
  `portitor.bundle.file.skipped`, and the bundle run in
  `GET /v1/harvest-jobs`, whose summary ends with
  `N files skipped (unknown role, ...)`.
- **The limit: a connector that skips a new kind of rule does not
  enforce it.** Upgrade the connector to enforce it, and do so while the
  authority's bundle address answers: the cached copy does not hold the
  skipped file, so after the upgrade the connector needs the authority's
  server once (the start stops if it cannot be reached and there is no
  other usable copy).
- A bundle in which the connector knows no role at all is refused like
  any invalid bundle (the version in use stays; with no version, the
  start stops). A role type that is a known one in other letter case
  (`Messages` for `messages`) is taken as a mistake and the bundle is
  refused. Everything else in a manifest is read strictly: a field the
  format does not have is an error.
- A skipped file may have any media type (`type/subtype`, RFC 6838); a
  file with a role the connector knows must be `text/turtle`,
  `application/n-triples` or `application/ld+json`.

**What is checked**

- Datasets are checked as the catalog serves them: read with the DSP
  context only. With a bundle, each dataset needs an absolute IRI as
  `@id` (JSON-LD drops a relative one, and with it every statement about
  the dataset) and a `dct:accessRights` IRI the bundle has shapes for;
  otherwise it is rejected.
- The catalog's `@context` is always the DSP 2025-1 context alone. The
  connector never fetches a JSON-LD context from the network: it has its
  own copy of the DSP 2025-1 contexts, and a bundle's `context` files
  serve only to read DSP messages whose `@context` names their URL. A
  message that names any other context URL is refused when the bundle
  has message shapes.
- Message rules: an inbound negotiation or transfer message that breaks
  them is answered 400 with the DSP error, and nothing changes; an
  outbound one is not sent and the negotiation or transfer ends (logged
  as `message breaks the dataspace's message rules; not sent, ...`).
  Catalog requests are not checked.
- The engine checks the SHACL Core subset of `portitor-shacl`.
  `sh:extends` and `sh:shape` are ignored with a warning, so DCAT-AP's
  "date or date and time" check on `dct:issued` and `dct:modified` is not
  done. Class targets apply only to nodes typed in the dataset itself.

**Writing datasets for HealthDCAT-AP**

Write HealthDCAT-AP datasets as DSP 2025-1 `Dataset` objects, as in
[examples/healthdcat-datasets.json](../examples/healthdcat-datasets.json).
Use the `dct:`, `dcat:`, `odrl:`, `xsd:` and `dspace:` prefixes of the
DSP context, and write every other term as a full IRI, for example
`"http://healthdataportal.eu/ns/health#healthCategory"` or
`"@type": "http://xmlns.com/foaf/0.1/Agent"`. Other prefixes are not
supported
([ADR 0005](adr/0005-schema-and-vocabulary-registry.md) says why). A
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
  (a list of texts). A constraint holds if one of the consumer's
  verified credentials of that type has the claim, as text, and the
  comparison is true; a claim that is missing or not text never grants
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
  in `scenarios/edc-interop/config/edc-controlplane.properties`. The Minimum Viable
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
  an ordinary dataspace credential and checked like any other. Policies
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

### Status list cache

The connector checks each partner's credentials against the issuer's
status list on every DSP request. It reuses a status list credential it
fetched for `PORTITOR_STATUS_LIST_CACHE_SECONDS` (default 300 seconds),
by the list's URL, in memory.

- **The trade-off.** A revocation or suspension takes up to that long to
  be seen: a partner whose credential was just revoked is still admitted
  until the cached list is fetched again. With `0` it is seen on the
  next request, at the cost of one fetch per credential status on every
  request. Eclipse EDC reuses status lists for 15 minutes by
  default (`edc.iam.credential.revocation.cache.validity`).
- Only a list that was fetched and verified (signed by the credential's
  issuer, valid now) is reused. A failed fetch or check is not: it is
  tried again on the next request.
- A list is never reused past its own end (`validUntil`, or the JWT's
  `exp`), even if that comes before the configured time.
- The cache is per process: a restart empties it. Presentations and
  tokens are not cached; they are checked on every request (ADR 0016).

### Object storage and push transfers

Since 1.1 ([ADR 0027](adr/0027-push-transfers-and-object-storage.md)).
One S3-compatible object storage per connector, for two uses: datasets
whose `portitor:source` is `s3://{bucket}/{key}`, and, as consumer, the
bucket where data pushed to you lands. Tested with RustFS
(`scenarios/two-party`); the connector uses path-style addresses
(`{endpoint}/{bucket}/{key}`) and AWS Signature Version 4, which RustFS,
MinIO and AWS S3 accept.

- **Settings.** `AWS_ENDPOINT_URL_S3`, `AWS_REGION`,
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`
  (the AWS SDK names) and `PORTITOR_PUSH_BUCKET`. Without the keys the
  connector has no object storage: `s3://` sources answer 502 and push
  transfers as consumer are refused. The log line `object storage`
  names the endpoint and the push bucket, never the keys.
- **Checked only with a push bucket.** With `PORTITOR_PUSH_BUCKET` set,
  a start with only one of the two keys, or an `AWS_ENDPOINT_URL_S3`
  that is not an `http(s)` URL, stops. Without it, such `AWS_*`
  variables (perhaps set for another tool, as a 1.0 connector allowed)
  are ignored: one INFO line, `no object storage`, and no object
  storage. If you use `s3://` sources without a push bucket, look for
  that line.
- **Temporary keys.** With keys from AWS STS (an assumed role, for
  example), also set `AWS_SESSION_TOKEN`; it is sent and signed as
  `x-amz-security-token`. The connector reads it once at start, so
  restart it with fresh keys before they expire.
- **The keys.** They follow the AWS rule, not Portitor's `_FILE` rule
  for secrets: inject `AWS_SECRET_ACCESS_KEY` into the container's
  environment from your secret store (Docker or Kubernetes secrets,
  Vault), never write it in a compose file or a shell history. Give the
  connector a key of its own, limited to reading the source buckets and
  writing the push bucket.
- **Create the push bucket first.** The connector does not create
  buckets. Each pushed transfer becomes the object `{consumerPid}` in
  it; a second push for the same transfer replaces it. Uploads go in
  parts of 8 MiB (multipart upload, aborted if anything fails, and when
  the request is cut half-way).
- **Add a lifecycle rule to the push bucket** that aborts incomplete
  multipart uploads after a day (`AbortIncompleteMultipartUpload`, for
  example `DaysAfterInitiation` 1; RustFS and MinIO support it too). An
  upload under way when the connector stops cannot be aborted by it,
  and its parts would otherwise stay in the bucket, and be billed.

**As provider**, nothing else is needed: list `HttpData-PUSH` in the
dataset's distributions. The consumer's transfer request carries its
endpoint (`endpointType`
`https://w3id.org/dspace-sig/profile/http-push`) and a token; once the
transfer is started, the connector sends the data as one `POST` with
`Authorization: Bearer <token>` and the source's `Content-Type`. A 2xx
answer completes the transfer; any other answer, or none, terminates
it, and the reason holds only the status code. The connection to the
endpoint must open within 10 seconds, and the push fails when the
endpoint takes no data and gives no answer for 120 seconds. Suspend or
terminate, by either side, cuts a running push; a start again sends
everything again. A push cut because the membership check could not
finish terminates the transfer.
The consumer's endpoint and token are kept in the SQLite file until the
transfer ends, and are never logged or shown. The consumer's membership
is checked before each push and while it runs, as for a download
([Revocation and running transfers](#revocation-and-running-transfers)).

- **The endpoint is an address the consumer chose** (R-17 in
  [RISKS_AND_TECHNICAL_DEBTS](RISKS_AND_TECHNICAL_DEBTS.md#r-17)), like
  the `callbackAddress`: it must be `https://` unless
  `PORTITOR_ALLOW_HTTP` is `true`; redirects are not followed, and the
  answer's body is not read. Internal addresses are not blocked, so
  keep the egress firewall of [Security](#security): it now also guards
  the data you push.

**As consumer**, set `PORTITOR_PUSH_BUCKET` with the keys, then request
a transfer with `"format": "HttpData-PUSH"`. The connector gives the
provider the address `{PORTITOR_PUBLIC_URL}/push/{consumerPid}` and a
token of 32 random bytes, of which it keeps only the SHA-256 once the
provider has it. The token works for `PORTITOR_DATA_TOKEN_SECONDS` and
only while the transfer is `REQUESTED` or `STARTED`; a start again
gives a new one. Its life counts from your request (or start again),
not from the provider's start: a provider that starts late leaves less
time for the push, so give `PORTITOR_DATA_TOKEN_SECONDS` room for the
provider's delay and the size of the data. The transfer
view shows `objectKey` once data has arrived; the provider completes the
transfer.

- **The receiving route** (`POST` or `PUT /push/{consumerPid}`) is open,
  like `/data`, and checks its own token before it reads the body; every
  refusal is the same 401. It is not subject to
  `PORTITOR_MAX_REQUEST_BYTES` nor to the request time limit, since a
  dataset can be large: let your reverse proxy pass large, long
  requests on `/push/` (for nginx, `client_max_body_size 0;` and a long
  `proxy_read_timeout` in that `location`), and limit them there if you
  need a bound. It has limits of its own: one upload at a time per
  transfer (a second one gets 409), at most 8 at once in all (one more
  gets 503; each holds up to 8 MiB in memory), and a body that sends
  nothing for 120 seconds is cut: the upload is aborted, the answer is
  400, and the audit line says `cut`.

### Revocation and running transfers

From 1.1 a provider checks the consumer's membership again for every
use of the data, not only when the transfer is asked for (ADR 0027).
When the consumer asks for a transfer, the provider keeps the issuer,
the end of validity and the `credentialStatus` of each of its
`MembershipCredential`s with the data flow. Then:

- **On every `/data` request** it checks them again, without the
  consumer: the issuer is still trusted, the credential has not
  expired, no status bit is set. A revocation is therefore seen on the
  next request after at most `PORTITOR_STATUS_LIST_CACHE_SECONDS` (the
  same delay as for DSP requests). The request is refused with 401, the
  transfer is ended (the provider sends the DSP termination) and the log
  says why.
- **While a download runs** it checks again every
  `PORTITOR_STATUS_LIST_CACHE_SECONDS`, but at least every 60 seconds
  and at most every 5. On a revocation the download is cut and the
  transfer ended. So a running download can last up to the cache time
  plus 60 seconds after a revocation, with the default cache 300 + 60.
- **A check that cannot finish** (a status list or the issuer's DID
  document does not answer) refuses the request, or cuts the download,
  but keeps the transfer: the next request may pass.
- **Transfers started before 1.1** have no membership kept. They are
  served as in 1.0 until their token expires (at most
  `PORTITOR_DATA_TOKEN_SECONDS`), and logged once per flow. A 1.1
  transfer with no membership kept is refused.

A partner whose membership was revoked can still end a transfer it
shares with you: a `TransferTerminationMessage` is accepted from a
caller with a valid DCP token who is the transfer's counter-party, and
logged (TD-47, [ADR 0016](adr/0016-dsp-caller-authentication.md),
amended in 1.1). Every other message from it is refused.

### Management API

Through the management API you read the harvest jobs
([Dataset harvest](#dataset-harvest)), and your own systems read a
provider's catalog and start and follow negotiations and transfers as
consumer. It is not part of DSP, so it has its own port
(`PORTITOR_MANAGEMENT_BIND`) and is off unless
`PORTITOR_MANAGEMENT_API_KEY_FILE` is set. Every call
needs `Authorization: Bearer <key>`, except `/v1/openapi.json`,
`/health` and the health checks. Every route and field is described in
[api/management-openapi.json](api/management-openapi.json), which the
connector also serves at `GET /v1/openapi.json`. The identity and the
issuer have their own management APIs, described in their steps.

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
offers in `hasPolicy`. `POST /v1/catalog/dataset/request` returns one
dataset, by its `@id`. Both request bodies follow Eclipse EDC's
management API (`/v4/catalog/request`, EDC v1.0.0-RC8), so a client
written for EDC can send the same body. What Portitor does with each
member (`querySpec` is refused, for example) is in the `CatalogRequest`
and `DatasetRequest` schemas of the OpenAPI file.

If the provider refuses, cannot be reached or sends something else, the
answer is 502 and its `detail` says which (for example `the provider
answered HTTP 401 Unauthorized`). The limits of these two calls, and
their other 502 cases, are under [Known limits](#known-limits).

Then negotiate, with an offer copied unchanged from the catalog (the
provider agrees only to the catalog's terms):

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

`GET /v1/negotiations/{consumerPid}` returns the negotiation; it holds
the agreement once `state` is `FINALIZED`. Then request a transfer under
that agreement, wait for `STARTED`, and fetch the data yourself with the
token from the data address (pull transfer,
[ADR 0014](adr/0014-transfers-as-consumer.md)):

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

To pause access without ending the transfer (since 1.1), suspend it
with `POST /v1/transfers/{consumerPid}/suspension`; the provider's data
plane refuses the token until you start it again with `POST
/v1/transfers/{consumerPid}/start`. The token of a pull stays the same;
the provider sends no new one (a push gets a new one). Either call
answers 202 at once; the state changes once the provider acknowledges the message, and a call while
another message is on its way gets 400. Start works whichever party
suspended the transfer (DSP 2025-1).

`format` defaults to `HttpData-PULL`. With `"format": "HttpData-PUSH"`
(since 1.1) the provider sends the data to your connector, which stores
it in your push bucket ([Object storage and push
transfers](#object-storage-and-push-transfers)): wait for `objectKey` in
the transfer, then read that object from your bucket. The two list calls
(`GET /v1/negotiations`, `GET /v1/transfers`) return up to 100 entries
at a time (`?after=`, `?limit=`).

As provider (1.1), you read the transfers your connector runs and end
one on your own (ADR 0027):

```sh
curl -s localhost:8081/v1/provided-transfers -H "authorization: Bearer $(cat key)"
curl -s -X POST localhost:8081/v1/provided-transfers/urn:uuid:.../termination \
  -H "authorization: Bearer $(cat key)"
```

The termination answers 202: access ends at once (the data flow is
terminated and every running download of it is cut), and the DSP
termination goes to the consumer in the background. The view shows the
consumer's DID (`counterPartyId`), never the data token or its hash.

## Security

- **Caller authentication (DCP, ADR 0016).** Every DSP endpoint, the
  consumer callbacks included, needs a self-issued ID token in
  `Authorization: Bearer`. The connector checks it, asks the caller's
  credential service for its presentations, and accepts the caller only
  with a valid `MembershipCredential` from a DID in
  `PORTITOR_TRUSTED_ISSUERS` whose subject is the caller, not revoked or
  suspended (the status list must be signed by the same issuer). Only
  `/.well-known/dspace-version`, `/health`, the data plane and, since
  1.1, the receiving route `/push` are open (they have their own
  tokens).
  - One credential from a trusted issuer that fails its checks (revoked,
    expired, bad signature, status list not readable, about someone
    else) makes its presentation invalid and refuses the caller, even
    next to a valid membership credential (DCP v1.0.1, 5.4.3, step 8);
    the log names it (`credential refused`, INFO). So a participant must
    delete a revoked credential once its reissued one arrives
    ([Getting a credential](#getting-a-credential)). Credentials from
    other issuers are skipped.
  - A refused caller gets a DSP error that says only "unauthorized":
    HTTP 404 on the negotiation and transfer routes (DSP 2025-1 HTTPS
    bindings, "Unauthorized Access"; Eclipse EDC does the same), HTTP
    401 on the catalog routes. The reason is in your log (`DSP request
    refused: caller not authenticated`, WARN).
  - When the check cannot finish because a server it fetches from does
    not answer, or answers 408, 429 or 5xx, the answer is HTTP 503, so
    the partner tries again later.
  - Each negotiation and transfer is bound to the DID that started it:
    another caller is told it does not exist.
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
  its setting, 2 MiB by default. These routes, whose messages are small
  by specification, have a fixed limit of 64 KiB, or the port's limit
  when that is lower:
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
  the bundle key that the authority's `did:web` document lists
  ([bundle](#schema-and-vocabulary-bundle)). The bundle key is separate
  from the issuer's credential key, and the issuer holds only its public
  part. Shapes, code lists and contexts are never fetched from the
  network apart from the authority's signed bundle, and no Portitor
  binary parses RDF/XML.
- **Outbound calls.** The connector POSTs to the `callbackAddress` that the
  partner chooses and, since 1.1, sends pushes to the endpoint the
  consumer chooses. Only `https://` is allowed, but internal addresses are
  not blocked yet. Put the connector in a network zone where it cannot
  reach internal services it should not call (egress firewall or network
  policy). It also calls your object storage at `AWS_ENDPOINT_URL_S3`,
  an address you choose ([R-17](RISKS_AND_TECHNICAL_DEBTS.md#r-17)).
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
- **Health checks and telemetry.** The health checks answer
  without a key and name what failed, so keep the management port on
  an internal network, as for the management API. Traces and metrics
  are off by default. Spans name your partners' hosts and the DSP
  process ids, so send them only to a receiver you control, over
  `https://` when it is not on your internal network, with its key in
  `OTEL_EXPORTER_OTLP_HEADERS`. Your data sources receive a
  `traceparent` header with telemetry on; partners never do
  ([Traces and metrics](OPERATING.md#traces-and-metrics)).
- **The database file holds access tokens.** As consumer, the connector
  keeps the data address of each usable transfer, token included, in
  `/data/portitor.db` until the transfer ends (ADR 0014). Since 1.1 it
  also holds push tokens: as consumer, our push token in the transfer
  request until the provider has it (then only its SHA-256); as
  provider, the consumer's push token and endpoint until the transfer
  ends ([R-19](RISKS_AND_TECHNICAL_DEBTS.md#r-19)). Protect the volume
  and its backups like the management key.
- Do not set `PORTITOR_ALLOW_HTTP=true` outside development and tests.
- Report vulnerabilities as described in [SECURITY.md](../SECURITY.md).

## After deployment

Running Portitor day to day is in [OPERATING](OPERATING.md): the
[health checks and probes](OPERATING.md#health-and-logs), the log lines
and what to do about them, [traces and metrics](OPERATING.md#traces-and-metrics),
[backup and restore](OPERATING.md#backup-and-restore),
[changing a key](OPERATING.md#rotating-a-key), and
[upgrades](OPERATING.md#upgrade-and-rollback) with the notes of every
release.

## Two data spaces

One connector serves one dataspace
([ADR 0024](adr/0024-several-data-spaces.md), accepted for 1.0). If
your organisation is in two dataspaces, run **one connector per
dataspace**. Each connector has its own:

- host name and `PORTITOR_PUBLIC_URL` (the connector serves
  `/dsp/2025-1` at the root of its host), and its own management port
  and key;
- database and volume (`PORTITOR_DB_FILE`; one connector per file);
- datasets file, with the description and the offers that dataspace
  asks for; the `portitor:source` of a dataset can be the same in both;
- `PORTITOR_TRUSTED_ISSUERS`, with the issuers of **that dataspace
  only**, and that dataspace's `PORTITOR_DCP_SCOPES`,
  `PORTITOR_POLICY_CREDENTIALS` and bundle settings.

A member of one dataspace is then refused by the connector of the
other, because its credential comes from an issuer that connector does
not trust.

For the identity there are two ways:

| | One identity and DID per dataspace (the default) | One shared identity and DID |
|---|---|---|
| Deploy | One `portitor-identity` per dataspace, each with its own DID, key, database and host name. Each connector uses the STS of its own identity | One `portitor-identity`. Both connectors have the same `PORTITOR_PARTICIPANT_ID`, `PORTITOR_STS_URL` and STS client secret |
| `PORTITOR_TRUSTED_ISSUERS` of the identity | That dataspace's issuers | The issuers of both dataspaces |
| Credentials | Each identity asks its dataspace's issuer. With Gaia-X, one onboarding per identity | The one identity asks each issuer |
| What partners learn | Nothing about the other dataspace | **A partner in one dataspace learns that you are a member of the other,** when both use the same credential type: it asks for `MembershipCredential` and gets both (DCP has no scope by issuer). Its own check skips the credential of the issuer it does not trust |

Use a shared DID only if this disclosure is acceptable in both
dataspaces. With a shared DID, also remember that a partner whose connector
trusts both issuers refuses you while one of the two credentials is
revoked or expired ([Security](#security), "Caller authentication").

Both ways were tested by hand on one machine
([TESTING](TESTING.md#two-data-spaces)). One connector for several
dataspaces is planned after 1.0.

## Users and roles

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

## Known limits

In this version. See also [STABILITY](STABILITY.md#what-10-does-not-do)
for what 1.0 does not do (no TLS in the services, one connector per
dataspace, no persons, no test with Simpl-Open),
[SECURITY-ASSURANCE.md](SECURITY-ASSURANCE.md), and
[RISKS_AND_TECHNICAL_DEBTS.md](RISKS_AND_TECHNICAL_DEBTS.md) for every
known risk and technical debt, with its status and target release.

- **Admission.** Partners are admitted by one credential type
  (`MembershipCredential`, `PORTITOR_DCP_SCOPES`); offers may add
  credential claims (ADR 0019), compared as text only, but the catalog
  still shows every offer to every member. Credential schemas are not
  checked: the issuer publishes each definition's `credentialSchema` URL
  and no component fetches it.
- **Revocation** takes up to `PORTITOR_STATUS_LIST_CACHE_SECONDS` to be
  seen ([Status list cache](#status-list-cache)); a running download up
  to 60 seconds more
  ([Revocation and running transfers](#revocation-and-running-transfers)).
- **Policies.** Only `dateTime` and credential claims are understood,
  any other term is refused, and prohibitions and duties are carried
  into agreements but not enforced ([Policies](#policies)).
- **Negotiations** start from the consumer only; a provider cannot start
  one with an offer ([TD-35](RISKS_AND_TECHNICAL_DEBTS.md#td-35)).
- **Transfers.** Pull, and since 1.1 push over HTTP into S3-compatible
  object storage; no other way to move data. A push token is the
  consumer's, and a push is sent whole, in one request, from the
  beginning each time. A data access token ends
  `PORTITOR_DATA_TOKEN_SECONDS` after the transfer starts and is not
  renewed; the consumer then asks for a new transfer. Only `GET` of the
  whole source; no credentials are sent to it. As provider, a transfer
  request repeated with the same `consumerPid` gets the transfer it
  already has ([TD-34](RISKS_AND_TECHNICAL_DEBTS.md#td-34)).
- **Transfers as consumer.** Each `POST /v1/transfers` starts a new
  transfer. Your own system fetches the data with the data address; the
  connector does not download it, except for a push, which it stores in
  your push bucket.
- **Reading a provider's catalog** (`POST /v1/catalog/request` and
  `/v1/catalog/dataset/request`, [Management API](#management-api)). At
  most 16 MiB of answer are read, a fixed limit with no setting; a
  larger catalog gets HTTP 502. The request is sent once and waits up
  to 30 seconds. Of a catalog sent in pages (DSP 2025-1, catalog HTTPS
  binding, "Pagination") only the first page is returned, with the
  provider's `Link` headers copied to the answer. No filter is sent:
  EDC's `querySpec` is refused with 400. A compressed answer
  (`Content-Encoding: gzip`, which the connector does not ask for) is
  not read and gets 502. A catalog without a `participantId`, or with
  one that differs from `counterPartyId`, gets 502
  ([TD-38](RISKS_AND_TECHNICAL_DEBTS.md#td-38),
  [TD-36](RISKS_AND_TECHNICAL_DEBTS.md#td-36)).
- **Datasets** come only from the datasets file; no other harvest source
  yet. No API writes datasets or policies.
- **Bundle.** A file with a role this version does not know is skipped,
  so a rule of a later kind is not enforced until the connector is
  upgraded ([bundle](#schema-and-vocabulary-bundle)). The readiness
  check `bundle` does not show skipped files (it only passes or
  fails); the log, the gauge and the job log do.
- **SHACL.** Only the subset `portitor-shacl` implements (ADR 0005).
  Shapes that use anything else are refused when the bundle is loaded;
  `sh:extends` and `sh:shape` are ignored with a warning.
- **Gaia-X.** Tested against a local Gaia-X compliance service in test
  mode (gx-compliance v2.16.0), not the Gaia-X Lab or a production
  clearing house. You get the registration number credential from a
  notary yourself; label levels 1 to 3, service offering credentials and
  OpenID4VC are not supported. The identity signs the legal person
  credential itself; another signer cannot be named. A Gaia-X compliance
  credential is checked between Portitor connectors only; EDC does not
  accept it (ADR 0022).
- **Scale.** One replica per database file; no PostgreSQL adapter yet.
  STS tokens are not cached: each DSP request costs one STS call, and
  each incoming one a presentation query. Every request waits for its
  database write to reach the disk, one write at a time per database
  file; measured for 0.12, new negotiations did not slow 100 pulls in
  flight beyond the agreed limit
  ([BENCHMARKS](BENCHMARKS.md#td-40-do-negotiations-slow-the-data-plane-down),
  [TD-40](RISKS_AND_TECHNICAL_DEBTS.md#td-40)). Each pull in flight uses
  memory in the connector and two connections (one from the consumer,
  one to your data source); see [BENCHMARKS](BENCHMARKS.md#data-plane)
  before you plan for hundreds of pulls at once, and raise the open
  files limit (`ulimit -n`) above twice that number.
- **Network.** Internal callback and push addresses are not blocked
  ([Security](#security), "Outbound calls").
- **Panics.** A panic (a bug) does not stop the process: the request
  gets HTTP 500 and a background task starts again
  ([OPERATING](OPERATING.md#health-and-logs), the `panic` lines). Alert
  on `portitor.http.request.panicked` and `portitor.task.panicked`.
  Still run each container with a restart policy: a failed start, a
  stopped management server and a kill for lack of memory end the
  process ([SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#what-a-panic-does),
  [TD-41](RISKS_AND_TECHNICAL_DEBTS.md#td-41)).
