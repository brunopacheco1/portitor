<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# edc-interop: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on. Before 2026-11-24 it was `scripts/edc-interop.sh`
with `interop/edc/`; the repository's [CHANGELOG](../../CHANGELOG.md)
has those entries in full.

## 2026-11-24, on 1.0.0-rc.1

- Moved to `scenarios/edc-interop/`, with this README and changelog. It
  takes the Gaia-X compliance service from
  `examples/gaiax/compliance.yaml`. What it checks is unchanged.

## 2026-11-22, 1.0.0-rc.1

- Test-only: Jackson 2 at 2.22.3 and the Gradle Shadow plugin at 9.6.1
  in the EDC runtimes, closing 17 advisories.

## 2026-11-21, 0.14.0

- Step 8: a second issuer, the EDC Issuer Service, issues the
  `InteropTestCredential` to both participants; each provider requires
  it; it revokes EDC's credential at the end. The revocation by
  `portitor-issuer` becomes step 9.

## 2026-11-20, 0.13.0

- EDC v1.0.0-RC8 (before: RC7).
- Gaia-X compliance: Portitor's onboarding, a Portitor offer for
  compliant participants, EDC refused without a compliance credential
  and admitted with one. The participants become
  `did:web:portitor.gaiax.test` and `did:web:edc.gaiax.test`.

## 2026-11-15, 0.7.0

- The HealthDCAT-AP bundle: EDC reads a Portitor dataset with full-IRI
  metadata, negotiates and transfers it.

## 2026-11-14, 0.6.0

- `portitor-issuer` is the data space's authority; EDC IdentityHub asks
  it for its credential. The test signs the Identity API's tokens
  itself.

## 2026-11-13, 0.5.0

- First version: Portitor against an EDC v1.0.0-RC7 control plane, data
  plane and IdentityHub. Credentials checked, catalog, negotiation and
  pull transfer both ways, revocation both ways. In CI on pull requests
  and nightly.
