#!/bin/sh
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# One PostgreSQL role and one database per Portitor component
# (docs/DEPLOYING.md, "Database"), for the official postgres image: mount
# it into /docker-entrypoint-initdb.d/, where the image runs it once, when
# it makes a new data folder. The scenarios' Compose files use it.
#
#   PORTITOR_DATABASES  the names, separated by spaces, for example
#                       "issuer provider_identity provider_connector";
#                       each is the name of a role and of the database it
#                       owns
# The password of <name> is in /run/secrets/<name>-database-password,
# with "_" in the name written "-" (a Compose secret).
#
# Each role owns its database and has no other right: the component makes
# its tables there at start. Nobody else may connect to it.
#
# When the server runs with TLS (ssl=on), connections over the network
# must use it: the image's "host" lines of pg_hba.conf are replaced by
#
#   hostssl all all all scram-sha-256
#
# so a connection without TLS is refused. The local socket lines stay,
# for the image's own scripts and for `docker exec ... psql`.
set -eu

for name in ${PORTITOR_DATABASES:?}; do
  file=/run/secrets/$(printf '%s' "$name" | tr _ -)-database-password
  # psql reads the password from the file itself (\set with backquotes),
  # so it is never on a command line, where other processes could read it
  # (/proc/<pid>/cmdline). psql quotes the variables: :"name" as an
  # identifier, :'password' as a literal, :'password_file' for the shell.
  psql -v ON_ERROR_STOP=1 -q --username "$POSTGRES_USER" --dbname postgres \
    -v name="$name" -v password_file="$file" <<'SQL'
\set password `cat :'password_file'`
CREATE ROLE :"name" LOGIN PASSWORD :'password';
CREATE DATABASE :"name" OWNER :"name";
REVOKE ALL ON DATABASE :"name" FROM PUBLIC;
SQL
  echo "portitor: database and role $name"
done

ssl=$(psql -tAq --username "$POSTGRES_USER" --dbname postgres -c 'SHOW ssl')
if [ "$ssl" = on ]; then
  hba="$PGDATA/pg_hba.conf"
  grep -v '^host' "$hba" > "$hba.new"
  echo 'hostssl all all all scram-sha-256' >> "$hba.new"
  mv "$hba.new" "$hba"
  echo "portitor: connections over the network only with TLS (pg_hba.conf)"
fi
