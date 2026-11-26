<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0028: A separate, pluggable data plane over Data Plane Signaling

- Status: **Accepted** (2026-11-25, by the maintainer: all five open
  decisions as recommended, listed below)
- Date: 2026-11-25
- Scope: a decision on paper. The research, numbers and sources are in
  the [spike](../spikes/1.x-separate-data-plane.md). Amends
  [ADR 0003](0003-component-split.md) (an optional fourth image) and
  [ADR 0027](0027-push-transfers-and-object-storage.md), section 7 (who
  re-checks membership). Answers [TD-17](../RISKS_AND_TECHNICAL_DEBTS.md#td-17)
  and [TD-48](../RISKS_AND_TECHNICAL_DEBTS.md#td-48).

## Context

The built-in data plane runs inside the connector. Since 1.1 it serves
HTTP pull, pushes over HTTP, reads and writes S3-compatible object
storage, and re-checks the consumer's membership on every use. It is
4,751 lines, 18 % of the connector. Adding a source or a transfer type
(Kafka, SFTP, a database) means Rust code and a Portitor release.

The maintainer wants (2026-11-25): separate responsibilities; the
connector governs data ingress and egress and the life of each transfer
job; operators add sources and transfer types without changing the
connector.

Data Plane Signaling v1.0-RC4 defines this: data planes register with
a control plane (`dataplaneId`, `endpoint`, `profiles`, `labels`,
`authorization`), and the control plane drives each data flow over
HTTP. EDC removed its own data plane after 0.18 and works only this way;
it selects a data plane by profile and labels and passes the source in
`metadata`. SDKs exist in Java, Go and Rust. Portitor already uses the
DPS messages, as Rust calls inside one process.

[STABILITY](../STABILITY.md) promises `/data` and `/push` on the
connector's public port, and the `PORTITOR_*` settings, for all of 1.x.

## Decision

1. **The control plane drives every data plane through DPS.** It serves
   the DPS control plane endpoints and calls data planes at their
   registered `endpoint`. No Portitor-specific plug-in interface.
2. **Operators register data planes** by configuration and by
   `PUT`/`DELETE /dataplanes` on the management port, with the DPS
   `DataPlaneRegistrationMessage`. A registration needs the
   `oauth2_client_credentials` authorization profile, unless a
   development setting allows none.
3. **Selection follows EDC:** a data plane that supports the transfer's
   profile and has the labels the dataset asks for; if several, any.
   The chosen id is kept on the transfer. The catalog offers the
   profiles of the registered data planes.
4. **The built-in data plane becomes a DPS data plane like the others**,
   in its own crate, with no access to the control plane's stores or
   credential checks. It gets the source of the dataset in `metadata`.
   The connector links it in by default (behaviour of 1.1, STABILITY
   kept). The same crate also ships as an optional image,
   `portitor-dataplane`.
5. **The control plane owns policy, credentials and revocation.** It
   re-checks the consumer's membership (ADR 0027, section 7.2) and
   sends `terminate`; data planes move bytes or grant access, and
   report `completed` or `errored`.
6. **Signals go out after the transfer's write**, through the outbox
   (ADR 0007), never inside the retried update.
7. **Order of work:** untie inside the process (no change of
   behaviour); signaling over HTTP and registration; the separate image
   and the consumer data plane; interop with an outside data plane and
   with an EDC control plane, and the DPS TCK.

## Consequences

- New transfer types and sources come as separate services in any
  language. Portitor routes by profile name and defines none.
- A fourth image exists but is optional. Default deployments do not
  change in 1.x. Making it required is a 2.0 question.
- The signaling API is a new surface that can change transfer states.
  It stays on the management port and requires authorization.
- ADR 0027's "the provider can always end access" holds fully for the
  built-in data plane. For an outside data plane, Portitor sends
  `terminate` and relies on the data plane to honour it; DEPLOYING says
  so.
- We follow DPS release candidates (R-06): each change goes in the
  changelog.

## Decisions taken (maintainer, 2026-11-25)

The five open decisions of the
[spike](../spikes/1.x-separate-data-plane.md#open-decisions-for-the-maintainer),
all as recommended:

1. **Default in 1.x:** the built-in data plane stays linked into the
   connector. `portitor-dataplane` is optional. Requiring it is a 2.0
   question.
2. **Registration:** both by configuration and by `PUT`/`DELETE
   /dataplanes` on the management port.
3. **Signaling security:** the `oauth2_client_credentials`
   authorization profile is required. A registration without one is
   refused, unless a development setting allows it.
4. **Revocation with outside data planes:** accepted that ADR 0027,
   section 7 holds only as far as an outside data plane honours
   `terminate`. DEPLOYING says so.
5. **When:** step 1 (untie inside the process, no change of behaviour)
   when it fits in a 1.x release. Steps 2 to 4 later in 1.x, or after
   DPS 1.0 is released.

## Note added in 1.2 (2026-11-26): step 1 is done

Release 1.2 did step 1 of decision 7: the data plane is untied inside
the process, with one accepted change of behaviour.

- The built-in data plane is the crate `portitor-dataplane`
  (decision 4). The connector links it in and builds it in
  `portitor-connector/src/data_plane.rs`. CI checks with `cargo tree`
  that the crate does not depend on `portitor-connector`.
- The membership check is a port of the data plane, `AccessGate`
  (`portitor-dataplane/src/access.rs`), and the control plane
  implements it: `MembershipGate` in
  `portitor-connector/src/flow_membership.rs` reads the memberships
  kept with each flow (the connector's `MembershipStore` port) and
  checks them again (decision 5; maintainer's decision D1 for 1.2). The
  data plane still asks on every request and while a download or a
  push runs, as in 1.1. Data Plane Signaling has no message for this
  question; how a data plane in its own process asks is left to step 2.
- The control plane sends the dataset's source in the `metadata` of the
  start message, as `{"portitor:source": "<url>"}` (decision 4;
  maintainer's decision D2 for 1.2). Data Plane Signaling v1.0-RC4
  gives `metadata` no fixed shape and names no key for a source, so
  the key is the one of the datasets file. The flow keeps the source
  until it ends: a source changed by a later harvest reaches only new
  transfers. This is the accepted change of behaviour.
- The flow also keeps the agreement and the consumer of its start
  message (`agreementId`, `counterPartyId`), so the audit line no
  longer reads the transfer store.
- Decision 6: the provider's transfer service records the data plane
  step in the transfer's own write (`transfer.flow_step`) and runs it
  right after that write wins, once, one step at a time; our DSP
  message waits until the step has run. This is the outbox pattern of
  ADR 0007 applied to the data plane step, in the same call rather
  than by the outbox's sender
  (`portitor-dsp/src/transfer/provider/flow_steps.rs`;
  [TD-48](../RISKS_AND_TECHNICAL_DEBTS.md#td-48), closed).
