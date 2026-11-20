# shellcheck shell=bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Which container engine the scripts use: Docker or Podman
# (docs/TESTING.md, "Docker or Podman"). The one place that decides it.
#
# Source it; do not run it. It stops the script with a message when no
# engine can be used. It sets:
#
#   CONTAINER_ENGINE  docker or podman
#   ENGINE            the engine's command, an array:  "${ENGINE[@]}" build ...
#   ENGINE_COMPOSE    its Compose command, an array:   "${ENGINE_COMPOSE[@]}" -f compose.yaml up -d
#
# and defines engine_bind_mount_user (below).
#
# How the engine is chosen:
#   1. CONTAINER_ENGINE=docker or CONTAINER_ENGINE=podman in the
#      environment: that one. It must be installed and answer.
#   2. Otherwise docker, if the command exists and `docker info` answers.
#   3. Otherwise podman, if the command exists and `podman info` answers.
# A `docker` command that is Podman under another name (`docker --version`
# says so) counts as podman.
#
# Compose: `docker compose` (the plugin), or `podman compose` with Docker
# Compose (the `docker-compose` binary) as its provider. `podman compose`
# only starts a provider, docker-compose before podman-compose:
# https://github.com/containers/podman/blob/main/docs/source/markdown/podman-compose.1.md.in
# podman-compose is refused: its `run` has no `--quiet`, which the scripts
# use (podman-compose 1.6.0, podman_compose.py).

container_engine_fail() {
  printf 'container engine: %s\n' "$*" >&2
  exit 1
}

container_engine_exists() { # command
  local engine=$1
  command -v "$engine" >/dev/null 2>&1
}

container_engine_answers() { # command: it exists and its daemon (or machine) answers
  local engine=$1
  container_engine_exists "$engine" && "$engine" info >/dev/null 2>&1
}

container_engine_is_podman() { # command: Podman, also under the name docker
  local engine=$1
  case $("$engine" --version 2>/dev/null) in
    *[Pp]odman*) return 0 ;;
    *) return 1 ;;
  esac
}

case ${CONTAINER_ENGINE:-} in
  docker | podman)
    container_engine_exists "$CONTAINER_ENGINE" ||
      container_engine_fail "CONTAINER_ENGINE=$CONTAINER_ENGINE, but there is no $CONTAINER_ENGINE command"
    container_engine_answers "$CONTAINER_ENGINE" ||
      container_engine_fail "CONTAINER_ENGINE=$CONTAINER_ENGINE, but \`$CONTAINER_ENGINE info\` fails: is it running? (Podman on macOS: podman machine start)"
    ;;
  '')
    if container_engine_answers docker; then
      CONTAINER_ENGINE=docker
    elif container_engine_answers podman; then
      CONTAINER_ENGINE=podman
    else
      container_engine_fail "neither \`docker info\` nor \`podman info\` answers: install and start Docker or Podman (Podman on macOS: podman machine start), or set CONTAINER_ENGINE=docker or CONTAINER_ENGINE=podman"
    fi
    ;;
  *)
    container_engine_fail "CONTAINER_ENGINE=$CONTAINER_ENGINE: must be docker or podman"
    ;;
esac

ENGINE=("$CONTAINER_ENGINE")
if [[ $CONTAINER_ENGINE == docker ]] && container_engine_is_podman docker; then
  CONTAINER_ENGINE=podman # the command stays `docker`
fi
ENGINE_COMPOSE=("${ENGINE[@]}" compose)

if [[ $CONTAINER_ENGINE == podman ]]; then
  # no "Executing external compose provider" line on stderr at each call
  # (PODMAN_COMPOSE_WARNING_LOGS, podman-compose(1))
  export PODMAN_COMPOSE_WARNING_LOGS=false
  CONTAINER_ENGINE_COMPOSE_VERSION=$("${ENGINE_COMPOSE[@]}" version 2>/dev/null) ||
    container_engine_fail "\`${ENGINE_COMPOSE[*]}\` found no Compose provider: install Docker Compose (macOS: brew install docker-compose)"
  case $CONTAINER_ENGINE_COMPOSE_VERSION in
    *[Dd]ocker\ [Cc]ompose*) ;;
    *)
      container_engine_fail "\`${ENGINE_COMPOSE[*]}\` uses a provider that is not Docker Compose ($(head -n 1 <<<"$CONTAINER_ENGINE_COMPOSE_VERSION")): install Docker Compose (macOS: brew install docker-compose), or name it in PODMAN_COMPOSE_PROVIDER"
      ;;
  esac
  unset CONTAINER_ENGINE_COMPOSE_VERSION
else
  "${ENGINE_COMPOSE[@]}" version >/dev/null 2>&1 ||
    container_engine_fail "\`${ENGINE_COMPOSE[*]}\` fails: install the Docker Compose plugin"
fi

# Prints the --user value for a container that writes into a folder of
# this machine mounted into it (a bind mount), so that the files belong to
# whoever runs the script.
#   Docker, and Podman with a root service: this user's ids.
#   Rootless Podman: 0:0. There, root in the container is the user who
#   runs Podman, and any other id is one of that user's subordinate ids,
#   which cannot write into the user's folders (podman-run(1), --userns).
engine_bind_mount_user() {
  if [[ $CONTAINER_ENGINE == podman &&
    $("${ENGINE[@]}" info --format '{{.Host.Security.Rootless}}' 2>/dev/null) == true ]]; then
    echo 0:0
  else
    echo "$(id -u):$(id -g)"
  fi
}
