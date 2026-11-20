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
#   scripts/ci_changes.sh self-test   checks the rule on fixed path lists,
#                                     and `detect` in a temporary git
#                                     repository (needs git)
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

# The decision "run everything", as the self-test writes a decision:
# code, then build.
readonly EVERYTHING="true true"
readonly NOTHING="false false"
readonly FUZZING_ONLY="true false"

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
  local why=$1
  echo "run everything: $why" >&2
  code=true
  build=true
  report
  exit 0
}

has_commit() {
  local commit=$1
  [[ -n $commit ]] && git cat-file -e "${commit}^{commit}" 2>/dev/null
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

self_test_classify() {
  # prose only: nothing to build
  expect "$NOTHING" CHANGELOG.md docs/ROADMAP.md site/index.html
  expect "$NOTHING" README.md
  expect "$NOTHING" docs/adr/0026-tls-termination.md docs/adr/README.md docs/READING-THE-CODE.md
  expect "$NOTHING" _http/collection.bru .vscode/launch.json site/fonts/a.woff2
  expect "$NOTHING" AGENTS.md docs/api/README.md
  # a path with a space is judged like any other
  expect "$NOTHING" "docs/my notes.md" "my file.md" "site/a b.html"
  expect "$EVERYTHING" "crates/portitor-dsp/src/a b.rs"
  # only the two top-level documents are special, not a file of the same name below
  expect "$NOTHING" docs/sub/STABILITY.md
  # the two documents that tests and scripts read
  expect "$EVERYTHING" docs/STABILITY.md
  expect "$EVERYTHING" docs/ARCHITECTURE.md
  expect "$EVERYTHING" CHANGELOG.md docs/STABILITY.md
  # other files under docs/ are test inputs
  expect "$EVERYTHING" docs/api/openapi.json
  expect "$EVERYTHING" docs/schemas/bundle-manifest.schema.json
  expect "$EVERYTHING" docs/diagrams/c2-containers.drawio.svg
  # code, manifests, scripts, workflows, test data
  expect "$EVERYTHING" crates/portitor-dsp/src/lib.rs
  expect "$EVERYTHING" crates/portitor-dsp/tests/schemas/README.md
  expect "$EVERYTHING" Cargo.toml
  expect "$EVERYTHING" Cargo.lock
  expect "$EVERYTHING" Dockerfile
  expect "$EVERYTHING" scripts/ci_changes.sh
  expect "$EVERYTHING" .github/workflows/ci.yml
  expect "$EVERYTHING" .github/PULL_REQUEST_TEMPLATE.md
  expect "$EVERYTHING" .github/dependabot.yml
  expect "$EVERYTHING" examples/datasets.json
  expect "$EVERYTHING" examples/bundle/healthdcat-ap-r8/README.md
  expect "$EVERYTHING" interop/edc/compose.yaml
  expect "$EVERYTHING" tck/datasets.json
  expect "$EVERYTHING" REUSE.toml
  expect "$EVERYTHING" LICENSE
  expect "$EVERYTHING" LICENSES/OFL-1.1.txt
  # one code file among prose is enough
  expect "$EVERYTHING" CHANGELOG.md site/index.html crates/portitor-dcp/src/lib.rs
  # an unknown folder, and names that only look like prose
  expect "$EVERYTHING" new-folder/file.txt
  expect "$EVERYTHING" sitemap/index.html
  expect "$EVERYTHING" docs.md/file.rs
  expect "$EVERYTHING" docs/notes.MD
  expect "$EVERYTHING" docs/notes.md.txt
  expect "$EVERYTHING" '"docs/odd\nname.md"'
  expect "$EVERYTHING" ./README.md
  # a line ending in a carriage return is not a known path
  expect "$EVERYTHING" $'README.md\r'
  # fuzzing only: the fuzzing, CodeQL and dependency jobs, not the build
  expect "$FUZZING_ONLY" fuzz/Cargo.lock
  expect "$FUZZING_ONLY" fuzz/README.md
  expect "$FUZZING_ONLY" .clusterfuzzlite/Dockerfile fuzz/fuzz_targets/dsp_messages.rs README.md
  expect "$EVERYTHING" fuzz/Cargo.lock crates/portitor-dsp/Cargo.toml
  # a list whose last line has no line end
  classify < <(printf 'README.md\ndocs/ROADMAP.md')
  if [[ "$code $build" != "$NOTHING" ]]; then
    echo "self-test FAILED: a list without a final line end: got '$code $build'" >&2
    failures=$((failures + 1))
  fi
  classify < <(printf 'README.md\nCargo.toml')
  if [[ "$code $build" != "$EVERYTHING" ]]; then
    echo "self-test FAILED: the last line, without a line end, was not read: got '$code $build'" >&2
    failures=$((failures + 1))
  fi
  # no path at all
  classify </dev/null
  if [[ "$code $build" != "$EVERYTHING" ]]; then
    echo "self-test FAILED: an empty list must run everything" >&2
    failures=$((failures + 1))
  fi
}

# --- self-test of `detect`, in a temporary git repository --------------------

# git for the temporary repository: no configuration of the machine or
# the user (a signing rule, hooks, another default branch), a fixed author.
test_git() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null git -C "$test_repo" \
    -c user.name=self-test -c user.email=self-test@portitor.invalid \
    -c commit.gpgsign=false -c core.hooksPath=/dev/null "$@"
}

