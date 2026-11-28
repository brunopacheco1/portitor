<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0032: Negotiating on a data user's behalf

- Status: **Proposed** (2026-11-28), for the maintainer to accept or
  reject.
- Date: 2026-11-28
- Scope: release 1.5
  ([ROADMAP](../ROADMAP.md#15-data-user-and-hdab-the-hdab-and-schuman-clinical-research)),
  way D of the health demo
  ([ROADMAP, "Ways to link data user, HDAB and holder"](../ROADMAP.md#ways-to-link-data-user-hdab-and-holder-maintainer-2026-11-23)).
  Answers [R-51](../RISKS_AND_TECHNICAL_DEBTS.md#r-51). Research only,
  no code in 1.5 (maintainer, 2026-11-28).

## Context

Way D: the HDAB negotiates with Lux Biobank for Schuman Clinical
Research. The agreement's assignee would be Schuman, not the HDAB, and
Schuman's connector would then transfer the data under it.

Portitor refuses this in four places today:

- An offer in a negotiation message whose `assignee` is not the
  consumer is refused (`portitor_dsp::policy::check_parties`; ADR 0009,
  amendment of 2026-11-18).
- The provider writes the negotiation's counter-party, the caller's
  verified DID, as the agreement's `assignee`
  (`crates/portitor-dsp/src/negotiation/provider.rs`).
- A transfer is accepted only under an agreement whose `assignee` is
  the caller (`crates/portitor-dsp/src/transfer/provider.rs`;
  [ADR 0013](0013-transfer-and-data-plane.md), check 2).
- The consumer refuses an agreement whose `assignee` is not itself
  (`crates/portitor-dsp/src/negotiation/consumer.rs`).

The caller is the DID of its DCP token, and its credentials are those
whose holder it is ([ADR 0016](0016-dsp-caller-authentication.md)).
Credential constraints are checked against the caller's credentials
([ADR 0019](0019-credential-constraints.md)).

The question: does a specification, a reference product or the EHDS
let one participant negotiate an agreement for another, in a way the
provider can check?

## What the sources say

All sources were read on 2026-11-28.

### DSP 2025-1: the assignee is "dataspace-specific"

Read from the tag `2025-1-err2` (commit
`d7d810b276f5f5710ff7a53bff630e2845cc0105`) of
[DataspaceProtocol](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/tree/2025-1-err2).

- Contract Agreement Message: "An Agreement MUST contain an `assigner`
  and `assignee`. The contents of these properties are a
  dataspace-specific unique identifier of the Agreement parties. These
  identifiers MAY be the same as the identifiers of the Participant
  Agents negotiating the Agreement (e.g., Connectors)."
- So DSP does not require the assignee to be the party that negotiates.
  It does not say how a provider would check another assignee either.
- Consumer: "A Participant that requests access to an offered Dataset."
  Participant Agent: "A technology system that performs operations ...
  on behalf of a Participant". The only "on behalf of" in DSP is an
  agent acting for its own participant.
- Transfer: "The `agreementId` property MUST refer to an existing
  Agreement between the Consumer and Provider."
- Authorization: "The semantics of such tokens are not part of this
  specification" (common protocol, "Authorization").

### DCP v1.0.1: the caller is the token's subject

Read from the tag `v1.0.1` (commit
`27175bc63ba47c13bae37dd4a158e1a1cb50af82`) of
[decentralized-claims-protocol](https://github.com/eclipse-dataspace-dcp/decentralized-claims-protocol/tree/v1.0.1).

- Base protocol, "Self-Issued ID Tokens": "The `iss` and `sub` claims
  MUST be equal and set to the bearer's (participant's) DID."
- Presentation protocol, "Presentation Validation": "if the specific
  semantics of a data space and credentials require cryptographic
  holder binding, the Verifier MUST assert that the
  `credentialSubject.id` in the Verifiable Credential and the DID part
  of the `VerificationMethod` of the Verifiable Presentation have the
  same value." Holder binding is left to the dataspace.
- Terminology: the holder "will typically be the subject".
- No text on delegation or acting for another participant (searched
  for "on behalf", delegat, "act for", "another participant",
  third-party).

### W3C VC Data Model 2.0: holder and subject may differ; no delegation

Read from the Recommendation of 15 May 2025,
<https://www.w3.org/TR/vc-data-model-2.0/>, and the Implementation
Guidelines source in [w3c/vc-imp-guide](https://github.com/w3c/vc-imp-guide)
(commit `f46a482fd1ba7d3bb419f64cb1bdcd4ae6074fce`; a non-normative
Note written for VC 1.x).

- Section 1.2, "Subjects are not always Holders": "In many cases, the
  holder of a verifiable credential is the subject, but in some
  instances it is not." It sends the reader to the Implementation
  Guidelines for these cases.
- No relationship or delegation credential type is defined
  ("RelationshipCredential" is only an example).
- Implementation Guidelines, "Holder Acts on Behalf of the Subject":
  "The additional mechanisms the issuer or the verifier uses to verify
  the relationship between the subject and the holder are outside the
  scope of this specification." "Issuer Authorizes Holder":
  "Verifiable credentials are not an authorization framework and
  therefore delegation is outside the scope of this specification."

### ODRL 2.2: any party may be assignee

Read from <https://www.w3.org/TR/odrl-model/> and
<https://www.w3.org/TR/odrl-vocab/> (Recommendations of 15 February
2018).

- `assignee`: "The Party is the recipient of the Rule." A Party is "a
  person, collection of people, organisation, or agent" (model,
  section 2.3). ODRL does not say who negotiates.
- `Offer`: "MAY contain a Party with Assignee function, but MUST not
  grant any privileges to that Party." `Ticket`: the holder "MAY remain
  unknown". Neither names a delegate.

### Eclipse EDC: the assignee is the caller

Read at [Connector](https://github.com/eclipse-edc/Connector) commit
`4215744f2f22764668ac9bfffda646e06238d888`.

- `NegotiationProcessorsImpl` (`core/control-plane/control-plane-contract/`):
  the agreement's policy gets
  `.assignee(negotiation.getCounterPartyId())` and the agreement
  `.consumerId(negotiation.getCounterPartyId())`.
- The counter-party is the token's identity
  (`ContractNegotiationProtocolServiceImpl`: `agent.getIdentity()`);
  with DCP it is the `credentialSubject.id` of the first presented
  credential (`DefaultDcpParticipantIdExtractionFunction`).
- `ContractValidationServiceImpl.validateAgreement`:
  `if (!Objects.equals(agent.getIdentity(), agreement.getConsumerId()))`
  fails. `TransferProcessProtocolServiceImpl` calls it on every transfer
  request.
- No delegation, proxy or "on behalf of" in negotiation. Multi-tenancy
  (`ParticipantContext`, decision record 2025-08-26) gives each tenant
  its own identity; one does not act for another.

### Tractus-X: no delegation

Read at [tractusx-edc](https://github.com/eclipse-tractusx/tractusx-edc)
commit `705412fd8e7d9804a772b1d2efece733cc3800aa`.

- No hit for "on behalf" or delegation of a BPN, identity or
  negotiation. Multi-tenancy uses a participant context per tenant
  (`docs/migration/2026_03-Version_0.11.x_0.12.x.md`), each with its
  own BPN.
- The only notion of one company acting for others is legal text,
  `cx.sharing.managedLegalEntity:1` ("is managed by", CX-0074), in a
  constraint that always holds.

### Simpl-Open: the consumer is the authenticated participant

Read on [code.europa.eu/simpl](https://code.europa.eu/simpl/simpl-open):
`connector-be` commit `4c56510423d94d78857c758e19c410ba9625ee07`,
`tier-2-gateway` commit `11277dceb22b23de739ffe5e370860d014a87a43`,
`contract-manager-be` commit `eec02566392c577bf6b466ebf34ad3547da6406b`.

- The Tier 2 gateway authenticates the caller by mTLS and sets the
  `Participant-Id` header from its session (`filters/HeadersFilter.java`,
  which first removes any such header the caller sent).
- `ContractAgreementTO` has one `providerId` and one `consumerId`.
- No delegation found in 13 repositories. The Simpl Confluence pages
  were not read.

### The EHDS: the HDAB requests data by law, and processes for the user

Read in Regulation (EU) 2025/327, Official Journal text from the
Publications Office
(<http://publications.europa.eu/resource/celex/32025R0327>; EUR-Lex
did not answer the fetch tool).

- Art. 57(1)(a)(iii): a task of the HDAB is "requesting electronic
  health data ... from relevant health data holders pursuant to a data
  permit issued or a health data request approved".
- Art. 60(1): holders "shall make relevant electronic health data ...
  available upon request to the health data access body, in accordance
  with a data permit". Art. 68(7): the HDAB "shall immediately request
  the electronic health data from the health data holder" and makes
  them available to the user.
- Art. 73(1): access "only through a secure processing environment".
- Art. 74(1): the holder is controller "for the making available ... to
  the health data access body"; the HDAB "shall be deemed to act as a
  processor on behalf of the health data user ... in the secure
  processing environment".
- Art. 2(2)(v): a data permit is "an administrative decision issued to
  a health data user by a health data access body". Contracts between
  holder and user are optional (Art. 52(4), for intellectual property
  and trade secrets).

So under the EHDS the holder gives the data **to the HDAB**, under a
legal duty and a permit, not under a contract the HDAB negotiates for
the user. "On behalf of the user" is the HDAB's role as processor in
the secure processing environment. Most of Chapter IV applies from 26
March 2029 (Art. 105).

## Options

### A. Way D as asked: the HDAB negotiates, Schuman is the assignee

- Pro: one negotiation per user at the holder; the agreement names the
  real user.
- Con: no specification defines how the provider checks that Schuman
  agreed. Portitor would need its own delegation credential (subject
  Schuman, holder the HDAB, or issued by Schuman to the HDAB), its own
  scope for it, and its own rule to relax the holder binding of ADR
  0016 for that type.
- Con: credential constraints (ADR 0019) would have to be checked
  against Schuman's credentials, which the HDAB's token cannot reach
  (DCP: the access token is the caller's own).
- Con: Schuman's connector would transfer under an agreement it did
  not negotiate; DSP says the agreement is "between the Consumer and
  Provider".
- Con: an EDC holder would refuse it (the assignee is always the
  caller) and an EDC user could not transfer under it
  (`validateAgreement`).
- Con: it does not match the EHDS, where the data goes to the HDAB
  (Art. 60(1), 74(1)).

### B. The HDAB is the assignee, the user is named in the terms

The HDAB negotiates for itself (as today) and the agreement names the
user as a term, for example a credential constraint on the user's
permit that the HDAB holds, or an ODRL `recipient` constraint.

- Pro: fits DSP, DCP, EDC and the EHDS flow: the HDAB receives the
  data.
- Con: `recipient` is a non-normative ODRL left operand with no
  evaluator in any product we read; Portitor would read it as a term
  only, as `purpose`.
- Con: Portitor checks credentials whose holder is the caller; the
  user's permit held by the HDAB would need a relaxed holder binding,
  which no specification defines.

### C. The user negotiates itself, with its permit

Schuman negotiates with Lux Biobank and presents its own data permit
(ways C and F).

- Pro: works with existing standards today: DCP presentation by the
  holder, credential constraints (ADR 0019). `scenarios/data-permits`
  shows it.
- Con: not the EHDS flow for personal data: the data must go to a
  secure processing environment (Art. 73), which needs the destination
  check of way C (1.10).

### D. Keep ADR 0013, build nothing

- Pro: no invented format; Portitor behaves as EDC and Simpl-Open.
- Con: a user that wants the HDAB to negotiate for it cannot use
  Portitor.

## Decision

**No existing way to follow was found.** DSP lets the assignee be
another party than the negotiating agent, but defines no check. DCP
binds the caller to its own DID. VC 2.0 leaves delegation out of scope.
EDC, Tractus-X and Simpl-Open always make the authenticated caller the
consumer. The EHDS does not have the HDAB negotiate for a user: the
holder makes data available to the HDAB, which processes it for the
user in a secure processing environment.

So (option D):

- **Portitor keeps ADR 0013**: the assignee is the caller, at
  negotiation and at transfer. No code in 1.5, none planned for 1.10.
- **The HDAB acts as itself.** The EHDS relation is covered by way A
  (the HDAB is the assignee and the processor, today's
  hdab-intermediary) and way C (the user's own permit, 1.10). Both use
  existing mechanisms.
- **What Portitor would need** if a specification adds delegation
  later, so the work is known:
  1. a delegation the provider can verify: a credential type and its
     DCP scope, defined by DCP or the dataspace, not by Portitor;
  2. a rule for holder binding when the holder is not the subject (DCP
     "Presentation Validation" leaves it to the dataspace);
  3. the four checks above changed together
     (`policy::check_parties`, the provider's agreement in
     `negotiation/provider.rs`, the transfer check in
     `transfer/provider.rs`, the consumer's check in
     `negotiation/consumer.rs`), and credential constraints evaluated
     on the delegating party's credentials;
  4. the same in EDC, or the EDC interop test fails.
- R-51 stays Open: the gap is real, but it is in the specifications.

## Consequences

- Nothing changes in Portitor's behaviour or messages.
- The health demo shows the HDAB as intermediary (way A) and the user
  with its own permit (ways F and C), not way D. The scenario row "The
  HDAB on a data user's behalf" in the ROADMAP is not built.
- If the maintainer wants way D anyway, it needs a new ADR that
  defines Portitor's own delegation credential, and accepts that it
  will not interoperate with EDC or Simpl-Open.
