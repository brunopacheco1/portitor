#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Decides which CI jobs a change needs (.github/workflows/changes.yml).
# It answers two questions about the changed files:
#
#   code   false only when EVERY changed file is prose
#   build  false only when every changed file is prose or fuzzing
#
# Prose is a short list of paths that no CI job reads, except the jobs
# that always run (REUSE, the documentation check) and the website
# workflow, which has its own filter:
#
#   site/**  _http/**  .vscode/**
#   *.md in the repository root
#   docs/**/*.md, except docs/STABILITY.md (the frozen_interface and
#   settings tests read it) and docs/ARCHITECTURE.md
#   (scripts/check_docs.py reads it)
#
# Fuzzing is fuzz/** and .clusterfuzzlite/**: a workspace of its own, read
# by the fuzzing, CodeQL and dependency jobs, not by the build and the TCKs.
#
# The rule is fail-safe: a path that is not on these lists means "run
# everything". So does any doubt: a schedule or a manual run, an empty
# diff, a commit that cannot be found. A new folder is tested by default.
# Do not add a path to the prose list without checking that no test and
# no script reads it (CONTRIBUTING.md, "Required checks").
#
# Usage:
#   scripts/ci_changes.sh detect      in a workflow; reads the environment
#                                     below, prints code= and build=, and
#                                     appends them to $GITHUB_OUTPUT
#   scripts/ci_changes.sh classify    the same for a list of paths on
#                                     standard input, one per line
#   scripts/ci_changes.sh self-test   checks the rule on fixed path lists
#
# Environment of `detect` (all from the github context):
#   EVENT_NAME  pull_request, push, schedule, workflow_dispatch, ...
#   BASE_SHA    pull_request: the base branch's commit
#   HEAD_SHA    pull_request: the pull request's last commit
#   BEFORE      push: the commit the branch pointed to before the push
#
# By hand, for a commit on main:
#   EVENT_NAME=push BEFORE=<commit>^ scripts/ci_changes.sh detect
# with <commit> checked out, or without a checkout:
#   git diff --name-only --no-renames <commit>^ <commit> | scripts/ci_changes.sh classify

set -euo pipefail

# Sets the globals `code` and `build` from the paths on standard input.
classify() {
  local path seen=0
  code=false
  build=false
  while IFS= read -r path || [[ -n $path ]]; do
    [[ -n $path ]] || continue
    seen=1
    # the two documents that tests and scripts read are not prose
    if [[ $path == docs/STABILITY.md || $path == docs/ARCHITECTURE.md ]]; then
      code=true
      build=true
    elif [[ $path =~ ^(site|_http|\.vscode)/ || $path =~ ^[^/]+\.md$ || $path =~ ^docs/.+\.md$ ]]; then
      : # prose
    elif [[ $path =~ ^(fuzz|\.clusterfuzzlite)/ ]]; then
      code=true
    else
      code=true
      build=true
    fi
  done
  # no path at all is a doubt, not a prose change
  if [[ $seen == 0 ]]; then
    code=true
    build=true
  fi
}

# Prints the decision, and gives it to the workflow.
report() {
  echo "code=$code"
  echo "build=$build"
  if [[ -n ${GITHUB_OUTPUT:-} ]]; then
    {
      echo "code=$code"
      echo "build=$build"
    } >>"$GITHUB_OUTPUT"
  fi
}

run_everything() {
  echo "run everything: $1" >&2
  code=true
  build=true
  report
  exit 0
}

has_commit() {
  [[ -n $1 ]] && git cat-file -e "$1^{commit}" 2>/dev/null
}

# A shallow clone may miss the commits to compare with: get the full
# history once. If that fails, the caller runs everything.
deepen() {
  if [[ $(git rev-parse --is-shallow-repository 2>/dev/null) == true ]]; then
    git fetch --quiet --no-tags --unshallow origin 2>/dev/null || true
  fi
}

