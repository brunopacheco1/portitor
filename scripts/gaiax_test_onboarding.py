#!/usr/bin/env python3
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
"""TEST ONLY: Gaia-X onboarding for a participant whose wallet has no
Gaia-X support, for scenarios/edc-interop/run.sh.

A Portitor identity does its Gaia-X onboarding itself (ADR 0021). The
EDC IdentityHub (v1.0.0-RC8) has no Gaia-X onboarding, so in the EDC
interop test this script plays the operator's tool: it signs, with the
EDC participant's key, the same credentials and presentation a Portitor
identity signs (crates/portitor-dcp/src/gaiax_sign.rs), and writes what
the IdentityHub's Identity API needs to store the result.

Commands:

  public-jwk   KEY X5U
        prints the public JWK of KEY with alg ES256 and x5u, as a Gaia-X
        signer's DID document needs (ADR 0021)
  private-jwk  KEY KID
        prints the private JWK of KEY with kid KID, the form in which the
        IdentityHub keeps a private key in its vault (KeyPairServiceImpl)
  sign         KEY DID KID PARTICIPANT_FILE REGISTRATION_JWT BASE OUT_DIR
        signs the gx:LegalPerson and gx:Issuer credentials of the
        participant file (the format of PORTITOR_GAIAX_PARTICIPANT_FILE),
        with ids BASE/legal-person and BASE/terms-and-conditions, writes
        them and a copy of the registration number credential to OUT_DIR
        (legal-person, terms-and-conditions, registration-number), and
        prints the presentation of the three (vp+jwt) for the compliance
        service
  manifest     JWT PARTICIPANT_CONTEXT_ID RESOURCE_ID
        prints the IdentityHub's VerifiableCredentialManifest (Identity
        API, POST /v1/participants/{id}/credentials) for the VC 2.0 JWT
        in the file JWT

KEY is a P-256 private key in PEM (openssl genpkey). Needs the
`cryptography` package.
"""

import base64
import datetime
import json
import os
import sys

from cryptography.hazmat.primitives.asymmetric import ec

# the signing helpers of the test notary, next to this file
from gaiax_test_notary import base64url, load_key, public_jwk, sign_es256

VC2_CONTEXT = "https://www.w3.org/ns/credentials/v2"
CONTEXT = "@context"  # the JSON-LD member that names the contexts
# how long the credentials and the presentation are valid: 90 days, as a
# Portitor identity writes them (VALIDITY_SECONDS in gaiax_sign.rs)
VALIDITY = datetime.timedelta(days=90)


def xsd_date_time(moment: datetime.datetime) -> str:
    """`2026-11-12T12:00:00Z`, as portitor_time::xsd_date_time writes it."""
    return moment.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def decode_payload(jwt: str) -> dict:
    """The payload of a compact JWS, not verified."""
    payload = jwt.split(".")[1]
    return json.loads(base64.urlsafe_b64decode(payload + "=" * (-len(payload) % 4)))


def private_jwk(key: ec.EllipticCurvePrivateKey, kid: str) -> dict:
    numbers = key.private_numbers()
    public = numbers.public_numbers
    return {
        "kty": "EC",
        "crv": "P-256",
        "kid": kid,
        "x": base64url(public.x.to_bytes(32, "big")),
        "y": base64url(public.y.to_bytes(32, "big")),
        "d": base64url(numbers.private_value.to_bytes(32, "big")),
    }


def credential(credential_id: str, did: str, contexts: list, kind: str, subject: dict, period: dict) -> dict:
    """A VC 2.0 credential of one Gaia-X type (`credential` in gaiax_sign.rs)."""
    return {
        CONTEXT: [VC2_CONTEXT, *contexts],
        "id": credential_id,
        "type": ["VerifiableCredential", kind],
        "issuer": did,
        **period,
        "credentialSubject": subject,
    }


