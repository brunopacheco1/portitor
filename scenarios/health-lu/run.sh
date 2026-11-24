#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The Luxembourg-style health data space (README.md) in Docker Compose
# (compose.yaml): Digital Lux Trust, Lux Biobank, Lux Biomedicine Lab, the
# HDAB with its own issuer, and Schuman Clinical Research. Starts it, runs
# the scenarios that use it from this machine, then removes everything,
# volumes included.
#
#  1. Keys and secrets, in run/; the HealthDCAT-AP bundle, built and
#     signed with the issuer image (scripts/demo-bundle.sh) into
#     run/authority/bundle/; the HDAB's datasets with its pilot offer,
#     which ends PILOT_SECONDS from now.
#  2. Docker Compose up.
#  3. The authority registers the four participants with their claims;
#     each identity asks it for its credentials.
#  4. The scenarios, each one's steps.sh, in this order:
#     ../hdab-intermediary (the HDAB pulls and counts for a data user),
#     then ../data-permits (the HDAB's issuer grants data permits).
#
# Usage: scenarios/health-lu/run.sh [connector image [identity image [issuer image]]]
#   Without images it uses the compose file's defaults (the release images).
# Environment:
#   SCENARIOS="a b"   the scenarios to run, in order (default
#                     "hdab-intermediary data-permits"); each also runs
#                     alone, but data-permits ends the HDAB's membership,
#                     so it never runs before hdab-intermediary
#   KEEP=1            leave the containers running, and run/, at the end
#   PILOT_SECONDS=n   how long the HDAB's pilot offer lasts (default 150);
#                     hdab-intermediary waits for its end at step 6
#   CONTAINER_ENGINE=docker|podman
#                     the container engine (scripts/container-engine.sh)
# Needs Docker or Podman with Docker Compose (docs/TESTING.md, "Docker or
# Podman"), bash 4.4 or later, curl, jq and openssl.

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

