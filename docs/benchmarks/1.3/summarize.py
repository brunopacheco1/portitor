# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""Medians of the key=value lines of scripts/bench.sh, as in docs/BENCHMARKS.md."""
import re, sys, statistics
def load(path):
    vals = {}
    for line in open(path):
        line=line.strip()
        if '=' not in line or line.startswith('#'): continue
        k, v = line.split('=',1)
        try: v=float(v)
        except ValueError: continue
        k2 = re.sub(r'_run\d+', '', k)
        vals.setdefault(k2, []).append(v)
    return {k: statistics.median(v) for k,v in vals.items()}, vals
files = sys.argv[1:]
data = [load(f) for f in files]
keys = [k for k in data[0][0]]
for k in keys:
    row = [d[0].get(k) for d in data]
    rng = [ (min(d[1][k]), max(d[1][k])) if k in d[1] else None for d in data]
    def fmt(x): return '-' if x is None else f'{x:g}'
    ch = ''
    if row[0] and len(row)>1 and row[1] is not None: ch = f'{(row[1]-row[0])/row[0]*100:+.1f}%'
    ch2 = ''
    if row[0] and len(row)>2 and row[2] is not None: ch2 = f'{(row[2]-row[0])/row[0]*100:+.1f}%'
    print(f'{k}\t' + '\t'.join(fmt(x) for x in row) + f'\t{ch}\t{ch2}\t' + ' '.join(f'[{fmt(r[0])}..{fmt(r[1])}]' if r else '-' for r in rng))
