# shellcheck shell=bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The scripts need bash 4.4 or later: they use associative arrays (4.0)
# and expand arrays that may be empty under `set -u` (an error before 4.4).
# macOS ships bash 3.2 as /bin/bash; the scripts start with
# `#!/usr/bin/env bash`, so a newer bash earlier in PATH is enough:
#   brew install bash
# Source it first, before anything else; do not run it. Written so that
# bash 3.2 can read it. See docs/TESTING.md, "Docker or Podman".

if [ -z "${BASH_VERSINFO:-}" ] ||
  [ "${BASH_VERSINFO[0]}" -lt 4 ] ||
  { [ "${BASH_VERSINFO[0]}" -eq 4 ] && [ "${BASH_VERSINFO[1]}" -lt 4 ]; }; then
  echo "$0 needs bash 4.4 or later; this is ${BASH_VERSION:-not bash}." >&2
  echo "On macOS: brew install bash, and have it before /bin in PATH." >&2
  exit 1
fi
