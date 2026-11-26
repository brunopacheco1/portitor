<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# hdab-intermediary: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on.

## 2026-11-26, for 1.2.0

- `compose.yaml` names the 1.2.0 release images by default.

- The README gives the release numbers of the roadmap as renumbered for
  1.2: the data permit and purpose come in 1.3 (was 1.2), a link between
  agreements in 1.4 (was 1.3;
  [R-39](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#r-39)). Nothing else
  changed; the scenario was not run again.

## 2026-11-24, for 1.1.0

- Step 1 by push (ADR 0027), the way `scenarios/two-party` does with
  `S3=1`: the lab runs its own object storage, `lab-storage`
  (`rustfs/rustfs:1.0.6`, S3 API on 127.0.0.1:18114), and its connector
  gets `AWS_ENDPOINT_URL_S3`, `AWS_ACCESS_KEY_ID`,
  `AWS_SECRET_ACCESS_KEY` and `PORTITOR_PUSH_BUCKET=lab-genotypes`.
  `run.sh` makes the keys for each run (`run/lab/s3.env`). Lux Biobank's
  genotypes also offer `HttpData-PUSH`. `steps.sh` creates the bucket,
  asks for a push transfer, waits for `COMPLETED` with `objectKey` and
  no data address, reads the object back and compares it with
  `genotypes.vcf`, and checks that the lab's receiving route refuses a
  request without the token (401, from Schuman's app). `lab-app` and
  `run/lab/storage` are gone. The diagrams show the push; the scenario
  diagram's messages after step 1 are renumbered, and its last message
  shows the TD-47 ending.
- The scenario now needs 1.1 connector images. `compose.yaml` names
  the 1.1.0 release images by default; `run.sh` stops before starting
  anything when the connector image is 1.0.
- Not run yet: written without a container engine. Scripts checked with
  `bash -n`, `compose.yaml` parsed as YAML, the diagrams rendered.
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

- 1.2: purpose and data permit; step 5's gap closes.
- 1.3: the HDAB's offers tied to its own agreement (R-39).
