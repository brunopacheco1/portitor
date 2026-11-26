#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The two-party scenario with two processes of each component on one
# PostgreSQL database each (ADR 0029, section 7): two issuers, two
# identities and two connectors per participant. A small proxy
# (scripts/round-robin-proxy.py) stands in front of each pair, as the load
# balancer of a real deployment, and sends each request to the next
# process in turn. Every address a participant or the script uses is the
# proxy's: the DIDs, the public URLs, the STS, the management APIs.
#
#  1. One new database per component on the server of
#     PORTITOR_TEST_DATABASE_URL; the bundle, the issuer pair, the data.
#  2. The identity pairs; the credentials, as in run-local.sh.
#  3. The connector pairs.
#  4. One process stopped in the middle: each check below stops (SIGKILL)
#     the process that holds the work, and the other process must finish
#     it. The proxy holds the counter-party's answers back (HTTP 503) for a
#     moment first, so that the work is still pending when the process
#     stops. A stopped process is started again only after its check.
#     a. Negotiation: the consumer process that took the request stops
#        while its ContractRequestMessage waits to be sent (TD-53, the
#        outbox).
#     b. Transfer: the provider process that took the transfer request
#        stops while its TransferStartMessage waits to be sent (TD-53).
#        The consumer fetches the data through the other process.
#     c. Credential delivery: the issuer process that accepted a
#        credential request stops before it delivered (TD-57).
#  5. Every process is up again: the steps of the scenario
#     (scenarios/two-party/steps.sh), with every request spread over both
#     processes of each component.
#  6. Each process served requests (the proxy's log).
# A failed check of step 4 is reported and the run goes on; the script
# exits non-zero at the end if any check failed. Steps 5 and 6 stop at
# the first failure.
#
# Usage: scenarios/two-party/run-replicas.sh [portitor-connector [portitor-identity [portitor-issuer]]]
#   Without paths it builds the debug binaries. The identity and issuer
#   binaries default to the ones next to the connector.
# Environment:
#   PORTITOR_TEST_DATABASE_URL  required: a PostgreSQL server (16 to 19)
#        whose user may create databases, as for the store tests
#        (docs/TESTING.md, "PostgreSQL"); the script makes five databases
#        there and drops them at the end
#   PORTITOR_TEST_DATABASE_PASSWORD  that user's password, if it needs one
#   TAKEOVER_SECONDS  how long the other process may take to finish the
#        work of a stopped one (default 75: a lease lasts 30 seconds)
#   KEEP=1  leave the processes and the databases at the end
# Needs curl, jq, openssl, python3 and psql. Uses the ports 18260 to 18499.

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

