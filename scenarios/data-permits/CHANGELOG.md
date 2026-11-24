<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# data-permits: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on.

## 2026-11-24, on 1.0.0-rc.1

- Moved to `scenarios/data-permits/`, with its own README and this
  changelog; steps 7 to 11 of the health demo are now steps 1 to 5. It
  runs alone with its own `run.sh`, so step 5 now has Digital
  Lux Trust revoke the HDAB's membership itself.
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
