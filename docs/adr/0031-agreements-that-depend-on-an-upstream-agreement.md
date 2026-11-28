<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0031: Agreements that depend on an upstream agreement

- Status: **Proposed** (2026-11-28), for the maintainer to accept or
  reject.
- Date: 2026-11-28
- Scope: release 1.5
  ([ROADMAP](../ROADMAP.md#15-data-user-and-hdab-the-hdab-and-schuman-clinical-research)),
  way B of the health demo
  ([ROADMAP, "Ways to link data user, HDAB and holder"](../ROADMAP.md#ways-to-link-data-user-hdab-and-holder-maintainer-2026-11-23)).
  Answers [R-39](../RISKS_AND_TECHNICAL_DEBTS.md#r-39). Research only;
  the maintainer decided on 2026-11-28: code only if this ADR finds an
  existing way to follow.
- Number: 0030 was a withdrawn draft that never reached `main`; the
  number is not used.

## Context

In the [hdab-intermediary](../../scenarios/hdab-intermediary/README.md)
scenario the HDAB holds an agreement with Lux Biobank on the genotypes.
It offers counts made from them to data users such as Schuman Clinical
Research. Its offer to Schuman must not be wider than its own
agreement with Lux Biobank, on three points:

- **purpose**: the HDAB may not offer the counts for a purpose its own
  agreement does not allow;
- **users** (assignee): who may take the counts;
- **duration**: the HDAB's offer may not outlive its own agreement.

Today nothing checks this. The HDAB's offer ends on 2027-12-31, inside
its agreement, only because the datasets file was written so by hand
(step 0 of the demo).

The question: does a specification or a reference product already
link a downstream offer or agreement to an upstream agreement, so that
Portitor can follow it?

## What the sources say

All sources were read on 2026-11-28.

### DSP 2025-1: one agreement, two parties, one dataset

Read from the tag `2025-1-err2` (commit
`d7d810b276f5f5710ff7a53bff630e2845cc0105`) of
[DataspaceProtocol](https://github.com/eclipse-dataspace-protocol-base/DataspaceProtocol/tree/2025-1-err2).

- Terminology: an Agreement is "A concrete Policy associated with a
  specific Dataset that has been agreed between the Provider and
  Consumer".
- Contract Agreement Message: "An Agreement MUST contain an `assigner`
  and `assignee`", "MUST contain a `target` property", and "MUST be
  associated with exactly one Dataset".
- `negotiation/contract-schema.json`, `Agreement`: properties `target`,
  `assigner`, `assignee`, `timestamp`; no field points to another
  agreement. The JSON-LD context (`context/odrl.jsonld`) has no term
  for `inheritFrom`, `nextPolicy`, `distribute` or `grantUse`.
- Catalog: "The value of the `target` attribute MUST be the Dataset
  identifier." Nothing says a dataset must be the provider's own
  original data, and nothing forbids offering data derived from data
  received under another agreement.
- The only intermediary in DSP is the Catalog Broker (catalog protocol,
  "Catalog Brokers"): it "SHOULD honor upstream access control
  requirements (Policies)". It aggregates catalogs; it does not
  negotiate or hold agreements.
- No text links two agreements (searched the specifications and
  schemas for "intermediar", "on behalf", "re-offer", "derived",
  "chain").

### ODRL 2.2: terms exist, no rule to compare

Read from the W3C Recommendations of 15 February 2018,
<https://www.w3.org/TR/odrl-model/> and <https://www.w3.org/TR/odrl-vocab/>,
and compared with `vocab/ODRL22.ttl` in
[w3c/poe](https://github.com/w3c/poe) (branch `gh-pages`, commit
`a2ba47f1fbde57267bebdbb7116500d0cf6b8efd`).

- `inheritFrom` (Core vocabulary): "Relates a (child) policy to another
  (parent) policy from which terms are inherited." Model, section 2.9:
  the child "MUST access the parent Policy and replicate" its assets,
  parties, actions and "All Rules from the parent Policy". Inheritance
  adds the parent's rules to the child; it does not check that the
  child is narrower.
- `nextPolicy` (Common vocabulary, non-normative) is an **action**:
  "To grant the specified Policy to a third party for their use of the
  Asset." Note: "Useful for downstream policies." `distribute` ("To
  supply the Asset to third-parties") and `grantUse` ("To grant the use
  of the Asset to third parties") recommend it. ODRL says nothing on
  how the next policy is written, found or evaluated.
- `Agreement`: "A Policy that grants the assignee a Rule over an Asset
  from an assigner." `assignee`: "The Party is the recipient of the
  Rule."
- Conflict (model, section 2.10) applies to merged policies; without a
  `conflict` value a conflicting merged policy is void.
- No rule says a downstream policy "MUST NOT be wider" than an upstream
  one (searched both texts for nextPolicy, wider, broader, downstream,
  third part, sublicen).

### Eclipse EDC: no link

Read at [Connector](https://github.com/eclipse-edc/Connector) commit
`4215744f2f22764668ac9bfffda646e06238d888`.

- `ContractAgreement`
  (`spi/control-plane-spi/.../contract/spi/types/agreement/ContractAgreement.java`)
  has `id, providerId, consumerId, contractSigningDate, assetId,
  policy, participantContextId, agreementId, claims, retired,
  retirementReason, retirementDate`. No field links to another
  agreement.
- A search of the code and `docs/` for contract chain, parent or
  upstream agreement, sublicen, onward, `nextPolicy`, `inheritFrom`,
  re-publish and redistribute found nothing. No code evaluates a policy
  against an upstream agreement when an asset or contract definition
  is created.

### Tractus-X: onward sharing as legal text, always true

Read at [tractusx-edc](https://github.com/eclipse-tractusx/tractusx-edc)
commit `705412fd8e7d9804a772b1d2efece733cc3800aa`,
[tractusx-profiles](https://github.com/eclipse-tractusx/tractusx-profiles)
commit `1fec333861e64cd05d77d0fa3011f7f4ea1edf36` and
[cx-odrl-profile](https://github.com/catenax-eV/cx-odrl-profile)
commit `354ff643c47847562cd66680c13466b7763847a2`.

- Catena-X states onward sharing in policies, as legal terms:
  `AffiliatesBpnl` and `AffiliatesRegion` ("Sublicensing" to affiliated
  companies), `UsageRestriction` with `cx.thirdParty.forbidden:1` ("The
  Data Consumer is prohibited from making the Data available to third
  parties ... unless otherwise individually agreed"), and
  `ConfidentialInformationSharing`. The texts are in the `$comment` of
  `edc-extensions/cx-policy/src/main/resources/jsonschema/cx-policy/*.json`.
- Their functions (`edc-extensions/cx-policy/.../affiliates/`,
  `usage/UsageRestrictionConstraintFunction.java`) extend
  `BaseConstraintFunction`, whose `evaluate` returns `true`. Only the
  operator and the right operand are validated. They are terms of the
  agreement, not checks, as `UsagePurpose` is (ADR 0009, amendment in
  1.4).
- `ContractReference` points to an outside contract by free text
  ("a URL, contract number", `cx/policy/specs/policy.definitions.md`),
  not to a dataspace agreement id, and also always holds.
- `FrameworkAgreement` is checked, but against a credential
  (`FrameworkAgreementConstraintFunction`), not against another
  agreement.

### Simpl-Open: no link

Read on [code.europa.eu/simpl](https://code.europa.eu/simpl/simpl-open):
`contract-manager-be` commit `eec02566392c577bf6b466ebf34ad3547da6406b`,
`policy-manager-be` commit `fe518b6c49a7bbeb68cd665fac600257afb48a1a`,
`connector-be` commit `4c56510423d94d78857c758e19c410ba9625ee07`, and
ten other repositories of the group.

- Policies are ODRL JSON-LD with a Simpl profile
  (`https://ec.europa.eu/simpl/profile#`, `policy-manager-be`
  `docs/architecture/odrl-template-model.md`).
- `ContractAgreementTO` (`contract-manager-be`) has one `providerId`
  and one `consumerId` and no link to another agreement.
- A search of the 13 repositories for re-share, onward, intermediar,
  upstream, parent or derived agreement, sublicen, redistribut and
  chain found nothing on agreements. The Simpl Confluence pages were
  not read.

### The EHDS: the permit is the limit

Read in Regulation (EU) 2025/327, Official Journal text from the
Publications Office
(<http://publications.europa.eu/resource/celex/32025R0327>; EUR-Lex
did not answer the fetch tool).

- The holder makes data available to the HDAB "in accordance with a
  data permit" (Art. 60(1)), within three months (Art. 60(2)).
- The HDAB makes the data available to the data user (Art. 68(7)), in
  a secure processing environment (Art. 73(1)).
- What the user may do is the permit's: purposes (Art. 53, 54), the
  authorised persons and the duration (Art. 68). Users "shall not
  provide access ... to third parties not mentioned in the data permit"
  (Art. 61(2)).

So under the EHDS the limit on the HDAB's onward offer comes from the
permit the HDAB itself issues, not from an agreement between holder and
HDAB. The scenario's peer to peer agreement between Lux Biobank and the
HDAB is the scenario's choice (maintainer, 2026-11-15), not the EHDS
flow.

## Options

### A. ODRL `inheritFrom`: the HDAB's offer inherits its own agreement

The HDAB's offer to Schuman names its Lux Biobank agreement as parent.

- Pro: an ODRL Core term.
- Con: not in the DSP schema or context. Portitor refuses an unknown
  policy field (ADR 0009, amendment of 2026-11-18); EDC drops it
  without a word (`JsonObjectToPolicyTransformer`, ADR 0009).
- Con: inheritance copies the parent's parties too, so the child's
  assignee would be the HDAB, not Schuman.
- Con: the parent is a private agreement between Lux Biobank and the
  HDAB. Schuman cannot read it, so it cannot check the result.
- Con: the targets differ (genotypes upstream, counts downstream).
  Inheritance would copy a target that is not the offered dataset.

### B. ODRL `nextPolicy` / `distribute` in Lux Biobank's offer

Lux Biobank states, in its offer, what the HDAB may offer onwards.

- Pro: says the limit where it comes from, at the holder.
- Con: Common vocabulary, non-normative; `nextPolicy` is an action with
  no defined way to write, find or evaluate the next policy.
- Con: a `nextPolicy` duty inside a permission is refused by Portitor
  (ADR 0009, amendment of 2026-11-18) and evaluated by no product we
  read.

### C. Tractus-X: onward sharing as a term of the agreement

Lux Biobank's offer carries a term such as Catena-X's
`UsageRestriction` `cx.thirdParty.forbidden:1`, with legal text, and
it always holds.

- Pro: an existing, used pattern; Portitor already treats `purpose`
  this way.
- Con: Catena-X terms, under a Catena-X namespace and legal framework.
  It states a limit; it never checks the HDAB's offer against it.
- Con: Portitor would need to read new left operands, in an offer it
  reads strictly (ADR 0009), only to carry text.

### D. Portitor's own check at the HDAB, when it harvests its datasets

A dataset in the HDAB's datasets file names the agreement it rests on
(a new `portitor:` field). At harvest, the connector compares each
offer of that dataset with the named agreement in its own consumer
store: no later `dateTime` end, purposes a subset of the agreement's,
and it refuses the dataset otherwise.

- Pro: small, and local to one connector: nothing new crosses the
  wire, so no partner is affected.
- Con: our own format and our own rule of "narrower". No
  specification or product does it, so it is the invention the
  maintainer asked to avoid.
- Con: covers duration and purpose only. "Which users" has no common
  form between the upstream agreement (an assignee) and the downstream
  offer (credential constraints).
- Con: under the EHDS the limit is the permit, not this agreement, so
  the check may test the wrong thing.

### E. Keep the limit by hand and record the gap

The scenario's datasets file keeps the HDAB's offer inside its
agreement, as step 0 does. The README says it is written by hand.

- Pro: no invented format; nothing to undo if DSP, EDC or Simpl-Open
  add a link later.
- Con: a mistake in the HDAB's datasets file goes unnoticed.

## Decision

**No existing way to follow was found.** DSP 2025-1 has no link
between agreements. ODRL 2.2 has the terms `inheritFrom` and
`nextPolicy`, but DSP does not carry them and ODRL defines no rule that
a downstream policy stays inside an upstream one. EDC and Simpl-Open
have no link. Tractus-X writes onward sharing as legal text that is
never checked.

So, as the maintainer set on 2026-11-28 (option E):

- **No code in 1.5.** The hdab-intermediary scenario keeps the HDAB's
  offer inside its agreement by hand. Its README says so, for each of
  the three points: duration (end date written by hand), purpose (the
  HDAB's offer names none today), users (no check).
- **The gap is recorded** in [R-39](../RISKS_AND_TECHNICAL_DEBTS.md#r-39),
  which stays Open.
- Option D is not built. If the maintainer later wants a local check,
  it needs its own ADR, and the README of the scenario stays the place
  that shows the limit until then.
- Look again when one of these changes: DSP adds a link between
  agreements, EDC or Simpl-Open add contract chaining, or the EHDS
  permit (1.10) gives a common form for purpose, users and duration
  that the HDAB's offers can be checked against.

## Consequences

- Portitor stays compatible with EDC and Simpl-Open: nothing new in
  its messages.
- The HDAB's operator remains responsible for the limit. A mistake in
  its datasets file is not caught.
- Under the EHDS the right place for the limit is likely the data
  permit the HDAB issues (Art. 61(2), 68). Way C and the permit
  scenario (1.10) are where Portitor can check it with an existing
  mechanism: credential constraints (ADR 0019).