detect() {
  local event=${EVENT_NAME:-} from to files
  case "$event" in
    pull_request)
      # what the pull request changes: from the commit where it left the
      # base branch (the merge base) to its last commit
      to=${HEAD_SHA:-}
      if ! has_commit "${BASE_SHA:-}" || ! has_commit "$to"; then
        deepen
      fi
      has_commit "${BASE_SHA:-}" || run_everything "the base commit '${BASE_SHA:-}' is not in this clone"
      has_commit "$to" || run_everything "the head commit '$to' is not in this clone"
      from=$(git merge-base "$BASE_SHA" "$to" 2>/dev/null) || {
        deepen
        from=$(git merge-base "$BASE_SHA" "$to" 2>/dev/null) || run_everything "no merge base of $BASE_SHA and $to"
      }
      ;;
    push)
      # what the push added: from the commit before it to the checked-out
      # one. A new branch has no commit before it (forty zeros), and after
      # a forced push the old commit may be gone.
      from=${BEFORE:-}
      to=HEAD
      [[ -n $from && ! $from =~ ^0+$ ]] || run_everything "a push with no commit before it"
      has_commit "$from" || deepen
      has_commit "$from" || run_everything "the commit before the push, $from, is not in this clone"
      ;;
    *)
      run_everything "event '${event}' is not a pull request or a push"
      ;;
  esac
  # --no-renames: a renamed file counts under its old and its new path.
  # core.quotePath: a path with unusual bytes is printed in quotes, which
  # no prose pattern matches.
  files=$(git -c core.quotePath=true diff --name-only --no-renames "$from" "$to" --) ||
    run_everything "git diff $from $to failed"
  [[ -n $files ]] || run_everything "no file changed between $from and $to"
  echo "changed files ($from to $to):" >&2
  echo "  ${files//$'\n'/$'\n'  }" >&2
  classify <<<"$files"
  report
}

# One case: the expected "code build", then the changed paths.
expect() {
  local want=$1
  shift
  classify < <(printf '%s\n' "$@")
  if [[ "$code $build" != "$want" ]]; then
    echo "self-test FAILED: expected '$want', got '$code $build' for: $*" >&2
    failures=$((failures + 1))
  fi
}

self_test() {
  failures=0
  # prose only: nothing to build
  expect "false false" CHANGELOG.md docs/ROADMAP.md site/index.html
  expect "false false" README.md
  expect "false false" docs/adr/0026-tls-termination.md docs/adr/README.md docs/READING-THE-CODE.md
  expect "false false" _http/collection.bru .vscode/launch.json site/fonts/a.woff2
  # the two documents that tests and scripts read
  expect "true true" docs/STABILITY.md
  expect "true true" docs/ARCHITECTURE.md
  expect "true true" CHANGELOG.md docs/STABILITY.md
  # other files under docs/ are test inputs
  expect "true true" docs/api/openapi.json
  expect "true true" docs/schemas/bundle-manifest.schema.json
  expect "true true" docs/diagrams/c2-containers.drawio.svg
  # code, manifests, scripts, workflows, test data
  expect "true true" crates/portitor-dsp/src/lib.rs
  expect "true true" crates/portitor-dsp/tests/schemas/README.md
  expect "true true" Cargo.toml
  expect "true true" Cargo.lock
  expect "true true" Dockerfile
  expect "true true" scripts/ci_changes.sh
  expect "true true" .github/workflows/ci.yml
  expect "true true" .github/PULL_REQUEST_TEMPLATE.md
  expect "true true" examples/datasets.json
  expect "true true" examples/bundle/healthdcat-ap-r8/README.md
  expect "true true" interop/edc/compose.yaml
  expect "true true" tck/datasets.json
  expect "true true" REUSE.toml
  expect "true true" LICENSE
  # one code file among prose is enough
  expect "true true" CHANGELOG.md site/index.html crates/portitor-dcp/src/lib.rs
  # an unknown folder, and names that only look like prose
  expect "true true" new-folder/file.txt
  expect "true true" sitemap/index.html
  expect "true true" docs.md/file.rs
  expect "true true" docs/notes.MD
  expect "true true" docs/notes.md.txt
  expect "true true" '"docs/odd\nname.md"'
  # fuzzing only: the fuzzing, CodeQL and dependency jobs, not the build
  expect "true false" fuzz/Cargo.lock
  expect "true false" .clusterfuzzlite/Dockerfile fuzz/fuzz_targets/dsp_messages.rs README.md
  expect "true true" fuzz/Cargo.lock crates/portitor-dsp/Cargo.toml
  # no path at all
  classify </dev/null
  if [[ "$code $build" != "true true" ]]; then
    echo "self-test FAILED: an empty list must run everything" >&2
    failures=$((failures + 1))
  fi
  if [[ $failures != 0 ]]; then
    echo "$failures self-test case(s) failed" >&2
    exit 1
  fi
  echo "self-test passed"
}

case "${1:-}" in
  detect) detect ;;
  classify)
    classify
    report
    ;;
  self-test) self_test ;;
  *)
    echo "usage: $0 detect | classify | self-test" >&2
    exit 2
    ;;
esac
