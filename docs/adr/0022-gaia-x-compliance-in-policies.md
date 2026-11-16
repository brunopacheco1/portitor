<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0022: Gaia-X compliance in policies

- Status: **Accepted** (2026-11-15, by the maintainer)
- Date: 2026-11-15
- Scope: release 0.8. Extends ADR 0016 (DSP caller authentication) and
  ADR 0019 (credential constraints).
- Decisions already taken by the maintainer (2026-11-15, 0.8 plan): tie
  the compliance credential to the caller with Gaia-X's own integrity
  check; test Gaia-X between Portitor connectors; the EDC interop test
  stays without Gaia-X.

## Context

A provider wants to say "only participants with a Gaia-X compliance
credential". The credential the clearing house issues
(`gx:ParticipantComplianceCredential`, see ADR 0021) is not about the
participant's DID: its subject `id` is `<credential id>#cs`. It names the
credentials it checked in `gx:compliantCredentials`, each with
`digestSRI` = `sha256-` + hex(SHA-256(JCS(credential))), RFC 8785 JCS,
the integrity check Gaia-X documents in gx-compliance
`docs/api-usage.md` ("Credential integrity check"). It has no
`credentialStatus`.

What the specifications and peers say (checked 2026-11-15):

- DCP v1.0.1, presentation protocol, "Presentation Validation": the
  holder binding (`credentialSubject.id` = the presenter's DID) is
  required only "if the specific semantics of a data space and
  credentials require cryptographic holder binding". Revocation is
  checked only "if the Verifiable Credential contains a revocation
  mechanism".
- Portitor's verifier (ADR 0016) applies the binding to every credential.
- EDC 1.0.0-RC7 applies it to every credential of a DCP presentation
  (`HasValidSubjectIds` in `VerifiableCredentialValidationServiceImpl`),
  so EDC refuses the Gaia-X compliance credential.
- No specification we follow (DCP, Gaia-X ICAM, the Gaia-X Trust
  Protocol) says how to present a Gaia-X compliance credential over DCP.
  Asked of the DCP project: https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278

## Decision

### Trust

- `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`: the DIDs of the compliance
  services the connector trusts, for example
  `did:web:compliance.lab.gaia-x.eu:v2`. Separate from
  `PORTITOR_TRUSTED_ISSUERS`: a compliance service is not a data space
  issuer, and a participant's self-signed credentials are trusted only
  through it.
- Reading this list from the Gaia-X Registry
  (`/api/trusted-issuers/compliance`) is later work.

### Checking a compliance credential

These checks are for credentials of type
`gx:ParticipantComplianceCredential` or `gx:LegalPerson` whose issuer is
not in `PORTITOR_TRUSTED_ISSUERS`. A data space issuer's credential of
these types is about the caller and is checked like any other (ADR
0016), holder binding included.

For a credential of type `gx:ParticipantComplianceCredential` (scope
type `ParticipantComplianceCredential`, ADR 0021):

1. Its issuer is in `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` (otherwise it
   is skipped, see below), and its signature is valid with that issuer's `assertionMethod` key (PS256 or
   ES256; the JWS rules of ADR 0015 stay).
2. Now is within `validFrom` and `validUntil`. The header's `iat` and
   `exp` are ignored: the service writes them in milliseconds.
   `validUntil` (`expirationDate` in VC 1.1) is required: without it the
   credential would never expire. A credential from a trusted service
   without it breaks this rule, so it fails this step and its
   presentation is invalid; it is not skipped.
3. No status list is expected; a `credentialStatus`, if present, is
   checked as for any other credential.
4. The same presentation holds a `gx:LegalPerson` credential whose
   `digestSRI` (computed as above, on its decoded payload) is listed in
   `gx:compliantCredentials`, whose `issuer` is the caller's DID, and
   whose signature is valid with the caller's `assertionMethod` key. Its
   `credentialStatus`, if any, is not fetched: the caller issued it and
   controls its status lists, so a status proves nothing, and fetching
   would only let the caller pick URLs the verifier fetches. The
   compliance credential's validity bounds it.
5. The holder binding of ADR 0016 is not applied to these two
   credentials; it stays for every other credential.

A compliance credential whose issuer is not in
`PORTITOR_GAIAX_COMPLIANCE_ISSUERS` is skipped, before anything is
fetched for it, like a credential from an untrusted issuer: a
participant of another clearing house can still use offers that need
only a membership. A compliance credential from a trusted service that
fails any other step makes its presentation invalid and the caller is
refused (DCP step 8, as in 0.6). A `gx:LegalPerson` credential that no
trusted compliance credential names is skipped too.

### Policies and scopes

- The left operand is ADR 0019's: `ParticipantComplianceCredential.<path>`,
  for example `ParticipantComplianceCredential.gx:labelLevel` `eq` `SC`.
  No new ODRL vocabulary.
- Policies see a checked compliance credential only as
  `gx:ParticipantComplianceCredential` (and
  `ParticipantComplianceCredential`), and the linked legal person
  credential only as `gx:LegalPerson` (and `LegalPerson`), whatever
  other types they list: their signers vouch for that type only. So a
  compliance credential also typed `MembershipCredential` neither lets
  the caller in nor meets a membership constraint.
- `PORTITOR_POLICY_CREDENTIALS` must list
  `ParticipantComplianceCredential` with its scope, as for any type.
- When a policy needs `ParticipantComplianceCredential`, both sides add
  the scope for `LegalPerson` with the same alias, so the holder returns
  the linked credential. The holder needs no Gaia-X logic.

## Consequences

- Gaia-X policies work between Portitor connectors. EDC cannot meet
  them without its own extension, and an EDC provider cannot ask
  Portitor for them; the EDC interop test keeps running without Gaia-X.
- The binding rule is our reading. The DCP project is asked how a
  compliance credential should be bound to its holder over DCP:
  https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/issues/278 (2026-11-16). The same
  question to Gaia-X is not asked yet. No Gaia-X document answers it: the
  Compliance Document 4.0.0 sets the compliance rules only; the ICAM
  document 25.11 adopts OID4VCI and OID4VP, and binds only party
  credentials to a holder key (`gx:holder`); the Architecture Document 3.1,
  section 4.5 "Gaia-X Trust Protocol", covers trust between ecosystems.
  The place to ask is the ICAM working group, on GitLab, where the
  maintainer has no account (2026-11-16).
- The verifier now computes JCS digests: one small, pure module in
  `portitor-dcp`, tested against the digests of the 0.8 spike.
