<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0013: Transfer process and the built-in data plane

- Status: **Accepted** (decided by the maintainer, 2026-11-13)
- Date: 2026-11-13
- Scope: release 0.4 (transfer process and HTTP pull data plane). Builds on
  ADR 0003 (data plane in the connector, behind Data Plane Signaling),
  ADR 0007 (outbox), ADR 0009 (minimal ODRL).
- Decisions already taken by the maintainer: pull transfers only in 0.4
  (2026-11-12); keep `Role` and add the agreement identity check on top
  (2026-11-13); a dataset's data comes from an upstream HTTP URL per
  dataset (2026-11-13).

## Context

A transfer gives a consumer access to a dataset under an agreement
(DSP 2025-1, "Transfer Process Protocol"). The consumer sends a
`TransferRequestMessage` with an `agreementId` and a `format`. The provider
must decide whether to accept it, then start the transfer: for a pull
transfer its `TransferStartMessage` carries a `dataAddress` with the
endpoint and an access token. Afterwards either party may suspend,
start again, complete or terminate.

DSP leaves open what the provider checks before it accepts. Data Plane
Signaling v1.0-RC4 (the internal interface, crate `portitor-dps`) leaves
open how the control plane and the data plane stay in step.

## Decision

### What the provider accepts

A `TransferRequestMessage` is accepted only if all of these hold. Any
failure is a `TransferError` (HTTP 400) and no transfer is created.

1. `agreementId` names an agreement from one of the provider's own
   negotiations, and that negotiation is FINALIZED.
2. The agreement's `assignee` is the caller. Until DCP authentication
   (0.5) every caller is the placeholder `urn:portitor:unauthenticated`,
   so this check only becomes effective then; the code is in place now.
   Unknown agreement and another consumer's agreement get the same answer,
   so a caller cannot learn which agreement ids exist.
3. The agreement's permissions still hold now (ADR 0009), for example its
   end date has not passed.
4. `format` is the format of one of the agreed dataset's distributions,
   and the built-in data plane supports it. In 0.4 that is `HttpData-PULL`
   only (the name EDC uses, which the DSP TCK sample configuration uses
   too); it maps to the Data Plane Signaling profile `http-pull`.
5. No `dataAddress` (that would be a push transfer; not in 0.4).
6. `callbackAddress` is present.

### Starting

By default the provider starts every accepted transfer at once (a
`TransferDecider` port, like the negotiation `Decider`, allows other
behaviour; the TCK build scripts it). Starting means:

