#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The data-permits scenario (README.md) in Docker Compose (compose.yaml):
# Digital Lux Trust, the HDAB's issuer, Lux Biobank and Schuman Clinical
# Research. Starts them, runs the steps (steps.sh) from this machine, then
# removes everything, volumes included.
#
#  1. Keys and secrets, in run/; the HealthDCAT-AP bundle, built and
#     signed with the issuer image (scripts/demo-bundle.sh) into
#     run/authority/bundle/.
#  2. Docker Compose up.
#  3. The authority registers the two participants; each identity asks
#     it for its MembershipCredential.
#  4. steps.sh.
#
# Usage: scenarios/data-permits/run.sh [connector image [identity image [issuer image]]]
#   Without images it uses the compose file's defaults (the release images).
# Environment:
#   KEEP=1            leave the containers running, and run/, at the end
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
DIR=scenarios/data-permits
RUN=$DIR/run
COMPOSE=("${ENGINE_COMPOSE[@]}" -f "$DIR/compose.yaml")

# who is who (compose.yaml)
AUTHORITY=did:web:authority%3A8070
MEMBER_OF="Lux health data space"
PARTICIPANTS=(biobank schuman)
declare -A ISSUER_MANAGEMENT=([authority]=http://127.0.0.1:18071 [hdab-issuer]=http://127.0.0.1:18125)
declare -A IDENTITY_MANAGEMENT=([biobank]=http://127.0.0.1:18103 [schuman]=http://127.0.0.1:18133)
declare -A MANAGEMENT=([biobank]=http://127.0.0.1:18101 [schuman]=http://127.0.0.1:18131)
# shellcheck source=scripts/scenario-lib.sh
source scripts/scenario-lib.sh
# shellcheck source=scripts/demo-database.sh
source scripts/demo-database.sh # write_database_secrets

# --- 1. keys, secrets, bundle -------------------------------------------------

step "Write the keys and the secrets into $RUN"
# containers and volumes left by an earlier run with KEEP=1 still use the
# old keys: remove them first, so this run starts clean
"${COMPOSE[@]}" --profile apps down --volumes --remove-orphans >/dev/null 2>&1 || true
rm -rf "$RUN"
write_issuer_keys authority
write_issuer_keys hdab-issuer
for participant in "${PARTICIPANTS[@]}"; do write_participant_keys "$participant"; done
# the database: a test CA, its certificate and one password per component
write_database_secrets "$RUN/database" authority hdab_issuer biobank_identity biobank_connector \
  schuman_identity schuman_connector

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

# the images run as user 65532 and must read them
chmod -R a+rX "$RUN"

# --- 2. start ---------------------------------------------------------------

step "Start the data space"
"${COMPOSE[@]}" up -d --quiet-pull
wait_for "${ISSUER_MANAGEMENT[authority]}/health"
wait_for "${ISSUER_MANAGEMENT[hdab-issuer]}/health"
for participant in "${PARTICIPANTS[@]}"; do
  wait_for "${IDENTITY_MANAGEMENT[$participant]}/health"
  wait_for "${MANAGEMENT[$participant]}/health"
done

# --- 3. the credentials -------------------------------------------------------

member() { # role
  jq -nc --arg m "$MEMBER_OF" --arg role "$1" '{memberOf: $m, role: $role}'
}

step "Digital Lux Trust registers the two participants with their claims"
register authority biobank "$(member data-holder)"
register authority schuman "$(member data-user)"

step "Each identity asks Digital Lux Trust for its MembershipCredential"
request_credentials biobank "$AUTHORITY" membership-vc20
request_credentials schuman "$AUTHORITY" membership-vc20

# --- 4. the steps -------------------------------------------------------------

export DIR RUN
"$DIR/steps.sh"
