#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The EDC interop test (interop/edc/compose.yaml, docs/TESTING.md): a
# Portitor participant and an EDC v1.0.0-RC8 participant, both members of
# a data space whose authority runs portitor-issuer, with a local Gaia-X
# compliance service (ADRs 0021 and 0022). Exits non-zero at the first
# failed check, then removes everything.
#
#  1. The keys and secrets (interop/edc/run/), a test CA and its
#     certificates (scripts/gaiax-lib.sh), each participant's Gaia-X files,
#     the key set of the tokens for the IdentityHub's Identity API, and the
#     HealthDCAT-AP demo bundle (scripts/demo-bundle.sh, with the issuer
#     image), which the issuer serves and the Portitor connector pins.
#  2. Docker Compose up; the EDC IdentityHub gets a participant with the
#     key made in step 1, the EDC control plane an asset and an offer.
#  3. The issuer registers both participants. Each identity asks the
#     issuer for its MembershipCredential through its own API (DCP 6.4),
#     and the issuer delivers it to its Storage API (DCP 6.5): the EDC
#     IdentityHub through its Identity API (.../credentials/request),
#     Portitor's identity through its management API
#     (/v1/credential-requests).
#  4. The EDC provider's catalog, read through the Portitor connector's
#     management API (POST /v1/catalog/request): EDC checks Portitor's
#     ID token and its credential (DCP presentation query).
#  5. Portitor consumer, EDC provider: negotiation, PULL transfer, fetch.
#  6. EDC consumer, Portitor provider: catalog (HealthDCAT-AP datasets
#     checked against the bundle, with dct:conformsTo and full-IRI
#     metadata), negotiation, PULL transfer, fetch through the EDC data
#     plane.
#  7. Gaia-X compliance:
#     a. Portitor's identity did its Gaia-X onboarding and holds a
#        gx:ParticipantComplianceCredential from the compliance service.
#     b. The Portitor provider's second dataset is offered only with
#        ParticipantComplianceCredential.gx:labelLevel eq "SC". The EDC
#        consumer, without a compliance credential, negotiates it:
#        TERMINATED.
#     c. The EDC participant gets its own compliance credential (signed
#        with its key by scripts/gaiax_test_onboarding.py, as the EDC
#        IdentityHub has no Gaia-X onboarding) and stores it in the
#        IdentityHub (Identity API). It negotiates the same offer again:
#        FINALIZED; PULL transfer, fetch.
#  8. Revocation: once the issuer revokes a participant's credential
#     (POST /v1/credentials/{id}/revoke), the other refuses it (both ways).
#
# Test-only: no real Gaia-X trust anchor, the compliance service in test
# mode (see interop/gaiax/compose.yaml). It needs Internet access: the
# compliance service loads shapes and contexts from the Gaia-X Registry
# (registry.lab.gaia-x.eu), w3id.org and www.w3.org.
#
# Usage: scripts/edc-interop.sh [--no-build]
#   --no-build  use the EDC images already built (compose up --no-build)
# Environment:
#   PORTITOR_IMAGE, PORTITOR_IDENTITY_IMAGE, PORTITOR_ISSUER_IMAGE
#                Portitor images (compose.yaml)
#   LOGS=FILE    on failure, write every container's whole log there
#   KEEP=1       leave the containers running, and interop/edc/run/, at the end
#                (docs/TESTING.md, "The EDC interop run by hand")
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
DIR=interop/edc
RUN=$DIR/run
# ENGINE_COMPOSE: `docker compose` or `podman compose`
# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")
UP_FLAGS=(--build)
if [[ ${1:-} == --no-build ]]; then UP_FLAGS=(--no-build); fi

# who is who
AUTHORITY=did:web:issuer%3A8070
CREDENTIAL_ID=membership-vc20 # config/issuer-credentials.json
# the public names (*.gaiax.test) are answered by the "tls" service of
# compose.yaml: the Gaia-X compliance service resolves did:web over HTTPS only
PORTITOR_DID=did:web:portitor.gaiax.test
# config/portitor-datasets.json, and the profile scripts/demo-bundle.sh signs
PORTITOR_DATASET=https://portitor.example/datasets/portitor-interop
# the dataset for holders of a Gaia-X compliance credential (ADR 0022)
GAIAX_DATASET=https://portitor.example/datasets/portitor-interop-gaiax
GAIAX_OPERAND=ParticipantComplianceCredential.gx:labelLevel
PROFILE=https://data.health.europa.eu/healthdcat-ap/releases/release-8/
EDC_HOST=edc.gaiax.test
EDC_DID=did:web:$EDC_HOST
EDC_CONTEXT=edc
# the EDC participant's key: its id in the DID document, and the name of
# the private key in the Vault
EDC_KEY_ID="$EDC_DID#key-1"
EDC_KEY_ALIAS=edc-participant-key
# where the EDC participant's Gaia-X credentials have their ids (served
# by the "tls" service from run/edc/www)
EDC_GAIAX_BASE=https://$EDC_HOST/credentials/gaiax
# COMPLIANCE_DID, NOTARY_DID, GAIAX_CONTEXT and TERMS_AND_CONDITIONS, and the
# functions for the test CA, the test notary, a participant's Gaia-X files
# and a Portitor identity's onboarding: shared with scripts/gaiax-e2e.sh
# shellcheck source=scripts/gaiax-lib.sh
source scripts/gaiax-lib.sh
DSP=dsp/2025-1
EDC_PROTOCOL=dataspace-protocol-http:2025-1
# the header of a request with a JSON body, and the jq path to a record's state
CONTENT_TYPE_JSON='content-type: application/json'
STATE_PATH='.state'

