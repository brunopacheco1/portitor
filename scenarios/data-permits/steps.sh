#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The data-permits scenario (README.md), against the data space that
# run.sh started. The HDAB's own issuer gives Schuman Clinical Research
# data permits as credentials (DataPermitCredential); Lux Biobank has one
# offer per permit and checks it when Schuman negotiates and at each
# transfer. Exits non-zero at the first failed check.
#
# 1. The HDAB grants permit P-2026-001: its issuer registers Schuman with
#    the permit's claims, and Schuman's identity asks for the credential.
# 2. Schuman takes Lux Biobank's offer for that permit and pulls the
#    genotypes. The offer for P-2026-002 is refused: no such permit yet.
# 3. A second permit, P-2026-002. The HDAB replaces Schuman's claims at
#    its issuer (PUT, since 1.4, TD-46); Schuman stays registered. Schuman
#    holds both permits and takes the second offer.
# 4. The HDAB revokes P-2026-001. Schuman's identity reads the status
#    lists of the credentials it holds (since 1.4, TD-20; every 2 seconds
#    here) and stops presenting the revoked permit: the transfer under
#    P-2026-002 works without deleting it by hand, and the one under
#    P-2026-001 is refused.
# 5. A trusted issuer is trusted only for its types (since 1.4, R-48):
#    the HDAB's issuer would give Schuman a MembershipCredential with the
#    role health-data-access-body, but Schuman's identity and Lux
#    Biobank trust it for DataPermitCredential only
#    (PORTITOR_TRUSTED_ISSUER_TYPES). Schuman's identity does not ask for
#    it, and Lux Biobank's offer for access bodies refuses Schuman.
#
# The permit's claims (permitId, purpose) are placeholders: no data permit
# format is published yet (EHDS Art. 68(10) lists what a permit states;
# the implementing act of Art. 70 is pending). Its end is the
# credential's own validUntil.
#
# Environment (set by run.sh):
#   DIR   scenarios/data-permits
#   RUN   its run/ folder, with the management keys

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../scripts/require-bash.sh"

: "${DIR:?}" "${RUN:?}"
# shellcheck source=scripts/container-engine.sh
source scripts/container-engine.sh
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")

# who is who (compose.yaml)
declare -A ISSUER_MANAGEMENT=([authority]=http://127.0.0.1:18071 [hdab-issuer]=http://127.0.0.1:18125)
declare -A IDENTITY_MANAGEMENT=([biobank]=http://127.0.0.1:18103 [schuman]=http://127.0.0.1:18133)
declare -A MANAGEMENT=([biobank]=http://127.0.0.1:18101 [schuman]=http://127.0.0.1:18131)
# shellcheck source=scripts/scenario-lib.sh
source scripts/scenario-lib.sh
BIOBANK=http://biobank:8080/$DSP
BIOBANK_ID=$(participant_did biobank)
SCHUMAN_ID=$(participant_did schuman)
HDAB_ISSUER=did:web:hdab-issuer%3A8070
GENOTYPES=https://biobank.health-lu.example/datasets/genotypes
ORIGINAL=examples/genotypes/genotypes.vcf
# EHDS Art. 53(1)(e), as the DPV EHDS extension names it
PURPOSE=https://w3id.org/dpv/legal/eu/ehds#HealthcareScientificResearch

replace_schuman_claims() { # claims-json: the HDAB's issuer replaces Schuman's claims; it stays registered
  issuer_api hdab-issuer PUT "/v1/participants/$(uri "$SCHUMAN_ID")" "$(jq -nc --argjson c "$1" '{claims: $c}')" \
    >/dev/null || fail "replace Schuman's claims"
  echo "hdab-issuer replaced schuman's claims: $1"
}

schuman_asks() { # credential-id: Schuman's identity asks the HDAB's issuer, then waits for it
  request_credentials schuman "$HDAB_ISSUER" "$1"
  echo "Schuman holds a new $1"
}

newest_permit() { # the id of the newest DataPermitCredential the HDAB's issuer gave Schuman
  issuer_api hdab-issuer GET "/v1/credentials?holder=$(uri "$SCHUMAN_ID")" |
    jq -r '[.[] | select(.credentialType == "DataPermitCredential")] | sort_by(.issued) | last | .id'
}

permits_held() { # how many DataPermitCredentials Schuman's identity holds
  identity_api schuman GET /v1/credentials | jq '[.[] | select(.types | index("DataPermitCredential"))] | length'
}

pull_genotypes() { # agreement: Schuman pulls the genotypes and checks them
  local view
  view=$(transfer schuman "$1")
  expect "Schuman's transfer under $1" "$view" STARTED
  [[ $(app_fetch schuman-app "$view" | sha256sum) == "$(sha256sum <"$ORIGINAL")" ]] ||
    fail "Schuman's copy differs"
  complete schuman "$view"
  echo "Schuman has the genotypes; transfer COMPLETED"
}

credential_status() { # credential-id: the status Schuman's identity recorded for it (null until checked)
  identity_api schuman GET /v1/credentials | jq -r --arg id "$1" '.[] | select(.id == $id) | .status'
}

until_credential_status() { # credential-id status: waits until Schuman's identity recorded that status
  local tries=0 found=""
  # PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS is 2 on schuman-identity: a
  # few periods are enough
  while ((tries < 100)); do
    found=$(credential_status "$1")
    [[ $found == "$2" ]] && return 0
    sleep 0.2
    tries=$((tries + 1))
  done
  fail "Schuman's identity has not recorded $1 as $2 (status: $found)"
}

permit_offer() { # catalog permit-id
  offer "$1" "$GENOTYPES" "genotypes:offer:permit-$2"
}

# --- 1 ------------------------------------------------------------------------

step "1. The HDAB grants Schuman data permit P-2026-001, as a credential from its own issuer"
register hdab-issuer schuman "$(jq -nc --arg p "$PURPOSE" '{permitId: "P-2026-001", purpose: $p}')"
schuman_asks data-permit-vc20
PERMIT_1=$(newest_permit)
echo "permit P-2026-001 is credential $PERMIT_1"

# --- 2 ------------------------------------------------------------------------

step "2. Schuman reads Lux Biobank's catalog: one offer per permit"
CATALOG=$(catalog schuman "$BIOBANK" "$BIOBANK_ID")
jq -c '.dataset[] | .hasPolicy[] | select(.["@id"] | startswith("genotypes:offer:permit-"))
  | {"@id", constraint: .permission[0].constraint}' <<<"$CATALOG"

step "2. Schuman takes the offer for P-2026-001: Lux Biobank checks the permit"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-001)")
expect "Schuman's agreement under P-2026-001" "$VIEW" FINALIZED
AGREEMENT_1=$(jq -r '.agreement["@id"]' <<<"$VIEW")
pull_genotypes "$AGREEMENT_1"

step "2. Schuman asks for the offer for P-2026-002: refused, it has no such permit"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-002)")
expect "Schuman's request under P-2026-002" "$VIEW" TERMINATED

# --- 3 ------------------------------------------------------------------------

step "3. A second permit, P-2026-002: the HDAB replaces Schuman's claims at its issuer; Schuman stays registered"
replace_schuman_claims "$(jq -nc --arg p "$PURPOSE" '{permitId: "P-2026-002", purpose: $p}')"
schuman_asks data-permit-vc20
[[ $(permits_held) == 2 ]] || fail "Schuman does not hold two permits: $(identity_api schuman GET /v1/credentials)"
echo "Schuman holds two permits"

step "3. Schuman takes the offer for P-2026-002; the agreement under P-2026-001 still works"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-002)")
expect "Schuman's agreement under P-2026-002" "$VIEW" FINALIZED
AGREEMENT_2=$(jq -r '.agreement["@id"]' <<<"$VIEW")
pull_genotypes "$AGREEMENT_1"

