#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# A manual test (docs/TESTING.md, "Two data spaces"): one organisation in
# two data spaces, with one connector per data space (ADR 0024, option
# (a); docs/DEPLOYING.md, "Two data spaces"). Everything runs on this
# machine, over plain HTTP.
#
# Each data space has its own portitor-issuer (A and B) and one consumer
# (an identity and a connector) that trusts only that issuer. The
# organisation offers examples/datasets.json in both, with one connector
# per data space; each connector has its own database and trusts only its
# data space's issuer.
#
#   scenarios/two-data-spaces/run.sh shared     the two connectors share one
#                                         identity: one DID, with a
#                                         membership credential of each
#                                         issuer
#   scenarios/two-data-spaces/run.sh separate   one identity, and so one DID,
#                                         per data space
#
# Checks, in both modes:
#  1. In each data space the consumer reads the catalog, agrees a
#     contract, starts a transfer and fetches the data.
#  2. Across data spaces the consumer of A is refused by the
#     organisation's connector for B (HTTP 401), and the other way round.
#  3. shared only: prints what a verifier of data space A receives when it
#     asks the shared identity for MembershipCredential: both membership
#     credentials. This is the disclosure ADR 0024 names.
#
# Builds the debug binaries. Needs curl, jq, openssl and python3. KEEP=1
# keeps the logs. Exits non-zero if a check fails. Not run by CI.
# PORTITOR_TEST_DATABASE_URL (a PostgreSQL server whose user may create
# databases, as for the store tests; needs psql): each component gets a
# database of its own there, dropped at the end (scripts/test-databases.sh).
# Without it, each keeps a SQLite file.

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"
cd "$(dirname "$0")/../.."
MODE=${1:-}
if [[ $MODE != shared && $MODE != separate ]]; then
  echo "usage: scenarios/two-data-spaces/run.sh shared|separate" >&2
  exit 2
