#!/usr/bin/env python3
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""TEST ONLY: a stand-in for a Gaia-X notary, for scripts/gaiax-e2e.sh.

A Gaia-X Digital Clearing House notary checks a legal registration
number and signs a credential for it (gx:VatID, gx:LeiCode, ...). A
participant gets that credential once and names it in its participant
file (ADR 0021, `registrationNumberCredential`). In the end-to-end test
the compliance service runs with `production=false`, so it accepts a
registration number credential from any issuer
(TrustedNotaryIssuerService in gx-compliance v2.16.0). This script plays
that issuer with a key made for the run. Its credentials prove nothing
and must never be used outside a test.

Two commands:

  did   KEY DID X5U OUT
        writes the notary's DID document (did:web, one JsonWebKey2020
        with alg ES256 and x5u, as a Gaia-X signer needs) to OUT
  vat   KEY DID CONTEXT CREDENTIAL_ID VAT_ID COUNTRY OUT
        writes a gx:VatID credential as VC-JWT (VC Data Model 2.0,
        typ vc+jwt, cty vc, headers iss and kid, ICAM 25.11 "Gaia-X
        credentials") to OUT; its subject id is CREDENTIAL_ID#cs

KEY is a P-256 private key in PEM (openssl genpkey). Needs the
`cryptography` package.
"""

import base64
import datetime
import json
import sys

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, utils

KEY_ID = "key-0"
VALIDITY = datetime.timedelta(days=365)


def base64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def load_key(path: str) -> ec.EllipticCurvePrivateKey:
    with open(path, "rb") as file:
        key = serialization.load_pem_private_key(file.read(), password=None)
    if not isinstance(key, ec.EllipticCurvePrivateKey) or key.curve.name != "secp256r1":
        sys.exit(f"{path}: not a P-256 private key")
    return key


def public_jwk(key: ec.EllipticCurvePrivateKey, x5u: str) -> dict:
    numbers = key.public_key().public_numbers()
    return {
        "kty": "EC",
        "crv": "P-256",
        "alg": "ES256",
        "x": base64url(numbers.x.to_bytes(32, "big")),
        "y": base64url(numbers.y.to_bytes(32, "big")),
        "x5u": x5u,
    }


def sign_es256(key: ec.EllipticCurvePrivateKey, header: dict, payload: dict) -> str:
    """A compact JWS; ES256 takes the raw r || s signature (RFC 7518 3.4)."""
    signing_input = ".".join(
        base64url(json.dumps(part, separators=(",", ":")).encode())
        for part in (header, payload)
    )
    der = key.sign(signing_input.encode(), ec.ECDSA(hashes.SHA256()))
    r, s = utils.decode_dss_signature(der)
    signature = r.to_bytes(32, "big") + s.to_bytes(32, "big")
    return f"{signing_input}.{base64url(signature)}"


def did_document(key_file: str, did: str, x5u: str, out: str) -> None:
    key = load_key(key_file)
    method = f"{did}#{KEY_ID}"
    document = {
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/suites/jws-2020/v1",
        ],
        "id": did,
        "verificationMethod": [
            {
                "id": method,
                "type": "JsonWebKey2020",
                "controller": did,
                "publicKeyJwk": public_jwk(key, x5u),
            }
        ],
        "assertionMethod": [method],
    }
    with open(out, "w", encoding="utf-8") as file:
        json.dump(document, file, indent=2)


def vat_credential(
    key_file: str,
    did: str,
    context: str,
    credential_id: str,
    vat_id: str,
    country: str,
    out: str,
) -> None:
    key = load_key(key_file)
    now = datetime.datetime.now(datetime.timezone.utc)
    credential = {
        "@context": ["https://www.w3.org/ns/credentials/v2", context],
        "id": credential_id,
        "type": ["VerifiableCredential", "gx:VatID"],
        "issuer": did,
        "validFrom": now.isoformat(timespec="seconds"),
        "validUntil": (now + VALIDITY).isoformat(timespec="seconds"),
        "credentialSubject": {
            "id": f"{credential_id}#cs",
            "gx:vatID": vat_id,
            "gx:countryCode": country,
        },
    }
    # the kid is the verification method's id: gx-compliance looks it up with ===
    header = {"alg": "ES256", "typ": "vc+jwt", "cty": "vc", "iss": did, "kid": f"{did}#{KEY_ID}"}
    with open(out, "w", encoding="utf-8") as file:
        file.write(sign_es256(key, header, credential))


def main(arguments: list[str]) -> None:
    match arguments:
        case ["did", key_file, did, x5u, out]:
            did_document(key_file, did, x5u, out)
        case ["vat", key_file, did, context, credential_id, vat_id, country, out]:
            vat_credential(key_file, did, context, credential_id, vat_id, country, out)
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
