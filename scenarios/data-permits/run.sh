#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Runs the data-permits scenario (README.md): starts the health-lu data space
# (../health-lu/run.sh) and runs only this scenario's steps.sh on it.
#
# Usage: scenarios/data-permits/run.sh [connector image [identity image [issuer image]]]
#   The environment of ../health-lu/run.sh applies (KEEP, PILOT_SECONDS,
#   CONTAINER_ENGINE).

set -euo pipefail
SCENARIOS=data-permits exec "$(dirname "$0")/../health-lu/run.sh" "$@"
