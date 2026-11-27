<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# 1.3 benchmark: raw results

The output of `scripts/bench.sh` behind
[1.3 compared with 1.2](../../BENCHMARKS.md#13-compared-with-12-on-sqlite-and-on-postgresql),
2026-11-27, one machine (4 virtual CPUs, Intel Xeon 2.10 GHz, 15 GB,
Linux 6.18, Rust 1.97.0). Settings for every file: `BENCH_LARGE_GB=2
BENCH_HISTORY=2000 BENCH_SECONDS=15 BENCH_RUNS=3`. Each file is the
`key=value` lines of one version; its first line names both commits,
this table says which one it is.

| File | What ran | Code |
|---|---|---|
| [1.2.txt](1.2.txt) | the base numbers: 1.2 on SQLite, every part | [`aaee8dd`](https://github.com/brunopacheco1/portitor/commit/aaee8dd7a0618527a3cba05b24c701dade344173), the code of tag [v1.2.0](https://github.com/brunopacheco1/portitor/releases/tag/v1.2.0) ([`d36b457`](https://github.com/brunopacheco1/portitor/commit/d36b4577ee472a3951a5ac330b42124640a6e56a)) plus the docs-only close-out [#101](https://github.com/brunopacheco1/portitor/pull/101) |
| [1.3-sqlite.txt](1.3-sqlite.txt) | 1.3 on SQLite, every part | [`25bcb82`](https://github.com/brunopacheco1/portitor/commit/25bcb82f00b2fd99fb0b796f246b01c996a8b614) on the release branch ([#167](https://github.com/brunopacheco1/portitor/pull/167)) |
| [1.3-postgresql.txt](1.3-postgresql.txt) | 1.3 on PostgreSQL 16 (local TCP, no TLS), every part | `25bcb82` |
| [1.2-repeat.txt](1.2-repeat.txt), [1.3-sqlite-repeat.txt](1.3-sqlite-repeat.txt) | the second round of the history, catalog and negotiation parts, to check the changes of more than 10 % | `aaee8dd`, `25bcb82` |
| [run-replicas.txt](run-replicas.txt) | `scenarios/two-party/run-replicas.sh` | `25bcb82` |

Both versions ran the same `scripts/bench.sh` (unchanged between the two
commits), built as release binaries for `x86_64-unknown-linux-gnu`.

`scripts/bench.sh` has no database setting yet, so the PostgreSQL run used
a copy of it changed by [bench-pg.patch](bench-pg.patch), which sources
[pg-overrides.sh](pg-overrides.sh): each component gets a new database of
its own for each part, dropped afterwards, and the database size is read
with `pg_database_size`. The tables in BENCHMARKS are the medians that
[summarize.py](summarize.py) prints: `python3 summarize.py 1.2.txt 1.3-sqlite.txt 1.3-postgresql.txt`.
