<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# hdab-intermediary: the HDAB pulls, counts and passes on only the counts

A scenario on the [health-lu](../health-lu/) data space. Changes are in
its [changelog](CHANGELOG.md).

## What it is about

Lux Biobank holds synthetic genotypes. Two data users reach them by two
roads. Lux Biomedicine Lab, an EU data processor, agrees with Lux
Biobank directly and takes a copy, peer to peer. Schuman Clinical
Research may not see the genotypes: it agrees with the health data
access body (HDAB), which pulls the genotypes from Lux Biobank only when
Schuman fetches, counts the carriers of each variant per population, and
passes on only the counts. This is way A of the
[roadmap](../../docs/ROADMAP.md#ways-to-link-data-user-hdab-and-holder-maintainer-2026-11-23),
the HDAB as intermediary.

It answers the roadmap's four questions for the health data space demo
([What it answers](#what-it-answers)).

## Scope

In:

- Two offers of one dataset, each checking another credential, and two
  agreements on it: the lab's and the HDAB's.
- A chained transfer: the HDAB's data source is a pipeline that pulls
  from Lux Biobank under the HDAB's own agreement, during Schuman's
  fetch, and keeps nothing.
- Refusals: the wrong role, a missing credential, an offer whose end has
  passed, a revoked member.

Out, on purpose:

- A data permit: none exists before 1.2, so any data user may take the
  HDAB's offer (step 5).
- A secure processing environment (EHDS Art. 73): a fixed pipeline
  stands in.
- Push transfers and object storage (1.1): the lab's app pulls into a
  plain folder.
- A link between the HDAB's agreement with Lux Biobank and its
  agreements with data users (1.3, R-39).

## Components tested, and why

| Component or interface | Why it is in the story |
| --- | --- |
| `portitor-issuer` (Digital Lux Trust) | Registers the participants and issues their `MembershipCredential` and `DataProcessorCredential`; revokes the HDAB's membership in step 6. Serves the HealthDCAT-AP bundle. |
| `portitor-identity`, one per participant | Holds the credentials, signs the presentations, and is the STS every DSP request needs (DCP). |
| `portitor-connector` as provider (Lux Biobank, the HDAB) | Harvests its datasets, checks them against the bundle (SHACL), serves its catalog, checks each offer's credential constraints at negotiation and again at each transfer. |
| `portitor-connector` as consumer (the lab, the HDAB, Schuman) | Reads catalogs, negotiates, requests transfers, through its management API. The HDAB is both provider and consumer. |
| The built-in data plane (HTTP pull) | Proxies the data with a token. In the chain, the HDAB's data plane waits up to 30 s for its pipeline. |
| Status lists | Lux Biobank and Schuman refuse the HDAB once its membership is revoked. |

## Run it

```sh
SCENARIOS=hdab-intermediary scenarios/health-lu/run.sh
```

[health-lu, "Run it"](../health-lu/README.md#run-it) says what it needs
and how to keep it running. It takes about two and a half minutes, most
of it waiting for the pilot offer to end (`PILOT_SECONDS=60` makes that
shorter). [steps.sh](steps.sh) runs the steps below.

## The steps

![The steps, as a C4 dynamic diagram](diagrams/scenario.drawio.svg)

Every call goes through a participant's own management API. Each step
prints what it checks and stops at the first failure.

1. **Lux Biomedicine Lab, peer to peer.** The lab reads Lux Biobank's
   catalog and takes the offer for EU processors. Lux Biobank checks its
   `DataProcessorCredential`. The lab's app pulls the genotypes into the
   lab's plain folder (`run/lab/storage`) and the file is checked
   against the original. The offer for access bodies is refused to the
   lab, because its role is `data-user`.
2. **The HDAB.** The HDAB takes Lux Biobank's offer for access bodies,
   once. Nothing is pulled yet: the HDAB has no transfer.
3. **Schuman Clinical Research and Lux Biobank.** Both of Lux Biobank's
   offers are refused to Schuman Clinical Research. It is not an access
   body and holds no `DataProcessorCredential`.
4. **Schuman Clinical Research, through the HDAB.** Schuman takes the
   HDAB's offer for data users. After the agreement, and again after its
   transfer has started, the HDAB still has pulled nothing. Only when
   Schuman's app fetches the data does the pipeline pull from Lux Biobank
   and count. The counts are checked against `counts.py` run on the
   file. A second transfer pulls again. Schuman also takes the pilot
   offer and transfers once.
5. **A known gap.** The lab is a data user too, so the HDAB agrees with
   it as well. There is no data permit to require until 1.2.
6. **Refusals.** Once the pilot offer has ended, a new transfer under it
   is refused. Then Digital Lux Trust revokes the HDAB's
   `MembershipCredential`, and the HDAB is out of the data space. Lux
   Biobank refuses to show it its catalog. Schuman's connector refuses
   the HDAB's messages, so a new transfer never starts, and Schuman ends
   it itself.

## What it answers

The [roadmap](../../docs/ROADMAP.md#the-health-dataspace-demo) asks four
questions. Here is what rc.1 shows.

| Question | What the run shows |
| --- | --- |
| Can the same data be reached through several channels? | Yes. One dataset at Lux Biobank with two offers gives two agreements: the lab's (step 1) and the HDAB's (step 2). Schuman reaches the same data only as counts, through the HDAB (step 4). |
| Does the HDAB negotiate with Lux Biobank for every new query? | No. One agreement covers every pull. Each pull is a new transfer, and Lux Biobank checks the HDAB's credentials again at each one (steps 4 and 6). rc.1 sends no parameters with a fetch, so a different query needs its own dataset and offer at the HDAB. |
| Does the HDAB negotiate with Lux Biobank for every new data user? | No. Each data user agrees with the HDAB only (steps 4 and 5). Nothing ties those agreements to the HDAB's agreement with Lux Biobank ([R-39](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#r-39)). |
| Can the HDAB's policy be less strict, and what can never be wider? | The policies differ: Lux Biobank asks for an access body, the HDAB for any data user. The HDAB's end date (2027-12-31) is no later than its own agreement's, but only because it was written so by hand. Nothing checks it. Purpose cannot be stated until 1.2. Which users may take the counts cannot be limited either (step 5). |

## Findings

- **No data permit** (1.2): any member with `role` = `data-user` can take
  the HDAB's offer, the lab too (step 5).
- **No link between agreements** (1.3,
  [R-39](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#r-39)): the HDAB's
  offer is kept inside its own agreement by hand.
- **Pull only** (1.1, [TD-10](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-10)):
  the lab's app fetches into a plain folder instead of object storage.
- **The chained fetch has 30 seconds.** The HDAB's data plane waits at
  most 30 seconds for its source (`UPSTREAM_TIMEOUT`, fixed in code). The
  pipeline's whole pull and count must fit. `steps.sh` prints how long
  the fetch took (with bash 5): 0.9 seconds on 2026-11-23, with
  locally built images on a cloud Linux machine, so far from the limit
  with this small file.
- **A revoked party leaves its counter-parties waiting.** After the
  revocation, Schuman's connector refuses the HDAB's messages, which is
  right. But that includes the HDAB's termination, so Schuman's transfer
  stays `REQUESTED` until Schuman ends it itself (step 6).
- **The catalog shows every offer to every member**
  ([TD-13](../../docs/RISKS_AND_TECHNICAL_DEBTS.md#td-13)): Schuman
  sees offers it cannot meet.

Findings from running it go into issues. A finding that needs an API or
configuration change blocks 1.0.0.
