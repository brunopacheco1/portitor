<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Demos

One folder per scenario. Each one starts its own small data space,
runs its checks and removes everything at the end. `KEEP=1` leaves the
containers running so you can look around.

## [two-party](two-party/): a provider and a consumer

The smallest data space: an authority (`portitor-issuer`), a provider and
a consumer, each with its connector and identity. The consumer
authenticates with DCP, reads a catalog checked against a HealthDCAT-AP
bundle, agrees a contract and pulls a file. Changed terms, a missing
claim and a revoked credential are refused. CI runs it on every pull
request, as binaries on this machine (`demo.sh`) and in Docker Compose
(`demo-compose.sh`). [docs/TESTING.md, "Two connectors"](../../docs/TESTING.md#two-connectors)
explains it.

```sh
examples/demos/two-party/demo-compose.sh
```

## [health-lu](health-lu/): a Luxembourg-style health data space

Five parties modelled on Luxembourg institutions share synthetic
genotypes the way the European Health Data Space plans it for
secondary use. A biobank offers the genotypes. A research lab takes a
copy peer to peer. A health data access body (HDAB) takes them too and
offers only counts made from them. A hospital research unit gets those
counts, and never the genotypes. A second scenario has the HDAB only
grant data permits, as credentials, which the biobank checks itself. It
answers the roadmap's four questions
for the health data space demo, and lists where Portitor falls short
today. It is a manual test, not run in CI.
[health-lu/README.md](health-lu/README.md) explains it.

```sh
examples/demos/health-lu/run.sh
```

## Planned: the HDAB as permit authority

The HDAB only issues data permits. The data holder checks the permit
when it negotiates and sends the data where the permit says. The
permit check runs today, in health-lu's second scenario. Sending the
data where the permit says needs push transfers and a destination
check, so it waits for 1.1 and 1.2
([roadmap](../../docs/ROADMAP.md#12-purpose-and-data-permit-in-policies)).

## Planned: the HDAB negotiates on a data user's behalf

The HDAB asks the data holder for an agreement for a data user. Today
the holder refuses it: an agreement belongs to the party that
negotiates it. It needs a delegation the holder can check, which no
specification we know defines, so it starts as research
([roadmap](../../docs/ROADMAP.md#13-data-user-and-hdab-the-hdab-and-schuman-clinical-research)).

## More planned scenarios

A mixed data space (one participant on Eclipse EDC), a holder that
withdraws its dataset, a cross-border data user, a foreign issuer, a
holder without its own connector, and opt-out. The [roadmap](../../docs/ROADMAP.md#more-health-scenarios-maintainer-2026-11-23)
lists what each one shows and which release it needs.
