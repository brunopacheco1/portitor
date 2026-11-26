<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# data-permits: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on.

## 2026-11-26, for 1.2.0

- `compose.yaml` names the 1.2.0 release images by default.

- The README gives the release numbers of the roadmap as renumbered for
  1.2: purpose, the data permit and the fixes of
  [R-48](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#r-48),
  [TD-20](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-20) and
  [TD-46](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-46) are planned for
  1.3 (was 1.2). Nothing else changed; the scenario was not run again.

## 2026-11-25, for 1.1.0

- `compose.yaml` takes the 1.1.0 release images by default. The
  scenario itself is unchanged; it does not use the 1.1 features.

## 2026-11-24, for 1.0.0

- `compose.yaml` takes the 1.0.0 release images by default. The
  scenario itself is unchanged; its last run was on 1.0.0-rc.1, whose
  code 1.0.0 keeps.

## 2026-11-24, on 1.0.0-rc.1

- On its own: its own `compose.yaml`, `run.sh` and datasets, and only
  what it needs (Digital Lux Trust, the HDAB's issuer, Lux Biobank,
  Schuman Clinical Research: 8 containers instead of 14). The lab, the
  HDAB's connector and its pipeline are gone. So step 5 no longer
  revokes the HDAB's membership: the HDAB is not a participant here.
  Lux Biobank's dataset has only the offers this scenario uses.
- Moved to `scenarios/data-permits/`, with its own README and this
  changelog; steps 7 to 11 of the health demo are now steps 1 to 5.
- First version, the same day: the HDAB's own issuer grants permits
  P-2026-001 and P-2026-002 as `DataPermitCredential`s (placeholder
  claims `permitId` and `purpose`); Lux Biobank has one offer per
  permit; one revoked permit blocks the other until deleted; an issuer
  trusted for permits can issue memberships too.
- Findings: R-48, TD-46, and TD-20 made wider.

## Next

- 1.2: a published permit format, if the EHDS template is out, and an
  ODRL `purpose` constraint.
- 1.1 and 1.2: the data sent where the permit says (push and a
  destination check), the "HDAB as permit authority" scenario of the
  roadmap.