def sign(
    key_file: str,
    did: str,
    kid: str,
    participant_file: str,
    registration_file: str,
    base: str,
    out_dir: str,
) -> None:
    key = load_key(key_file)
    with open(participant_file, encoding="utf-8") as file:
        participant = json.load(file)
    with open(registration_file, encoding="utf-8") as file:
        registration_jwt = file.read().strip()
    # the legal person links to the subject of the notary's credential
    registration_subject = decode_payload(registration_jwt)["credentialSubject"]["id"]
    context = participant["context"]
    contexts = context if isinstance(context, list) else [context]
    now = datetime.datetime.now(datetime.timezone.utc)
    period = {"validFrom": xsd_date_time(now), "validUntil": xsd_date_time(now + VALIDITY)}

    legal_id = f"{base}/legal-person"
    legal_subject = {"id": f"{legal_id}#cs", **participant["legalPerson"]}
    legal_subject["gx:registrationNumber"] = {"id": registration_subject}
    legal = credential(legal_id, did, contexts, "gx:LegalPerson", legal_subject, period)

    terms_id = f"{base}/terms-and-conditions"
    terms_subject = {"id": f"{terms_id}#cs", "gaiaxTermsAndConditions": participant["termsAndConditions"]}
    terms = credential(terms_id, did, contexts, "gx:Issuer", terms_subject, period)

    # the kid is the verification method's id: gx-compliance looks it up with ===
    vc_header = {"alg": "ES256", "typ": "vc+jwt", "cty": "vc", "iss": did, "kid": kid}
    legal_jwt = sign_es256(key, vc_header, legal)
    terms_jwt = sign_es256(key, vc_header, terms)

    os.makedirs(out_dir, exist_ok=True)
    for name, jwt in (
        ("legal-person", legal_jwt),
        ("terms-and-conditions", terms_jwt),
        ("registration-number", registration_jwt),
    ):
        with open(os.path.join(out_dir, name), "w", encoding="utf-8") as file:
            file.write(jwt)

    # `presentation` and `envelope` in gaiax_sign.rs
    presentation = {
        CONTEXT: [VC2_CONTEXT],
        "type": "VerifiablePresentation",
        "verifiableCredential": [
            {
                CONTEXT: VC2_CONTEXT,
                "id": f"data:application/vc+jwt,{jwt}",
                "type": "EnvelopedVerifiableCredential",
            }
            for jwt in (legal_jwt, terms_jwt, registration_jwt)
        ],
        "issuer": did,
        **period,
    }
    vp_header = {"alg": "ES256", "typ": "vp+jwt", "cty": "vp", "iss": did, "kid": kid}
    print(sign_es256(key, vp_header, presentation))


def instant(text: str) -> str:
    """An xsd:dateTime as the UTC instant the IdentityHub reads (java.time.Instant)."""
    return xsd_date_time(datetime.datetime.fromisoformat(text))


def manifest(jwt_file: str, participant_context_id: str, resource_id: str) -> None:
    """The fields of VerifiableCredentialManifest, VerifiableCredentialContainer
    and VerifiableCredential (EDC IdentityHub and Connector v1.0.0-RC8)."""
    with open(jwt_file, encoding="utf-8") as file:
        jwt = file.read().strip()
    payload = decode_payload(jwt)
    issuer = payload["issuer"]
    subject = payload["credentialSubject"]
    described = {
        CONTEXT: payload[CONTEXT],
        "id": payload["id"],
        "type": payload["type"],
        "issuer": {"id": issuer["id"] if isinstance(issuer, dict) else issuer},
        "issuanceDate": instant(payload["validFrom"]),
        "credentialSubject": subject if isinstance(subject, list) else [subject],
        "dataModelVersion": "V_2_0",
    }
    if "validUntil" in payload:
        described["expirationDate"] = instant(payload["validUntil"])
    document = {
        "id": resource_id,
        "participantContextId": participant_context_id,
        "verifiableCredentialContainer": {
            "rawVc": jwt,
            "format": "VC2_0_JOSE",
            "credential": described,
        },
    }
    print(json.dumps(document))


def main(arguments: list[str]) -> None:
    match arguments:
        case ["public-jwk", key_file, x5u]:
            print(json.dumps(public_jwk(load_key(key_file), x5u)))
        case ["private-jwk", key_file, kid]:
            print(json.dumps(private_jwk(load_key(key_file), kid)))
        case ["sign", key_file, did, kid, participant_file, registration_file, base, out_dir]:
            sign(key_file, did, kid, participant_file, registration_file, base, out_dir)
        case ["manifest", jwt_file, participant_context_id, resource_id]:
            manifest(jwt_file, participant_context_id, resource_id)
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
