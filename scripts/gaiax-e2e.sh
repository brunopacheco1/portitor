#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The Gaia-X end-to-end test (interop/gaiax/compose.yaml, docs/TESTING.md,
# ADRs 0021 and 0022): Portitor participants get a Gaia-X compliance
# credential from a local compliance service, and a provider offers a
# dataset only to holders of one. Exits non-zero at the first failed
# check, then removes everything.
#
#  1. A test CA made for this run, and from it: the TLS certificate of the
#     public names (*.gaiax.test), the compliance service's certificate
#     (RSA, PS256), the test notary's, and one for each Gaia-X participant's
#     identity key (PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE).
#  2. The test notary (scripts/gaiax_test_notary.py) signs each Gaia-X
#     participant's registration number credential (gx:VatID); each gets
#     a participant file (PORTITOR_GAIAX_PARTICIPANT_FILE).
#  3. Docker Compose up. The compliance service's DID and the participants'
#     DIDs resolve over HTTPS with the test CA; each Gaia-X identity's DID
#     document has alg ES256 and an x5u to its certificate chain.
#  4. The issuer registers the three participants; each identity asks it
#     for its MembershipCredential.
#  5. The consumer's identity runs its Gaia-X onboarding
#     (POST /v1/gaiax/compliance) and holds a
#     gx:ParticipantComplianceCredential signed by the compliance service.
#  6. The consumer negotiates the provider's offer, whose constraint is
#     ParticipantComplianceCredential.gx:labelLevel eq "SC": FINALIZED.
#  7. The outsider, a member without Gaia-X, negotiates the same offer:
#     TERMINATED.
#
# Test-only: no real Gaia-X trust anchor, the compliance service in test
# mode (see interop/gaiax/compose.yaml). It needs Internet access: the
# compliance service loads shapes and contexts from the Gaia-X Registry
# (registry.lab.gaia-x.eu), w3id.org and www.w3.org.
#
# Usage: scripts/gaiax-e2e.sh
# Environment:
#   PORTITOR_IMAGE, PORTITOR_IDENTITY_IMAGE, PORTITOR_ISSUER_IMAGE
#                Portitor images (compose.yaml)
#   LOGS=FILE    on failure, write every container's whole log there
#   KEEP=1       leave the containers running, and interop/gaiax/run/, at the end
#   CONTAINER_ENGINE=docker|podman
#                the container engine; without it, docker if it answers,
#                then podman (scripts/container-engine.sh)
# Needs Docker or Podman with Docker Compose (docs/TESTING.md, "Docker or
# Podman"), bash 4.4 or later, curl, jq, openssl, and python3 with the
# `cryptography` package.

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/require-bash.sh"

cd "$(dirname "$0")/.."
DIR=interop/gaiax
RUN=$DIR/run
# ENGINE_COMPOSE: `docker compose` or `podman compose`
# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")

# who is who
AUTHORITY=did:web:issuer%3A8070
PROVIDER_DID=did:web:provider.gaiax.test
CONSUMER_DID=did:web:consumer.gaiax.test
OUTSIDER_DID=did:web:outsider-identity%3A8090
# COMPLIANCE_DID, NOTARY_DID, GAIAX_CONTEXT and TERMS_AND_CONDITIONS, and the
# functions for the test CA, the test notary, the participants' files and
# the onboarding calls: shared with scripts/edc-interop.sh
# shellcheck source=scripts/gaiax-lib.sh
source scripts/gaiax-lib.sh
MEMBER_OF="Portitor Gaia-X test dataspace"
DATASET=gaiax-compliant-only # config/datasets.json
DSP=dsp/2025-1
PROVIDER_DSP_INSIDE=http://provider-connector:8080/$DSP

# how this script reaches them
ISSUER_MANAGEMENT=http://127.0.0.1:18471
declare -A IDENTITY_MANAGEMENT=(
  [provider]=http://127.0.0.1:18491
  [consumer]=http://127.0.0.1:18492
  [outsider]=http://127.0.0.1:18493
)
declare -A CONNECTOR_MANAGEMENT=(
  [provider]=http://127.0.0.1:18481
  [consumer]=http://127.0.0.1:18482
  [outsider]=http://127.0.0.1:18483
)
CONTENT_TYPE_JSON='content-type: application/json'

step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  exit 1
}

wait_for() { # url [what]: until it answers 2xx, at most 150 seconds
  local url=$1 what=${2:-$1}
  for _ in $(seq 1 150); do
    curl -sf -o /dev/null "$url" && return 0
    sleep 1
  done
  fail "$what did not answer"
}

fetch() { # url [curl flags]: a request from inside the network, trusting the test CA
  "${COMPOSE[@]}" run --rm -q fetch -sS --fail-with-body "$@"
}