# how this script reaches them
ISSUER_MANAGEMENT=http://127.0.0.1:18371
IDENTITY=http://127.0.0.1:18190
IDENTITY_MANAGEMENT=http://127.0.0.1:18191
CONNECTOR=http://127.0.0.1:18080
CONNECTOR_MANAGEMENT=http://127.0.0.1:18081
EDC_IDENTITY_API=http://127.0.0.1:18281/api/identity/v1
EDC_MANAGEMENT=http://127.0.0.1:18181/api/management
EDC_MANAGEMENT_KEY=edc-interop-test-key # config/edc-controlplane.properties
EDC_DATAPLANE=http://127.0.0.1:18180/api
# and how they reach each other
EDC_DSP_INSIDE=http://edc-controlplane:8082/api/dsp/2025-1
PORTITOR_DSP_INSIDE=http://portitor-connector:8080/$DSP

step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  exit 1
}

wait_for() { # url [what]
  local url=$1 what=${2:-$1}
  for _ in $(seq 1 150); do
    curl -sf -o /dev/null "$url" && return 0
    sleep 1
  done
  fail "$what did not answer"
}

wait_up() { # url what: until it answers HTTP at all (EDC answers 404 on /)
  local url=$1 what=$2
  for _ in $(seq 1 150); do
    [[ $(curl -s -o /dev/null -w '%{http_code}' "$url") != 000 ]] && return 0
    sleep 1
  done
  fail "$what did not answer"
}

# --- 1. keys and secrets ----------------------------------------------------

# The IdentityHub's Identity API (its "oauth2" variant) takes JWTs checked
# against a key set (config/edc-identityhub.properties). This test signs
# them itself with a test-only RSA key made for each run (RS256: openssl
# writes that signature as it is), and serves the key set from
# identity-api-jwks (compose.yaml).
IDENTITY_API_KEY=$RUN/identity-api-key.pem
IDENTITY_API_KID=edc-interop-admin-key

base64url() { # stdin -> base64url without padding
  openssl base64 -A | tr '+/' '-_' | tr -d '='
}

hex_to_bytes() { # hex text on stdin -> bytes on stdout
  local hex
  hex=$(tr -d '\n')
  # shellcheck disable=SC2059 # the hex digits become \x escapes on purpose
  printf "$(sed 's/../\\x&/g' <<<"$hex")"
}

identity_api_key_set() { # writes the key and its JWKS
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$IDENTITY_API_KEY" 2>/dev/null
  local modulus
  modulus=$(openssl rsa -in "$IDENTITY_API_KEY" -noout -modulus | cut -d= -f2 | hex_to_bytes | base64url)
  mkdir -p "$RUN/identity-api"
  jq -n --arg n "$modulus" --arg kid "$IDENTITY_API_KID" \
    '{keys: [{kty: "RSA", n: $n, e: "AQAB", kid: $kid, alg: "RS256", use: "sig"}]}' \
    >"$RUN/identity-api/jwks.json"
}

identity_api_token() { # scope: a five-minute token for the Identity API
  local scope=$1 now header claims input signature
  now=$(date +%s)
  header=$(jq -nc --arg kid "$IDENTITY_API_KID" '{alg: "RS256", typ: "JWT", kid: $kid}')
  claims=$(jq -nc --arg scope "$scope" --argjson now "$now" --arg jti "$(openssl rand -hex 16)" \
    '{iss: "edc-interop-admin", sub: "admin", scope: $scope, jti: $jti,
      iat: $now, nbf: $now, exp: ($now + 300)}')
  input="$(printf '%s' "$header" | base64url).$(printf '%s' "$claims" | base64url)"
  signature=$(printf '%s' "$input" | openssl dgst -sha256 -sign "$IDENTITY_API_KEY" | base64url)
  echo "$input.$signature"
}

step "Write the keys, the secrets and the Identity API's key set"
rm -rf "$RUN"
mkdir -p "$RUN/issuer" "$RUN/edc/www/.well-known" "$RUN/edc/www/credentials/gaiax"
identity_api_key_set
new_ec_key "$RUN/issuer/issuer-key.pem"
openssl rand -hex 32 >"$RUN/issuer/management.key"
participant portitor # run/portitor/: the identity key, the API keys, the STS secret
# The EDC participant's key is made here, not by the IdentityHub: its
# Gaia-X onboarding (step 7c) signs with it, and its DID document must
# name the key's certificate chain (x5u, ADR 0021)
new_ec_key "$RUN/edc/identity-key.pem"

step "Gaia-X: a test CA, its certificates, the test notary and the participants' files"
# the TLS certificate covers every public name (config/tls.conf)
gaiax_test_ca "portitor.gaiax.test,$EDC_HOST,compliance.gaiax.test,notary.gaiax.test"
gaiax_test_notary
gaiax_participant portitor portitor.gaiax.test "Portitor Test Provider S.A." LU12345678
gaiax_participant edc "$EDC_HOST" "EDC Test Consumer S.A." LU87654321
cp "$RUN/edc/certificate-chain.pem" "$RUN/edc/www/.well-known/x509CertificateChain.pem"
# the Portitor images run as user 65532 and must read them
chmod -R a+rX "$RUN"
find "$RUN" -type f -exec chmod a+r {} +

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

