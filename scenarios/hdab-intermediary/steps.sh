#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The hdab-intermediary scenario (README.md), against the health data
# space that ../health-lu/run.sh started. Runs alone, or first: it needs
# the HDAB still a member and Schuman without an access body's role. Every call goes through a participant's own
# management API; the data users' apps (lab-app, schuman-app) fetch the
# data. Exits non-zero at the first failed check.
#
# 1. Lux Biomedicine Lab reads Lux Biobank's catalog, takes the offer for
#    EU processors (its DataProcessorCredential) and pulls the genotypes
#    into its plain folder. The offer for access bodies is refused to it.
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
#    Schuman's connector, so a new transfer never starts.
#
# Environment (set by run.sh):
#   DEMO   scenarios/health-lu
#   RUN    its run/ folder, with the management keys
#   AUTHORITY_MANAGEMENT  the authority's management API base URL
#   PILOT_END  when the HDAB's pilot offer ends

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

: "${DEMO:?}" "${RUN:?}" "${AUTHORITY_MANAGEMENT:?}" "${PILOT_END:?}"
# shellcheck source=scenarios/health-lu/steps-lib.sh
source "$DEMO/steps-lib.sh"

hdab_pulls() { # how many transfers the HDAB has asked Lux Biobank for
  api hdab GET /v1/transfers | jq '.transfers | length'
}

# --- 1 ------------------------------------------------------------------------

step "1. Lux Biomedicine Lab reads Lux Biobank's catalog"
CATALOG=$(catalog lab "$BIOBANK" "$BIOBANK_ID")
jq -c '.dataset[] | {"@id", offers: [.hasPolicy[] | {"@id", constraint: .permission[0].constraint}]}' <<<"$CATALOG"

step "1. The lab takes the offer for EU processors: Lux Biobank checks its DataProcessorCredential"
VIEW=$(negotiate lab "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:eu-processors)")
expect "the lab's agreement" "$VIEW" FINALIZED
LAB_AGREEMENT=$(jq -r '.agreement["@id"]' <<<"$VIEW")

step "1. The lab pulls the genotypes into its plain folder ($RUN/lab/storage)"
TRANSFER=$(transfer lab "$LAB_AGREEMENT")
expect "the lab's transfer" "$TRANSFER" STARTED
app_fetch lab-app "$TRANSFER" /storage/genotypes.vcf
complete lab "$TRANSFER"
cmp -s "$RUN/lab/storage/genotypes.vcf" "$DEMO/data/genotypes.vcf" || fail "the lab's copy differs"
echo "the lab has genotypes.vcf, $(wc -c <"$RUN/lab/storage/genotypes.vcf") bytes; transfer COMPLETED"

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
EXPECTED=$("${COMPOSE[@]}" run --rm --no-deps -q -v "$PWD/$DEMO:/demo:ro" schuman-app \
  python /demo/pipeline/counts.py /demo/data/genotypes.vcf)
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

step "5. Known gap: the lab is a data user too, and the HDAB agrees (no data permit until 1.2)"
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
authority_api() { # method path
  curl -sS --fail-with-body -X "$1" "$AUTHORITY_MANAGEMENT$2" \
    -H "authorization: Bearer $(cat "$RUN/authority/management.key")"
}
CREDENTIAL=$(authority_api GET "/v1/credentials?holder=$(jq -rn --arg d "$HDAB_ID" '$d | @uri')" |
  jq -r '.[] | select(.credentialType == "MembershipCredential") | .id') || fail "list the HDAB's credentials"
authority_api POST "/v1/credentials/$(jq -rn --arg c "$CREDENTIAL" '$c | @uri')/revoke" >/dev/null ||
  fail "revoke $CREDENTIAL"
echo "revoked $CREDENTIAL"

step "6. The HDAB is out of the data space: Lux Biobank refuses to show it its catalog"
if api hdab POST /v1/catalog/request "$(jq -nc --arg addr "$BIOBANK" --arg id "$BIOBANK_ID" --arg p "$PROTOCOL" \
  '{counterPartyAddress: $addr, counterPartyId: $id, protocol: $p}')" >/dev/null; then
  fail "Lux Biobank answered the HDAB"
fi
echo "refused"

step "6. Schuman's connector refuses the HDAB's messages too: a new transfer never starts"
PID=$(api schuman POST /v1/transfers "$(jq -nc --arg a "$SCHUMAN_AGREEMENT" '{agreementId: $a}')" |
  jq -r '.consumerPid') || fail "request transfer"
sleep 5
VIEW=$(api schuman GET "/v1/transfers/$PID") || fail "read the transfer"
expect "Schuman's transfer, 5 seconds later" "$VIEW" REQUESTED
echo "the HDAB's start message was refused (its MembershipCredential is revoked); Schuman ends the transfer itself"
api schuman POST "/v1/transfers/$PID/termination" >/dev/null || fail "terminate"
expect "Schuman's transfer" "$(until_state schuman "/v1/transfers/$PID" TERMINATED)" TERMINATED

step "Done: two data users took the same genotypes by two roads, peer to peer and through the HDAB; the HDAB pulled only after Schuman's transfer started and kept nothing; roles, claims, an ended offer and a revoked access body were refused"
