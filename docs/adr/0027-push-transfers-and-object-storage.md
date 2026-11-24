<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0027: Push transfers, object storage and revoking access

- Status: **Proposed** (2026-11-24). The maintainer accepted the
  direction on 2026-11-24 (HTTP push by the Data Plane Signaling HTTP
  profile; data in the provider's object storage served through the
  provider's own data plane; access always revocable). Section 7 decided on
  2026-11-24.
- Date: 2026-11-24
- Scope: release 1.1 ([ROADMAP](../ROADMAP.md#11-push-to-object-storage)).
  Answers [TD-10](../RISKS_AND_TECHNICAL_DEBTS.md#td-10) (push) and
  [R-38](../RISKS_AND_TECHNICAL_DEBTS.md#r-38) (no S3 data address).
  Amends ADR 0013 (a second format, a second kind of source) and
  ADR 0014 (the consumer gets a receiving route). Additive only: nothing
  listed in [STABILITY](../STABILITY.md) changes.

## Context

1.0 has one way to move data: HTTP pull. The provider's built-in data
plane serves `GET /data/{flowId}` with a short-lived bearer token and
streams the dataset from an upstream HTTP URL (ADR 0013). The consumer
side has no data plane; the operator's own system fetches the data
(ADR 0014).

The roadmap asked for push into S3-compatible object storage, reusing
the data address of EDC or Simpl-Open. What we found on 2026-11-24, in
the code of each project:

| Fact | Source |
| --- | --- |
| DSP 2025-1: a push transfer is one where "the Provider's data plane initiates sending data to a Consumer endpoint". The `dataAddress` of the transfer request "MUST only be provided if the `format` REQUIRES a push transfer" and "MAY include an `endpoint`, a temporary authorization via the `endpointProperties`". Only the consumer sends a `TransferRequestMessage`. | DataspaceProtocol, tag `2025-1-err2`, `transfer.process.protocol.md` |
| EDC removed its S3 data plane (`data-plane-aws-s3`) on 2026-07-01. Only the property names of its `AmazonS3` address remain. | eclipse-edc/Technology-Aws, commit c84f9ed (#688) |
| EDC's new data plane, Siglet, issues tokens for `HttpData-PULL` and `HttpData-PUSH`. For push the token comes from the consumer ("token source: client"). It does not move data itself. | eclipse-dataplane-core/siglet, `docs/siglet.md` |
| Simpl-Open uses IONOS's EDC extension (`IonosS3`). It creates buckets and keys through the IONOS cloud API. | ionos-cloud/edc-ionos-s3; [ADR 0017](0017-simpl-open.md) |
| The Data Plane Signaling **HTTP Transfer Profile** defines push: the consumer's data plane exposes an HTTPS API, the provider sends data to it and presents the `authorization` token with `authType` `bearer`. `endpointType` is `https://w3id.org/dspace-sig/profile/http-push`. Properties: `authorization`, `authType`, `contentType`. It does not name the HTTP method. Draft, not tagged; last changed 2026-09-25. | eclipse-dataplane-signaling/profiles, `specifications/profiles/http.md` and `schemas/http/data-address-schema.json` |
| The Signaling endpoint type registry, where an S3 profile would be published, is empty. | eclipse-dataplane-signaling/endpoint-type-registry |
| EDC's own push test data plane sends the data with `POST`. | eclipse-edc/Connector, `system-tests/e2e-transfer-test/signaling-data-plane`, `SignalingDataPlaneRuntimeExtension.java` |

So no specification names an S3 address, and the one EDC had is gone.
The newest text that covers push is the HTTP Transfer Profile, and it
keeps storage keys out of the exchange: only a token for one transfer
crosses between participants.

The maintainer set a rule on 2026-11-24: **a provider must always be
able to revoke access**. That rules out handing out pre-signed S3 URLs
or S3 keys, which cannot be withdrawn before they expire.

## Decision

Three ways to move data, all started by the consumer as DSP requires.

| Way | Format in the catalog | Who moves the bytes | Where the data ends |
| --- | --- | --- | --- |
| Pull (1.0) | `HttpData-PULL` | the consumer's system, `GET /data/{flowId}` | wherever the consumer's system writes it |
| Pull from the provider's object storage (new) | `HttpData-PULL`, unchanged | as above; the provider's data plane reads its own bucket | as above |
| Push (new) | `HttpData-PUSH` | the provider's data plane, to the consumer's receiving route | the consumer's object storage |

### 1. Push, as provider

- A dataset offers push when its entry in the datasets file has a
  distribution with `format` `HttpData-PUSH` (the name EDC and Siglet
  use, next to the `HttpData-PULL` Portitor already uses). No new
  setting.
- The transfer request must carry a `dataAddress` with `endpointType`
  `https://w3id.org/dspace-sig/profile/http-push`, an `endpoint`, and,
  if `authorization` is present, `authType` `bearer`. Anything else is
  refused with 400 before a flow starts. The `endpoint` must be
  `https://` unless `PORTITOR_ALLOW_HTTP` is `true`, as for every
  address a partner gives (the profile says a receiving party MUST
  reject a non-HTTPS endpoint).
- The provider sends the start message (no `dataAddress`, as DSP says
  for push), then its data plane reads the dataset's source and sends it
  as the body of one `POST` to the `endpoint`, with `Authorization:
  Bearer <token>` and the source's `Content-Type`. The profile does not
  name a method; `POST` is what EDC's push test sends.
- A 2xx answer: the provider sends the completion message. Any other
  answer, or no answer: the provider terminates the transfer with the
  reason (status code only; the URL and token are not logged).
- The consumer's token and endpoint are kept in the SQLite file until
  the transfer ends, like the consumer's pull token (ADR 0014), so a
  restart can finish or terminate the transfer. They are never logged
  or shown in the management API.
- Suspend or terminate, from either side, cuts a push that is running.
  Starting again sends the whole dataset again from the beginning. A
  start-again message from the consumer may carry a new `dataAddress`
  (a fresh token); it replaces the old one.
- The `endpoint` is an address the partner chose. Calling it carries the
  same risk as callbacks ([R-17](../RISKS_AND_TECHNICAL_DEBTS.md#r-17));
  the same 1.x rule will apply to it.

### 2. Push, as consumer: the receiving route

- `POST /v1/transfers` accepts `format` `HttpData-PUSH` when the
  consumer has object storage configured (below); otherwise 400.
- The connector builds the `dataAddress`: `endpoint`
  `{PORTITOR_PUBLIC_URL}/push/{consumerPid}`, `endpointType` as above, a
  bearer token of 32 random bytes from `ring` (base64url), of which only
  the SHA-256 hash is stored, as for pull tokens (ADR 0013). The token
  lives `PORTITOR_DATA_TOKEN_SECONDS` and only while the transfer is
  `REQUESTED` or `STARTED`.
- New public route `POST /push/{consumerPid}` (also `PUT`, since the
  profile does not name a method). It checks the token, then streams
  the body into the consumer's bucket as object `{consumerPid}` with S3
  multipart upload. A second request replaces the object. Every refusal
  gets the same 401, as on `/data`.
- The storage keys stay in the consumer's own configuration. Nothing
  S3-specific crosses between participants.
- The transfer view in the management API gains an optional
  `objectKey` once data has arrived. The operator's system completes the
  transfer as today, or the provider's completion message does.

### 3. Pull from the provider's object storage

- `portitor:source` may be an `s3://{bucket}/{key}` URI, the form the
  AWS command line and SDKs use, next to an `http(s)://` URL.
- The data plane reads the object with a signed S3 `GET` and streams it
  to the consumer exactly as for an HTTP source. The consumer sees the
  1.0 pull, so EDC and 1.0 consumers need nothing new.
- The provider's keys never leave the provider.

### 4. Object storage settings

One S3-compatible store per connector, used for both the provider's
sources and the consumer's received data. The names are the standard
AWS SDK environment variables, not new ones:

| Variable | Meaning |
| --- | --- |
| `AWS_ENDPOINT_URL_S3` | The store's address, for example a RustFS server. Without it, AWS S3 for `AWS_REGION` |
| `AWS_REGION` | Region used in the signature (RustFS accepts any; `us-east-1` by convention) |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | The keys. They follow the AWS SDK rule, not Portitor's `_FILE` rule; DEPLOYING says to inject them from a secret store |
| `PORTITOR_PUSH_BUCKET` | Bucket where pushed data lands. Without it the connector does not offer to receive push |

Requests are signed with AWS Signature Version 4, written with `ring`
(HMAC-SHA256, SHA-256), already a dependency, and tested against the
AWS test vectors and RustFS. Path-style addresses
(`{endpoint}/{bucket}/{key}`) are used, which RustFS and MinIO accept.

### 5. Revoking access

- The provider's `/data` route stays the only door to the provider's
  data, whatever the source.
- New in 1.1: suspend or terminate cuts every download of that flow
  that is already running. In 1.0 a running download goes on to its end
  (measured for token expiry, BENCHMARKS "Data plane").
- Data that already reached the consumer, by pull or push, cannot be
  taken back by any protocol. Deleting copies would be an ODRL duty
  ([TD-12](../RISKS_AND_TECHNICAL_DEBTS.md#td-12)).

### 6. Audit line

Each finished pull, push sent and push received writes one log line at
INFO, `data transfer done`, with `flow`, `agreement`, `counterParty`
(DID), `bytes`, `outcome`. It uses the existing JSON log (ADR 0023);
no new format ([TD-31](../RISKS_AND_TECHNICAL_DEBTS.md#td-31), first
half).

### 7. The provider can always end access (maintainer, 2026-11-24)

The maintainer chose both of the following on 2026-11-24. In 1.0 the
provider had no management route for its own transfers, and `/data`
checked only the flow's token, so a consumer whose membership was
revoked could go on pulling until the token expired (one hour by
default).

1. **Provider management routes:** `GET /v1/provided-transfers`,
   `GET /v1/provided-transfers/{providerPid}` and
   `POST /v1/provided-transfers/{providerPid}/termination`. A termination
   sends the DSP termination message and cuts running downloads and
   pushes at once. Suspend stays out of 1.1 (maintainer, D5).
2. **The data plane checks the consumer's membership on every
   request.** When a flow starts, the provider keeps, with the flow, the
   status entry (`credentialStatus`) and end of validity of the
   `MembershipCredential` the consumer presented, and its issuer. Every
   `/data` request, and every push, checks again that the issuer is
   still in `PORTITOR_TRUSTED_ISSUERS`, the credential has not expired,
   and its status bit is not set, with the status list cache of ADR 0016
   (`PORTITOR_STATUS_LIST_CACHE_SECONDS`, default 300, `0` fetches the
   list on every check). A running download or push is checked again at
   the same interval and cut when the check fails; the flow is then
   terminated with the reason. No DCP token is needed on `/data`, so the
   consumer side does not change. A check that cannot finish (status
   list unreachable) refuses, as ADR 0016 (amended in 0.11) does.
   The worst-case delay between a revocation and the refusal is the
   status list cache time.

**As built in 1.1 (2026-11-24), for review.** Details the text above
leaves open, decided while building it:

- The memberships are kept in their own table (`flow_membership`,
  migration 12), keyed by the flow id, which is the provider pid. They
  are written when the transfer request is accepted, before the start
  message (and so the token) goes out. Every kept membership must pass,
  as at the DSP check (DCP 5.4.3).
- A failed check (revoked, expired, issuer no longer trusted) refuses
  with 401 and ends the transfer as the provider's own termination,
  with a WARN line. A check that cannot finish refuses, or cuts the
  download, but keeps the transfer: the next request may pass.
- "At the same interval" is `PORTITOR_STATUS_LIST_CACHE_SECONDS`
  clamped to 5 to 60 seconds, so a cache of 0 does not check a running
  download in a busy loop and a long cache still checks every minute.
  The worst case is therefore the cache time for a new request, and up
  to the cache time plus 60 seconds for a running download.
- Flows that started before 1.1 have nothing kept; the migration marks
  them. They are served as in 1.0 until their token expires, logged
  once per flow and process. A 1.1 flow with nothing kept is refused.
- The provider's termination (route 1) replaces a message still on its
  way, unlike the consumer's own steps: access must end now. Suspend
  and terminate cut running downloads; completion does not.
- The membership check is `MembershipGate::check` and the cut switch is
  `RunningFlows`; both are there for push to call.

`PORTITOR_DATA_TOKEN_SECONDS` keeps its default of one hour, the same as
Eclipse Siglet's default token life (`exp` = `iat` + 3600). With the two
checks above, revocation no longer depends on it; it only limits how
long a leaked token is of use.

## Consequences

- Push works with any consumer that follows the HTTP Transfer Profile,
  and with Portitor as consumer into any S3-compatible store.
- Portitor as consumer cannot receive an EDC push today: EDC has no
  production push data plane. EDC as consumer can ask Portitor to push
  to an endpoint; the edc-interop scenario tests that.
- The profile is a draft, like Data Plane Signaling itself
  ([R-06](../RISKS_AND_TECHNICAL_DEBTS.md#r-06)).
- Pull keeps `https://w3id.org/idsa/v4.1/HTTP` as its `endpointType`,
  although the profile now names `.../http-pull`: changing it would break
  1.0 clients and the EDC interop test. To look at again for 2.0.
- R-38 is closed by not needing an S3 address; S3 stays inside each
  participant.
- The consumer now holds data in its connector process for a moment
  (streamed, not buffered whole). ADR 0014's "the consumer has no data
  plane" no longer holds when push is used.
