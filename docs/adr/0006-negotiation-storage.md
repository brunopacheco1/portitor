<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0006: Negotiation storage

- Status: **Accepted** (decided by the maintainer, 2026-11-12)
- Date: 2026-11-12

## Context

From 0.3 the connector keeps contract negotiations: their state, the
offer, the agreement and where to send the next message. Agreements are
the basis for data access, so they must survive a restart. The roadmap
first named PostgreSQL. A connector should still start with no other
service, so a single participant or a demo needs nothing but the image.

## Decision

- Negotiations are kept behind the `NegotiationStore` port in the domain
  (ADR 0004). The port has one write rule: `replace_if`, a
  compare-and-update on the previous state, so two messages that race for
  the same negotiation cannot both win.
- The default adapter is a **SQLite file** (`LDSC_DB_FILE`, default
  `ldsc.db`; `/data/ldsc.db` in the image). SQLite is compiled into the
  binary (`rusqlite`, `bundled`), so the static image needs no system
  library. WAL journal, `synchronous = FULL`.
- **One process per file.** Running several replicas needs the PostgreSQL
  adapter, which comes later on the same port and the same migrations
  (plain, portable SQL; schema version in `PRAGMA user_version`).
- An in-memory adapter exists for tests only.

### Amended in 0.12: three rules of the negotiation state machine (2026-11-18)

The 2026-11-18 review of specifications and code quality (D4, D5,
[TD-36](../RISKS_AND_TECHNICAL_DEBTS.md#td-36)) found three points about
the negotiations this ADR stores. The maintainer decided each on
2026-11-18. The rules are in the domain
(`crates/portitor-dsp/src/negotiation.rs` and `negotiation/`).

- **Who may terminate: either party, in every state that is not final.**
  DSP 2025-1, "Contract Negotiation Termination Message": "The message
  MAY be sent at any state of a Contract Negotiation". The state machine
  figure is narrower: it draws the step to TERMINATED from ACCEPTED and
  VERIFIED for the provider only, and from AGREED for the consumer only.
  Portitor follows the text (`next_state`), which is also the safer
  reading for interoperability. Eclipse EDC 1.0.0-RC7 does the same
  (`ContractNegotiation.canBeTerminated`: every state but FINALIZED). No
  code change; a test comment that named the figure is corrected.
- **As consumer, an agreement without a `timestamp` is refused** with
  HTTP 400 and a `ContractNegotiationError`; so is one whose `timestamp`
  is not an `xsd:dateTime`. DSP 2025-1, "Contract Agreement Message":
  "An Agreement MUST contain a `timestamp` property defined as an XSD
  DateTime type." The JSON Schema (`contract-schema.json`, `Agreement`)
  gives the property a pattern but does not list it under `required`, so
  the rule comes from the text. A time zone may be left out, as in
  `xsd:dateTime` and in the schema's pattern. Eclipse EDC refuses such an
  agreement too (`JsonObjectToContractAgreementMessageTransformer`).
  Until 0.11 Portitor accepted it.
- **As provider, a `ContractRequestMessage` on a running negotiation
  must name its `consumerPid`.** A request to
  `/negotiations/{providerPid}/request` whose `consumerPid` is not the
  one the negotiation started with is refused with HTTP 400 ("process
  ids do not match"), as every other message with a wrong process id
  already is. Until 0.11 this one message was not checked.

### Amended in 1.3: PostgreSQL, and SQLite deprecated (2026-11-26)

The text above is kept as it was decided. From 1.3 the PostgreSQL
adapter exists, on the same ports, and several processes may share one
PostgreSQL database. Its migrations are a list of their own, which
starts with the 1.3 schema as a whole, not the SQLite ones. SQLite is
deprecated: it still works for one process, a second process on the
same file refuses to start, and it is removed in 2.0. The decision and
its reasons are in [ADR 0029](0029-several-replicas.md).

## Consequences

- Operators mount a volume on `/data` to keep negotiations across
  containers.
- The binary grows by the SQLite library (about 1 MB).
- Messages being sent when the process stops are not sent again after a
  restart; the negotiation waits in its last state. An outbox that
  resumes them is a follow-up. (Addressed by
  [ADR 0007](0007-outbox-and-proof-of-delivery.md), which also replaces
  the compare on the previous state with a compare on a version number.)
- `rusqlite` 0.42 needs Rust 1.89, so the minimum Rust version moves from
  1.85 to 1.89.
