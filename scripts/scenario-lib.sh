# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# What the scenarios' scripts share (scenarios/README.md): writing the
# keys, building the HealthDCAT-AP bundle, the authority's and the
# participants' management APIs, and the calls of a contract and a
# transfer. Sourced, not run, from the repository root.
#
# The script that sources it sets:
#   RUN                  the scenario's run/ folder; each party has its
#                        keys in RUN/<party>/ (write_participant_keys,
#                        write_issuer_keys)
#   COMPOSE              the Compose command with the scenario's file, as
#                        an array
#   MANAGEMENT           participant -> its connector's management API
#   IDENTITY_MANAGEMENT  participant -> its identity's management API
#   ISSUER_MANAGEMENT    issuer -> its management API
# shellcheck shell=bash disable=SC2034 # the variables are for the scripts that source it

DSP=dsp/2025-1
PROTOCOL=dataspace-protocol-http:2025-1

step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  exit 1
}

wait_for() { # url
  local url=$1
  for _ in $(seq 1 150); do
    curl -sf "$url" >/dev/null && return 0
    sleep 0.2
  done
  fail "$url did not answer"
}

uri() { # text: the text, percent-encoded for a URL path
  local text=$1
  jq -rn --arg v "$text" '$v | @uri'
}

# --- keys and the bundle ------------------------------------------------------

write_issuer_keys() { # issuer: its signing key and its management API key
  local dir=$RUN/$1
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/issuer-key.pem" 2>/dev/null
  openssl rand -hex 32 >"$dir/management.key"
}

write_participant_keys() { # participant: its identity key and the secrets of its identity and connector
  local dir=$RUN/$1 secret
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/identity-key.pem" 2>/dev/null
  for secret in identity-management.key sts-client-secret management.key; do
    openssl rand -hex 32 >"$dir/$secret"
  done
}

build_bundle() { # authority-did: the HealthDCAT-AP bundle into RUN/authority/bundle, signed with the issuer image
  # the Compose service "authority" runs the issuer image (scripts/demo-bundle.sh)
  cat >"$RUN/issuer-command" <<SCRIPT
#!/bin/sh
exec ${COMPOSE[*]} run --rm --no-deps -q --user "$(engine_bind_mount_user)" \\
  -v "$PWD:$PWD" -w "$PWD" authority "\$@"
SCRIPT
  chmod +x "$RUN/issuer-command"
  mkdir -p "$RUN/authority/bundle/bundles" # before the engine makes the mount point, as root
  AUTHORITY=$1 scripts/demo-bundle.sh "$RUN/authority/bundle" "$RUN/issuer-command" >/dev/null ||
    fail "build the bundle"
}

# --- the issuers and the identities --------------------------------------------

