<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# ADR 0029: Several replicas on one PostgreSQL database

- Status: **Accepted** (decided by the maintainer, 2026-11-26)
- Date: 2026-11-26
- Scope: release 1.3
  ([ROADMAP](../ROADMAP.md#13-several-replicas-and-postgresql-released-2026-11-27-as-v130)).
  Amends [ADR 0006](0006-negotiation-storage.md) (one process per
  SQLite file) and the data line of the stack that
  [ADR 0002](0002-technology-stack.md) proposed (sqlx). Answers
  [TD-08](../RISKS_AND_TECHNICAL_DEBTS.md#td-08),
  [TD-09](../RISKS_AND_TECHNICAL_DEBTS.md#td-09) and
  [TD-50](../RISKS_AND_TECHNICAL_DEBTS.md#td-50) to
  [TD-58](../RISKS_AND_TECHNICAL_DEBTS.md#td-58).

## Context

Each image keeps its state in one SQLite file, and only one process may
use the file (ADR 0006). So a participant cannot run two containers of
an image, neither for load nor to survive the loss of one.

The maintainer set a principle on 2026-11-26
([ARCHITECTURE, principle 14](../ARCHITECTURE.md#architecture-principles)):
correctness never depends on state held in one process. Several
containers of each image (connector, identity, issuer) share one
database, and the processes coordinate through it. Release 1.2 already
did this for one case: the lease on a provider's transfer while its
data plane step runs
([ADR 0007](0007-outbox-and-proof-of-delivery.md), note on the lease).

A code review on 2026-11-26 found the places where 1.2 still keeps
state in one process: used token ids, the dataset copy, the issuer's
status lists, the switches that cut running downloads, the outbox
loops, the push and upload guards, the bundle in use, the scheduled
jobs, the issuer's deliveries and the DID document during a key change
(TD-09, TD-50 to TD-58).

Eclipse EDC, our reference counterpart
([ADR 0011](0011-interoperability-targets.md)), runs several replicas
on one PostgreSQL database. Where it solves the same problem, this ADR
follows it and names the place in its code.

[STABILITY](../STABILITY.md) promises every 1.0 setting until 2.0,
`PORTITOR_DB_FILE` among them.

## Decision

### 1. PostgreSQL is the database; SQLite is deprecated

- From 1.3, each image can keep its state in PostgreSQL. Several
  containers of an image may then share one database.
- PostgreSQL 16 to 19 are supported. CI tests the oldest and the newest
  (16 and 19).
- **SQLite is deprecated in 1.3.** It still works, for one process. A
  component that uses a SQLite file logs a warning at start. SQLite and
  `PORTITOR_DB_FILE` are removed in 2.0. Until then STABILITY keeps the
  setting, as semantic versioning requires.
- PostgreSQL becomes the documented default: DEPLOYING, the demo, the
  quickstart and the scenarios use it.
- One database per component. A table `portitor_schema (component,
  version)` records which component owns a database. Another component
  refuses to use it.

### 2. Settings

- `PORTITOR_DATABASE_URL`: a PostgreSQL URI in the form libpq reads,
  for example
  `postgresql://portitor@db.internal:5432/portitor?sslmode=verify-full`.
- The password only in a file, `PORTITOR_DATABASE_PASSWORD_FILE`. A URL
  that holds a password is refused, so it never shows in a process
  list, a log or an error.
- TLS: `sslmode=verify-full` is the default when `sslmode` is missing.
  `verify-full`, `verify-ca` and `require` all mean the same in
  Portitor: TLS, with the server's certificate **and** its host name
  checked. `sslrootcert=<PEM file>` names a private CA, then the only
  one trusted, as in libpq (`sslrootcert=system`: the system's CAs,
  also the default). `disable` turns
  TLS off, for a private network only. `prefer` and `allow` are
  refused: they fall back to plain text without saying so.
- Both `PORTITOR_DATABASE_URL` and `PORTITOR_DB_FILE` set: the
  component refuses to start.

Portitor reads `verify-ca` and `require` more strictly than libpq does.
The maintainer chose this: a database connection that does not check
the host name is not offered.

### 3. The PostgreSQL client (D1)

- `tokio-postgres` 0.7. TLS is Portitor's own glue on `tokio-rustls`,
  with the system's CA certificates (the image's bundle, or
  `SSL_CERT_FILE` and `SSL_CERT_DIR`), or those of `sslrootcert`. Not
  `tokio-postgres-rustls`, which brings its own bundled root store.
- **The store ports stay synchronous.** A store call waits on the
  calling thread, as it waited for the SQLite lock before. A database
  thread of its own drives the sockets
  (`crates/portitor-runtime/src/database/postgres.rs`). The wait is
  bounded: a statement timeout (30 s) on the server, TCP keepalives and
  a TCP user timeout on the socket, and 40 s for an answer, after which
  the connection is closed (DEPLOYING, "Database"). Asynchronous
  store ports are on the roadmap ("Later").
- **SQL is written once** for both databases, with `$1`, `$2`
  placeholders (rewritten for SQLite). Only the schema differs.
- **Transactions on PostgreSQL are `SERIALIZABLE`.** When PostgreSQL
  answers that two transactions collided (SQLSTATE `40001` or
  `40P01`), the transaction runs again. So a store keeps the "one
  writer at a time" behaviour it had on SQLite, and the
  compare-and-update writes of ADR 0007 keep their meaning.

### 4. Migrations (D2)

- Each component has its own PostgreSQL migration list. It starts with
  one baseline: the 1.3 SQLite schema of that component, as a whole. A
  test compares the two schemas, so they cannot drift apart.
- Migrations run at start, **forward only**, as before
  ([TD-22](../RISKS_AND_TECHNICAL_DEBTS.md#td-22)).
- They run under a PostgreSQL advisory lock, in one transaction.
  Replicas that start together migrate once: the first one migrates,
  the others wait and then find nothing to do. Flyway, which EDC uses
  for its migrations, takes the same kind of lock on PostgreSQL.
- A release never edits a released migration; it adds one to both
  lists.

### 5. SQLite: one process per file is enforced (D3)

A second live process on the same SQLite file refuses to start. The
first process holds a lease row in the file and renews it while it
runs. Before 1.3 this rule was only written in DEPLOYING.

### 6. Coordination through the database

The pattern of 1.2's transfer lease, for every place the review found.
A lease has a holder (a random id of the process, made at start, never
a host name) and an end time, 30 seconds after it was taken or renewed.
A process that dies leaves its leases to expire; another process then
takes the work over. Leases use each process's own clock, as EDC does:
the clocks of the hosts must be in sync (NTP).

| Item | What 1.3 does | EDC precedent |
|---|---|---|
| [TD-09](../RISKS_AND_TECHNICAL_DEBTS.md#td-09) used token ids | A table `used_token (token_id, expires_at)` in all three images. One `INSERT ... ON CONFLICT DO NOTHING` decides: a token whose id is already there is a replay. A row is kept until its token expires | The `edc_jti_validation` table |
| [TD-50](../RISKS_AND_TECHNICAL_DEBTS.md#td-50) dataset copy (D4) | The catalog, and each dataset the DSP code needs, is read from the database on each request. No copy in memory decides an answer | EDC reads its catalog from its asset index (`AssetIndex`) on each request |
| [TD-51](../RISKS_AND_TECHNICAL_DEBTS.md#td-51) status lists (D5) | The process that revokes signs the new status list in the same transaction and stores it in the database. Every replica serves the stored list | EDC IssuerService keeps its status list credentials in its database |
| [TD-52](../RISKS_AND_TECHNICAL_DEBTS.md#td-52) cuts of running downloads and pushes (D6) | A watchdog in each running download and push reads its flow's state from the database every 5 seconds and stops it when the state changed. A suspension or termination that lands on another container takes up to 5 seconds | none: EDC has no built-in data plane |
| [TD-53](../RISKS_AND_TECHNICAL_DEBTS.md#td-53) DSP outbox | Lease columns on the outbox, and a loop in every process that takes the due messages nobody leases. Nothing waits for a restart | `nextNotLeased` and the lease columns (`extensions/common/sql/sql-lease`) |
| [TD-54](../RISKS_AND_TECHNICAL_DEBTS.md#td-54) pushes and uploads | A lease per running push and per consumer upload, renewed while it runs. The loop of TD-53 takes over an expired one | as TD-53 |
| [TD-55](../RISKS_AND_TECHNICAL_DEBTS.md#td-55) bundle (S3) | The bundle version in use moves to the database; each process moves to it. The cache folder stays per container | none |
| [TD-56](../RISKS_AND_TECHNICAL_DEBTS.md#td-56) scheduled jobs | A lease per kind of job (the dataset harvest, the Gaia-X renewal). Job ids come from a database sequence or identity column. No "mark every running run as failed" at start | as TD-53 |
| [TD-57](../RISKS_AND_TECHNICAL_DEBTS.md#td-57) issuer deliveries | A lease per delivery before it is sent | as TD-53 |
| [TD-58](../RISKS_AND_TECHNICAL_DEBTS.md#td-58) DID document during a key change (D7) | No code. A two-step rolling key rotation in [OPERATING](../OPERATING.md#rotating-a-key-with-several-replicas) (below) | none |

**D7, the key rotation.** Step 1: list the new **public** key on every
replica as a retired key ("listed, never signs"), then restart the
replicas one by one. Step 2: switch the key file to the new key and
list the old public key as retired, then restart them one by one. Once
step 1 is done, every replica lists both keys, so during step 2 a
partner finds the signing key in the document of any replica. The
start checks allow step 1: a retired
key file must hold a public key that is not the current key
(`read_retired_keys` in `crates/portitor-runtime/src/did_document.rs`).

### 7. Tests (S4)

- The store tests run on SQLite and on PostgreSQL 16 and 19 in CI. A
  missing PostgreSQL fails CI instead of skipping the tests
  ([TESTING](../TESTING.md#postgresql)).
- A test runs two processes of each image on one PostgreSQL database.
- One DSP TCK run on PostgreSQL.
- Before the tag, the maintainer runs the scenarios on PostgreSQL on his
  laptop, one of them with two connectors behind nginx.

### 8. Not in 1.3 (S5)

A tool that copies a SQLite file into PostgreSQL comes in a later 1.x
release, before 2.0.

## Consequences

- Operators can run several containers of each image. They run and back
  up a PostgreSQL server, one database per component.
- A 1.2 SQLite file is still read by 1.3, for one process. Moving its
  data to PostgreSQL waits for the copy tool (S5). Until then a move to
  PostgreSQL starts with an empty database.
- The hosts' clocks matter more: a lease taken on a host whose clock is
  ahead ends early on the others. DEPLOYING already asks for NTP.
- A cut of a running download or push on another container takes up to
  5 seconds (D6). On the same container it stays immediate.
- Each catalog request reads the database (D4), which costs more than
  a read from memory.
- A database call still blocks the thread that makes it
  ([TD-40](../RISKS_AND_TECHNICAL_DEBTS.md#td-40)). With PostgreSQL the
  wait includes a network round trip.
- `tokio-postgres` brings 32 crate versions that the imported cargo-vet
  audits do not cover ([R-04](../RISKS_AND_TECHNICAL_DEBTS.md#r-04)).
- The schema exists twice, once per database, until 2.0 removes
  SQLite. The schema test keeps them equal.
- The demo, the quickstart and the scenarios need a PostgreSQL
  container.

## Alternatives rejected

- **sqlx**, which ADR 0002 proposed before the first line of storage
  code. It is asynchronous only, and its compile-time checked queries
  need a database, or a prepared cache, at build time. `rusqlite`
  stays for SQLite, so sqlx would only serve PostgreSQL, behind
  synchronous ports, with more crates than `tokio-postgres`.
- **Asynchronous store ports now.** Every port, adapter and caller would
  change in the same release as the database. Kept for later
  ("Later" in the ROADMAP); the measured cost of blocking is in
  [TD-40](../RISKS_AND_TECHNICAL_DEBTS.md#td-40).
- **PostgreSQL `LISTEN`/`NOTIFY`** to tell the other processes about a
  change. A notification is lost when a process is not connected at
  that moment, so correctness would still need the reads of this ADR.
  It could make things faster later; it never decides.
- **A generation number for the dataset copy** (the first
  recommendation for TD-50): each process keeps its copy and loads it
  again when a number in the database moved. The maintainer chose
  option B, the catalog read on each request (D4), as EDC does: no copy
  in memory can answer with old data, not even for a moment.
- **A revocation counter for the status lists** (the earlier text of
  TD-51): each process signs its lists again when a counter moved. Two
  processes would sign the same list at once, and a replica serves an
  old list until it notices. Signing once in the revoking transaction
  (D5) leaves one signed list, in the database.
- **The DID document in the database** (TD-58). The document comes from
  the key files, which are mounted into each container. Keeping it in
  the database would make the database decide which keys a participant
  lists, and still need the key files to agree. The two-step rotation
  (D7) needs no code.
- **Dropping SQLite in 1.3.** It would break the
  [STABILITY](../STABILITY.md) promise for `PORTITOR_DB_FILE`, and
  every 1.x deployment that uses it. Deprecated now, removed in 2.0.

## Note added in 1.4 (2026-11-27): no copy tool

The maintainer dropped the copy tool of decision 8 (S5) on 2026-11-27:
no deployment needs one yet, so no data has to be copied. A component
moved to PostgreSQL starts with an empty database
([OPERATING](../OPERATING.md#from-sqlite-to-postgresql)). SQLite is
still removed in 2.0 ([TD-59](../RISKS_AND_TECHNICAL_DEBTS.md#td-59)).

## Note added in 1.4 (2026-11-27): the credential status check

The identity has a new scheduled job: it reads the status lists of the
credentials it holds every `PORTITOR_CREDENTIAL_STATUS_CHECK_SECONDS`
(default 60) and records what they say
([TD-20](../RISKS_AND_TECHNICAL_DEBTS.md#td-20), maintainer decision
D6). It runs as the dataset harvest of decision 6: one round at a time
under the lease `job`/`credential_status_check` in the identity's
database, kept after a round until a second before the next tick
(`crates/portitor-identity/src/status_check.rs`). Its result is a row
change per credential, guarded by the credential's JWT, so a round run
twice records the same answers again.
