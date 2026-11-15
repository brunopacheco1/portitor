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
#     identity key (PORTITOR_IDENTITY_CERTIFICATE_CHAIN).
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
# Needs docker with the compose plugin, curl, jq, openssl, and python3
# with the `cryptography` package.

set -euo pipefail

cd "$(dirname "$0")/.."
DIR=interop/gaiax
RUN=$DIR/run
COMPOSE=(docker compose -f "$DIR/compose.yaml")

# who is who
AUTHORITY=did:web:issuer%3A8070
PROVIDER_DID=did:web:provider.gaiax.test
CONSUMER_DID=did:web:consumer.gaiax.test
OUTSIDER_DID=did:web:outsider-identity%3A8090
COMPLIANCE_DID=did:web:compliance.gaiax.test
NOTARY_DID=did:web:notary.gaiax.test
# the Gaia-X ontology the compliance service validates against
GAIAX_CONTEXT='https://w3id.org/gaia-x/development#'
# the Gaia-X terms and conditions hash gx-compliance v2.16.0 accepts
# (IssuersHaveTermsAndConditionsFilter)
TERMS_AND_CONDITIONS=4bd7554097444c960292b4726c2efa1373485e8a5565d94d41195214c5e0ceb3
MEMBER_OF="Portitor Gaia-X test dataspace"
DATASET=gaiax-compliant-only # config/datasets.json
DSP=dsp/2025-1
PROVIDER_DSP_INSIDE=http://provider-connector:8080/$DSP

# how this script reaches them
ISSUER_MANAGEMENT=http://127.0.0.1:18471
PROVIDER_DSP=http://127.0.0.1:18480
CONSUMER_IDENTITY=http://127.0.0.1:18490
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

CA=$RUN/ca

new_ec_key() { # file: a P-256 key
  local file=$1
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$file" 2>/dev/null
}

issue_certificate() { # key common-name dns-names(comma-separated) out: a leaf from the test CA
  local key=$1 name=$2 dns=$3 out=$4 extensions
  extensions=$(printf 'subjectAltName=%s\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth,clientAuth\n' \
    "DNS:${dns//,/,DNS:}")
  openssl req -new -key "$key" -subj "/CN=$name/O=Portitor Gaia-X test (test only)" 2>/dev/null |
    openssl x509 -req -CA "$CA/ca.pem" -CAkey "$RUN/ca-key.pem" -set_serial "0x$(openssl rand -hex 16)" \
      -days 30 -extfile <(echo "$extensions") -out "$out" 2>/dev/null
}

chain() { # leaf out: the leaf, then the test CA
  local leaf=$1 out=$2
  cat "$leaf" "$CA/ca.pem" >"$out"
}

step "Make a test CA and its certificates"
rm -rf "$RUN"
mkdir -p "$CA" "$RUN/tls" "$RUN/compliance" "$RUN/issuer" "$RUN/notary/www/.well-known" "$RUN/notary/www/credentials"
new_ec_key "$RUN/ca-key.pem"
openssl req -x509 -key "$RUN/ca-key.pem" -subj "/CN=Portitor Gaia-X test CA (test only)" -days 30 \
  -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
  -out "$CA/ca.pem" 2>/dev/null
# the TLS certificate of every public name (config/tls.conf)
new_ec_key "$RUN/tls/server-key.pem"
issue_certificate "$RUN/tls/server-key.pem" gaiax.test \
  provider.gaiax.test,consumer.gaiax.test,compliance.gaiax.test,notary.gaiax.test "$RUN/tls/leaf.pem"
chain "$RUN/tls/leaf.pem" "$RUN/tls/server.pem"
# the compliance service signs with PS256: an RSA key; its DID document
# serves the chain (x5u), and its answers are valid at most until it expires
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$RUN/compliance/private-key.pem" 2>/dev/null
issue_certificate "$RUN/compliance/private-key.pem" compliance.gaiax.test compliance.gaiax.test \
  "$RUN/compliance/leaf.pem"
chain "$RUN/compliance/leaf.pem" "$RUN/compliance/certificate-chain.pem"
# the test notary
new_ec_key "$RUN/notary/key.pem"
issue_certificate "$RUN/notary/key.pem" notary.gaiax.test notary.gaiax.test "$RUN/notary/leaf.pem"
chain "$RUN/notary/leaf.pem" "$RUN/notary/www/.well-known/x509CertificateChain.pem"

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
python3 scripts/gaiax_test_notary.py did "$RUN/notary/key.pem" "$NOTARY_DID" \
  https://notary.gaiax.test/.well-known/x509CertificateChain.pem "$RUN/notary/www/.well-known/did.json"

participant() { # name: keys and secrets of a participant
  local name=$1
  mkdir -p "$RUN/$name"
  new_ec_key "$RUN/$name/identity-key.pem"
  openssl rand -hex 32 >"$RUN/$name/identity-management.key"
  openssl rand -hex 32 >"$RUN/$name/connector-management.key"
  openssl rand -hex 32 >"$RUN/$name/sts-client-secret"
}