cd "$(dirname "$0")/../.."
if [[ $# -gt 0 ]]; then export PORTITOR_IMAGE=$1; fi
if [[ $# -gt 1 ]]; then export PORTITOR_IDENTITY_IMAGE=$2; fi
if [[ $# -gt 2 ]]; then export PORTITOR_ISSUER_IMAGE=$3; fi
# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
DEMO=scenarios/health-lu
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DEMO/compose.yaml")
RUN=$DEMO/run
PILOT_SECONDS=${PILOT_SECONDS:-150}

# who is who (compose.yaml)
AUTHORITY=did:web:authority%3A8070
AUTHORITY_MANAGEMENT=http://127.0.0.1:18071
HDAB_ISSUER_MANAGEMENT=http://127.0.0.1:18125
MEMBER_OF="Lux health data space"
PARTICIPANTS=(biobank lab hdab schuman)
declare -A IDENTITY_PORT=([biobank]=18103 [lab]=18113 [hdab]=18123 [schuman]=18133)
declare -A CONNECTOR_PORT=([biobank]=18101 [lab]=18111 [hdab]=18121 [schuman]=18131)

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

# --- 1. keys, secrets, bundle, datasets ---------------------------------------

step "Write the keys and the secrets into $RUN"
# containers and volumes left by an earlier run with KEEP=1 still use the
# old keys: remove them first, so this run starts clean
"${COMPOSE[@]}" --profile apps down --volumes --remove-orphans >/dev/null 2>&1 || true
rm -rf "$RUN"
mkdir -p "$RUN/authority" "$RUN/hdab-issuer" "$RUN/pipeline" "$RUN/lab/storage"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$RUN/authority/authority-key.pem" 2>/dev/null
openssl rand -hex 32 >"$RUN/authority/management.key"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$RUN/hdab-issuer/issuer-key.pem" 2>/dev/null
openssl rand -hex 32 >"$RUN/hdab-issuer/management.key"
for participant in "${PARTICIPANTS[@]}"; do
  mkdir -p "$RUN/$participant"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 \
    -out "$RUN/$participant/identity-key.pem" 2>/dev/null
  for secret in identity-management.key sts-client-secret management.key; do
    openssl rand -hex 32 >"$RUN/$participant/$secret"
  done
done

cleanup() {
  status=$?
  if [[ $status -ne 0 ]]; then "${COMPOSE[@]}" logs --no-color --tail 30 >&2 || true; fi
  if [[ ${KEEP:-} != 1 ]]; then
    "${COMPOSE[@]}" --profile apps down --volumes --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$RUN"
  fi
  exit "$status"
}
trap cleanup EXIT

step "Build and sign the HealthDCAT-AP bundle with the issuer image"
# as scenarios/two-party/run.sh does
cat >"$RUN/issuer-command" <<SCRIPT
#!/bin/sh
exec ${COMPOSE[*]} run --rm --no-deps -q --user "$(engine_bind_mount_user)" \\
  -v "$PWD:$PWD" -w "$PWD" authority "\$@"
SCRIPT
chmod +x "$RUN/issuer-command"
mkdir -p "$RUN/authority/bundle/bundles" # before the engine makes the mount point, as root
AUTHORITY=$AUTHORITY scripts/demo-bundle.sh "$RUN/authority/bundle" "$RUN/issuer-command" >/dev/null ||
  fail "build the bundle"

step "The HDAB's datasets: its pilot offer ends in $PILOT_SECONDS seconds"
PILOT_END=$(date -u -d "@$(($(date +%s) + PILOT_SECONDS))" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -r "$(($(date +%s) + PILOT_SECONDS))" +%Y-%m-%dT%H:%M:%SZ) # GNU date, then BSD date
# the pilot: the same terms as the data users' offer, with an end soon
jq --arg end "$PILOT_END" '.[0].hasPolicy += [.[0].hasPolicy[0]
  | .["@id"] = "variant-carriers:offer:pilot"
  | .permission[0].constraint |= map(if .leftOperand == "dateTime" then .rightOperand = $end else . end)]' \
  "$DEMO/datasets/hdab.json" >"$RUN/hdab/datasets.json"
echo "pilot offer until $PILOT_END"
# the images run as user 65532 and must read them; the lab's app writes
# its folder
chmod -R a+rX "$RUN"
chmod a+w "$RUN/lab/storage"

# --- 2. start ---------------------------------------------------------------

step "Start the data space"
"${COMPOSE[@]}" up -d --quiet-pull
wait_for "$AUTHORITY_MANAGEMENT/health"
wait_for "$HDAB_ISSUER_MANAGEMENT/health"
for participant in "${PARTICIPANTS[@]}"; do
  wait_for "http://127.0.0.1:${IDENTITY_PORT[$participant]}/health"
  wait_for "http://127.0.0.1:${CONNECTOR_PORT[$participant]}/health"
done

# --- 3. the credentials -------------------------------------------------------

authority_api() { # method path [body]: the authority's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$AUTHORITY_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/authority/management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

register() { # participant claims-json
  local did="did:web:$1-identity%3A8090" claims=$2
  authority_api POST /v1/participants "$(jq -nc --arg did "$did" --argjson claims "$claims" \
    '{did: $did, claims: $claims}')" >/dev/null || fail "register $did"
  echo "$did: $claims"
}

request_credentials() { # participant credential-id...: asks, then waits for the delivery
  local name=$1
  shift
  local port=${IDENTITY_PORT[$name]} key ids holder_pid view status
  key=$(cat "$RUN/$name/identity-management.key")
  ids=$(printf '%s\n' "$@" | jq -R . | jq -sc .)
  holder_pid=$(curl -sS --fail-with-body -X POST "http://127.0.0.1:$port/v1/credential-requests" \
    -H "authorization: Bearer $key" -H 'content-type: application/json' \
    -d "$(jq -nc --arg issuer "$AUTHORITY" --argjson ids "$ids" '{issuerDid: $issuer, credentialIds: $ids}')" |
    jq -r '.holderPid') || fail "the $name's credential request"
  for _ in $(seq 1 100); do
    view=$(curl -sf "http://127.0.0.1:$port/v1/credential-requests/$holder_pid" \
      -H "authorization: Bearer $key") || fail "read the $name's credential request"
    status=$(jq -r '.status' <<<"$view")
    case $status in
      ISSUED) return 0 ;;
      REJECTED) fail "the authority rejected the $name's request: $view" ;;
    esac
    sleep 0.2
  done
  fail "the $name's credentials did not arrive: $view"
}

step "Digital Lux Trust registers the four participants with their claims"
member() { # role [processing]
  jq -nc --arg m "$MEMBER_OF" --arg role "$1" --arg p "${2-}" \
    '{memberOf: $m, role: $role} + (if $p == "" then {} else {processing: $p} end)'
}
register biobank "$(member data-holder)"
register lab "$(member data-user eu)"
register hdab "$(member health-data-access-body)"
register schuman "$(member data-user)"

step "Each identity asks Digital Lux Trust for its credentials"
request_credentials biobank membership-vc20
request_credentials lab membership-vc20 data-processor-vc20
request_credentials hdab membership-vc20
request_credentials schuman membership-vc20
echo "the lab holds a MembershipCredential and a DataProcessorCredential; the others a MembershipCredential"

# --- 4. the scenarios ---------------------------------------------------------

export DEMO RUN AUTHORITY_MANAGEMENT HDAB_ISSUER_MANAGEMENT PILOT_END
for scenario in ${SCENARIOS:-hdab-intermediary data-permits}; do
  [[ -x scenarios/$scenario/steps.sh ]] || fail "no scenario $scenario"
  step "Scenario $scenario (scenarios/$scenario/README.md)"
  "scenarios/$scenario/steps.sh"
done
