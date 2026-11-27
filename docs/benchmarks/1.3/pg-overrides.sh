# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Sourced by bench-pg.sh after scripts/participants.sh: each component gets
# its own new PostgreSQL database (per bench part) instead of a SQLite file.
PG_SERVER=${BENCH_PG_URL:-postgresql://portitor@localhost:5432/postgres?sslmode=disable}
PG_RUN=$(openssl rand -hex 3)
pg_admin() { psql "$PG_SERVER" -v ON_ERROR_STOP=1 -qAt -c "$1"; }
pg_name() { local n="bench_${PG_RUN}_$(basename "$WORK")_$1"; echo "${n//-/_}"; }
pg_url() { # component: creates its database, prints its URL
  local db; db=$(pg_name "$1")
  pg_admin "CREATE DATABASE $db" >/dev/null || { echo "create $db failed" >&2; return 1; }
  sed -E "s#^(postgres(ql)?://[^/?]*)(/[^?]*)?#\\1/$db#" <<<"$PG_SERVER"
}
pg_drop_all() {
  local db
  for db in $(pg_admin "SELECT datname FROM pg_database WHERE datname LIKE 'bench_${PG_RUN}_%'"); do
    pg_admin "DROP DATABASE IF EXISTS $db WITH (FORCE)" >/dev/null || true
  done
}
pg_size() { pg_admin "SELECT pg_database_size('$(pg_name "$1")')"; }
start_issuer() { # [more VAR=value settings]: the authority's portitor-issuer, with a new key and database
  local dir=$WORK/issuer
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/issuer-key.pem" 2>/dev/null
  openssl rand -hex 32 >"$dir/management.key"
  env PORTITOR_PARTICIPANT_ID="$AUTHORITY" \
    PORTITOR_BIND="$ISSUER_ADDRESS" \
    PORTITOR_ISSUER_SERVICE_URL="http://$ISSUER_ADDRESS/issuer" \
    PORTITOR_ISSUER_KEY_FILE="$dir/issuer-key.pem" \
    PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE="$ISSUER_CREDENTIALS" \
    PORTITOR_DATABASE_URL="$(pg_url issuer)" \
    PORTITOR_MANAGEMENT_BIND="$ISSUER_MANAGEMENT" \
    PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/management.key" \
    PORTITOR_ALLOW_HTTP=true \
    "$@" "$ISSUER" >"$WORK/issuer.log" 2>&1 &
  PIDS+=($!)
}
start_identity() { # name did address management-address
  local name=$1 did=$2 address=$3 management_address=$4
  local dir=$WORK/$name
  mkdir -p "$dir"
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$dir/identity-key.pem" 2>/dev/null
  openssl rand -hex 32 >"$dir/identity-management.key"
  openssl rand -hex 32 >"$dir/sts-client-secret" # shared with its connector
  PORTITOR_PARTICIPANT_ID=$did \
  PORTITOR_BIND=$address \
  PORTITOR_CREDENTIAL_SERVICE_URL=http://$address/cs \
  PORTITOR_IDENTITY_KEY_FILE="$dir/identity-key.pem" \
  PORTITOR_DATABASE_URL="$(pg_url $name-identity)" \
  PORTITOR_MANAGEMENT_BIND=$management_address \
  PORTITOR_MANAGEMENT_API_KEY_FILE="$dir/identity-management.key" \
  PORTITOR_STS_CLIENT_SECRET_FILE="$dir/sts-client-secret" \
  PORTITOR_TRUSTED_ISSUERS=$AUTHORITY \
  PORTITOR_ALLOW_HTTP=true \
    "$IDENTITY" >"$WORK/$name-identity.log" 2>&1 &
  PIDS+=($!)
}
start_connector() { # name did address identity-address [more VAR=value settings]
  local name=$1 did=$2 address=$3 identity_address=$4
  shift 4 # the rest: the more VAR=value settings
  env PORTITOR_PARTICIPANT_ID="$did" \
    PORTITOR_BIND="$address" \
    PORTITOR_PUBLIC_URL="http://$address" \
    PORTITOR_DATABASE_URL="$(pg_url $name-connector)" \
    PORTITOR_STS_URL="http://$identity_address/sts/token" \
    PORTITOR_STS_CLIENT_SECRET_FILE="$WORK/$name/sts-client-secret" \
    PORTITOR_TRUSTED_ISSUERS="$AUTHORITY" \
    PORTITOR_ALLOW_HTTP=true \
    "$@" "$CONNECTOR" >"$WORK/$name-connector.log" 2>&1 &
  PIDS+=($!)
}
