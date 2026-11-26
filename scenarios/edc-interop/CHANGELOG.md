<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# edc-interop: changelog

How the scenario changed, newest first. Each entry names the Portitor
version it ran on. Before 2026-11-24 it was `scripts/edc-interop.sh`
with `interop/edc/`; the repository's [CHANGELOG](../../CHANGELOG.md)
has those entries in full.

## 2026-11-26, for 1.3.0

- The Portitor components keep their state in PostgreSQL (1.3,
  [ADR 0029](../../docs/adr/0029-several-replicas.md)): `compose.yaml`
  has a `database` service (`postgres` 19, pinned by digest) with one
  database and role per Portitor component (EDC keeps its state in
  memory), made by
  `examples/postgres/init-databases.sh`. `run.sh` writes a test CA, the
  server's certificate and one password per component into
  `run/database/` (`scripts/demo-database.sh`); the components connect
  with TLS, checking the certificate and the host name, and read their
  password from a Compose secret. Images before 1.3 ignore
  `PORTITOR_DATABASE_URL` and keep their SQLite file. The steps are
  unchanged.
  Not run with the change yet.

## 2026-11-24, for 1.1.0

- Step 6 adds a push transfer (`HttpData-PUSH`, ADR 0027 section 1),
  EDC consumer, Portitor provider, under the step's agreement: EDC asks
  its data plane for the receiving address and sends it in its transfer
  request; Portitor `POST`s the dataset there and completes the
  transfer. The run checks `COMPLETED` on both sides and that the bytes
  received equal `data/portitor-provider.csv`.
- The Portitor dataset `portitor-interop` gains an `HttpData-PUSH`
  distribution. The EDC data plane is registered for `HttpData-PUSH`
  too, and `runtimes/dataplane` names the push address (endpoint type
  `https://w3id.org/dspace-sig/profile/http-push`, a bearer token),
  and keeps what arrives at once. Portitor waits until EDC reports the
  transfer started before it pushes (README, "Findings").
- Needs 1.1.0 Portitor images: push came in 1.1.

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
