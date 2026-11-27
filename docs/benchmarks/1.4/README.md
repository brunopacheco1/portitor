<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# 1.4 benchmark: raw results

The output of `scripts/bench.sh catalog` behind
[1.4: the catalog with access policies](../../BENCHMARKS.md#14-the-catalog-with-access-policies),
2026-11-27, on the maintainer's laptop (12th Gen Intel Core i3-1215U,
8 CPUs, 7.6 GB, Linux 7.2, Rust 1.100.0). `BENCH_RUNS=3` for every
file. Each file is the `key=value` lines of one run; its first line
names the commit.

| File | What ran | Code |
|---|---|---|
| [1.3-r1.txt](1.3-r1.txt), [1.3-r2.txt](1.3-r2.txt) | the base numbers: 1.3.0 on SQLite | [`b41bd46`](https://github.com/brunopacheco1/portitor/commit/b41bd464b79d63b198fcd8cf3c3c05a6c4adaa1f), tag [v1.3.0](https://github.com/brunopacheco1/portitor/releases/tag/v1.3.0) |
| [1.4-r1.txt](1.4-r1.txt), [1.4-r2.txt](1.4-r2.txt) | 1.4 on SQLite, datasets without an access policy | [`e8e71c5`](https://github.com/brunopacheco1/portitor/commit/e8e71c5abc4d75029fc368f89f8c42426f2aba5c) on the release branch |
| [1.4-access-policy-r1.txt](1.4-access-policy-r1.txt), [1.4-access-policy-r2.txt](1.4-access-policy-r2.txt) | 1.4 on SQLite, every dataset with an access policy (`BENCH_ACCESS_POLICY=1`) | `e8e71c5` |
| [load.txt](load.txt) | the load average every 10 s during all six runs | |

The runs went in this order, twice: 1.3, 1.4, 1.4 with access
policies. The tables in BENCHMARKS are the medians of both rounds
together, from [summarize.py of 1.3](../1.3/summarize.py):

```sh
cat 1.3-r1.txt 1.3-r2.txt > 1.3.txt
cat 1.4-r1.txt 1.4-r2.txt > 1.4.txt
cat 1.4-access-policy-r1.txt 1.4-access-policy-r2.txt > 1.4-access-policy.txt
python3 ../1.3/summarize.py 1.3.txt 1.4.txt 1.4-access-policy.txt
```
