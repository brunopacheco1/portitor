#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The demo's checks, against a provider and a consumer that already run.
# scenarios/two-party/run-local.sh runs them on this machine; scenarios/two-party/run.sh runs
# them in Docker Compose. Exits non-zero at the first failed check.
#
# The provider uses the data space's HealthDCAT-AP bundle (scripts/demo-bundle.sh)
# and offers examples/healthdcat-datasets.json.
#
# 1. Without a DCP token the provider refuses the catalog request (401).
#    Asked through the consumer's management API (POST
#    /v1/catalog/request), the consumer's connector gets a token from its
#    STS; the provider checks the consumer's MembershipCredential and
#    answers. The two datasets that pass the bundle's shapes are in it,
#    with dct:conformsTo naming the bundle's profile and their
#    HealthDCAT-AP metadata as full IRIs, under the plain DSP @context;
#    bed-occupancy is not. One dataset is read the same way (POST
#    /v1/catalog/dataset/request).
# 2. The provider's harvest job (its management API) shows bed-occupancy
#    rejected by the SHACL check, with the SHACL report.
# 3. Negotiate the hospital-admissions offer unchanged, copied from the
#    catalog, through the consumer's management API: FINALIZED.
# 4. The same offer id without its constraint: TERMINATED (ADR 0009).
# 5. Credential constraints (ADR 0019): the restricted
#    cancer-registry-extract dataset's offer for members (the memberOf
#    claim of the consumer's MembershipCredential): FINALIZED; its offer
#    for auditors (a role claim the demo credential does not have):
#    TERMINATED.
# 6. Request a transfer under the hospital-admissions agreement: STARTED,
#    with a data address. Fetch the data with the token: it equals the
#    expected file. Suspend the transfer through the consumer's management
#    API: SUSPENDED, and the token is refused (401). Start it again:
#    STARTED, the same token, and the data is fetched again. The
#    provider's log has one audit line (data transfer done) for each of
#    the two fetches, complete, with the bytes of the file. Complete the
#    transfer: COMPLETED, the token is gone and refused.
# 7. A transfer under the members' agreement of cancer-registry-extract:
#    STARTED, and its data fetched (with object storage: from the
#    provider's bucket, s3://provider-data/..., ADR 0027).
# 8. A second credential type (ADR 0019): the hospital-admissions offer
#    for EU processors checks the consumer's DataProcessorCredential:
#    FINALIZED, a transfer STARTED. Once the issuer revokes that
#    credential, a new transfer request under the same agreement is
#    refused: TERMINATED.
# 9a. With object storage only (S3_ENDPOINT set): a push transfer
#    (HttpData-PUSH, ADR 0027) under the hospital-admissions agreement.
#    The provider sends the file to the consumer's receiving route, which
#    stores it in the consumer's bucket: COMPLETED, with objectKey, and the
#    object equals the file. The provider's log has a complete audit line
#    for it. The receiving route refuses a request without the token
#    (401).
# 9. The provider ends a transfer on its own (ADR 0027, its management
#    API: POST /v1/provided-transfers/{providerPid}/termination): the
#    transfer of step 7 is TERMINATED on both sides and its token is
#    refused at once (401).
# 10. Membership checked again on every data request (ADR 0027): a new
#    transfer under the hospital-admissions agreement, its data fetched;
#    the issuer revokes the consumer's MembershipCredential, and the next
#    request for the data is refused (401, the status list cache is 0);
#    the provider ends the transfer: TERMINATED.
#
# Environment:
#   PROVIDER    the provider's base URL, as the consumer reaches it
#   HERE        the provider's base URL, as this script reaches it
#               (default: PROVIDER)
#   PROVIDER_MANAGEMENT  the provider's management API base URL
#   PROVIDER_KEY_FILE    the file holding its management API key
#   MANAGEMENT  the consumer's management API base URL
#   KEY_FILE    the file holding the management API key
#   CONSUMER_ID, PROVIDER_ID  the two participants' DIDs
#   DATA        the folder of the files the provider serves (examples/data)
#   ISSUER_MANAGEMENT  the issuer's management API base URL
#   ISSUER_KEY_FILE    the file holding the issuer's management API key
#   PROVIDER_LOG       a file that receives the provider connector's log
#                      (JSON lines, as it writes them)
#   S3_ENDPOINT, S3_ACCESS_KEY, S3_SECRET_KEY  optional: the participants'
#                      object storage (RustFS) as this script reaches it,
#                      and its keys; with them, the object storage steps
#                      run (buckets provider-data and consumer-push)
#   CONSUMER_PUBLIC    optional: the consumer's public base URL, as this
#                      script reaches it (its receiving route)

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