step "Build and sign the HealthDCAT-AP demo bundle with the issuer image"
# `portitor-issuer bundle build`, run in the issuer's image with this
# folder mounted at the same path
cat >"$RUN/issuer-command" <<SCRIPT
#!/bin/sh
exec ${COMPOSE[*]} run --rm --no-deps -q --user "$(engine_bind_mount_user)" \\
  -v "$PWD:$PWD" -w "$PWD" issuer "\$@"
SCRIPT
chmod +x "$RUN/issuer-command"
mkdir -p "$RUN/issuer/bundle/bundles" # before the engine makes the mount point, as root
AUTHORITY=$AUTHORITY scripts/demo-bundle.sh "$RUN/issuer/bundle" "$RUN/issuer-command" >/dev/null ||
  fail "build the demo bundle"
chmod -R a+rX "$RUN/issuer/bundle"

# --- 2. start, and set up the EDC participant ------------------------------

step "Start the data space"
"${COMPOSE[@]}" up -d --quiet-pull "${UP_FLAGS[@]}"
wait_for "$ISSUER_MANAGEMENT/health" "Portitor issuer"
wait_for "$IDENTITY/health" "Portitor identity"
wait_for "$CONNECTOR/health" "Portitor connector"
wait_up http://127.0.0.1:18281/ "EDC IdentityHub"
wait_up http://127.0.0.1:18181/ "EDC control plane"
wait_up "$EDC_DATAPLANE/" "EDC data plane"

fetch() { # url [curl flags]: a request from inside the network, trusting the test CA
  "${COMPOSE[@]}" run --rm -q fetch -sS --fail-with-body "$@"
}

edc_identity() { # method path [body]: the IdentityHub's Identity API, as admin
  local method=$1 path=$2 body=${3-}
  local token args
  token=$(identity_api_token identity-api:admin)
  args=(-sS --fail-with-body -X "$method" "$EDC_IDENTITY_API$path" -H "authorization: Bearer $token")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

edc_management() { # method path [body]: the control plane's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$EDC_MANAGEMENT$path" -H "x-api-key: $EDC_MANAGEMENT_KEY")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

wait_identity_api() { # until the Identity API answers 2xx, at most 120 seconds
  # wait_up only shows that the IdentityHub's web server answers; until the
  # Identity API is registered, a request to it gets 405
  local deadline=$((SECONDS + 120))
  until edc_identity GET /participants >/dev/null 2>&1; do
    ((SECONDS < deadline)) || fail "the EDC IdentityHub's Identity API did not answer within 120 s"
    sleep 1
  done
}

step "EDC IdentityHub: wait for its Identity API"
wait_identity_api

step "EDC IdentityHub: the participant $EDC_DID, with the key made for this run"
# "Keys already exist" (IdentityHub, KeyDescriptor): the private key is in
# the Vault under its alias before the participant is created, as a JWK,
# the form in which the IdentityHub stores a key it makes itself; the
# public key is given as a JWK. The Vault is EDC's HashiCorp Vault in dev
# mode: secrets are at /v1/secret/data/<alias>, as {"data": {"content": ...}}
# (HashicorpVaultClient).
EDC_PRIVATE_JWK=$(python3 scripts/gaiax_test_onboarding.py private-jwk \
  "$RUN/edc/identity-key.pem" "$EDC_KEY_ID") || fail "the EDC participant's private JWK"
stored=
for _ in $(seq 1 60); do
  if fetch -X POST "http://vault:8200/v1/secret/data/$EDC_KEY_ALIAS" \
    -H 'X-Vault-Token: root' -H "$CONTENT_TYPE_JSON" \
    -d "$(jq -nc --arg content "$EDC_PRIVATE_JWK" '{data: {content: $content}}')" \
    >/dev/null 2>&1; then
    stored=yes
    break
  fi
  sleep 1
done
[[ -n $stored ]] || fail "store the EDC participant's key in the Vault"
# the public JWK carries alg ES256 and x5u, the URL of the key's
# certificate chain: a Gaia-X compliance service needs both in the
# signer's DID document (ADR 0021)
EDC_PUBLIC_JWK=$(python3 scripts/gaiax_test_onboarding.py public-jwk \
  "$RUN/edc/identity-key.pem" "https://$EDC_HOST/.well-known/x509CertificateChain.pem") ||
  fail "the EDC participant's public JWK"
# the IdentityHub makes the STS account, and keeps the STS secret in the
# Vault as "<context>-sts-client-secret", where the control plane reads it
# (config/edc-controlplane.properties)
edc_identity POST /participants "$(jq -nc --arg did "$EDC_DID" --arg ctx "$EDC_CONTEXT" \
  --arg kid "$EDC_KEY_ID" --arg alias "$EDC_KEY_ALIAS" --argjson jwk "$EDC_PUBLIC_JWK" '{
  participantContextId: $ctx, did: $did, active: true, roles: [],
  keys: [{keyId: $kid, privateKeyAlias: $alias, publicKeyJwk: $jwk}],
  serviceEndpoints: [
    {type: "CredentialService", id: "edc-credentialservice",
     serviceEndpoint: "http://edc-identityhub:7082/api/credentials/v1/participants/\($ctx)"},
    {type: "ProtocolEndpoint", id: "edc-dsp",
     serviceEndpoint: "http://edc-controlplane:8082/api/dsp/2025-1"}]}')" >/dev/null ||
  fail "create the EDC participant"
# the DID resolves under its public name over HTTPS (as the compliance
# service reads it), and its key has alg ES256 and x5u
EDC_DOCUMENT=$(fetch "https://$EDC_HOST/.well-known/did.json") || fail "the EDC DID does not resolve"
jq -e --arg did "$EDC_DID" --arg kid "$EDC_KEY_ID" \
  --arg x5u "https://$EDC_HOST/.well-known/x509CertificateChain.pem" '.id == $did
  and any(.verificationMethod[]; .id == $kid and .publicKeyJwk.alg == "ES256" and .publicKeyJwk.x5u == $x5u)
  and (.assertionMethod | index($kid) != null)' <<<"$EDC_DOCUMENT" >/dev/null ||
  fail "the EDC DID document is not $EDC_DID with the key $EDC_KEY_ID, alg ES256 and x5u: $EDC_DOCUMENT"
fetch "https://$EDC_HOST/.well-known/x509CertificateChain.pem" >"$RUN/edc/served-chain.pem" ||
  fail "fetch the EDC participant's certificate chain"
cmp -s "$RUN/edc/served-chain.pem" "$RUN/edc/certificate-chain.pem" ||
  fail "the EDC participant's x5u does not serve its certificate chain"

step "EDC control plane: the data plane, an asset and an offer"
# EDC 1.0 data planes speak the Data Plane Signaling API; each side
# registers the other (no authorization: test-only, see runtimes/; the
# test data plane names every caller "controlplane")
edc_management PUT /v4/dataplanes '{
  "dataplaneId": "edc-dataplane", "endpoint": "http://edc-dataplane:8080/api/v1/dataflows",
  "profiles": ["HttpData-PULL"], "labels": [], "authorization": {"type": "none"}}' \
  >/dev/null || fail "register the data plane"
