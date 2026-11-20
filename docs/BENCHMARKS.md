<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Benchmarks

What Portitor does under the load a data space puts on it, measured
for release 0.12. Architecture principle 4 asks for these numbers every
release; until 0.11 they were in
[ADR 0002](adr/0002-technology-stack.md), section 7.

> **Read this first.** These numbers compare one release with the
> next, on one small shared machine with every party on it. They are
> **not a capacity promise**. Your network, TLS, disk and data source
> will set other numbers. Run `scripts/bench.sh` on your own hardware
> before you plan capacity.

- **Date:** 2026-11-19.
- **Commit:** the 0.12 branch `r012-bench`, on top of `9472407` (0.11.0
  plus the 2026-11-18 review), with `panic = "abort"` removed.
- **Machine:** a shared cloud virtual machine (KVM): 4 virtual CPUs
  (Intel Xeon, 2.10 GHz), 15 GB memory, a virtual disk of unknown type
  (reported as rotational), Linux 6.18, Rust 1.97.0. Other work can run
  on the same host, so single values move from run to run.
- **Build:** release binaries for `x86_64-unknown-linux-gnu` (the
  release images are static musl builds). Logs at `warn`.
- **Setup:** everything on the one machine over plain HTTP, without
  TLS: an issuer, a provider and a consumer (each a connector and an
  identity, with DCP on every DSP message), a data source and the load
  tool. They share the 4 CPUs, so the load tool and the data source
  take part of what is measured.
- **Spread:** where a measure is cheap it ran 3 times; the tables give
  the median and, in brackets, the lowest and highest run.

## The reference load

Negotiations are rare in real health data platforms; data transfers
are many, long and large, and the history grows for years:

