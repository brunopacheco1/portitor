<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# data-permits: the HDAB grants data permits, the holder checks them

A health data space for secondary use, the way the European Health Data
Space (EHDS, [Regulation (EU) 2025/327](https://eur-lex.europa.eu/eli/reg/2025/327/oj))
plans it, on one machine. Changes are in its [changelog](CHANGELOG.md).

## What it is about

Here the health data access body (HDAB) does not fetch or count
anything. It only grants data permits, as verifiable credentials from
its own issuer. Schuman Clinical Research holds them, and Lux Biobank
checks them itself: one offer per permit, checked when Schuman
negotiates and again at each transfer. This is way F of the
[roadmap](../../docs/ROADMAP.md#ways-to-link-data-user-hdab-and-holder-maintainer-2026-11-23),
permit-gated peer to peer.

It asks: can v1.0.0-rc.1 carry a data permit with configuration alone,
and what breaks when a data user holds several permits and one is
revoked?

### Who takes part

The organisations are made up. Each one is modelled on a Luxembourg
institution that could play the role. None of those institutions takes
part in or endorses this scenario. Luxembourg has not named its HDAB
yet, so the scenario calls it just "HDAB".

| Party | Role | Modelled on | Runs |
| --- | --- | --- | --- |
| Digital Lux Trust | The authority: issues the memberships and the HealthDCAT-AP bundle | Ministry for Digitalisation | `portitor-issuer` |
| HDAB | Health data access body: grants data permits. It needs no connector here | Luxembourg's future HDAB | `portitor-issuer` |
| Lux Biobank | Health data holder: offers the genotypes, one offer per permit | Integrated Biobank of Luxembourg (IBBL), part of LIH | `portitor-connector`, `portitor-identity` |
| Schuman Clinical Research | Data user: holds the permits and gets the genotypes | Hôpitaux Robert Schuman | `portitor-connector`, `portitor-identity` |

Digital Lux Trust gives Lux Biobank a `MembershipCredential` with the
`role` `data-holder`, and Schuman one with `data-user`. The HDAB's
issuer has its own credential definitions,
[hdab-issuer-credentials.json](hdab-issuer-credentials.json): a
`DataPermitCredential` (claims `permitId` and `purpose`) and, on
purpose, a `MembershipCredential` (step 5). Lux Biobank's connector and
Schuman's identity trust the HDAB's issuer for `DataPermitCredential`
only (`PORTITOR_TRUSTED_ISSUER_TYPES`, since 1.4).

![Deployment: data-permits on one machine](diagrams/deployment.drawio.svg)

### The data and the dataset

Lux Biobank's data are the [synthetic genotypes](../../examples/genotypes/)
in `examples/genotypes/`: no real person is in them. Its dataset
([datasets/biobank.json](datasets/biobank.json)) is in HealthDCAT-AP,
checked against the data space's release 8 bundle at start, with three
offers: one per permit (`DataPermitCredential.permitId` = `P-2026-001`,
or `P-2026-002`, and `DataPermitCredential.purpose` =
`https://w3id.org/dpv/legal/eu/ehds#HealthcareScientificResearch`, the
[DPV EHDS](https://w3id.org/dpv/legal/eu/ehds) purpose for EHDS Art.
53(1)(e), with an ODRL `purpose` constraint of the same value, until
2027-12-31), and one for access bodies (`MembershipCredential.role` =
`health-data-access-body`).

## Scope

In:

- A data permit as a credential type defined only in the HDAB issuer's
  configuration ([hdab-issuer-credentials.json](hdab-issuer-credentials.json)),
  with no code.
- Two permits held at once by one data user (maintainer, 2026-11-24),
  each with its own offer and agreement.
- Revoking a permit, and what it does to the other.
- A second trusted issuer next to the authority, trusted for data
  permits only.

Out, on purpose:

- A real permit format. The claims `permitId` and `purpose` are
  placeholders (the purpose value is a DPV EHDS IRI): EHDS Art. 68(10) lists what a permit states, and the
  template of Art. 70 is still to come. The permit's end is the
  credential's own `validUntil`.
- A secure processing environment. Schuman pulls the genotypes itself.
  The EHDS would not allow that for these data (Art. 73), so this shows
  the checks only.
- Sending the data where the permit says: that needs push transfers and
  a destination check (1.1, 1.4).

## Components tested, and why

| Component or interface | Why it is in the story |
| --- | --- |
| `portitor-issuer` (the HDAB's) | Issues the permits from configuration alone; replaces a participant's claims for a second permit (`PUT /v1/participants/{did}`, since 1.4); revokes a permit. |
| `portitor-issuer` (Digital Lux Trust) | Issues the memberships and serves the HealthDCAT-AP bundle. |
| `portitor-identity` (Schuman) | Trusts both issuers, the HDAB's for data permits only; stores both permits, presents every credential of the type asked for, reads their status lists and leaves the revoked permit out (`PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS`, since 1.4), and does not ask the HDAB's issuer for a membership. |
| `portitor-connector` as provider (Lux Biobank) | Trusts both issuers (`PORTITOR_TRUSTED_ISSUERS`), the HDAB's for data permits only (`PORTITOR_TRUSTED_ISSUER_TYPES`), reads `DataPermitCredential` claims in its offers (`PORTITOR_POLICY_CREDENTIALS`), checks them at negotiation and at each transfer, refuses a presentation with a revoked credential. |
| `portitor-connector` as consumer (Schuman) | Asks for the `DataPermitCredential` scope the offer needs, negotiates and transfers. |
| The built-in data plane (HTTP pull) | Schuman's app fetches the genotypes, checked against the original. |
| Status lists | Schuman's identity reads the HDAB issuer's list and records the permit as revoked; Lux Biobank's connector reads the same list at each check. |
| PostgreSQL (`postgres` 19) | One database and role per Portitor component, over TLS ([ADR 0029](../../docs/adr/0029-several-replicas.md)): the documented default from 1.3. A store bug that only PostgreSQL shows would show here. |

## Run it

```sh
scenarios/data-permits/run.sh
```

It needs Docker or Podman with Docker Compose, bash 4.4 or later, curl,
jq and openssl ([docs/TESTING.md, "Docker or Podman"](../../docs/TESTING.md#docker-or-podman)).
The images can be given as arguments, and `CONTAINER_ENGINE=podman`
picks Podman, as in [hdab-intermediary](../hdab-intermediary/README.md#run-it).
[run.sh](run.sh) writes the keys into `run/`, starts the 9 containers of
[compose.yaml](compose.yaml), has Digital Lux Trust register the two
participants and issue their memberships, then runs
[steps.sh](steps.sh), the steps below. It takes about a minute.

`KEEP=1` leaves everything running, with the keys in `run/`. The
management APIs are then on `127.0.0.1`: Digital Lux Trust 18071, the
HDAB's issuer 18125, Lux Biobank 18101, Schuman Clinical Research 18131.
The key of each is in `run/<party>/management.key`. The host ports are
those of hdab-intermediary, so run one at a time.

## The steps

![The steps, as a C4 dynamic diagram](diagrams/scenario.drawio.svg)

1. **A permit.** The HDAB's issuer registers Schuman with the claims of
   permit P-2026-001, and Schuman's identity asks for the
   `DataPermitCredential`.
2. **The holder checks it.** Schuman takes Lux Biobank's offer for
   P-2026-001 and pulls the genotypes, checked against the original. The
   offer for P-2026-002 is refused: Schuman has no such permit.
3. **A second permit.** A data user may hold several permits at once.
   The HDAB replaces Schuman's claims at its issuer with those of
   P-2026-002 (`PUT /v1/participants/{did}`); Schuman stays registered
   all along. Schuman holds both, takes the second offer, and the first
   agreement still works.
4. **A revoked permit.** The HDAB revokes P-2026-001. Schuman's
   identity reads the status lists of the credentials it holds (since
   1.4, every 2 seconds here: `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS`
   in [compose.yaml](compose.yaml)); the script waits until its
   management API shows the permit `REVOKED`. From then on the identity
   leaves the revoked permit out of its presentations, so a transfer
   under P-2026-002 works without deleting anything by hand, and a
   transfer under P-2026-001 is refused: Schuman no longer presents that
   permit. Before 1.4, the revoked permit made Lux Biobank refuse every
   presentation of Schuman's permits until Schuman deleted it.
5. **Trust by type.** The HDAB's issuer would give Schuman a
   `MembershipCredential` with the role `health-data-access-body`: its
   configuration holds one on purpose. Schuman's identity trusts that
   issuer for data permits only, so it refuses to ask for the
   membership, with the reason. Lux Biobank's offer for access bodies
   refuses Schuman. Had Schuman held such a membership, Lux Biobank's
   connector would skip it too, unfetched: it trusts the HDAB's issuer
   for data permits only.

## Findings

- **An issuer trusted for permits could issue memberships too**
  ([R-48](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#r-48), fixed in 1.4):
  `PORTITOR_TRUSTED_ISSUER_TYPES` limits the HDAB's issuer to data
  permits (step 5). An issuer the setting does not name still keeps
  every type until 2.0 ([TD-74](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-74)).
  The list is configuration, so an issuer also stays trusted when its
  owner leaves the data space.
- **One revoked permit blocked all the others**
  ([TD-20](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-20), fixed in
  1.4): the data user's identity now reads the status lists of what it
  holds and stops presenting a revoked permit, within one check period
  (step 4). Until it has checked, a transfer can still be refused, and
  the data user is then told no reason.
- **One set of claims per participant at the issuer**
  ([TD-46](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-46), fixed in
  1.4): a second permit replaces the claims, and the participant stays
  registered (step 3). The issuer still keeps one set of claims at a
  time: the new claims go into the next request.
- **One offer per permit.** Lux Biobank must add an offer, by editing
  its datasets file, for each permit the HDAB grants; here both are
  written in advance.
- **The catalog shows every offer to every member**
  ([TD-13](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-13)): every member
  sees the offers for every permit.
