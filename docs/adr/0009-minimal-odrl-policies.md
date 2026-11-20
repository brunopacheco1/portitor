<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0009: Minimal ODRL policies

- Status: **Accepted** (decided by the maintainer, 2026-11-12)
- Date: 2026-11-12
- Scope: release 0.3, roadmap item "Minimal ODRL policy support: permit
  all, and one constraint type".

## Context

Offers and agreements in DSP 2025-1 are ODRL policies. Until now the
provider agreed to any request that named an offer `@id` from its catalog,
and the consumer accepted any offer and verified any agreement. This left
two gaps:

- A consumer could send a known offer `@id` with different terms (for
  example without its date limit), and the provider agreed to those terms.
- A constraint in an offer was never checked, so an offer meant to end on
  a date was agreed after that date.

Facts checked (2026-11-12):

- The DSP 2025-1 examples use `{"leftOperand": "dateTime", "operator":
  "lteq", "rightOperand": "2023-12-31T06:00Z"}`. That form has minutes but
  no seconds. The DSP JSON Schema allows the operators `eq`, `gt`, `gteq`,
  `lt`, `lteq`, `neq`, `isA`, `hasPart`, `isPartOf`, `isAllOf`, `isAnyOf`
  and `isNoneOf`, and a right operand that is a string, object or array.
  In the DSP JSON-LD context, `leftOperand` and `operator` are ODRL
  vocabulary terms.
- The ODRL 2.2 vocabulary defines `eq`, `neq`, `lt`, `lteq`, `gt` and
  `gteq` as comparisons with the right operand (W3C ODRL Vocabulary,
  section 3.16). It lists `dateTime` as a left operand. Its definition
  could not be read from this environment. From memory: "the date (and
  optional time and timezone) of exercising the action of the Rule", with
  an `xsd:date` or `xsd:dateTime` right operand. Check before 1.0.
- The DSP TCK 1.0.2 sends offers with one `use` permission and an empty
  constraint list, so the checks below keep the TCK green.

## Decision

- **One constraint type: `dateTime`**, the time the action is exercised,
  compared with "now". The operators are `eq`, `neq`, `lt`, `lteq`, `gt`
  and `gteq`. `and`, `andSequence`, `or` and `xone` combine constraints.
  Several constraints on one rule must all hold. A rule without
  constraints permits ("permit all").
- **Values.** The right operand is a date (`2030-12-31`, meaning the
  whole day, in UTC unless a time zone is given) or a date and time with a
  time zone. Seconds and a fraction are optional. A date and time without
  a time zone is refused, because it names a different instant in every
  time zone. Terms may be written short (`lteq`), prefixed (`odrl:lteq`)
  or as a full ODRL IRI.
- **Fail closed.** Any other left operand, operator or value is not
  understood, and a policy that is not understood never holds. Every part
  of a logical constraint is checked, so an unknown part is never skipped.
- **Provider.** It agrees only when two things are true. First, the
  negotiated offer has the same terms (permissions, prohibitions, duties,
  profile) as the catalog offer with that `@id`. Second, its permissions
  hold now. Otherwise it terminates the negotiation with the reason. The
  rule is in the domain, so it applies to every decider. A datasets file
  with an offer that is not understood stops the connector at start.
- **Consumer.** The default decider accepts a provider's offer only if
  its permissions hold now. It verifies an agreement only if the agreement
  has the terms of the offer it asked for or accepted. Otherwise it
  terminates the negotiation.
- **Only permissions are evaluated**, because they decide whether an
  agreement may be made now. Prohibitions and duties are terms of the
  agreement; they are compared and carried into it, but not enforced.
- The evaluator is a pure module in `portitor-dsp` (`policy`), with no new
  dependency.

## Consequences

- A known offer `@id` can no longer be used to agree to other terms. A
  consumer counter-offer with different terms is terminated by the
  default provider policy; negotiating new terms comes later.
- Terms are compared as written, except for the action name. The same
  constraint written another way (for example `odrl:lteq` instead of
  `lteq`) counts as different terms. Consumers should copy offers from the
  catalog unchanged.
- Offers that use other constraints (for example `spatial` or `purpose`)
  cannot be published until the evaluator understands them. The example
  datasets file now uses `dateTime`.
- Time comes from the host clock. A wrong clock gives wrong decisions near
  a limit, so hosts must run NTP (DEPLOYING).
- Usage control at transfer time (0.4 and later) will evaluate the
  agreement again; this ADR covers only the decision to agree.

## Amendment, 2026-11-18: policies are read strictly

