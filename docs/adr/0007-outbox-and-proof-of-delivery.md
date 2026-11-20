<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0007: Save each message before sending it (outbox), and proof of delivery

- Status: **Accepted** (decided by the maintainer, 2026-11-12)
- Date: 2026-11-12
- Asked by: the maintainer, 2026-11-12 ("a delay is a remediation, not a
  fix"); option "Save message before sending" chosen the same day.

## Context

DSP 2025-1 changes a negotiation's state when the partner acknowledges a
message ("a state changes upon receipt and acknowledgement"). The partner
may send its next message before our side has recorded that
acknowledgement. Example, as consumer: we send `ACCEPTED`; the provider
records it, answers 200 and sends its agreement at once. The agreement can
arrive while we are still `OFFERED`, where an agreement is not allowed.

Release 0.3 PR 4a handled this with a wait: such a message waited up to
3 seconds for our acknowledgement. That hides the race but does not remove
it: a slow store, a slow network or a busy host still loses the message,
and the time limit is a guess.

Two other problems had the same root:

- A message being sent when the process stopped was lost; the negotiation
  stayed in its last state (ADR 0006, consequences).
- A send that failed (timeout, lost answer) ended the negotiation, even if
  the partner had received the message.

### Options considered

1. **Wait (as in 0.3 PR 4a).** Rejected: a time limit, not a rule.
2. **A lock or queue per negotiation**, so one message is handled only
   after the previous one is complete. Rejected: "complete" includes the
   partner's HTTP answer. If the partner sends its next message before it
   answers ours (DSP allows this, and the TCK does it), the partner waits
   for us and we wait for it until one side times out. It also needs
   shared state across replicas.
3. **One database transaction** around the state change and the send.
   Not possible: a transaction cannot include an HTTP call to another
   organisation.
4. **Save the message before sending it, and treat the partner's next
   message as proof of delivery.** Chosen.

## Decision

- **Outbox.** When a step decides to send a message, the message is saved
  with the negotiation as `pending`, in the same write as the state change
  that caused it. Only then is it sent. The state changes when the partner
  acknowledges it (`delivered`), as DSP requires.
- **Version check.** Every write is a compare-and-update on a `version`
  number of the negotiation (it replaces the compare on the previous
  state). If another write came first, the step reads the negotiation
  again and repeats, at most 3 times.
- **Proof of delivery.** A partner message that is not valid in the
  current state, but is valid in the state our pending message leads to,
  proves that the partner received our message: it could not have sent it
  otherwise. Both are applied in one write. No waiting.
  - As consumer, any provider message for a negotiation also proves our
    first request arrived (it carries the provider's process id).
  - While our own termination is pending, partner messages other than a
    termination are refused (HTTP 400).
- **Lost answers.** When a send fails after its retries, we ask the
  partner for the negotiation's state (`GET /negotiations/:providerPid` on
  the provider, `GET /:callback/negotiations/:consumerPid` on the
  consumer, both in the DSP 2025-1 HTTPS binding). If it is already in the
  state our message leads to, the message is taken as delivered. If not,
  as before: the negotiation is terminated (or, for a first request, ends
  without a message).
- **A failure only acts on its own message.** If the failed message is no
  longer pending (a partner message already proved it arrived), the
  failure changes nothing.
- **Restart.** At start, every pending message is sent again
  (`Negotiator::resume`).

The rules are in the domain (`ldsc-dsp`, provider and consumer services);
the store adapters only keep `pending` and `version`; the HTTP adapter
sends, checks the partner's state and resumes. SQLite schema version 3
adds the two columns to both tables.

### Amended in 0.11: which failures are retried (2026-11-17)

Until 0.11 a send was retried on any failure, also when the partner
answered with a 4xx status, a message it will never take
([TD-30](../RISKS_AND_TECHNICAL_DEBTS.md#td-30)). The maintainer decided
(ROADMAP 0.11, 2026-11-15): a send is retried only when no answer came
(connection failed, timeout, an answer that could not be read) or the
answer was 408, 429 or 5xx. Any other answer ends the tries at once.
What follows a failed send does not change: the partner's state is
still read, and a message it shows as arrived still counts as
delivered ("Lost answers" above).

A 401 is final because a Portitor partner answers 503, not 401, when it
could not check our credentials because a service was down (ADR 0016,
amended in 0.11). A failure without a status is tried again, also one
from our own side such as a refused URL; proof of delivery makes a
message that did arrive count once.

From 0.12 a Portitor partner answers 404, not 401, on the negotiation
and transfer routes when it refuses our credentials (ADR 0016, amended
in 0.12; Eclipse EDC does the same). The rule above already covers it:
a 404 is final after one try, the partner's state is then read (which a
partner that refuses us answers with 404 too), and the process ends. Our
log shows `HTTP 404`; only the partner's log says whether it refused us
or did not know the process.

### Amendment for 0.13: a transfer request settles a finalization that is on its way (2026-11-20)

The provider records `FINALIZED` only when the consumer acknowledged the
finalization event. The consumer records it on receipt, a moment
earlier. A transfer request sent in that moment found no finalized
agreement, got HTTP 400, and the consumer's transfer ended `TERMINATED`
([TD-44](../RISKS_AND_TECHNICAL_DEBTS.md#td-44); 2 to 4 times in 6,000
to 10,000 transfers in the 0.12 benchmark).

The maintainer decided (2026-11-20) to apply "Proof of delivery" above
to this case too:

- A `TransferRequestMessage` that names the agreement of a negotiation
  whose finalization event is pending (state `VERIFIED`, the event saved
  in the outbox, not yet acknowledged), sent by that negotiation's
  consumer, shows that the consumer acts on the agreement after
  verifying it. That is taken as enough; it does not prove that the
  event itself arrived (see "The cost" below). The negotiation is written
  as `FINALIZED` with nothing pending, exactly as its acknowledgement
  would write it, and the request is then checked as any other
  (ADR 0013).
- **The caller is checked first.** Nothing is written unless the caller
  is the negotiation's consumer and the agreement's `assignee`
  (ADR 0013, rule 2). Another caller gets the same 400 as for an
  agreement that does not exist.
- **Any other state is refused as before**: `AGREED` (not verified),
  `VERIFIED` with no message or with our termination pending,
  `TERMINATED`.
- **No lock and no wait.** The write is the compare-and-update on the
  negotiation's `version`. Two transfer requests at once: one write
  wins, the other reads the negotiation again and finds it `FINALIZED`.
  The acknowledgement at the same moment: whichever write comes first
  wins, the other changes nothing.
- **The event's later acknowledgement or failure is harmless.** The
  outbox still holds the event it is sending. Its acknowledgement, and
  its failure after the retries, find it no longer pending and change
  nothing ("A failure only acts on its own message"). It is not sent
  again after a restart.
- The negotiation is settled when the request names the agreement and
  the caller is its consumer, also if the request is then refused for
  another reason (the agreement no longer allows it, the format). What
  counts is that the consumer asked, not that the transfer is granted.

The cost, accepted: a consumer knows the agreement's id from the
agreement message, so one that asks for a transfer after it sent its
verification, but before it received the finalization event, is served
too. If that event is then lost and the provider restarts, the provider
shows `FINALIZED` and such a consumer `VERIFIED`. The same holds for a
consumer whose callback address cannot be reached: it never receives the
event, but can get `FINALIZED` and a transfer this way, and the outbox
then still logs and counts the event as given up
(`portitor.outbox.message.given_up`). A consumer that follows DSP asks only
after `FINALIZED`.

Compared with Eclipse EDC (checked by the integrator in EDC 1.0.0-RC8):
EDC accepts a transfer request as soon as the agreement record exists
and does not look at the negotiation's state
(`TransferProcessProtocolServiceImpl.fetchContractAgreement`; the SQL is
`SELECT * FROM <agreement table> where <id>=?`,
`BaseSqlDialectStatements.getFindContractAgreementTemplate`). EDC
records `FINALIZED` after the finalization event was dispatched
successfully (`NegotiationProcessorsImpl.processFinalizing`). So this
rule is stricter than EDC: Portitor still refuses a transfer under an
agreement that the consumer has not verified.

Whatever the outcome, the provider now logs a transfer request refused
for this reason: `transfer request is refused; answered 400` (INFO)
with `reason` "no finalized agreement with this id for the caller". The
line names neither the agreement nor the caller.

Where: the rule is in the domain
(`portitor-dsp`: `negotiation::provider::finalization_proved`,
`transfer::provider::TransferService::agreement_settling_finalization`); the `Agreements` port
got the read of a negotiation by agreement id and the versioned write;
the SQL is in the connector (`store/transfers.rs`) and uses the index of
migration 10, so there is no new migration.

## Consequences

- The 3 second wait is gone; messages that race their acknowledgement are
  accepted at once.
- Pending messages survive a restart and are sent again. The partner may
  therefore get a message twice. A partner that already applied it answers
  with an error; the state check then finds the partner in the expected
  state and the negotiation goes on.
- A partner that is already further than the state our message leads to,
  and whose next message never reached us, is not recognised by the state
  check; the negotiation is terminated as before. This needs two failures
  in a row and is visible in the logs.
- The state check trusts the partner's answer about its own state. Until
  DSP calls are authenticated (0.5, DCP), anyone who can answer at the
  partner's address can claim it; the same holds for every DSP message
  until then.
- Store adapters (a later PostgreSQL one too) must implement the version
  compare-and-update and list negotiations with a pending message.

## Note added in 0.14 (2026-11-20)

The text above is kept as it was decided. One sentence of the 0.13
amendment no longer holds: it says that, for a consumer whose callback
address cannot be reached, "the outbox then still logs and counts the
event as given up". From 0.14 a finalization event that a transfer
request settled is not counted in `portitor.outbox.message.given_up`.
The outbox logs at INFO that the message was not delivered and that the
process was settled without it (`Outbox::settled_without_delivery` and
`count_if_given_up` in `portitor-connector/src/outbox.rs`;
[TD-44](../RISKS_AND_TECHNICAL_DEBTS.md#td-44)). The rule itself is
unchanged.