issuer_api() { # issuer method path [body]: that issuer's management API
  local issuer=$1 method=$2 path=$3 body=${4-}
  local args=(-sS --fail-with-body -X "$method" "${ISSUER_MANAGEMENT[$issuer]}$path"
    -H "authorization: Bearer $(cat "$RUN/$issuer/management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

identity_api() { # participant method path [body]: that participant's identity management API
  local who=$1 method=$2 path=$3 body=${4-}
  local args=(-sS --fail-with-body -X "$method" "${IDENTITY_MANAGEMENT[$who]}$path"
    -H "authorization: Bearer $(cat "$RUN/$who/identity-management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

participant_did() { echo "did:web:$1-identity%3A8090"; }

register() { # issuer participant claims-json: the issuer registers the participant with its claims
  local issuer=$1 participant=$2 claims=$3 did
  did=$(participant_did "$participant")
  issuer_api "$issuer" POST /v1/participants "$(jq -nc --arg did "$did" --argjson claims "$claims" \
    '{did: $did, claims: $claims}')" >/dev/null || fail "$issuer: register $did"
  echo "$issuer registered $participant: $claims"
}

request_credentials() { # participant issuer-did credential-id...: asks, then waits for the delivery
  local who=$1 issuer=$2
  shift 2
  local ids holder_pid view status
  ids=$(printf '%s\n' "$@" | jq -R . | jq -sc .)
  holder_pid=$(identity_api "$who" POST /v1/credential-requests \
    "$(jq -nc --arg issuer "$issuer" --argjson ids "$ids" '{issuerDid: $issuer, credentialIds: $ids}')" |
    jq -r '.holderPid') || fail "$who: credential request to $issuer"
  for _ in $(seq 1 100); do
    view=$(identity_api "$who" GET "/v1/credential-requests/$holder_pid") || fail "$who: read the credential request"
    status=$(jq -r '.status' <<<"$view")
    case $status in
      ISSUED) return 0 ;;
      REJECTED) fail "$issuer rejected $who's request: $view" ;;
      *) ;; # still asked: wait
    esac
    sleep 0.2
  done
  fail "$who's credentials did not arrive: $view"
}

# --- the connectors -------------------------------------------------------------

api() { # participant method path [body]: that participant's connector management API
  local who=$1 method=$2 path=$3 body=${4-}
  local args=(-sf -X "$method" "${MANAGEMENT[$who]}$path"
    -H "authorization: Bearer $(cat "$RUN/$who/management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

state_of() { # record: its state
  local record=$1
  jq -r '.state' <<<"$record"
}

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
  local who=$1 address=$2 provider=$3
  api "$who" POST /v1/catalog/request "$(jq -nc --arg addr "$address" --arg id "$provider" --arg p "$PROTOCOL" \
    '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')" || fail "$who: catalog of $provider"
}

offer() { # catalog dataset offer-id: the offer, as the catalog gives it
  local catalog=$1 dataset=$2 offer_id=$3
  jq -ce --arg d "$dataset" --arg o "$offer_id" '.dataset[] | select(.["@id"] == $d) | .hasPolicy[]
    | select(.["@id"] == $o)' <<<"$catalog" || fail "no offer $offer_id in the catalog"
}

negotiate() { # participant address provider-id dataset offer: prints the ended negotiation
  local who=$1 address=$2 provider=$3 dataset=$4 offer=$5 pid
  pid=$(api "$who" POST /v1/negotiations "$(jq -nc --arg addr "$address" --arg id "$provider" --arg d "$dataset" \
    --argjson offer "$offer" '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: $d, offer: $offer}')" |
    jq -r '.consumerPid') || fail "$who: start negotiation"
  until_state "$who" "/v1/negotiations/$pid" 'FINALIZED|TERMINATED'
}

transfer() { # participant agreement: prints the transfer once STARTED or TERMINATED
  local who=$1 agreement=$2 pid
  pid=$(api "$who" POST /v1/transfers "$(jq -nc --arg a "$agreement" '{agreementId: $a}')" |
    jq -r '.consumerPid') || fail "$who: request transfer"
  until_state "$who" "/v1/transfers/$pid" 'STARTED|TERMINATED'
}

complete() { # participant transfer
  local who=$1 transfer=$2 pid
  pid=$(jq -r '.consumerPid' <<<"$transfer")
  api "$who" POST "/v1/transfers/$pid/completion" >/dev/null || fail "$who: complete $pid"
  until_state "$who" "/v1/transfers/$pid" 'COMPLETED' >/dev/null
}

app_fetch() { # app transfer [output]: the data user's app (a Compose service) fetches the data with the token
  local app=$1 view=$2 output=${3:--} endpoint token
  endpoint=$(jq -r '.dataAddress.endpoint' <<<"$view")
  token=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' <<<"$view")
  "${COMPOSE[@]}" run --rm --no-deps -q --user "$(engine_bind_mount_user)" "$app" \
    wget -q -O "$output" --header "authorization: Bearer $token" "$endpoint"
}

expect() { # what record expected-state
  local what=$1 record=$2 expected=$3
  [[ $(state_of "$record") == "$expected" ]] ||
    fail "$what: expected $expected, got $(jq -c '{state, errorDetail}' <<<"$record")"
  echo "$what: $expected"
}
