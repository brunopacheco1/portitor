<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Synthetic genotypes

The data that Lux Biobank offers in the health scenarios
([hdab-intermediary](../../scenarios/hdab-intermediary/) and
[data-permits](../../scenarios/data-permits/)).

[genotypes.vcf](genotypes.vcf) holds synthetic genotypes: 410 variants
on 100 kb of simulated human DNA, for 150 people, 50 in each of three
populations (YRI, CEU and CHB, the names of the out-of-Africa model of
[Gutenkunst et al., 2009](https://doi.org/10.1371/journal.pgen.1000695)).
Each person's population is in the VCF header (`##SAMPLE` lines, as the
VCF 4.3 specification shows them). No real person is in the file.

[generate.py](generate.py) makes it with
[msprime](https://tskit.dev/msprime/) 1.3.4 and
[stdpopsim](https://popsim-consortium.github.io/stdpopsim-docs/) 0.3.0,
with a fixed seed. Both are GPL-3.0. They run only to make the file and
are not shipped. The GNU GPL FAQ says a program's output is generally
not covered by the program's licence
([GPL FAQ](https://www.gnu.org/licenses/gpl-faq.html#WhatCaseIsOutputGPL)).
The file is licensed like the rest of the repository.

Why not real data? None of the public synthetic or open genomic datasets
we checked (the CINECA synthetic cohorts, HAPNEST, 1000 Genomes) states
a licence that lets us copy it into this repository.