1. Ask the data plane to `start` (Data Plane Signaling `DataFlowStartMessage`,
   `dataFlowId` = the provider's process id, profile `http-pull`). The data
   plane answers STARTED with the `DataAddress`.
2. Save the transfer as REQUESTED with the `TransferStartMessage` (carrying
   that address) as its pending message (outbox, ADR 0007).
3. Send it; the transfer is STARTED when the consumer acknowledges it.

If the data plane refuses, the provider sends a `TransferTerminationMessage`
instead. If the save fails, the data flow is terminated.

### Keeping the data plane in step

The transfer process (control plane) is the record of truth. For each
later message (suspend, start again, complete, terminate), from either
party, the control plane signals the data plane **first** and saves the
new transfer state **after**:

- A signal the data plane has already applied (the flow is already in the
  target state) counts as done, so a repeated step is harmless.
- If the save fails after the signal, the data plane is one step ahead.
  For suspend, complete and terminate that only removes access; it never
  grants access the transfer does not allow. For "start again" the caller
  gets an error and may retry.

### Suspend and start again (pull)

While a transfer is SUSPENDED, the data plane refuses its token. Starting
again makes the same token valid again, so the provider does not need to
send a new `dataAddress`. This fits the TCK, which expects no start message
from the provider after the consumer restarts a transfer (TP:02-04).

Data Plane Signaling says only the party that suspended may resume; DSP
2025-1 lets either party start a suspended transfer again. Portitor follows
DSP, which governs the exchange between connectors, and reports the
difference upstream (ROADMAP 0.4). Reported upstream: https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/135

### The data

Each dataset in the datasets file may name an upstream HTTP URL. The data
plane fetches it and streams the answer to the consumer that presents a
valid token. The URL is never shown in the catalog. A dataset without one
cannot be transferred: the request is refused with 400 before any data
flow starts. Settled with the data plane (0.4 PR 4):

- The URL is `portitor:source` in the dataset's entry of the datasets
  file. Plain `http://` is allowed, because the operator sets it.
- The data address is `{public URL}/data/{flowId}` with a bearer token
  (`authorization`, `authType: bearer`), 32 random bytes from `ring` in
  base64url. Only its SHA-256 hash is stored, with the flow in the SQLite
  file.
- The token works while the flow is STARTED and until
  `PORTITOR_DATA_TOKEN_SECONDS` (default one hour) after the start. It is
  not renewed: DSP 2025-1 has no message for it, so the consumer asks for
  a new transfer.
- Every refusal (no token, wrong token, unknown flow, suspended, ended,
  expired) gets the same 401, so a caller learns nothing about other
  flows.
- The data plane sends a plain `GET` to the source, with no caller
  headers and no credentials, and passes back a 2xx body as a stream with
  its `Content-Type`. Anything else is a 502; the log names the flow, not
  the URL. The source must start its answer within 30 seconds.

### `endpointType` in the `dataAddress`

`https://w3id.org/idsa/v4.1/HTTP` (settled in 0.4 PR 4). DSP's own
examples, the DSP TCK and EDC's DSP tests use it. The Data Plane Signaling
HTTP profile uses `https://w3id.org/dspace-sig/profile/http-pull` between
control plane and data plane; that value stays inside Portitor. The EDC
interop test (0.5) confirms the choice. Reported upstream:
https://github.com/eclipse-dataplane-signaling/profiles/issues/8

### Amended in 0.12: a repeated transfer request starts nothing new (2026-11-18)

The 2026-11-18 review of specifications and code quality (D3,
[TD-34](../RISKS_AND_TECHNICAL_DEBTS.md#td-34)) found that a
`TransferRequestMessage` sent again, for example after a timeout, started
a second transfer with a second data token. The maintainer decided the
change on 2026-11-18. DSP 2025-1, "Transfer Request Message": "Providers
SHOULD implement idempotent behavior for Transfer Request Messages based
on the value of `consumerPid`."

- The provider looks a request up by the **caller's verified DID and the
  `consumerPid`**. If that caller already has a transfer with that
  `consumerPid`, the answer is that transfer as it is now (HTTP 201 with
  the `TransferProcess`: the same `providerPid`, its current state).
  Nothing else happens: no data flow, no token, no message.
- The request is checked first (agreement held by the caller, policy,
  format), then looked up. Eclipse EDC 1.0.0-RC7 uses this order and
  this answer (`TransferProcessProtocolServiceImpl.requestedAction`,
  `findForCorrelationId`). So a repeated request gets 400, not the
  transfer it already has, once the agreement's permissions no longer
  hold or the dataset is no longer offered in the asked format.
- DSP adds that the provider "SHOULD respond with an appropriate
  Transfer Start Message". Portitor does not send the start message a
  second time, and neither does EDC: only the token's hash is kept, so
  the first data address cannot be sent again, and a new one would be the
  second token this change avoids. A start message that was never
  acknowledged is still in the outbox and is sent again by its own rules
  (ADR 0007).
- **A `consumerPid` belongs to one consumer.** Another participant that
  uses the same value never gets, or learns of, the first one's transfer:
  with its own agreement it gets a transfer of its own, and with the
  first consumer's agreement it is refused as before (400, "no finalized
  agreement with this id for the caller").
- **A repeat must be the same request.** If the same caller sends the
  same `consumerPid` with another `agreementId`, `format` or
  `callbackAddress`, the request is refused with HTTP 400. DSP does not
  cover this case. EDC answers with the existing transfer whatever the
  message holds; Portitor refuses, as it refuses other mismatched ids, so
  that a consumer never takes the answer for a transfer it did not ask
  for.
- There is no time limit: a `consumerPid` is recognised as long as the
  transfer is stored, also after it ended (DSP: providers "MAY choose to
  implement idempotent behavior for a certain period of time").
- Storage: SQLite schema version 9 adds an index on
  (`counter_party`, `consumer_pid`) to the `transfer` table. It is not a
  UNIQUE index, because a file written before 0.12 may already hold
  several transfers for one repeated request and must still open; for
  such a file the transfer stored first answers. New rows are kept unique
  by the insert itself (one statement that adds the row only if there is
  none for that consumer and `consumerPid`), so two copies of a request
  that arrive together give one transfer.

## Consequences

- No transfer without a finalized agreement held by the caller; once DCP
  is in (0.5), a leaked agreement id alone is not enough.
- The provider's transfer store follows the negotiation store: SQLite,
  outbox, version check (ADR 0006, ADR 0007).
- A data plane signal and the transfer save are not one transaction. The
  order above makes a failure between them reduce access, except for
  "start again", which the caller retries.
- Push transfers, provider-initiated suspend and complete from the
  management API, and a separate data plane process are later work.
