# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""Makes the synthetic genotypes of the health demo: no real person is in them.

Simulates 100 kb of human DNA for 150 people (50 in each of the three
populations of the out-of-Africa model of Gutenkunst et al., 2009,
https://doi.org/10.1371/journal.pgen.1000695, as stdpopsim names it:
YRI, CEU, CHB) with msprime, and writes:

  genotypes.vcf   the genotypes, one column per person, with each
                  person's population in the header, as the VCF 4.3
                  specification's "Sample field format" shows it
                  (##META and ##SAMPLE lines; tskit writes VCF 4.2, so the
                  header says 4.3)

The seed is fixed, so a run gives the same files. msprime (GPL-3.0) and
stdpopsim (GPL-3.0) run only here, to make the files; nothing of them is
shipped. Run it from the repository root:

  python3 -m venv /tmp/gen && /tmp/gen/bin/pip install msprime==1.3.4 stdpopsim==0.3.0
  /tmp/gen/bin/python examples/genotypes/generate.py
"""

import io
import pathlib

import msprime
import stdpopsim

HERE = pathlib.Path(__file__).parent
SEED = 20261123
PER_POPULATION = 50
LENGTH = 100_000

species = stdpopsim.get_species("HomSap")
model = species.get_demographic_model("OutOfAfrica_3G09")
contig = species.get_contig(length=LENGTH)
populations = [p.name for p in model.populations]

ancestry = msprime.sim_ancestry(
    samples={name: PER_POPULATION for name in populations},
    demography=model.model,
    sequence_length=LENGTH,
    recombination_rate=contig.recombination_map.mean_rate,
    random_seed=SEED,
)
tree_sequence = msprime.sim_mutations(
    ancestry, rate=contig.mutation_rate, random_seed=SEED, discrete_genome=True
)

# person ids: SYN0001 and on
names = []
samples = []
for number, individual in enumerate(tree_sequence.individuals(), start=1):
    population = tree_sequence.population(individual.population).metadata["name"]
    name = f"SYN{number:04d}"
    names.append(name)
    samples.append(f"##SAMPLE=<ID={name},Population={population}>")

text = io.StringIO()
tree_sequence.write_vcf(text, contig_id="syn1", individual_names=names)
lines = text.getvalue().splitlines()
assert lines[0] == "##fileformat=VCFv4.2", lines[0]
header = lines.index(next(line for line in lines if line.startswith("#CHROM")))
meta = f"##META=<ID=Population,Type=String,Number=.,Values=[{', '.join(populations)}]>"
lines = ["##fileformat=VCFv4.3", *lines[1:header], meta, *samples, *lines[header:]]
(HERE / "genotypes.vcf").write_text("\n".join(lines) + "\n")
print(f"{tree_sequence.num_sites} sites, {len(names)} people")
