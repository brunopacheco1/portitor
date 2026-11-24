#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Measures the reference load of docs/BENCHMARKS.md on this machine: the
# data plane under many, large and long pulls, the stored history, the
# catalog, negotiations at one per second, and the TD-40 rule (do
# negotiations slow the data plane down?). scripts/measure.sh measures
# the footprint and the cost of DCP.
#
# Each part starts its own participants with empty databases, as
# scenarios/two-party/demo.sh does (an issuer, a provider and a consumer, each a
# connector and an identity), and a data source that makes its bytes up
# (the `upstream` mode of the bench-load example). The consumer's
# management API negotiates and starts the transfers; the bench-load
# example then pulls from the provider's data plane with the transfers'
# tokens. Everything runs on this one machine over plain HTTP.
#
# Usage: scripts/bench.sh [part...]
#   Parts: parallel large long history catalog negotiation td40
#   (default: all, in this order). Builds the release binaries and the
#   bench-load example unless CONNECTOR and BENCH_LOAD name them.
# Environment:
#   CONNECTOR      the portitor-connector binary; portitor-identity and
#                  portitor-issuer are taken from the same folder
#   BENCH_LOAD     the bench-load binary
#   BENCH_RUNS     how often a cheap measure is repeated (default 3)
#   BENCH_LARGE_GB the size of the large pull in GiB (default 10)
#   BENCH_HISTORY  how many negotiations and transfers are stored (default 10000)
#   BENCH_SECONDS  how long each steady load of the td40 part runs (default 30)
#   BENCH_KEEP_LOGS  a folder to copy each part's logs to (default: not kept)
# Needs Linux, curl, jq, openssl and python3. Prints key=value lines; exits
# non-zero if a step fails. Every wait is bounded.

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/require-bash.sh"

# Linux only: it reads /proc and uses GNU stat and date
[[ $(uname -s) == Linux ]] || {
  echo "$0 runs on Linux only (it reads /proc)." >&2
  exit 1
}

cd "$(dirname "$0")/.."
TARGET=${CARGO_TARGET_DIR:-target}
if [[ -z ${CONNECTOR:-} ]]; then
  cargo build --quiet --release -p portitor-connector -p portitor-identity -p portitor-issuer
  CONNECTOR=$TARGET/release/portitor-connector
fi
IDENTITY=$(dirname "$CONNECTOR")/portitor-identity
ISSUER=$(dirname "$CONNECTOR")/portitor-issuer
if [[ -z ${BENCH_LOAD:-} ]]; then
  cargo build --quiet --release -p portitor-connector --example bench-load
  BENCH_LOAD=$TARGET/release/examples/bench-load