gaiax_participant() { # name host legal-name vat-id: what ADR 0021 needs
  local name=$1 host=$2 legal_name=$3 vat_id=$4
  local dir=$RUN/$name
  local credential_id=https://$host/credentials/gaiax/registration-number
  # the certificate for the identity key, then the CA
  issue_certificate "$dir/identity-key.pem" "$host" "$host" "$dir/identity-certificate.pem"
  chain "$dir/identity-certificate.pem" "$dir/certificate-chain.pem"
  # the registration number credential, from the notary
  python3 scripts/gaiax_test_notary.py vat "$RUN/notary/key.pem" "$NOTARY_DID" "$GAIAX_CONTEXT" \
    "$credential_id" "$vat_id" LU "$dir/registration-number.jwt"
  # the participant file (ADR 0021): Gaia-X terms only
  jq -n --arg context "$GAIAX_CONTEXT" --arg name "$legal_name" --arg terms "$TERMS_AND_CONDITIONS" '{
    context: $context,
    legalPerson: {
      name: $name,
      "gx:legalAddress": {type: "gx:Address", "gx:countryCode": "LU",
        "vcard:street-address": "1 Rue du Test", "vcard:locality": "Luxembourg",
        "vcard:postal-code": "1111"},
      "gx:headquartersAddress": {type: "gx:Address", "gx:countryCode": "LU",
        "vcard:street-address": "1 Rue du Test", "vcard:locality": "Luxembourg",
        "vcard:postal-code": "1111"}
    },
    termsAndConditions: $terms,
    registrationNumberCredential: "/config/registration-number.jwt"
  }' >"$dir/participant.json"
  echo "$name: $host, $legal_name, VAT $vat_id"
}

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
  local name=$1
  identity_api "$name" GET /v1/credentials | jq -e --arg issuer "$COMPLIANCE_DID" \
    'any(.[]; (.types | index("gx:ParticipantComplianceCredential")) and .issuer == $issuer)' \
    >/dev/null
}

start_compliance_run() { # name: POST /v1/gaiax/compliance; 409: a run is on already
  local name=$1 status
  status=$(curl -s -o /dev/null -w '%{http_code}' -X POST "${IDENTITY_MANAGEMENT[$name]}/v1/gaiax/compliance" \
    -H "authorization: Bearer $(cat "$RUN/$name/identity-management.key")")
  [[ $status == 202 || $status == 409 ]] || fail "the $name's compliance run: HTTP $status"
}

until_compliance_run_ended() { # name: prints the newest run once it is no longer RUNNING
  local name=$1 runs outcome=
  for _ in $(seq 1 150); do
    runs=$(identity_api "$name" GET /v1/gaiax/compliance) || fail "read the $name's compliance runs"
    outcome=$(jq -r '.[0].outcome // empty' <<<"$runs")
    if [[ -n $outcome && $outcome != RUNNING ]]; then
      jq -c '.[0]' <<<"$runs"
      return 0
    fi
    sleep 1
  done
  fail "the $name's compliance run is still ${outcome:-not started}"
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

step "Read the provider's catalog with the consumer's identity"
TOKEN=$(curl -sf -X POST "$CONSUMER_IDENTITY/sts/token" \
  --data-urlencode grant_type=client_credentials \
  --data-urlencode "client_id=$CONSUMER_DID" \
  --data-urlencode "client_secret=$(cat "$RUN/consumer/sts-client-secret")" \
  --data-urlencode "audience=$PROVIDER_DID" \
  --data-urlencode "bearer_access_scope=org.eclipse.dspace.dcp.vc.type:MembershipCredential" |
  jq -r '.access_token') || fail "the consumer's STS gave no token"
CATALOG=$(curl -sS --fail-with-body -X POST "$PROVIDER_DSP/$DSP/catalog/request" \
  -H "authorization: Bearer $TOKEN" -H "$CONTENT_TYPE_JSON" \
  -d '{"@context": ["https://w3id.org/dspace/2025/1/context.jsonld"], "@type": "CatalogRequestMessage"}') ||
  fail "catalog request: $CATALOG"
OFFER=$(jq -c --arg id "$DATASET" '[.dataset] | flatten | .[] | select(.["@id"] == $id)
  | [.hasPolicy] | flatten | .[0]' <<<"$CATALOG")
[[ -n $OFFER && $OFFER != null ]] || fail "no $DATASET offer in the catalog: $CATALOG"
echo "the provider offers: $OFFER"

negotiate() { # name: the participant negotiates the offer; prints the ended negotiation
  local name=$1 pid view state=
  pid=$(connector_api "$name" POST /v1/negotiations "$(jq -nc --arg addr "$PROVIDER_DSP_INSIDE" \
    --arg id "$PROVIDER_DID" --arg dataset "$DATASET" --argjson offer "$OFFER" \
    '{providerAddress: $addr, providerId: $id, datasetId: $dataset, offer: $offer}')" |
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