# --- 1. the test CA and the certificates ------------------------------------------

step "Make a test CA and its certificates"
rm -rf "$RUN"
mkdir -p "$RUN/issuer"
# the TLS certificate covers every public name (config/tls.conf)
gaiax_test_ca provider.gaiax.test,consumer.gaiax.test,compliance.gaiax.test,notary.gaiax.test

cleanup() {
  status=$?
  if [[ $status -ne 0 ]]; then
    "${COMPOSE[@]}" logs --no-color --tail 60 >&2 || true
    if [[ -n ${LOGS:-} ]]; then "${COMPOSE[@]}" logs --no-color >"$LOGS" 2>&1 || true; fi
  fi
  if [[ ${KEEP:-} != 1 ]]; then
    "${COMPOSE[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$RUN"
  fi
  exit "$status"
}
trap cleanup EXIT

# --- 2. the participants' keys, credentials and participant files --------------

step "The test notary: its DID document"
gaiax_test_notary

step "The participants: keys, certificates, registration numbers, participant files"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$RUN/issuer/issuer-key.pem" 2>/dev/null
openssl rand -hex 32 >"$RUN/issuer/management.key"
for name in provider consumer outsider; do participant "$name"; done
gaiax_participant provider provider.gaiax.test "Portitor Test Provider S.A." LU12345678
gaiax_participant consumer consumer.gaiax.test "Portitor Test Consumer S.A." LU87654321
# the Portitor images run as user 65532 and must read them
chmod -R a+rX "$RUN"
find "$RUN" -type f -exec chmod a+r {} +

# --- 3. start, and resolve the DIDs ------------------------------------------------

step "Start the compliance service, the authority and the participants"
"${COMPOSE[@]}" up -d --quiet-pull
wait_for "$ISSUER_MANAGEMENT/health" "the issuer"
for name in provider consumer outsider; do
  wait_for "${IDENTITY_MANAGEMENT[$name]}/health" "the $name's identity"
  wait_for "${CONNECTOR_MANAGEMENT[$name]}/health" "the $name's connector"
done

step "The compliance service's DID resolves over HTTPS with the test CA"
fetch https://compliance.gaiax.test/.well-known/did.json |
  jq -e --arg did "$COMPLIANCE_DID" '.id == $did and (.verificationMethod[0].publicKeyJwk.x5u != null)' ||
  fail "the compliance service's DID document"

step "Each Gaia-X identity's DID document: alg ES256 and x5u, the chain served for its key"
for host in provider.gaiax.test consumer.gaiax.test; do
  name=${host%%.*}
  DOCUMENT=$(fetch "https://$host/.well-known/did.json") || fail "resolve did:web:$host"
  X5U=$(jq -r '.verificationMethod[0].publicKeyJwk | select(.alg == "ES256") | .x5u // empty' <<<"$DOCUMENT")
  [[ $X5U == "https://$host/.well-known/x509CertificateChain.pem" ]] ||
    fail "did:web:$host has no alg ES256 and x5u: $DOCUMENT"
  fetch "$X5U" >"$RUN/$name/served-chain.pem" || fail "fetch $X5U"
  cmp -s "$RUN/$name/served-chain.pem" "$RUN/$name/certificate-chain.pem" ||
    fail "$X5U is not the chain the identity was given"
  echo "did:web:$host: x5u $X5U"
done

# --- 4. the MembershipCredentials ---------------------------------------------------

