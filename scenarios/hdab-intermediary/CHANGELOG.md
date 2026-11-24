<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# hdab-intermediary: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on.

## 2026-11-24, for 1.1.0

- Step 6: the HDAB's termination of Schuman's last transfer is now
  accepted (TD-47, fixed in 1.1), so `steps.sh` waits for `TERMINATED`
  instead of finding the transfer `REQUESTED` and ending it from
  Schuman's side. Not run yet on 1.1: the change follows the connector's
  tests of TD-47.

## 2026-11-24, for 1.0.0

- `compose.yaml` takes the 1.0.0 release images by default. The
  scenario itself is unchanged; its last run was on 1.0.0-rc.1, whose
  code 1.0.0 keeps.

## 2026-11-24, on 1.0.0-rc.1

- On its own: its own `compose.yaml`, `run.sh`, datasets and pipeline,
  without the HDAB's issuer of data-permits. Lux Biobank trusts only
  Digital Lux Trust, and its dataset has only the two offers this
  scenario uses. The genotypes moved to `examples/genotypes/`. What it
  checks is unchanged.
- Moved to `scenarios/hdab-intermediary/`, with its own README and this
  changelog.

## 2026-11-23, on 1.0.0-rc.1

- First version: steps 1 to 6. The lab peer to peer; the HDAB's
  agreement with Lux Biobank; Schuman refused by Lux Biobank; Schuman's
  counts through the HDAB's pipeline, which pulls only during Schuman's
  fetch; the known gap of step 5; an ended pilot offer and the HDAB's
  revoked membership. A manual test, not run in CI (maintainer,
  2026-11-15).
- Finding: a revoked party leaves its counter-parties' transfers
  `REQUESTED`.

## Next

- 1.1: push transfers into object storage for the lab.
- 1.2: purpose and data permit; step 5's gap closes.
- 1.3: the HDAB's offers tied to its own agreement (R-39).
