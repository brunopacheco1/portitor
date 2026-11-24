<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0014: Transfers as consumer

- Status: **Accepted** (maintainer, 2026-11-13)
- Date: 2026-11-13
- Scope: release 0.4 (transfer process, consumer side, and the provider's
  own steps). Builds on ADR 0007 (outbox and proof of delivery) and
  ADR 0013 (transfer process as provider, built-in data plane).

## Context

ADR 0013 covers the provider. A connector is also a consumer: it asks a
provider for a dataset under an agreement it negotiated, receives a data
address with an access token, and someone fetches the data.

DSP 2025-1 ("Transfer Process Protocol") lets either party suspend, start
again, complete or terminate a transfer, and says for the
`TransferStartMessage`: "The dataAddress MUST be provided if the current
transfer is a pull transfer". It does not say who on the consumer side
fetches the data, nor how the consumer keeps the token.

## Decision

### Requesting a transfer

- A transfer is requested only under an agreement of one of the
  consumer's own negotiations that reached `FINALIZED` (port
  `HeldAgreements`). The provider's DSP address and participant id come
  from that negotiation, not from the caller, so the management API cannot
  be used to make the connector call an arbitrary address.
- Pull only (as ADR 0013): the request carries `format` (default
  `HttpData-PULL`) and no `dataAddress`.
- The transfer is saved before the request is sent, in state `REQUESTED`
  without a provider process id, with the request as its pending message
  (outbox, ADR 0007). A provider message that arrives before the
  acknowledgement proves the request arrived. If the request cannot be
  delivered at all, the transfer ends locally (`TERMINATED`) without a
  message, as for negotiations.

### Who fetches the data

The operator's own system fetches it, not Portitor. The management API
returns the transfer with its `dataAddress` once it is `STARTED`; the
system sends `GET <endpoint>` with the `authorization` property as a bearer
token, then completes the transfer through the management API. Portitor
does not download or store data as consumer. That keeps large data out of
the connector and leaves where data goes to the operator.

### Keeping the token

- The consumer must present the token, so it cannot keep only a hash (as
  the provider does). The data address, token included, is kept in the
  SQLite file while the transfer can still be used (`STARTED` or
  `SUSPENDED`) and removed when it ends (`COMPLETED` or `TERMINATED`).
- The first start message must carry a data address (DSP, quoted above);
  one without it is refused. A later start (after a suspension) may carry
  a new address, which replaces the old one; without one, the old one is
  kept.
- The management API shows the token to callers with the API key. The
  SQLite file and the API key must be protected alike (DEPLOYING.md).
  Encrypting the token in the file is later work.

### The consumer's own steps

- Default policy (port `ConsumerTransferDecider`): never act on its own
  (`OnlyWhenAsked`). The operator's system completes or terminates a
  transfer through the management API. Suspend and start again from the
  management API are later work; the domain supports them.
- A step is refused while another message of ours is still on its way, or
  when the state machine does not allow it.
- If the provider cannot be reached for a later message, the transfer is
  terminated, and the termination is sent (errors answering a termination
  are ignored, as DSP allows).

### The provider's own steps

The provider's policy (`TransferDecider`, ADR 0013) may now also suspend,
start again, complete or terminate a transfer it runs, not only start it.
The data plane is told first, as ADR 0013 says.

When the provider starts a suspended pull transfer again, DSP requires a
data address in its start message. The provider keeps only the token's
hash, so it cannot send the old token again: its built-in data plane gives
the flow a new token with a full lifetime (`NewAddress`), and the start
message carries it. Data Plane Signaling v1.0-RC4 has no message for
this; it is a Portitor extension of the built-in data plane, listed with
the other Signaling findings for the upstream report (ARCHITECTURE.md).
Reported upstream: https://github.com/eclipse-dataplane-signaling/dataplane-signaling/issues/135
When the consumer starts the transfer again, the token stays the same and
no start message is sent back (ADR 0013).

### DSP TCK

- TCK v1.0.2 disables TP:02-04 and TP_C:02-04 (suspend, then start again
  by the consumer). The other 15 provider and 15 consumer transfer tests
  must pass in CI.
- The TCK's consumer creates its transfer before it sends the request,
  but moves it to REQUESTED only after it reads our answer, and its
  handler for the start message removes itself after the first start that
  arrives, even one it refused. A start message sent at once can arrive
  before that answer is read, is refused, and every retry then gets 404
  (wording corrected 2026-11-13 from the TCK v1.0.2 source). The TCK build therefore
  sends start messages 300 ms late (`tck::LateStart`). A real consumer,
  Portitor included, saves its transfer before it sends the request, so the
  release build sends at once. The race is listed for an upstream report.
Reported upstream: https://github.com/eclipse-dataspacetck/dsp-tck/issues/338

### Amended in 1.1: push transfers into the consumer's object storage (proposed 2026-11-24)

[ADR 0027](0027-push-transfers-and-object-storage.md) adds push
(`HttpData-PUSH`): the request carries a `dataAddress` built by the
connector, and a new public route `POST /push/{consumerPid}` receives
the data and streams it into the consumer's own S3-compatible bucket.
"Pull only" and "the consumer side has no data plane" below hold only
for pull transfers. Suspend and start again are added to the management
API (`POST /v1/transfers/{consumerPid}/suspension` and `/start`,
[TD-11](../RISKS_AND_TECHNICAL_DEBTS.md#td-11)).

Built in 1.1 (2026-11-24): this replaces "Suspend and start again from
the management API are later work" under "The consumer's own steps".
Both calls follow the rules of completion and termination: refused
(400) while another message of ours is on its way or when the state
machine does not allow the step, and the message is saved in the outbox
before it is sent (ADR 0007). Start again works whichever party
suspended the transfer, as DSP 2025-1 allows (`next_state`); our start
message carries no `dataAddress`, the token stays the same, and the
provider sends no new start message (ADR 0013). Neither call takes a
body: the suspension carries a fixed reason, as the termination does.
The DSP TCK does not test this (TP:02-04 and TP_C:02-04 are disabled in
v1.0.2, above); the connector's own tests and the two-party scenario do.

## Consequences

- One shape for both sides and for negotiations: the domain decides, the
  adapter sends, every write compares a version, and pending messages are
  sent again after a restart.
- A data access token sits in the consumer's SQLite file while the
  transfer is usable. Whoever can read the file or holds the API key can
  fetch the data until the transfer ends or the token expires.
- The consumer side has no data plane of its own: no push transfers into
  Portitor, no caching of data.
