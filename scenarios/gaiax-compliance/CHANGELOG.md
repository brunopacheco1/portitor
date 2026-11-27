<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# gaiax-compliance: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on. Before 2026-11-24 it was `scripts/gaiax-e2e.sh` with
`interop/gaiax/`; the repository's [CHANGELOG](../../CHANGELOG.md) has
those entries in full.

## 2026-11-26, for 1.3.0

- The Portitor components keep their state in PostgreSQL (1.3,
  [ADR 0029](../../docs/adr/0029-several-replicas.md)): `compose.yaml`
  has a `database` service (`postgres` 19, pinned by digest) with one
  database and role per component, made by
  `examples/postgres/init-databases.sh`. `run.sh` writes a test CA, the
  server's certificate and one password per component into
  `run/database/` (`scripts/demo-database.sh`); the components connect
  with TLS, checking the certificate and the host name, and read their
  password from a Compose secret. Images before 1.3 ignore
  `PORTITOR_DATABASE_URL` and keep their SQLite file. The steps are
  unchanged.
  Not run with the change yet.

## 2026-11-24, on 1.0.0-rc.1

- Moved to `scenarios/gaiax-compliance/`, with this README and
  changelog. The compliance service and its graph database moved to
  `examples/gaiax/compliance.yaml`, shared with edc-interop. What it
  checks is unchanged.

## 2026-11-20, 0.13.0

- Runs with Docker or Podman; shares `scripts/gaiax-lib.sh` (test CA,
  test notary, a participant's Gaia-X files) with the EDC interop test.

## 2026-11-15, 0.8.0

- First version: Portitor identities get a compliance credential from a
  local gx-compliance v2.16.0 in test mode (ADR 0021), and a Portitor
  provider agrees only with a holder of one (ADR 0022). A test CA and a
  test notary. In CI on pull requests and nightly.
