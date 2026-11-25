# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The demo datasets (examples/healthdcat-datasets.json) for a run with
# object storage (S3=1, ADR 0027): hospital-admissions also offers push
# (a second distribution, format HttpData-PUSH), and the data of
# cancer-registry-extract comes from the provider's own bucket instead of
# the web server. Used by run.sh and run-local.sh.
map(
  if .["@id"] == "https://provider.example/datasets/hospital-admissions" then
    .distribution += [
      .distribution[0]
      | .["@id"] |= sub("/csv$"; "/push")
      | .format = "HttpData-PUSH"
    ]
  elif .["@id"] == "https://provider.example/datasets/cancer-registry-extract" then
    .["portitor:source"] = "s3://provider-data/cancer-registry-extract.csv"
  else
    .
  end
)