cd "$(dirname "$0")/../.."
: "${PORTITOR_TEST_DATABASE_URL:?set PORTITOR_TEST_DATABASE_URL to a PostgreSQL server where its user may create databases}"
if [[ $# -eq 0 ]]; then
  cargo build --quiet -p portitor-connector -p portitor-identity -p portitor-issuer
fi
CONNECTOR=${1:-${CARGO_TARGET_DIR:-target}/debug/portitor-connector}
IDENTITY=${2:-$(dirname "$CONNECTOR")/portitor-identity}
ISSUER=${3:-$(dirname "$CONNECTOR")/portitor-issuer}
TAKEOVER_SECONDS=${TAKEOVER_SECONDS:-75}

# Who listens where. The address of each component is the proxy's; its
# two processes listen on the same port + 100 and + 200.
DATA=127.0.0.1:18270
ISSUER_ADDRESS=127.0.0.1:18260 # DID document, issuer service, status lists
ISSUER_MANAGEMENT=127.0.0.1:18261
PROVIDER=127.0.0.1:18280 # DSP and the data plane
PROVIDER_MANAGEMENT=127.0.0.1:18281
PROVIDER_IDENTITY=127.0.0.1:18282 # DID document, STS, credential service
PROVIDER_IDENTITY_MANAGEMENT=127.0.0.1:18283
CONSUMER=127.0.0.1:18290
CONSUMER_MANAGEMENT=127.0.0.1:18291
CONSUMER_IDENTITY=127.0.0.1:18292
CONSUMER_IDENTITY_MANAGEMENT=127.0.0.1:18293
PROXY_CONTROL=127.0.0.1:18299

backend() { # address replica(1|2): where that process listens
  local address=$1 replica=$2
  echo "${address%:*}:$((${address##*:} + 100 * replica))"
}

WORK=$(mktemp -d)
PIDS=()                # the proxy, the data server, the log followers
declare -A PROCESS     # "name replica" -> pid of a Portitor process
FAILED=()
# shellcheck source=scripts/test-databases.sh
source scripts/test-databases.sh # test_database_create, test_database_settings, test_databases_drop

cleanup() {
  local status=$?
  if [[ ${KEEP:-} == 1 ]]; then
    echo "KEEP=1: processes ${PROCESS[*]} ${PIDS[*]}, databases ${TEST_DATABASES[*]}, files in $WORK"
    exit "$status"
  fi
  for pid in "${PROCESS[@]}" "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  test_databases_drop
  rm -rf "$WORK"
  exit "$status"
}
trap cleanup EXIT

step() { printf '\n== %s\n' "$*"; }
# shellcheck source=scripts/participants.sh
source scripts/participants.sh # fail, wait_for, serve, did_web, AUTHORITY, register, request_credentials
PROVIDER_ID=$(did_web "$PROVIDER_IDENTITY")
CONSUMER_ID=$(did_web "$CONSUMER_IDENTITY")

check_failed() { # what: a check of step 4 failed; the run goes on
  printf 'CHECK FAILED: %s\n' "$*" >&2
  FAILED+=("$*")
}

# --- the processes -----------------------------------------------------------------

# Each function starts one process of a component; both processes of a
# component get the same settings, keys and database, and differ only in
# the ports they listen on. Their logs go to <name>-<replica>.log.
issuer_process() { # replica
  local r=$1 dir=$WORK/issuer
  test_database_settings issuer
  env PORTITOR_PARTICIPANT_ID="$AUTHORITY" \
    PORTITOR_BIND="$(backend "$ISSUER_ADDRESS" "$r")" \
    PORTITOR_ISSUER_SERVICE_URL="http://$ISSUER_ADDRESS/issuer" \
    PORTITOR_ISSUER_KEY_FILE="$dir/issuer-key.pem" \
    PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE="$ISSUER_CREDENTIALS" \
    "${DATABASE[@]}" \
    PORTITOR_MANAGEMENT_BIND="$(backend "$ISSUER_MANAGEMENT" "$r")" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/management.key" \
    PORTITOR_ISSUER_BUNDLE_KEY_FILE="$WORK/bundle/bundle-key.jwk" \
    PORTITOR_ISSUER_BUNDLE_DIR="$WORK/bundle/bundles" \
    PORTITOR_ALLOW_HTTP=true \
    "$ISSUER" >>"$WORK/issuer-$r.log" 2>&1 &
  PROCESS["issuer $r"]=$!
}

identity_process() { # name replica
  local name=$1 r=$2 dir=$WORK/$1 address management did
  address=$([[ $name == provider ]] && echo "$PROVIDER_IDENTITY" || echo "$CONSUMER_IDENTITY")
  management=$([[ $name == provider ]] && echo "$PROVIDER_IDENTITY_MANAGEMENT" || echo "$CONSUMER_IDENTITY_MANAGEMENT")
  did=$(did_web "$address")
  test_database_settings "$name-identity"
  env PORTITOR_PARTICIPANT_ID="$did" \
    PORTITOR_BIND="$(backend "$address" "$r")" \
    PORTITOR_CREDENTIAL_SERVICE_URL="http://$address/cs" \
    PORTITOR_IDENTITY_KEY_FILE="$dir/identity-key.pem" \
    "${DATABASE[@]}" \
    PORTITOR_MANAGEMENT_BIND="$(backend "$management" "$r")" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/identity-management.key" \
    PORTITOR_STS_CLIENT_SECRET_FILE="$dir/sts-client-secret" \
    PORTITOR_TRUSTED_ISSUERS="$AUTHORITY" \
    PORTITOR_ALLOW_HTTP=true \
    "$IDENTITY" >>"$WORK/$name-identity-$r.log" 2>&1 &
  PROCESS["$name-identity $r"]=$!
}

connector_process() { # name replica
  local name=$1 r=$2 dir=$WORK/$1 address management identity more=()
  if [[ $name == provider ]]; then
    address=$PROVIDER management=$PROVIDER_MANAGEMENT identity=$PROVIDER_IDENTITY
    # the provider follows the newest bundle the issuer serves (ADR 0005)
    more=(PORTITOR_DATASETS_FILE="$WORK/datasets.json"
      PORTITOR_BUNDLE_URL="http://$ISSUER_ADDRESS/bundles/" PORTITOR_BUNDLE_AUTHORITY="$AUTHORITY"
      PORTITOR_BUNDLE_CACHE_DIR="$dir/bundle-cache-$r")
  else
    address=$CONSUMER management=$CONSUMER_MANAGEMENT identity=$CONSUMER_IDENTITY
  fi
  test_database_settings "$name-connector"
  env PORTITOR_PARTICIPANT_ID="$(did_web "$identity")" \
    PORTITOR_BIND="$(backend "$address" "$r")" \
    PORTITOR_PUBLIC_URL="http://$address" \
    "${DATABASE[@]}" \
    PORTITOR_STS_URL="http://$identity/sts/token" \
    PORTITOR_STS_CLIENT_SECRET_FILE="$dir/sts-client-secret" \
    PORTITOR_TRUSTED_ISSUERS="$AUTHORITY" \
    PORTITOR_STATUS_LIST_CACHE_SECONDS=0 \
    PORTITOR_POLICY_CREDENTIALS=DataProcessorCredential \
    PORTITOR_MANAGEMENT_BIND="$(backend "$management" "$r")" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/management.key" \
    PORTITOR_ALLOW_HTTP=true \
    "${more[@]}" \
    "$CONNECTOR" >>"$WORK/$name-connector-$r.log" 2>&1 &
  PROCESS["$name-connector $r"]=$!
}

start() { # name replica: starts that process
  local name=$1 r=$2
  case $name in
    issuer) issuer_process "$r" ;;
    *-identity) identity_process "${name%-identity}" "$r" ;;
    *-connector) connector_process "${name%-connector}" "$r" ;;
  esac
}

