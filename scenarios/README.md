<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Scenarios

A scenario is a small data space started on one machine, a story run
against it step by step, and checks that stop at the first failure.
Each one removes everything at the end; `KEEP=1` leaves the containers
running so you can look around.

| Scenario | What it shows | Runs |
| --- | --- | --- |
| [two-party](two-party/) | A provider and a consumer: DCP, a checked catalog, contracts, a transfer, refusals | in CI, on every pull request |
| [edc-interop](edc-interop/) | Portitor and Eclipse EDC in one data space, both ways, with Gaia-X and a second issuer | in CI, on pull requests that touch it, and nightly |
| [gaiax-compliance](gaiax-compliance/) | A provider admits only Gaia-X compliant participants | in CI, on pull requests that touch it, and nightly |
| [two-data-spaces](two-data-spaces/) | One organisation in two data spaces, one connector each | by hand |
| [hdab-intermediary](hdab-intermediary/) | A health data access body (HDAB) pulls genotypes, counts, and passes on only the counts | by hand |
| [data-permits](data-permits/) | The HDAB grants data permits as credentials; the data holder checks them itself | by hand |

Each scenario stands alone: its `run.sh` starts only what that scenario
needs, and removes it at the end. What several scenarios use lives
outside them: the synthetic genotypes in
[examples/genotypes](../examples/genotypes/), the Gaia-X compliance
service in [examples/gaiax](../examples/gaiax/), and the shell helpers in
[scripts/scenario-lib.sh](../scripts/scenario-lib.sh). The health
scenarios use the same host ports, so run one at a time.

```sh
scenarios/two-party/run.sh
scenarios/data-permits/run.sh portitor-connector:local portitor-identity:local portitor-issuer:local
```

## What each scenario folder holds

- `README.md`, with these sections, in this order:
  1. **What it is about**: the story in a few lines, and the question it
     answers.
  2. **Scope**: what is in, and what is out on purpose.
  3. **Components tested, and why**: each Portitor component and
     interface the run goes through, and the reason it is in the story.
  4. **Run it**: the command, what it needs, how long it takes.
  5. **The steps**: one item per step of its script, with what is
     checked.
  6. **Findings**: what the run showed that Portitor lacks or does
     wrong, each linked to its risk or debt in
     [RISKS_AND_TECHNICAL_DEBTS](../docs/RISKS_AND_TECHNICAL_DEBTS.md).
- `diagrams/`: a C4 deployment diagram and a C4 dynamic diagram in
  sequence style, as `.drawio.svg` (draw.io opens them). A scenario on a
  shared data space shows that data space's deployment diagram.
- `CHANGELOG.md`: how the scenario changed. One entry for each change to
  what it shows or checks, newest first, with the date and the Portitor
  version it ran on. A change of Portitor alone (a new release that runs
  it unchanged) needs no entry.
- `run.sh`, which starts the data space, runs the steps and removes
  everything, and `steps.sh`, which holds the steps and their checks.
  Both take the same arguments and settings in every scenario: images
  as arguments (connector, identity, issuer), `KEEP=1`,
  `CONTAINER_ENGINE`. A scenario may add `run-local.sh` for a run
  without containers.
- Not yet: the scenarios that were tests before (edc-interop,
  gaiax-compliance, two-data-spaces) keep their steps in `run.sh`, take
  the images as `PORTITOR_IMAGE`, `PORTITOR_IDENTITY_IMAGE` and
  `PORTITOR_ISSUER_IMAGE`, and two-data-spaces runs processes, not
  containers. Each README says so.

The repository's [CHANGELOG](../CHANGELOG.md) names a new or removed
scenario; the scenario's own changelog keeps the detail.

## Planned

The [roadmap](../docs/ROADMAP.md#more-health-scenarios-maintainer-2026-11-23)
lists the scenarios to come, what each shows and the release it needs:
the HDAB as permit authority with push transfers, the HDAB negotiating
on a data user's behalf, a mixed data space with Eclipse EDC, a holder
that withdraws its dataset, a cross-border data user, a foreign issuer,
a holder without its own connector, and opt-out.