curl -sS --fail-with-body -X PUT "$EDC_DATAPLANE/v1/controlplanes" \
  -H "$CONTENT_TYPE_JSON" -d '{
  "controlplaneId": "controlplane", "endpoint": "http://edc-controlplane:8084/api/signaling",
  "authorization": {"type": "none"}}' >/dev/null || fail "register the control plane"
# the management API v4 context (EDC_CONNECTOR_MANAGEMENT_CONTEXT_V2)
# (the asset has no data address: the test data plane serves one file,
# interop.dataplane.source.url in config/edc-dataplane.properties)
CONTEXT='["https://w3id.org/edc/connector/management/v2"]'
edc_management POST /v4/assets "$(jq -nc --argjson c "$CONTEXT" '{
  "@context": $c, "@type": "Asset", "@id": "edc-interop",
  properties: {name: "EDC interop test data"}}')" >/dev/null ||
  fail "create the asset"
edc_management POST /v4/policydefinitions "$(jq -nc --argjson c "$CONTEXT" '{
  "@context": $c, "@type": "PolicyDefinition", "@id": "use",
  policy: {"@type": "Set", permission: [{action: "use"}]}}')" >/dev/null ||
  fail "create the policy"
edc_management POST /v4/contractdefinitions "$(jq -nc --argjson c "$CONTEXT" '{
  "@context": $c, "@type": "ContractDefinition", "@id": "edc-interop-offer",
  accessPolicyId: "use", contractPolicyId: "use", assetsSelector: []}')" >/dev/null ||
  fail "create the contract definition"

# --- 3. the credentials, from the issuer -------------------------------------

issuer_api() { # method path [body]: the issuer's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$ISSUER_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/issuer/management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

step "Issuer: register both participants"
for did in "$PORTITOR_DID" "$EDC_DID"; do
  issuer_api POST /v1/participants "$(jq -nc --arg did "$did" \
    '{did: $did, claims: {memberOf: "Portitor interop dataspace"}}')" >/dev/null ||
    fail "register $did"
done

until_issued() { # what read-function: until its .status is ISSUED, at most 60 seconds
  local what=$1 read=$2
  local view status=
  for _ in $(seq 1 60); do
    view=$("$read") || fail "read the $what"
    status=$(jq -r '.status' <<<"$view")
    case $status in
      ISSUED) return 0 ;;
      REJECTED | ERROR | TERMINATED) fail "the $what ended $status: $view" ;;
    esac
    sleep 1
  done
  fail "the $what is still $status: $view"
}

step "EDC IdentityHub: asks the issuer for its MembershipCredential (Identity API, credentials/request)"
# the IdentityHub sends the DCP CredentialRequestMessage with a token from
# its STS, and accepts the issuer's delivery for that request only. Its
# answer, 201, names the request in its Location header.
LOCATION=$(curl -sS --fail-with-body -X POST -D - -o /dev/null \
  "$EDC_IDENTITY_API/participants/$EDC_CONTEXT/credentials/request" \
  -H "authorization: Bearer $(identity_api_token identity-api:admin)" -H "$CONTENT_TYPE_JSON" \
  -d "$(jq -nc --arg issuer "$AUTHORITY" --arg id "$CREDENTIAL_ID" '{issuerDid: $issuer,
    credentials: [{id: $id, type: "MembershipCredential", format: "vc20-bssl/jwt"}]}')" |
  tr -d '\r' | awk 'tolower($1) == "location:" {print $2}') || fail "the EDC credential request"
