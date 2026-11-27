# shellcheck shell=bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Databases for the scripts that run Portitor as processes on this machine
# (scenarios/two-party/run-replicas.sh, scenarios/two-data-spaces/run.sh).
# Sourced, not run. With PORTITOR_TEST_DATABASE_URL, the server the store
# tests use (docs/TESTING.md, "PostgreSQL"), each component gets a new
# database there, dropped at the end; without it, a SQLite file.
#
#   PORTITOR_TEST_DATABASE_URL       a PostgreSQL server whose user may
#                                    create databases (needs psql)
#   PORTITOR_TEST_DATABASE_PASSWORD  that user's password, if it needs one
#
# The script that sources it sets WORK (a scratch directory) and calls
# test_databases_drop when it ends. Functions:
#   test_database_create NAME          makes the database of NAME
#   test_database_settings NAME [FILE] sets DATABASE, an array of VAR=value
#                                      settings: NAME's database on the
#                                      server, or the SQLite file FILE
#   database_for NAME FILE             both, for a script that runs one
#                                      process per component
#   test_databases_drop                drops every database made here

TEST_DATABASES=()
TEST_DATABASE_RUN=$(openssl rand -hex 4)
DATABASE=()

test_database_admin() { # sql: runs it on the server of PORTITOR_TEST_DATABASE_URL
  PGPASSWORD=${PORTITOR_TEST_DATABASE_PASSWORD:-} PGCONNECT_TIMEOUT=10 \
    psql "$PORTITOR_TEST_DATABASE_URL" -v ON_ERROR_STOP=1 -qAt -c "$1"
}

test_database_name() { # component name -> a database name for this run
  local name=${1//-/_}
  echo "portitor_${TEST_DATABASE_RUN}_${name}"
}

test_database_create() { # name
  local db
  db=$(test_database_name "$1")
  command -v psql >/dev/null || {
    echo "PORTITOR_TEST_DATABASE_URL is set: this needs psql, the PostgreSQL client" >&2
    return 1
  }
  test_database_admin "CREATE DATABASE $db" >/dev/null || {
    echo "could not create the database $db" >&2
    return 1
  }
  TEST_DATABASES+=("$db")
}

test_database_settings() { # name [sqlite-file]
  local name=$1 file=${2-} db url
  if [[ -z ${PORTITOR_TEST_DATABASE_URL:-} ]]; then
    DATABASE=(PORTITOR_DB_FILE="$file")
    return
  fi
  db=$(test_database_name "$name")
  # the server's URL, with the database's name as its path
  url=$(sed -E "s#^(postgres(ql)?://[^/?]*)(/[^?]*)?#\\1/$db#" <<<"$PORTITOR_TEST_DATABASE_URL")
  DATABASE=(PORTITOR_DATABASE_URL="$url")
  if [[ -n ${PORTITOR_TEST_DATABASE_PASSWORD:-} ]]; then
    printf '%s\n' "$PORTITOR_TEST_DATABASE_PASSWORD" >"$WORK/database-password"
    DATABASE+=(PORTITOR_DATABASE_PASSWORD_FILE="$WORK/database-password")
  fi
}

database_for() { # name sqlite-file
  if [[ -n ${PORTITOR_TEST_DATABASE_URL:-} ]]; then test_database_create "$1" || return 1; fi
  test_database_settings "$1" "$2"
}

test_databases_drop() {
  local db
  for db in "${TEST_DATABASES[@]}"; do
    test_database_admin "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null || true
  done
  TEST_DATABASES=()
}