fi
cargo build --quiet -p portitor-connector -p portitor-identity -p portitor-issuer
BIN=${CARGO_TARGET_DIR:-target}/debug
WORK=$(mktemp -d)
PIDS=()
# shellcheck source=scripts/test-databases.sh
source scripts/test-databases.sh # database_for: PostgreSQL with PORTITOR_TEST_DATABASE_URL, else SQLite
cleanup() {
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  test_databases_drop
  if [[ ${KEEP:-} == 1 ]]; then echo "logs kept in $WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT
step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  for log in "$WORK"/*.log; do printf -- '--- %s\n' "$log" >&2; tail -8 "$log" >&2; done
  exit 1
}
wait_for() { # url
  local url=$1
  for _ in $(seq 1 100); do curl -sf "$url" >/dev/null && return 0; sleep 0.2; done
  fail "$url did not answer"
}
did_web() { # address
  local address=$1
  echo "did:web:${address/:/%3A}"
}
JSON=(-H 'content-type: application/json')

DATA=127.0.0.1:19070
ISSUER_A=127.0.0.1:19100; ISSUER_B=127.0.0.1:19200
DID_A=$(did_web $ISSUER_A); DID_B=$(did_web $ISSUER_B)

start_issuer() { # name address management-address
  local name=$1 address=$2 management=$3
  local dir=$WORK/$name
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/key.pem" 2>/dev/null
  openssl rand -hex 32 >"$dir/management.key"
  database_for "$name" "$dir/issuer.db" || fail "the database of $name"
  env PORTITOR_PARTICIPANT_ID="$(did_web "$address")" PORTITOR_BIND="$address" \
    PORTITOR_ISSUER_SERVICE_URL="http://$address/issuer" PORTITOR_ISSUER_KEY_FILE="$dir/key.pem" \
    PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE=examples/issuer-credentials.json \
    "${DATABASE[@]}" PORTITOR_MANAGEMENT_BIND="$management" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/management.key" PORTITOR_ALLOW_HTTP=true \
    "$BIN/portitor-issuer" >"$WORK/$name.log" 2>&1 &
  PIDS+=($!)
}
start_identity() { # name address management-address trusted-issuers
  local name=$1 address=$2 management=$3 trusted=$4
  local dir=$WORK/$name
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/key.pem" 2>/dev/null
  openssl rand -hex 32 >"$dir/management.key"
  openssl rand -hex 32 >"$dir/sts-client-secret"
  database_for "$name" "$dir/identity.db" || fail "the database of $name"
  env PORTITOR_PARTICIPANT_ID="$(did_web "$address")" PORTITOR_BIND="$address" \
    PORTITOR_CREDENTIAL_SERVICE_URL="http://$address/cs" PORTITOR_IDENTITY_KEY_FILE="$dir/key.pem" \
    "${DATABASE[@]}" PORTITOR_MANAGEMENT_BIND="$management" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/management.key" \
    PORTITOR_STS_CLIENT_SECRET_FILE="$dir/sts-client-secret" \
    PORTITOR_TRUSTED_ISSUERS="$trusted" PORTITOR_ALLOW_HTTP=true \
    "$BIN/portitor-identity" >"$WORK/$name.log" 2>&1 &
  PIDS+=($!)
}
register() { # issuer-name issuer-management did
  local issuer=$1 management=$2 did=$3
  curl -sS --fail-with-body -X POST "http://$management/v1/participants" \
    -H "authorization: Bearer $(cat "$WORK/$issuer/management.key")" "${JSON[@]}" \
    -d "$(jq -nc --arg did "$did" --arg m "data space of $issuer" '{did: $did, claims: {memberOf: $m}}')" \
    >/dev/null || fail "register $did at $issuer"
}
request_membership() { # identity-name identity-management issuer-did
  local identity=$1 management=$2 issuer_did=$3
  local key view pid
  key=$(cat "$WORK/$identity/management.key")
  pid=$(curl -sS --fail-with-body -X POST "http://$management/v1/credential-requests" \
    -H "authorization: Bearer $key" "${JSON[@]}" \
    -d "$(jq -nc --arg issuer "$issuer_did" '{issuerDid: $issuer, credentialIds: ["membership-vc20"]}')" |
    jq -r '.holderPid') || fail "$identity asks $issuer_did"
  for _ in $(seq 1 100); do
    view=$(curl -sf "http://$management/v1/credential-requests/$pid" -H "authorization: Bearer $key")
    [[ $(jq -r '.status' <<<"$view") == ISSUED ]] && return 0
    sleep 0.2
  done
  fail "$identity: no credential from $issuer_did: $view"
}
start_connector() { # name address management-address identity-name identity-address trusted-issuers [more VAR=value settings]
  local name=$1 address=$2 management=$3 identity=$4 identity_address=$5 trusted=$6
  shift 6 # the rest: the more VAR=value settings
  mkdir -p "$WORK/$name"
  openssl rand -hex 32 >"$WORK/$name/management.key"
  database_for "$name" "$WORK/$name/connector.db" || fail "the database of $name"
  env PORTITOR_PARTICIPANT_ID="$(did_web "$identity_address")" PORTITOR_BIND="$address" \
    PORTITOR_PUBLIC_URL="http://$address" "${DATABASE[@]}" \
    PORTITOR_STS_URL="http://$identity_address/sts/token" \
    PORTITOR_STS_CLIENT_SECRET_FILE="$WORK/$identity/sts-client-secret" \
    PORTITOR_TRUSTED_ISSUERS="$trusted" PORTITOR_ALLOW_HTTP=true \
    PORTITOR_STATUS_LIST_CACHE_SECONDS=0 PORTITOR_MANAGEMENT_BIND="$management" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$WORK/$name/management.key" \
    "$@" "$BIN/portitor-connector" >"$WORK/$name.log" 2>&1 &
  PIDS+=($!)
}
sts_token() { # identity-name identity-address audience-did
  local identity=$1 address=$2 audience=$3
  curl -sf -X POST "http://$address/sts/token" --data-urlencode grant_type=client_credentials \
    --data-urlencode "client_id=$(did_web "$address")" \
    --data-urlencode "client_secret=$(cat "$WORK/$identity/sts-client-secret")" \
    --data-urlencode "audience=$audience" \
    --data-urlencode "bearer_access_scope=org.eclipse.dspace.dcp.vc.type:MembershipCredential" |
    jq -r '.access_token'
}
catalog_status() { # consumer-identity-name consumer-identity-address provider-address provider-did
  local identity=$1 identity_address=$2 provider=$3 provider_did=$4
  local token
  token=$(sts_token "$identity" "$identity_address" "$provider_did") || fail "no STS token from $identity"
  curl -s -o "$WORK/catalog.json" -w '%{http_code}' -X POST "http://$provider/dsp/2025-1/catalog/request" \
    -H "authorization: Bearer $token" "${JSON[@]}" \
    -d '{"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"}'
}
api() { # connector-name management-address method path [body]
  local connector=$1 management=$2 method=$3 path=$4
  local args=(-sf -X "$method" "http://$management$path" -H "authorization: Bearer $(cat "$WORK/$connector/management.key")")
  if [[ $# -gt 4 ]]; then
    local body=$5
    args+=("${JSON[@]}" -d "$body")
  fi
  curl "${args[@]}"
}
until_state() { # connector-name management-address path states
  local connector=$1 management=$2 path=$3 states=$4
  local view state
  for _ in $(seq 1 100); do
    view=$(api "$connector" "$management" GET "$path") || fail "read $path"
    state=$(jq -r '.state' <<<"$view")
    if [[ $state =~ ^($states)$ ]]; then echo "$view"; return 0; fi
    sleep 0.2
  done
  fail "$path still $state"
}
# a whole exchange: catalog, negotiation, transfer, data pull
exchange() { # label consumer-connector consumer-management consumer-identity consumer-identity-address provider-address provider-did
  local label=$1 connector=$2 management=$3 identity=$4 identity_address=$5 provider=$6 provider_did=$7
  local status offer body pid view agreement endpoint token
  status=$(catalog_status "$identity" "$identity_address" "$provider" "$provider_did")
  [[ $status == 200 ]] || fail "$label: catalog answered $status"
  [[ $(jq -r '.participantId' "$WORK/catalog.json") == "$provider_did" ]] || fail "$label: other participant"
  offer=$(jq -c '.dataset[] | select(.["@id"] == "weather-stations") | .hasPolicy[0]' "$WORK/catalog.json")
  body=$(jq -nc --arg addr "http://$provider/dsp/2025-1" --arg id "$provider_did" --argjson offer "$offer" \
    '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: "weather-stations", offer: $offer}')
  pid=$(api "$connector" "$management" POST /v1/negotiations "$body" | jq -r '.consumerPid') || fail "$label: start"
  view=$(until_state "$connector" "$management" "/v1/negotiations/$pid" 'FINALIZED|TERMINATED')
  [[ $(jq -r '.state' <<<"$view") == FINALIZED ]] || fail "$label: negotiation $(jq -c . <<<"$view")"
  agreement=$(jq -r '.agreement["@id"]' <<<"$view")
  pid=$(api "$connector" "$management" POST /v1/transfers "$(jq -nc --arg a "$agreement" '{agreementId: $a}')" |
    jq -r '.consumerPid') || fail "$label: transfer"
  view=$(until_state "$connector" "$management" "/v1/transfers/$pid" 'STARTED|TERMINATED')
  [[ $(jq -r '.state' <<<"$view") == STARTED ]] || fail "$label: transfer $(jq -c . <<<"$view")"
  endpoint=$(jq -r '.dataAddress.endpoint' <<<"$view")
  token=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' <<<"$view")
  curl -sf "$endpoint" -H "authorization: Bearer $token" -o "$WORK/fetched" || fail "$label: fetch"
  cmp -s "$WORK/fetched" examples/data/weather-stations.csv || fail "$label: data differs"
  echo "$label: catalog 200, negotiation FINALIZED, transfer STARTED, data fetched from $endpoint"
}

step "Two data spaces: issuer A $DID_A, issuer B $DID_B; the data source on $DATA"
start_issuer issuer-a $ISSUER_A 127.0.0.1:19101
start_issuer issuer-b $ISSUER_B 127.0.0.1:19201
( cd examples/data && exec python3 -m http.server --bind 127.0.0.1 19070 ) >"$WORK/data.log" 2>&1 &
PIDS+=($!)
jq --arg data "http://$DATA/" 'map(if .["portitor:source"] then .["portitor:source"] |= sub("^http://[^/]+/"; $data) else . end)' \
  examples/datasets.json >"$WORK/datasets.json"
wait_for "http://$ISSUER_A/.well-known/did.json"; wait_for "http://$ISSUER_B/.well-known/did.json"

step "One consumer in each data space (identity and connector), each trusting only its data space's issuer"
start_identity consumer-a-id 127.0.0.1:19400 127.0.0.1:19401 "$DID_A"
start_identity consumer-b-id 127.0.0.1:19500 127.0.0.1:19501 "$DID_B"
wait_for http://127.0.0.1:19400/.well-known/did.json; wait_for http://127.0.0.1:19500/.well-known/did.json
register issuer-a 127.0.0.1:19101 "$(did_web 127.0.0.1:19400)"
register issuer-b 127.0.0.1:19201 "$(did_web 127.0.0.1:19500)"
request_membership consumer-a-id 127.0.0.1:19401 "$DID_A"
request_membership consumer-b-id 127.0.0.1:19501 "$DID_B"
start_connector consumer-a 127.0.0.1:19410 127.0.0.1:19411 consumer-a-id 127.0.0.1:19400 "$DID_A"
start_connector consumer-b 127.0.0.1:19510 127.0.0.1:19511 consumer-b-id 127.0.0.1:19500 "$DID_B"

if [[ $MODE == shared ]]; then
  step "The organisation: ONE identity that trusts both issuers and holds a membership credential of each"
  start_identity org-id 127.0.0.1:19300 127.0.0.1:19301 "$DID_A,$DID_B"
  wait_for http://127.0.0.1:19300/.well-known/did.json
  ORG_A=127.0.0.1:19300; ORG_B=127.0.0.1:19300; ID_A=org-id; ID_B=org-id
  register issuer-a 127.0.0.1:19101 "$(did_web $ORG_A)"
  register issuer-b 127.0.0.1:19201 "$(did_web $ORG_B)"
  request_membership org-id 127.0.0.1:19301 "$DID_A"
  request_membership org-id 127.0.0.1:19301 "$DID_B"
  curl -sf http://127.0.0.1:19301/v1/credentials -H "authorization: Bearer $(cat "$WORK/org-id/management.key")" |
    jq -c '[.[] | {types: (.types // .credentialType // .type), issuer}]'
else
  step "The organisation: one identity (and DID) per data space"
  start_identity org-id-a 127.0.0.1:19300 127.0.0.1:19301 "$DID_A"
  start_identity org-id-b 127.0.0.1:19350 127.0.0.1:19351 "$DID_B"
  wait_for http://127.0.0.1:19300/.well-known/did.json; wait_for http://127.0.0.1:19350/.well-known/did.json
  ORG_A=127.0.0.1:19300; ORG_B=127.0.0.1:19350; ID_A=org-id-a; ID_B=org-id-b
  register issuer-a 127.0.0.1:19101 "$(did_web $ORG_A)"
  register issuer-b 127.0.0.1:19201 "$(did_web $ORG_B)"
  request_membership org-id-a 127.0.0.1:19301 "$DID_A"
  request_membership org-id-b 127.0.0.1:19351 "$DID_B"
fi

step "The organisation's two connectors: one per data space, each with its own database, datasets file and trusted issuer"
start_connector org-a 127.0.0.1:19310 127.0.0.1:19311 "$ID_A" "$ORG_A" "$DID_A" \
  PORTITOR_DATASETS_FILE="$WORK/datasets.json"
start_connector org-b 127.0.0.1:19320 127.0.0.1:19321 "$ID_B" "$ORG_B" "$DID_B" \
  PORTITOR_DATASETS_FILE="$WORK/datasets.json"
for port in 19410 19510 19310 19320; do wait_for "http://127.0.0.1:$port/health"; done
for port in 19411 19511 19311 19321; do
  curl -sf "http://127.0.0.1:$port/api/check/readiness" >/dev/null || fail "connector on $port is not ready"
done
echo "all four connectors are ready"

step "Inside each data space: the whole exchange works"
exchange "data space A" consumer-a 127.0.0.1:19411 consumer-a-id 127.0.0.1:19400 127.0.0.1:19310 "$(did_web $ORG_A)"
exchange "data space B" consumer-b 127.0.0.1:19511 consumer-b-id 127.0.0.1:19500 127.0.0.1:19320 "$(did_web $ORG_B)"

step "Across data spaces: a member of A is refused by the connector of B, and the other way round"
STATUS=$(catalog_status consumer-a-id 127.0.0.1:19400 127.0.0.1:19320 "$(did_web $ORG_B)")
echo "consumer of A at the organisation's connector for B: HTTP $STATUS"
[[ $STATUS == 401 ]] || fail "expected 401"
STATUS=$(catalog_status consumer-b-id 127.0.0.1:19500 127.0.0.1:19310 "$(did_web $ORG_A)")
echo "consumer of B at the organisation's connector for A: HTTP $STATUS"
[[ $STATUS == 401 ]] || fail "expected 401"

if [[ $MODE == shared ]]; then
  step "The disclosure of a shared DID: what a verifier of data space A gets when it asks for MembershipCredential"
  # as a verifier does: a token of consumer A for the organisation, carrying the
  # organisation's access token, then the presentation query
  ORG_TOKEN=$(sts_token org-id 127.0.0.1:19300 "$(did_web 127.0.0.1:19400)")
  ACCESS=$(cut -d. -f2 <<<"$ORG_TOKEN" | tr '_-' '/+' | base64 -d 2>/dev/null | jq -r '.token' || true)
  VERIFIER_TOKEN=$(curl -sf -X POST http://127.0.0.1:19400/sts/token --data-urlencode grant_type=client_credentials \
    --data-urlencode "client_id=$(did_web 127.0.0.1:19400)" \
    --data-urlencode "client_secret=$(cat "$WORK/consumer-a-id/sts-client-secret")" \
    --data-urlencode "audience=$(did_web 127.0.0.1:19300)" --data-urlencode "token=$ACCESS" | jq -r '.access_token') ||
    fail "the STS of consumer A gave no token for the organisation's credential service"
  curl -s -X POST http://127.0.0.1:19300/cs/presentations/query -H "authorization: Bearer $VERIFIER_TOKEN" \
    "${JSON[@]}" -d '{"@context": ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"],
      "type": "PresentationQueryMessage", "scope": ["org.eclipse.dspace.dcp.vc.type:MembershipCredential"]}' \
    >"$WORK/presentation.json"
  python3 - "$WORK/presentation.json" <<'PY'
import base64, json, sys
def claims(jwt):
    part = jwt.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))
answer = json.load(open(sys.argv[1]))
presentations = answer.get("presentation")
if not presentations:
    sys.exit("no presentation in the answer: " + json.dumps(answer)[:300])
issuers = set()
for jwt in presentations:
    vp = claims(jwt)
    held = (vp.get("vp") or vp).get("verifiableCredential", [])
    for credential in held:
        if isinstance(credential, dict):
            credential = credential.get("id", "").split(",")[-1]
        vc = claims(credential)
        body = vc.get("vc") or vc
        issuers.add(body.get("issuer") or vc.get("iss"))
        print("presented:", body.get("type"), "issued by", body.get("issuer") or vc.get("iss"))
if len(issuers) != 2:
    sys.exit("expected the membership credentials of both issuers")
PY
  echo "the verifier of A sees that the organisation is also a member of B; its own check skips B's credential (the exchange above worked)"
fi
step "Done ($MODE)"
