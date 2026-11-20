#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The gate job of a workflow (CI gate, TCK gate, Dependencies gate): the
# one required check of each (CONTRIBUTING.md, "Required checks"). It
# reads the results of the jobs the gate job `needs` and decides whether
# the workflow passed.
#
# A job skipped by its `if:` reports "skipped", and GitHub takes a
# skipped required check as passed. So the gate job always runs, and
# tells a job that was skipped on purpose (the change is prose only,
# scripts/ci_changes.sh) from one that should have run.
#
# The gate FAILS when:
#   - the `changes` job did not succeed (then nobody knows what the
#     change needs);
#   - a job failed or was cancelled;
#   - a job that always runs (ALWAYS_JOBS) did not succeed;
#   - the change needs the gated jobs (the output GATED_BY of `changes`
#     is anything but the text "false", also when it is missing) and one
#     of them (NEEDED_JOBS) did not succeed.
# It passes otherwise: every job succeeded, or the gated jobs were
# skipped because `changes` said "false".
#
# Usage:
#   scripts/ci_gate.sh check      in a gate job; reads the environment below
#   scripts/ci_gate.sh self-test  checks the rule on fixed `needs` objects
#
# Environment of `check`:
#   NEEDS        the gate job's `needs` context as JSON: ${{ toJSON(needs) }}
#   GATED_BY     the output of the `changes` job that says whether the
#                gated jobs are needed: `build` or `code`
#   NEEDED_JOBS  the gated jobs, separated by spaces
#   ALWAYS_JOBS  the jobs that run for every change, separated by spaces
#                (may be empty)
#
# Needs jq.

set -euo pipefail

# Prints why the gate fails, or nothing when it passes.
#   $1 needs (JSON)  $2 gated by  $3 needed jobs  $4 always jobs
reason_to_fail() {
  local needs=$1 gated_by=$2 needed_jobs=$3 always_jobs=$4
  local job

  if [[ $gated_by != build && $gated_by != code ]]; then
    echo "GATED_BY is '$gated_by', not build or code"
    return
  fi
  if [[ -z $needed_jobs ]]; then
    echo "NEEDED_JOBS is empty"
    return
  fi
  if ! jq -e 'type == "object"' <<<"$needs" >/dev/null 2>&1; then
    echo "NEEDS is not a JSON object"
    return
  fi
  if ! jq -e '.changes.result == "success"' <<<"$needs" >/dev/null; then
    echo "the changes job did not succeed"
    return
  fi
  if ! jq -e 'all(.[]; .result == "success" or .result == "skipped")' <<<"$needs" >/dev/null; then
    echo "a job failed or was cancelled"
    return
  fi
  for job in $always_jobs; do
    if ! succeeded "$needs" "$job"; then
      echo "the job $job always runs, and it did not succeed"
      return
    fi
  done
  # anything but the text "false" means "needed": a missing or empty
  # output is a doubt, and a doubt runs everything
  if jq -e --arg output "$gated_by" '.changes.outputs[$output] != "false"' <<<"$needs" >/dev/null; then
    for job in $needed_jobs; do
      if ! succeeded "$needs" "$job"; then
        echo "the change needs the job $job, and it did not succeed (skipped?)"
        return
      fi
    done
  fi
}

# Whether the job `job` is in `needs` with the result "success".
succeeded() { # needs job
  local needs=$1 job=$2
  jq -e --arg job "$job" '.[$job].result == "success"' <<<"$needs" >/dev/null
}

check() {
  local needs=${NEEDS:-} reason
  # one line per job, for the reader of the log
  jq -r 'to_entries[] | "\(.key): \(.value.result)"' <<<"$needs" 2>/dev/null || true
  reason=$(reason_to_fail "$needs" "${GATED_BY:-}" "${NEEDED_JOBS:-}" "${ALWAYS_JOBS:-}")
  if [[ -n $reason ]]; then
    echo "::error::$reason"
    exit 1
  fi
  echo "gate passed"
}

# --- self-test ---------------------------------------------------------------

readonly PASS=pass
readonly FAIL=fail

# A `needs` object: the `changes` job with its two outputs, then one
# result per job, as name=result words.
#   $1 result of changes  $2 output code  $3 output build  $4... name=result
# An output given as "-" is left out, as when the job wrote none.
needs_of() {
  local changes=$1 code=$2 build=$3 pair
  shift 3
  local outputs='{}'
  if [[ $code != - ]]; then outputs=$(jq -c --arg v "$code" '.code = $v' <<<"$outputs"); fi
  if [[ $build != - ]]; then outputs=$(jq -c --arg v "$build" '.build = $v' <<<"$outputs"); fi
  local needs
  needs=$(jq -nc --arg result "$changes" --argjson outputs "$outputs" \
    '{changes: {result: $result, outputs: $outputs}}')
  for pair in "$@"; do
    needs=$(jq -c --arg job "${pair%%=*}" --arg result "${pair#*=}" \
      '.[$job] = {result: $result, outputs: {}}' <<<"$needs")
  done
  echo "$needs"
}

