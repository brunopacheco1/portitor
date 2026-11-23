# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""The pipeline's count: python3 -m unittest, from this folder."""

import unittest

import counts

SAMPLES = [("A1", "AAA")] * 6 + [("B1", "BBB")] * 6
VCF = "\n".join(
    [
        "##fileformat=VCFv4.3",
        "##META=<ID=Population,Type=String,Number=.,Values=[AAA, BBB]>",
        *(f"##SAMPLE=<ID={p}{i},Population={p}>" for i, (_, p) in enumerate(SAMPLES)),
        "\t".join(["#CHROM", "POS", "ID", "REF", "ALT", "QUAL", "FILTER", "INFO", "FORMAT"])
        + "\t"
        + "\t".join(f"{p}{i}" for i, (_, p) in enumerate(SAMPLES)),
        # AAA: 5 carriers (0|1, 1|0, 1|1 count once each), BBB: 2
        "x\t10\t.\tA\tG\t.\tPASS\t.\tGT\t"
        + "\t".join(["0|1", "1|0", "1|1", "0|1", "1|0", "0|0"] + ["0|1", "1|1"] + ["0|0"] * 4),
        # 1 carrier in all: below 5 everywhere, left out
        "x\t20\t.\tC\tT\t.\tPASS\t.\tGT\t" + "\t".join(["0|1"] + ["0|0"] * 11),
        # nobody in BBB: 0 shown as 0
        "x\t30\t.\tG\tA\t.\tPASS\t.\tGT\t" + "\t".join(["1|1"] * 6 + ["0|0"] * 6),
    ]
)


class CountTest(unittest.TestCase):
    def test_carriers_with_small_counts_suppressed(self):
        self.assertEqual(
            counts.count(VCF),
            "chrom,pos,ref,alt,AAA_carriers,BBB_carriers\n"
            "x,10,A,G,5,<5\n"
            "x,30,G,A,6,0\n",
        )


if __name__ == "__main__":
    unittest.main()
