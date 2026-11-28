<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Changelog

All notable changes are listed here, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/); until 1.0, minor
versions may change behaviour and configuration.

## [Unreleased]

Portitor 1.5 (in progress): the HDAB and Schuman Clinical Research.

### Documentation

- ROADMAP: a management UI for operators is planned for 1.7, after
  the user login of that release; no UI before.
- ROADMAP and risks: the 1.5 plan approved by the maintainer
  (2026-11-28). TD-35 moves to 1.9, TD-11 to 1.6, TD-12 and the permit
  scenario (way C) to 1.10. New items: R-51 (no delegation from a data
  user to the HDAB that the holder can verify) and TD-79 (the
  identity's status check names an unknown status bit SUSPENDED and
  checks a REVOKED credential again).
- Research note [1.5-dpv-ehds-purposes](docs/spikes/1.5-dpv-ehds-purposes.md)
  (R-40): DPV EHDS 2.3 names the point of EHDS Article 53(1) for each
  of its secondary use purposes and covers every point; it is a draft,
  and HealthDCAT-AP does not use it. The two purpose IRIs of the docs
  and scenarios are right. A dated note in ADR 0009 corrects its 1.4
  remark.
- [ADR 0031](docs/adr/0031-agreements-that-depend-on-an-upstream-agreement.md),
  Proposed (R-39, way B): no specification or product (DSP 2025-1,
  ODRL 2.2, EDC, Tractus-X, Simpl-Open) links a downstream offer to an
  upstream agreement, so the hdab-intermediary scenario keeps the
  HDAB's offer inside its agreement by hand; no code. ADR number 0030
  is not used (a withdrawn draft).
- [ADR 0032](docs/adr/0032-negotiating-on-a-data-users-behalf.md),
  Proposed (R-51, way D): DSP 2025-1, DCP v1.0.1, VC 2.0, EDC,
  Tractus-X, Simpl-Open and the EHDS define no way for the HDAB to
  negotiate an agreement for a data user that the holder can check;
  ADR 0013 stays (the assignee is the caller), and the ADR lists what
  delegation would need. No code.

## [1.4.0] - 2026-11-27

Portitor 1.4: purpose and data permits in policies. Offers can name a
purpose, datasets can be hidden behind an access policy, issuers can be
trusted for some credential types only, and the identity stops
presenting a revoked credential by itself. Every 1.3 route, setting and
file format still works, and every new setting is optional
([STABILITY](docs/STABILITY.md)). One behaviour changes without a new
setting: the identity reads the status lists of the credentials it
holds every 60 seconds and no longer presents a revoked one (set
`PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS=0` to keep the 1.3
behaviour).

