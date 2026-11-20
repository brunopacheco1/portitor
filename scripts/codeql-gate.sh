#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Fails when CodeQL reported a finding that is not on the list of accepted
# findings. The CodeQL workflow runs it after the analysis; it can be run
# by hand on any SARIF file CodeQL wrote (docs/TESTING.md).
#
#   scripts/codeql-gate.sh <language> <folder with *.sarif> [<accepted list>]
#
# <language> is rust or actions: the first part of CodeQL's rule ids.
# Every result counts, whatever its severity. The accepted list
# (.github/codeql/accepted-findings.json) names false positives as rule,
# file and how many; one more finding of that rule in that file fails.
# An entry with fewer findings than it accepts is reported, and does not
# fail: a pull request run may hold only the findings in changed lines.
#
# Needs jq. Exit code 0: nothing new; 1: findings; 2: wrong call or input.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo "usage: $0 <language> <folder with *.sarif> [<accepted list>]" >&2
  exit 2
fi
language=$1
sarif_folder=$2
accepted_list=${3:-"$(dirname "$0")/../.github/codeql/accepted-findings.json"}

shopt -s nullglob
sarif_files=("$sarif_folder"/*.sarif)
if [ "${#sarif_files[@]}" -eq 0 ]; then
  # no file is not the same as no finding: the analysis did not write here
  echo "::error::CodeQL gate: no SARIF file in '$sarif_folder'" >&2
  exit 2
fi
if [ ! -f "$accepted_list" ]; then
  echo "::error::CodeQL gate: no accepted list at '$accepted_list'" >&2
  exit 2
fi

# One line per finding: rule, file, line, message (tab separated).
findings=$(jq -r '
  .runs[].results[]
  | .locations[0].physicalLocation as $place
  | [.ruleId, $place.artifactLocation.uri, ($place.region.startLine // 0),
     (.message.text | gsub("[\\n\\t]"; " "))]
  | @tsv' "${sarif_files[@]}")

# One line per accepted entry of this language: rule, file, count.
accepted=$(jq -r --arg prefix "$language/" '
  .accepted[] | select(.rule | startswith($prefix)) | [.rule, .file, .count] | @tsv
  ' "$accepted_list")

# How many findings of `rule` in `file` the list accepts.
accepted_count() {
  local rule=$1 file=$2 listed_rule listed_file count
  while IFS=$'\t' read -r listed_rule listed_file count; do
    if [ "$listed_rule" = "$rule" ] && [ "$listed_file" = "$file" ]; then
      echo "$count"
      return
    fi
  done <<<"$accepted"
  echo 0
}

# How many findings of `rule` in `file` the analysis reported.
found_count() {
  local rule=$1 file=$2 found_rule found_file rest count=0
  while IFS=$'\t' read -r found_rule found_file rest; do
    if [ "$found_rule" = "$rule" ] && [ "$found_file" = "$file" ]; then
      count=$((count + 1))
    fi
  done <<<"$findings"
  echo "$count"
}

total=0
new=0
if [ -n "$findings" ]; then
  while IFS=$'\t' read -r rule file line message; do
    total=$((total + 1))
    found=$(found_count "$rule" "$file")
    allowed=$(accepted_count "$rule" "$file")
    if [ "$found" -gt "$allowed" ]; then
      new=$((new + 1))
      echo "::error file=$file,line=$line::CodeQL $rule: $message ($found in this file, $allowed accepted)"
    fi
  done <<<"$findings"
fi

if [ -n "$accepted" ]; then
  while IFS=$'\t' read -r rule file count; do
    found=$(found_count "$rule" "$file")
    if [ "$found" -lt "$count" ]; then
      echo "::notice file=$file::CodeQL gate: $count of $rule accepted here, $found found. If this is a run on main, lower or remove the entry in $accepted_list"
    fi
  done <<<"$accepted"
fi

echo "CodeQL gate ($language): $total findings, $new not accepted."
if [ "$new" -gt 0 ]; then
  echo "Fix them, or, for a false positive, add it with its reason to .github/codeql/accepted-findings.json (docs/TESTING.md)."
  exit 1
fi
