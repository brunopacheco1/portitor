<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0021: Gaia-X onboarding in the identity component

- Status: **Accepted** (2026-11-15, by the maintainer)
- Date: 2026-11-15
- Scope: release 0.8. Extends ADR 0015 (identity component).
- Decisions already taken by the maintainer (2026-11-15, 0.8 plan):
  keep the identity's P-256 key and publish its X.509 chain with `x5u`;
  renew the certificate on the same key, key rotation stays in 0.10; talk
  to the clearing house with its REST API, not OpenID4VC; no test against
  the real Gaia-X Lab in 0.8.
- Update (2026-11-15): "key rotation stays in 0.10" was written before
  the hardening release was split into 0.10 to 0.13 (maintainer,
  2026-11-15). Key custody and key rotation are now investigated in
  0.11 (ROADMAP, [TD-05](../RISKS_AND_TECHNICAL_DEBTS.md#td-05)).

## Context

The Gaia-X Trust Framework gives a participant a compliance credential
once a Gaia-X Digital Clearing House (GXDCH) has checked a presentation
of three credentials (Compliance Document 3.1.0, "Process", steps A to F):

| Credential | Signed by | Gaia-X type |
| --- | --- | --- |
| Legal person: name, addresses, registration number | the participant (or a signer it chooses) | `gx:LegalPerson` |
| Acceptance of the Gaia-X terms and conditions | the participant | `gx:Issuer` (`gaiaxTermsAndConditions`, a SHA-256 hash) |
| Legal registration number (VAT, LEI, EORI, ...) | a GXDCH notary | `gx:VatID`, `gx:LeiCode`, `gx:EORI`, ... |

Facts this ADR relies on (checked 2026-11-15; details in
[the 0.8 spike](../spikes/0.8-gaia-x-compliance.md)):

- Gaia-X credentials are VC Data Model 2.0 as VC-JWT, `typ: vc+jwt`,
  `cty: vc`; presentations `vp+jwt`, `cty: vp` ([ICAM 25.11, Gaia-X
  credentials](https://docs.gaia-x.eu/technical-committee/identity-credential-access-management/25.11/gaia-x_credentials/)).
- The signer's JWK in its `did:web` document MUST carry `x5c` or `x5u`,
  and the chain must end at a Gaia-X trust anchor ([technical
  compatibility](https://docs.gaia-x.eu/technical-committee/architecture-document/25.11/gaia-x_technical_compatibility_specifications/),
  [trust anchors](https://docs.gaia-x.eu/policy-rules-committee/compliance-document/25.10/Gaia-X_Trust_Anchors/)).
- The compliance service (gx-compliance v2.16.0) takes the presentation
  at `POST {base}/api/credential-offers/standard-compliance?vcid=<id>`
  with `Content-Type: application/vp+jwt` and answers a VC-JWT of type
  `gx:ParticipantComplianceCredential`. ES256 keys work if the JWK has
  `alg`. The answer is valid for at most 90 days, less if the service's
  certificate expires earlier.
- Gaia-X supports VC-API style REST and OpenID4VC, not DCP
  ([formats and protocols](https://gaia-x.gitlab.io/technical-committee/architecture-working-group/architecture-document/credential_formats_protocols/)).
  The Compliance Dispatcher's wallet flow (OpenID4VP and OpenID4VCI) is
  announced, not released ([Gaia-X, 2026-07-24](https://gaia-x.eu/from-back-end-to-wallet-oid4vp-and-oid4vci-in-the-gaia-x-compliance-dispatcher/)).

## Decision

### The key and its certificate

- The identity keeps its one P-256 key (ADR 0015).
- `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` (optional): a PEM file, the
  certificate for the identity key first, then its chain. When set:
  - at start the identity refuses to run if the first certificate's
    public key is not the identity key;
  - it serves the file at `GET /.well-known/x509CertificateChain.pem`
    below its public base (the path Gaia-X's own services use);
  - the DID document's JWK gets `x5u` (RFC 7517 section 4.6) with that
    URL, and `alg: ES256` (RFC 7517 section 4.4; Gaia-X needs it).
- A renewed certificate is picked up without a restart: the identity
  reads the file again when it changes. A new file whose first
  certificate is for another key is refused and logged; the old chain
  stays. Getting the certificate (ACME, eIDAS provider) is the
  operator's job (DEPLOYING).
- `alg: ES256` is added to the JWK in every case, with or without a
  chain. It is optional in RFC 7517 and does not change DCP.

### Onboarding

- Gaia-X is on when `PORTITOR_GAIAX_COMPLIANCE_URL` is set: the base URL
  of a compliance service (for example `https://compliance.lab.gaia-x.eu/v2`).
  It needs `PORTITOR_IDENTITY_CERTIFICATE_CHAIN`,
  `PORTITOR_GAIAX_PARTICIPANT_FILE` and `PORTITOR_GAIAX_COMPLIANCE_DID`,
  the service's `did:web` DID; the identity does not start without them.
  An answer issued by any other DID is refused before that DID is
  resolved (0.8 review: a wrong or hijacked URL must not hand the
  identity a credential from an issuer verifiers do not trust).
- The participant file (JSON) holds only Gaia-X terms, copied from the
  Gaia-X ontology the service expects:
  - `context`: the Gaia-X JSON-LD context IRI, for example
    `https://w3id.org/gaia-x/development#`;
  - `legalPerson`: the `credentialSubject` claims of the `gx:LegalPerson`
    credential, without `id` and without `gx:registrationNumber` (the
    identity fills both);
  - `termsAndConditions`: the `gaiaxTermsAndConditions` hash;
  - `registrationNumberCredential`: the path of the registration number
    VC-JWT that a GXDCH notary issued. Calling the notary from Portitor
    is later work; the operator gets this credential once (DEPLOYING).
- The identity signs the `gx:LegalPerson` and `gx:Issuer` credentials
  with its key: VC 2.0, `typ: vc+jwt`, `cty: vc`, headers `iss` (its
  DID) and `kid`, `validFrom` now, `validUntil` 90 days later, ids under
  `{public base}/credentials/gaiax/` (`legal-person`, `terms-and-conditions`).
  The public base is the URL of the DID document without `did.json`
  (and without `/.well-known` for a DID without a path):
  `https://host.example/participants/a` for
  `did:web:host.example:participants:a`, so participants on one host
  get different ids.
  It serves these two and the registration number credential, as VC-JWT,
  at their ids, so anyone can recompute their digests.
- It wraps the three in a `vp+jwt` signed with the same key and sends it
  to the compliance service. It checks the answer (signature by the
  service's `did:web` key, type `gx:ParticipantComplianceCredential`,
  every `digestSRI` equals what it sent, ADR 0022) and stores the four
  credentials in its store, like any other credential, so the
  presentation API can return them.
- When: at start if there is no valid compliance credential, then again
  once two thirds of its validity have passed. `POST /v1/gaiax/compliance`
  on the management API starts a run now; `GET /v1/gaiax/compliance`
  lists the last runs (start, end, outcome, error). A failed run keeps
  the stored credentials until they expire and logs the error; the next
  attempt follows the outbox retry policy.
- The service is a port (`GaiaxCompliance`) with an HTTP adapter, so
  OpenID4VCI can be added as a second adapter when the Compliance
  Dispatcher ships it.

### Credential types and scopes

Gaia-X types are compact IRIs (`gx:ParticipantComplianceCredential`),
but a DCP scope discriminator cannot hold a colon: the DCP TCK's
default pattern is `vc.type:(?<type>[^:]+)(?<suffix>:.+)?`, and EDC uses
the same shape. DCP leaves the scope values to the data space ("A scope
is an alias for a well-defined Presentation Definition", presentation
protocol, "Scopes"). So:

- A scope type `T` matches a credential whose `type` holds `T`, or holds
  `gx:T` while the credential's `@context` contains a Gaia-X context
  (an IRI starting with `https://w3id.org/gaia-x/`). That is the same
  IRI under JSON-LD; nothing else is expanded.
- This rule is ours. The upstream report asks the DCP project how a
  scope should name a type that is a compact IRI.

## Consequences

- An operator who wants Gaia-X needs a certificate for the identity key
  from a Gaia-X trust anchor (an eIDAS trust service provider, or EV SSL
  where the Gaia-X Registry has no alternative). For tests, the Gaia-X
  Lab accepts other certificates (gx-compliance README, "non-EV SSL
  valid"), and a local compliance service in test mode accepts any.
- The certificate expires before the key; renewing it on the same key
  causes no gap. Changing the key still breaks every signature it made,
  now including the self-signed Gaia-X credentials, so a key change also
  means a new compliance run (key rotation stays in 0.10; now
  investigated in 0.11, see the update above).
- The identity now calls an outside service on its own. It goes through
  the shared HTTP client (HTTPS only, limits, no redirects).
- Not in this ADR: Gaia-X service offering credentials, label levels 1
  to 3, calling the notary, reading trust anchors from the Gaia-X
  Registry, OpenID4VC.
