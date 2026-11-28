<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0030: A management console for demos

- Status: **Proposed** (2026-11-28)
- Date: 2026-11-28
- Scope: release 1.5
  ([ROADMAP](../ROADMAP.md#15-data-user-and-hdab-the-hdab-and-schuman-clinical-research)).
  Asked for by the maintainer on 2026-11-28: "let's implement a
  management UI, propose something and let's add it to the 1.5".

## Context

A Portitor demo today is a terminal: each scenario's `run.sh` and
`steps.sh` call the management APIs with curl, the
[Bruno](https://www.usebruno.com/) collection in `_http/` sends one
request at a time, and the website publishes the OpenAPI pages. People
who do not read curl or logs cannot follow it. Nothing shows the state
of a data space at a glance: who holds which credential and with which
status, which datasets a partner offers, where a negotiation or a
transfer stands, and why something was refused.

Facts checked on 2026-11-28:

- **Eclipse EDC** has a separate dashboard for this,
  [DataDashboard](https://github.com/eclipse-edc/DataDashboard)
  (Apache-2.0, commit `5d2cf81`). Its README: "This repository does not
  contain production-grade code and is only intended for demonstration
  purposes." It is an Angular application with daisyUI and tailwindcss,
  configured with a list of connectors, and it calls EDC's Management
  API v3 through `edc-connector-client`. Portitor's management API is
  not EDC's, so it cannot be reused as it is; its shape (a separate
  application, a list of participants, one view per kind of object) is
  the precedent.
- Portitor's management APIs (`docs/STABILITY.md`) already cover what a
  demo shows and does: credentials and their status, credential
  requests and offers (identity); participants, claims, credentials and
  revocation (issuer); catalog and dataset requests, negotiations,
  transfers as consumer and as provider, harvest jobs (connector). One
  thing is missing: the connector cannot list its own datasets (only
  its harvest jobs).
- The management APIs send no CORS headers, and every call needs the
  management API key of that component. A page in a browser cannot call
  them directly from another origin, and must not hold the keys.
- Architecture principles: small, simple components, and few of them;
  the three images stay as they are (ADR 0003).

## Decision (proposed)

1. **A separate console, never in a release image.** A folder
   `console/` holds a static web application: HTML, CSS and plain
   JavaScript modules. No build step and no npm dependencies, so there
   is no second supply chain to audit next to cargo-vet, and REUSE
   covers every file. The three images do not change.
2. **Served with a proxy that holds the keys.** A pinned nginx image
   serves `console/` and passes `/api/<participant>/<component>/...`
   to that component's management port, adding its
   `Authorization: Bearer` header from the key file the scenario
   already writes (`run/<party>/management.key`). The browser sees one
   origin and no key. The proxy listens on `127.0.0.1` only.
3. **Configured per scenario.** Each scenario that offers the console
   adds one service to its `compose.yaml` and one file
   `console.json`: the participants, their roles and the management
   address of each component. The console reads that file at start.
4. **What it shows (1.5).** One page per participant, and a data space
   overview:
   - identity: the credentials held, with type, issuer, validity and
     the status of 1.4 (`VALID`, `REVOKED`, `SUSPENDED`,
     `UNCHECKABLE`); credential requests; offers;
   - issuer (the authority, the HDAB): participants and their claims,
     issued credentials, revoke;
   - connector: a partner's catalog (datasets, offers and their
     policies, in words: "valid until 2027-12-31", "needs a
     DataPermitCredential with permitId P-2026-001", "purpose: health
     care scientific research"); negotiations and agreements; transfers
     as consumer and as provider, with their state; harvest jobs and
     their errors;
   - each refusal shown with the reason the API gives.
5. **What it does (1.5).** Only calls that the management APIs already
   have: ask for a credential, register a participant or replace its
   claims, revoke a credential, read a catalog, start a negotiation
   from an offer, start, suspend, complete or end a transfer. The
   scenario scripts stay the source of truth for the checks; the
   console runs the same calls, by hand.
6. **First scenario: two-party**, the quickstart, then data-permits and
   hdab-intermediary (the health demo). The website's "Try it" gains
   one line: open `http://127.0.0.1:<port>/`.
7. **Not for production in 1.x.** DEPLOYING says so, as EDC does for
   its dashboard. Users and roles come in 1.7
   ([ADR 0025](0025-users-and-roles.md),
   [TD-04](../RISKS_AND_TECHNICAL_DEBTS.md#td-04)); a console for
   operators needs them first.
8. **Tests.** A Playwright script, run by hand like the scenarios
   (`scenarios/two-party/console-check.sh`), opens the console on the
   running two-party scenario and walks the quickstart: credentials
   shown, catalog read, negotiation finalized, transfer completed, data
   fetched. CI checks the static files only (REUSE, links in
   `console.json`).

## Options not chosen

- **A UI inside each image** (served on the management port). One more
  surface per component, a web stack in a Rust binary, and the key or a
  login in the browser. Against "small, simple components".
- **A fourth image, `portitor-console`.** Possible later, when there is
  a use outside demos; in 1.5 an nginx image and a folder are enough.
- **EDC's DataDashboard, adapted.** It speaks EDC's Management API v3
  through EDC's client library; adapting it means a fork of an Angular
  application and its npm tree.
- **A frontend framework** (React, Angular, Vue) with a build step. A
  better developer experience, but a second toolchain and dependency
  tree for a demo tool the maintainer must read.
- **Swagger UI on the OpenAPI files.** It works today, with no code,
  but it shows requests, not the state of a data space.

## Consequences

- Demos can be followed without a terminal; the scripts keep the
  checks.
- `console/` and one service per scenario to keep current when a
  management route changes (the frozen interface makes that rare).
- The console needs the connector to list its own datasets for the
  provider's page. Proposed: an additive route
  `GET /v1/datasets` on the connector's management port (the read copy
  of ADR 0020, with each dataset's harvest result), listed in
  STABILITY in the same pull request.
- Plain JavaScript is a second language in the repository. Kept small,
  with comments in the style of `docs/RUST-IDIOMS.md`.

## Questions for the maintainer

1. Name: "console" (as here), "dashboard" (EDC's word), or another?
2. Read and act (as proposed), or read only in 1.5?
3. The new `GET /v1/datasets` route: yes, or show harvest jobs only?
4. First scenario: two-party (the quickstart, as proposed) or the
   health demo?