issuer_api() { # method path [body]: the issuer's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$ISSUER_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/issuer/management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

identity_api() { # name method path [body]: a participant identity's management API
  local name=$1 method=$2 path=$3 body=${4-}
  local args=(-sS --fail-with-body -X "$method" "${IDENTITY_MANAGEMENT[$name]}$path"
    -H "authorization: Bearer $(cat "$RUN/$name/identity-management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

connector_api() { # name method path [body]: a participant connector's management API
  local name=$1 method=$2 path=$3 body=${4-}
  local args=(-sS --fail-with-body -X "$method" "${CONNECTOR_MANAGEMENT[$name]}$path"
    -H "authorization: Bearer $(cat "$RUN/$name/connector-management.key")")
  if [[ $# -gt 3 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

step "The issuer registers the three participants; each identity asks for its MembershipCredential"
for did in "$PROVIDER_DID" "$CONSUMER_DID" "$OUTSIDER_DID"; do
  issuer_api POST /v1/participants "$(jq -nc --arg did "$did" --arg m "$MEMBER_OF" \
    '{did: $did, claims: {memberOf: $m}}')" >/dev/null || fail "register $did"
done
for name in provider consumer outsider; do
  HOLDER_PID=$(identity_api "$name" POST /v1/credential-requests "$(jq -nc --arg issuer "$AUTHORITY" \
    '{issuerDid: $issuer, credentialIds: ["membership-vc20"]}')" | jq -r '.holderPid') ||
    fail "the $name's credential request"
  STATUS=
  for _ in $(seq 1 60); do
    STATUS=$(identity_api "$name" GET "/v1/credential-requests/$HOLDER_PID" | jq -r '.status') ||
      fail "read the $name's credential request"
    [[ $STATUS == ISSUED || $STATUS == REJECTED ]] && break
    sleep 1
  done
  [[ $STATUS == ISSUED ]] || fail "the $name's MembershipCredential: $STATUS"
  echo "$name: MembershipCredential issued"
done

# --- 5. Gaia-X onboarding -------------------------------------------------------------

has_compliance_credential() { # name: true once it holds one from the compliance service
  gaiax_has_compliance_credential "${IDENTITY_MANAGEMENT[$1]}" "$RUN/$1/identity-management.key"
}

start_compliance_run() { # name: POST /v1/gaiax/compliance; 409: a run is on already
  gaiax_start_compliance_run "${IDENTITY_MANAGEMENT[$1]}" "$RUN/$1/identity-management.key" "the $1"
}

until_compliance_run_ended() { # name: prints the newest run once it is no longer RUNNING
  gaiax_until_compliance_run_ended "${IDENTITY_MANAGEMENT[$1]}" "$RUN/$1/identity-management.key" "the $1"
}

step "The consumer's identity runs its Gaia-X onboarding (POST /v1/gaiax/compliance)"
# the identity starts a run on its own at start too; then the POST gets 409
start_compliance_run consumer
RUN_VIEW=$(until_compliance_run_ended consumer)
echo "$RUN_VIEW"
[[ $(jq -r '.outcome' <<<"$RUN_VIEW") == SUCCEEDED ]] || fail "the consumer's compliance run: $RUN_VIEW"
has_compliance_credential consumer ||
  fail "the consumer holds no gx:ParticipantComplianceCredential from $COMPLIANCE_DID"
identity_api consumer GET /v1/credentials |
  jq '[.[] | select(.types | index("gx:ParticipantComplianceCredential"))]'
# the provider gets its own as well; nothing below needs it
start_compliance_run provider

# --- 6. and 7. the offer for compliant participants -----------------------------------

step "Read the provider's catalog through the consumer's management API"
CATALOG=$(connector_api consumer POST /v1/catalog/request "$(jq -nc \
  --arg addr "$PROVIDER_DSP_INSIDE" --arg id "$PROVIDER_DID" \
  '{counterPartyAddress: $addr, counterPartyId: $id, protocol: "dataspace-protocol-http:2025-1"}')") ||
  fail "catalog request: $CATALOG"
OFFER=$(jq -c --arg id "$DATASET" '[.dataset] | flatten | .[] | select(.["@id"] == $id)
  | [.hasPolicy] | flatten | .[0]' <<<"$CATALOG")
[[ -n $OFFER && $OFFER != null ]] || fail "no $DATASET offer in the catalog: $CATALOG"
echo "the provider offers: $OFFER"

negotiate() { # name: the participant negotiates the offer; prints the ended negotiation
  local name=$1 pid view state=
  pid=$(connector_api "$name" POST /v1/negotiations "$(jq -nc --arg addr "$PROVIDER_DSP_INSIDE" \
    --arg id "$PROVIDER_DID" --arg dataset "$DATASET" --argjson offer "$OFFER" \
    '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: $dataset, offer: $offer}')" |
    jq -r '.consumerPid') || fail "the $name starts the negotiation"
  for _ in $(seq 1 120); do
    view=$(connector_api "$name" GET "/v1/negotiations/$pid") || fail "read the $name's negotiation"
    state=$(jq -r '.state' <<<"$view")
    if [[ $state == FINALIZED || $state == TERMINATED ]]; then
      echo "$view"
      return 0
    fi
    sleep 0.5
  done
  fail "the $name's negotiation is still $state"
}

step "The consumer, with its compliance credential, negotiates: the provider agrees"
VIEW=$(negotiate consumer)
jq '{consumerPid, providerPid, state, agreement}' <<<"$VIEW"
[[ $(jq -r '.state' <<<"$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
[[ $(jq -r '.agreement["@id"] // empty' <<<"$VIEW") != "" ]] || fail "no agreement: $VIEW"

step "The outsider, a member without Gaia-X, negotiates: the provider refuses"
VIEW=$(negotiate outsider)
jq '{consumerPid, providerPid, state, errorDetail}' <<<"$VIEW"
[[ $(jq -r '.state' <<<"$VIEW") == TERMINATED ]] || fail "expected TERMINATED: $VIEW"

step "Done: Gaia-X compliance credential issued to a Portitor identity, required by a Portitor provider, and its absence refused"
