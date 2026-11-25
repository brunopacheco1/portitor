<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0028: A separate, pluggable data plane over Data Plane Signaling

- Status: **Proposed** (2026-11-25)
- Date: 2026-11-25
- Scope: a decision on paper. The research, numbers and sources are in
  the [spike](../spikes/1.x-separate-data-plane.md). Would amend
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

## Decision (proposed)

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

## Open questions

The five decisions listed at the end of the
[spike](../spikes/1.x-separate-data-plane.md#open-decisions-for-the-maintainer).
