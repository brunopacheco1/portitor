<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0019: Credential constraints in ODRL policies

- Status: **Accepted** (2026-11-14, by the maintainer)
- Date: 2026-11-14
- Scope: release 0.6. Extends ADR 0009 (minimal ODRL: `dateTime` only,
  fail closed) and ADR 0016 (DSP caller authentication with DCP).
- Decision already taken by the maintainer (2026-11-14, 0.6 plan): the
  left operand follows EDC's convention `<CredentialType>.<claim path>`,
  and scopes are derived from its prefix.

## Context

Since 0.5 the provider checks on every DSP request that the caller holds
a valid `MembershipCredential` from a trusted issuer (ADR 0016). An offer
cannot yet say "only participants with role `provider`" or "only holders
of a `DataProcessorCredential`". Any left operand other than `dateTime`
makes a policy "not understood", so it never holds (ADR 0009).

DCP has no policy language; a verifier asks the holder's credential
service for credentials by **scope**, and it can only read what the
holder's access token allows. So both sides must derive the same scopes
from a policy, or the verifier never sees the credential it needs.

What EDC does (checked 2026-11-14 in the Minimum Viable Dataspace,
commit 17bb5b8, `k8s/provider/application/controlplane-config.yaml`):

- Scopes are configured per prefix:
  `edc.iam.dcp.scopes.manufacturer.type: "POLICY"`,
  `...prefix-mapping: "ManufacturerCredential"`,
  `...value: "org.eclipse.dspace.dcp.vc.type:ManufacturerCredential:read"`.
  A policy whose left operand starts with the prefix adds that scope.
- The MVD's policies use left operands such as
  `ManufacturerCredential.part_types` and `DataAccess.level`.
- EDC IdentityHub expects the `:read` suffix on scopes (found in 0.5;
  reported upstream: https://github.com/eclipse-edc/IdentityHub/issues/1278).

## Decision

### The left operand

- A credential constraint is `{"leftOperand": "<Type>.<path>",
  "operator": ..., "rightOperand": ...}`. `<Type>` is a credential type
  the operator has configured; `<path>` is a dot path into the
  credential's `credentialSubject` (for example `role`, or
  `level.name`).
- Operators: `eq`, `neq` with a string; `isAnyOf`, `isNoneOf` with an
  array of strings. Anything else is not understood, and fails closed
  (ADR 0009).
- The constraint holds if at least one valid credential of that type,
  from a trusted issuer, has the claim and the comparison is true. A
  missing claim makes `eq` and `isAnyOf` false, and `neq` and `isNoneOf`
  false as well: a missing fact never grants access.
- `dateTime` stays as in ADR 0009. Constraints combine with `and`, `or`,
  `xone` and `andSequence` as today.

### Configuration

- `PORTITOR_POLICY_CREDENTIALS` lists the credential types that policies
  may use, with their scope, for example
  `DataProcessorCredential=org.eclipse.dspace.dcp.vc.type:DataProcessorCredential:read`.
  `MembershipCredential` is always there (ADR 0016).
- A datasets file whose offer uses a type that is not configured stops
  the connector at start, as an offer that is not understood does today.
- Trusted issuers stay one list for all types (ADR 0016).

### Where it is checked

- **Provider, contract request and agreement:** the provider derives the
  scopes from the offer's left operands, adds the membership scope, and
  asks the consumer's credential service for them with the access token
  the consumer sent. It agrees only if the permissions hold with those
  credentials (ADR 0009 plus this ADR).
- **Provider, transfer start:** the agreement's policy is checked again
  with fresh credentials, so a revoked credential stops new transfers.
- **Consumer:** when it sends a contract request or a transfer request,
  it asks its own STS for an access token with the scopes derived from
  the offer, the same rule as the provider's.
- **Catalog:** shows every offer in 0.6. Hiding offers the caller cannot
  meet (EDC's "access policy") is later work.

### Code layout

- The evaluator stays a pure module in `portitor-dsp` (`policy`). It gets
  the caller's verified credentials as plain input (types and subject
  claims), so it does no I/O and does not depend on `portitor-dcp`.
