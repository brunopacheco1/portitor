<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# gaiax-compliance: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on. Before 2026-11-24 it was `scripts/gaiax-e2e.sh` with
`interop/gaiax/`; the repository's [CHANGELOG](../../CHANGELOG.md) has
those entries in full.

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