Decided by the maintainer on 2026-11-18, after finding D1 of the
[review of that day](../reviews/2026-11-18-specifications-and-code-quality.md)
(TD-32).

**Problem.** The policy types kept the fields they knew and dropped the
others when the JSON was read: a `duty` inside a permission, `unit` on a
constraint, a misspelled key. The term comparison then compared what was
left, so Portitor could agree to an offer with a duty it never saw. That
went against "fail closed" above.

**Decision: strict.** A policy term that Portitor does not read is
refused, with an error that names the field. It is never dropped.

- **Datasets file and harvest.** The dataset is rejected (check `odrl`,
  message `offer "<id>" is not understood: unknown field ...`). As for
  the other harvest checks (ADR 0020), only that dataset stops; its
  previous good version, if any, stays.
- **A partner's offer, contract request or agreement.** The message is
  answered HTTP 400 with a `ContractNegotiationError` whose reason names
  the field (DSP 2025-1, Contract Negotiation HTTPS binding, error
  responses). The negotiation does not change state.
- **A `duty` inside a permission** is a real ODRL 2.2 term, and the DSP
  context names it. Portitor cannot enforce it, so it is refused, not
  agreed.
- The operator's own offer on `POST /v1/negotiations` is read the same
  way (400).

**Fields read.** They come from the DSP 2025-1 JSON Schema
(`negotiation/contract-schema.json`) and JSON-LD context
(`context/odrl.jsonld`); nothing else is accepted.

| Object | Fields | Source |
|---|---|---|
| Offer | `@id`, `@type`, `profile`, `permission`, `prohibition`, `obligation`, `target` | schema: `PolicyClass`, `MessageOffer` |
| Offer | `assigner`, `assignee` | context `odrl.jsonld`; not in the schema for an offer, but Eclipse EDC 1.0.0-RC7 and the DSP TCK 1.0.2 send them in the offer of a message |
| Agreement | the Offer fields and `timestamp`; `target`, `assigner`, `assignee` required | schema: `PolicyClass`, `Agreement` |
| Rule (permission, prohibition, duty) | `action`, `constraint` | schema: `Rule`, `Duty` |
| Atomic constraint | `leftOperand`, `operator`, `rightOperand` | schema: `AtomicConstraint` |
| Logical constraint | exactly one of `and`, `andSequence`, `or`, `xone` | schema: `LogicalConstraint` |

The DSP schema does not forbid other fields (no
`additionalProperties: false`), so a message with one is valid DSP and is
still refused here. That is the point of the decision.

**Parties and target are checked, not only read.** An offer in a catalog
(the datasets file) has no `target`, `assigner` or `assignee`. In a
message, an offer's `assigner` must be the provider and its `assignee`
the consumer, when present; an agreement's were already checked. The term
comparison leaves them out, as before.

**Unchanged.** Prohibitions and duties at the top of a policy are read,
compared and carried into the agreement, and not enforced. Their
constraints are not evaluated, so they may use any left operand; only
their fields are checked.

**Stricter than Eclipse EDC, on purpose.** EDC 1.0.0-RC7 ignores unknown
policy fields without an error (`JsonObjectToPolicyTransformer` and
`JsonObjectToPermissionTransformer`: `default -> doNothing()`), and it
reads a `duty` inside a permission. So an offer that EDC accepts can be
refused by Portitor. What EDC itself writes into a policy
(`JsonObjectFromPolicyTransformer`) is accepted: `@id`, `@type`,
`permission`, `prohibition`, `obligation` (also as empty lists),
`assigner`, `assignee`, `target`, `profile`, `timestamp`, and in a rule
`action` and `constraint`. EDC writes `duty`, `remedy`, `consequence`
and an action with `includedIn` or `refinement` only when its own policy
has them; these are refused.

**Consequences.**

- After the upgrade the stored datasets are checked again at start (ADR
  0020, step 8). A dataset whose offer has an unknown field is hidden
  until its source is corrected.
- A typing error in a policy is now an error, not a silent change of the
  terms.
- A partner whose policies use ODRL terms outside the table cannot
  negotiate with Portitor until the term is supported or removed.
- A JSON key given twice is refused too, in a DSP negotiation or
  transfer message, in a constraint and in the datasets file (added
  2026-11-19, after the 0.12 testing review). JSON permits it, but RFC
  8259, section 4, calls the behaviour of readers "unpredictable": one
  keeps the first value, another the last, so the two parties could
  agree on different terms. The typed readers already refused it; the
  places that read plain JSON first now do the same
  (`portitor_dsp::unique_keys`).