stop() { # name replica: stops that process at once, as a crash would
  local name=$1 r=$2
  kill -KILL "${PROCESS["$name $r"]}" 2>/dev/null || true
  wait "${PROCESS["$name $r"]}" 2>/dev/null || true
  unset "PROCESS[$name $r]"
  echo "stopped $name, process $r"
}

health_of() { # name replica: the management address of that process, once it is healthy
  local name=$1 r=$2 management
  case $name in
    issuer) management=$ISSUER_MANAGEMENT ;;
    provider-identity) management=$PROVIDER_IDENTITY_MANAGEMENT ;;
    consumer-identity) management=$CONSUMER_IDENTITY_MANAGEMENT ;;
    provider-connector) management=$PROVIDER_MANAGEMENT ;;
    consumer-connector) management=$CONSUMER_MANAGEMENT ;;
  esac
  wait_for "http://$(backend "$management" "$r")/api/check/readiness"
}

# the proxy: one listener per address above, round robin over its two processes
listener() { # name address
  echo "$1=$2=$(backend "$2" 1),$(backend "$2" 2)"
}
python3 scripts/round-robin-proxy.py --log "$WORK/proxy.log" --control "$PROXY_CONTROL" \
  "$(listener issuer "$ISSUER_ADDRESS")" "$(listener issuer-management "$ISSUER_MANAGEMENT")" \
  "$(listener provider "$PROVIDER")" "$(listener provider-management "$PROVIDER_MANAGEMENT")" \
  "$(listener provider-identity "$PROVIDER_IDENTITY")" \
  "$(listener provider-identity-management "$PROVIDER_IDENTITY_MANAGEMENT")" \
  "$(listener consumer "$CONSUMER")" "$(listener consumer-management "$CONSUMER_MANAGEMENT")" \
  "$(listener consumer-identity "$CONSUMER_IDENTITY")" \
  "$(listener consumer-identity-management "$CONSUMER_IDENTITY_MANAGEMENT")" \
  >"$WORK/proxy-stdout.log" 2>&1 &
PIDS+=($!)
proxy() { # down|up listener [path prefix]
  curl -sf -X POST "http://$PROXY_CONTROL/$1/$2${3:+/$3}" || fail "proxy: $*"
}
served_by() { # listener method path-pattern: the process (1 or 2) that answered the last such request
  local name=$1 method=$2 pattern=$3 line address
  line=$(awk -v n="$name" -v m="$method" -v p="$pattern" '$1 == n && $2 == m && $3 ~ p && $4 != "-"' \
    "$WORK/proxy.log" | tail -n 1)
  [[ -n $line ]] || return 1
  address=$(awk '{print $4}' <<<"$line")
  case ${address##*:} in
    183*) echo 1 ;;
    184*) echo 2 ;;
    *) return 1 ;;
  esac
}