# One case: the expected decision, what the case is, then the four
# arguments of reason_to_fail.
expect() { # want what needs gated_by needed_jobs always_jobs
  local want=$1 what=$2 needs=$3 gated_by=$4 needed_jobs=$5 always_jobs=$6
  local reason got=$PASS
  reason=$(reason_to_fail "$needs" "$gated_by" "$needed_jobs" "$always_jobs")
  if [[ -n $reason ]]; then got=$FAIL; fi
  if [[ $got != "$want" ]]; then
    echo "self-test FAILED: $what: expected $want, got $got ${reason:+($reason)}" >&2
    failures=$((failures + 1))
  fi
}

# The three gates, as the workflows call them.
expect_ci() { # want what needs
  local want=$1 what=$2 needs=$3
  expect "$want" "CI gate, $what" "$needs" build "check sonar" "reuse docs"
}
expect_tck() { # want what needs
  local want=$1 what=$2 needs=$3
  expect "$want" "TCK gate, $what" "$needs" build "tck dcp-tck dcp-verifier-tck dcp-issuer-tck" ""
}
expect_dependencies() { # want what needs is-pull-request
  local want=$1 what=$2 needs=$3 is_pull_request=$4
  local needed="deny vet"
  if [[ $is_pull_request == true ]]; then needed="deny vet review"; fi
  expect "$want" "Dependencies gate, $what" "$needs" code "$needed" ""
}