# --- 4 ------------------------------------------------------------------------

step "4. The HDAB revokes permit P-2026-001"
issuer_api hdab-issuer POST "/v1/credentials/$(uri "$PERMIT_1")/revoke" >/dev/null || fail "revoke $PERMIT_1"
echo "revoked $PERMIT_1"

step "4. Schuman's identity reads the HDAB issuer's status list and records the permit as revoked"
until_credential_status "$PERMIT_1" REVOKED
[[ $(permits_held) == 2 ]] || fail "Schuman holds $(permits_held) permits, not 2: nobody deletes one by hand"
echo "Schuman's identity: $PERMIT_1 REVOKED, no longer presented; still held"

step "4. A transfer under P-2026-002 works: the revoked permit is not presented, nobody deleted it"
pull_genotypes "$AGREEMENT_2"

step "4. A transfer under P-2026-001 is refused: Schuman no longer presents that permit"
TRANSFER=$(transfer schuman "$AGREEMENT_1")
expect "a transfer under P-2026-001" "$TRANSFER" TERMINATED

# --- 5 ------------------------------------------------------------------------

step "5. The HDAB's issuer would make Schuman an access body: it gives Schuman the role health-data-access-body"
replace_schuman_claims '{"memberOf": "Lux health data space", "role": "health-data-access-body"}'

step "5. Schuman's identity does not ask for that MembershipCredential: it trusts the HDAB's issuer for data permits only"
ASK=$(jq -nc --arg issuer "$HDAB_ISSUER" '{issuerDid: $issuer, credentialIds: ["membership-vc20"]}')
if ANSWER=$(identity_api schuman POST /v1/credential-requests "$ASK"); then
  fail "Schuman's identity asked the HDAB's issuer for a MembershipCredential: $ANSWER"
fi
grep -q PORTITOR_TRUSTED_ISSUER_TYPES <<<"$ANSWER" || fail "Schuman's identity refused for another reason: $ANSWER"
echo "refused: $(jq -r '.detail' <<<"$ANSWER")"

step "5. Lux Biobank's offer for access bodies refuses Schuman"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:access-body)")
expect "Schuman's request for the offer for access bodies" "$VIEW" TERMINATED
echo "refused: Lux Biobank trusts the HDAB's issuer for data permits only"

step "Done: two permits gave two agreements under one registration; a revoked permit stopped only its own transfers, with no deletion by hand; an issuer trusted for permits could not make Schuman an access body"
