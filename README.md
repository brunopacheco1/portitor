<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Portitor

A small and fast connector for dataspaces. It implements the
[Eclipse Dataspace Protocol](https://eclipse-dataspace-protocol-base.github.io/DataspaceProtocol/2025-1-err2/)
(DSP 2025-1) and the Decentralized Claims Protocol (DCP v1.0.1). It can
also get a participant's Gaia-X compliance credential and require one in
a policy (Gaia-X Trust Framework).

[![Release](https://img.shields.io/github/v/release/brunopacheco1/portitor)](https://github.com/brunopacheco1/portitor/releases/latest)
[![CI](https://github.com/brunopacheco1/portitor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/ci.yml)
[![TCK](https://github.com/brunopacheco1/portitor/actions/workflows/tck.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/tck.yml)
[![CodeQL](https://github.com/brunopacheco1/portitor/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/brunopacheco1/portitor/badge)](https://scorecard.dev/viewer/?uri=github.com/brunopacheco1/portitor)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/16578/badge)](https://www.bestpractices.dev/projects/16578)
[![OpenSSF Baseline](https://www.bestpractices.dev/projects/16578/baseline)](https://www.bestpractices.dev/projects/16578)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=brunopacheco1_portitor&metric=alert_status)](https://sonarcloud.io/summary/overall?id=brunopacheco1_portitor)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=brunopacheco1_portitor&metric=coverage)](https://sonarcloud.io/component_measures?id=brunopacheco1_portitor&metric=coverage)
[![Reproducible build](https://github.com/brunopacheco1/portitor/actions/workflows/reproducible.yml/badge.svg?branch=main)](https://github.com/brunopacheco1/portitor/actions/workflows/reproducible.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![REUSE status](https://api.reuse.software/badge/github.com/brunopacheco1/portitor)](https://api.reuse.software/info/github.com/brunopacheco1/portitor)
[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.1-4baaaa.svg)](CODE_OF_CONDUCT.md)

Status: 0.x, the release candidate for 1.0. The HTTP routes and the
settings are frozen ([docs/STABILITY.md](docs/STABILITY.md)). Not for
production use yet.

Website: <https://bruno.pacheco.lu/portitor/> (objectives, minimum
functionality, components, roadmap).

## What it does

A participant is an organisation with one DID (a decentralised
identifier, here a `did:web` address). With Portitor it offers
datasets with policies, agrees contracts with other participants and
transfers the data (HTTP pull). Each participant proves who it is with
credentials from the dataspace's issuer, on every request.

Credentials are system to system: a participant holds them, and
connectors check them. Portitor does not identify the people or
applications inside an organisation
([who holds the credentials](docs/ARCHITECTURE.md#who-holds-the-credentials)).

The name: in Latin a *portitor* is both the customs officer at a port,
who checks goods against the rules, and the ferryman who carries them
across. A connector does both. The project was called LDSC until 0.3
([ADR 0008](docs/adr/0008-project-name.md)).

## Components

Three services, each its own container image
([ADR 0003](docs/adr/0003-component-split.md)). Deploy them in this
order.

| Order | Image | Who runs it | Needed? |
|---|---|---|---|
| 1 | `portitor-issuer` | The dataspace authority (the party that decides the dataspace's rules), once per dataspace. A participant never runs it | Optional for a participant; the dataspace needs one issuer |
| 2 | `portitor-identity` | Every participant | Mandatory, unless the participant uses another DCP wallet ([ARCHITECTURE](docs/ARCHITECTURE.md#components)): it holds the participant's key and credentials, and the connector does not start without it |
| 3 | `portitor-connector` | Every participant | Mandatory: catalog, contract negotiation and data transfer |

Each service keeps its state in one SQLite file. Each has a management
API for its operator, on its own port and behind an API key; that port
must never be exposed to the Internet. Inside a deployment, two things
are optional: the authority's signed schema and vocabulary bundle (the
shapes that dataset metadata is checked against), and Gaia-X
compliance.

## Try it

The [quickstart](https://bruno.pacheco.lu/portitor/#try) on the website
runs a small dataspace on your machine: an issuer, a provider and a
consumer. It needs Docker with the Compose plugin, or
[Podman](docs/TESTING.md#docker-or-podman), and bash 4.4 or later,
`curl`, `jq` and `openssl`.

```sh
scenarios/two-party/run.sh
```

The script checks that the two participants authenticate, agree a
contract and transfer a file. What it does step by step, and how to run
the same without Docker, is in
[docs/TESTING.md](docs/TESTING.md#two-connectors).

## Where to read next

| You want to | Read |
|---|---|
| Deploy it: order, settings, security, known limits | [docs/DEPLOYING.md](docs/DEPLOYING.md) |
| Operate it: health checks, logs, traces and metrics, backups, key changes, upgrades | [docs/OPERATING.md](docs/OPERATING.md) |
| Call it: every HTTP route, as OpenAPI 3.1 | [docs/api/](docs/api/) (connector, identity, issuer; public and management) |
| Know what 1.0 keeps stable, and what it does not do | [docs/STABILITY.md](docs/STABILITY.md) |
| Understand the design | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), decisions in [docs/adr/](docs/adr/) |
| Test a change, or run the demos by hand | [docs/TESTING.md](docs/TESTING.md); Gaia-X: [how to test it](docs/TESTING.md#how-to-test-gaia-x-compliance) |
| See the plan and the known risks | [docs/ROADMAP.md](docs/ROADMAP.md), [docs/RISKS_AND_TECHNICAL_DEBTS.md](docs/RISKS_AND_TECHNICAL_DEBTS.md) |
| Check the security work | [SECURITY.md](SECURITY.md), [docs/SECURITY-ASSURANCE.md](docs/SECURITY-ASSURANCE.md) |
| See measured numbers | [docs/BENCHMARKS.md](docs/BENCHMARKS.md) |
| Read the Rust code | [docs/RUST-IDIOMS.md](docs/RUST-IDIOMS.md) |
| Contribute | [CONTRIBUTING.md](CONTRIBUTING.md), [GOVERNANCE.md](GOVERNANCE.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) |

## Licence

[Apache-2.0](LICENSE) ([ADR 0010](docs/adr/0010-licence.md)).