# --- 1. the databases, the bundle, the issuers and the data -------------------------

step "One database per component on the test server"
for component in issuer provider-identity consumer-identity provider-connector consumer-connector; do
  test_database_create "$component" || fail "create the database of $component"
  test_database_settings "$component"
  echo "$component: ${DATABASE[0]#PORTITOR_DATABASE_URL=}"
done

step "Build and sign the HealthDCAT-AP demo bundle for $AUTHORITY"
AUTHORITY=$AUTHORITY scripts/demo-bundle.sh "$WORK/bundle" "$ISSUER" >/dev/null ||
  fail "build the demo bundle"

step "Two issuer processes behind $ISSUER_ADDRESS"
mkdir -p "$WORK/issuer"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$WORK/issuer/issuer-key.pem" 2>/dev/null
openssl rand -hex 32 >"$WORK/issuer/management.key"
# both start together: they migrate the database once, under a lock (ADR 0029, D2)
start issuer 1
start issuer 2
health_of issuer 1
health_of issuer 2

serve data "$DATA" examples/data
jq --arg data "http://$DATA/" 'map(.["portitor:source"] |= sub("^http://data/"; $data))' \
  examples/healthdcat-datasets.json >"$WORK/datasets.json"

# --- 2. the identities and the credentials ------------------------------------------

step "Two identity processes per participant: provider $PROVIDER_ID, consumer $CONSUMER_ID"
for name in provider consumer; do
  mkdir -p "$WORK/$name"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$WORK/$name/identity-key.pem" 2>/dev/null
  openssl rand -hex 32 >"$WORK/$name/identity-management.key"
  openssl rand -hex 32 >"$WORK/$name/sts-client-secret"
  openssl rand -hex 32 >"$WORK/$name/management.key"
  start "$name-identity" 1
  start "$name-identity" 2
done
for name in provider consumer; do
  health_of "$name-identity" 1
  health_of "$name-identity" 2
done

step "The issuer registers both participants; each identity asks it for its credentials"
register "$PROVIDER_ID" "$(jq -nc --arg m "$MEMBER_OF" '{memberOf: $m}')"
register "$CONSUMER_ID" "$(jq -nc --arg m "$MEMBER_OF" '{memberOf: $m, processing: "eu"}')"
request_credentials provider "$PROVIDER_IDENTITY_MANAGEMENT" membership-vc20
request_credentials consumer "$CONSUMER_IDENTITY_MANAGEMENT" membership-vc20 data-processor-vc20

# --- 3. the connectors ------------------------------------------------------------

step "Two connector processes per participant: the provider on $PROVIDER, the consumer on $CONSUMER"
for name in provider consumer; do
  start "$name-connector" 1
  start "$name-connector" 2
done
for name in provider consumer; do
  health_of "$name-connector" 1
  health_of "$name-connector" 2
done
wait_for "http://$DATA/hospital-admissions.csv"

