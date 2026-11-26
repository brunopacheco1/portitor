# shellcheck shell=bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# The PostgreSQL server of a scenario (ADR 0029): one container of the
# official postgres image, one database and role per Portitor component,
# made by examples/postgres/init-databases.sh. Sourced, not run, by the
# scenarios' run.sh.
#
# write_database_secrets DIR NAME...  writes into DIR:
#   ca.pem                   a test CA, made for this run (its key is deleted)
#   server.pem, server.key   the server's certificate, for the host name
#                            "database", signed by that CA
#   admin-password           the password of the server's superuser
#   <name>-password          one password per component, NAME with "_"
#                            written "-" (provider_connector ->
#                            provider-connector-password)
# The components connect with TLS and check the certificate and the host
# name (sslmode=verify-full, sslrootcert=ca.pem); the passwords are only
# in files, mounted as Compose secrets, never in a URL.

write_database_secrets() { # dir name...
  local dir=$1 name
  shift
  mkdir -p "$dir"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 7 \
    -subj "/CN=Portitor demo database CA" \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
    -keyout "$dir/ca.key" -out "$dir/ca.pem" 2>/dev/null
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=database \
    -keyout "$dir/server.key" -out "$dir/server.csr" 2>/dev/null
  printf '%s\n' subjectAltName=DNS:database basicConstraints=critical,CA:FALSE \
    keyUsage=critical,digitalSignature extendedKeyUsage=serverAuth >"$dir/server.ext"
  openssl x509 -req -in "$dir/server.csr" -CA "$dir/ca.pem" -CAkey "$dir/ca.key" \
    -set_serial "0x$(openssl rand -hex 8)" -days 7 -extfile "$dir/server.ext" \
    -out "$dir/server.pem" 2>/dev/null
  rm -f "$dir/ca.key" "$dir/server.csr" "$dir/server.ext"
  openssl rand -hex 32 >"$dir/admin-password"
  for name in "$@"; do
    openssl rand -hex 32 >"$dir/${name//_/-}-password"
  done
}
