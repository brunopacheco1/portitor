<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Read the code in this order

A path through the code for a reader who does not write Rust every day.
It names real files and functions, so you can search for them. When a
line of Rust is hard to read, look the pattern up in
[RUST-IDIOMS](RUST-IDIOMS.md). For the pictures of what you read here,
see the C4 diagrams in [ARCHITECTURE](ARCHITECTURE.md#diagrams-c4-model).

Two rules make the code easier to follow
([ADR 0004](adr/0004-hexagonal-architecture.md)):

- A **domain crate** holds protocol rules and does no I/O. You can read
  it top to bottom without knowing HTTP or SQL.
- Whatever the rules need from outside is a **port**: a `trait`. An
  **adapter** implements it. To find the code behind a port, search for
  `impl <TraitName> for`
  ([RUST-IDIOMS, "Traits"](RUST-IDIOMS.md#traits-ports-and-adapters)).

All paths below start at `crates/`.

## Part 1: the crates, from pure to adapters

Read the first lines of each `lib.rs`: they say what the crate does.

1. `portitor-time/src/lib.rs`. Dates and times. One short file; a good
   warm-up.
2. `portitor-dsp/src/negotiation.rs`. The DSP negotiation states
   (`NegotiationState`), the messages, and one function that is the
   whole state machine: `next_state`. It is a single `match`. Read its
   test `state_machine_matches_the_specification` next to the figure in
   the DSP specification.
3. `portitor-dsp/src/policy.rs`. What Portitor understands of ODRL:
   `validate`, `permissions_hold`, `same_terms`.
4. `portitor-dps/src/port.rs` and `portitor-dps/src/state.rs`. The two
   ports between control plane and data plane (`DataPlane`,
   `ControlPlane`) and the data flow states. Small.
5. `portitor-dcp/src/id_token.rs` (`validate_self_issued`) and
   `portitor-dcp/src/credential.rs` (`Credential::verify`). How a token
   and a credential are checked. Leave the rest of `portitor-dcp` and
   all of `portitor-shacl` for when you need them.
6. `portitor-runtime/src/lib.rs`. The list of adapters the three
   binaries share. Read `sqlite.rs` (how a database file is opened and
   migrated) and skim `serve.rs` and `panics.rs`.
7. `portitor-connector/src/lib.rs`. The connector: its settings
   (`Config`), and `build`, which wires ports to adapters and lists
   every route. `portitor-identity` and `portitor-issuer` have the same
   shape: `main.rs`, `config.rs`, `lib.rs` with the routes, `store.rs`.

## Part 2: one request, end to end

A consumer asks a Portitor provider for a contract:
`POST /dsp/2025-1/negotiations/request`. The diagram is "C4 Code:
provider contract negotiation" in ARCHITECTURE.

**1. Route.** `portitor-connector/src/lib.rs`, function
`negotiation_routes`. The line with `"{n}/request"` sends this path to
the handler `negotiation::request_new`
([RUST-IDIOMS, "Web handlers"](RUST-IDIOMS.md#web-handlers-axum)).

**2. Who is calling.** Before any handler runs, `auth::require_caller`
(`portitor-connector/src/auth.rs`) checks the caller's DCP token through
the port `CallerVerifier`. A refused caller never reaches the handler.
The verified caller is handed on as a `Caller` value.

**3. Handler.** `request_new` in `portitor-connector/src/negotiation.rs`
does three things and holds no protocol rule:

- `provider.read(...)` runs the message past the `MessageGuard` port
  and turns the JSON into a `ContractRequestMessage`
  (`inbound::read_message`). Bad JSON stops here with HTTP 400.
- `provider.credentials(...)` asks for the caller's credentials if the
  offer needs some.
- `provider.service.request(...)` calls the domain.

**4. Domain service.** `ProviderService::request` in
`portitor-dsp/src/negotiation/provider.rs`. For a new negotiation it
calls `start`, which:

- checks that the offer is the one in the catalog (`check_offer`);
- asks the state machine for the next state (`next_state`, from Part 1);
- builds a `Negotiation` value with a new `provider_pid`;
- decides what the provider sends next (`next`, through the `Decider`
  port) and stores that message in the field `pending`;
- saves it: `self.store.insert(...)`.

`ProviderService<S, D, C, E>` has four type parameters. They are its
four ports: `S` the store, `D` the decider, `C` the catalog, `E` the
environment (clock and new ids). Tests pass fakes; the connector passes
the real adapters.

**5. Store.** The port is the trait `NegotiationStore`, in the same
file. Its SQLite adapter is `SqliteNegotiations` in
`portitor-connector/src/store.rs`: `insert` for a new negotiation and
`replace_if` for every later change. `replace_if` writes only if the
row's `version` is the one the caller read, so two messages that arrive
together cannot both win
([ADR 0007](adr/0007-outbox-and-proof-of-delivery.md)).

**6. Answer and outbox.** Back in the handler, `finish` answers the
consumer with HTTP 201 and hands the pending message to
`outbox::deliver` (`portitor-connector/src/outbox.rs`). The message was
saved in step 4, in the same write as the state. So a crash here loses
nothing: at the next start `Provider::resume` sends every pending
message again.

**7. Delivery.** In `outbox.rs`, `send_and_record` makes one round. It
calls `attempt`, which calls `send`: `send` runs the message past the
guard, gets a token (port `Prover`), posts the message (port `Sender`)
and waits between tries; `worth_retrying` decides whether a failure is
tried again. When the consumer acknowledges,
`ProviderService::delivered` moves the negotiation to its next state
with `replace_if`. If the consumer cannot be reached,
`ProviderService::abort` ends the negotiation.

The next messages of the same negotiation (`event`, `verify`,
`terminate`) take the same seven steps. In the service they go through
`handle`, which calls `update` in `portitor-dsp/src/process.rs`: it
reads the negotiation, changes it and writes it with `replace_if`. The
change applies the message with `process::receive`, which also takes
it as proof that our pending message arrived (ADR 0007), then asks the
decider for the next message (`next`).

## The three siblings

Negotiation as provider is one of four services with the same shape
([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)). Once you can read one,
you can read the others.

The steps they share are written once, in `portitor-dsp/src/process.rs`
(read its module comment first):

- `receive`: apply the counter-party's message; one that fits only the
  state our pending message leads to proves that message arrived, and
  both are applied (ADR 0007);
- `apply_sent`: apply our message once the counter-party has it;
- `update`: read, change, write with `replace_if`, and try again a few
  times when another write came first;
- `is_pending`, `can_abort`, `pending_messages`: what `delivered`,
  `abort` and `pending` check.

Each service implements two traits for the core: `Record` on its
record (`Negotiation`, `ConsumerNegotiation`, `Transfer`,
`ConsumerTransfer`: state, pending message, version) and `Records` on
itself (how it reads and writes its store, and its "not found"
answer). The states and transitions come from `ProcessState`
(`portitor-dsp/src/lib.rs`), which `NegotiationState` and
`TransferState` implement with their `next_state`. What stays in each
service file is what differs: its checks, its decider, the messages it
builds.

| | Domain service (`portitor-dsp/src/`) | Connector adapter (`portitor-connector/src/`) |
|---|---|---|
| All four | `process.rs` (`Record`, `Records`, `receive`, `update`) | `outbox.rs` (`Outbox`) |
| Negotiation, provider | `negotiation/provider.rs` (`ProviderService`) | `negotiation.rs` |
| Negotiation, consumer | `negotiation/consumer.rs` (`ConsumerService`) | `consumer.rs` |
| Transfer, provider | `transfer/provider.rs` (`TransferService`) | `transfer.rs` |
| Transfer, consumer | `transfer/consumer.rs` (`ConsumerTransferService`) | `consumer_transfer.rs` |

**A change to a shared step goes in `process.rs`** and reaches all four.
A change to one service's own rules (a check, a message) may still be
needed in its siblings: check the other three.

The connector keeps every store in SQLite (`store.rs`). Unit tests that
need a store in memory find the few that are left in
`portitor-connector/src/test_stores.rs`, which is compiled only for
tests; the integration tests use an in-memory SQLite database.

The consumer side starts at the management API, not at a DSP route:
`start` in `portitor-connector/src/management.rs` calls
`Negotiator::start` (`lib.rs`), which calls `ConsumerService::start`.
From there the steps are the same: state machine, store, outbox.

## Where to go next

- A transfer: `TransferService` adds two ports, `Agreements` and
  `DataFlows`. `DataFlows` leads to the data plane
  (`portitor-connector/src/data_flows.rs`, `data_plane.rs`). The
  provider's own steps (read, list, end a transfer, 1.1) are in
  `portitor-dsp/src/transfer/provider/own_steps.rs`; their management
  routes in `portitor-connector/src/management/provided_transfers.rs`.
- How a provider ends access (ADR 0027, 1.1): `running_flows.rs` (the
  per-flow switch that cuts running downloads), `flow_membership.rs`
  (the consumer's membership, kept with the flow and checked again),
  `data_plane/download.rs` (the body that can be cut, and the check
  while a download runs) and `auth/dcp/membership.rs` (the check itself,
  without the consumer).
- The DCP check of step 2 in full:
  `portitor-connector/src/auth/dcp.rs`.
- Datasets and the bundle: `portitor-connector/src/harvest.rs` and
  `bundle_harvest.rs`.
- Files above 1,000 lines and the macros that write types are known
  debts ([R-01](RISKS_AND_TECHNICAL_DEBTS.md#r-01)); read them last.

When a file or a function named here is renamed or moved, change this
page in the same pull request.