# Writes the files (the content does not matter, it only has to differ
# from commit to commit), commits everything, and prints the new commit.
test_commit() { # message path...
  local message=$1 path
  shift
  for path in "$@"; do
    mkdir -p "$test_repo/$(dirname "$path")"
    echo "$message" >>"$test_repo/$path"
  done
  test_git add --all
  test_git commit --quiet --allow-empty -m "$message"
  test_git rev-parse HEAD
}

# One case of `detect`: the expected "code build", what the case is, then
# the environment of `detect` as NAME=value words. `detect` runs in the
# temporary repository, in a subshell: it may end with `exit`.
expect_detect() { # want what NAME=value...
  local want=$1 what=$2 printed got
  shift 2
  printed=$(
    cd "$test_repo"
    # nothing of the real workflow run reaches the case
    unset EVENT_NAME BASE_SHA HEAD_SHA BEFORE GITHUB_OUTPUT
    # the words are NAME=value pairs, which `export` takes as they are
    # shellcheck disable=SC2163
    export "$@"
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null detect 2>/dev/null
  ) || printed="detect failed"
  # two lines, "code=..." and "build=...", as one "code build"
  got=$(sed -n 's/^code=//p' <<<"$printed") || true
  got="$got $(sed -n 's/^build=//p' <<<"$printed")"
  if [[ $got != "$want" ]]; then
    echo "self-test FAILED: detect, $what: expected '$want', got '$got'" >&2
    failures=$((failures + 1))
  fi
}

