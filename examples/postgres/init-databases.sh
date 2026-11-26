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
set -eu

for name in ${PORTITOR_DATABASES:?}; do
  file=/run/secrets/$(printf '%s' "$name" | tr _ -)-database-password
  password=$(cat "$file")
  # psql variables, quoted by psql (:"name" an identifier, :'password' a literal)
  psql -v ON_ERROR_STOP=1 -q --username "$POSTGRES_USER" --dbname postgres \
    -v name="$name" -v password="$password" <<'SQL'
CREATE ROLE :"name" LOGIN PASSWORD :'password';
CREATE DATABASE :"name" OWNER :"name";
REVOKE ALL ON DATABASE :"name" FROM PUBLIC;
SQL
  echo "portitor: database and role $name"
done
