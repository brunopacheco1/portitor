#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The hdab-intermediary scenario (README.md), against the data space that
# run.sh started. Every call goes through a participant's own management
# API. Lux Biobank pushes the lab's copy into the lab's bucket; Schuman's
# app (schuman-app) fetches the counts. Exits non-zero at the first
# failed check.
#
# 1. Lux Biomedicine Lab reads Lux Biobank's catalog, takes the offer for
#    EU processors (its DataProcessorCredential) and asks for a push
#    transfer (HttpData-PUSH, ADR 0027): Lux Biobank sends the genotypes
#    to the lab connector's receiving route, which stores them in the
#    lab's own object storage (RustFS, bucket lab-genotypes). The object
#    must equal the original file. The offer for EU processors names a
#    purpose (1.4): a request with another purpose is refused. The offer
#    for access bodies is refused to the lab.
# 2. The HDAB takes the offer for access bodies (its MembershipCredential
#    role). Nothing is pulled.
# 3. Schuman Clinical Research is refused both of Lux Biobank's offers.
# 4. Schuman takes the HDAB's counts offer and starts a transfer; the
#    HDAB still has pulled nothing. When Schuman's app fetches the data,
#    the HDAB's pipeline pulls the genotypes from Lux Biobank and counts.
#    A second fetch, under a new transfer, pulls again.
# 5. Known gap: the lab, also a data user, may take the counts offer too.
# 6. Refusals: a transfer under the pilot offer once its end has passed;
#    after Digital Lux Trust revokes the HDAB's MembershipCredential, the
#    HDAB is out of the data space: Lux Biobank refuses it, and so does
#    Schuman's connector, so a new transfer never starts. The HDAB's
#    termination is the one message Schuman still accepts from it (TD-47,
#    fixed in 1.1), so the transfer ends TERMINATED on its own.
#
# Environment (set by run.sh):
#   DIR        scenarios/hdab-intermediary
#   RUN        its run/ folder, with the management keys
#   PILOT_END  when the HDAB's pilot offer ends
#   LAB_S3_ACCESS_KEY, LAB_S3_SECRET_KEY
#              the keys of the lab's object storage (lab-storage)

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

: "${DIR:?}" "${RUN:?}" "${PILOT_END:?}" "${LAB_S3_ACCESS_KEY:?}" "${LAB_S3_SECRET_KEY:?}"
# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")