self_test() {
  failures=0
  local ok=success skip=skipped

  # --- CI gate: check and sonar need `build`; reuse and docs always run
  expect_ci "$PASS" "all succeeded" \
    "$(needs_of $ok true true check=$ok reuse=$ok docs=$ok sonar=$ok)"
  expect_ci "$PASS" "prose only: check and sonar skipped on purpose" \
    "$(needs_of $ok false false check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$PASS" "fuzzing only: check and sonar skipped on purpose" \
    "$(needs_of $ok true false check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "check failed" \
    "$(needs_of $ok true true check=failure reuse=$ok docs=$ok sonar=$ok)"
  expect_ci "$FAIL" "sonar cancelled" \
    "$(needs_of $ok true true check=$ok reuse=$ok docs=$ok sonar=cancelled)"
  expect_ci "$FAIL" "a failure in a prose-only change" \
    "$(needs_of $ok false false check=$skip reuse=failure docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "code changed, check skipped" \
    "$(needs_of $ok true true check=$skip reuse=$ok docs=$ok sonar=$ok)"
  expect_ci "$FAIL" "code changed, sonar skipped" \
    "$(needs_of $ok true true check=$ok reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "reuse skipped" \
    "$(needs_of $ok false false check=$skip reuse=$skip docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "docs skipped" \
    "$(needs_of $ok true true check=$ok reuse=$ok docs=$skip sonar=$ok)"
  expect_ci "$FAIL" "the changes job failed, everything else skipped" \
    "$(needs_of failure - - check=$skip reuse=$skip docs=$skip sonar=$skip)"
  expect_ci "$FAIL" "the changes job failed, the jobs that always run succeeded" \
    "$(needs_of failure - - check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "the changes job cancelled" \
    "$(needs_of cancelled - - check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "the changes job skipped" \
    "$(needs_of $skip - - check=$skip reuse=$ok docs=$ok sonar=$skip)"
  # a doubt about the outputs means "needed"
  expect_ci "$FAIL" "outputs are empty texts, check and sonar skipped" \
    "$(needs_of $ok "" "" check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "no outputs at all, check and sonar skipped" \
    "$(needs_of $ok - - check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "the output is False, not false" \
    "$(needs_of $ok False False check=$skip reuse=$ok docs=$ok sonar=$skip)"
  expect_ci "$PASS" "no outputs at all, and everything ran" \
    "$(needs_of $ok - - check=$ok reuse=$ok docs=$ok sonar=$ok)"
  # only `build` counts for this gate, not `code`
  expect_ci "$FAIL" "code=false but build=true, check skipped" \
    "$(needs_of $ok false true check=$skip reuse=$ok docs=$ok sonar=$skip)"
  # a job the gate names but `needs` does not have (a renamed job)
  expect_ci "$FAIL" "a needed job is missing from needs" \
    "$(needs_of $ok true true check=$ok reuse=$ok docs=$ok)"
  expect_ci "$FAIL" "a job that always runs is missing from needs" \
    "$(needs_of $ok false false check=$skip docs=$ok sonar=$skip)"
  expect_ci "$FAIL" "needs is empty" '{}'
  expect_ci "$FAIL" "needs is no JSON" ''
  expect_ci "$FAIL" "needs is a list" '[]'

  # --- TCK gate: the four TCK jobs need `build`
  expect_tck "$PASS" "all succeeded" \
    "$(needs_of $ok true true tck=$ok dcp-tck=$ok dcp-verifier-tck=$ok dcp-issuer-tck=$ok)"
  expect_tck "$PASS" "prose only: all skipped on purpose" \
    "$(needs_of $ok false false tck=$skip dcp-tck=$skip dcp-verifier-tck=$skip dcp-issuer-tck=$skip)"
  expect_tck "$PASS" "fuzzing only: all skipped on purpose" \
    "$(needs_of $ok true false tck=$skip dcp-tck=$skip dcp-verifier-tck=$skip dcp-issuer-tck=$skip)"
  expect_tck "$FAIL" "one TCK failed" \
    "$(needs_of $ok true true tck=$ok dcp-tck=failure dcp-verifier-tck=$ok dcp-issuer-tck=$ok)"
  expect_tck "$FAIL" "one TCK cancelled" \
    "$(needs_of $ok true true tck=$ok dcp-tck=$ok dcp-verifier-tck=$ok dcp-issuer-tck=cancelled)"
  expect_tck "$FAIL" "code changed, the last TCK skipped" \
    "$(needs_of $ok true true tck=$ok dcp-tck=$ok dcp-verifier-tck=$ok dcp-issuer-tck=$skip)"
  expect_tck "$FAIL" "the changes job failed" \
    "$(needs_of failure - - tck=$skip dcp-tck=$skip dcp-verifier-tck=$skip dcp-issuer-tck=$skip)"
  expect_tck "$FAIL" "no outputs, all skipped" \
    "$(needs_of $ok - - tck=$skip dcp-tck=$skip dcp-verifier-tck=$skip dcp-issuer-tck=$skip)"

  # --- Dependencies gate: deny and vet need `code`; the review too, on
  # a pull request only (the job does not exist for a push)
  expect_dependencies "$PASS" "pull request, all succeeded" \
    "$(needs_of $ok true true review=$ok deny=$ok vet=$ok)" true
  expect_dependencies "$PASS" "push, the review skipped" \
    "$(needs_of $ok true true review=$skip deny=$ok vet=$ok)" false
  expect_dependencies "$FAIL" "pull request, the review skipped" \
    "$(needs_of $ok true true review=$skip deny=$ok vet=$ok)" true
  expect_dependencies "$PASS" "prose only: all skipped on purpose" \
    "$(needs_of $ok false false review=$skip deny=$skip vet=$skip)" true
  expect_dependencies "$PASS" "fuzzing only, and all three ran" \
    "$(needs_of $ok true false review=$ok deny=$ok vet=$ok)" true
  # fuzzing has dependencies of its own: `code`, not `build`, counts here
  expect_dependencies "$FAIL" "fuzzing only, deny skipped" \
    "$(needs_of $ok true false review=$ok deny=$skip vet=$ok)" true
  expect_dependencies "$FAIL" "vet failed" \
    "$(needs_of $ok true true review=$ok deny=$ok vet=failure)" true
  expect_dependencies "$FAIL" "push, vet skipped" \
    "$(needs_of $ok true true review=$skip deny=$ok vet=$skip)" false
  expect_dependencies "$FAIL" "the changes job failed" \
    "$(needs_of failure - - review=$skip deny=$skip vet=$skip)" false
  expect_dependencies "$FAIL" "no outputs, all skipped" \
    "$(needs_of $ok - - review=$skip deny=$skip vet=$skip)" false

  # --- a gate that is set up wrongly fails
  local all_ok
  all_ok=$(needs_of $ok true true check=$ok)
  expect "$FAIL" "GATED_BY is not set" "$all_ok" "" "check" ""
  expect "$FAIL" "GATED_BY is another word" "$all_ok" "Build" "check" ""
  expect "$FAIL" "NEEDED_JOBS is empty" "$all_ok" build "" ""
  expect "$PASS" "a well set up gate with one job" "$all_ok" build "check" ""

  if [[ $failures != 0 ]]; then
    echo "$failures self-test case(s) failed" >&2
    exit 1
  fi
  echo "self-test passed"
}

case "${1:-}" in
  check) check ;;
  self-test) self_test ;;
  *)
    echo "usage: $0 check | self-test" >&2
    exit 2
    ;;
esac
