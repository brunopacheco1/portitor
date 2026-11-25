<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# two-party: a provider and a consumer

The smallest data space Portitor makes. Changes are in its
[changelog](CHANGELOG.md).

## What it is about

An authority, a provider and a consumer. The consumer authenticates with
the Decentralized Claims Protocol (DCP), reads a catalog checked against
a HealthDCAT-AP bundle, agrees contracts and pulls a file. Changed
terms, a missing claim and a revoked credential are refused. It asks:
do the three components work together end to end, on every change? It
is also the [quickstart](https://bruno.pacheco.lu/portitor/#try) on the
website.

## Scope

In:

- Joining the data space: the authority registers both participants,
  each identity asks for its credentials (`MembershipCredential`, and a
  `DataProcessorCredential` for the consumer).
- DSP calls authenticated with DCP, and one refused without a token.
- The provider's datasets harvested and checked against the signed
  HealthDCAT-AP bundle; one dataset rejected by SHACL.
- Contract negotiation with credential constraints (ADR 0019), a pull
  transfer, its suspension and start again (1.1), its completion, and a
  refusal after revocation.
- The provider's audit line for each pull (`data transfer done`, 1.1).
- With object storage turned on (1.1, ADR 0027): a source in the
  provider's RustFS bucket (`s3://`), and a push transfer into the
  consumer's bucket.

Out, on purpose:

- More than one provider or consumer, and chained transfers: see the
  health scenarios ([index](../README.md)).
- Purpose or data permits (1.2).
- Push to an endpoint other than a Portitor consumer's receiving route.
- Gaia-X compliance and other connectors: the Gaia-X and EDC interop
  tests cover them ([TESTING](../../docs/TESTING.md)).

## Components tested, and why

| Component or interface | Why it is in the story |
| --- | --- |
| `portitor-issuer` (the authority) | Registers the participants, issues and revokes credentials, builds and serves the bundle: a data space cannot start without it. |
| `portitor-identity`, one per participant | Holds the credentials, signs presentations, and is the STS that every DSP request needs (DCP). |
| `portitor-connector` as provider | Harvests and checks its datasets, serves the catalog, checks offers at negotiation and transfer, serves the data. |
| `portitor-connector` as consumer | Reads the catalog, negotiates and transfers, through its management API. |
| The built-in data plane (HTTP pull) | Serves the file with a token, refused while the transfer is suspended and once it is complete, and writes an audit line for each pull. |
| Status lists | The provider sees the revoked `DataProcessorCredential`, and later the revoked `MembershipCredential`, at once (`PORTITOR_STATUS_LIST_CACHE_SECONDS=0`). |
| The provider's management API for its transfers | Lists the transfers it runs as provider and ends one; access ends at once. |
| RustFS (`rustfs/rustfs:1.0.6`), with object storage on | One S3 store for both participants: the provider reads a source from bucket `provider-data`, the consumer stores pushed data in bucket `consumer-push`. |

## Run it

![Deployment: the Compose version](diagrams/deployment.drawio.svg)

```sh
scenarios/two-party/run.sh             # Docker or Podman, the release images
scenarios/two-party/run-local.sh       # binaries on this machine, built in debug
```

It takes a few seconds once the images are there. CI runs both on every
pull request, with the binaries and images it has just built.
[TESTING, "Two connectors"](../../docs/TESTING.md#two-connectors) says
what each script sets up, how to use your own images, keep the
containers running (`KEEP=1`) and add the observability stack.

The object storage steps are off by default: they start one more
container, RustFS. Turn them on (connector images from 1.1 on; CI runs
them):

```sh
S3=1 scenarios/two-party/run.sh                  # starts the compose profile s3 (RustFS)
RUSTFS=/path/to/rustfs scenarios/two-party/run-local.sh   # a rustfs 1.0.6 binary
```

Each run makes the store's keys for itself and passes them to both
connectors as `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, with
`AWS_ENDPOINT_URL_S3`; the consumer also gets
`PORTITOR_PUSH_BUCKET=consumer-push`. [s3-datasets.jq](s3-datasets.jq)
changes the demo datasets: `cancer-registry-extract` comes from
`s3://provider-data/cancer-registry-extract.csv`, and
`hospital-admissions` also offers `HttpData-PUSH`.

## The steps

![The steps, as a C4 dynamic diagram](diagrams/scenario.drawio.svg)

[steps.sh](steps.sh) checks, in this order:

1. A catalog request without a token is refused (401).
2. One made through the consumer's management API
   (`POST /v1/catalog/request`, for which the connector gets a token
   from its STS) is answered. Under the plain DSP `@context`, the two
   valid datasets carry `dct:conformsTo` with the release 8 profile and
   their metadata as full IRIs, and `bed-occupancy` is missing. The
   provider's `GET /v1/harvest-jobs/{id}` shows it rejected by the
   `shacl` check with its SHACL report.
3. The `hospital-admissions` offer copied from the catalog negotiates to
   `FINALIZED`; the same offer id without its constraint is
   `TERMINATED`.
4. The restricted `cancer-registry-extract` offer for members (the
   `memberOf` claim of the demo `MembershipCredential`, ADR 0019) is
   `FINALIZED`; its offer for auditors, a `role` claim the demo
   credential does not have, is `TERMINATED`.
5. A transfer under the first agreement: the script fetches the file
   with the token and checks it equals
   `examples/data/hospital-admissions.csv`. It suspends the transfer
   (`POST /v1/transfers/{consumerPid}/suspension`) and checks that the
   token is refused (401) while it is `SUSPENDED`, then starts it again
   (`POST /v1/transfers/{consumerPid}/start`) and fetches the file again
   with the same token. The provider's log (`PROVIDER_LOG`) must hold
   one audit line, `data transfer done`, for each of the two fetches:
   the agreement, the consumer's DID, the file's size in `bytes` and
   `outcome` `complete`. Then it completes the transfer and checks that
   the token is refused afterwards, and transfers the restricted dataset
   under the members' agreement.
6. A second credential type (ADR 0019): the `hospital-admissions` offer
   for EU processors checks the consumer's `DataProcessorCredential`;
   it negotiates to `FINALIZED` and a transfer starts. The issuer
   revokes that credential (`POST /v1/credentials/{id}/revoke`), and a
   new transfer request under the same agreement is `TERMINATED`.
7. The provider ends a transfer on its own (ADR 0027): its management
   API lists the members' transfer of step 5
   (`GET /v1/provided-transfers`) and terminates it
   (`POST /v1/provided-transfers/{providerPid}/termination`, 202). The
   token is refused at once (401), and the consumer's transfer is
   `TERMINATED`.
8. Membership checked again on every data request (ADR 0027): a new
   transfer under the first agreement, and its data fetched. The issuer
   revokes the consumer's `MembershipCredential`; the next request for
   the data is refused (401), and the provider ends the transfer
   (`TERMINATED`). The provider sees the revocation at once, because its
   status list cache is 0.

With object storage on, `steps.sh` also:

- creates the buckets `provider-data` and `consumer-push` and uploads
  the provider's source, before step 1; the members' transfer of step 5
  then fetches its data from the provider's bucket;
- before step 7, requests a push transfer (`HttpData-PUSH`) under the
  first agreement. The provider sends the file to the consumer's
  receiving route (`/push/{consumerPid}`), which stores it in
  `consumer-push`. The transfer must be `COMPLETED` with `objectKey`
  and no data address, the object must equal the file, and the
  provider's log must hold a `complete` audit line with its size. A
  request to the receiving route without the token is refused (401).

## Findings

None recorded yet. A finding from a run goes here, linked to its risk
or debt in [RISKS_AND_TECHNICAL_DEBTS](../../docs/RISKS_AND_TECHNICAL_DEBTS.md).
