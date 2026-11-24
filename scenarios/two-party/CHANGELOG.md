<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# two-party: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on. The repository's [CHANGELOG](../../CHANGELOG.md) has
the release notes these come from.

## 2026-11-24, for 1.0.0

- `compose.yaml` takes the 1.0.0 release images by default. The
  scenario itself is unchanged; its last run was on 1.0.0-rc.1, whose
  code 1.0.0 keeps.

## 2026-11-24, on 1.0.0-rc.1

- Moved to `scenarios/two-party/`, with a README, diagrams and this
  changelog. The scripts have the names every scenario uses:
  `demo-compose.sh` is `run.sh`, `demo-steps.sh` is `steps.sh`, and
  `demo.sh` is `run-local.sh`. What it checks is unchanged.

## 2026-11-23, on 1.0.0-rc.1

- Moved from `scripts/demo.sh`, `scripts/demo-compose.sh`,
  `scripts/demo-steps.sh` and `examples/compose/` into one folder.

## 2026-11-20, 0.13.0

- Runs with Podman as well as Docker (`CONTAINER_ENGINE`), with bash
  4.4 or later.

## 2026-11-15, 0.9.0

- The Compose version starts each component once those it needs are
  healthy (the images' `health` subcommand).
- An optional observability stack (OpenTelemetry Collector, Jaeger,
  Prometheus), with `EXTRA_COMPOSE`.
- It became the website's quickstart.

## 2026-11-15, 0.7.0

- The authority builds, signs and serves the HealthDCAT-AP demo bundle;
  the provider follows it and offers `examples/healthdcat-datasets.json`.
  The catalog shows `dct:conformsTo`; `bed-occupancy` is rejected with a
  SHACL report.
- The restricted dataset is transferred under its
  `MembershipCredential` constraint.

## 2026-11-14, 0.6.0

- `portitor-issuer` is the authority: it registers both participants,
  each identity asks it for its credentials, and it revokes them.
- A second credential type: the consumer's `DataProcessorCredential` is
  checked by an offer, and a new transfer is refused once it is revoked.
- Offers for members only and for auditors (a claim the credentials do
  not have).

## 2026-11-13, 0.5.0

- DCP: each participant has its own `portitor-identity` with a
  `MembershipCredential`; a catalog request without a token gets 401.

## 2026-11-13, 0.4.0

- A Compose version: provider, consumer and a web server for the data.
- A pull transfer: the file is fetched, checked and the transfer
  completed.

## 2026-11-12, 0.3.0

- First version, `scripts/demo.sh`: two connectors on one machine agree
  a contract and refuse changed terms. CI runs it on every pull request.