self_test_detect() {
  local zeros=0000000000000000000000000000000000000000
  local start code_change prose_change base_moved_on head renamed back empty spaced spaced_code
  local deleted_prose output
  test_repo=$(mktemp -d)
  # the folder is removed when the script ends, whatever the result
  trap 'rm -rf "$test_repo"' EXIT
  test_git init --quiet --initial-branch=main

  start=$(test_commit "start" README.md crates/a/src/lib.rs docs/ROADMAP.md docs/old.md)

  # --- pushes to a branch
  code_change=$(test_commit "code" crates/a/src/lib.rs)
  expect_detect "$EVERYTHING" "a push that changes code" EVENT_NAME=push "BEFORE=$start"
  prose_change=$(test_commit "prose" README.md docs/ROADMAP.md)
  expect_detect "$NOTHING" "a push that changes prose only" EVENT_NAME=push "BEFORE=$code_change"
  expect_detect "$EVERYTHING" "a push of code and prose, compared with the commit before both" \
    EVENT_NAME=push "BEFORE=$start"
  # doubts: all of them run everything
  expect_detect "$EVERYTHING" "a new branch (forty zeros before it)" EVENT_NAME=push "BEFORE=$zeros"
  expect_detect "$EVERYTHING" "a push with nothing before it" EVENT_NAME=push BEFORE=
  expect_detect "$EVERYTHING" "a commit before the push that is not in the clone" \
    EVENT_NAME=push BEFORE=1111111111111111111111111111111111111111
  expect_detect "$EVERYTHING" "an empty diff (before is the checked-out commit)" \
    EVENT_NAME=push "BEFORE=$prose_change"
  empty=$(test_commit "empty")
  expect_detect "$EVERYTHING" "an empty commit" EVENT_NAME=push "BEFORE=$prose_change"
  expect_detect "$EVERYTHING" "a scheduled run" EVENT_NAME=schedule
  expect_detect "$EVERYTHING" "a manual run" EVENT_NAME=workflow_dispatch
  expect_detect "$EVERYTHING" "no event at all"

  # --- renames count under both names
  test_git mv crates/a/src/lib.rs docs/lib.md
  renamed=$(test_commit "rename code to prose")
  expect_detect "$EVERYTHING" "a rename from code to prose" EVENT_NAME=push "BEFORE=$empty"
  test_git mv docs/lib.md crates/a/src/lib.rs
  back=$(test_commit "rename prose to code")
  expect_detect "$EVERYTHING" "a rename from prose to code" EVENT_NAME=push "BEFORE=$renamed"
  test_git rm --quiet docs/old.md
  deleted_prose=$(test_commit "delete prose")
  expect_detect "$NOTHING" "a deleted prose file" EVENT_NAME=push "BEFORE=$back"

  # --- paths with a space
  spaced=$(test_commit "prose with a space" "docs/my notes.md" "site/a b.html")
  expect_detect "$NOTHING" "prose paths with a space" EVENT_NAME=push "BEFORE=$deleted_prose"
  spaced_code=$(test_commit "code with a space" "crates/a/src/a b.rs")
  expect_detect "$EVERYTHING" "a code path with a space" EVENT_NAME=push "BEFORE=$spaced"

  # --- a pull request: compared with the commit where it left the base
  # branch (the merge base), not with the base branch as it is now
  test_git switch --quiet --create pull-request
  head=$(test_commit "the pull request: prose" docs/ROADMAP.md)
  test_git switch --quiet main
  base_moved_on=$(test_commit "main moved on: code" crates/a/src/lib.rs)
  test_git switch --quiet pull-request
  expect_detect "$NOTHING" "a prose pull request whose base branch moved on with code" \
    EVENT_NAME=pull_request "BASE_SHA=$base_moved_on" "HEAD_SHA=$head"
  expect_detect "$EVERYTHING" "a pull request with no change (head is its merge base)" \
    EVENT_NAME=pull_request "BASE_SHA=$base_moved_on" "HEAD_SHA=$spaced_code"
  head=$(test_commit "the pull request: code too" crates/a/src/lib.rs)
  expect_detect "$EVERYTHING" "a pull request with prose and code" \
    EVENT_NAME=pull_request "BASE_SHA=$base_moved_on" "HEAD_SHA=$head"
  expect_detect "$EVERYTHING" "a pull request whose base commit is not in the clone" \
    EVENT_NAME=pull_request BASE_SHA=1111111111111111111111111111111111111111 "HEAD_SHA=$head"
  expect_detect "$EVERYTHING" "a pull request whose head commit is not in the clone" \
    EVENT_NAME=pull_request "BASE_SHA=$base_moved_on" HEAD_SHA=

  # --- the decision reaches the workflow through $GITHUB_OUTPUT
  output=$test_repo/.git/github-output
  test_commit "prose again" README.md >/dev/null
  expect_detect "$NOTHING" "a prose push, with a file for the outputs" \
    EVENT_NAME=push "BEFORE=$head" "GITHUB_OUTPUT=$output"
  if [[ $(cat "$output" 2>/dev/null) != $'code=false\nbuild=false' ]]; then
    echo "self-test FAILED: detect did not write its decision to \$GITHUB_OUTPUT" >&2
    failures=$((failures + 1))
  fi
}

self_test() {
  failures=0
  self_test_classify
  self_test_detect
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