# who is who (compose.yaml)
declare -A ISSUER_MANAGEMENT=([authority]=http://127.0.0.1:18071)
declare -A MANAGEMENT=([biobank]=http://127.0.0.1:18101 [lab]=http://127.0.0.1:18111
  [hdab]=http://127.0.0.1:18121 [schuman]=http://127.0.0.1:18131)
# shellcheck source=scripts/scenario-lib.sh
source scripts/scenario-lib.sh
BIOBANK=http://biobank:8080/$DSP
BIOBANK_ID=$(participant_did biobank)
HDAB=http://hdab:8080/$DSP
HDAB_ID=$(participant_did hdab)
GENOTYPES=https://biobank.health-lu.example/datasets/genotypes
CARRIERS=https://hdab.health-lu.example/datasets/variant-carriers
ORIGINAL=examples/genotypes/genotypes.vcf
LAB_S3=http://127.0.0.1:18114 # lab-storage, the lab's RustFS
LAB_BUCKET=lab-genotypes      # the lab connector's PORTITOR_PUSH_BUCKET
# the purpose of Lux Biobank's offer for EU processors (datasets/biobank.json),
# EHDS Art. 53(1)(e) as the DPV EHDS extension names it, and another one
PURPOSE_IRI=https://w3id.org/dpv/legal/eu/ehds#HealthcareScientificResearch
WRONG_PURPOSE_IRI=https://w3id.org/dpv/legal/eu/ehds#ImproveHealthcareEducation

hdab_pulls() { # how many transfers the HDAB has asked Lux Biobank for
  api hdab GET /v1/transfers | jq '.transfers | length'
}

lab_s3() { # method path [curl flags]: a request to the lab's object storage, signed (SigV4)
  local method=$1 path=$2
  shift 2
  curl -s -X "$method" "$LAB_S3$path" --aws-sigv4 "aws:amz:us-east-1:s3" \
    --user "$LAB_S3_ACCESS_KEY:$LAB_S3_SECRET_KEY" "$@"
}

# --- 1 ------------------------------------------------------------------------

step "1. Lux Biomedicine Lab reads Lux Biobank's catalog"
CATALOG=$(catalog lab "$BIOBANK" "$BIOBANK_ID")
jq -c '.dataset[] | {"@id", offers: [.hasPolicy[] | {"@id", constraint: .permission[0].constraint}]}' <<<"$CATALOG"

step "1. The lab takes the offer for EU processors: Lux Biobank checks its DataProcessorCredential"
VIEW=$(negotiate lab "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:eu-processors)")
expect "the lab's agreement" "$VIEW" FINALIZED
LAB_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")
jq -c '.agreement.permission[0].constraint[] | select(.leftOperand == "purpose")' <<<"$VIEW"
jq -e --arg p "$PURPOSE_IRI" \
  'any(.agreement.permission[0].constraint[]; .leftOperand == "purpose" and .operator == "eq" and .rightOperand == $p)' \
  <<<"$VIEW" >/dev/null || fail "the lab's agreement does not name the purpose $PURPOSE_IRI: $VIEW"

step "1. The lab asks with another purpose than the offer's: refused, the terms differ"
OFFER=$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:eu-processors)
WRONG_PURPOSE=$(jq -c --arg p "$WRONG_PURPOSE_IRI" \
  '(.permission[0].constraint[] | select(.leftOperand == "purpose") | .rightOperand) = $p' <<<"$OFFER")
VIEW=$(negotiate lab "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$WRONG_PURPOSE")
expect "the lab's request with purpose $WRONG_PURPOSE_IRI" "$VIEW" TERMINATED

step "1. The lab's object storage (RustFS): its bucket $LAB_BUCKET"
for _ in $(seq 1 100); do
  [[ $(curl -s -o /dev/null -w '%{http_code}' "$LAB_S3/") != 000 ]] && break
  sleep 0.2
done
STATUS=$(lab_s3 PUT "/$LAB_BUCKET" -o /dev/null -w '%{http_code}')
[[ $STATUS == 200 || $STATUS == 409 ]] || fail "create bucket $LAB_BUCKET: HTTP $STATUS"
echo "bucket $LAB_BUCKET ready"

step "1. The lab asks for a push: Lux Biobank sends the genotypes into the lab's bucket"
PID=$(api lab POST /v1/transfers "$(jq -nc --arg a "$LAB_AGREEMENT" '{agreementId: $a, format: "HttpData-PUSH"}')" |
  jq -r '.consumerPid') || fail "the lab: request a push transfer"
# Lux Biobank's data plane POSTs the file to http://lab:8080/push/$PID with
# the lab's token; the lab's connector stores it as object $PID, and Lux
# Biobank completes the transfer
VIEW=$(until_state lab "/v1/transfers/$PID" 'COMPLETED|TERMINATED')
jq -c '{consumerPid, state, format, objectKey}' <<<"$VIEW"
expect "the lab's push transfer" "$VIEW" COMPLETED
[[ $(jq -r '.objectKey' <<<"$VIEW") == "$PID" ]] || fail "no objectKey: $VIEW"
[[ $(jq -r '.dataAddress' <<<"$VIEW") == null ]] || fail "a push shows a data address: $VIEW"
lab_s3 GET "/$LAB_BUCKET/$(uri "$PID")" -f -o "$RUN/lab/genotypes.vcf" || fail "read the pushed object"
cmp -s "$RUN/lab/genotypes.vcf" "$ORIGINAL" || fail "the object in the lab's bucket differs from genotypes.vcf"
echo "the lab's bucket holds $LAB_BUCKET/$PID, $(wc -c <"$RUN/lab/genotypes.vcf" | tr -d ' ') bytes, equal to genotypes.vcf; transfer COMPLETED"

step "1. The lab's receiving route refuses anyone without its token: Schuman's app tries"
STATUS=$("${COMPOSE[@]}" run --rm --no-deps -q schuman-app python -c '
import sys, urllib.error, urllib.request
try:
    urllib.request.urlopen(urllib.request.Request(sys.argv[1], data=b"not Lux Biobank", method="POST"))
    print(200)
except urllib.error.HTTPError as e:
    print(e.code)' "http://lab:8080/push/$(uri "$PID")") || fail "Schuman's app did not run"
[[ $STATUS == 401 ]] || fail "the lab's receiving route answered $STATUS without a token, not 401"
echo "refused: $STATUS"

step "1. The lab asks for the offer for access bodies: refused, its role is data-user"
VIEW=$(negotiate lab "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:access-body)")
expect "the lab's request" "$VIEW" TERMINATED

# --- 2 ------------------------------------------------------------------------

step "2. The HDAB takes Lux Biobank's offer for access bodies: Lux Biobank checks its role"
CATALOG=$(catalog hdab "$BIOBANK" "$BIOBANK_ID")
VIEW=$(negotiate hdab "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:access-body)")
expect "the HDAB's agreement" "$VIEW" FINALIZED
jq -r '.agreement["@id"]' <<<"$VIEW" >"$RUN/pipeline/agreement"
echo "the HDAB's pipeline will pull under agreement $(cat "$RUN/pipeline/agreement")"
[[ $(hdab_pulls) == 0 ]] || fail "the HDAB pulled before anyone asked"
echo "the HDAB has pulled nothing"

# --- 3 ------------------------------------------------------------------------

step "3. Schuman Clinical Research asks Lux Biobank for the genotypes: both offers refused"
CATALOG=$(catalog schuman "$BIOBANK" "$BIOBANK_ID")
for id in genotypes:offer:access-body genotypes:offer:eu-processors; do
  VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" "$id")")
  expect "Schuman's request for $id" "$VIEW" TERMINATED
done

# --- 4 ------------------------------------------------------------------------

step "4. Schuman reads the HDAB's catalog and takes the counts offer for data users"
CATALOG=$(catalog schuman "$HDAB" "$HDAB_ID")
jq -c '.dataset[] | {"@id", offers: [.hasPolicy[] | {"@id", constraint: .permission[0].constraint}]}' <<<"$CATALOG"
VIEW=$(negotiate schuman "$HDAB" "$HDAB_ID" "$CARRIERS" "$(offer "$CATALOG" "$CARRIERS" variant-carriers:offer:data-users)")
expect "Schuman's agreement" "$VIEW" FINALIZED
SCHUMAN_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")
[[ $(hdab_pulls) == 0 ]] || fail "the HDAB pulled before a transfer started"
echo "the HDAB has still pulled nothing"

step "4. Schuman starts a transfer; the HDAB pulls only when Schuman's app fetches"
TRANSFER=$(transfer schuman "$SCHUMAN_AGREEMENT")
expect "Schuman's transfer" "$TRANSFER" STARTED
[[ $(hdab_pulls) == 0 ]] || fail "the HDAB pulled before the fetch"
BEFORE=${EPOCHREALTIME:-}
COUNTS=$(app_fetch schuman-app "$TRANSFER") || fail "Schuman's app got no counts"
if [[ -n $BEFORE ]]; then # bash 5: the fetch time, app start included (the HDAB's data plane waits 30 s at most)
  echo "the fetch took $(awk -v a="$BEFORE" -v b="$EPOCHREALTIME" 'BEGIN { printf "%.1f", b - a }') s"
fi
complete schuman "$TRANSFER"
[[ $(hdab_pulls) == 1 ]] || fail "the HDAB did not pull once: $(api hdab GET /v1/transfers)"
EXPECTED=$("${COMPOSE[@]}" run --rm --no-deps -q -v "$PWD:/repo:ro" schuman-app \
  python "/repo/$DIR/pipeline/counts.py" "/repo/$ORIGINAL")
[[ $COUNTS == "$EXPECTED" ]] || fail "the counts differ from counts.py on the genotypes"
echo "Schuman got $(($(wc -l <<<"$COUNTS") - 1)) variants; the first lines:"
head -5 <<<"$COUNTS"
echo "the HDAB pulled the genotypes once, transfer $(api hdab GET /v1/transfers | jq -r '.transfers[0].state')"

step "4. A second transfer: the HDAB pulls again, nothing is kept"
TRANSFER=$(transfer schuman "$SCHUMAN_AGREEMENT")
expect "Schuman's second transfer" "$TRANSFER" STARTED
SECOND=$(app_fetch schuman-app "$TRANSFER") || fail "no counts the second time"
complete schuman "$TRANSFER"
[[ $SECOND == "$COUNTS" ]] || fail "other counts the second time"
[[ $(hdab_pulls) == 2 ]] || fail "the HDAB did not pull a second time"
echo "the same counts; the HDAB pulled twice"

step "4. Schuman also takes the pilot offer (until $PILOT_END) and transfers once"
PILOT=$(negotiate schuman "$HDAB" "$HDAB_ID" "$CARRIERS" "$(offer "$CATALOG" "$CARRIERS" variant-carriers:offer:pilot)")
expect "Schuman's pilot agreement" "$PILOT" FINALIZED
PILOT_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$PILOT")
TRANSFER=$(transfer schuman "$PILOT_AGREEMENT")
expect "a transfer under the pilot" "$TRANSFER" STARTED
complete schuman "$TRANSFER"

# --- 5 ------------------------------------------------------------------------

step "5. Known gap: the lab is a data user too, and the HDAB agrees (no data permit asked for here; data permits since 1.4: data-permits scenario)"
CATALOG=$(catalog lab "$HDAB" "$HDAB_ID")
VIEW=$(negotiate lab "$HDAB" "$HDAB_ID" "$CARRIERS" "$(offer "$CATALOG" "$CARRIERS" variant-carriers:offer:data-users)")
expect "the lab's request for the counts" "$VIEW" FINALIZED

# --- 6 ------------------------------------------------------------------------

step "6. The pilot ends at $PILOT_END: a new transfer under it is refused"
END=$(date -u -d "$PILOT_END" +%s 2>/dev/null || date -u -j -f %Y-%m-%dT%H:%M:%SZ "$PILOT_END" +%s)
WAIT=$((END - $(date +%s) + 2))
if [[ $WAIT -gt 0 ]]; then
  echo "waiting $WAIT seconds"
  sleep "$WAIT"
fi
TRANSFER=$(transfer schuman "$PILOT_AGREEMENT")
expect "a transfer under the ended pilot" "$TRANSFER" TERMINATED

step "6. Digital Lux Trust revokes the HDAB's MembershipCredential"
CREDENTIAL=$(issuer_api authority GET "/v1/credentials?holder=$(uri "$HDAB_ID")" |
  jq -r '.[] | select(.credentialType == "MembershipCredential") | .id') || fail "list the HDAB's credentials"
issuer_api authority POST "/v1/credentials/$(uri "$CREDENTIAL")/revoke" >/dev/null || fail "revoke $CREDENTIAL"
echo "revoked $CREDENTIAL"

step "6. The HDAB is out of the data space: Lux Biobank refuses to show it its catalog"
if api hdab POST /v1/catalog/request "$(jq -nc --arg addr "$BIOBANK" --arg id "$BIOBANK_ID" --arg p "$PROTOCOL" \
  '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')" >/dev/null; then
  fail "Lux Biobank answered the HDAB"
fi
echo "refused"

step "6. Schuman's connector refuses the HDAB's messages too: a new transfer never starts, and ends"
PID=$(api schuman POST /v1/transfers "$(jq -nc --arg a "$SCHUMAN_AGREEMENT" '{agreementId: $a}')" |
  jq -r '.consumerPid') || fail "request transfer"
# the HDAB's start message is refused (its MembershipCredential is
# revoked); its termination is accepted from it all the same (TD-47)
VIEW=$(until_state schuman "/v1/transfers/$PID" 'STARTED|TERMINATED')
expect "Schuman's transfer" "$VIEW" TERMINATED
echo "the HDAB's start message was refused; its termination was accepted, so the transfer ended without Schuman's operator"

step "Done: two data users took the same genotypes by two roads, peer to peer (pushed into the lab's bucket) and through the HDAB; the HDAB pulled only after Schuman's transfer started and kept nothing; roles, claims, an ended offer and a revoked access body were refused"