: "${PROVIDER:?}" "${MANAGEMENT:?}" "${KEY_FILE:?}" "${DATA:?}"
: "${PROVIDER_MANAGEMENT:?}" "${PROVIDER_KEY_FILE:?}"
: "${CONSUMER_ID:?}" "${PROVIDER_ID:?}"
: "${ISSUER_MANAGEMENT:?}" "${ISSUER_KEY_FILE:?}" "${PROVIDER_LOG:?}"
HERE=${HERE:-$PROVIDER}
DSP=dsp/2025-1
DSP_CONTEXT=https://w3id.org/dspace/2025/1/context.jsonld
# the one protocol name the management API's catalog calls take (the
# name Eclipse EDC's management API uses for DSP 2025-1 over HTTPS)
PROTOCOL=dataspace-protocol-http:2025-1
# examples/healthdcat-datasets.json, and the profile scripts/demo-bundle.sh signs
DATASETS=https://provider.example/datasets
ADMISSIONS=$DATASETS/hospital-admissions
REGISTRY=$DATASETS/cancer-registry-extract
BED_OCCUPANCY=$DATASETS/bed-occupancy
PROFILE=https://data.health.europa.eu/healthdcat-ap/releases/release-8/

step() { printf '\n== %s\n' "$*"; }
fail() {
  printf 'FAILED: %s\n' "$*" >&2
  exit 1
}

wait_for() { # url
  local url=$1
  for _ in $(seq 1 100); do
    curl -sf "$url" >/dev/null && return 0
    sleep 0.2
  done
  fail "$url did not answer"
}

