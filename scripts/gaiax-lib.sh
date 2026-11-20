# shellcheck shell=bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# What the Gaia-X end-to-end test (scripts/gaiax-e2e.sh) and the EDC
# interop test (scripts/edc-interop.sh) share: the test CA and its
# certificates, the test notary, a Gaia-X participant's files, and the
# calls to a Portitor identity's Gaia-X onboarding (ADR 0021).
#
# Source it; do not run it. The caller sets RUN (its run/ directory) and
# defines fail(). Test-only: no real Gaia-X trust anchor (docs/TESTING.md).
# Needs curl, jq, openssl, and python3 with the `cryptography` package.

# who is who in both tests (the "tls" service of each Compose file answers
# for these names)
COMPLIANCE_DID=did:web:compliance.gaiax.test
NOTARY_DID=did:web:notary.gaiax.test
# the Gaia-X ontology the compliance service validates against
GAIAX_CONTEXT='https://w3id.org/gaia-x/development#'
# the Gaia-X terms and conditions hash gx-compliance v2.16.0 accepts
# (IssuersHaveTermsAndConditionsFilter)
TERMS_AND_CONDITIONS=4bd7554097444c960292b4726c2efa1373485e8a5565d94d41195214c5e0ceb3

# --- the test CA and the certificates ---------------------------------------------

new_ec_key() { # file: a P-256 key
  local file=$1
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$file" 2>/dev/null
}

issue_certificate() { # key common-name dns-names(comma-separated) out: a leaf from the test CA
  local key=$1 name=$2 dns=$3 out=$4 extensions
  extensions=$(printf 'subjectAltName=%s\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth,clientAuth\n' \
    "DNS:${dns//,/,DNS:}")
  openssl req -new -key "$key" -subj "/CN=$name/O=Portitor Gaia-X test (test only)" 2>/dev/null |
    openssl x509 -req -CA "$RUN/ca/ca.pem" -CAkey "$RUN/ca-key.pem" -set_serial "0x$(openssl rand -hex 16)" \
      -days 30 -extfile <(echo "$extensions") -out "$out" 2>/dev/null
}

chain() { # leaf out: the leaf, then the test CA
  local leaf=$1 out=$2
  cat "$leaf" "$RUN/ca/ca.pem" >"$out"
}

# names(comma-separated): a test CA made for this run and, from it, the TLS
# certificate of the public names (the "tls" service), the compliance
# service's certificate (RSA, PS256) and the test notary's
gaiax_test_ca() {
  local names=$1
  mkdir -p "$RUN/ca" "$RUN/tls" "$RUN/compliance" "$RUN/notary/www/.well-known" "$RUN/notary/www/credentials"
  new_ec_key "$RUN/ca-key.pem"
  openssl req -x509 -key "$RUN/ca-key.pem" -subj "/CN=Portitor Gaia-X test CA (test only)" -days 30 \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
    -out "$RUN/ca/ca.pem" 2>/dev/null
  # the TLS certificate of every public name (config/tls.conf)
  new_ec_key "$RUN/tls/server-key.pem"
  issue_certificate "$RUN/tls/server-key.pem" gaiax.test "$names" "$RUN/tls/leaf.pem"
  chain "$RUN/tls/leaf.pem" "$RUN/tls/server.pem"
  # the compliance service signs with PS256: an RSA key; its DID document
  # serves the chain (x5u), and its answers are valid at most until it expires
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$RUN/compliance/private-key.pem" 2>/dev/null
  issue_certificate "$RUN/compliance/private-key.pem" compliance.gaiax.test compliance.gaiax.test \
    "$RUN/compliance/leaf.pem"
  chain "$RUN/compliance/leaf.pem" "$RUN/compliance/certificate-chain.pem"
  # the test notary
  new_ec_key "$RUN/notary/key.pem"
  issue_certificate "$RUN/notary/key.pem" notary.gaiax.test notary.gaiax.test "$RUN/notary/leaf.pem"
  chain "$RUN/notary/leaf.pem" "$RUN/notary/www/.well-known/x509CertificateChain.pem"
}

# the test notary's DID document (scripts/gaiax_test_notary.py), a static
# file the "tls" service serves
gaiax_test_notary() {
  python3 scripts/gaiax_test_notary.py did "$RUN/notary/key.pem" "$NOTARY_DID" \
    https://notary.gaiax.test/.well-known/x509CertificateChain.pem "$RUN/notary/www/.well-known/did.json"
}

