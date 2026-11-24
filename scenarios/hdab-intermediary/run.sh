#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The hdab-intermediary scenario (README.md) in Docker Compose
# (compose.yaml): Digital Lux Trust, Lux Biobank, Lux Biomedicine Lab, the
# HDAB and Schuman Clinical Research. Starts them, runs the steps
# (steps.sh) from this machine, then removes everything, volumes
# included.
#
#  1. Keys and secrets, in run/, and the keys of the lab's object
#     storage (RustFS), for this run only; the HealthDCAT-AP bundle, built
#     and signed with the issuer image (scripts/demo-bundle.sh) into
#     run/authority/bundle/; the HDAB's datasets with its pilot offer,
#     which ends PILOT_SECONDS from now.
#  2. Docker Compose up.
#  3. The authority registers the four participants with their claims;
#     each identity asks it for its credentials.
#  4. steps.sh.
#
# Usage: scenarios/hdab-intermediary/run.sh [connector image [identity image [issuer image]]]
#   Without images it uses the compose file's defaults (the release images).
#   Step 1 pushes into object storage (ADR 0027), so the connector image
#   must be 1.1 or later: with a 1.0 image run.sh stops before it starts
#   anything.
# Environment:
#   KEEP=1            leave the containers running, and run/, at the end
#   PILOT_SECONDS=n   how long the HDAB's pilot offer lasts (default 150);
#                     step 6 waits for its end
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
DIR=scenarios/hdab-intermediary
RUN=$DIR/run
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")
PILOT_SECONDS=${PILOT_SECONDS:-150}

# who is who (compose.yaml)
AUTHORITY=did:web:authority%3A8070
MEMBER_OF="Lux health data space"
PARTICIPANTS=(biobank lab hdab schuman)
declare -A ISSUER_MANAGEMENT=([authority]=http://127.0.0.1:18071)
declare -A IDENTITY_MANAGEMENT=([biobank]=http://127.0.0.1:18103 [lab]=http://127.0.0.1:18113
  [hdab]=http://127.0.0.1:18123 [schuman]=http://127.0.0.1:18133)
declare -A MANAGEMENT=([biobank]=http://127.0.0.1:18101 [lab]=http://127.0.0.1:18111
  [hdab]=http://127.0.0.1:18121 [schuman]=http://127.0.0.1:18131)
# shellcheck source=scripts/scenario-lib.sh
source scripts/scenario-lib.sh

# step 1 needs push (1.1): stop now with a 1.0 connector image, such as
# the default that compose.yaml names until 1.1.0 is released
DEFAULT_IMAGE=$(sed -n 's/^ *image: [$]{PORTITOR_IMAGE:-\([^}]*\)}.*/\1/p' "$DIR/compose.yaml")
CONNECTOR_IMAGE=${PORTITOR_IMAGE:-$DEFAULT_IMAGE}
if [[ $CONNECTOR_IMAGE == *:1.0 || $CONNECTOR_IMAGE == *:1.0.* ]]; then
  fail "the connector image $CONNECTOR_IMAGE has no push transfers, which step 1 uses: pass a 1.1 image (README.md, \"Run it\")"
fi

# --- 1. keys, secrets, bundle, datasets ---------------------------------------

step "Write the keys and the secrets into $RUN"
# containers and volumes left by an earlier run with KEEP=1 still use the
# old keys: remove them first, so this run starts clean
"${COMPOSE[@]}" --profile apps down --volumes --remove-orphans >/dev/null 2>&1 || true
rm -rf "$RUN"
mkdir -p "$RUN/pipeline"
write_issuer_keys authority
for participant in "${PARTICIPANTS[@]}"; do write_participant_keys "$participant"; done
# the lab's object storage: its keys, for this run only; compose.yaml
# passes them to RustFS and, as AWS_* settings, to the lab's connector
LAB_S3_ACCESS_KEY=lab-$(openssl rand -hex 8)
LAB_S3_SECRET_KEY=$(openssl rand -hex 32)
export LAB_S3_ACCESS_KEY LAB_S3_SECRET_KEY
# the same, for a look at the bucket with KEEP=1 (README.md)
printf 'AWS_ENDPOINT_URL_S3=http://127.0.0.1:18114\nAWS_ACCESS_KEY_ID=%s\nAWS_SECRET_ACCESS_KEY=%s\n' \
  "$LAB_S3_ACCESS_KEY" "$LAB_S3_SECRET_KEY" >"$RUN/lab/s3.env"

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
build_bundle "$AUTHORITY"

step "The HDAB's datasets: its pilot offer ends in $PILOT_SECONDS seconds"
PILOT_END=$(date -u -d "@$(($(date +%s) + PILOT_SECONDS))" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null ||
  date -u -r "$(($(date +%s) + PILOT_SECONDS))" +%Y-%m-%dT%H:%M:%SZ) # GNU date, then BSD date
# the pilot: the same terms as the data users' offer, with an end soon
jq --arg end "$PILOT_END" '.[0].hasPolicy += [.[0].hasPolicy[0]
  | .["@id"] = "variant-carriers:offer:pilot"
  | .permission[0].constraint |= map(if .leftOperand == "dateTime" then .rightOperand = $end else . end)]' \
  "$DIR/datasets/hdab.json" >"$RUN/hdab/datasets.json"
echo "pilot offer until $PILOT_END"
# the images run as user 65532 and must read them
chmod -R a+rX "$RUN"

# --- 2. start ---------------------------------------------------------------

step "Start the data space"
"${COMPOSE[@]}" up -d --quiet-pull
wait_for "${ISSUER_MANAGEMENT[authority]}/health"
for participant in "${PARTICIPANTS[@]}"; do
  wait_for "${IDENTITY_MANAGEMENT[$participant]}/health"
  wait_for "${MANAGEMENT[$participant]}/health"
done

# --- 3. the credentials -------------------------------------------------------

member() { # role [processing]
  jq -nc --arg m "$MEMBER_OF" --arg role "$1" --arg p "${2-}" \
    '{memberOf: $m, role: $role} + (if $p == "" then {} else {processing: $p} end)'
}

step "Digital Lux Trust registers the four participants with their claims"
register authority biobank "$(member data-holder)"
register authority lab "$(member data-user eu)"
register authority hdab "$(member health-data-access-body)"
register authority schuman "$(member data-user)"

step "Each identity asks Digital Lux Trust for its credentials"
request_credentials biobank "$AUTHORITY" membership-vc20
request_credentials lab "$AUTHORITY" membership-vc20 data-processor-vc20
request_credentials hdab "$AUTHORITY" membership-vc20
request_credentials schuman "$AUTHORITY" membership-vc20
echo "the lab holds a MembershipCredential and a DataProcessorCredential; the others a MembershipCredential"

# --- 4. the steps -------------------------------------------------------------

export DIR RUN PILOT_END
"$DIR/steps.sh"