api() { # method path [body]: the management API with the key
  local method=$1 path=$2 body=${3-}
  local args=(-sf -X "$method" "$MANAGEMENT$path" -H "authorization: Bearer $(cat "$KEY_FILE")")
  if [[ $# -gt 2 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

state_of() { # record: its state
  local record=$1
  jq -r '.state' <<<"$record"
}

until_state() { # path, states (regex): prints the record once in one of them
  local path=$1 states=$2
  local view state
  for _ in $(seq 1 100); do
    view=$(api GET "$path") || fail "read $path"
    state=$(state_of "$view")
    if [[ $state =~ ^($states)$ ]]; then
      echo "$view"
      return 0
    fi
    sleep 0.2
  done
  fail "$path still $state"
}

wait_for "$HERE/health"
wait_for "$MANAGEMENT/health"

s3() { # method path [curl flags]: a request to the object storage, signed (SigV4)
  local method=$1 path=$2
  shift 2
  curl -s -X "$method" "$S3_ENDPOINT$path" --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$S3_ACCESS_KEY:$S3_SECRET_KEY" "$@"
}

if [[ -n ${S3_ENDPOINT:-} ]]; then
  step "Object storage: the buckets, and the provider's source in its bucket"
  for _ in $(seq 1 100); do
    [[ $(curl -s -o /dev/null -w '%{http_code}' "$S3_ENDPOINT/") != 000 ]] && break
    sleep 0.2
  done
  for bucket in provider-data consumer-push; do
    STATUS=$(s3 PUT "/$bucket" -o /dev/null -w '%{http_code}')
    [[ $STATUS == 200 || $STATUS == 409 ]] || fail "create bucket $bucket: HTTP $STATUS"
  done
  STATUS=$(s3 PUT /provider-data/cancer-registry-extract.csv -o /dev/null -w '%{http_code}' \
    -H 'content-type: text/csv' --data-binary "@$DATA/cancer-registry-extract.csv")
  [[ $STATUS == 200 ]] || fail "upload the provider's source: HTTP $STATUS"
  echo "s3://provider-data/cancer-registry-extract.csv uploaded"
fi

catalog_request() { # [curl flags]: the provider's catalog, asked directly
  curl -s -X POST "$HERE/$DSP/catalog/request" "$@" \
    -H 'content-type: application/json' -d '{
      "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
      "@type": "CatalogRequestMessage"}'
}

step "Read the provider's catalog without a DCP token: the provider refuses"
STATUS=$(catalog_request -o /dev/null -w '%{http_code}')
[[ $STATUS == 401 ]] || fail "without a token the provider answered $STATUS, not 401"
echo "the provider answered $STATUS"

step "Read the provider's catalog through the consumer's management API"
# the consumer's connector gets a token from its STS, as before every DSP
# request it sends; the provider checks it, then asks the consumer's
# credential service for its MembershipCredential
CATALOG=$(api POST /v1/catalog/request "$(jq -nc --arg addr "$PROVIDER/$DSP" \
  --arg id "$PROVIDER_ID" --arg protocol "$PROTOCOL" \
  '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $protocol}')") ||
  fail "catalog request"
[[ $(jq -r '.participantId' <<<"$CATALOG") == "$PROVIDER_ID" ]] || fail "another provider: $CATALOG"
jq -c '{"@context": .["@context"], dataset: [.dataset[] | {"@id", "dct:conformsTo"}]}' <<<"$CATALOG"

step "Read one dataset the same way"
DATASET=$(api POST /v1/catalog/dataset/request "$(jq -nc --arg addr "$PROVIDER/$DSP" \
  --arg id "$PROVIDER_ID" --arg protocol "$PROTOCOL" --arg dataset "$ADMISSIONS" \
  '{"@id": $dataset, counterPartyAddress: $addr, counterPartyId: $id, protocol: $protocol}')") ||
  fail "dataset request"
[[ $(jq -r '.["@id"]' <<<"$DATASET") == "$ADMISSIONS" ]] || fail "another dataset: $DATASET"
jq -c '{"@id", offers: [.hasPolicy[]["@id"]]}' <<<"$DATASET"

step "The catalog follows the bundle: dct:conformsTo, the metadata as full IRIs, no bed-occupancy"
# the @context is the DSP context alone (the DSP JSON Schema allows URLs
# only); terms outside it are written as full IRIs
jq -e --arg dsp "$DSP_CONTEXT" '.["@context"] == [$dsp]' <<<"$CATALOG" >/dev/null ||
  fail "the catalog @context is not the DSP context: $(jq -c '.["@context"]' <<<"$CATALOG")"
for dataset in "$ADMISSIONS" "$REGISTRY"; do
  jq -e --arg id "$dataset" --arg profile "$PROFILE" '[.dataset[] | select(.["@id"] == $id)
    | .["dct:conformsTo"] == {"@id": $profile, "@type": "dct:Standard"}
      and .["http://healthdataportal.eu/ns/health#healthCategory"]["@id"] != null] == [true]' \
    <<<"$CATALOG" >/dev/null || fail "$dataset is not in the catalog with dct:conformsTo $PROFILE"
done
jq -e --arg id "$BED_OCCUPANCY" '[.dataset[] | select(.["@id"] == $id)] == []' \
  <<<"$CATALOG" >/dev/null || fail "$BED_OCCUPANCY is in the catalog"
echo "both valid datasets conform to $PROFILE; $BED_OCCUPANCY is not offered"

step "The provider's harvest jobs: the bundle taken at start, and bed-occupancy rejected by the SHACL check, with its report"
provider_api() { # path: the provider's management API
  local path=$1
  curl -sf "$PROVIDER_MANAGEMENT$path" -H "authorization: Bearer $(cat "$PROVIDER_KEY_FILE")"
}
JOBS=$(provider_api "/v1/harvest-jobs?limit=10") || fail "list the harvest jobs"
# the bundle check at start is in the same job log (kind bundle)
BUNDLE_JOB=$(jq -c '[.jobs[] | select(.kind == "bundle")][0]' <<<"$JOBS")
[[ $(jq -r '.outcome' <<<"$BUNDLE_JOB") == succeeded ]] || fail "no bundle run that succeeded: $JOBS"
jq -r '.summary' <<<"$BUNDLE_JOB"
JOB=$(jq -r '[.jobs[] | select(.kind == "datasets")][0].id' <<<"$JOBS")
JOB=$(provider_api "/v1/harvest-jobs/$JOB") || fail "read harvest job $JOB"
jq '{id, trigger, outcome, summary}' <<<"$JOB"
REJECTED=$(jq -c --arg id "$BED_OCCUPANCY" '.errors[] | select(.datasetId == $id)' <<<"$JOB")
[[ -n $REJECTED ]] || fail "harvest job has no error for $BED_OCCUPANCY: $JOB"
[[ $(jq -r '.check' <<<"$REJECTED") == shacl ]] || fail "not rejected by the SHACL check: $REJECTED"
jq -e '.report != null' <<<"$REJECTED" >/dev/null || fail "no SHACL report: $REJECTED"
jq -r '.message' <<<"$REJECTED"
[[ $(jq '.errors | length' <<<"$JOB") == 1 ]] || fail "other datasets were rejected: $JOB"

negotiate() { # dataset offer -> prints the ended negotiation
  local dataset=$1 offer=$2
  local body pid
  body=$(jq -nc --arg addr "$PROVIDER/$DSP" --arg id "$PROVIDER_ID" --argjson offer "$offer" \
    --arg dataset "$dataset" \
    '{counterPartyAddress: $addr, counterPartyId: $id, datasetId: $dataset, offer: $offer}')
  pid=$(api POST /v1/negotiations "$body" | jq -r '.consumerPid') || fail "start negotiation"
  until_state "/v1/negotiations/$pid" 'FINALIZED|TERMINATED'
}

catalog_offer() { # dataset offer-id: the offer, as the catalog gives it
  local dataset=$1 offer_id=$2
  jq -c --arg dataset "$dataset" --arg id "$offer_id" '.dataset[] | select(.["@id"] == $dataset)
    | .hasPolicy[] | select(.["@id"] == $id)' <<<"$CATALOG"
}

step "Negotiate the hospital-admissions offer unchanged: the provider agrees"
OFFER=$(catalog_offer "$ADMISSIONS" hospital-admissions:offer:until-2030)
[[ -n $OFFER ]] || fail "no hospital-admissions offer in the catalog"
echo "provider $PROVIDER_ID offers: $OFFER"
VIEW=$(negotiate "$ADMISSIONS" "$OFFER")
jq '{consumerPid, providerPid, state, agreement}' <<<"$VIEW"
[[ $(state_of "$VIEW") == FINALIZED ]] || fail "expected FINALIZED"
AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")
[[ $AGREEMENT != null ]] || fail "no agreement"

step "Negotiate the same offer id without its constraint: the provider refuses"
CHANGED=$(jq -c 'del(.permission[0].constraint)' <<<"$OFFER")
REFUSED=$(negotiate "$ADMISSIONS" "$CHANGED")
jq '{consumerPid, providerPid, state}' <<<"$REFUSED"
[[ $(state_of "$REFUSED") == TERMINATED ]] || fail "expected TERMINATED"

step "Negotiate the restricted dataset's offer for members: the provider checks the consumer's MembershipCredential claim"
MEMBERS=$(catalog_offer "$REGISTRY" cancer-registry-extract:offer:members)
[[ -n $MEMBERS ]] || fail "no members' offer in the catalog"
VIEW=$(negotiate "$REGISTRY" "$MEMBERS")
jq '{state, constraint: .agreement.permission[0].constraint}' <<<"$VIEW"
[[ $(state_of "$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
MEMBERS_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")

step "Negotiate its offer for auditors: the consumer's credential has no role claim, the provider refuses"
AUDITORS=$(catalog_offer "$REGISTRY" cancer-registry-extract:offer:auditors)
[[ -n $AUDITORS ]] || fail "no auditors' offer in the catalog"
VIEW=$(negotiate "$REGISTRY" "$AUDITORS")
jq '{state}' <<<"$VIEW"
[[ $(state_of "$VIEW") == TERMINATED ]] || fail "expected TERMINATED: $VIEW"

transfer() { # agreement -> prints the transfer once STARTED or TERMINATED
  local agreement=$1 pid
  pid=$(api POST /v1/transfers "$(jq -nc --arg a "$agreement" '{agreementId: $a}')" |
    jq -r '.consumerPid') || fail "request transfer"
  until_state "/v1/transfers/$pid" 'STARTED|TERMINATED'
}

fetch() { # transfer expected-file: fetches the data with the transfer's token
  local view=$1 expected=$2
  local endpoint token
  endpoint=$(jq -r '.dataAddress.endpoint' <<<"$view")
  token=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' \
    <<<"$view")
  echo "data address: $endpoint (token not shown)"
  curl -sf "$endpoint" -H "authorization: Bearer $token" -o "${TMPDIR:-/tmp}/fetched" ||
    fail "fetch $endpoint"
  cmp -s "${TMPDIR:-/tmp}/fetched" "$expected" || fail "the data differs from $expected"
  cat "${TMPDIR:-/tmp}/fetched"
}

step "Request a transfer under agreement $AGREEMENT: the provider starts it"
TRANSFER=$(transfer "$AGREEMENT")
[[ $(state_of "$TRANSFER") == STARTED ]] || fail "expected STARTED: $TRANSFER"
TPID=$(jq -r '.consumerPid' <<<"$TRANSFER")
ENDPOINT=$(jq -r '.dataAddress.endpoint' <<<"$TRANSFER")
TOKEN=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' \
  <<<"$TRANSFER")

step "Fetch the data with the token"
fetch "$TRANSFER" "$DATA/hospital-admissions.csv"

step "Suspend the transfer: the provider's data plane refuses the token"
api POST "/v1/transfers/$TPID/suspension" >/dev/null || fail "suspend"
SUSPENDED=$(until_state "/v1/transfers/$TPID" 'SUSPENDED|TERMINATED')
[[ $(state_of "$SUSPENDED") == SUSPENDED ]] || fail "expected SUSPENDED: $SUSPENDED"
STATUS=$(curl -s -o /dev/null -w '%{http_code}' "$ENDPOINT" -H "authorization: Bearer $TOKEN")
[[ $STATUS == 401 ]] || fail "while suspended the data plane answered $STATUS, not 401"
echo "while SUSPENDED the data plane answered $STATUS"

step "Start the transfer again: the same token works"
api POST "/v1/transfers/$TPID/start" >/dev/null || fail "start again"
STARTED=$(until_state "/v1/transfers/$TPID" 'STARTED|TERMINATED')
[[ $(state_of "$STARTED") == STARTED ]] || fail "expected STARTED: $STARTED"
[[ $(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' \
  <<<"$STARTED") == "$TOKEN" ]] || fail "the token changed"
fetch "$STARTED" "$DATA/hospital-admissions.csv" >/dev/null
echo "fetched again with the same token"

step "The provider's log has an audit line for each of the two fetches"
# the flow id is the provider's process id; the 401 answers wrote none.
# Compose may prefix each line, so the JSON is read from its first {
FLOW=$(jq -r '.providerPid' <<<"$STARTED")
BYTES=$(wc -c <"$DATA/hospital-admissions.csv" | tr -d ' ')
audit_lines() {
  grep -F '"data transfer done"' "$PROVIDER_LOG" | sed 's/^[^{]*//' |
    jq -c --arg flow "$FLOW" 'select(.fields.flow == $flow) | .fields' || true
}
for _ in $(seq 1 50); do
  [[ $(audit_lines | wc -l) -ge 2 ]] && break
  sleep 0.2
done
LINES=$(audit_lines)
echo "$LINES"
[[ $(wc -l <<<"$LINES") == 2 ]] || fail "expected 2 audit lines for flow $FLOW, got: $LINES"
jq -se --arg agreement "$AGREEMENT" --arg consumer "$CONSUMER_ID" --argjson bytes "$BYTES" \
  'all(.agreement == $agreement and .counterParty == $consumer and .bytes == $bytes
       and .outcome == "complete")' <<<"$LINES" >/dev/null ||
  fail "the audit lines do not name the agreement, the consumer, $BYTES bytes and complete"

step "Complete the transfer: the token is dropped and refused"
api POST "/v1/transfers/$TPID/completion" >/dev/null || fail "complete"
DONE=$(until_state "/v1/transfers/$TPID" 'COMPLETED|TERMINATED')
[[ $(state_of "$DONE") == COMPLETED ]] || fail "expected COMPLETED"
[[ $(jq -r '.dataAddress' <<<"$DONE") == null ]] || fail "the token is still kept"
STATUS=$(curl -s -o /dev/null -w '%{http_code}' "$ENDPOINT" -H "authorization: Bearer $TOKEN")
[[ $STATUS == 401 ]] || fail "after completion the data plane answered $STATUS, not 401"

step "Transfer the restricted dataset under the members' agreement $MEMBERS_AGREEMENT"
TRANSFER=$(transfer "$MEMBERS_AGREEMENT")
[[ $(state_of "$TRANSFER") == STARTED ]] || fail "expected STARTED: $TRANSFER"
fetch "$TRANSFER" "$DATA/cancer-registry-extract.csv"
MEMBERS_TRANSFER=$TRANSFER

issuer_api() { # method path: the issuer's management API
  curl -sS --fail-with-body -X "$1" "$ISSUER_MANAGEMENT$2" \
    -H "authorization: Bearer $(cat "$ISSUER_KEY_FILE")"
}

step "Negotiate the offer for EU processors: the provider checks the consumer's DataProcessorCredential"
PROCESSING=$(catalog_offer "$ADMISSIONS" hospital-admissions:offer:eu-processors)
[[ -n $PROCESSING ]] || fail "no offer for EU processors in the catalog"
VIEW=$(negotiate "$ADMISSIONS" "$PROCESSING")
jq '{state, constraint: .agreement.permission[0].constraint}' <<<"$VIEW"
[[ $(state_of "$VIEW") == FINALIZED ]] || fail "expected FINALIZED: $VIEW"
PROCESSING_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")
TRANSFER=$(transfer "$PROCESSING_AGREEMENT")
[[ $(state_of "$TRANSFER") == STARTED ]] || fail "expected STARTED: $TRANSFER"
echo "transfer $(jq -r '.consumerPid' <<<"$TRANSFER") STARTED"

step "The issuer revokes the consumer's DataProcessorCredential: a new transfer is refused"
CREDENTIAL=$(issuer_api GET "/v1/credentials?holder=$(jq -rn --arg d "$CONSUMER_ID" '$d | @uri')" |
  jq -r '.[] | select(.credentialType == "DataProcessorCredential") | .id') ||
  fail "list the consumer's credentials at the issuer"
[[ -n $CREDENTIAL ]] || fail "the issuer has no DataProcessorCredential for $CONSUMER_ID"
issuer_api POST "/v1/credentials/$(jq -rn --arg c "$CREDENTIAL" '$c | @uri')/revoke" >/dev/null ||
  fail "revoke $CREDENTIAL"
echo "revoked $CREDENTIAL"
TRANSFER=$(transfer "$PROCESSING_AGREEMENT")
jq '{state}' <<<"$TRANSFER"
[[ $(state_of "$TRANSFER") == TERMINATED ]] || fail "expected TERMINATED: $TRANSFER"

provider_call() { # method path: the provider's management API, printing the status
  curl -s -o "${TMPDIR:-/tmp}/provider-answer" -w '%{http_code}' -X "$1" "$PROVIDER_MANAGEMENT$2" \
    -H "authorization: Bearer $(cat "$PROVIDER_KEY_FILE")"
}

provided_transfer() { # consumer pid: the provider's view of that transfer
  local cpid=$1
  provider_api "/v1/provided-transfers" |
    jq -c --arg c "$cpid" '.transfers[] | select(.consumerPid == $c)'
}

data_status() { # transfer view: the status of one more request for its data
  local view=$1 endpoint token
  endpoint=$(jq -r '.dataAddress.endpoint' <<<"$view")
  token=$(jq -r '.dataAddress.endpointProperties[] | select(.name == "authorization") | .value' \
    <<<"$view")
  curl -s -o /dev/null -w '%{http_code}' "$endpoint" -H "authorization: Bearer $token"
}

if [[ -n ${S3_ENDPOINT:-} ]]; then
  step "Push: the provider sends hospital-admissions into the consumer's bucket"
  PUSH_PID=$(api POST /v1/transfers "$(jq -nc --arg a "$AGREEMENT" \
    '{agreementId: $a, format: "HttpData-PUSH"}')" | jq -r '.consumerPid') ||
    fail "request a push transfer"
  PUSHED=$(until_state "/v1/transfers/$PUSH_PID" 'COMPLETED|TERMINATED')
  jq '{consumerPid, state, format, objectKey}' <<<"$PUSHED"
  [[ $(state_of "$PUSHED") == COMPLETED ]] || fail "expected COMPLETED: $PUSHED"
  [[ $(jq -r '.objectKey' <<<"$PUSHED") == "$PUSH_PID" ]] || fail "no objectKey: $PUSHED"
  [[ $(jq -r '.dataAddress' <<<"$PUSHED") == null ]] || fail "a push shows a data address"
  s3 GET "/consumer-push/$(jq -rn --arg k "$PUSH_PID" '$k | @uri')" -f \
    -o "${TMPDIR:-/tmp}/pushed" || fail "read the pushed object"
  cmp -s "${TMPDIR:-/tmp}/pushed" "$DATA/hospital-admissions.csv" ||
    fail "the pushed object differs from hospital-admissions.csv"
  echo "the consumer's bucket holds consumer-push/$PUSH_PID, equal to the file"
  PUSH_FLOW=$(jq -r '.providerPid' <<<"$PUSHED")
  PUSH_LINE=""
  for _ in $(seq 1 50); do
    PUSH_LINE=$(grep -F '"data transfer done"' "$PROVIDER_LOG" | sed 's/^[^{]*//' |
      jq -c --arg flow "$PUSH_FLOW" 'select(.fields.flow == $flow) | .fields' || true)
    [[ -n $PUSH_LINE ]] && break
    sleep 0.2
  done
  echo "$PUSH_LINE"
  BYTES=$(wc -c <"$DATA/hospital-admissions.csv" | tr -d ' ')
  jq -e --argjson bytes "$BYTES" '.outcome == "complete" and .bytes == $bytes' \
    <<<"$PUSH_LINE" >/dev/null || fail "no complete audit line of $BYTES bytes for the push"
  if [[ -n ${CONSUMER_PUBLIC:-} ]]; then
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$CONSUMER_PUBLIC/push/$PUSH_PID" \
      --data-binary 'not the provider')
    [[ $STATUS == 401 ]] || fail "the receiving route answered $STATUS without a token, not 401"
    echo "the receiving route refuses a request without the token: $STATUS"
  fi
fi

step "The provider ends the members' transfer on its own: access ends at once"
MEMBERS_PID=$(jq -r '.consumerPid' <<<"$MEMBERS_TRANSFER")
PROVIDED=$(provided_transfer "$MEMBERS_PID")
[[ -n $PROVIDED ]] || fail "the provider does not list transfer $MEMBERS_PID"
jq '{providerPid, consumerPid, state, counterPartyId}' <<<"$PROVIDED"
PROVIDER_PID=$(jq -r '.providerPid' <<<"$PROVIDED")
STATUS=$(provider_call POST "/v1/provided-transfers/$(jq -rn --arg p "$PROVIDER_PID" '$p | @uri')/termination")
[[ $STATUS == 202 ]] || fail "the termination answered $STATUS: $(cat "${TMPDIR:-/tmp}/provider-answer")"
STATUS=$(data_status "$MEMBERS_TRANSFER")
[[ $STATUS == 401 ]] || fail "after the provider's termination the data plane answered $STATUS, not 401"
echo "the data plane refuses the token: $STATUS"
ENDED=$(until_state "/v1/transfers/$MEMBERS_PID" 'TERMINATED')
echo "the consumer's transfer is $(state_of "$ENDED")"

step "The issuer revokes the consumer's MembershipCredential: its next request for data is refused"
TRANSFER=$(transfer "$AGREEMENT")
[[ $(state_of "$TRANSFER") == STARTED ]] || fail "expected STARTED: $TRANSFER"
fetch "$TRANSFER" "$DATA/hospital-admissions.csv" >/dev/null
echo "data fetched while a member"
MEMBERSHIP=$(issuer_api GET "/v1/credentials?holder=$(jq -rn --arg d "$CONSUMER_ID" '$d | @uri')" |
  jq -r '.[] | select(.credentialType == "MembershipCredential") | .id') ||
  fail "list the consumer's credentials at the issuer"
[[ -n $MEMBERSHIP ]] || fail "the issuer has no MembershipCredential for $CONSUMER_ID"
issuer_api POST "/v1/credentials/$(jq -rn --arg c "$MEMBERSHIP" '$c | @uri')/revoke" >/dev/null ||
  fail "revoke $MEMBERSHIP"
echo "revoked $MEMBERSHIP"
STATUS=$(data_status "$TRANSFER")
[[ $STATUS == 401 ]] || fail "after the revocation the data plane answered $STATUS, not 401"
echo "the data plane refuses the next request: $STATUS"
REVOKED_PID=$(jq -r '.consumerPid' <<<"$TRANSFER")
for _ in $(seq 1 100); do
  [[ $(provided_transfer "$REVOKED_PID" | jq -r '.state') == TERMINATED ]] && break
  sleep 0.2
done
[[ $(provided_transfer "$REVOKED_PID" | jq -r '.state') == TERMINATED ]] ||
  fail "the provider did not end the transfer: $(provided_transfer "$REVOKED_PID")"
echo "the provider ended the transfer: TERMINATED"

step "Done: the consumer authenticated with DCP, the catalog checked against the data space's HealthDCAT-AP bundle, a contract agreed, changed terms refused, credential claims checked, data transferred, the transfer suspended and started again, each pull in the provider's audit log, the transfer completed, a revoked credential refused, a transfer ended by the provider, and a revoked membership refused at the data plane"