# the connector's management API, through the proxy
api() { # participant method path [body]
  local who=$1 method=$2 path=$3 body=${4-} address
  address=$([[ $who == provider ]] && echo "$PROVIDER_MANAGEMENT" || echo "$CONSUMER_MANAGEMENT")
  local args=(-sS --fail-with-body -X "$method" "http://$address$path"
    -H "authorization: Bearer $(cat "$WORK/$who/management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}
until_state() { # participant path states(regex) seconds: prints the record once in one of them
  local who=$1 path=$2 states=$3 seconds=$4 view state=unknown
  for _ in $(seq 1 $((seconds * 2))); do
    if view=$(api "$who" GET "$path" 2>/dev/null); then
      state=$(jq -r '.state' <<<"$view")
      if [[ $state =~ ^($states)$ ]]; then
        echo "$view"
        return 0
      fi
    fi
    sleep 0.5
  done
  echo "$path still $state after $seconds seconds" >&2
  return 1
}

# --- 4. one process stopped in the middle ---------------------------------------------

step "The consumer's catalog of the provider, and the hospital-admissions offer"
CATALOG=$(api consumer POST /v1/catalog/request "$(jq -nc --arg addr "http://$PROVIDER/dsp/2025-1" \
  --arg id "$PROVIDER_ID" '{counterPartyAddress: $addr, counterPartyId: $id, protocol: "dataspace-protocol-http:2025-1"}')") ||
  fail "the consumer reads the provider's catalog"
ADMISSIONS=https://provider.example/datasets/hospital-admissions
OFFER=$(jq -ce --arg d "$ADMISSIONS" '.dataset[] | select(.["@id"] == $d) | .hasPolicy[]
  | select(.["@id"] == "hospital-admissions:offer:until-2030")' <<<"$CATALOG") ||
  fail "no hospital-admissions offer in the catalog"

step "4a. Negotiation: the consumer process that took it stops before its request is sent"
proxy down provider
NEGOTIATION=$(api consumer POST /v1/negotiations "$(jq -nc --arg addr "http://$PROVIDER/dsp/2025-1" \
  --arg id "$PROVIDER_ID" --arg d "$ADMISSIONS" --argjson offer "$OFFER" \
  '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: $d, offer: $offer}')") ||
  fail "start the negotiation"
NPID=$(jq -r '.consumerPid' <<<"$NEGOTIATION")
HOLDER=$(served_by consumer-management POST '^/v1/negotiations$') || fail "no process took the negotiation"
sleep 0.3 # the process now waits to send the request again (the provider answered 503)
stop consumer-connector "$HOLDER"
proxy up provider
AGREEMENT=
if FINAL=$(until_state consumer "/v1/negotiations/$NPID" 'FINALIZED|TERMINATED' "$TAKEOVER_SECONDS"); then
  if [[ $(jq -r '.state' <<<"$FINAL") == FINALIZED ]]; then
    AGREEMENT=$(jq -r '.agreement["@id"] // .agreementId' <<<"$FINAL")
    echo "the other consumer process finished it: FINALIZED, agreement $AGREEMENT"
  else
    check_failed "4a: the negotiation ended TERMINATED: $(jq -c '{state, errorDetail}' <<<"$FINAL")"
  fi
else
  check_failed "4a: the other consumer process did not finish the negotiation within $TAKEOVER_SECONDS seconds (TD-53)"
fi
start consumer-connector "$HOLDER"
health_of consumer-connector "$HOLDER"
if [[ -z $AGREEMENT ]]; then
  # after a restart the started process resends what it finds (1.2 behaviour)
  if FINAL=$(until_state consumer "/v1/negotiations/$NPID" 'FINALIZED' 30); then
    AGREEMENT=$(jq -r '.agreement["@id"] // .agreementId' <<<"$FINAL")
    echo "finished only once the stopped process was started again"
  else
    fail "the negotiation did not finish, even after a restart"
  fi
fi

step "4b. Transfer: the provider process that took it stops before its start message is sent"
proxy down consumer
TRANSFER=$(api consumer POST /v1/transfers "$(jq -nc --arg a "$AGREEMENT" '{agreementId: $a}')") ||
  fail "request the transfer"
TPID=$(jq -r '.consumerPid' <<<"$TRANSFER")
HOLDER=
for _ in $(seq 1 50); do
  HOLDER=$(served_by provider POST '^/dsp/2025-1/transfers/request$') && break
  sleep 0.1
done
[[ -n $HOLDER ]] || fail "the transfer request did not reach the provider"
sleep 0.3 # the provider process now waits to send its start message again (503)
stop provider-connector "$HOLDER"
proxy up consumer
STARTED=
if STARTED=$(until_state consumer "/v1/transfers/$TPID" 'STARTED|TERMINATED' "$TAKEOVER_SECONDS"); then
  if [[ $(jq -r '.state' <<<"$STARTED") == STARTED ]]; then
    echo "the other provider process finished it: STARTED"
  else
    check_failed "4b: the transfer ended TERMINATED: $(jq -c '{state, errorDetail}' <<<"$STARTED")"
    STARTED=
  fi
else
  check_failed "4b: the other provider process did not start the transfer within $TAKEOVER_SECONDS seconds (TD-53)"
  STARTED=
fi
if [[ -n $STARTED ]]; then
  ENDPOINT=$(jq -r '.dataAddress.endpoint' <<<"$STARTED")
  TOKEN=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' <<<"$STARTED")
  if curl -sf "$ENDPOINT" -H "authorization: Bearer $TOKEN" -o "$WORK/fetched" &&
    cmp -s "$WORK/fetched" examples/data/hospital-admissions.csv; then
    echo "fetched the data through the other provider process"
  else
    check_failed "4b: the data could not be fetched from the other provider process"
  fi
  api consumer POST "/v1/transfers/$TPID/completion" >/dev/null || check_failed "4b: complete the transfer"
  until_state consumer "/v1/transfers/$TPID" 'COMPLETED' 20 >/dev/null ||
    check_failed "4b: the transfer did not complete"
fi
start provider-connector "$HOLDER"
health_of provider-connector "$HOLDER"

step "4c. Credential delivery: the issuer process that accepted the request stops before it delivers"
# The provider asks for a second MembershipCredential (the steps of
# step 5 revoke the consumer's one, so the consumer keeps one only). The
# issuer cannot reach the provider's credential service for a moment.
proxy down provider-identity cs
KEY=$(cat "$WORK/provider/identity-management.key")
HOLDER_PID=$(curl -sS --fail-with-body -X POST "http://$PROVIDER_IDENTITY_MANAGEMENT/v1/credential-requests" \
  -H "authorization: Bearer $KEY" -H 'content-type: application/json' \
  -d "$(jq -nc --arg issuer "$AUTHORITY" '{issuerDid: $issuer, credentialIds: ["membership-vc20"]}')" |
  jq -r '.holderPid') || fail "the provider's second credential request"
HOLDER=
for _ in $(seq 1 50); do
  HOLDER=$(served_by issuer POST '^/issuer/credentials$') && break
  sleep 0.1
done
if [[ -z $HOLDER ]]; then
  check_failed "4c: the credential request did not reach the issuer"
else
  sleep 0.5 # the issuer process decides, then waits to deliver again (503)
  stop issuer "$HOLDER"
  proxy up provider-identity
  STATUS=unknown
  for _ in $(seq 1 $((TAKEOVER_SECONDS * 2))); do
    STATUS=$(curl -sf "http://$PROVIDER_IDENTITY_MANAGEMENT/v1/credential-requests/$HOLDER_PID" \
      -H "authorization: Bearer $KEY" | jq -r '.status') || STATUS=unknown
    [[ $STATUS == ISSUED || $STATUS == REJECTED ]] && break
    sleep 0.5
  done
  if [[ $STATUS == ISSUED ]]; then
    echo "the other issuer process delivered the credential"
  else
    check_failed "4c: the other issuer process did not deliver the credential within $TAKEOVER_SECONDS seconds (TD-57): $STATUS"
  fi
  start issuer "$HOLDER"
  health_of issuer "$HOLDER"
fi
proxy up provider-identity

# --- 5. the scenario's steps, spread over both processes ----------------------------

step "5. The scenario's steps, every request spread over both processes of each component"
# steps.sh reads the provider's log for its audit lines: both processes' logs, followed into one file
touch "$WORK/provider-connector-1.log" "$WORK/provider-connector-2.log"
tail -q -n +1 -F "$WORK/provider-connector-1.log" "$WORK/provider-connector-2.log" \
  >"$WORK/provider-connector.log" 2>/dev/null &
PIDS+=($!)
env PROVIDER="http://$PROVIDER" MANAGEMENT="http://$CONSUMER_MANAGEMENT" \
  PROVIDER_MANAGEMENT="http://$PROVIDER_MANAGEMENT" PROVIDER_KEY_FILE="$WORK/provider/management.key" \
  KEY_FILE="$WORK/consumer/management.key" \
  CONSUMER_ID="$CONSUMER_ID" PROVIDER_ID="$PROVIDER_ID" \
  ISSUER_MANAGEMENT="http://$ISSUER_MANAGEMENT" ISSUER_KEY_FILE="$WORK/issuer/management.key" \
  PROVIDER_LOG="$WORK/provider-connector.log" \
  DATA=examples/data TMPDIR="$WORK" scenarios/two-party/steps.sh ||
  fail "the scenario's steps"

# --- 6. both processes did work -----------------------------------------------------

step "6. Each process served requests"
for name in issuer provider provider-identity consumer consumer-identity \
  issuer-management provider-management consumer-management; do
  for r in 1 2; do
    count=$(awk -v n="$name" -v r="$r" '$1 == n && $4 ~ (":" (r == 1 ? "183" : "184")) ' "$WORK/proxy.log" | wc -l)
    echo "$name, process $r: $count requests"
    [[ $count -gt 0 ]] || fail "$name, process $r served no request"
  done
done

if [[ ${#FAILED[@]} -gt 0 ]]; then
  printf '\n%s check(s) of step 4 failed:\n' "${#FAILED[@]}" >&2
  printf -- '- %s\n' "${FAILED[@]}" >&2
  exit 1
fi
step "Done: two processes of each component on one PostgreSQL database each; the scenario's steps passed with requests spread over both, and the other process finished the work of a stopped one"