**Upgrading from 1.3.** Back up first: migrations run at start, forward
only. Stop every 1.3 replica before the first 1.4 replica starts. The
identity now calls issuers' DID documents and status lists. Details:
[OPERATING, From 1.3 to 1.4](docs/OPERATING.md#from-13-to-14).

### Added

- Policies: the left operand `purpose`, with `eq` or `isAnyOf` and
  purposes as IRIs. It is a term of the agreement and is not evaluated;
  a contract request with another purpose is refused, as other changed
  terms are ([ADR 0009](docs/adr/0009-minimal-odrl-policies.md),
  amendment in 1.4; [TD-12](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-12),
  [DEPLOYING](docs/DEPLOYING.md#policies)). The hdab-intermediary
  scenario: Lux Biobank's offer for EU processors names a purpose, and
  a request with another purpose is refused.
- Access policy per dataset: the optional `portitor:accessPolicy` in
  the datasets file says who may see a dataset. It is checked with the
  caller's credentials on DSP catalog and dataset requests (a dataset
  hidden from the caller is left out, or answered as unknown) and, as
  in EDC, on every contract request (an offer of a hidden dataset gets
  the answer of an unknown offer). It is never shown. The provider asks
  for the access policies' scopes in a second presentation query; when
  that query fails, the caller keeps the credentials of the first check,
  and a credential service that does not answer (or answers 408, 429,
  5xx) makes a catalog or dataset request answer HTTP 503 with a
  `CatalogError`, so the partner tries again rather than see a catalog
  with datasets missing. A database migration adds the column
  `dataset.access_policy`. The connector's management API takes an
  optional `additionalScopes` on `POST /v1/catalog/dataset/request`, as
  on `POST /v1/catalog/request`, to read a dataset behind an access
  policy ([ADR 0019](docs/adr/0019-credential-constraints.md),
  amendment in 1.4; [TD-13](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-13),
  [DEPLOYING](docs/DEPLOYING.md#datasets-file)). A catalog request asks
  every caller for the scopes of all access policies
  ([R-50](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-50)).
- Setting `PORTITOR_TRUSTED_ISSUER_TYPES` on the connector and the
  identity: limits a trusted issuer to a list of credential types, as
  EDC's `supportedtypes`; an issuer not named keeps every type
  ([TD-74](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-74)). A limited issuer
  must be trusted for each type of a credential except
  `VerifiableCredential`, as EDC's `HasValidIssuer`. The connector skips
  any other credential before it fetches anything for it, and checks a
  membership kept with a transfer again for each of its types (one kept
  before 1.4 counts as a `MembershipCredential`). The identity refuses
  such a credential in a delivery and such a type in an offer (`400`,
  with the reason), and does not ask a limited issuer for another type
  ([ADR 0019](docs/adr/0019-credential-constraints.md), amendment in
  1.4; [R-48](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-48),
  [DEPLOYING](docs/DEPLOYING.md#trusted-issuers-by-type)). The
  data-permits scenario, step 5, is now a refusal.
- The identity checks the status of the credentials it holds: every
  `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS` (new setting, default 60,
  0 off) it reads their status lists and no longer presents a revoked
  or suspended credential; a lifted suspension presents it again. A
  list that cannot be read now (no answer, 408, 429, 5xx) keeps the
  last status; a list that cannot be used for another reason (404,
  another signer, an issuer DID that cannot be resolved) makes the
  credential `UNCHECKABLE`, not presented, as verifiers would refuse
  it. Each round reads a list once. One replica runs each round, under
  the lease `job`/`credential_status_check`. `GET /v1/credentials` of
  the identity's management API shows `status` and `statusCheckedAt`. A
  database migration adds the columns `credential.status_state` and
  `credential.status_checked_at`. The status list reading and its cache
  moved from the connector to `portitor-runtime` (`status_lists.rs`),
  unchanged. So a revoked credential need not be deleted by hand any
  more: in the data-permits scenario, step 4, the transfer under the
  second permit works once Schuman's identity has checked
  ([TD-20](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-20),
  [ADR 0029](docs/adr/0029-several-replicas.md), note added in 1.4;
  [DEPLOYING](docs/DEPLOYING.md#getting-a-credential)).
- Settings `PORTITOR_BLOCK_INTERNAL_ADDRESSES` (default `false`) and
  `PORTITOR_INTERNAL_ADDRESSES_ALLOWED` on all three components: refuse
  internal addresses in URLs a partner chooses: those IANA's
  special-purpose registries mark as not globally reachable, multicast
  and broadcast, and an IPv6 address that carries one, such as NAT64 or
  6to4. The address checked is the one the call connects to. A refused
  call is not sent and counts as a partner that cannot be reached; its
  error in the log names the setting that allows it. URLs the operator
  chooses are not checked, the Gaia-X compliance service's DID
  (`PORTITOR_GAIAX_COMPLIANCE_DID`) among them. Each binary builds its
  clients to other parties in a library function that `main.rs` calls
  (`ports_for_partners`, `Ports::for_partners`), so tests check the
  block as the binaries set it
  ([ADR 0016](docs/adr/0016-dsp-caller-authentication.md), amended in
  1.4; [R-17](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-17),
  [DEPLOYING](docs/DEPLOYING.md#internal-addresses)).
- Issuer management route `PUT /v1/participants/{did}`: replaces a
  registered participant's claims without deleting the registration;
  issued credentials keep theirs, a request decided from then on gets
  the new ones. The data-permits scenario, step 3, uses it
  ([TD-46](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-46),
  [DEPLOYING](docs/DEPLOYING.md#registering-participants-and-issuing)).

### Changed

- As consumer, the connector accepts a provider's offer (DSP `OFFERED`)
  with credential constraints it understands, without evaluating them,
  as Eclipse EDC; the provider checks them at the acceptance
  ([TD-11](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-11)).
- A caller's token is checked before its DID is resolved: the
  identity's presentation API checks the access token and the ID
  token's `aud` first, and the connector's DSP check and the identity's
  and issuer's DCP checks refuse a token for another verifier, out of
  its time, or whose `iss` is not its `sub`. Such a token makes them
  fetch nothing; a caller without a valid access token gets the same
  401 as before (F-06 in
  [SECURITY-ASSURANCE](docs/SECURITY-ASSURANCE.md),
  [ADR 0016](docs/adr/0016-dsp-caller-authentication.md), amended in
  1.4).
- A partner no longer learns about our network from a termination: a
  push to an endpoint the block on internal addresses refuses ends with
  the reason "the consumer's endpoint is not allowed", and a transfer
  ended because the consumer's membership no longer holds with "the
  consumer's membership no longer holds"; the details (host, address,
  status list) are in our log only.
- Code, no change of behaviour: `spawn_every` in `portitor-runtime`
  runs a background round on a timer, used by the connector's outbox
  loop, transfer step sweep, dataset harvest and bundle check, and by
  the identity's status check
  ([TD-70](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-70)); the identity's
  and the issuer's `DatabaseStore` share one wrapper, `ComponentStore`
  in `portitor-runtime`, with the used token ids and leases written
  once ([TD-71](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-71)).
- Code: the 1.4 readability review (2026-11-27), no change of
  behaviour of the binaries: plainer names (`sender_to_providers`,
  `Link::to_consumers`, `negotiation_requested_by`,
  `last_status_check`, `authenticate_trusted_issuer`,
  `resolve_did_document`), an enum for the consumer's view of a
  constraint, one `add_once` helper, a plainer refusal text for
  internal addresses, and three more patterns in
  `docs/RUST-IDIOMS.md`. The catalog port `CatalogDatasets` no longer
  has default bodies that answered "no access policy": an adapter must
  write them, so one that forgets cannot show every dataset to every
  caller. New debt [TD-75](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-75).

### Fixed

- A first contract request sent again by the same consumer (same
  `consumerPid`, our answer lost) gets the negotiation it already
  started, with the same status, instead of starting a second one; a
  repeat with another offer or callback address gets 400. Two replicas
  that get the repeat at once make one negotiation. A database
  migration adds the index `negotiation_request` on `negotiation`
  ([TD-60](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-60)).
- The outbox counts a refused message as delivered only when the
  partner's state is the one the message leads to **for the same
  process**: `providerPid` and `consumerPid` are compared too
  ([TD-61](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-61)).
- PostgreSQL: a transaction that collided with another one (SQLSTATE
  `40001` or `40P01`) now runs again after a random pause that doubles
  with each collision (from up to 5 ms to at most 200 ms), for up to 5
  seconds or 50 tries, and gives its connection back to the pool while
  it waits. Before, both sides waited the same pause (5 ms more for
  each try) and gave up after 10 tries, which busy replicas could reach
  ([ARCHITECTURE](docs/ARCHITECTURE.md#storage)).

### Dependencies

- `tower-service` 0.3 is now a direct dependency of `portitor-runtime`
  (the check of internal addresses); it was already in the tree through
  `hyper-util` and `axum`. No new crate version: still 185 cargo-vet
  exemptions ([R-04](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-04)).

### Documentation

- ADR amendments of 2026-11-27: [ADR 0009](docs/adr/0009-minimal-odrl-policies.md)
  (purpose), [ADR 0016](docs/adr/0016-dsp-caller-authentication.md)
  (internal addresses, the order of checks),
  [ADR 0019](docs/adr/0019-credential-constraints.md) (access policies,
  trusted issuers by type, negative operators) and
  [ADR 0029](docs/adr/0029-several-replicas.md) (no copy tool; the
  credential status check).
- DEPLOYING, "Credential constraints": `neq` and `isNoneOf` are not
  deny lists ([R-43](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-43)).
- The SQLite to PostgreSQL copy tool announced for 1.4 is dropped
  (maintainer, 2026-11-27): no deployment needs one yet. A component
  moved to PostgreSQL starts empty; SQLite is still removed in 2.0
  ([OPERATING, From SQLite to PostgreSQL](docs/OPERATING.md#from-sqlite-to-postgresql),
  [TD-59](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-59)).
- TD-73 (a negotiation step slower on SQLite in 1.3): the statements of
  one provider negotiation counted against 1.2.0. The cause is the new
  commits of 1.3 (a lease claim and release for each message sent, and
  the used token id of each DCP token), not a bug; moved to 1.12 with
  TD-40 ([TD-73](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-73)).
- The OpenSSF Baseline level 1 self-assessment checked again against
  v2026.08.28 on 2026-11-27, with the GitHub settings read through the
  API: 21 met, 1 to confirm, 1 not met (the website's "Enforce HTTPS"
  is off), 1 not applicable
  ([SECURITY-ASSURANCE](docs/SECURITY-ASSURANCE.md#level-1-self-assessment),
  [R-05](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-05)).
- A research note on checking that a transfer's destination matches a
  data permit (maintainer, 2026-11-27: research only in 1.4). No
  specification or product does it; the note proposes checking the
  receiving participant's DID in 1.5 or later
  ([spike](docs/spikes/1.4-destination-matches-permit.md)).
- RISKS_AND_TECHNICAL_DEBTS: R-48, TD-13, TD-20, TD-46, TD-47 (fixed in
  1.1), TD-60, TD-61, TD-70 and TD-71 closed; R-17, TD-11 and TD-12
  narrowed; new R-50, TD-74 to TD-78. ROADMAP, TESTING (the 1.4 runs of
  the scenarios and the EDC interop test), STABILITY, ARCHITECTURE,
  SECURITY-ASSURANCE (F-01, F-06, F-08), the C2 and C3 diagrams, the
  website.

## [1.3.0] - 2026-11-27

Portitor 1.3: several containers of each image (connector,
identity, issuer) share one PostgreSQL database, and SQLite is
deprecated ([ADR 0029](docs/adr/0029-several-replicas.md)). Every 1.2
route, setting and file format still works
([STABILITY](docs/STABILITY.md)).

**Upgrading from 1.2.** Back up first: migrations run at start, forward
only. A component on a SQLite file keeps working, for one process; a
second process on the same file now refuses to start. To use
PostgreSQL, start with an empty database: the tool that copies a
SQLite file into PostgreSQL comes in 1.4
([TD-59](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-59)). Several replicas
need PostgreSQL and the two-step key rotation. Details:
[OPERATING, From 1.2 to 1.3](docs/OPERATING.md#from-12-to-13).

### Added

- PostgreSQL 16 to 19 as the database of each image, behind the same
  store ports. The database layer is in `portitor-runtime`
  (`database.rs`, `database/`): SQL written once for both databases,
  `SERIALIZABLE` transactions on PostgreSQL that run again when two
  collide, a PostgreSQL migration list per component that runs under an
  advisory lock (replicas that start together migrate once), and one
  database per component (`portitor_schema`). The client is
  `tokio-postgres`, with Portitor's own TLS on `tokio-rustls` and the
  system's CA certificates. Each process opens at most 10 connections.
- Settings `PORTITOR_DATABASE_URL` (a libpq URI; `sslmode`
  `verify-full` by default, `verify-ca` and `require` also check the
  host name, `disable` turns TLS off, `prefer` and `allow` are refused;
  `sslrootcert`, as in libpq, a PEM file whose CAs are then the only
  ones trusted, or `system`; without it the image's CAs) and
  `PORTITOR_DATABASE_PASSWORD_FILE`.
- Timeouts on PostgreSQL connections, each settable in the URL as in
  libpq: `connect_timeout` (10 s), a `statement_timeout` (30 s, in
  `options`), `tcp_user_timeout` (30 s) and TCP keepalives (after 15 s
  idle, every 5 s). A call without an answer after 40 s fails and its
  connection is closed, so a connection to a server that went away (a
  failover, a network cut) no longer blocks a request or a lease
  renewal without limit.
  A URL that holds a password is refused, and so is a start with both
  `PORTITOR_DATABASE_URL` and `PORTITOR_DB_FILE`
  ([DEPLOYING, Database](docs/DEPLOYING.md#database)).
- CI runs the store tests on SQLite and on PostgreSQL 16 and 19, and
  the TLS test of the database layer on a PostgreSQL 16 with TLS
  ([TESTING, PostgreSQL](docs/TESTING.md#postgresql)). The schema test
  of each component compares column types, `NOT NULL`, primary keys and
  unique indexes of the two databases, and a test per component opens a
  SQLite file of 1.2 with 1.3. Test databases are dropped after their
  test.
- A replica test, `scenarios/two-party/run-replicas.sh`, run by CI on
  PostgreSQL 19: two processes of each image, each pair on one
  database, behind a round-robin proxy (`scripts/round-robin-proxy.py`).
  It stops a process in the middle of a negotiation, a transfer and a
  credential delivery and checks that the other process finishes, then
  runs the two-party steps with every request spread over both
  ([TESTING, Several replicas](docs/TESTING.md#several-replicas)).
- `scenarios/two-party/replicas.yaml`: a Compose override that runs the
  demo's provider as two connectors behind nginx, for the test before a
  release ([TESTING](docs/TESTING.md#two-connectors-behind-nginx)).

### Deprecated

- SQLite and `PORTITOR_DB_FILE`. A component on a SQLite file still
  works, for one process, and logs at start `SQLite is deprecated since
  1.3 and is removed in 2.0`. Both are removed in 2.0. A tool that
  copies a SQLite file into PostgreSQL comes in 1.4
  ([TD-59](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-59)).

### Changed

State that one process held is now in the database, so several
replicas give the same answers (ADR 0029, decision 6). One entry per
change:

- Used token ids (`jti`) are kept in the database of each image (table
  `used_token`) until their token expires, so a token used on one
  replica is refused on the others, also after a restart. The limit of
  100,000 ids in memory is gone
  ([TD-09](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-09)).
- The connector reads the catalog, and each dataset that an offer, an
  agreement, a transfer request or a data flow start needs, from the
  database on each request; the copy of the datasets in memory is gone.
  When the database cannot be read, a catalog or dataset request gets
  HTTP 500 (`internal error`), and a contract or transfer request HTTP
  500 instead of a refusal. The catalog's content and order are
  unchanged ([TD-50](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-50)).
- The issuer stores each signed status list in its database (table
  `status_list`, with the id of the signing key). A revocation signs
  the new list in the same transaction, and every replica serves the
  stored list. At start a list is signed only when none is stored or
  another key signed it (a key rotation)
  ([TD-51](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-51)).
- Each running download and push reads its flow's state every
  5 seconds, so a suspension or termination handled by another
  container cuts it within 5 seconds
  ([TD-52](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-52)).
- The connector's background work runs under leases in its database
  (table `lease`, as Eclipse EDC's lease columns; 30 seconds, renewed
  while the work runs). Each saved DSP message is sent under a lease,
  and a loop in every process, every second, sends the pending messages
  that nobody leases, instead of only at start
  ([TD-53](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-53)). A push runs under
  a lease, and the same loop starts the pushes that nobody leases; a
  second upload of one transfer gets 409 also when the first runs on
  another container ([TD-54](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-54)).
  The scheduled dataset harvest runs once per interval among the
  processes, and its job ids come from the database
  ([TD-56](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-56)).
- The identity's Gaia-X run, scheduled or started by the operator, runs
  under a lease too. The identity no longer marks every `RUNNING` run as
  failed at start: the next run that takes the lease marks those of a
  process that stopped (error `its process stopped`)
  ([TD-56](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-56)).
- The connector's bundle version in use is recorded in its database
  (table `bundle_in_use`): the process that takes a new version records
  it, and every other process moves to it at start and within 5 seconds
  after, from its own cache folder or downloaded. A version issued
  before the recorded one is never recorded, except a fixed
  `PORTITOR_BUNDLE_VERSION`
  ([TD-55](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-55)).
- The issuer takes a lease of 30 seconds on a credential delivery
  before it sends it, so two replicas do not both send it; a process
  that stops leaves the lease to run out, then another one sends it
  ([TD-57](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-57)).
- A second process on the same SQLite file refuses to start, with `the
  SQLite file ... is in use by another process`. The process that uses
  the file holds a lease row in it (`portitor_process_lease`, made by
  `portitor-runtime` in all three images), renewed every 10 seconds and
  removed when it stops; after a crash the next process starts at most
  30 seconds later (ADR 0029, D3).
- The images no longer set `PORTITOR_DB_FILE` (it would conflict with
  `PORTITOR_DATABASE_URL`). The SQLite defaults stay `/data/...`; the
  connector image now runs in the working directory `/data`, so a
  relative path in a connector setting is read from there.
- The demo, the quickstart and the scenarios in Docker Compose
  (two-party, data-permits, hdab-intermediary, gaiax-compliance,
  edc-interop) keep their state in PostgreSQL: one `postgres` 19
  container (pinned by digest) with one database and role per
  component, made by `examples/postgres/init-databases.sh`; TLS with
  the server's certificate and host name checked against a test CA made
  for each run (`scripts/demo-database.sh`), and the passwords in
  Compose secrets. Each scenario's changelog has the details.
  `init-databases.sh` reads each password from its file inside `psql`,
  never on a command line, and, when the server runs with TLS, lets
  connections over the network in only with TLS (`hostssl` in
  `pg_hba.conf`).
  `two-data-spaces`, which runs processes, uses PostgreSQL when
  `PORTITOR_TEST_DATABASE_URL` names a server (`scripts/test-databases.sh`).
- The DSP TCK run in CI keeps the connector's state in PostgreSQL 19.
- A lease is held by one claim (the process id and a random id), not
  by the process. Work whose lease is lost stops: when another holder
  took it over, or when no renewal succeeded for about 20 seconds (a
  database failover). A push or an upload is cut (the upload is
  answered 503), a message is left to the process that took it over, a
  Gaia-X run is recorded failed and stores nothing. In one process, a
  push of a transfer never starts while another one of it runs (as in
  1.2). An upload whose lease cannot be taken because the database does
  not answer gets 503, not 409.
- Dataset harvest runs of several connector containers no longer undo
  each other: a run (at start, after new rules, a bundle job, or
  scheduled) is written only if no other run was written since it read
  the store, and is otherwise planned again from the stored datasets.
  The connector's database gets a migration: a `version` column in
  `harvest_state` and an index on `lease.lease_until`.
- The issuer's outbox reads the clock again for each delivery it
  claims and records, not once per round: after a slow try, the next
  delivery's lease could end at once and another issuer container send
  it too.
- On SQLite, a transaction takes the file's write lock when it starts
  (`BEGIN IMMEDIATE`). A transaction that read and then wrote failed at
  once with `database is locked` when the lease thread of 1.3 wrote the
  file in between.
- On PostgreSQL, the connector's keys that lists are paged by
  (negotiations, transfers, data flows) sort by their bytes
  (`COLLATE "C"`), as on SQLite, whatever the server's collation.

### Dependencies

- `tokio-postgres` 0.7 and the crates it brings: 32 crate versions not
  covered by the imported cargo-vet audits, exempted
  ([R-04](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-04)).
- cargo-vet: the six delta audits recorded before 1.3 are replaced by
  exemptions, because an audit must mean a person read the code
  (maintainer, 2026-11-26). Now 57 crate versions are covered by imported
  audits, 1 partly, and 185 are exempted; `supply-chain/audits.toml` is
  empty ([CONTRIBUTING](CONTRIBUTING.md#dependency-audits-cargo-vet)).
- regex 1.13.1 to 1.13.2 (with regex-automata 0.4.19 and regex-syntax
  0.8.12; Dependabot #96) and jsonschema 0.61.2 to 0.63.0 (test-only;
  Dependabot #97); their cargo-vet exemptions moved to the new versions.

### Documentation

- ADR 0029 "Several replicas on one PostgreSQL database"; ADR 0006 gets
  a dated note. DEPLOYING: a "Database" section, a Compose example of
  the database, and PostgreSQL in the first deployment and the
  `docker run` examples. OPERATING: backup with `pg_dump`, a two-step
  rolling key rotation for several replicas
  ([TD-58](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-58)), when status lists
  are signed again after a key change, upgrades with several replicas,
  and moving from SQLite to PostgreSQL. STABILITY: PostgreSQL, and
  SQLite deprecated. ARCHITECTURE: principle 14 met by design, and a
  "Storage" section. The C2, C3 and deployment diagrams show each
  image's database as PostgreSQL, shared by the replicas, or a SQLite
  file (deprecated). ROADMAP, RISKS_AND_TECHNICAL_DEBTS (TD-08, TD-09
  and TD-50 to TD-58 done on the 1.3 branch; new R-49 and TD-59),
  TESTING (store tests on PostgreSQL; "What runs where" with the
  replica test, the DSP TCK on PostgreSQL and the nginx test; the
  scenarios' database), SECURITY-ASSURANCE.

## [1.2.0] - 2026-11-26

Portitor 1.2: the built-in data plane in its own crate, the first step
to a separate, pluggable data plane (ADR 0028). Every 1.1 route,
setting and file format still works; one change of behaviour: a
transfer keeps the data source it started with. Upgrading from 1.1:
[OPERATING, From 1.1 to 1.2](docs/OPERATING.md#from-11-to-12).

- Version 1.2.0. The website quickstart clones `v1.2.0`, and the
  scenarios that pull release images (two-party, hdab-intermediary,
  data-permits) default to the 1.2.0 images.

### Changed

- The built-in data plane is its own crate, `portitor-dataplane`
  ([ADR 0028](docs/adr/0028-separate-pluggable-data-plane.md), step 1).
  The connector links it in as before: no route, setting or image
  changes. The data plane no longer reads the connector's stores. It
  asks the control plane through ports: `AccessGate` for the
  membership check, still on every request
  (`MembershipGate` in `flow_membership.rs` implements it), and
  `FlowStore` for its own flows. The memberships kept with each flow
  are now behind the connector's `MembershipStore`. The data plane's
  `lib.rs` is split into one file per topic
  ([R-01](docs/RISKS_AND_TECHNICAL_DEBTS.md#r-01)).
- The control plane names the dataset's source in the Data Plane
  Signaling start message, in `metadata` as
  `{"portitor:source": "<url>"}` (the key of the datasets file; Data
  Plane Signaling v1.0-RC4 names no key for this). The data plane checks
  it at start, as before, and the data flow keeps it until it ends.
  **Behaviour change:** a `portitor:source` changed in the datasets file
  reaches only transfers that start after the next harvest run; a
  running transfer keeps the source it started with, also when it is
  suspended and started again.
- Each data flow keeps the agreement and the consumer its start
  message names (`agreementId`, `counterPartyId`), for the audit line.
  The data plane no longer reads the transfer store for it, so the WARN
  line `audit: transfer store failed` is gone. The audit line itself
  does not change.
- The data plane's log lines (all but the audit line) have the target
  `portitor_dataplane::...` instead of `portitor_connector::...`; their
  texts do not change.
- CI checks with `cargo tree` that `portitor-dataplane` does not depend
  on `portitor-connector`.
- Scenarios: a failed management call in `scripts/scenario-lib.sh`
  (`api`, used by hdab-intermediary and data-permits) now shows its
  HTTP status and answer, or curl's error, instead of failing silently.

### Fixed

- The provider tells the data plane after the transfer's write, once
  ([TD-48](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-48); ADR 0028, decision
  6). Before, the data plane was called inside the retried
  read-change-write of a transfer, so a step could run although its
  write lost, or run twice. Now the write records the step it needs
  (`transfer.flow_step`), the same call runs it right after the write,
  and our DSP message waits until it has run. While the step runs, the
  transfer is leased in the database (holder: a random id of the
  process; 30 seconds), taken in the write that records the step and
  given back in the write that records that it ran. Another write of
  that transfer, from this process or another one sharing the
  database, is refused at once: a consumer's message gets HTTP 503 with
  the DSP `TransferError` (code `503`, reason `the transfer is being
  changed; retry`) and `Retry-After: 1`, and so does
  `POST /v1/provided-transfers/{providerPid}/termination` (a problem
  document). This 503 is new: in 1.1 such a message waited. Eclipse EDC
  answers the same way and retries a 5xx; DSP 2025-1 itself names only
  4xx refusals ([ADR 0007](docs/adr/0007-outbox-and-proof-of-delivery.md),
  note added in 1.2). Our own background calls (recording an
  acknowledgement, the end of a push, a membership that no longer
  holds) try again for a little more than one lease instead. A step
  whose lease has ended (its process stopped, or recording it failed)
  is taken over by the next call that changes the transfer, by a sweep
  every 10 seconds, and at start; running it twice is safe. The state
  change of a transfer is logged and counted once its step has run,
  from the state kept with the step in the database, so a write taken
  back logs nothing, as before, and any process can report it. A step
  the data plane refuses still ends the transfer when it is the
  provider's own decision or a step taken over; for a consumer's
  message, an undelivered message, the provider's own termination or
  the end of a push, the write is taken back and the caller gets the
  failure, as before. A transfer request whose insert fails or loses to
  a repeated request no longer starts a data flow (before: started,
  then terminated).
  Only when the database fails: if a step ran but recording that it
  ran failed twice, the caller gets an error although its write landed,
  and the step runs again once its lease has ended; a transfer whose
  step cannot be taken up is skipped and logged (`transfer not taken
  up`, ERROR) instead of stopping the connector.

### Upgrade notes

- Database migrations 14, 15, 16 and 17 (forward only) add
  `data_flow.source`, `data_flow.agreement_id` and
  `data_flow.counter_party_id`, `transfer.flow_step`, and the lease of a
  transfer (`transfer.state_before_step`, `transfer.lease_holder`,
  `transfer.lease_until`, and the partial index
  `transfer_recorded_step`). Flows of 1.1
  get the source their dataset has in the harvest's read copy, and the
  agreement and consumer of their transfer. A 1.1 flow whose dataset had
  no `portitor:source` keeps none and is refused, as in 1.1: 401 for a
  pull, a push transfer is terminated. Back up `/data` first:
  [OPERATING, From 1.1 to 1.2](docs/OPERATING.md#from-11-to-12).

### Documentation

- 1.2 documentation: OPERATING (upgrade from 1.1 and the log lines),
  DEPLOYING (a changed source reaches only new transfers), ADRs 0007,
  0013, 0014, 0026 and 0028 (dated notes), the C3 and C4 diagrams,
  ARCHITECTURE, the reading guide, SECURITY-ASSURANCE (the data plane's
  files), RUST-IDIOMS (the patterns the 1.2 code uses), TD-48 closed in
  RISKS_AND_TECHNICAL_DEBTS, and the scenario READMEs, which now give
  the roadmap's new release numbers.
- TESTING: the 1.1.0 run of all six scenarios on Linux before the tag,
  and how to build the images with Podman 4.9, whose Buildah does not
  know `HEALTHCHECK --start-interval`.
- Spike report `docs/spikes/1.x-negotiation-state-machine.md`: the
  contract negotiation states and transitions in DSP 2025-1 terms, what
  is stored for each, a comparison with EDC 1.0.0-RC8, and a check of
  the stored records under every delivery order. Roadmap entry marked
  done. New UML state diagram
  `docs/diagrams/state-negotiation.drawio.svg`, shown in ARCHITECTURE.
  TD-49 (Accepted): after a failed message the two parties of a
  negotiation can stay in different states, as in EDC.
- Several replicas (maintainer, 2026-11-26): ARCHITECTURE principle 14
  (asynchronous, stateless APIs, coordination in the database; not met
  yet in 1.2) and a rule for contributors in AGENTS.md. ROADMAP: a new
  release, 1.3 "Several replicas and PostgreSQL"; the releases planned
  after it move up by one (purpose and data permit are now 1.4, the
  last is 1.11). RISKS_AND_TECHNICAL_DEBTS: TD-50 to TD-58, from a code
  review of the state held in one process; TD-08 and TD-09 planned for
  1.3.

## [1.1.0] - 2026-11-25

Portitor 1.1: push transfers, data in object storage, and a provider
that can always end access. Every 1.0 route, setting and file format
still works; this release only adds
([STABILITY](docs/STABILITY.md)). Before the tag, CI ran the two-party
scenario with RustFS (a push landed in the consumer's bucket, equal to
the file) and the EDC interop test with EDC v1.0.0-RC8 asking Portitor
to push. Upgrading from 1.0:
[OPERATING, From 1.0 to 1.1](docs/OPERATING.md#from-10-to-11).

- Version 1.1.0. The website quickstart clones `v1.1.0`, and the
  scenarios that pull release images (two-party, hdab-intermediary,
  data-permits) default to the 1.1.0 images.

### Added

- Push transfers (`HttpData-PUSH`, ADR 0027, TD-10). As provider: a
  dataset offers push with a distribution in that format; the transfer
  request must carry an `http-push` data address
  (`https://w3id.org/dspace-sig/profile/http-push`, an `https://`
  endpoint unless `PORTITOR_ALLOW_HTTP`, `authType` `bearer`), else 400.
  Once started, and once the consumer no longer reports `REQUESTED` (at
  most 30 seconds: Eclipse EDC applies a start after it acknowledges
  it), the data plane sends the dataset as one `POST` with the
  consumer's bearer token and the source's `Content-Type`; 2xx completes
  the transfer, anything else terminates it with the status code as the
  reason. Suspend and terminate cut a running push; a start again sends
  it again from the beginning and may carry a new address.
- As consumer: `POST /v1/transfers` takes `"format": "HttpData-PUSH"`
  when object storage and `PORTITOR_PUSH_BUCKET` are set (else 400). The
  connector gives `{PORTITOR_PUBLIC_URL}/push/{consumerPid}` with a
  random bearer token (only its SHA-256 is kept). New open routes
  `POST /push/{consumerPid}` and `PUT /push/{consumerPid}` stream the
  data into the push bucket as the object `{consumerPid}` (multipart
  upload); every refusal is the same 401. The transfer view gains
  `objectKey`.
- Pull from the provider's object storage: `portitor:source` may be
  `s3://{bucket}/{key}`, read with a signed `GET` (AWS Signature Version
  4, path-style), with the same 401 and 502 answers as an HTTP source.
- Object storage settings, the AWS SDK names: `AWS_ENDPOINT_URL_S3`,
  `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
  `AWS_SESSION_TOKEN` (temporary keys, sent and signed as
  `x-amz-security-token`), and `PORTITOR_PUSH_BUCKET`
  ([DEPLOYING, Object storage and push transfers](docs/DEPLOYING.md#object-storage-and-push-transfers)).
- The audit line `data transfer done` is also written for each push sent
  and each push received.

- Management API: `POST /v1/transfers/{consumerPid}/suspension` and
  `POST /v1/transfers/{consumerPid}/start` suspend a transfer as
  consumer and start it again, with the rules of `/completion` and
  `/termination` (TD-11, first half). The token of a pull transfer
  stays the same; a push transfer gets a new token for the receiving
  route.
- Audit line for each data pull: the provider's data plane writes one
  INFO line, `data transfer done` (target `portitor::audit`), with
  `flow`, `agreement`, `counterParty`, `direction` (`sent`,
  `received`), `bytes` and `outcome` (`complete`, `cut`, `failed`) for
  each `/data` answer ([OPERATING](docs/OPERATING.md), TD-31, first
  half). `flow` is the transfer's `providerPid` on both sides. The
  message, the target and these fields are stable in 1.x
  ([STABILITY, Audit line](docs/STABILITY.md#audit-line); maintainer,
  2026-11-25).
- The provider can always end access (ADR 0027, section 7). Management
  routes for the transfers a connector runs as provider:
  `GET /v1/provided-transfers`, `GET /v1/provided-transfers/{providerPid}`
  and `POST /v1/provided-transfers/{providerPid}/termination`. A
  termination ends access at once and sends the DSP termination in the
  background. The view never shows the data token or its hash.
- Suspending or terminating a transfer as provider cuts every running
  download of its data flow (ADR 0027, section 5).
- The data plane checks the consumer's `MembershipCredential` again on
  every `/data` request, and while a download runs: issuer still
  trusted, not expired, status bit not set, with the status list cache.
  A failed check refuses with 401, cuts running downloads and ends the
  transfer; a check that cannot finish refuses but keeps the transfer.
  [DEPLOYING, Revocation and running transfers](docs/DEPLOYING.md#revocation-and-running-transfers).

### Fixed

Found in the review of the 1.1 branch, before its release:

- A push cut because the consumer's membership could not be checked,
  or one that could not begin, terminates the transfer; before, the
  transfer stayed `STARTED` with nothing sending.
- A push has time limits: the connection must open within 10 seconds,
  and the push fails after 120 seconds in which the endpoint takes no
  data and gives no answer.
- The receiving route `/push/{consumerPid}` takes one upload at a time
  per transfer (a second one gets 409) and at most 8 at once (503), and
  cuts a body that sends nothing for 120 seconds (400, upload aborted).
- A multipart upload cut half-way is aborted, so the store keeps no
  parts. DEPLOYING now recommends a lifecycle rule
  (`AbortIncompleteMultipartUpload`) on the push bucket.
- A transfer that started before 1.1 gets no new token when the
  provider starts it again (only a scripted provider policy does), so
  it is no longer exempt from the membership check for good; the
  transfer is terminated instead.
- `AWS_*` variables are checked only when `PORTITOR_PUSH_BUCKET` is
  set: a 1.0 configuration with, say, only `AWS_ACCESS_KEY_ID` set for
  another tool starts again, without object storage.
- A partner whose membership was revoked can still end a transfer it
  shares with you: its `TransferTerminationMessage` is accepted when its
  DCP token is valid and it is the transfer's counter-party, and logged
  (`DSP termination accepted from a partner without membership`). Every
  other message from it is refused as before. Before, such a transfer
  stayed `REQUESTED` until the operator ended it
  ([TD-47](docs/RISKS_AND_TECHNICAL_DEBTS.md#td-47); ADR 0016, amended
  in 1.1).

### Upgrade notes

- Database migration 13 (forward only) adds the columns
  `transfer.data_address` and `data_flow.push_address` (the consumer's
  push address, emptied when the transfer ends) and the table
  `push_receipt` (our push tokens' hashes and object keys).
- Database migration 12 (forward only) adds the table `flow_membership`.
  Transfers that started before the upgrade have no membership kept:
  they are served as in 1.0 until their token expires
  (`PORTITOR_DATA_TOKEN_SECONDS`), with one INFO line per flow. Back up
  `/data` first, as for every upgrade.
- New log lines: [OPERATING, Health and logs](docs/OPERATING.md#health-and-logs).

## [1.0.0] - 2026-11-24

Portitor 1.0, the first stable release. From now on, a 1.x release may
add to what [STABILITY](docs/STABILITY.md) lists, and only 2.0 may remove
or rename it. No setting, route, file format or database schema changed
since 1.0.0-rc.1: the changes below are the reproducible-build check,
documentation, and the demos moved into `scenarios/`. The only Rust
changes are comments and test file paths. All six scenarios passed on
the release candidate on Linux (five on its images, two-data-spaces
built from source), and its images were checked with
`cosign verify`, before this release. Upgrading from 0.9 or later:
[OPERATING, From 0.9 to 1.0 in one step](docs/OPERATING.md#from-09-to-10-in-one-step).

### Fixed

- Reproducible build, release mode: the comparison read the rebuild's
  digests from the wrong directory (a download pattern that matches one
  artifact puts its files straight into the target directory), so the
  first rebuild of a release (`v1.0.0-rc.1`) failed with "different"
  although the rebuild logs show the same digests as the release. Each
  copy is now downloaded by name, and a missing digest list is reported
  as such.

### Documentation

- `docs/TESTING.md`: what rootless Podman needs on Linux (Ubuntu and
  Pop!\_OS 24.04: netavark, aardvark-dns, the API socket, the
  `docker-compose` binary), the first passing Linux run of
  `scripts/demo-compose.sh`, and its known failure with Podman 4.9.3
  (about one run in ten; run again).

### Added

- Two health scenarios on a Luxembourg-style health data space, with
  synthetic genotypes made with msprime (`examples/genotypes/`):
  `scenarios/hdab-intermediary/`, where an HDAB pulls and counts only
  during a data user's transfer, and `scenarios/data-permits/`, where
  the HDAB's own issuer grants data permits as credentials that the
  biobank checks itself. They give the
  first answers to the roadmap's four questions on v1.0.0-rc.1. Manual
  tests, not run in CI.
- Every scenario has a README (what it is about, scope, components
  tested and why, steps, findings), C4 deployment and dynamic diagrams,
  and its own changelog; `scenarios/README.md` sets the template.

### Changed

- The demos are scenarios, in `scenarios/` at the root, one folder each.
  The two-party demo moved there: `scripts/demo.sh`,
  `scripts/demo-compose.sh` and `scripts/demo-steps.sh` are now
  `scenarios/two-party/run-local.sh`, `run.sh` and `steps.sh`, with
  `examples/compose/`'s files. The EDC interop, Gaia-X and two data
  spaces tests are scenarios too: `interop/edc/` and
  `scripts/edc-interop.sh` are now `scenarios/edc-interop/`,
  `interop/gaiax/` and `scripts/gaiax-e2e.sh` are now
  `scenarios/gaiax-compliance/`, and `scripts/two-data-spaces.sh` is
  `scenarios/two-data-spaces/run.sh`. The Gaia-X compliance service
  that both use is in `examples/gaiax/compliance.yaml`.
- Each scenario stands alone: its own `run.sh` starts only the services
  it needs. The helpers the scenarios share are in
  `scripts/scenario-lib.sh`.
- Version 1.0.0. The website quickstart clones `v1.0.0`, and the
  scenarios that pull release images (two-party, hdab-intermediary,
  data-permits) default to the 1.0.0 images.

## [1.0.0-rc.1] - 2026-11-22

The first release candidate of 1.0. No setting and no route changed
since 0.14.0. It is a pre-release: the GitHub release is marked as one,
and 1.0.0 follows when the candidate shows no mistake.

### Security

- EDC interop test (test-only): Jackson 2 at 2.22.3 and the Gradle Shadow
  plugin at 9.6.1, which close the 17 advisories that the OpenSSF
  Scorecard found in `interop/edc/runtimes/gradle/verification-metadata.xml`
  (jackson-core and jackson-databind 2.19.2 and 2.22.1, Shadow 9.5.1). The
  OSV scan in the Dependencies workflow now reads that file too.

### Build

- A tag with a pre-release part (`v1.0.0-rc.1`) makes a GitHub release
  marked as a pre-release, so it does not become the latest release.
  RELEASING says how to cut a release candidate, and what to do when a
  tag was pushed before its release pull request was merged.
- The website's "latest release" value names the newest stable tag, never
  a release candidate.
- Reproducible release builds. The release binaries are built by
  `scripts/build-release.sh` (fixed paths in the binaries,
  `SOURCE_DATE_EPOCH`), the images by a pinned BuildKit with
  `rewrite-timestamp=true`. A new workflow, `Reproducible build`,
  rebuilds each release tag and compares the binaries and image digests
  with what was published, and builds `main` twice to compare. README
  badge and website value (docs/RELEASING.md, "Reproducible builds").
- The six release binaries get a signed build provenance (Sigstore,
  stored by GitHub), like the images; `gh attestation verify` checks it
  (docs/RELEASING.md, "Checking a release").
- README badges for the SonarQube Cloud quality gate and coverage; the
  website's "Trust and quality" list adds the coverage and the
  reproducible-build result. The SonarQube Cloud project's main branch
  is now `main`.

### Documentation

- ADR 0017 (joining a Simpl-Open data space) is Accepted, with the
  maintainer's answers to its six questions. The "trusted Simpl gateway"
  adapter is accepted in principle only and reviewed again before it is
  built. The ADR adds what Simpl-Open's Helm install of the governance
  authority needs (Argo CD, Kubernetes 1.33, about 8 CPU and 32 GB RAM),
  why the local test uses a Compose file of the needed components
  instead, and where Simpl-Open uses OpenID4VP today (user sign-in,
  Tier 1). ROADMAP 1.7: step 1 is a DSP-level test against
  `connector-be`, step 2 runs the governance authority parts it needs
  from that Compose file, and a watch item follows the
  protocol of Simpl-Open's SSI Tier 2. ARCHITECTURE no longer says a
  Simpl participant can skip `portitor-identity` with "x509 only"; ADRs
  0002 and 0003 get a note that their Simpl-Open fact is corrected by
  ADR 0017.
- OPERATING, "From 0.9 to 1.0 in one step": one upgrade note for an
  operator who goes from 0.9 straight to 1.0, with what to change in the
  configuration, the management API clients and the monitoring, and how
  to roll back. It links to the note of each release. It adds two
  changes the release notes left out: a JSON key given twice in the
  datasets file (0.12) and the 60 s limit on the management port (0.13).
- The website has a section "What 1.0 keeps stable", a summary of
  STABILITY with links to it.
- STABILITY: 1.0 is the 0.14 release candidate, not the 0.13 one, as the
  roadmap says.
- ROADMAP: the next twelve months, to the end of 2027. Releases 1.6 to
  1.9 add the functions an EHDS dataspace participant may need (dataset
  descriptions with the minimum metadata of Art. 77(4), permits, data
  requests, hand-over to a secure processing environment, opt-out
  research, the hand-over to HealthData@EU) and tests against
  Simpl-Open, plus what Portitor does not plan to do.
- CONTRIBUTING has a "Code review" section: what a reviewer checks
  before a pull request merges.
- Close-out of 0.14: the roadmap and the website mark 0.14 released and
  1.0 as the current release.
- CONTRIBUTING and ROADMAP describe the ruleset on `main` as it is set:
  five required checks (the three gates and the two CodeQL jobs) and one
  approval. The six accepted CodeQL findings are dismissed on GitHub.
- SECURITY-ASSURANCE: private vulnerability reporting is on, so OpenSSF
  Baseline OSPS-VM-02.01 is met.
- RISKS_AND_TECHNICAL_DEBTS, R-25: the name check is done. A preliminary
  search (TMview, WIPO Global Brand Database, domains, package
  registries) found no trademark conflict in Nice classes 9 or 42. Other
  uses of the name are listed; the maintainer accepted the risk, so R-25
  no longer blocks 1.0. ROADMAP and ADR 0008 say so.

## [0.14.0] - 2026-11-21

The release candidate for 1.0. No setting
and no route changed. Two checks are stricter; what to do:
[OPERATING, From 0.13 to 0.14](docs/OPERATING.md#from-013-to-014).

### Added

- **The EDC Issuer Service in the EDC interop test.** The test
  (`scripts/edc-interop.sh`, new step 8) has a second issuer, the Issuer
  Service of EDC IdentityHub v1.0.0-RC8 (`did:web:edc-issuer.gaiax.test`,
  a fourth runtime in `interop/edc/runtimes`). It issues a plain test
  credential, the `InteropTestCredential`, to both participants over DCP:
  `portitor-identity` asks with `POST /v1/credential-requests`, the EDC
  IdentityHub through its Identity API. Each provider then offers data
  only with `InteropTestCredential.level eq "basic"`, and each consumer
  negotiates and pulls from the other; before anyone holds the
  credential, both are refused. At the end the Issuer Service revokes the
  EDC participant's credential, and the Portitor connector, which reads
  the Bitstring Status List itself, ends EDC's next negotiation. The
  revocation by `portitor-issuer` is now step 9.
- A test in `portitor-dcp` (`tests/edc_issuer_service.rs`) reads a
  credential and the status list of an EDC Issuer Service, before and
  after a revocation, from that runtime's recorded output.

### Changed

- Connector: a new DEBUG log line, `status list fetched and verified`
  (with the list's URL and issuer), each time a status list credential
  is fetched; nothing at the default level `info`.
- EDC interop test, step 8: Portitor's negotiation without the
  credential must end `TERMINATED` and EDC must have no new agreement;
  the connector's log must show that it fetched the EDC Issuer Service's
  status list; after the revocation the run needs proof of why Portitor
  refused, or fails.
- **Behaviour change, all three: an item of a list setting may be at
  most 4096 bytes.** This holds for every comma-separated `PORTITOR_*`
  setting (DIDs, DCP scopes, credential types, file paths). A service
  with a longer item does not start; the error names the setting, the
  item's position in the list and its length. It shows no part of the
  item, which may be a secret pasted by mistake.
- **Behaviour change, connector management API: the catalog must be the
  provider's own.** `POST /v1/catalog/request` answers 502 when the
  provider's catalog has no `participantId` (DSP 2025-1 requires it;
  0.13 handed such a catalog on) or one that differs from the
  `counterPartyId` of the request. The two are compared as written,
  with one exception: in a `did:web` DID the escaped colon before a port
  may be `%3A` or `%3a`, as for the DID resolver. There is no setting to
  turn the check off: a provider whose DSP `participantId` is not the
  DID given as `counterPartyId` cannot be read with this call. No new
  route, member or status code.
  `POST /v1/catalog/dataset/request` is unchanged.
- Tests: the issuer's bundle tests name their scratch folders by process
  id and a counter, so two tests, or two runs at once, never share one.
- The code of the two catalog calls is split into smaller modules
  (`management/catalog/`, `catalog_reader/`). No change in behaviour;
  the OpenAPI description is the same.
- CI: a change to prose only (`site/`, `_http/`, `.vscode/`, `*.md` in the root and under `docs/` except `STABILITY.md` and `ARCHITECTURE.md`) no longer runs the build, the tests, SonarQube Cloud, the TCKs and the dependency checks; a change to fuzzing only skips the build and the TCKs. Any other path runs everything (`scripts/ci_changes.sh`, with a self-test). The licence check and `scripts/check_docs.py` always run. Each of `CI`, `TCK` and `Dependencies` ends with a gate job (`CI gate`, `TCK gate`, `Dependencies gate`); only gate jobs are required checks (CONTRIBUTING, "Required checks").
- CI: the rule of the three gate jobs is one script, `scripts/ci_gate.sh`, with a self-test that every gate job runs first; the self-test of `scripts/ci_changes.sh` also covers `detect`, in a temporary git repository (merge base, a new branch, renames, an empty diff, paths with spaces).
- CI: a newer push to a pull request cancels the run in progress in `CI`, `TCK`, `EDC interop` and `Gaia-X`; a run on `main`, a scheduled and a manual run are never cancelled and never wait for another run; every job has a timeout; the built DSP TCK jar is cached under its pinned commit; ORT runs on `main` only when a manifest or its configuration changed.
- CI: the EDC interop test also runs for changes to the root `Cargo.toml`, `.dockerignore`, `scripts/demo-bundle.sh` and `examples/bundle/`; the Gaia-X test for the root `Cargo.toml` and `.dockerignore`; pull request fuzzing for the root `Cargo.toml` and its own workflow.
- Release: the dry run also runs when the root `Cargo.toml` changes, so the release pull request builds and tests on arm64 and writes the SBOMs before the tag.
- **The website has an ancient Roman theme**, built on the two meanings
  of *portitor*, the customs officer and the ferryman: a gate, a toll
  coin and a ferry. The same drawing is the logo and the favicon
  (`site/favicon.svg`). Headings use the font Cinzel Regular, stored in
  the repository (`site/fonts/cinzel-regular.woff2`) under the SIL Open
  Font License 1.1 (`LICENSES/OFL-1.1.txt`, `NOTICE`, `REUSE.toml`). The
  font is part of the website only, not of any image or binary.

### Fixed

- The metric `portitor.outbox.message.given_up` no longer counts a
  finalization event that the consumer's transfer request had already
  settled (ADR 0007, note of 0.14; TD-44). The negotiation is
  `FINALIZED`, so nothing was given up. The metric keeps its name; an
  INFO log line says when a message is not counted.
- The same metric no longer counts a message whose outcome the store
  could not record (WARN `outgoing message not recorded`). Such a
  message is still saved as pending and is sent again after a restart;
  it is counted then, if it fails again.

### Security

- Release: the workflow refuses a signed tag whose commit is not on `main`.
- CI: `SONAR_TOKEN` is given to the scan step only, not to the build and tests of the dependencies; checkouts no longer keep the workflow token in `.git/config` (except the tag check, which fetches).
- CI: a CodeQL finding now fails the CodeQL job, on a pull request, on `main` and in the weekly run: `scripts/codeql-gate.sh` fails on every result that is not in `.github/codeql/accepted-findings.json` (TESTING, "CodeQL"; CONTRIBUTING, "Pull requests").

### Documentation

- **Documentation simplification** (maintainer, 2026-11-20): each fact
  has one home and the other files link to it.
  - `docs/DEPLOYING.md` is in deployment order (issuer, identity,
    connector), starts with a "First deployment" path and has one
    settings table per component (TD-39).
  - New `docs/OPERATING.md`: health and logs, traces and metrics, backup
    and restore, key rotation, upgrade and rollback, and the upgrade
    notes of every release. These sections left DEPLOYING; links to
    them changed from `DEPLOYING.md#...` to `OPERATING.md#...`.
  - The README is cut to what Portitor is, its components, how to try
    it and where to read. Its route and settings tables are gone: the
    OpenAPI files and DEPLOYING hold them.
  - New `docs/READING-THE-CODE.md`: the order to read the code in, and
    one request followed from the route to the outbox (R-01).
  - New `docs/adr/README.md`: an index of the ADRs with their status.
  - ARCHITECTURE describes the system as it is and has one table,
    "Specification versions", of what Portitor is checked against
    (TD-43). The roadmap's released sections are one table. The risks
    file has a "Top risks" box, and closed items are one line each.
  - TESTING: one place for the image build and the demos; a rule for
    the four near-copies of the negotiation and transfer logic,
    "changed one, check the other three" (TD-42); the steps of the EDC
    Issuer Service test.
- New in the risks file: R-46 (the EDC Issuer Service names a request
  status address that is not found, so `portitor-identity` shows
  `issuerStatusError`) and R-47 (its admin API may change; test only).
- GOVERNANCE, "Continuity": what an account successor takes over, and
  what stays tied to the maintainer's account.
- RELEASING: the documentation review also checks OPERATING,
  READING-THE-CODE and the ADR index.
- Close-out of 0.13: the roadmap and the website mark 0.13 released and
  0.14 as the current release; the website quickstart clones v0.13.0.

## [0.13.0] - 2026-11-20

### Added

- **Docker or Podman.** `scripts/demo-compose.sh`,
  `scripts/gaiax-e2e.sh` and `scripts/edc-interop.sh` run with either:
  Docker if it answers, otherwise Podman, or the one named in
  `CONTAINER_ENGINE` (`scripts/container-engine.sh`). For Podman, every
  pulled image is named with its registry, the Compose demo has its own
  health checks, and the interop tests' web server takes its name server
  from the container. Every script checks for bash 4.4 or later first.
  CI stays on Docker; the Podman runs are by hand (TESTING, "Docker or
  Podman").
- **Connector management API: read a provider's catalog** (TD-38).
  `POST /v1/catalog/request` returns the provider's DSP `Catalog` and
  `POST /v1/catalog/dataset/request` one `Dataset`, as the provider
  sent them. The connector gets the DCP token from its STS, so no
  hand-made STS call is needed. The bodies are the `CatalogRequest`
  and `DatasetRequest` of Eclipse EDC's management API v4
  (`counterPartyAddress`, `counterPartyId`, `protocol` or `profile`,
  `additionalScopes`, `@id`). `protocol` must be
  `dataspace-protocol-http:2025-1`; a `querySpec` is refused with 400.
  A provider that refuses or does not answer gives 502. At most 16 MiB
  are read (a fixed limit; other answers keep 64 KiB), and of a catalog
  in pages only the first, with the provider's `Link` headers (README,
  "Management API"; DEPLOYING, "Known limits").
- The demo, the Gaia-X and the EDC interoperability scripts and the
  Bruno collection read catalogs through the new call.

### Changed

Six changes made before the interface freeze (maintainer, 2026-11-20);
after 1.0 they could only be made in 2.0. What to do:
[OPERATING, From 0.12 to 0.13](docs/OPERATING.md#from-012-to-013).

- **Breaking, all three: `PORTITOR_ALLOW_HTTP_CALLBACKS` is now
  `PORTITOR_ALLOW_HTTP`.** It allows plain `http://` for every outgoing
  call, not only callbacks. A service that finds the old name set does
  not start and names the new one. `PORTITOR_STS_ALLOW_HTTP` is
  unchanged.
- **Breaking, identity: `PORTITOR_IDENTITY_CERTIFICATE_CHAIN` is now
  `PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE`** (it is a file path). The
  old name stops the start.
- **Breaking, issuer: `PORTITOR_ISSUER_CREDENTIALS_FILE` is now
  `PORTITOR_ISSUER_CREDENTIAL_DEFINITIONS_FILE`** (it holds credential
  definitions). The old name stops the start.
- **Breaking, connector management API: `providerAddress` and
  `providerId` are now `counterPartyAddress` and `counterPartyId`**, in
  the body of `POST /v1/negotiations` and in the negotiation and
  transfer views that `/v1/negotiations` and `/v1/transfers` return:
  the names of Eclipse EDC (`ContractRequest`, `TransferRequest`,
  `CatalogRequest`, EDC v1.0.0-RC8) and of the catalog calls. The old
  names are refused with 400 as unknown fields. `providerPid` and
  `consumerPid` keep their DSP names.
- **Breaking, connector: `PORTITOR_PARTICIPANT_ID` and
  `PORTITOR_PUBLIC_URL` are required.** They defaulted to
  `did:web:localhost%3A8080` and `http://localhost:8080`; the connector
  now refuses to start without them, as the identity and the issuer do.
- **Breaking, all three: one rule for every setting.** Spaces around a
  value are dropped, and an empty value means "not set": a required
  setting that is empty stops the start, an optional one takes its
  default. The connector did not trim and took an empty value as set.
  Changed for a value that is set but empty: `PORTITOR_DCP_SCOPES` and
  `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` (were refused, now the
  default), `PORTITOR_SCOPE_TYPE_ALIASES` (was "no alias", now the
  default alias), `PORTITOR_BIND` and `PORTITOR_DB_FILE` (now the
  default), and the connector's settings in seconds (now the default).
- **Bundle manifest: a file with a role the reader does not know may
  have any media type** of the shape `type/subtype` (RFC 6838,
  section 4.2), so a later role that needs another media type does not
  make a 1.0 connector refuse the bundle. Files with the four known
  roles keep the three media types. The JSON schema had the closed list
  for every file and is corrected (ADR 0005, amended).
- The management OpenAPI descriptions say in one shared place (the
  `api_key` security scheme) that the API key is opaque; the
  connector's also says it for the data plane token.
- DEPLOYING, "Known limits": a repeated transfer request does not
  start a second transfer (since 0.12, TD-34); the sentence was stale.
- `docs/STABILITY.md` reserves two things for 1.x: a 1.x release may
  refuse internal network addresses on URLs that a partner chooses, by
  default, with a setting to allow them (F-01, R-17); and it says that
  each service needs its own host name.
- The connector's management OpenAPI lists HTTP 403 on every route
  that needs the key. Nothing answers 403 today: it is reserved for
  scoped access in 1.x, and the API key keeps every right (ADR 0025).
- The EDC interop test (`scripts/edc-interop.sh`) runs against EDC
  v1.0.0-RC8 (Connector and IdentityHub; before: RC7). The Gradle
  dependency verification file is written again for RC8.
- The EDC interop test now includes Gaia-X compliance, with the local
  compliance service, test notary and test CA of the Gaia-X test:
  Portitor's identity does its onboarding; a second dataset of the
  Portitor provider asks for a `gx:ParticipantComplianceCredential`; the
  EDC consumer is refused without one (`TERMINATED`) and gets a contract
  and the data once its IdentityHub holds one. The EDC IdentityHub has no
  Gaia-X onboarding, so the test-only `scripts/gaiax_test_onboarding.py`
  signs for the EDC participant. Not covered: an EDC provider that asks
  Portitor for the credential (see `docs/TESTING.md`). The participants
  are now `did:web:portitor.gaiax.test` and `did:web:edc.gaiax.test`.
- `scripts/gaiax-e2e.sh` and `scripts/edc-interop.sh` share
  `scripts/gaiax-lib.sh` (test CA, test notary, a participant's Gaia-X
  files).
- **Behaviour, connector as provider: a transfer request sent at once
  after the agreement is no longer refused** (TD-44; maintainer,
  2026-11-20; ADR 0007 amended). A transfer request from the
  agreement's consumer that arrives while the finalization event is
  still on its way (sent, its acknowledgement not yet recorded) proves
  that the event arrived: the negotiation is recorded as `FINALIZED`
  and the request is checked as any other. Before, it got HTTP 400 and
  the consumer's transfer ended `TERMINATED` (2 to 4 times in 6,000 to
  10,000 transfers in the 0.12 benchmark). An agreement of a
  negotiation in any other state, or of another consumer, is refused as
  before. No new migration: the lookup uses the index of migration 10.
- Connector as provider: a transfer request refused because it names no
  finalized agreement of the caller is now logged, `transfer request is
  refused; answered 400` (INFO). The line names neither the agreement
  nor the caller.
- **Behaviour, connector: a bundle file with a role the connector does
  not know is skipped** (maintainer, 2026-11-20; ADR 0005 amended;
  ADR 0025, point 4). Before, such a manifest made the whole bundle
  invalid. Now the file is not downloaded and not used, the other files
  are, and the connector says so: a WARN line for each file
  (`bundle file skipped: ...`, with path and role type), the new gauge
  `portitor.bundle.file.skipped`, and the summary of the bundle run in
  `GET /v1/harvest-jobs`. A bundle in which the connector knows no role
  is refused like any invalid bundle. The limit: a connector that skips
  a new kind of rule does not enforce it. Everything else in a manifest
  is read as strictly as before, and the issuer still builds, signs and
  serves only roles it knows. `docs/schemas/bundle-manifest.schema.json`
  says the same: known roles are closed, a role object with another
  type is valid, at least one file has a known role.
- **Found by the engineering review of the release, changed before the
  freeze** (DEPLOYING, "From 0.12 to 0.13", points 7 to 9):
  - **Connector: `PORTITOR_PARTICIPANT_ID` and `PORTITOR_PUBLIC_URL`
    are checked at start**, as the identity's and the issuer's are: a
    `did:web` DID, and an `http(s)` URL without a query or fragment.
    Anything else stops the start.
  - **All three: a switch is `true` or `false`.** Any other value of
    `PORTITOR_ALLOW_HTTP` or `PORTITOR_STS_ALLOW_HTTP` (`1`, `yes`)
    stops the start; before, it was read as off without a word.
  - **All three: a list setting that is set must name something.** A
    value of commas only (`","`) stops the start, for every
    comma-separated setting. Before, some read it as an empty list
    (`PORTITOR_GAIAX_COMPLIANCE_ISSUERS`, `PORTITOR_SCOPE_TYPE_ALIASES`,
    the retired key files) and some stopped.
  - **Connector management port: a time limit.** A request that takes
    longer than 60 seconds is answered 503 with a problem document, as
    on the public port, and at most 4 provider catalogs or datasets are
    read at a time (each may hold 16 MiB); a further request waits.
    Before, the port had no limit on time or on such reads.
  - **Catalog calls.** A provider's `Dataset` without `@type` is
    accepted (DSP 2025-1 does not require it there; a different `@type`
    is still 502), an answer that is not a JSON object is refused also
    when it is an array naming the type, and a dataset id of `.` or
    `..` is refused with 400.
  - **Bundle: a role type that is a known one in other letter case**
    (`Messages`) is refused as a mistake, not skipped as a later role.
    The manifest's JSON Schema says the same, and it now also refuses
    the version `latest` in any letter case, as the code always did.
  - **Management API: an empty id is refused.** `datasetId` of
    `POST /v1/negotiations` (connector) and `holderPid` of
    `POST /v1/credential-requests` (identity) that are empty or only
    spaces answer 400. Before, a `datasetId` of spaces started a
    negotiation, an empty `holderPid` got a new id and one of spaces
    was stored. Leave `holderPid` out for a new id.
  - **Transfer request while the finalization is on its way** (TD-44):
    nothing is written unless the caller is also the agreement's
    assignee (it always was in practice), and ADR 0007 now says what
    the rule shows: the consumer acts on the agreement after verifying
    it, not that the event arrived.
  - The interop scripts stop the containers of an earlier `KEEP=1` run
    before they delete its files, clean up from their first line, and
    say why they stop when `curl` or `openssl` fails.

### Fixed

- Connector management API, `POST /v1/catalog/request` and
  `POST /v1/catalog/dataset/request`: a provider's answer that is not
  UTF-8 text inside a member the connector does not read was handed on
  with HTTP 200 as `application/json`. It is now refused with 502, as
  any other answer that is not JSON (RFC 8259, section 8.1).

### Documentation

- `docs/TESTING.md`, "EDC interop test": every step with its expected
  result, what is pinned, a "last run that passed" table, what the
  Gaia-X steps show and do not show, the changes from RC7 to RC8, and
  how to use a kept run by hand (`KEEP=1`), with the new Bruno
  environment `_http/environments/edc-interop.bru`.
- ODRL `dateTime` (R-26): the definition in the ODRL 2.2 vocabulary was
  read and compared with ADR 0009 and the code. The text ADR 0009 used
  from memory was the definition, and the code follows it; no code
  changed. ADR 0009 has the quoted definition, its source and the
  comparison, also with Eclipse EDC (`edc:inForceDate`). A test holds
  the specification's own examples.

- Close-out of 0.12: the roadmap and the website mark 0.12 released and
  0.13 as the current release; the website quickstart clones v0.12.0.
  CONTRIBUTING gives `cargo vet diff` with `--mode local`.
- Identity and issuer management APIs: every route that needs the API
  key documents HTTP 403 in the OpenAPI files, reserved for tokens with
  limited rights in 1.x (ADR 0025, freeze point 2). 1.0 never answers
  403: the API key grants every right. The descriptions say that the
  API key is an opaque string (freeze point 1).
- **API and configuration freeze.** New `docs/STABILITY.md`: the HTTP
  routes, environment variables, file formats and command line that 1.0
  keeps stable under semantic versioning, what is not part of the stable
  interface (metric, span and log names; the Rust crates; development
  aids), and what 1.0 does not do. Migrations stay forward only (TD-22,
  confirmed by the maintainer, 2026-11-20). README and DEPLOYING say
  that the management API key and the data plane token are opaque.
- Tests: a freeze guard (`tests/frozen_interface.rs` in the three
  binaries' crates) fails when a route or environment variable listed
  in `docs/STABILITY.md` leaves the code, and when the code gains one
  that is not listed.
- **Decisions of the maintainer (2026-11-20), recorded.**
  [ADR 0024](docs/adr/0024-several-data-spaces.md) accepted: 1.0 is one
  connector per data space; one DID per data space is the default, a
  shared DID is allowed. [ADR 0025](docs/adr/0025-users-and-roles.md)
  accepted: 1.0 stays system to system; access tokens with scopes are
  planned for 1.5. New
  [ADR 0026](docs/adr/0026-tls-termination.md): TLS stays the
  operator's job (TD-19). TD-03, TD-04, TD-19 and TD-22 updated.
- DEPLOYING: new sections "Two data spaces" and "Users and roles". The
  two-connector setup was run by hand with the new
  `scripts/two-data-spaces.sh` (a manual test, TESTING), with a shared
  identity and with one identity per data space.

## [0.12.0] - 2026-11-20

Interoperability and performance. Read "From 0.11 to 0.12" in
`docs/DEPLOYING.md` before you upgrade: two changes need action.

### Changed

- **Breaking, issuer configuration:** every entry of the definitions
  file (`PORTITOR_ISSUER_CREDENTIALS_FILE`) needs `credentialSchema`,
  the absolute `http(s)` URL of the credential's schema; the issuer
  does not start without it. The issuer publishes the URL in its
  metadata, together with `offerReason: reissue`, so the metadata now
  passes DCP's `issuer-metadata-schema.json` (TD-33; ADR 0018 amended).
  Offers carry `credentialSchema` too. Nothing fetches the URL.
- **Behaviour, connector: policies are read strictly** (TD-32;
  maintainer, 2026-11-18; ADR 0009 amended). A field of an offer, rule,
  constraint or agreement that the connector does not read is refused
  and named, where 0.11 dropped it silently. In the datasets file the
  harvest of that dataset stops (check `odrl`); a partner's message
  gets HTTP 400 with the DSP error and changes nothing. A `duty` inside
  a permission is refused. An offer's `assigner` and `assignee` are
  read in a message and must be the provider and the consumer; in the
  datasets file they are refused. Stored datasets are checked again at
  the first start. Eclipse EDC ignores unknown fields; Portitor is
  stricter on purpose.
- **Behaviour, connector: a refused caller gets HTTP 404** on the
  negotiation and transfer routes, the consumer's callbacks included,
  as the DSP bindings say and as Eclipse EDC answers; the catalog
  routes still answer 401, and 503 stays when a service the check needs
  is down (TD-36; ADR 0016 amended). The log line `DSP request refused:
  caller not authenticated` has a new `status` field.
- **A JSON key given twice is refused** in a DSP negotiation or
  transfer message, in a constraint, and in the datasets file, with
  "duplicate field" and the key's name. Before, the last value won
  silently, so two parties could read two different policies in the
  same bytes (RFC 8259 calls the result "unpredictable"). Found by the
  0.12 testing review.
- A repeated `TransferRequestMessage` with the `consumerPid` of a
  transfer the same caller already asked for gets that transfer again:
  no second data flow or token (TD-34; ADR 0013 amended). A repeat with
  another agreement, format or callback address gets 400.
- As consumer, an agreement without a `timestamp`, or with one that is
  not an `xsd:dateTime`, is refused. As provider, a new request on a
  running negotiation must name the stored `consumerPid` (TD-36; ADR
  0006 amended).
- A panic no longer stops the process (TD-41; `panic = "abort"` is
  gone). The request that met it gets HTTP 500 and the others go on.
  Background tasks are watched: a schedule starts again after a pause
  (5 seconds, doubling up to 5 minutes), a DSP message whose delivery
  panicked is sent again the same way, a credential request whose
  decision panicked no longer blocks the issuer's outbox, and the
  process stops if the management server ends. The release binaries are about 13%
  larger (connector 12.9 to 14.7 MB; ADR 0002 amended).
- The connector's database gets three indexes (schema version 11): the
  lookup of a repeated transfer request, and the agreement id on both
  negotiation tables. A 0.11 connector cannot open the file afterwards:
  back it up first.
- Release workflow: the SBOM attestation uses `actions/attest` v4.2.2
  instead of `actions/attest-sbom`, which GitHub deprecated. The
  attestation and `gh attestation verify` stay the same.

### Added

- `PORTITOR_DID_CACHE_SECONDS` in the connector, the identity and the
  issuer: how long a resolved DID document is reused. Default 300, as
  before; `0` turns the cache off; at most 3600 (R-41; ADR 0016
  amended).
- Metrics `portitor.http.request.panicked` and `portitor.task.panicked`,
  and one ERROR log line per panic with the place in the code (ADR 0023
  amended).
- INFO log line `inbound DSP message is not valid; answered 400`, with
  the message type and the reason. Before, such a refusal was only
  counted.
- `docs/BENCHMARKS.md`: data plane with 100 and 1,000 pulls at once, a
  10 GiB pull, a pull that outlives its token, 10,000 stored
  negotiations and transfers, catalogs up to 10,000 datasets, and the
  cost of DCP measured again (R-29). `scripts/bench.sh` and the
  `bench-load` example reproduce them. The reference load comes from
  published reports of real platforms.

### Fixed

- The dataset harvest goes on after a run that panicked. Before, every
  later run, scheduled or manual, was skipped as "a run is going on"
  until a restart.
- A credential whose `kid` is the full key id (`did:web:issuer#key-1`)
  verifies when the issuer's DID document writes that key as `#key-1`
  only (W3C DID Core, relative DID URLs).
- An agreement `timestamp` that writes midnight as `24:00:00`, a valid
  `xsd:dateTime`, is accepted.
- A transfer start no longer slows down with stored history: the
  provider read every negotiation to find the agreement (8 ms on an
  empty file, 38 ms with 10,000 negotiations).
- Small answers no longer wait: about 5% of small data pulls took 43 ms
  because `TCP_NODELAY` was not set; now 0.4 ms.

### Security

- A credential is refused when the key that signed it is listed in its
  issuer's DID document under the id of another DID (DCP v1.0.1, 5.4.3,
  step 4; ADR 0016 amended). Credentials from `portitor-issuer` and
  from EDC IdentityHub are not affected.

### Documentation

- ADR 0025 (Proposed): users and roles, the answer to TD-04. Portitor
  stays system to system in 1.0; it names four small points to settle
  before the 0.13 freeze.
- ADR 0016 records three DCP choices with their reasons (TD-37): the
  `token` claim is required, presentations are JWT strings only, and
  `/.well-known/dspace-trust` is not served.
- The blocking database calls were measured and stay (TD-40): with 100
  pulls running, the data plane's p99 time to first byte stays under 15
  ms at any negotiation rate the test machine produced.
- New technical debts from the benchmark: TD-44 (a transfer request
  sent at once after the agreement can be refused) and TD-45 (memory
  per running pull).
- The HTTP collection names the saved process ids
  `mgmtNegotiationConsumerPid` and `mgmtTransferConsumerPid`.
- Close-out of 0.11: the roadmap and the website mark 0.11 released and
  0.12 as the current release; the website quickstart clones v0.11.0.
- Review of 2026-11-18 against DSP 2025-1, DCP v1.0.1, Data Plane
  Signaling v1.0-RC4 and the Gaia-X rules, and of code quality: the
  report is in `docs/reviews/`; its findings are TD-32 to TD-43 in the
  risks list, planned in the roadmap for 0.12 and 0.13, and named in
  DEPLOYING "Known limits". The hand-written OpenAPI files (DSP, issuer)
  say where the answer differs from the specification (TD-32 to TD-36),
  and the website no longer claims strict parsing or a Gaia-X
  conformance kit. No code changed.
- Roadmap, "Code quality in 1.x" (maintainer, 2026-11-18): one shared
  core for the negotiation and transfer logic, no file above 1,000
  lines, simpler types and macros, a better name for `Negotiator`, and a
  Helm chart (TD-42, TD-39, R-01).
- DEPLOYING and ARCHITECTURE: what the dataspace authority decides
  before participants join, because DCP v1.0.1 leaves it to the
  dataspace (trusted issuers, scope strings, holder binding, Gaia-X
  compliance, dataset metadata bundle), each with its Portitor setting.

### Dependencies

- `rusqlite` 0.42.0 to 0.43.0 (SQLite itself is unchanged). An empty
  or comment-only SQL text is now an error in `rusqlite`; Portitor
  sends none. The maintainer audits the change with cargo-vet.
- CI actions: `actions/setup-java` 6.3.0, `taiki-e/install-action`
  2.90.5.

## [0.11.0] - 2026-11-17

### Security

- Request size limits per port and route, set by Portitor (R-18). New
  settings in the three binaries: `PORTITOR_MAX_REQUEST_BYTES` (public
  port) and `PORTITOR_MANAGEMENT_MAX_REQUEST_BYTES` (management port),
  default 2097152 (2 MiB, the value axum applied before, so nothing
  changes by default), from 1 to 67108864 (64 MiB); another value stops
  the start. Routes whose messages are small by specification have a
  fixed limit of 64 KiB, or the port's when lower: the DSP catalog
  request, negotiation event, verification and termination, and
  transfer suspension, completion and termination (also on the consumer
  callbacks), the identity's `POST /sts/token` and the issuer's
  `POST <issuer service>/credentials`. A larger body gets HTTP 413 with
  a problem body; a declared `Content-Length` over the limit is refused
  before the body is read, and a body without one is not read before
  the DSP caller check. Data plane answers are still streamed.

- Every parser of input from outside the process has a fuzz target
  (roadmap 0.11). New targets: `dcp_x509` (X.509 chains), `dps_messages`
  (Data Plane Signaling messages and states), `time_parse` (`xsd:date`
  and `xsd:dateTime`), `shacl_datatypes` (lexical forms for
  `sh:datatype`), `harvest_datasets` (the harvested datasets file, through
  the DSP and SHACL checks), `bundle_rules` (the bundle's contexts, shapes
  and vocabularies as the connector reads them, and the message guard)
  and `http_inputs` (`Authorization` headers and URLs that other
  participants choose). Existing targets now also read the credential
  request and request status messages, scopes, the DSP
  acknowledgements and errors, `latest.json`, the claims read before a
  token is checked, and the keys of DID documents. The list of parsers
  and targets is in SECURITY-ASSURANCE, "Parsers and fuzz targets".

### Added

- Status list cache in the connector's DCP verifier (TD-06, ADR 0016
  amended): `PORTITOR_STATUS_LIST_CACHE_SECONDS`, default 300, at most
  3600 (a larger value stops the start; empty counts as not set), `0`
  fetches the list on every check. Only lists that were fetched and verified are kept, by URL, in
  memory, never past the list's own `validUntil` or `exp`. A revocation
  now takes up to that long to be seen (DEPLOYING, "Status list
  cache"). The Compose demo, `scripts/demo.sh`, the EDC interop test
  and the DCP TCK run set `0`.

- Key rotation without a verification gap (TD-05):
  `PORTITOR_IDENTITY_RETIRED_KEY_FILES` and
  `PORTITOR_ISSUER_RETIRED_KEY_FILES` list old **public** keys (PEM,
  comma-separated) in the DID document, each as its own verification
  method in the same relationships as the current key. What the old key
  signed (tokens, presentations, access tokens, credentials, status
  lists, Gaia-X credentials) keeps verifying until the operator removes
  it from the list; a retired key never signs. The start fails for a
  private key, a key that is not P-256, the current key, a key given
  twice, or (issuer) the bundle key. DEPLOYING has a "Rotating a key"
  runbook; ADR 0015 and ADR 0018 are amended, ADR 0021 has an update
  note.

### Changed

- The connector's DCP check answers HTTP 503, not 401, when it could not
  finish because a server it fetches from did not answer or answered
  408, 429 or 5xx (the caller's DID document or Credential Service, an
  issuer's DID document or status list, our STS), or took longer than
  20 s. A partner's outbox then tries again, where a 401 ends the tries.
  Real refusals (bad token, revoked credential, untrusted issuer, a 404
  from the caller's server) still answer 401. ADR 0016 is amended
  (maintainer's choice, 2026-11-17).
- The connector's outbox retries a DSP message only when no answer came,
  or the answer was 408, 429 or 5xx; any other answer (400, 404, 409,
  ...) ends the tries at once (TD-30, ADR 0007 amended). The state check
  after a failed send is unchanged. The `tck` build, never released,
  still tries a 404 again, because the DSP TCK registers some handlers
  late (R-08). The issuer's delivery outbox already
  stopped at a refusal (it also retries 401 and a holder not found) and
  is not changed.

### Documentation

- Threat model review 0.11 in SECURITY-ASSURANCE: a STRIDE pass over
  each trust boundary, checked against the code of 0.11.0, with ten
  findings (no high; medium: internal addresses not blocked with no
  release named, no audit trail, one management key). New items in
  RISKS_AND_TECHNICAL_DEBTS: R-41 to R-45 and TD-31; R-17 names the
  issuer's and the identity's partner-chosen URLs too. The threat table
  gets a row "Evidence of who did what".
- ADR 0024 (Proposed): a provider in several data spaces (TD-03).
  Compares one connector per data space, EDC's dataspace profiles and
  EDC's participant contexts with DSP 2025-1, DCP v1.0.1, Gaia-X and
  Simpl-Open; proposes one connector per data space for 1.0, EDC-style
  profiles after 1.0, and trusted issuers by credential type. TD-03
  points to it.
- ADR 0022: says which Gaia-X documents were checked for how a
  compliance credential is bound to its holder (none defines it), and
  that "Gaia-X Trust Protocol" is a section of the Architecture Document
  3.1, not a separate document.
- README badges, as in praecepta: release, CI, TCK, CodeQL, OpenSSF
  Scorecard, Best Practices and Baseline, licence, REUSE compliance
  (registered with the REUSE API, 2026-11-16) and Contributor Covenant.
- Close-out of 0.10: the roadmap and the website mark 0.10 released and
  0.11 as the current release.
- Roadmap 0.11: the SonarQube Cloud quality gate and coverage badges,
  once SonarQube Cloud's main branch is renamed to `main`.

## [0.10.0] - 2026-11-16

### Dependencies

- jsonschema 0.61.1 to 0.61.2 (test-only; Dependabot #50) and
  SonarSource/sonarqube-scan-action 8.4.1 to 8.5.0 (CI; Dependabot #51).

### Security

- cargo-vet in CI (workflow `Dependencies`): every crate version in
  `Cargo.lock` must be audited or exempted (`supply-chain/`). Audits are
  imported from Mozilla, Google, Bytecode Alliance, ISRG and Zcash (45
  crate versions); the other 159 are exempted at the versions locked on
  2026-11-16, so a
  new or bumped crate needs a maintainer's audit or a recorded exemption
  (CONTRIBUTING, "Dependency audits").

### Fixed

- The website shows the SonarQube Cloud quality gate again: the site
  build asks for the `main` branch, because SonarQube Cloud still names
  its main branch `master`.

### Documentation

- ADR 0023 (health checks, traces and metrics) accepted.
- CONTRIBUTING explains what to do when cargo-vet fails; TESTING,
  SECURITY-ASSURANCE and RELEASING list the check; risk R-04 (crates not
  audited) is now mitigated.
- Close-out of 0.9: the roadmap and the website mark 0.9 released and
  0.10 as the current release.
- Roadmap "Later": a health data space demo with four participants and
  an authority (HDAB, ICUdata4EU), which must answer how the same data is
  reached through a provider and an intermediary with a secure
  processing environment.
- The upstream reports drafted from 0.4 to 0.8 are filed (2026-11-16) and
  linked from R-07, the roadmap, the ADRs, ARCHITECTURE, TESTING and the
  code comments next to each workaround.

## [0.9.0] - 2026-11-15

### Documentation

- C1 and C2 diagrams redrawn with more context: C1 shows the dataspace
  as a boundary (the authority, which sets the rules, and its members),
  the people and applications inside the organisations, the whole
  optional Gaia-X side (compliance service, GXDCH notary, certificate
  authority) and a legend. C2 has fewer crossings and is split in two:
  `c2-containers` (one participant: trust settings, public and internal
  ports, who pulls from the data plane, the Gaia-X files) and the new
  `c2-dataspace-authority-containers` (the issuer, and the bundle built
  offline with `portitor-issuer bundle build`).
- New C4 deployment diagram (`deployment`: hosts, proxies, ports,
  volumes and mounts of a participant and the dataspace authority) and
  three C4 dynamic diagrams in sequence style (`dynamic-join`,
  `dynamic-negotiate-transfer`, `dynamic-gaiax-onboarding`), shown in
  ARCHITECTURE.md under "Deployment and flows".
- Documentation review after 0.8: the README links the website at its own address; AGENTS.md
  names DCP v1.0.1 and Gaia-X as shipped; the security assurance case
  names the compliance issuer pin (`PORTITOR_GAIAX_COMPLIANCE_DID`) and
  that compliance credentials from unlisted services are skipped.
- The `_http/` Bruno collection covers what was built since 0.4:
  transfers and their data, harvest jobs, the identity's credentials,
  credential requests and Gaia-X runs, and the issuer's participants and
  credentials. `scripts/dev-identities.sh` serves `examples/data`, and the
  `weather-stations` dataset of `examples/datasets.json` has a
  `portitor:source`, so a transfer runs in the VS Code set-up. The STS
  token script no longer uses `URLSearchParams`, which the Bruno CLI's
  script runtime lacks. Roadmap 0.10: get the website's SonarQube Cloud
  and REUSE values shown.
- C4 code diagrams brought up to date with 0.8: caller authentication
  shows the Gaia-X compliance checks (ADR 0022), presentations the `gx:`
  scope type rule (ADR 0021), and Gaia-X onboarding the steps of a run
  and its schedule.
- [ADR 0023](docs/adr/0023-observability.md) (health checks, traces and
  metrics; Proposed until the maintainer accepts it), with a section on
  what changed while building, and the
  [0.9 spike report](docs/spikes/0.9-opentelemetry.md) (OpenTelemetry
  within the footprint budget).
- The roadmap splits the hardening release 0.10 into 0.10 (supply
  chain), 0.11 (security hardening), 0.12 (interoperability and
  performance) and 0.13 (release candidate), each with exit criteria
  from its items. ADR 0012, ADR 0017 and ADR 0021 note the new
  numbers; RISKS_AND_TECHNICAL_DEBTS and SECURITY-ASSURANCE follow
  them.
- RISKS_AND_TECHNICAL_DEBTS: R-36 (the OpenTelemetry crates are 0.x),
  R-37 (telemetry costs some throughput at saturation: 8% with export
  off, 12% to 18% with it on; sampling does not win it back), TD-28 (no
  trace across participants, by design), TD-29 (no OTLP logs, gRPC or
  compression) and TD-30 (the outbox resends a message the counter-party
  refused with a 4xx); TD-18 (operations) addressed. SECURITY-ASSURANCE:
  telemetry off by default, operational data in the threat model, the
  operator's monitoring as a trust boundary.
- DEPLOYING: "Health and logs" with the health checks and probes,
  "Traces and metrics" (variables, sampling, what a trace shows, the
  metrics), an upgrade note "From 0.8 to 0.9", and security and network
  notes for both.
- TESTING: "Observability stack (by hand)", the health and telemetry
  export tests, and how to run the Compose demo with local images.
- RUST-IDIOMS: `AtomicBool`, spans with `.instrument`, and log targets.
- `_http/` Bruno requests for the liveness and readiness checks of the
  connector, the identity component and the issuer.

### Added

- Health checks in Eclipse EDC's Observability API shape on the
  management port of the connector, the identity component and the
  issuer, without the API key (ADR 0023): `GET /api/check/liveness`,
  `/readiness`, `/startup`, `/health`, HTTP 200 or 503 with a
  `HealthStatus` body. Readiness checks only the component's own
  dependencies (its SQLite file, its key, its DID document or status
  lists; for the connector the start harvest, the bundle and its own
  DID document and STS), never other participants.
- A `health` subcommand in each binary (`portitor-connector health`):
  calls its own readiness check and exits 0 or 1. Each image has a
  Docker `HEALTHCHECK` with it, and the Compose demo starts each
  component once those it needs are healthy.
- An optional observability stack for the Compose demo
  (`examples/compose/observability.yaml`, ADR 0023 section 6): an
  OpenTelemetry Collector, Jaeger and Prometheus, pinned by digest,
  started by hand. `scripts/demo-compose.sh` takes it with
  `EXTRA_COMPOSE=examples/compose/observability.yaml`. See TESTING,
  "Observability stack (by hand)".
- A quickstart on the website ("Try it"): what to install, how to run
  the Compose demo, where to look and how to clean up. The README links
  to it.
- Traces and metrics with OpenTelemetry in all three components
  ([ADR 0023](docs/adr/0023-observability.md)), off unless
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set: OTLP over HTTP with Protocol
  Buffers, sent with Portitor's own HTTPS client (no HTTP/2, no gRPC).
  A span per request served (route templates only) and sent, a span per
  DSP outbox delivery, and each negotiation and transfer state change as
  a span event with its DSP process ids. Metrics:
  `http.server.request.duration`, `http.client.request.duration` and
  Portitor's own under `portitor.` (negotiations and transfers ended,
  DSP requests refused, outbox resends and give-ups, harvest runs,
  rejected datasets, bundle version in force); the list is in
  DEPLOYING, section 6. The `traceparent` header crosses only calls
  inside one participant (connector to its STS and to its data
  sources); DSP calls carry none and ignore one received. With export
  on, JSON log lines inside a span start with `trace_id`. New
  dependencies: `opentelemetry`, `opentelemetry_sdk`,
  `opentelemetry-otlp`, `opentelemetry-http` 0.33 and
  `tracing-opentelemetry` 0.34, grouped in one Dependabot update.
- Each negotiation and transfer state change is logged (INFO, target
  `portitor::state`) with both process ids and the old and new state.
- The health checks are neither traced nor measured, and the
  connector's readiness work sends no `traceparent`.

### Changed

- The management port is always bound now (default `127.0.0.1`).
  Without `PORTITOR_MANAGEMENT_API_KEY_FILE` it serves only the health
  checks; with it, also the management API, as before. Two processes on
  one host need different `PORTITOR_MANAGEMENT_BIND` ports.
- The Compose demo's default images are 0.9.0: older images have no
  `health` subcommand.
- `portitor-connector` and `portitor-identity` refuse any argument other
  than `health` (exit code 2); `portitor-issuer` takes `bundle build`
  and `health`.
- One parser for `PORTITOR_MANAGEMENT_BIND` and
  `PORTITOR_MANAGEMENT_API_KEY_FILE` in all three components: values
  are trimmed and an empty value counts as unset. An empty
  `PORTITOR_MANAGEMENT_API_KEY_FILE` now switches the connector's
  management API off.
- If `OTEL_EXPORTER_OTLP_ENDPOINT` (or a signal-specific
  `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` or
  `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`) is already in the environment,
  for example set by the OpenTelemetry Operator, export turns on. Unset
  it if you do not want export.
- A failed readiness check is logged only when its result changes:
  `readiness check failed` (WARN) when it starts failing,
  `readiness check passes again` (INFO) when it recovers.
- The start log line `management API listening` is now
  `management port listening`: the port is bound also without the API.
- `scripts/demo-compose.sh` runs `docker compose down --volumes
  --remove-orphans` before deleting `examples/compose/run/`, so a second
  run after `KEEP=1` works.

## [0.8.0] - 2026-11-15

### Documentation

- New `docs/RISKS_AND_TECHNICAL_DEBTS.md`: every risk and technical debt
  raised from 0.0 to 0.8, with its status (Open, Mitigated, Addressed,
  Accepted) and target release. The roadmap's technical debt entries
  moved there; the roadmap keeps their titles with links. README,
  ARCHITECTURE, DEPLOYING ("Known limits"), RELEASING (the release
  review checks the file), ADR 0005, AGENTS and the website link to it.

### Added

- Gaia-X compliance in policies (ADR 0022). An offer can require a
  Gaia-X compliance credential, for example
  `ParticipantComplianceCredential.gx:labelLevel` `eq` `SC`. New
  connector setting `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`: the compliance
  services trusted, separate from `PORTITOR_TRUSTED_ISSUERS`. The
  verifier accepts a `gx:ParticipantComplianceCredential` from such a
  service (PS256 or ES256, validity from `validFrom` and `validUntil`
  only, `validUntil` required) with the `gx:LegalPerson` credential it
  names by `digestSRI`, issued and signed by the caller; the holder
  binding is skipped for these two only, and the legal person
  credential's status is not fetched (the caller controls it). Policies
  see these two only as their Gaia-X type. A compliance credential from
  a service not trusted is skipped, so its holder can still use offers
  that need only a membership. A `gx:LegalPerson` from an issuer in
  `PORTITOR_TRUSTED_ISSUERS` is checked like any data space credential.
  The `digestSRI` (RFC 8785 JCS) writes numbers exactly as JavaScript
  does (serde_json `float_roundtrip`). A compliance scope brings the
  `LegalPerson` scope on both sides. A credential claim path may now hold a prefixed name such
  as `gx:labelLevel`. The start log warns when
  `PORTITOR_POLICY_CREDENTIALS` lists `ParticipantComplianceCredential`
  without a compliance service.
- Identity: the X.509 chain of its key (`PORTITOR_IDENTITY_CERTIFICATE_CHAIN`,
  ADR 0021). The first certificate must be for the identity key, or the
  identity does not start. Served at
  `GET /.well-known/x509CertificateChain.pem` below the DID's path
  (`/participants/a/.well-known/...` for `did:web:host:participants:a`)
  and named by `x5u` in the DID document. The file is read again every minute; a new file for another
  key is refused and logged, and the old chain stays.
- Identity: Gaia-X onboarding (ADR 0021), on with
  `PORTITOR_GAIAX_COMPLIANCE_URL`, `PORTITOR_GAIAX_COMPLIANCE_DID` and
  `PORTITOR_GAIAX_PARTICIPANT_FILE`.
  The identity signs `gx:LegalPerson` and `gx:Issuer` VC-JWTs, sends them
  with the notary's registration number credential to the compliance
  service (`POST {url}/api/credential-offers/standard-compliance`,
  `application/vp+jwt`), checks the `gx:ParticipantComplianceCredential`
  it answers (issued by `PORTITOR_GAIAX_COMPLIANCE_DID` and signed by its
  `did:web` key, type, id, every
  `digestSRI`) and stores the four credentials. Runs at start without a
  valid compliance credential, then at two thirds of its validity, at
  least a minute after the last good run; failed runs are retried with
  the outbox's delays. A run the service refuses keeps the start of its
  answer (up to 2 KiB) in the run's error and log. Management API:
  `POST /v1/gaiax/compliance` (run now) and `GET /v1/gaiax/compliance`
  (last runs). Public: `GET /credentials/gaiax/{name}` for the three
  credentials sent, below the DID's path like the chain; credential ids
  keep that path, so participants on one host do not collide. A
  credential request's `holderPid` must not start with `gaiax:` (400):
  the identity keeps its Gaia-X credentials under those ids.
- `portitor-dcp`: `x509` (the P-256 key of a PEM chain's first
  certificate, our own small DER reader), `gaiax_sign` (the Gaia-X
  credentials, presentation and answer check), and
  `SigningKey::sign_with_headers`. `portitor-runtime`:
  `HttpSender::post_text`, `did_document::add_x5u`, and `retry` (the
  outbox's retry delay).

### Changed

- ADR 0021 (Gaia-X onboarding) and ADR 0022 (Gaia-X compliance in
  policies) accepted by the maintainer before the release.
- Roadmap 0.10: explicit request size limits per port (today axum's 2 MB
  default on parsed bodies), filing the upstream reports, and the interop
  line without Simpl-Open and Gaia-X.
- The DID documents of the identity and the issuer give their key
  `alg: ES256` (RFC 7517 section 4.4; Gaia-X needs it). DCP ignores it.
- The presentation API: a scope type `T` also matches a credential of
  type `gx:T` whose `@context` names a Gaia-X context (ADR 0021), so
  `ParticipantComplianceCredential` finds
  `gx:ParticipantComplianceCredential`. Plain types match as before.
- ADR 0005 (schema and vocabulary registry, with the bundle format) and
  ADR 0020 (dataset harvest) accepted by the maintainer after the 0.7
  release.
- Docs: the roadmap lists key custody and key rotation as technical
  debt (one key per component today, no overlap when it changes; a key
  custody service such as OpenBao to be investigated). DEPLOYING and
  SECURITY-ASSURANCE describe the bundle that follows the newest version
  instead of a pinned one, and the 0.6 to 0.7 upgrade names database
  schema version 8. The 0.7.0 entry carries its tag date.
- CI: the DCP TCK jar is cached between runs (`actions/cache`, keyed on
  its SHA-256, checked after every restore or download), and the DSP TCK
  build's Gradle dependencies too (`setup-java` cache), since Maven
  Central answered HTTP 429; downloads are retried.
- CI: a Gaia-X end-to-end test (`scripts/gaiax-e2e.sh`, workflow
  `Gaia-X`): Portitor identities get a compliance credential from a local
  gx-compliance v2.16.0 in test mode (ADR 0021), and a Portitor provider
  agrees a contract only with a holder of one (ADR 0022). No real Gaia-X
  trust anchor: a test CA and a test-only notary
  (`scripts/gaiax_test_notary.py`). See docs/TESTING.md.
- Version 0.8.0 in `Cargo.toml` and every OpenAPI file in `docs/api/`
  (the documentation check needs both). The DID document descriptions
  in `identity-openapi.json` and `issuer-openapi.json` name `alg`, `x5u`
  and the issuer's bundle key.
- Docs, release review: the C4 diagrams show the Gaia-X compliance
  service (C1, C2), the certificate chain and the onboarding in the
  identity (C2, C3) and the trusted compliance services of the connector
  (C3); a new C4 diagram of the Gaia-X onboarding flow
  (`c4-gaiax-onboarding-code.drawio.svg`). ARCHITECTURE has a Gaia-X
  section; DEPLOYING has the 0.7 to 0.8 upgrade (identity database
  schema version 3, no way back without the backup) and the Gaia-X
  limits, and no longer says a new bundle version needs a restart;
  TESTING lists the Gaia-X unit and integration tests;
  SECURITY-ASSURANCE covers Gaia-X; the website shows 0.8 as done.

## [0.7.0] - 2026-11-15

### Added

- Dataset harvest into a read copy (ADR 0020). The connector reads its
  datasets from a source, today the datasets file
  (`PORTITOR_DATASETS_FILE`, same format), at start and every
  `PORTITOR_HARVEST_INTERVAL_SECONDS` (default 3600; `0`: only at start),
  one run at a time. Each dataset is checked on its own (JSON, DSP
  `Dataset` form, catalog rules, ODRL constraints, `portitor:source`); a
  dataset that passes is added or updated, one that fails is rejected and
  its previous good version stays. A dataset no longer in the source is
  withdrawn: out of the catalog, new contract requests refused, transfers
  under agreements already made still work. When the rules change, stored
  datasets are checked again at start and those that fail are hidden.
  Each run commits in one transaction; the last 100 runs are kept.
- Management API: `GET /v1/harvest-jobs` and `GET /v1/harvest-jobs/{id}`
  (read only): each run's trigger, outcome, counts, summary and the
  error of each rejected dataset.
- `portitor-shacl`, our own SHACL Core subset on Oxigraph's RDF crates
  (ADR 0005): `sh:class`, `sh:datatype`, `sh:nodeKind`, `sh:minCount`,
  `sh:maxCount`, `sh:hasValue`, `sh:node`, `sh:or`, `sh:pattern`,
  `sh:property`; class and node targets; property, inverse, sequence and
  alternative paths. Any other SHACL term is refused when shapes are
  loaded; `sh:extends` and `sh:shape` are ignored with a warning. JSON-LD
  to RDF with contexts from a given map only. Tested with 42 W3C SHACL
  core tests, the HealthDCAT-AP release 8 and DCAT-AP 3.0.0 shapes, and
  against the EU ITB validator on the 0.6 spike samples.
- Signed schema and vocabulary bundle (ADR 0005, "Bundle format"):
  `portitor-issuer bundle build` builds and signs a bundle offline
  (`manifest.json`, compact JWS ES256 `manifest.jws`, typ
  `portitor-bundle+jws`); JSON Schema in
  `docs/schemas/bundle-manifest.schema.json`. The issuer lists the bundle
  key as `<did>#bundle-key` (`PORTITOR_ISSUER_BUNDLE_KEY_FILE`) and serves
  built bundles read only at `GET /bundles/{version}/{path}`
  (`PORTITOR_ISSUER_BUNDLE_DIR`). `bundle build` also writes
  `latest.json`, the newest version (unless `--not-latest`; never back to
  an older `issued`), served at `GET /bundles/latest.json` with
  `Cache-Control: no-cache`.
- The connector pulls the authority's bundle (`PORTITOR_BUNDLE_URL` or
  `PORTITOR_BUNDLE_DIR`, the bundles folder; `PORTITOR_BUNDLE_AUTHORITY`),
  verifies it against the authority's DID document and every digest, and
  keeps it in a disk cache (`PORTITOR_BUNDLE_CACHE_DIR`) used only when
  the source cannot be reached. It follows the newest version
  (`PORTITOR_BUNDLE_VERSION=latest`, the default; or one fixed version),
  looked for every `PORTITOR_BUNDLE_REFRESH_SECONDS` (default 3600; `0`:
  only at start). A new version is put in use without a restart, then
  the stored datasets are checked again and the source harvested again.
  It never goes back to a version issued before the one in use. Each
  check is in the harvest job log with `"kind": "bundle"` (migration to
  schema version 8).
- New crate `portitor-bundle`: the bundle source (`BundleSource`,
  `UrlSource`, `FolderSource`) and read copy (`BundleStore`,
  `CacheFolder`), so other services can reuse them. The manifest's
  `issued` is a UTC date and time (`YYYY-MM-DDThh:mm:ssZ`, DCAT 3
  `dcterms:issued`). How schemas are collected and stored is recorded as
  technical debt in `docs/ROADMAP.md`.
- With a bundle, each harvested dataset is checked against the SHACL
  shapes for its `dct:accessRights` value (read with the DSP context only;
  its `@id` must be an absolute IRI); a dataset with violations is
  rejected with the SHACL report in its harvest job, one that passes is
  served with `"dct:conformsTo": {"@id": <profile>, "@type": "dct:Standard"}`.
  With message shapes, DSP negotiation and transfer messages are checked
  in and out (SHACL `MessageGuard`): an inbound one that fails is answered
  400, an outbound one is not sent and its process ends (logged as a
  rule breach, not as an unreachable counter-party). The catalog
  `@context` stays the DSP context alone: write metadata terms outside it
  as full IRIs. A bundle's `context` files serve only to read messages
  whose `@context` names them. A bundle that cannot be loaded, or whose
  shapes the engine refuses, stops the start.
- Demo bundle sources for HealthDCAT-AP release 8
  (`examples/bundle/healthdcat-ap-r8`: release 8 and DCAT-AP 3.0.0
  shapes, reduced code lists), `scripts/demo-bundle.sh` to build and
  sign it, `scripts/reduce_code_lists.py` to make the reduced code lists,
  and `examples/healthdcat-datasets.json` (full IRIs; two datasets that
  pass, `bed-occupancy` with 7 violations).
- The demos (`scripts/demo.sh`, `scripts/demo-compose.sh`) run with the
  demo bundle: the issuer builds and serves it, the provider follows it and
  offers the HealthDCAT-AP datasets (the catalog shows `dct:conformsTo`,
  the harvest job the SHACL report of `bed-occupancy`); the restricted
  dataset is transferred under its `MembershipCredential` constraint.
  The provider follows the issuer's `latest.json`.
  The EDC interop test runs with the bundle too: EDC reads a
  HealthDCAT-AP dataset with full-IRI metadata, negotiates and transfers
  it. `scripts/dcp-issuer-tck.sh` has a `BUNDLE_KEY=1` variant (the DID
  document lists `#bundle-key`), run in CI as well.
- New fuzz targets: `shacl_load`, `shacl_jsonld`, `bundle_manifest`.
- ADR 0020 (datasets harvested into a read copy) proposed, and ADR 0005
  updated with the bundle format for 0.7; ROADMAP 0.7 rewritten for the
  approved plan.

### Changed

- A distribution in the catalog keeps its own metadata (`@id`,
  `dcat:accessURL`, DCAT-AP properties); before, only `@type`, `format`
  and `accessService` were served.
- The Compose demo and `scripts/demo.sh` use
  `examples/healthdcat-datasets.json` instead of `examples/datasets.json`
  and `examples/compose/datasets.json` (removed); its offers cover the
  same checks (changed terms, member and auditor offers, the
  `DataProcessorCredential` offer and its revocation). The provider's
  management API is on `127.0.0.1:18081` in the Compose demo.
- **Behaviour change:** a datasets file that cannot be read, or a dataset
  that breaks a rule (including a credential constraint on a type not in
  `PORTITOR_POLICY_CREDENTIALS`, or a `portitor:source` that is not an
  HTTP(S) URL), no longer stops the connector at start. The dataset is
  rejected and logged, and the connector starts without it. To keep a
  strict start, check `GET /v1/harvest-jobs` in a start-up probe.
- Datasets live in the connector's SQLite database (migration to schema
  version 7: read copy, harvest jobs and errors); the catalog and the
  data plane read the copy in memory, replaced as a whole after a run
  that changed it.
- The log warning about a dataset without `portitor:source` now comes
  from the harvest (`harvest: dataset has no portitor:source; its
  transfers will be refused`); the 0.6 start warnings are gone.
- ADR 0018 (issuer component, with its amendment from the 0.6 review) and
  ADR 0019 (credential constraints) accepted by the maintainer.
- `PORTITOR_HARVEST_INTERVAL_SECONDS` and `PORTITOR_DATA_TOKEN_SECONDS`
  above `31536000` (one year) are refused at start. Before, a huge
  harvest interval aborted the connector right after start.
- The harvest's rules version names the Portitor version
  (`dsp(...; portitor <version>)`), so after an upgrade the stored
  datasets are checked again at start (a `bundle` job), and the start run
  checks every entry.
- Harvest limits: a datasets file larger than 256 MiB fails the run; a
  job stores at most the first 1000 entry errors (the summary says when
  more were rejected); a stored SHACL report keeps its first 50 results
  and says how many more in `resultsNotStored`.
- The bundle cache removes the `.partial-*` and `.old-*` folders a
  failed or stopped write left behind, and a failed swap puts the old
  copy back.

### Security

- The bundle is signed with its own bundle key, separate from the
  issuer's credential key; the issuer refuses to start if they are the
  same, and needs only the public key. The connector accepts only the
  configured version, signed (ES256 only) by the bundle key
  `<authority>#bundle-key` that the authority's `did:web` document lists
  under `assertionMethod`; a bundle signed with any other key of the
  authority, such as its credential key, is refused, and `bundle build`
  refuses any other `--kid`. Every file's SHA-256 must match; a
  bundle that fails a check is never replaced by the cache, and never
  replaces the version in use. `latest.json` is not signed, so the
  connector refuses a version issued before the one in use (no
  rollback, by the manifest's `issued`, the DCAT 3 `dcterms:issued`, a
  UTC date and time; `bundle build` writes the time of the build by
  default). Limits: manifest 1 MiB, 64 MiB per file, 256 MiB in all, 120 s
  per download.
- No JSON-LD context is fetched from the network: `portitor-shacl` reads
  contexts only from a map the caller fills (the bundle's and the DSP
  contexts). No Portitor binary parses RDF/XML: `bundle build` refuses it,
  and code lists travel as N-Triples.
- `portitor-shacl` refuses shapes that use a SHACL term it does not
  implement, so nothing passes silently.
- `portitor-shacl` checks each node against a shape about once, also
  with recursive shapes on data with loops (a layered graph that took 79 s
  now takes milliseconds), and one validation makes at most 1,000,000
  checks (`MAX_CHECKS`): past that, the report has a Violation
  (`urn:portitor:shacl:WorkLimit`), never a pass.
- `portitor-shacl` refuses a `sh:pattern` that Rust's `regex` would read
  differently from `XPath`: a `[` inside a character class (`XPath` class
  subtraction such as `[a-z-[aeiou]]`), and with the `x` flag a `#` or
  white space inside a class.
- The issuer's public bundle route sends a file in 64 KiB pieces as it
  reads it, with its `Content-Length`, instead of reading the whole file
  into memory for each request, and keeps each version's parsed manifest
  (read again only when the file changes).

### Documentation

- Four more C4 level 4 diagrams (transfer, DSP caller authentication with
  DCP, issuance, presentation), and corrections from a review of the docs,
  the website and the diagrams against the 0.6 code.
- Two new C4 level 4 diagrams (dataset harvest; bundle publish and load);
  C1, C2 and C3 diagrams show the harvest, the read copy, the shape
  source, the SHACL checks and the issuer's bundles.
- DEPLOYING: dataset harvest, schema and vocabulary bundle (connector and
  issuer), new settings and log messages, upgrade note "From 0.6 to 0.7".
  README, ARCHITECTURE, TESTING, SECURITY-ASSURANCE, RUST-IDIOMS and the
  website updated for 0.7.
- RUST-IDIOMS: log fields (`%`, `?`), `self: &Arc<Self>`,
  `spawn_blocking`, an `async` block passed to `timeout`, `Box<dyn
  Iterator>` and boxed errors, `map_or`, `entry().or_default()`, function
  pointers, a crate's trait for a standard type, tuple-struct `.0` and
  `*guard =`, `usize::from(bool)`, byte strings, `include_str!`, `as`
  against `try_from`, and `const fn`.

## [0.6.0] - 2026-11-14

### Added

- `portitor-issuer`, the issuer component and third image (ADR 0018),
  run once per dataspace by its authority. Public port (8070): its
  did:web document with an `IssuerService` entry; under
  `PORTITOR_ISSUER_SERVICE_URL` the issuer metadata (`GET /metadata`),
  credential requests (`POST /credentials`, `201 Created` with
  `Location`) and their status (`GET /requests/{issuerPid}`), DCP 6.4,
  6.7 and 6.8, and the status lists (`GET /status-lists/vc11`, `/vc20`).
- `portitor-issuer`: only participants the authority registered may
  call; anyone else gets 401 before its DID is resolved, and nothing is
  stored. An unknown credential id ends `REJECTED`, and so does a request
  beyond `PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` (default 10 active
  credentials per holder and definition); a repeated `holderPid` of the
  same holder is refused (409). Credential types come from
  `PORTITOR_ISSUER_CREDENTIALS_FILE`, checked at start.
- `portitor-issuer`: issued credentials go to the holder's Storage API in
  one `CredentialMessage`, through an outbox, with a self-issued ID token
  that carries back the holder's access token. No answer, 401, 408, 429
  and 5xx are retried for about 8 hours; any other 4xx ends the delivery
  `FAILED` at once. A request whose decision failed is decided again
  within a minute.
- `portitor-issuer`: one Bitstring Status List credential per profile
  (131,072 entries, `revocation`), each credential at a random free
  position, signed again on each revocation.
- `portitor-issuer`: management API (8071, API key) for participants,
  offers, issued credentials and revocation
  (`docs/api/issuer-management-openapi.json`); the public port is
  described in `docs/api/issuer-openapi.json`.
- `portitor-identity` asks issuers for credentials (holder side of DCP
  issuance, ADR 0018): `POST /v1/credential-requests` with
  `credentialIds` finds the issuer's `IssuerService` in its DID document,
  checks the ids against its metadata and sends a
  `CredentialRequestMessage` (DCP 6.4) with a self-issued ID token that
  carries an access token. `GET /v1/credential-requests` lists requests,
  `GET /v1/credential-requests/{holderPid}` shows one with its status at
  the issuer (DCP 6.8), and `POST /v1/offers/request` asks for what an
  issuer offered (DCP 6.6). Only trusted issuers are asked; offers are
  never answered by themselves. Without `credentialIds` the request is
  only recorded, as before.
- Credential constraints in ODRL policies (ADR 0019): an offer may
  require a claim of the consumer's credentials, with the left operand
  `<CredentialType>.<claim path>` and `eq`, `neq`, `isAnyOf` or
  `isNoneOf`. A missing claim never grants access; anything else fails
  closed. The provider asks the consumer's credential service for the
  scopes the offer needs (with the access token the consumer sent) at the
  contract request, and again at each transfer request, so a revoked
  credential stops new transfers. As consumer, the connector grants those
  scopes in its tokens.
- Connector setting `PORTITOR_POLICY_CREDENTIALS`: the credential types
  offers may use, with their scopes (`MembershipCredential` is always
  there); an offer on another type stops the connector at start.
- `portitor-dcp`: the DCP issuance messages `CredentialRequestMessage`
  and `CredentialStatus` (DCP v1.0.1 sections 6.4 and 6.8), checked
  against the official schemas; `RequestStatus::as_str` and `from_name`;
  `issuance` signs credentials of any type and claims in both profiles,
  status list credentials and the delivery token.
- `portitor-runtime`: `caller` checks a DCP caller's self-issued token at
  the HTTP edge, shared by the identity component and the issuer; so are
  the component's own DID document (`did_document`), the base path of a
  service URL (`settings::base_path`), `http::answered_status` and
  `HttpSender::post_for_location` (the `Location` of a `201 Created`).
- DCP TCK v1.2.1 issuer tests (41) in the `TCK` workflow against
  `portitor-issuer` (`scripts/dcp-issuer-tck.sh`).
- Release pipeline for the third image,
  `ghcr.io/brunopacheco1/portitor-issuer`: a `portitor-issuer` target in
  the `Dockerfile` (ports 8070 and 8071, `/data/portitor-issuer.db`),
  built, signed, attested and released with an SBOM like the other two;
  CI builds it on every pull request.
- Demos with a second credential type (ADR 0019): `scripts/demo.sh` and
  the Compose demo issue the consumer a `DataProcessorCredential`, its
  `processing-records` offer checks it, and once the issuer revokes it a
  new transfer request is refused. The example datasets add an offer for
  members only (`MembershipCredential.memberOf`) and one that the demo
  credentials do not meet. The demo credential definitions are in
  `examples/issuer-credentials.json`.
- ADR 0018 (issuer component) and ADR 0019 (credential constraints in
  ODRL), both proposed.
- Spike report `docs/spikes/0.6-shacl-engine.md` on the SHACL engine for
  ADR 0005; the maintainer chose an own SHACL Core subset, built in 0.7.
- C4 model diagrams in draw.io format (system context, containers,
  components of the three images, and the provider negotiation as code),
  shown in `docs/ARCHITECTURE.md`.
- `docs/api/identity-openapi.json`: the identity's public interface (DID
  document, STS, DCP credential service); a test checks every route in it
  is served.
- `scripts/check_docs.py`, run in CI: the diagrams name every crate and
  image and only files and types that exist; every OpenAPI file has the
  workspace version.
- A documentation review step for every release (`docs/RELEASING.md`).
- `docs/DEPLOYING.md`: an issuer section (who runs it, order, key backup,
  never on a participant's host); deployment order issuer, then identity,
  then connector, also in `docs/ARCHITECTURE.md` and on the website.

### Changed

- Roadmap: the schema and vocabulary registry moves to 0.7, Gaia-X to
  0.8, operations to 0.9 and hardening to 0.10; ARCHITECTURE,
  SECURITY-ASSURANCE and the website follow.
- `portitor-identity`: a delivery (`CredentialMessage`) for a request the
  component sent to the issuer must carry back, in the issuer's ID token,
  the access token sent with the request (DCP 6.1). Requests only
  recorded need none, as before.
- `portitor-identity`: a repeated delivery (same `issuerPid` and status
  for a request it closed) is answered `200` and stores nothing, instead
  of `400`. A credential request is withdrawn only when the issuer
  refuses it with an HTTP 4xx (not 408 or 429); after a timeout, a
  network error, 408, 429 or 5xx it stays open for a late delivery, and
  the management API answers `202` with a `sendError`.
- `portitor-identity` database: a forward-only migration adds the
  request's credential ids, status URL, access token id, creation time,
  `sendError` and `issuerPid`.
- DEPLOYING: after a reissue, delete the revoked credential from the
  identity (`DELETE /v1/credentials/{id}`); the identity does not read
  status lists, and verifiers refuse a presentation that holds it.
- Connector, DCP verifier: the log names a credential that fails its
  checks and why (`credential refused`). Such a credential still makes
  its presentation invalid and refuses the caller, also next to a valid
  `MembershipCredential` (DCP v1.0.1, 5.4.3, step 8).
- Connector: `auth::Caller` is a struct with the verified credentials;
  `CallerVerifier` gets `credentials` (ask the caller for more scopes),
  `Prover::authorization` takes the extra scopes to grant, and
  `HeldAgreements` gets `agreement`.
- `portitor-dsp::policy`: `validate`, `permissions_hold` and
  `prepare_datasets` take the configured credential types and the
  caller's credentials; the provider services take the caller's
  credentials on a contract request, an acceptance and a transfer
  request.
- As consumer, the connector does not accept a provider's offer (DSP
  `OFFERED`) that has credential constraints: it cannot check its own
  credentials as the provider will.
- The demos, `scripts/measure.sh`, `scripts/dev-identities.sh` and the EDC
  interop test run `portitor-issuer` as the data space's authority: it
  registers the participants, each identity asks it for its credentials
  through its own management API, and revocation is
  `POST /v1/credentials/{id}/revoke`. EDC IdentityHub v1.0.0-RC7 asks it
  through its own Identity API. `scripts/demo.sh` and `scripts/measure.sh`
  take the issuer binary as a third argument, `scripts/demo-compose.sh`
  the issuer image (`PORTITOR_ISSUER_IMAGE`); `TEST_ISSUER` is gone. The
  Compose demo's default images are 0.6.0.
- The EDC interop test signs the IdentityHub's Identity API tokens itself
  (a test-only RSA key per run, `openssl`).
- Readability review, no behaviour change: clearer issuer names, one
  `Directory` port and a typed `HttpError` in `portitor-runtime`, shared
  `merge_scopes` and `HeldCredential::has_type`, shorter functions, and
  more patterns explained in `docs/RUST-IDIOMS.md`.
- Documentation review: the C4 diagrams show the identity's issuer
  client and the issuer's modules; the OpenAPI files, README, TESTING,
  SECURITY-ASSURANCE and the website match the release; the website
  also publishes the issuer's management API description.

### Removed

- The test-only `test-issuer` example of `portitor-identity`, its
  fixture key and its dev-dependencies (`hyper`, `http-body-util`)
  (ADR 0018).

### Fixed

- The CI check that domain crates have no I/O dependencies (ADR 0004) also
  refuses SQLite, TLS and low-level network crates; ADR 0004 records why
  `portitor-dcp` takes signing randomness from the operating system.
- The website publishes the identity's and the issuer's OpenAPI
  descriptions next to the connector's, and its API page links all of
  them.
- `docs/ARCHITECTURE.md` no longer lists PostgreSQL as required: each
  image keeps its state in SQLite until the PostgreSQL adapter exists.
- TESTING: the local DSP TCK run sets `PORTITOR_TCK_AUTHORIZATION`, as CI
  does.

## [0.5.0] - 2026-11-13

### Added
- EDC interop test (`scripts/edc-interop.sh`, workflow `EDC interop`):
  Portitor's connector and identity against an EDC v1.0.0-RC7 control
  plane, data plane and IdentityHub in Docker Compose. Credentials
  checked, catalog, negotiation and pull transfer, both ways, and
  revocation both ways. Runs on pull requests that touch the code and
  nightly. The EDC runtimes are built from Maven Central
  (`interop/edc/runtimes`).
- `test-issuer`, a test-only demo authority (an example of
  `portitor-identity`, never in a release image; ADR 0015): writes its
  `did:web` site and a 16 KiB Bitstring Status List, signs
  `MembershipCredential`s in both DCP profiles, delivers them to an
  identity component through its management and Storage APIs, and
  revokes them. `portitor-dcp` gains `status_list::set_bit` and
  `MIN_LIST_BYTES`.
- ADR 0017 (proposed): how to join a Simpl-Open data space. No
  x509/mTLS profile in Portitor; Simpl-Open's connector speaks DSP
  2025-1 but not DCP, and its agents authenticate at a Tier 2 gateway.
- DCP v1.0.1 building blocks in `portitor-dcp` (0.5, ADR 0015), used by
  the identity component and the connector: the presentation query and response
  messages, the credential and offer messages of the issuance protocol,
  scopes (`org.eclipse.dspace.dcp.vc.type`, `...vc.id`, and configured
  aliases such as EDC's), and status list checks for revocation
  (Bitstring Status List and StatusList2021). The DCP JSON Schemas and
  examples (tag v1.0.1) are bundled for tests. Two new fuzz targets:
  `dcp_messages`, `dcp_status_list`.
- Verifiable credentials and presentations as JWTs in both DCP profiles
  (ADR 0015): `vc11-sl2021/jwt` (VC Data Model 1.1, `vc` and `vp`
  claims, as the DCP TCK uses) and `vc20-bssl/jwt` (VC Data Model 2.0,
  `vc+jwt` and `vp+jwt`, including the `EnvelopedVerifiablePresentation`
  wrapper EDC Identity Hub sends). Issuer keys need `assertionMethod`,
  holder keys `authentication`; a presentation may not mix profiles.
  Fuzz target `dcp_credentials`.
- `portitor-time`: the `xsd:date` and `xsd:dateTime` parser moves out
  of the ODRL code, so credentials can use it too.
- `portitor-runtime`: the adapters the connector and the identity
  component share (outbound HTTP client, API key check, logging,
  shutdown) move out of the connector, with a new `did:web` resolver
  that caches documents for 5 minutes (ADR 0016).
- `portitor-identity`, the identity component (ADR 0015): its `did:web`
  document (one P-256 key from a PEM file), a Secure Token Service
  (`POST /sts/token`, DCP's `identity-trust-sts-api.yaml`: self-issued ID
  tokens, with an access token for `bearer_access_scope`), the
  presentation API (scopes only, one presentation per profile), the
  Storage API (trusted issuers, recorded requests only, proofs checked)
  and credential offers (sparse ones completed from the issuer's
  metadata). Management API on its own port: credential requests,
  credentials, offers (`docs/api/identity-management-openapi.json`).
  Credentials in SQLite. Settings in `DEPLOYING.md`, section 4.
- The DCP TCK v1.2.1 credential service tests (presentation and holder
  side of issuance, 54 tests) run in CI against `portitor-identity`.
- `portitor-dcp`: presentations and access tokens a holder signs, PEM
  keys, JWK thumbprints, issuer metadata. Fuzz target `dcp_access_token`.
- `portitor-runtime`: opening and migrating SQLite files.
- Caller authentication in the connector (DCP, ADR 0016). Every DSP
  endpoint, provider endpoints and consumer callbacks alike, needs a
  self-issued ID token; the connector checks it, queries the caller's
  credential service, verifies the presentations (both profiles) and
  requires a `MembershipCredential` about the caller from a trusted
  issuer, not revoked or suspended (status lists must be signed by the
  issuer; fail closed). A refusal is HTTP 401 with a DSP error. Every
  outgoing DSP request carries a fresh token from the participant's STS for
  the counter-party's DID. At start the connector checks that its DID
  document and its STS agree. New settings: `PORTITOR_STS_URL`,
  `PORTITOR_STS_CLIENT_ID`, `PORTITOR_STS_CLIENT_SECRET_FILE`,
  `PORTITOR_TRUSTED_ISSUERS`, `PORTITOR_DCP_SCOPES`.
- The DCP TCK v1.2.1 verifier tests (22) run in CI against the connector,
  with `portitor-identity` as its identity.
- `/.well-known/dspace-version` names the authentication: DCP 1.0, both
  profiles.
- `portitor-dcp`: reading the status list of a status entry from a status
  list credential (`status_list::list_in`).
- Release pipeline: a second image, `ghcr.io/brunopacheco1/portitor-identity`
  (ports 8090 and 8091, state in `/data/portitor-identity.db`), built,
  signed and attested like the connector's. One `Dockerfile` with a build
  target per image; the image jobs run as a matrix over the components.
  CI builds and starts both images.
- `PORTITOR_STS_ALLOW_HTTP` (connector): plain HTTP for the STS URL only,
  for an STS on the internal network. An `http://` `PORTITOR_STS_URL`
  without it stops the start.
- Fuzz target `dcp_did_document`: DID document parsing and key selection.

### Changed
- Readability pass for the maintainer's review (no behaviour change
  apart from the bearer rule above): `portitor-dcp`'s `lib.rs` is split
  into `jws.rs`, `keys.rs`, `did.rs` and `id_token.rs` (same public
  paths); `holder::Validity` is now `holder::TokenFrame` (field `jti`);
  `Credential::verify` and `Presentation::verify` take `u64` seconds;
  new `Credential::is_only_about`; the unused `DidResolver` trait is
  gone. In `portitor-runtime`: `BoxFuture` replaces `SendFuture` (and the
  identity component's `Lookup`), `callback_allowed` is
  `outbound_url_allowed`, and `settings::comma_list` and
  `api_key::internal_error` are shared. `docs/RUST-IDIOMS.md` covers the
  patterns the 0.5 code uses.
- One rule reads `Authorization: Bearer <token>` everywhere
  (`portitor_runtime::http::bearer_token`): DSP requests, the data plane,
  the identity component's STS callers and both management APIs now all
  accept the scheme name in any case (`bearer`, RFC 7235) and ignore
  spaces around the token. Before, the identity component and the
  management API keys required exactly `Bearer `.
- Release assets: one provenance bundle per image,
  `<component>-X.Y.Z.intoto.jsonl`, instead of `portitor-X.Y.Z.intoto.jsonl`.
  `docker build` needs `--target portitor-connector` or
  `--target portitor-identity` (without it, the connector is built).
- Self-issued ID tokens with an `iat` in the future are refused, and used
  token ids are forgotten once their token expires.
- A presentation query with both `scope` and `presentationDefinition` is
  an error (DCP 5.4.1); one with only a definition gets 501.
- A VC 1.1 credential whose `jti` differs from its `id` is accepted (the
  DCP TCK writes both); the `id` counts.
- The `DSP TCK` workflow is now called `TCK` and has a second job for the
  DCP TCK.
- The demos (`scripts/demo.sh`, `scripts/demo-compose.sh` with
  `examples/compose`) run with DCP: each participant is a connector and
  its own `portitor-identity`, holding a `MembershipCredential` the demo
  authority (`test-issuer`) delivers. The demo checks that a catalog
  request without a token gets 401. `demo.sh` takes the identity binary
  as a second argument, `demo-compose.sh` the identity image; both use
  `TEST_ISSUER`. The Compose demo keeps its keys and secrets in
  `examples/compose/run/` (written by the script) and the compose file's
  default images are 0.5.0. CI's connector image check now expects it to
  refuse to start without the DCP settings.
- **Breaking:** the connector does not start without `PORTITOR_STS_URL`
  and the other DCP settings, and refuses unauthenticated DSP requests.
  Each negotiation and transfer is bound to the participant that started
  it; as provider, another caller is told it does not exist, and as
  consumer, only the provider it was started with may use the callbacks.
  Upgrading from 0.4: finish or terminate negotiations and transfers in
  progress first, and negotiate again for agreements made by 0.4 (their
  consumer is `urn:portitor:unauthenticated`, so no transfer starts under
  them). See `DEPLOYING.md`, "From 0.4 to 0.5".
- The management API refuses a `providerId` that is not a `did:web` DID,
  and an agreement whose assigner is not the provider is refused.
- The catalog request endpoint also reads the expanded JSON-LD request
  the DCP TCK sends.
- The VS Code configurations and the `_http/` Bruno collection run with
  DCP: a task (`scripts/dev-identities.sh`) starts the demo authority and
  the provider's and consumer's identities with their credentials, the
  connectors use them, and the collection gets a token from the
  consumer's STS before every DSP request.
- The DSP TCK job sends a fixed `Authorization` header, which the `tck`
  build (`PORTITOR_TCK_AUTHORIZATION`) maps to the TCK participant.
- The Storage API takes one answer per credential request (ADR 0015):
  `ISSUED` or `REJECTED` closes it, and a renewal needs a new request.
  The DCP TCK run records one request per test that delivers.
- As consumer, an agreement whose `assignee` is not our participant id is
  refused (400).
- A scope for the type `VerifiableCredential` is refused (STS
  `invalid_scope`, presentation API 400), and the connector checks
  `PORTITOR_DCP_SCOPES` at start.
- The empty `tck` feature of `portitor-identity` is gone.

- `scripts/measure.sh` runs with DCP: it starts a provider and a
  consumer identity as the demo does, and measures with the new
  `dsp-load` example (a fresh token per request) instead of `oha`. The
  0.5 numbers and the cost of DCP per DSP request (about 2 ms, idle) are
  in ADR 0002, section 7.

- The EDC interop test's Gradle build verifies the SHA-256 of every
  downloaded plugin and library
  (`interop/edc/runtimes/gradle/verification-metadata.xml`).

### Fixed
- EDC can read Portitor's catalog: a catalog request whose `filter`
  selects nothing (EDC always sends a `QuerySpec` with only paging) gets
  the whole catalog instead of 400. Paging is ignored.

### Security
- Status lists are GZIP data from other parties: decompression is capped
  at 16 MiB, and uses `miniz_oxide` without its unsafe SIMD feature
  (banned in `deny.toml`), with Portitor's own CRC-32 check.
- Both components refuse self-issued ID tokens valid for more than an
  hour, and remember at most 100,000 used token ids (new tokens are
  refused when full; expired ids are dropped in order of expiry).
- The connector's caller check skips credentials from untrusted issuers
  before fetching anything for them, reads at most 8 presentations, 16
  credentials each and 4 status entries per credential, and ends after
  20 s. The `did:web` cache holds at most 1,000 documents. The public
  ports answer 503 after 60 s and work on at most 256 requests at once.

## [0.4.0] - 2026-11-13

### Added
- Demo in Docker Compose (`examples/compose`, `scripts/demo-compose.sh`):
  a provider, a consumer and a web server for the provider's data. Two
  connectors agree a contract, transfer a file, fetch it and complete the
  transfer. CI runs it with the image it builds. `scripts/demo.sh` now
  does the same transfer without Docker.
- Transfer process as consumer (DSP 2025-1, ADR 0014): the callback
  endpoints `/dsp/2025-1/callback/transfers/...`, and in the management
  API `POST /v1/transfers` (under the agreement of one of our finalized
  negotiations), `GET /v1/transfers`, `GET /v1/transfers/{consumerPid}`
  (with the data address once started), and `.../completion` and
  `.../termination`. Your own system fetches the data with that address.
  The address and its token are kept in the SQLite file while the transfer
  can be used, and removed when it ends (new migration).
- The provider's policy may now also suspend, start again, complete or
  terminate a transfer. When the provider starts a pull transfer again,
  its data plane gives a new token, since DSP requires a data address in
  that start message.
- DSP TCK: the transfer tests as provider (TP) and as consumer (TP_C) are
  now required in CI (65 tests in all).
- `tests/end_to_end.rs`: two connectors negotiate, transfer, fetch the
  data and complete, in one process.
- Transfer process as provider (DSP 2025-1, ADR 0013): the
  `/transfers/...` endpoints. A request is accepted only under a finalized
  agreement held by the caller, whose permissions still hold, in a format
  the dataset offers and the data plane serves (`HttpData-PULL`; no push).
  The provider then starts the data flow and sends the data address. The
  consumer may suspend, start again, complete or terminate; the data plane
  is told first, then the transfer is saved. Transfers are kept in the
  SQLite file with the same outbox as negotiations (new migration).
- Built-in HTTP pull data plane (ADR 0013): each dataset may name its data
  with `portitor:source` in the datasets file (an HTTP URL, never shown in
  the catalog). A started transfer gets the address
  `{PORTITOR_PUBLIC_URL}/data/{flowId}` with a random bearer token; the
  connector streams the source's answer while the transfer is started and
  the token is fresh (`PORTITOR_DATA_TOKEN_SECONDS`, default 3600). Data
  flows and token hashes are kept in the SQLite file (new migration). A
  request for a dataset without a source is refused. Cargo feature
  `data-plane`, on by default.
- Bridge from the transfer process to Data Plane Signaling
  (`data_flows.rs`): repeated signals are harmless.
- Transfer process (DSP 2025-1) in `portitor-dsp`: the five message types,
  `DataAddress`, the acknowledgement and error types, and the state
  machine as a pure function. Checked against every transfer example and
  JSON Schema of the specification, and fuzzed (`dsp_transfer`).
- Data Plane Signaling v1.0-RC4 in the new domain crate `portitor-dps`: the
  data flow messages, the data flow state machine, and the two ports
  (`DataPlane`, called by the control plane; `ControlPlane`, called back by
  the data plane). Checked against the specification's JSON Schemas and
  examples. No data plane implements the port yet.
- VS Code debugging: shared `.vscode/launch.json` (CodeLLDB) for the connector
  and every test target, tasks, and recommended extensions. See
  [TESTING.md](docs/TESTING.md#debugging-in-vs-code).
- `docs/RUST-IDIOMS.md`: a short guide to the Rust patterns the code uses,
  for readers who use Rust now and then.

### Changed
- Readability review before the release (no behaviour change): the
  connector's wiring is in `ports.rs`, outbound HTTP in `http_client.rs`,
  and ids and time in `system.rs`, instead of all in `negotiation.rs`;
  one wrapper (`Shared`) for every port chosen at start; full names
  instead of one-letter ones; comments where a pattern hides the intent
  (the DSP message macros, boxed futures and errors, proof of delivery).
  In Rust code, the port wrappers moved from `negotiation`, `consumer`,
  `transfer` and `data_plane` to `portitor_connector::ports`, and
  `Sender` and `SendFuture` to `portitor_connector::http_client`.

### Fixed
- `.vscode/launch.json` was not valid JSON (a test configuration was cut
  short).
- ADR 0014 and ARCHITECTURE described the DSP TCK race slightly wrongly:
  the TCK's consumer has the transfer but marks it REQUESTED only after
  reading the answer, and its start handler removes itself even after a
  refusal.

## [0.3.0] - 2026-11-12

### Added
- Contract negotiation (DSP 2025-1) in `portitor-dsp`: the six message types,
  the acknowledgement and error types, and the state machine as a pure
  function. Checked against every negotiation example and JSON Schema of
  the specification, and fuzzed (`dsp_negotiation`).
- ODRL logical constraints (`and`, `or`, `xone`, `andSequence`), duties
  and profiles in policies, so agreements from other connectors parse.
- Provider side of contract negotiation: the DSP 2025-1 negotiation
  endpoints, a default policy that agrees to catalog offers, and delivery of
  offers, agreements, finalization and termination to the consumer's
  callback with retries.
- Consumer side of contract negotiation: the DSP 2025-1 consumer callback
  endpoints under `/dsp/2025-1/callback`, a default policy that accepts the
  provider's offer and verifies its agreement, and delivery of requests,
  acceptance, verification and termination to the provider with retries.
  Consumer negotiations are kept in the same SQLite file (schema version 2;
  migrated at start).
- Minimal ODRL policies (ADR 0009): the `dateTime` constraint with `eq`,
  `neq`, `lt`, `lteq`, `gt`, `gteq` and the logical constraints; anything
  else fails closed. The provider agrees only to an offer with the
  catalog's terms whose constraints hold now, and terminates otherwise;
  the consumer accepts only an offer whose constraints hold and verifies
  only an agreement with the terms it asked for. A datasets file with a
  constraint that is not understood stops the connector at start.
- Each outgoing negotiation message is saved before it is sent (an
  outbox), on both sides, and sent again after a restart. A partner's
  message that arrives before its acknowledgement of ours is recorded is
  taken as proof that ours arrived, and accepted at once; this replaces
  the 3 second wait. A failed send first asks the partner for its state
  before terminating. Writes compare a version number. SQLite schema
  version 3; migrated at start. See ADR 0007.
- Management API on its own port (`PORTITOR_MANAGEMENT_BIND`, default
  `127.0.0.1:8081`), off unless `PORTITOR_MANAGEMENT_API_KEY_FILE` is set:
  start a negotiation as consumer, read one, list them. Every call needs
  the API key as a bearer token. Its OpenAPI description is generated from
  the code (utoipa) and served at `/v1/openapi.json`.
- Negotiations are kept in a SQLite file (`PORTITOR_DB_FILE`, default
  `portitor.db`; `/data/portitor.db` in the image, mount a volume there). See
  ADR 0006.
- `MessageGuard` port: every inbound and outbound negotiation message goes
  through it, so a dataspace can add its own rules, for example SHACL shapes
  (ADR 0005). The default allows everything.
- `PORTITOR_ALLOW_HTTP_CALLBACKS`: allow `http://` callback addresses
  (development and tests only; `https://` only by default).
- The container image now carries the CA certificate bundle (Mozilla's,
  as packaged by Debian, MPL-2.0) so the connector can verify `https`
  callback addresses.
- Docs: `docs/DEPLOYING.md` (runbook for operators), `docs/TESTING.md`
  (how to test, for contributors) and `docs/api/openapi.json` (OpenAPI 3.1
  description of the HTTP interface, using the official DSP JSON Schemas),
  also published on the website (`/api/`).
- The connector stops cleanly on SIGTERM (`docker stop`, Kubernetes); it
  used to wait to be killed.
- The DSP TCK now runs the 15 provider negotiation tests (CN:01 to CN:03)
  and the 16 consumer negotiation tests (CN_C:01 to CN_C:03), with a
  test-only `tck` build feature that scripts decisions and adds the
  endpoint the TCK uses to start consumer tests.
- A [Bruno](https://www.usebruno.com/) collection in `_http/` for manual
  tests: metadata, catalog, provider-side negotiation and the management
  API, with environments for one connector and for two (see
  `docs/TESTING.md`).
- `scripts/demo.sh`: two connectors on one machine agree a contract and
  refuse changed terms. CI runs it on every pull request.

### Changed
- **The project is renamed from LDSC to Portitor** ([ADR 0008](docs/adr/0008-project-name.md)).
  This breaks existing deployments of 0.1 and 0.2:
  - Image: `ghcr.io/brunopacheco1/portitor-connector` (was `ldsc-connector`).
    Images 0.1.0 and 0.2.0 stay at the old name with their signatures.
  - Binary and crates: `portitor-connector`, `portitor-dsp`, `portitor-dcp`.
  - Environment variables: `PORTITOR_*` instead of `LDSC_*`
    (for example `PORTITOR_DB_FILE`, `PORTITOR_PUBLIC_URL`).
  - Repository and website move to `brunopacheco1/portitor`.
- Minimum Rust version 1.89 (was 1.85), needed by `rusqlite`.
- A dataset definition whose offer has a `target` is refused at start;
  DSP forbids `target` on offers inside a catalog.
- The example datasets file uses a `dateTime` constraint instead of
  `spatial`, which is not understood yet.

## [0.2.0] - 2026-11-12

### Added
- Datasets are read from a JSON file (`LDSC_DATASETS_FILE`) of DSP 2025-1
  `Dataset` objects, so each dataset has its own offers and distributions.
  Definitions are checked at start (at least one offer and one
  distribution, unique ids); a broken file stops the start.
- Datasets keep metadata properties LDSC does not interpret (for example
  `dct:title`) and the catalog serves them; before, they were dropped.
- [ADR 0005](docs/adr/0005-schema-and-vocabulary-registry.md) (proposed): schema and vocabulary registry run by the
  dataspace authority, built in 0.6.
- Fuzzing with cargo-fuzz and ClusterFuzzLite: three targets for untrusted
  input (DSP catalog messages and dataset definitions, DCP token parsing,
  DCP token validation), five minutes on each pull request and 30 minutes
  weekly.

### Changed
- The catalog and data service `@id`s are derived from `LDSC_PUBLIC_URL`
  and stay the same across restarts (they were random UUIDs).
- **Breaking:** `LDSC_DATASETS` is removed; the connector refuses to start
  while it is set, so a catalog never goes empty silently.
- Roadmap: 0.2 raises the OpenSSF scores: Best Practices to **passing**
  (moved from 0.1) and Scorecard fuzzing and branch protection.

### Fixed
- DCP token validation added `leeway` to the token's `exp` and to the
  current time without overflow checks. A token with `exp` near the u64
  maximum panicked in debug builds; release builds wrapped around and
  rejected the token, so no invalid token was accepted. Found by the new
  fuzz target.

## [0.1.0] - 2026-11-12

### Added
- Technology-stack spike (0.0): Rust accepted ([ADR 0002](docs/adr/0002-technology-stack.md)),
  three images ([ADR 0003](docs/adr/0003-component-split.md)), hexagonal
  architecture ([ADR 0004](docs/adr/0004-hexagonal-architecture.md)).
- `ldsc-connector`: DSP 2025-1 version endpoint, catalog request and dataset
  request; passes the DSP TCK metadata and catalog tests.
- `ldsc-dcp`: validation of DCP 1.0 self-issued ID tokens (ES256, PS256).
- Repository foundations: Apache-2.0 licence, REUSE compliance, code of
  conduct (Contributor Covenant 2.1), contributing guide, governance,
  security policy, security assurance case, releasing guide, issue forms
  and pull request template.

- Supply-chain checks: Dependabot (Cargo, GitHub Actions), CodeQL (Rust,
  Actions), OpenSSF Scorecard, dependency review, OSV-Scanner, cargo-deny
  (`deny.toml`), ORT, and SonarQube Cloud with coverage.
- Release pipeline on signed `v*` tags: multi-arch (amd64, arm64) images
  on GHCR, signed with cosign, with SBOM and build provenance attestations,
  and a GitHub release with binaries and third-party notices.
- OCI image labels (`org.opencontainers.image.*`) in the `Dockerfile`, and
  the same annotations on the multi-arch image index.
- Project website in `site/`, published on GitHub Pages; check values are
  read at build time, so the page loads nothing from other sites.
- OpenSSF Scorecard and Best Practices badges in the README, the Best
  Practices level on the website, and a README section on the HTTP
  interface and configuration.

### Security
- `base64` is built without its default `simd-unsafe` feature, and
  `deny.toml` bans that feature.

### Removed
- The Java on Quarkus native bake-off slice; its results stay in ADR 0002.