EDC_HOLDER_PID=${LOCATION##*/}
[[ -n $EDC_HOLDER_PID ]] || fail "the IdentityHub gave no request id"
edc_request() { edc_identity GET "/participants/$EDC_CONTEXT/credentials/request/$EDC_HOLDER_PID"; }
until_issued "EDC credential request $EDC_HOLDER_PID" edc_request
edc_identity GET "/participants/$EDC_CONTEXT/credentials?type=MembershipCredential" |
  jq -e --arg issuer "$AUTHORITY" 'length == 1 and .[0].issuerId == $issuer' >/dev/null ||
  fail "the IdentityHub does not hold the issuer's MembershipCredential"

step "Portitor identity: asks the issuer for its MembershipCredential (management API)"
identity_management() { # method path [body]: the Portitor identity's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$IDENTITY_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/portitor/identity-management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}
PORTITOR_HOLDER_PID=$(identity_management POST /v1/credential-requests "$(jq -nc \
  --arg issuer "$AUTHORITY" --arg id "$CREDENTIAL_ID" '{issuerDid: $issuer, credentialIds: [$id]}')" |
  jq -r '.holderPid') || fail "Portitor's credential request"
portitor_request() { identity_management GET "/v1/credential-requests/$PORTITOR_HOLDER_PID"; }
until_issued "Portitor credential request $PORTITOR_HOLDER_PID" portitor_request
# (it also holds its Gaia-X credentials, once its onboarding ended: step 7a)
identity_management GET /v1/credentials |
  jq -e '[.[] | select(.types | index("MembershipCredential"))] | length == 1' >/dev/null ||
  fail "the identity does not hold one MembershipCredential"

# --- 4. EDC checks Portitor's identity --------------------------------------

# Asks the Portitor connector's management API for the EDC provider's
# catalog. Prints the HTTP status of that call: 200 with the catalog, or
# 502 with a problem whose detail has EDC's own status. The answer is in
# $RUN/catalog.json.
edc_catalog_for_portitor() {
  curl -sS -o "$RUN/catalog.json" -w '%{http_code}' \
    -X POST "$CONNECTOR_MANAGEMENT/v1/catalog/request" \
    -H "authorization: Bearer $(cat "$RUN/portitor/connector-management.key")" \
    -H "$CONTENT_TYPE_JSON" \
    -d "$(jq -nc --arg addr "$EDC_DSP_INSIDE" --arg id "$EDC_DID" --arg p "$EDC_PROTOCOL" \
      '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')"
}

step "The EDC provider's catalog, read through Portitor's management API"
# the Portitor connector gets a token from its STS, as before each DSP
# request it sends
STATUS=$(edc_catalog_for_portitor)
[[ $STATUS == 200 ]] || fail "the catalog call answered $STATUS: $(cat "$RUN/catalog.json")"
EDC_OFFER=$(jq -c '[.dataset] | flatten | .[] | select(.["@id"] == "edc-interop")
  | [.hasPolicy] | flatten | .[0]' "$RUN/catalog.json")
[[ -n $EDC_OFFER && $EDC_OFFER != null ]] || fail "no edc-interop offer: $(cat "$RUN/catalog.json")"
echo "EDC offers: $EDC_OFFER"

# --- 5. Portitor consumer, EDC provider --------------------------------------

portitor() { # method path [body]: the Portitor connector's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$CONNECTOR_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/portitor/connector-management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H "$CONTENT_TYPE_JSON" -d "$body"); fi
  curl "${args[@]}"
}

until_state() { # api path states (regex): prints the record once in one of them
  local api=$1 path=$2 states=$3
  local view state=
  for _ in $(seq 1 120); do
    view=$("$api" GET "$path") || fail "read $path"
    state=$(jq -r "$STATE_PATH" <<<"$view")
    if [[ $state =~ ^($states)$ ]]; then
      echo "$view"
      return 0
    fi
    sleep 0.5
  done
  fail "$path still $state"
}

step "Portitor negotiates the EDC offer"
PID=$(portitor POST /v1/negotiations "$(jq -nc --arg addr "$EDC_DSP_INSIDE" \
  --arg id "$EDC_DID" --argjson offer "$EDC_OFFER" \
  '{providerAddress: $addr, providerId: $id, datasetId: "edc-interop", offer: $offer}')" |
  jq -r '.consumerPid') || fail "start the negotiation"
VIEW=$(until_state portitor "/v1/negotiations/$PID" 'FINALIZED|TERMINATED')
[[ $(jq -r "$STATE_PATH" <<<"$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")

step "Portitor pulls the data from the EDC data plane"
TPID=$(portitor POST /v1/transfers "$(jq -nc --arg a "$AGREEMENT" \
  '{agreementId: $a, format: "HttpData-PULL"}')" | jq -r '.consumerPid') ||
  fail "request the transfer"
TRANSFER=$(until_state portitor "/v1/transfers/$TPID" 'STARTED|TERMINATED')
[[ $(jq -r "$STATE_PATH" <<<"$TRANSFER") == STARTED ]] || fail "expected STARTED: $TRANSFER"
ENDPOINT=$(jq -r '.dataAddress.endpoint' <<<"$TRANSFER")
TOKEN=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' \
  <<<"$TRANSFER")
"${COMPOSE[@]}" run --rm -q fetch -sf "$ENDPOINT" -H "authorization: Bearer $TOKEN" \
  >"$RUN/fetched-from-edc" || fail "fetch $ENDPOINT"
cmp -s "$RUN/fetched-from-edc" "$DIR/data/edc-provider.csv" || fail "the data from EDC differs"
echo "received $(wc -c <"$RUN/fetched-from-edc") bytes, as published"

# --- 6. EDC consumer, Portitor provider --------------------------------------

step "EDC reads the Portitor provider's catalog"
CATALOG=$(edc_management POST /v4/catalog/request "$(jq -nc --argjson c "$CONTEXT" \
  --arg addr "$PORTITOR_DSP_INSIDE" --arg id "$PORTITOR_DID" --arg p "$EDC_PROTOCOL" '{
  "@context": $c, "@type": "CatalogRequest",
  counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')") ||
  fail "EDC catalog request: $CATALOG"
catalog_entry() { # dataset id: prints the dataset as EDC read it
  local id=$1 entry
  entry=$(jq -c --arg id "$id" '[.["dcat:dataset"] // .dataset] | flatten | .[]
    | select(.["@id"] == $id)' <<<"$CATALOG")
  [[ -n $entry ]] || fail "no $id in the catalog: $CATALOG"
  echo "$entry"
}
first_offer() { # dataset entry: prints its first offer
  local entry=$1 offer
  offer=$(jq -c '[.["odrl:hasPolicy"] // .hasPolicy] | flatten | .[0]' <<<"$entry")
  [[ -n $offer && $offer != null ]] || fail "no offer in $entry"
  echo "$offer"
}
PORTITOR_ENTRY=$(catalog_entry "$PORTITOR_DATASET")
PORTITOR_OFFER=$(first_offer "$PORTITOR_ENTRY")
echo "Portitor offers: $PORTITOR_OFFER"
# EDC read the HealthDCAT-AP metadata (full IRIs) and the bundle's profile
echo "as EDC read it: $(jq -c '{conformsTo: (.["dct:conformsTo"] // .["http://purl.org/dc/terms/conformsTo"]),
  healthCategory: .["http://healthdataportal.eu/ns/health#healthCategory"]}' <<<"$PORTITOR_ENTRY")"
jq -e --arg p "$PROFILE" '[.["dct:conformsTo"] // .["http://purl.org/dc/terms/conformsTo"]] | flatten
  | any(.["@id"] == $p)' <<<"$PORTITOR_ENTRY" >/dev/null ||
  fail "EDC did not read dct:conformsTo $PROFILE: $PORTITOR_ENTRY"
jq -e '.["http://healthdataportal.eu/ns/health#healthCategory"] != null' <<<"$PORTITOR_ENTRY" \
  >/dev/null || fail "EDC did not read the HealthDCAT-AP metadata: $PORTITOR_ENTRY"

edc_negotiate() { # dataset offer: prints the ended EDC negotiation for that Portitor offer
  local dataset=$1 offer=$2 id
  id=$(edc_management POST /v4/contractnegotiations "$(jq -nc --argjson c "$CONTEXT" \
    --arg addr "$PORTITOR_DSP_INSIDE" --arg p "$EDC_PROTOCOL" --arg id "$PORTITOR_DID" \
    --argjson offer "$offer" --arg dataset "$dataset" '{
    "@context": $c, "@type": "ContractRequest", providerId: $id,
    counterPartyAddress: $addr, protocol: $p,
    policy: ($offer + {assigner: $id, target: $dataset})}')" | jq -r '.["@id"]') ||
    fail "start the EDC negotiation"
  until_state edc_management "/v4/contractnegotiations/$id" 'FINALIZED|TERMINATED'
}

step "EDC negotiates the Portitor offer"
VIEW=$(edc_negotiate "$PORTITOR_DATASET" "$PORTITOR_OFFER")
[[ $(jq -r "$STATE_PATH" <<<"$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
EDC_AGREEMENT=$(jq -r '.contractAgreementId' <<<"$VIEW")

edc_pull() { # agreement published-file: EDC pulls the data; checks it equals the file
  local agreement=$1 published=$2 transfer
  transfer=$(edc_management POST /v4/transferprocesses "$(jq -nc --argjson c "$CONTEXT" \
    --arg addr "$PORTITOR_DSP_INSIDE" --arg p "$EDC_PROTOCOL" --arg a "$agreement" '{
    "@context": $c, "@type": "TransferRequest",
    counterPartyAddress: $addr, protocol: $p, contractId: $a,
    transferType: "HttpData-PULL"}')" | jq -r '.["@id"]') || fail "start the EDC transfer"
  until_state edc_management "/v4/transferprocesses/$transfer" 'STARTED|TERMINATED' >"$RUN/edc-transfer.json"
  [[ $(jq -r "$STATE_PATH" "$RUN/edc-transfer.json") == STARTED ]] ||
    fail "expected STARTED: $(cat "$RUN/edc-transfer.json")"
  # the EDC data plane kept the data address Portitor sent; it fetches with it
  rm -f "$RUN/fetched-from-portitor"
  for _ in $(seq 1 20); do
    curl -sf "$EDC_DATAPLANE/consumer/$transfer/data" -o "$RUN/fetched-from-portitor" && break
    sleep 0.5
  done
  cmp -s "$RUN/fetched-from-portitor" "$published" || fail "the data from Portitor differs from $published"
  echo "received $(wc -c <"$RUN/fetched-from-portitor") bytes, as published"
}

step "EDC pulls the data from the Portitor provider"
edc_pull "$EDC_AGREEMENT" "$DIR/data/portitor-provider.csv"

# --- 7. Gaia-X compliance ---------------------------------------------------------

step "Gaia-X: Portitor's identity did its onboarding and holds a compliance credential"
# its DID document names the certificate chain of its key (ADR 0021)
PORTITOR_DOCUMENT=$(fetch https://portitor.gaiax.test/.well-known/did.json) ||
  fail "resolve $PORTITOR_DID over HTTPS"
X5U=$(jq -r '.verificationMethod[0].publicKeyJwk | select(.alg == "ES256") | .x5u // empty' <<<"$PORTITOR_DOCUMENT")
[[ $X5U == https://portitor.gaiax.test/.well-known/x509CertificateChain.pem ]] ||
  fail "$PORTITOR_DID has no alg ES256 and x5u: $PORTITOR_DOCUMENT"
# the identity starts a run on its own at start; then the POST gets 409
gaiax_start_compliance_run "$IDENTITY_MANAGEMENT" "$RUN/portitor/identity-management.key" Portitor
RUN_VIEW=$(gaiax_until_compliance_run_ended "$IDENTITY_MANAGEMENT" "$RUN/portitor/identity-management.key" Portitor)
echo "$RUN_VIEW"
[[ $(jq -r '.outcome' <<<"$RUN_VIEW") == SUCCEEDED ]] || fail "Portitor's compliance run: $RUN_VIEW"
gaiax_has_compliance_credential "$IDENTITY_MANAGEMENT" "$RUN/portitor/identity-management.key" ||
  fail "Portitor's identity holds no gx:ParticipantComplianceCredential from $COMPLIANCE_DID"
echo "Portitor's identity holds a gx:ParticipantComplianceCredential from $COMPLIANCE_DID"

step "Gaia-X: the Portitor provider's offer for compliant participants, as EDC read it"
GAIAX_ENTRY=$(catalog_entry "$GAIAX_DATASET")
GAIAX_OFFER=$(first_offer "$GAIAX_ENTRY")
echo "Portitor offers: $GAIAX_OFFER"
# EDC kept the constraint (ADR 0022): the left operand, and the label level
jq -e --arg operand "$GAIAX_OPERAND" '[.. | strings] | index($operand) != null and index("SC") != null' \
  <<<"$GAIAX_OFFER" >/dev/null || fail "the offer EDC read has no $GAIAX_OPERAND constraint: $GAIAX_OFFER"

step "Gaia-X: EDC, without a compliance credential, negotiates that offer: Portitor refuses"
edc_identity GET "/participants/$EDC_CONTEXT/credentials" |
  jq -e '[.[].verifiableCredential.credential.type[]] | index("gx:ParticipantComplianceCredential") == null' \
    >/dev/null || fail "the IdentityHub already holds a compliance credential"
VIEW=$(edc_negotiate "$GAIAX_DATASET" "$GAIAX_OFFER")
[[ $(jq -r "$STATE_PATH" <<<"$VIEW") == TERMINATED ]] || fail "expected TERMINATED: $VIEW"
# EDC is a member, so Portitor took the request and then ended the
# negotiation with a ContractNegotiationTerminationMessage: the policy
# does not hold. EDC notes an HTTP error as "code=<status>" (step 8); there
# is none here.
if jq -r '.errorDetail // empty' <<<"$VIEW" | grep -q 'code='; then
  fail "refused with an HTTP error, not with a termination message: $VIEW"
fi
echo "the negotiation ended TERMINATED: Portitor sent a termination message"

step "Gaia-X: the EDC participant's onboarding (signed with its key by scripts/gaiax_test_onboarding.py)"
# The EDC IdentityHub has no Gaia-X onboarding. This does what a Portitor
# identity does by itself (ADR 0021): it signs the participant's
# gx:LegalPerson and gx:Issuer credentials and the presentation of both
# with the registration number credential, and sends it to the compliance
# service, which resolves $EDC_DID over HTTPS.
python3 scripts/gaiax_test_onboarding.py sign "$RUN/edc/identity-key.pem" "$EDC_DID" "$EDC_KEY_ID" \
  "$RUN/edc/participant.json" "$RUN/edc/registration-number.jwt" "$EDC_GAIAX_BASE" \
  "$RUN/edc/www/credentials/gaiax" >"$RUN/edc/presentation.jwt" || fail "sign the EDC participant's Gaia-X credentials"
chmod -R a+rX "$RUN/edc/www"
# as Portitor's identity sends it (crates/portitor-identity/src/gaiax.rs):
# POST {base}/api/credential-offers/standard-compliance?vcid=<id>,
# Content-Type application/vp+jwt, no Accept header; the answer is a VC-JWT
VCID=$EDC_GAIAX_BASE/compliance
fetch -X POST "https://compliance.gaiax.test/api/credential-offers/standard-compliance?vcid=$(jq -rn --arg v "$VCID" '$v | @uri')" \
  -H 'content-type: application/vp+jwt' -H 'accept:' -d "$(cat "$RUN/edc/presentation.jwt")" \
  >"$RUN/edc/www/credentials/gaiax/compliance" ||
  fail "the compliance service refused the EDC participant: $(cat "$RUN/edc/www/credentials/gaiax/compliance")"
COMPLIANCE_MANIFEST=$(python3 scripts/gaiax_test_onboarding.py manifest \
  "$RUN/edc/www/credentials/gaiax/compliance" "$EDC_CONTEXT" gaiax-compliance) ||
  fail "the compliance service's answer is not a VC-JWT: $(cat "$RUN/edc/www/credentials/gaiax/compliance")"
jq -e --arg issuer "$COMPLIANCE_DID" --arg id "$VCID" '.verifiableCredentialContainer.credential
  | .id == $id and .issuer.id == $issuer and (.type | index("gx:ParticipantComplianceCredential") != null)
    and .credentialSubject[0]["gx:labelLevel"] == "SC"' <<<"$COMPLIANCE_MANIFEST" >/dev/null ||
  fail "not a gx:ParticipantComplianceCredential $VCID from $COMPLIANCE_DID with label level SC: $COMPLIANCE_MANIFEST"
echo "the compliance service issued $VCID"

step "Gaia-X: the EDC IdentityHub stores the compliance credential and the legal person credential (Identity API)"
# the two credentials a verifier needs (ADR 0022): the compliance
# credential, and the gx:LegalPerson credential it names by digest
LEGAL_PERSON_MANIFEST=$(python3 scripts/gaiax_test_onboarding.py manifest \
  "$RUN/edc/www/credentials/gaiax/legal-person" "$EDC_CONTEXT" gaiax-legal-person) ||
  fail "describe the legal person credential"
edc_identity POST "/participants/$EDC_CONTEXT/credentials" "$COMPLIANCE_MANIFEST" >/dev/null ||
  fail "store the compliance credential in the IdentityHub"
edc_identity POST "/participants/$EDC_CONTEXT/credentials" "$LEGAL_PERSON_MANIFEST" >/dev/null ||
  fail "store the legal person credential in the IdentityHub"
for gaiax_type in gx:ParticipantComplianceCredential gx:LegalPerson; do
  edc_identity GET "/participants/$EDC_CONTEXT/credentials?type=$(jq -rn --arg t "$gaiax_type" '$t | @uri')" |
    jq -e 'length == 1' >/dev/null || fail "the IdentityHub does not hold one $gaiax_type"
done
echo "the IdentityHub holds gx:ParticipantComplianceCredential and gx:LegalPerson"

step "Gaia-X: EDC, with its compliance credential, negotiates the same offer: Portitor agrees"
VIEW=$(edc_negotiate "$GAIAX_DATASET" "$GAIAX_OFFER")
[[ $(jq -r "$STATE_PATH" <<<"$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
GAIAX_AGREEMENT=$(jq -r '.contractAgreementId' <<<"$VIEW")

step "Gaia-X: EDC pulls the data for compliant participants from the Portitor provider"
# Portitor asks for the credentials again at the transfer request (ADR 0019)
edc_pull "$GAIAX_AGREEMENT" "$DIR/data/portitor-provider-gaiax.csv"

# --- 8. revocation --------------------------------------------------------------

step "The issuer revokes Portitor's credential: EDC refuses Portitor"
revoke() { # holder DID: the issuer revokes its credentials
  local holder=$1 id
  for id in $(issuer_api GET "/v1/credentials?holder=$(jq -rn --arg d "$holder" '$d | @uri')" |
    jq -r '.[].id'); do
    issuer_api POST "/v1/credentials/$(jq -rn --arg c "$id" '$c | @uri')/revoke" >/dev/null ||
      fail "revoke $id"
    echo "revoked $id"
  done
  sleep 2 # EDC reads status lists again after a second (revocation.cache.validity)
}
revoke "$PORTITOR_DID"
STATUS=$(edc_catalog_for_portitor)
# Portitor's management API answers 502 and names EDC's status
DETAIL=$(jq -r '.detail // empty' "$RUN/catalog.json")
[[ $STATUS == 502 && $DETAIL == "the provider answered HTTP 401"* ]] ||
  fail "after the revocation the catalog call answered $STATUS, not 502 for EDC's 401: $(cat "$RUN/catalog.json")"
echo "EDC answered 401 ($DETAIL)"

step "The issuer revokes the EDC participant's credential: Portitor refuses EDC"
revoke "$EDC_DID"
VIEW=$(edc_negotiate "$PORTITOR_DATASET" "$PORTITOR_OFFER")
[[ $(jq -r "$STATE_PATH" <<<"$VIEW") == TERMINATED ]] || fail "expected TERMINATED: $VIEW"
# a caller that is not authorized gets 404 on the negotiation routes (DSP
# 2025-1, "Contract Negotiation HTTPS Binding", "Unauthorized Access")
jq -r '.errorDetail' <<<"$VIEW" | grep -q 'code=404' || fail "not refused with 404: $VIEW"
echo "the negotiation ended TERMINATED: Portitor answered 404"

step "Done: credentials issued and checked both ways, data transferred both ways, Gaia-X compliance required and met, revocation honoured"
if [[ ${KEEP:-} == 1 ]]; then
  echo "KEEP=1: the data space keeps running; see docs/TESTING.md, \"The EDC interop run by hand\"."
  echo "Remove it with: ${COMPOSE[*]} down --volumes && rm -rf $RUN"
fi