- Deriving scopes from a policy is a pure function next to it.
- The connector's DCP adapter returns the verified credentials with the
  caller's DID; today it keeps only the DID.

## Consequences

- An EDC participant must configure the same prefix mapping for any type
  a Portitor offer uses; DEPLOYING shows the EDC setting next to ours.
- Each negotiation that uses credential constraints costs one more
  presentation query (the first one, on the HTTP edge, asks for the
  membership only). Measured in the 0.6 release.
- Prohibitions and duties are still not enforced (ADR 0009).
- Offers that use credential constraints are now accepted at start;
  offers with any other left operand still stop the connector.

## Amendment in 1.4: access policies, trusted issuers by type, negative operators (accepted 2026-11-27)

Decided by the maintainer on 2026-11-27 (1.4 plan, D4 and D7), for
[TD-13](../RISKS_AND_TECHNICAL_DEBTS.md#td-13),
[R-48](../RISKS_AND_TECHNICAL_DEBTS.md#r-48) and
[R-43](../RISKS_AND_TECHNICAL_DEBTS.md#r-43).

### Access policy per dataset (TD-13)

Eclipse EDC builds its catalog from the contract definitions whose
**access policy** holds for the caller
(`ContractDefinitionResolverImpl`, read at EDC Connector commit
`4215744` on 2026-11-27); the contract policy is checked at
negotiation. Portitor does the same, per dataset:

- A dataset in the datasets file may have `portitor:accessPolicy`, an
  object with `permission` rules read as an offer's (strictly, ADR
  0009): `{"permission": [{"action": "use", "constraint": [...]}]}`.
  It uses the constraints offers use: `dateTime` and credential
  constraints. A `purpose` constraint, a prohibition or a duty in an
  access policy is not understood, and the dataset is rejected (check
  `odrl`), as a bad offer is.
- The access policy is checked on DSP catalog requests and dataset
  requests. A dataset whose access policy does not hold for the caller
  is left out of the catalog, and a dataset request for it gets the
  same answer as an unknown dataset. Like offers, a policy that is not
  understood never holds.
- A dataset without an access policy is shown to every member, as
  before 1.4.
- Like `portitor:source`, the access policy is never shown in the
  catalog.
- The verifier asks the caller's credential service for the scopes of
  the access policies' credential types once per catalog request
  (ADR 0019 scopes, as for offers).
  A dataset request asks for the scopes of that dataset's access policy.
  The caller's token must allow them: Portitor as consumer adds those
  given in `additionalScopes` of its management catalog request.
- Details found when building it (2026-11-27): an access policy must
  have at least one permission, and has no other field (no `@id` or
  `@type`). It is stored with the dataset, in its own column, and kept
  in the stored entry too, so a check with new rules reads it again. A
  negotiation checks the offer only, not the access policy.

### Trusted issuers by credential type (R-48)

"Trusted issuers stay one list for all types" (Configuration, above)
changes. The new, optional setting `PORTITOR_TRUSTED_ISSUER_TYPES`
limits an issuer of `PORTITOR_TRUSTED_ISSUERS` to a list of credential
types (`<issuer DID>=<type>` items), on the connector and the identity,
as EDC's `edc.iam.trustedissuer.<alias>.supportedtypes`. A credential
from an issuer that is not trusted for its type is skipped, as one from
an untrusted issuer. An issuer the setting does not name keeps every
type (EDC's `"*"`), so a 1.3 deployment behaves the same. The safer
default waits for 2.0 ([TD-74](../RISKS_AND_TECHNICAL_DEBTS.md#td-74)).

### Negative operators are not deny lists (R-43)

A constraint holds if **at least one** valid credential of the type has
the claim and the comparison is true (Decision above). So `neq` and
`isNoneOf` say "the caller holds a credential of this type whose claim
is not this value", not "no credential of the caller has this value". A
holder with two credentials of the type, one with the excluded value
and one without, meets the constraint, and the holder chooses what it
presents. Do not use `neq` or `isNoneOf` to keep someone out; refuse
them at the issuer (revoke the credential) or limit the issuer by type.
