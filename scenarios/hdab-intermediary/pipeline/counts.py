# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""The HDAB's pipeline: counts carriers in Lux Biobank's genotypes.

The HDAB's connector names http://pipeline:8000/counts as the source of
its counts dataset. Its data plane calls it when Schuman Clinical
Research fetches the data of a transfer that has started; only then does
the pipeline:

  1. ask the HDAB's connector, through its management API, for a transfer
     under the HDAB's agreement with Lux Biobank (the agreement id is in
     AGREEMENT_FILE, written once the HDAB has negotiated it);
  2. wait until it is STARTED, fetch the genotypes with its token, and
     complete it;
  3. count, for each variant, the people of each population who carry
     it (one or two alternative alleles), and answer a CSV. Counts from 1
     to 4 are shown as "<5" (small-count suppression), and a variant
     below 5 in every population is left out.

Nothing is kept: the genotypes stay in memory for one answer. The
connector waits at most 30 seconds for the answer.

Python standard library only. Environment:
  HDAB_MANAGEMENT  the HDAB connector's management API base URL
  HDAB_KEY_FILE    the file holding its management API key
  AGREEMENT_FILE   the file holding the agreement id
  PORT             the port to answer on (default 8000)
"""

import http.server
import json
import os
import sys
import time
import urllib.error
import urllib.request

# empty when only counting a file (below)
MANAGEMENT = os.environ.get("HDAB_MANAGEMENT", "")
KEY_FILE = os.environ.get("HDAB_KEY_FILE", "")
AGREEMENT_FILE = os.environ.get("AGREEMENT_FILE", "")
PORT = int(os.environ.get("PORT", "8000"))
SMALL = 5  # counts below this are suppressed
WAIT = 20  # seconds to wait for the transfer to start


def management(method, path, body=None):
    """Calls the HDAB connector's management API; returns the JSON answer."""
    with open(KEY_FILE) as file:
        key = file.read().strip()
    request = urllib.request.Request(
        MANAGEMENT + path,
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"authorization": f"Bearer {key}", "content-type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=10) as answer:
        text = answer.read()
    return json.loads(text) if text else None


def pull_genotypes():
    """Transfers the genotypes under the HDAB's agreement; returns the VCF text."""
    with open(AGREEMENT_FILE) as file:
        agreement = file.read().strip()
    started = management("POST", "/v1/transfers", {"agreementId": agreement})
    pid = started["consumerPid"]
    deadline = time.monotonic() + WAIT
    while True:
        transfer = management("GET", f"/v1/transfers/{pid}")
        if transfer["state"] == "STARTED":
            break
        if transfer["state"] == "TERMINATED" or time.monotonic() > deadline:
            raise RuntimeError(f"Lux Biobank did not start the transfer: {transfer['state']}")
        time.sleep(0.2)
    address = transfer["dataAddress"]
    token = next(p["value"] for p in address["endpointProperties"] if p["name"] == "authorization")
    request = urllib.request.Request(address["endpoint"], headers={"authorization": f"Bearer {token}"})
    try:
        with urllib.request.urlopen(request, timeout=10) as answer:
            return answer.read().decode()
    finally:
        management("POST", f"/v1/transfers/{pid}/completion")


def count(vcf):
    """Carriers per variant and population, as CSV with small counts suppressed."""
    population_of = {}
    populations = []
    rows = []
    columns = []
    for line in vcf.splitlines():
        if line.startswith("##SAMPLE=<"):
            fields = dict(f.split("=", 1) for f in line[len("##SAMPLE=<") : -1].split(","))
            population_of[fields["ID"]] = fields["Population"]
            if fields["Population"] not in populations:
                populations.append(fields["Population"])
        elif line.startswith("#CHROM"):
            columns = [population_of[name] for name in line.split("\t")[9:]]
        elif line and not line.startswith("#"):
            fields = line.split("\t")
            carriers = dict.fromkeys(populations, 0)
            for population, genotype in zip(columns, fields[9:]):
                if any(allele not in ("0", ".") for allele in genotype.replace("|", "/").split("/")):
                    carriers[population] += 1
            if max(carriers.values()) >= SMALL:
                shown = [str(n) if n == 0 or n >= SMALL else f"<{SMALL}" for n in carriers.values()]
                rows.append(",".join([fields[0], fields[1], fields[3], fields[4], *shown]))
    header = ",".join(["chrom", "pos", "ref", "alt", *(f"{p}_carriers" for p in populations)])
    return "\n".join([header, *rows]) + "\n"


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            self.answer(200, "text/plain", "ok\n")
            return
        if self.path != "/counts":
            self.answer(404, "text/plain", "not found\n")
            return
        try:
            self.answer(200, "text/csv", count(pull_genotypes()))
        except (OSError, RuntimeError, KeyError, StopIteration, urllib.error.URLError) as error:
            # the connector answers its caller with an error of its own
            print(f"counts failed: {error}", file=sys.stderr, flush=True)
            self.answer(502, "text/plain", "the genotypes could not be pulled\n")

    def answer(self, status, content_type, text):
        body = text.encode()
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    if len(sys.argv) == 2:  # a check: counts of a local VCF file
        with open(sys.argv[1]) as file:
            sys.stdout.write(count(file.read()))
    else:
        print(f"the pipeline answers on port {PORT}", flush=True)
        http.server.ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