- Findata (Finland) issued 316 data permits in 2024 (second-hand
  source: [Quantify Research](https://quantifyresearch.com/2025/03/26/quantify-leads-with-most-findata-permits-issued-in-2024-for-an-rwe-provider/)).
- UK Biobank counts 7,653 approved applications in total, more than
  3,800 projects in progress and 30 PB of data
  ([annual report 2025](https://www.ukbiobank.ac.uk/uk-biobank-annual-report-2025)).
- The HDR UK Gateway lists more than 1,100 datasets
  ([ELIXIR-UK, 2025](https://elixiruknode.org/news/2025/health-data-research-gateway-joins-elixir-uk-services/)).

So one negotiation per second is already far above what such a
platform sees (316 a year is fewer than one a day), and what matters
is the data plane under many parallel and long pulls, and the stored
history. The load agreed with the maintainer (2026-11-18):

| Area | Load |
|---|---|
| Data plane, parallel | 100 and 1,000 pulls at the same time |
| Data plane, large | one 10 GB pull |
| Data plane, long | a pull that runs longer than its token lives |
| Stored history | 10,000 negotiations, agreements and transfers |
| Catalog | 100, 1,000 and 10,000 datasets |
| Negotiation | 1 per second |
| DCP cost | as in 0.5, now with the status list cache |

## Results

### Footprint

`scripts/measure.sh`, 3 runs.

| Measure | 0.12 | 0.11 code, same machine | Target (ADR 0002) |
|---|---|---|---|
| Binary size, connector / identity / issuer | 14.7 / 10.6 / 10.2 MB | 12.9 / 9.4 / 9.1 MB | — |
| Connector start to first healthy answer (includes resolving its DID and a token from its STS) | 24 ms (23 to 25) | not measured | < 1 s |
| Idle memory (RSS), connector / identity / issuer | 14.4 / 13.1 / 12.3 MB | not measured | < 30 MB |
| Peak memory, 50 clients sending catalog requests for 20 s: connector with DCP / without | 42.0 / 18.2 MB | not measured | — |

The binaries grew by 13.9%, 13.3% and 12.9%. The only difference
between the two builds is `panic = "abort"` and the panic handling
that replaces it ([SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#what-a-panic-does)):
the code that unwinds is now in the binary. Image sizes were not
measured here (no Docker on this machine).

### Data plane

`scripts/bench.sh parallel large long`. The data source makes its
bytes up and sends them as fast as the reader takes them. "Direct" is
the same download with the same tool, straight from the data source.
Time to first byte is from sending the request to the first byte of
the answer's body.

| Load | Through the provider | Direct | Provider's memory (RSS) |
|---|---|---|---|
| 100 pulls started at once, 64 MiB each | first byte p50 74 ms (70 to 93), p99 149 ms (121 to 168); 4,306 MB/s in total (4,158 to 4,328) | first byte p50 14 ms, p99 32 ms; 7,926 MB/s (7,657 to 8,870) | peak 85 MB (17 MB before) |
| 1,000 pulls started at once, 8 MiB each | first byte p50 474 ms (303 to 972), p99 1,449 ms (1,319 to 2,170); 1,893 MB/s (1,724 to 2,022) | first byte p50 93 ms, p99 915 ms; 3,430 MB/s (2,806 to 4,839) | peak 923 MB (21 MB before) |
| One pull of 10 GiB (1 run) | 2,177 MB/s; 4.9 s | 2,755 MB/s; 3.9 s | 14 MB before, 19 MB during, peak 19 MB: flat |
| One pull of 20 s with a token that lives 5 s | **not cut**: all 13,107,200 bytes arrived after 20.1 s. A second request with the same token got 401 | — | — |

What this says:

- **Throughput** through the provider is 54% (100 pulls), 55% (1,000)
  and 79% (one pull) of the direct download. The provider reads and
  writes every byte, on CPUs it shares with the source and the reader,
  so half is the expected floor here. No pull failed.
- **Memory per pull.** One large pull is flat at 19 MB, whatever its
  size: the data is streamed, not kept. But each pull in flight costs
  memory: about 0.7 MB at 100 pulls and 0.9 MB at 1,000, so 1,000
  pulls at once took 923 MB. Plan memory for the number of pulls in
  flight, not for their size.
- **Time to first byte at a burst.** 1,000 requests arriving in the
  same instant wait: the public server works on 256 requests at once
  (`MAX_CONCURRENT_REQUESTS`; a pull holds its place only until its
  answer starts), and all share 4 CPUs with the tool that sends them.
  The direct download shows the same shape (p99 915 ms). Each pull
  needs two connections at the provider, so the open files limit must
  be above twice the number of pulls (`ulimit -n`; the script sets
  16,384).
- **Long pulls.** The token is checked when the request arrives, not
  while the body is sent. A download that has started runs to its end
  after the token expired; a new request needs a new transfer, since
  tokens are not renewed
  ([TD-10](RISKS_AND_TECHNICAL_DEBTS.md#td-10)). The 60 s request time
  limit also ends when the answer starts.
- **Small answers can wait 40 ms.** About 5% of the 1 KiB pulls of the
  next section took 43 ms instead of 0.1 ms. The servers do not set
  `TCP_NODELAY`: when the answer's headers and its small body go out as
  two writes, the second waits for the reader's acknowledgement. A
  trial build with `TCP_NODELAY` on the public listener brought the p99
  from 43 ms to 0.4 ms. Not changed in 0.12; pulls larger than a
  network packet are not affected.

### Stored history

`scripts/bench.sh history`. A new negotiation, a transfer start, a
token check and a 1 KiB pull, one at a time, first against empty
databases, then after 10,000 negotiations with an agreement and a
started transfer each were stored (made 16 at a time through the
consumer's management API: 146 s, 68 per second). Then the provider's
database file is 25.9 MB and the consumer's 15.9 MB. Each run is 50
negotiations and transfers, so its p99 is close to its slowest one.

| Measure | Empty databases | With 10,000 stored |
|---|---|---|
| New negotiation, request to `FINALIZED` (both connectors, DCP on each of its messages; includes up to 2 ms of polling) | p50 11.7 ms (11.6 to 11.7), p99 22.5 ms (21.4 to 36.1) | p50 11.7 ms (11.5 to 11.8), p99 20.7 ms (15.6 to 25.9) |
| Transfer start, request to `STARTED` | p50 8.1 ms (8.0 to 8.2), p99 13.2 ms (8.9 to 34.9) | **p50 37.9 ms (36.2 to 40.1), p99 52.3 ms (47.4 to 76.7)** |
| Data token check (a wrong token for a known transfer: refused with 401 after the transfer is read; about 74,000 checks per run) | p50 0.06 ms, p99 0.14 ms (0.13 to 0.16) | p50 0.06 ms, p99 0.12 ms (0.11 to 0.12) |
| 1 KiB pull, time to first byte | p50 0.12 ms, p99 43.9 ms (see above) | p50 0.14 ms, p99 43.3 ms |
| Provider's peak memory | — | 29 MB |

What this says:

- Negotiations and token checks do not slow down with 10,000 stored.
- **A transfer start gets slower with every stored negotiation**: 30 ms
  more at 10,000, about 3 ms per 1,000. The provider finds the
  agreement of a transfer request by reading every stored negotiation
  (`Agreements::finalized` in the connector's stores). At 100,000
  stored negotiations a transfer start would take about 0.3 s. It
  needs a lookup by agreement id; not changed in 0.12.
- While the stored history was made, 2 to 4 transfer requests per run
  (3 runs of 6,000 to 10,000) were refused with HTTP 400 and the
  transfer ended `TERMINATED`. The
  consumer shows a negotiation `FINALIZED` when the provider's
  finalization message arrives; the provider records `FINALIZED` only
  when that message is acknowledged. A transfer requested in between
  finds no finalized agreement. It happened only with 16 at a time, never
  one at a time; the provider logs nothing for it. Asking again
  works. Release 0.13 closes this: such a request is accepted and the
  refusal is logged (ADR 0007, amendment for 0.13); not measured again.

### Catalog

`scripts/bench.sh catalog`. One client, 20 catalog requests one after
the other per run, each with a fresh DCP token (the provider checks the
caller on every request); the time is the request without getting the
token. A dataset here has one offer and one distribution. The catalog
is one answer: there is no paging.

| Datasets | Answer size | Request | Provider's peak memory |
|---|---|---|---|
| 100 | 26 kB | p50 1.6 ms (1.2 to 1.6), slowest 2.5 ms (1.9 to 5.4) | 15 MB |
| 1,000 | 257 kB | p50 2.8 ms (2.8 to 3.1), slowest 4.0 ms (3.4 to 7.5) | 27 MB |
| 10,000 | 2.57 MB | p50 13.0 ms (12.9 to 13.7), slowest 25.1 ms (22.5 to 29.3) | 150 MB |

Size and time grow in step with the number of datasets, about 257
bytes and 1.2 µs per dataset; memory about 13 kB per dataset. Note for
consumers: Portitor's own DSP client reads answers up to 64 kB
(`MAX_ANSWER`), which is a catalog of about 250 such datasets. Today no
Portitor code reads a catalog
([TD-38](RISKS_AND_TECHNICAL_DEBTS.md#td-38)); the call that will must
allow more.

### Negotiation at one per second

`scripts/bench.sh negotiation`. 30 negotiations per run, one started
each second through the consumer's management API, timed until the
consumer shows `FINALIZED`.

| Measure | Result |
|---|---|
| Request to `FINALIZED` | p50 15.8 ms (14.3 to 15.9), slowest 21.6 ms (20.0 to 22.4) |

In trial runs before these, one negotiation in 30 took 0.5 s once and
2.1 s once. No warning was logged and it did not happen again in 9
later runs; the cause is not known.

### Cost of DCP

`scripts/measure.sh` with `TCK_CONNECTOR`, 3 runs: catalog requests of
`tck/datasets.json`, with DCP (a fresh token each) and without (the
test-only `tck` build with one fixed header). From 0.11 the issuer's
status list is cached for 5 minutes
(`PORTITOR_STATUS_LIST_CACHE_SECONDS`); in 0.5 it was fetched on every
request.

| | Without DCP | With DCP, 0.12 | With DCP, 0.5 (ADR 0002) |
|---|---|---|---|
| One client: DSP request p50 / p99 | 0.12 / 0.27 ms | 1.63 / 2.49 ms | 1.82 / 2.91 ms |
| One client: token from the consumer's STS p50 / p99 | — | 0.36 / 0.63 ms | 0.28 / 0.53 ms |
| 50 clients: requests per second | 23,751 (23,643 to 24,456) | 2,935 (2,909 to 2,958) | 1,667 |
| 50 clients: DSP request p50 / p99 / p99.9 | 2.0 / 4.6 / 7.5 ms | 13.9 / 25.8 / 32.4 ms | 24.3 / 66.0 / 94.0 ms |
| 50 clients: token p50 / p99 | — | 2.2 / 10.3 ms | 1.8 / 18.7 ms |
| Errors | 0 | 0 | 0 |

DCP still adds about 2 ms to an idle DSP request (0.4 ms for the
consumer's token, 1.5 ms at the provider). Under load the provider now
answers 8 times fewer requests than without DCP; in 0.5 it was 13
times. The 0.5 numbers come from another machine of the same kind, so
only the ratios compare. This is the second measurement
[R-29](RISKS_AND_TECHNICAL_DEBTS.md#r-29) asked for. What is left per
request: the caller's presentation is queried and verified every time
([TD-07](RISKS_AND_TECHNICAL_DEBTS.md#td-07)).

## TD-40: do negotiations slow the data plane down?

Each store is one SQLite connection behind one lock, and every write
waits for the disk (`synchronous=FULL`); the HTTP handlers call it
directly ([TD-40](RISKS_AND_TECHNICAL_DEBTS.md#td-40)). The rule agreed
with the maintainer (2026-11-18): the database calls move to blocking
threads **if**, with 100 parallel pulls and 1 negotiation per second,
the data plane's p99 time to first byte more than doubles against the
same pulls alone, or passes 100 ms.

`scripts/bench.sh td40`: 100 pulls in flight for 30 s (each 320 KiB
from a source that takes 0.4 s to send it, started again when it
ends: 7,400 pulls per run), alone and while new negotiations run
through the same provider. 3 runs.

| Load next to the 100 pulls | Time to first byte p50 | Time to first byte p99 | Slowest |
|---|---|---|---|
| None | 1.2 ms (1.1 to 1.3) | 9.2 ms (7.6 to 39.2) | 13.8 ms (13.1 to 46.0) |
| 1 negotiation per second | 1.1 ms (0.9 to 1.2) | 8.5 ms (6.4 to 9.4) | 13.7 ms (12.6 to 14.2) |
| 10 negotiations per second | 1.0 ms (1.0 to 1.0) | 9.0 ms (7.1 to 9.8) | 15.1 ms (11.4 to 15.4) |
| As many as 16 clients can make: 260 per second | 2.5 ms (2.5 to 2.8) | 14.5 ms (14.4 to 14.7) | 23.4 ms (20.6 to 25.7) |

**Result: no need.** With 1 negotiation per second the p99 is 8.5 ms
against 9.2 ms alone: it did not double and is far below 100 ms. The
database calls stay where they are; no code changed.

**The measured limit:** at 260 negotiations per second, the most this
pair of connectors made on this machine, the p99 was 14.5 ms, 1.6
times the pulls alone, still inside the rule. The rule was not broken
at any rate we could produce. The negotiations themselves took p50
61 ms and p99 92 ms at that rate, against 16 and 26 ms at one per
second: the lock and the disk limit negotiations, not pulls. (One run
of the pulls alone had a p99 of 39 ms; the 43 ms wait for small writes
described above is the likely cause.)

## Method and how to reproduce

```sh
# the binaries and the two load tools
cargo build --release -p portitor-connector -p portitor-identity -p portitor-issuer
cargo build --release -p portitor-connector --example bench-load --example dsp-load

# data plane, stored history, catalog, negotiation, TD-40 (about 20 minutes)
CONNECTOR=target/release/portitor-connector \
  BENCH_LOAD=target/release/examples/bench-load scripts/bench.sh

# footprint and the cost of DCP: the connector without DCP is a second build
cargo build --release -p portitor-connector --features tck
cp target/release/portitor-connector /tmp/portitor-connector-tck
cargo build --release -p portitor-connector
for run in 1 2 3; do
  DSP_LOAD=target/release/examples/dsp-load TCK_CONNECTOR=/tmp/portitor-connector-tck \
    scripts/measure.sh target/release/portitor-connector
done
```

`scripts/bench.sh` takes the parts to run as arguments (`parallel`,
`large`, `long`, `history`, `catalog`, `negotiation`, `td40`) and
prints `key=value` lines, one set per run; the tables above are the
medians of those lines. `BENCH_RUNS`, `BENCH_LARGE_GB`, `BENCH_HISTORY`
and `BENCH_SECONDS` change the load. It needs 2 GB of free memory for
the 1,000 pulls and deletes its files after each part.

| Row | Method |
|---|---|
| Parallel pulls | The consumer negotiates and starts one transfer per pull. The `pulls` mode of `bench-load` then sends all requests at once, each with its transfer's token, and reads every body to the end. The same number of downloads straight from the data source gives "direct". Peak memory is `VmHWM` of the provider's process |
| Large pull | One transfer of a 10 GiB dataset. The data source makes the bytes up, so no file is written. The provider's `VmRSS` is read every 2 s during the pull |
| Long pull | The provider runs with `PORTITOR_DATA_TOKEN_SECONDS=5`; the data source sends 12.5 MB in 20 s |
| Stored history | See the section. The fill is real work: each of the 10,000 is a negotiation and a transfer between the two connectors |
| Catalog | The provider's datasets file has 100, 1,000 or 10,000 datasets; the `catalog` mode of `bench-load` sends the requests and reads the whole answer |
| Negotiation | The `contracts` mode of `bench-load` with `CONTRACTS_PER_SECOND=1` |
| TD-40 | See the section |
| Footprint, DCP cost | `scripts/measure.sh`, unchanged since 0.5 |

Limits of these results:

- One machine, shared CPUs, no TLS, no network between the parties.
  Over the internet the round trips of DCP (to the consumer's identity
  and the issuer) cost more than the CPU work measured here.
- The data source is a test tool that makes bytes up at memory speed.
  A real source is slower, and then it, not the provider, limits a
  pull.
- All 10,000 stored negotiations are with one consumer, and so is the
  consumer's history; a real provider has many partners.
- The disk type is not known. Negotiations wait for the disk on every
  write, so their times depend on it most.

## Cost of traces (0.9)

Measured for the 0.9 release, which added traces and metrics
(ADR 0023), and not repeated since. One 4-core machine, 50 clients,
`scripts/measure.sh`, mean of three runs. Runs vary by up to 15%, so
these differences are close to the noise.

| Connector | DSP requests per second |
|---|---|
| 0.8.0 | 2,074 |
| 0.9, export off | 1,909 |
| 0.9, every request traced | 1,820 |
| 0.9, one trace in ten kept | 1,701 |

Sampling did not raise throughput: the spans are made in the process
either way. How to set it: [OPERATING](OPERATING.md#traces-and-metrics).
