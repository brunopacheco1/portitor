#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The health demo's second scenario (README.md, "Second scenario: data
# permits"), after steps.sh, against the data space run.sh started. The
# HDAB's own issuer gives Schuman Clinical Research data permits as
# credentials (DataPermitCredential); Lux Biobank has one offer per
# permit and checks it when Schuman negotiates and at each transfer.
# Exits non-zero at the first failed check.
#
# 7. The HDAB grants permit P-2026-001: its issuer registers Schuman with
#    the permit's claims, and Schuman's identity asks for the credential.
# 8. Schuman takes Lux Biobank's offer for that permit and pulls the
#    genotypes. The offer for P-2026-002 is refused: no such permit yet.
# 9. A second permit, P-2026-002. The issuer keeps one set of claims per
#    participant, so the HDAB registers Schuman again with the new ones.
#    Schuman holds both permits and takes the second offer.
# 10. The HDAB revokes P-2026-001. Schuman shows every permit it holds,
#    and a presentation with a revoked credential is refused as a whole:
#    the transfer under P-2026-002 is refused too, until Schuman deletes
#    the revoked permit from its identity.
# 11. A trusted issuer is trusted for every credential type: the HDAB's
#    issuer gives Schuman a MembershipCredential with the role
#    health-data-access-body, and Lux Biobank's offer for access bodies
#    accepts it. Digital Lux Trust revoked the HDAB's own
#    MembershipCredential in step 6; its issuer is trusted all the same.
#
# The permit's claims (permitId, purpose) are placeholders: no data permit
# format is published yet (EHDS Art. 68(10) lists what a permit states;
# the implementing act of Art. 70 is pending). Its end is the
# credential's own validUntil.
#
# Environment (set by run.sh):
#   DEMO   examples/demos/health-lu
#   RUN    its run/ folder, with the management keys
#   HDAB_ISSUER_MANAGEMENT  the HDAB issuer's management API base URL

set -euo pipefail
# shellcheck source=scripts/require-bash.sh
source "$(dirname "$0")/../../../scripts/require-bash.sh"

: "${DEMO:?}" "${RUN:?}" "${HDAB_ISSUER_MANAGEMENT:?}"
# shellcheck source=examples/demos/health-lu/steps-lib.sh
source "$DEMO/steps-lib.sh"

HDAB_ISSUER=did:web:hdab-issuer%3A8070
SCHUMAN_ID=did:web:schuman-identity%3A8090
SCHUMAN_IDENTITY=http://127.0.0.1:18133
PURPOSE=scientific-research # EHDS Art. 53(1)(e), as a placeholder value

issuer_api() { # method path [body]: the HDAB issuer's management API
  local method=$1 path=$2 body=${3-}
  local args=(-sS --fail-with-body -X "$method" "$HDAB_ISSUER_MANAGEMENT$path"
    -H "authorization: Bearer $(cat "$RUN/hdab-issuer/management.key")")
  if [[ $# -gt 2 ]]; then args+=(-H 'content-type: application/json' -d "$body"); fi
  curl "${args[@]}"
}

identity_api() { # method path: Schuman's identity management API
  curl -sS --fail-with-body -X "$1" "$SCHUMAN_IDENTITY$2" \
    -H "authorization: Bearer $(cat "$RUN/schuman/identity-management.key")"
}

uri() { jq -rn --arg v "$1" '$v | @uri'; }

register_schuman() { # claims-json: the HDAB's issuer registers Schuman, again if it was
  if issuer_api GET /v1/participants | jq -e --arg d "$SCHUMAN_ID" 'any(.[]; .did == $d)' >/dev/null; then
    issuer_api DELETE "/v1/participants/$(uri "$SCHUMAN_ID")" >/dev/null || fail "remove Schuman's registration"
  fi
  issuer_api POST /v1/participants "$(jq -nc --arg did "$SCHUMAN_ID" --argjson claims "$1" \
    '{did: $did, claims: $claims}')" >/dev/null || fail "register Schuman at the HDAB's issuer"
  echo "the HDAB's issuer registered Schuman with $1"
}

schuman_asks() { # credential-id: Schuman's identity asks the HDAB's issuer, then waits for it
  local holder_pid view status
  holder_pid=$(curl -sS --fail-with-body -X POST "$SCHUMAN_IDENTITY/v1/credential-requests" \
    -H "authorization: Bearer $(cat "$RUN/schuman/identity-management.key")" \
    -H 'content-type: application/json' \
    -d "$(jq -nc --arg issuer "$HDAB_ISSUER" --arg id "$1" '{issuerDid: $issuer, credentialIds: [$id]}')" |
    jq -r '.holderPid') || fail "Schuman's request for $1"
  for _ in $(seq 1 100); do
    view=$(identity_api GET "/v1/credential-requests/$holder_pid") || fail "read Schuman's request"
    status=$(jq -r '.status' <<<"$view")
    case $status in
      ISSUED)
        echo "Schuman holds a new $1"
        return 0
        ;;
      REJECTED) fail "the HDAB's issuer rejected Schuman's request: $view" ;;
    esac
    sleep 0.2
  done
  fail "Schuman's $1 did not arrive: $view"
}

newest_permit() { # the id of the newest DataPermitCredential the HDAB's issuer gave Schuman
  issuer_api GET "/v1/credentials?holder=$(uri "$SCHUMAN_ID")" |
    jq -r '[.[] | select(.credentialType == "DataPermitCredential")] | sort_by(.issued) | last | .id'
}

