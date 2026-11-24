# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# What the health demo's scenarios (steps.sh, steps-permits.sh) share:
# who is who, and the calls to the participants' management APIs and
# apps. Sourced, not run.
# shellcheck shell=bash disable=SC2034 # the variables are for the scripts that source it

# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DEMO/compose.yaml")
DSP=dsp/2025-1
PROTOCOL=dataspace-protocol-http:2025-1
declare -A MANAGEMENT=([biobank]=http://127.0.0.1:18101 [lab]=http://127.0.0.1:18111
  [hdab]=http://127.0.0.1:18121 [schuman]=http://127.0.0.1:18131)
BIOBANK=http://biobank:8080/$DSP
BIOBANK_ID=did:web:biobank-identity%3A8090
HDAB=http://hdab:8080/$DSP
HDAB_ID=did:web:hdab-identity%3A8090
GENOTYPES=https://biobank.health-lu.example/datasets/genotypes
CARRIERS=https://hdab.health-lu.example/datasets/variant-carriers

step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  exit 1
}

api() { # participant method path [body]: that participant's management API
  local who=$1 method=$2 path=$3 body=${4-}
  local args=(-sf -X "$method" "${MANAGEMENT[$who]}$path"
    -H "authorization: Bearer $(cat "$RUN/$who/management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

state_of() { jq -r '.state' <<<"$1"; }

until_state() { # participant path states(regex): prints the record once in one of them
  local who=$1 path=$2 states=$3 view state
  for _ in $(seq 1 100); do
    view=$(api "$who" GET "$path") || fail "read $path"
    state=$(state_of "$view")
    if [[ $state =~ ^($states)$ ]]; then
      echo "$view"
      return 0
    fi
    sleep 0.2
  done
  fail "$who: $path still $state"
}

catalog() { # participant address provider-id: the provider's catalog
  api "$1" POST /v1/catalog/request "$(jq -nc --arg addr "$2" --arg id "$3" --arg p "$PROTOCOL" \
    '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')" || fail "$1: catalog of $3"
}

offer() { # catalog dataset offer-id: the offer, as the catalog gives it
  jq -ce --arg d "$2" --arg o "$3" '.dataset[] | select(.["@id"] == $d) | .hasPolicy[]
    | select(.["@id"] == $o)' <<<"$1" || fail "no offer $3 in the catalog"
}

negotiate() { # participant address provider-id dataset offer: prints the ended negotiation
  local who=$1 pid
  pid=$(api "$who" POST /v1/negotiations "$(jq -nc --arg addr "$2" --arg id "$3" --arg d "$4" \
    --argjson offer "$5" '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: $d, offer: $offer}')" |
    jq -r '.consumerPid') || fail "$who: start negotiation"
  until_state "$who" "/v1/negotiations/$pid" 'FINALIZED|TERMINATED'
}

transfer() { # participant agreement: prints the transfer once STARTED or TERMINATED
  local who=$1 pid
  pid=$(api "$who" POST /v1/transfers "$(jq -nc --arg a "$2" '{agreementId: $a}')" |
    jq -r '.consumerPid') || fail "$who: request transfer"
  until_state "$who" "/v1/transfers/$pid" 'STARTED|TERMINATED'
}

complete() { # participant transfer
  local who=$1 pid
  pid=$(jq -r '.consumerPid' <<<"$2")
  api "$who" POST "/v1/transfers/$pid/completion" >/dev/null || fail "$who: complete $pid"
  until_state "$who" "/v1/transfers/$pid" 'COMPLETED' >/dev/null
}

app_fetch() { # app transfer [output]: the data user's app fetches the data with the token
  local app=$1 view=$2 output=${3:--} endpoint token
  endpoint=$(jq -r '.dataAddress.endpoint' <<<"$view")
  token=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' <<<"$view")
  "${COMPOSE[@]}" run --rm --no-deps -q --user "$(engine_bind_mount_user)" "$app" \
    wget -q -O "$output" --header "authorization: Bearer $token" "$endpoint"
}

expect() { # what record expected-state
  [[ $(state_of "$2") == "$3" ]] || fail "$1: expected $3, got $(jq -c '{state, errorDetail}' <<<"$2")"
  echo "$1: $3"
}

