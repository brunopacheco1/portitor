<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# two-data-spaces: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on.

## 2026-11-26, for 1.3.0

- With `PORTITOR_TEST_DATABASE_URL` each component keeps its state in a
  database of its own on that PostgreSQL server (1.3,
  [ADR 0029](../../docs/adr/0029-several-replicas.md);
  `scripts/test-databases.sh`), dropped at the end; without it, a
  SQLite file as before. The checks are unchanged. Both modes passed on
  PostgreSQL 16 with the 1.3 code.

## 2026-11-24, on 1.0.0-rc.1

- Moved from `scripts/two-data-spaces.sh` to
  `scenarios/two-data-spaces/run.sh`, with this README and changelog.
  What it checks is unchanged.

## 2026-11-20, 0.13.0

- First version: one organisation in two data spaces with one connector
  each (ADR 0024), in two modes, `shared` and `separate`. Run by hand,
  both modes passed.