fi
RUNS=${BENCH_RUNS:-3}
LARGE_GB=${BENCH_LARGE_GB:-10}
HISTORY=${BENCH_HISTORY:-10000}
SECONDS_PER_LOAD=${BENCH_SECONDS:-30}
if [[ $# -eq 0 ]]; then
  set -- parallel large long history catalog negotiation td40
fi

UPSTREAM=127.0.0.1:18370
ISSUER_ADDRESS=127.0.0.1:18360
ISSUER_MANAGEMENT=127.0.0.1:18361
PROVIDER=127.0.0.1:18380
PROVIDER_MANAGEMENT=127.0.0.1:18381
PROVIDER_IDENTITY=127.0.0.1:18382
PROVIDER_IDENTITY_MANAGEMENT=127.0.0.1:18383
CONSUMER=127.0.0.1:18390
CONSUMER_MANAGEMENT=127.0.0.1:18391
CONSUMER_IDENTITY=127.0.0.1:18392
CONSUMER_IDENTITY_MANAGEMENT=127.0.0.1:18393

KIB=1024
MIB=$((1024 * KIB))

ROOT=$(mktemp -d)
WORK=$ROOT
PIDS=()
stop_participants() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  PIDS=()
}
cleanup() {
  stop_participants
  rm -rf "$ROOT"
}
trap cleanup EXIT

# shellcheck source=scripts/participants.sh
source scripts/participants.sh # fail, wait_for, the issuer, start_identity, request_credentials, start_connector
PROVIDER_ID=$(did_web "$PROVIDER_IDENTITY")
CONSUMER_ID=$(did_web "$CONSUMER_IDENTITY")
export RUST_LOG=${RUST_LOG:-warn} # the default, info, logs every state change
# 1,000 pulls at once are 2,000 connections at the provider
ulimit -n 16384 2>/dev/null || true
echo "open_files_limit=$(ulimit -n)"

rss_kb() { # pid field: VmRSS (now) or VmHWM (peak)
  local pid=$1 field=$2
  awk -v field="$field:" '$1 == field {print $2}' "/proc/$pid/status"
}

# write_datasets file [extra datasets]: the provider's datasets file. Every
# dataset has the offer `<id>:offer:use`. The fixed datasets get their bytes
# from the upstream; `extra` more datasets without data fill the catalog.
write_datasets() {
  local file=$1 extra=${2:-0}
  python3 - "$file" "http://$UPSTREAM/bytes" "$extra" "$((LARGE_GB * 1024 * MIB))" <<'PYTHON'
import json, sys
file, upstream, extra, large = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
KIB, MIB = 1024, 1024 * 1024
sources = {
    "pull-tiny": f"{upstream}/{KIB}",                          # 1 KiB
    "pull-8m": f"{upstream}/{8 * MIB}",
    "pull-64m": f"{upstream}/{64 * MIB}",
    "pull-large": f"{upstream}/{large}",
    "pull-steady": f"{upstream}/{5 * 64 * KIB}?pause_ms=100",  # 5 pieces: 0.4 s
    "pull-slow": f"{upstream}/{200 * 64 * KIB}?pause_ms=100",  # 200 pieces: 20 s
}
def dataset(name, source=None):
    entry = {
        "@id": name,
        "@type": "Dataset",
        "hasPolicy": [{"@id": f"{name}:offer:use", "@type": "Offer",
                       "permission": [{"action": "use"}]}],
        "distribution": [{"@type": "Distribution", "format": "HttpData-PULL"}],
    }
    if source:
        entry["portitor:source"] = source
    return entry
datasets = [dataset(name, source) for name, source in sources.items()]
datasets += [dataset(f"catalog-{n:05}") for n in range(extra)]
json.dump(datasets, open(file, "w"))
PYTHON
}

# start_participants part [extra datasets] [VAR=value settings of the provider...]:
# the upstream, the issuer, both identities with their credentials, and
# both connectors, with new keys and empty databases under $ROOT/<part>.
start_participants() {
  local part=$1 extra=${2:-0}
  shift
  [[ $# -gt 0 ]] && shift
  WORK=$ROOT/$part
  mkdir -p "$WORK"
  write_datasets "$WORK/datasets.json" "$extra"
  UPSTREAM_BIND=$UPSTREAM "$BENCH_LOAD" upstream >"$WORK/upstream.log" 2>&1 &
  PIDS+=($!)
  start_issuer
  start_identity provider "$PROVIDER_ID" "$PROVIDER_IDENTITY" "$PROVIDER_IDENTITY_MANAGEMENT"
  start_identity consumer "$CONSUMER_ID" "$CONSUMER_IDENTITY" "$CONSUMER_IDENTITY_MANAGEMENT"
  wait_for "http://$ISSUER_ADDRESS/.well-known/did.json"
  wait_for "http://$PROVIDER_IDENTITY/.well-known/did.json"
  wait_for "http://$CONSUMER_IDENTITY/.well-known/did.json"
  register "$PROVIDER_ID" "$(jq -nc --arg m "$MEMBER_OF" '{memberOf: $m}')"
  register "$CONSUMER_ID" "$(jq -nc --arg m "$MEMBER_OF" '{memberOf: $m}')"
  request_credentials provider "$PROVIDER_IDENTITY_MANAGEMENT" membership-vc20
  request_credentials consumer "$CONSUMER_IDENTITY_MANAGEMENT" membership-vc20
  openssl rand -hex 32 >"$WORK/consumer/management.key"
  start_connector provider "$PROVIDER_ID" "$PROVIDER" "$PROVIDER_IDENTITY" \
    PORTITOR_DATASETS_FILE="$WORK/datasets.json" PORTITOR_MANAGEMENT_BIND="$PROVIDER_MANAGEMENT" "$@"
  PROVIDER_PID=${PIDS[-1]}
  start_connector consumer "$CONSUMER_ID" "$CONSUMER" "$CONSUMER_IDENTITY" \
    PORTITOR_MANAGEMENT_BIND="$CONSUMER_MANAGEMENT" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$WORK/consumer/management.key"
  wait_for "http://$PROVIDER/health"
  wait_for "http://$CONSUMER/health"
}

# stop_part: stops the participants and deletes their files
stop_part() {
  stop_participants
  if [[ -n ${BENCH_KEEP_LOGS:-} ]]; then
    mkdir -p "$BENCH_KEEP_LOGS/$(basename "$WORK")"
    cp "$WORK"/*.log "$BENCH_KEEP_LOGS/$(basename "$WORK")/"
  fi
  rm -rf "$WORK"
  WORK=$ROOT
}

# tool name mode [VAR=value...]: runs bench-load, prefixes its lines with name
tool() {
  local name=$1 mode=$2
  shift 2
  env "$@" "$BENCH_LOAD" "$mode" >"$WORK/$name.out" || fail "bench-load $mode ($name)"
  sed "s/^/${name}_/" "$WORK/$name.out"
  # a load with errors is not a result, unless the caller said it counts them
  [[ $mode == catalog || -n ${ERRORS_ARE_COUNTED:-} ]] && return 0
  grep -qx 'errors=0' "$WORK/$name.out" || fail "$name had errors"
}

# contracts name dataset count [VAR=value...]: negotiations, and a transfer
# under each, through the consumer; the transfers' URLs and tokens go to
# $WORK/<name>.targets
contracts() {
  local name=$1 dataset=$2 count=$3
  shift 3
  tool "$name" contracts CONTRACTS_MANAGEMENT="http://$CONSUMER_MANAGEMENT" \
    CONTRACTS_KEY_FILE="$WORK/consumer/management.key" \
    CONTRACTS_PROVIDER_ADDRESS="http://$PROVIDER/dsp/2025-1" CONTRACTS_PROVIDER_ID="$PROVIDER_ID" \
    CONTRACTS_DATASET="$dataset" CONTRACTS_COUNT="$count" \
    CONTRACTS_TARGETS_FILE="$WORK/$name.targets" "$@"
}

# pulls name targets-file [VAR=value...]
pulls() {
  local name=$1 targets=$2
  shift 2
  tool "$name" pulls PULL_TARGETS_FILE="$targets" "$@"
}

# direct_targets file path count: `count` downloads of the upstream's path, without a token
direct_targets() {
  local file=$1 path=$2 count=$3
  for _ in $(seq 1 "$count"); do echo "http://$UPSTREAM/bytes/$path -"; done >"$file"
}

# --- data plane: many pulls at the same time ------------------------------------

# parallel_pulls count dataset bytes: `count` transfers, then all pulled at
# once through the provider, and the same downloads straight from the upstream
parallel_pulls() {
  local count=$1 dataset=$2 bytes=$3
  local name=parallel_$count
  start_participants "$name"
  contracts "${name}_setup" "$dataset" "$count" CONTRACTS_PARALLEL=16 >/dev/null
  direct_targets "$WORK/direct.targets" "$bytes" "$count"
  echo "${name}_provider_rss_before_kb=$(rss_kb "$PROVIDER_PID" VmRSS)"
  for run in $(seq 1 "$RUNS"); do
    pulls "${name}_run${run}_provider" "$WORK/${name}_setup.targets" PULL_PARALLEL="$count"
    pulls "${name}_run${run}_direct" "$WORK/direct.targets" PULL_PARALLEL="$count"
  done
  echo "${name}_provider_peak_rss_kb=$(rss_kb "$PROVIDER_PID" VmHWM)"
  stop_part
}

part_parallel() {
  parallel_pulls 100 pull-64m $((64 * MIB))
  parallel_pulls 1000 pull-8m $((8 * MIB))
}

# --- data plane: one large pull -------------------------------------------------

# sample_rss pid file: writes the process's memory to `file` every 2 seconds
sample_rss() {
  local pid=$1 file=$2
  while kill -0 "$pid" 2>/dev/null; do
    rss_kb "$pid" VmRSS >>"$file" || true
    sleep 2
  done
}

part_large() {
  start_participants large
  contracts large_setup pull-large 1 >/dev/null
  direct_targets "$WORK/direct.targets" $((LARGE_GB * 1024 * MIB)) 1
  echo "large_gib=$LARGE_GB"
  echo "large_provider_rss_before_kb=$(rss_kb "$PROVIDER_PID" VmRSS)"
  sample_rss "$PROVIDER_PID" "$WORK/rss.samples" &
  local sampler=$!
  pulls large_provider "$WORK/large_setup.targets"
  kill "$sampler" 2>/dev/null || true
  # the memory while the pull ran, one sample every 2 seconds
  echo "large_provider_rss_samples_kb=$(paste -sd, "$WORK/rss.samples")"
  echo "large_provider_peak_rss_kb=$(rss_kb "$PROVIDER_PID" VmHWM)"
  pulls large_direct "$WORK/direct.targets"
  stop_part
}

# --- data plane: a pull longer than its token lives ------------------------------

part_long() {
  # the token lives 5 seconds; the pull takes 20
  start_participants long 0 PORTITOR_DATA_TOKEN_SECONDS=5
  contracts long_setup pull-slow 1 >/dev/null
  echo "long_token_seconds=5"
  pulls long_pull "$WORK/long_setup.targets"
  # the token has expired by now: a second pull with it must be refused
  pulls long_second_pull "$WORK/long_setup.targets" PULL_EXPECT_STATUS=401
  stop_part
}

# --- stored history --------------------------------------------------------------

# history_measures name: a new negotiation, a transfer start, a token check
# (a wrong token, refused after the flow is read) and a small pull, one at a time
history_measures() {
  local name=$1
  for run in $(seq 1 "$RUNS"); do
    contracts "${name}_run${run}_new" pull-tiny 50
    pulls "${name}_run${run}_token_check" "$WORK/${name}_run${run}_new.targets" \
      PULL_SECONDS=5 PULL_TOKEN=wrong PULL_EXPECT_STATUS=401
    pulls "${name}_run${run}_tiny_pull" "$WORK/${name}_run${run}_new.targets" PULL_SECONDS=5
  done
}

part_history() {
  start_participants history
  history_measures history_empty
  # 16 at a time; a few in 10,000 may fail (BENCHMARKS, "Stored history"): they are counted
  ERRORS_ARE_COUNTED=1 contracts history_fill pull-tiny "$HISTORY" CONTRACTS_PARALLEL=16
  echo "history_provider_db_bytes=$(stat -c %s "$WORK/provider/connector.db")"
  echo "history_consumer_db_bytes=$(stat -c %s "$WORK/consumer/connector.db")"
  history_measures history_full
  echo "history_provider_peak_rss_kb=$(rss_kb "$PROVIDER_PID" VmHWM)"
  stop_part
}

# --- catalog ---------------------------------------------------------------------

part_catalog() {
  local size
  for size in 100 1000 10000; do
    # the six datasets with data are in every catalog
    start_participants "catalog_$size" $((size - 6))
    for run in $(seq 1 "$RUNS"); do
      tool "catalog_${size}_run${run}" catalog LOAD_URL="http://$PROVIDER/dsp/2025-1/catalog/request" \
        LOAD_AUDIENCE="$PROVIDER_ID" PORTITOR_PARTICIPANT_ID="$CONSUMER_ID" \
        PORTITOR_STS_URL="http://$CONSUMER_IDENTITY/sts/token" \
        PORTITOR_STS_CLIENT_SECRET_FILE="$WORK/consumer/sts-client-secret" \
        PORTITOR_TRUSTED_ISSUERS="$AUTHORITY"
    done
    echo "catalog_${size}_provider_peak_rss_kb=$(rss_kb "$PROVIDER_PID" VmHWM)"
    stop_part
  done
}

# --- negotiation: one per second -------------------------------------------------

part_negotiation() {
  start_participants negotiation
  for run in $(seq 1 "$RUNS"); do
    contracts "negotiation_run${run}" pull-tiny 30 CONTRACTS_PER_SECOND=1 CONTRACTS_TRANSFERS=0
  done
  stop_part
}

# --- TD-40: do negotiations slow the data plane down? ----------------------------

# steady_pulls name [negotiations per second]: 100 pulls in flight for
# $SECONDS_PER_LOAD seconds, alone or with new negotiations at that rate
# (`max`: 16 at a time, as fast as they go)
steady_pulls() {
  local name=$1 rate=${2:-0}
  local negotiating=
  if [[ $rate == max ]]; then
    contracts "${name}_negotiations" pull-tiny 1000000 CONTRACTS_TRANSFERS=0 \
      CONTRACTS_PARALLEL=16 CONTRACTS_SECONDS="$SECONDS_PER_LOAD" &
    negotiating=$!
  elif [[ $rate != 0 ]]; then
    contracts "${name}_negotiations" pull-tiny $((rate * SECONDS_PER_LOAD)) CONTRACTS_TRANSFERS=0 \
      CONTRACTS_PER_SECOND="$rate" &
    negotiating=$!
  fi
  pulls "$name" "$WORK/td40_setup.targets" PULL_PARALLEL=100 PULL_SECONDS="$SECONDS_PER_LOAD"
  if [[ -n $negotiating ]]; then wait "$negotiating" || fail "the negotiations of $name"; fi
}

part_td40() {
  start_participants td40
  contracts td40_setup pull-steady 100 CONTRACTS_PARALLEL=16 >/dev/null
  for run in $(seq 1 "$RUNS"); do
    steady_pulls "td40_run${run}_pulls_alone"
    steady_pulls "td40_run${run}_with_1_per_sec" 1
    steady_pulls "td40_run${run}_with_10_per_sec" 10
    steady_pulls "td40_run${run}_with_max" max
  done
  stop_part
}

for part in "$@"; do
  case $part in
    parallel | large | long | history | catalog | negotiation | td40) "part_$part" ;;
    *) fail "unknown part $part" ;;
  esac
done