# --- a participant's keys, credentials and participant file -------------------------

participant() { # name: keys and secrets of a Portitor participant, in $RUN/<name>/
  local name=$1
  mkdir -p "$RUN/$name"
  new_ec_key "$RUN/$name/identity-key.pem"
  openssl rand -hex 32 >"$RUN/$name/identity-management.key"
  openssl rand -hex 32 >"$RUN/$name/connector-management.key"
  openssl rand -hex 32 >"$RUN/$name/sts-client-secret"
}

# name host legal-name vat-id: what ADR 0021 needs, in $RUN/<name>/, for
# the key $RUN/<name>/identity-key.pem: its certificate chain, a
# registration number credential from the test notary, the participant file
gaiax_participant() {
  local name=$1 host=$2 legal_name=$3 vat_id=$4
  local dir=$RUN/$name
  local credential_id=https://$host/credentials/gaiax/registration-number
  # the certificate for the identity key, then the CA
  issue_certificate "$dir/identity-key.pem" "$host" "$host" "$dir/identity-certificate.pem"
  chain "$dir/identity-certificate.pem" "$dir/certificate-chain.pem"
  # the registration number credential, from the notary
  python3 scripts/gaiax_test_notary.py vat "$RUN/notary/key.pem" "$NOTARY_DID" "$GAIAX_CONTEXT" \
    "$credential_id" "$vat_id" LU "$dir/registration-number.jwt"
  # the participant file (ADR 0021): Gaia-X terms only
  jq -n --arg context "$GAIAX_CONTEXT" --arg name "$legal_name" --arg terms "$TERMS_AND_CONDITIONS" '{
    context: $context,
    legalPerson: {
      name: $name,
      "gx:legalAddress": {type: "gx:Address", "gx:countryCode": "LU",
        "vcard:street-address": "1 Rue du Test", "vcard:locality": "Luxembourg",
        "vcard:postal-code": "1111"},
      "gx:headquartersAddress": {type: "gx:Address", "gx:countryCode": "LU",
        "vcard:street-address": "1 Rue du Test", "vcard:locality": "Luxembourg",
        "vcard:postal-code": "1111"}
    },
    termsAndConditions: $terms,
    registrationNumberCredential: "/config/registration-number.jwt"
  }' >"$dir/participant.json"
  echo "$name: $host, $legal_name, VAT $vat_id"
}

# --- a Portitor identity's Gaia-X onboarding (its management API) -------------------

# In the three functions below: url is the identity's management API,
# key-file holds its API key, who names the participant in messages.

gaiax_start_compliance_run() { # url key-file who: POST /v1/gaiax/compliance; 409: a run is on already
  local url=$1 key_file=$2 who=$3 status
  status=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$url/v1/gaiax/compliance" \
    -H "authorization: Bearer $(cat "$key_file")")
  [[ $status == 202 || $status == 409 ]] || fail "$who's compliance run: HTTP $status"
}

gaiax_until_compliance_run_ended() { # url key-file who: prints the newest run once it is no longer RUNNING
  local url=$1 key_file=$2 who=$3 runs outcome=
  for _ in $(seq 1 150); do
    runs=$(curl -sS --fail-with-body "$url/v1/gaiax/compliance" \
      -H "authorization: Bearer $(cat "$key_file")") || fail "read $who's compliance runs"
    outcome=$(jq -r '.[0].outcome // empty' <<<"$runs")
    if [[ -n $outcome && $outcome != RUNNING ]]; then
      jq -c '.[0]' <<<"$runs"
      return 0
    fi
    sleep 1
  done
  fail "$who's compliance run is still ${outcome:-not started}"
}

gaiax_has_compliance_credential() { # url key-file: true once the identity holds one from the compliance service
  local url=$1 key_file=$2
  curl -sS --fail-with-body "$url/v1/credentials" -H "authorization: Bearer $(cat "$key_file")" |
    jq -e --arg issuer "$COMPLIANCE_DID" \
      'any(.[]; (.types | index("gx:ParticipantComplianceCredential")) and .issuer == $issuer)' \
      >/dev/null
}