permits_held() { # how many DataPermitCredentials Schuman's identity holds
  identity_api GET /v1/credentials | jq '[.[] | select(.types | index("DataPermitCredential"))] | length'
}

pull_genotypes() { # agreement: Schuman pulls the genotypes and checks them
  local view
  view=$(transfer schuman "$1")
  expect "Schuman's transfer under $1" "$view" STARTED
  [[ $(app_fetch schuman-app "$view" | sha256sum) == "$(sha256sum <"$DEMO/data/genotypes.vcf")" ]] ||
    fail "Schuman's copy differs"
  complete schuman "$view"
  echo "Schuman has the genotypes; transfer COMPLETED"
}

revoked_permit_refusals() { # how often Lux Biobank's log refused a presentation for a revoked permit
  "${COMPOSE[@]}" logs --no-color biobank | grep -c 'a DataPermitCredential credential fails its checks.*revoked' || true
}

permit_offer() { # catalog permit-id
  offer "$1" "$GENOTYPES" "genotypes:offer:permit-$2"
}

# --- 7 ------------------------------------------------------------------------

step "7. The HDAB grants Schuman data permit P-2026-001, as a credential from its own issuer"
register_schuman "$(jq -nc --arg p "$PURPOSE" '{permitId: "P-2026-001", purpose: $p}')"
schuman_asks data-permit-vc20
PERMIT_1=$(newest_permit)
echo "permit P-2026-001 is credential $PERMIT_1"

# --- 8 ------------------------------------------------------------------------

step "8. Schuman reads Lux Biobank's catalog: one offer per permit"
CATALOG=$(catalog schuman "$BIOBANK" "$BIOBANK_ID")
jq -c '.dataset[] | .hasPolicy[] | select(.["@id"] | startswith("genotypes:offer:permit-"))
  | {"@id", constraint: .permission[0].constraint}' <<<"$CATALOG"

step "8. Schuman takes the offer for P-2026-001: Lux Biobank checks the permit"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-001)")
expect "Schuman's agreement under P-2026-001" "$VIEW" FINALIZED
AGREEMENT_1=$(jq -r '.agreement["@id"]' <<<"$VIEW")
pull_genotypes "$AGREEMENT_1"

step "8. Schuman asks for the offer for P-2026-002: refused, it has no such permit"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-002)")
expect "Schuman's request under P-2026-002" "$VIEW" TERMINATED

# --- 9 ------------------------------------------------------------------------

step "9. A second permit, P-2026-002: the issuer keeps one set of claims per participant, so the HDAB registers Schuman again"
register_schuman "$(jq -nc --arg p "$PURPOSE" '{permitId: "P-2026-002", purpose: $p}')"
schuman_asks data-permit-vc20
[[ $(permits_held) == 2 ]] || fail "Schuman does not hold two permits: $(identity_api GET /v1/credentials)"
echo "Schuman holds two permits"

step "9. Schuman takes the offer for P-2026-002; the agreement under P-2026-001 still works"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(permit_offer "$CATALOG" P-2026-002)")
expect "Schuman's agreement under P-2026-002" "$VIEW" FINALIZED
AGREEMENT_2=$(jq -r '.agreement["@id"]' <<<"$VIEW")
pull_genotypes "$AGREEMENT_1"

# --- 10 -----------------------------------------------------------------------

step "10. The HDAB revokes permit P-2026-001"
issuer_api POST "/v1/credentials/$(uri "$PERMIT_1")/revoke" >/dev/null || fail "revoke $PERMIT_1"
echo "revoked $PERMIT_1"
TRANSFER=$(transfer schuman "$AGREEMENT_1")
expect "a transfer under P-2026-001" "$TRANSFER" TERMINATED
REFUSALS=$(revoked_permit_refusals)

step "10. A transfer under P-2026-002 is refused too: Schuman shows both permits, and one is revoked"
TRANSFER=$(transfer schuman "$AGREEMENT_2")
expect "a transfer under P-2026-002" "$TRANSFER" TERMINATED
# Schuman's record gives no reason; Lux Biobank's log does
[[ $(revoked_permit_refusals) -gt $REFUSALS ]] || fail "Lux Biobank's log does not name the revoked permit"
echo "Lux Biobank's log: a DataPermitCredential is revoked, so the presentation is invalid; Schuman is told no reason"

step "10. Schuman deletes the revoked permit from its identity: P-2026-002 works again"
identity_api DELETE "/v1/credentials/$(uri "$PERMIT_1")" >/dev/null || fail "delete $PERMIT_1"
[[ $(permits_held) == 1 ]] || fail "Schuman still holds $(permits_held) permits"
pull_genotypes "$AGREEMENT_2"

# --- 11 -----------------------------------------------------------------------

step "11. The HDAB's issuer gives Schuman a MembershipCredential with the role health-data-access-body"
register_schuman '{"memberOf": "Lux health data space", "role": "health-data-access-body"}'
schuman_asks membership-vc20

step "11. Lux Biobank's offer for access bodies accepts it: a trusted issuer is trusted for every type"
VIEW=$(negotiate schuman "$BIOBANK" "$BIOBANK_ID" "$GENOTYPES" "$(offer "$CATALOG" "$GENOTYPES" genotypes:offer:access-body)")
expect "Schuman's request for the offer for access bodies" "$VIEW" FINALIZED
echo "accepted: Lux Biobank cannot say which issuer may issue which type, and the HDAB's own MembershipCredential was revoked in step 6"

step "Done: two permits gave two agreements; one revoked permit blocked the other until Schuman deleted it; an issuer trusted for permits could also make Schuman an access body"
