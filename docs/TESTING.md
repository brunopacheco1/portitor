<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Testing Portitor

For contributors: how to check a change before you open a pull request,
how the project checks every change, and how to run the demos by hand.
All commands run from the repository root. Deploying for real is in
[DEPLOYING](DEPLOYING.md).

## What runs where

| Check | Command | Runs in CI |
|---|---|---|
| Format | `cargo fmt --all --check` | every PR that changes more than prose (`CI`) |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | every PR that changes more than prose (`CI`) |
| Unit and integration tests | `cargo test --workspace` | every PR that changes more than prose (`CI`) |
| Coverage | `cargo llvm-cov --workspace` | every PR that changes more than prose (`CI`, sent to SonarQube Cloud) |
| Dependencies: advisories, licences, sources | `cargo deny check` | every PR that changes more than prose, and weekly (`Dependencies`) |
| Dependencies: every crate version audited or exempted | `cargo vet --locked`, see [CONTRIBUTING](../CONTRIBUTING.md#dependency-audits-cargo-vet) | every PR that changes more than prose (`Dependencies`) |
| Licence headers | `reuse lint` | every PR (`REUSE compliance`) |
| Documentation matches the code (C4 diagrams, OpenAPI versions) | `python3 scripts/check_docs.py` | every PR (`CI`) |
| DSP conformance (TCK) | see [below](#dsp-conformance-tck) | every PR that changes more than prose (`TCK`) |
| DCP conformance: identity component, connector as verifier, issuer (TCK) | see [below](#dcp-conformance-tck) | every PR that changes more than prose (`TCK`) |
| Interoperability with EDC v1.0.0-RC8, with Gaia-X compliance | `scripts/edc-interop.sh`, see [below](#edc-interop-test) | PRs that touch `crates/`, `Cargo.toml`, `Cargo.lock`, the `Dockerfile`, `.dockerignore`, `interop/edc/`, `interop/gaiax/compose.yaml`, `examples/bundle/`, the scripts it uses or its workflow, and nightly (`EDC interop`) |
| Gaia-X compliance between Portitor participants, against a local Gaia-X compliance service in test mode | `scripts/gaiax-e2e.sh`, see [below](#gaia-x-end-to-end-test) | PRs that touch `crates/`, `Cargo.toml`, `Cargo.lock`, the `Dockerfile`, `.dockerignore`, `examples/issuer-credentials.json`, `interop/gaiax/`, the scripts or the workflow, and nightly (`Gaia-X`) |
| Fuzzing | see [below](#fuzzing) | PRs that touch `crates/`, `fuzz/`, `.clusterfuzzlite/`, `Cargo.toml` or `Cargo.lock` (short), weekly (long) |
| Two participants authenticate with DCP, agree a contract and transfer a file | `examples/demos/two-party/demo.sh`, see [below](#two-connectors) | every PR that changes more than prose (`CI`) |
| The same in Docker Compose | `examples/demos/two-party/demo-compose.sh`, see [below](#two-connectors) | every PR that changes more than prose (`CI`) |
| The health demo: five parties, a chained transfer through an HDAB, data permits as credentials, refusals | `examples/demos/health-lu/run.sh`, see [its README](../examples/demos/health-lu/README.md) | no: by hand, a manual test (maintainer, 2026-11-15) |
| The container scripts with Podman instead of Docker | `CONTAINER_ENGINE=podman` and the script, see [below](#docker-or-podman) | no: by hand, last run noted there |
| Images build and start | `docker build --target portitor-connector` / `--target portitor-identity` / `--target portitor-issuer`; the connector and the issuer refuse to start without their settings, the identity serves its DID document | every PR that changes more than prose (`CI`) |
| Reproducible build: same binaries and image digests | see [RELEASING](RELEASING.md#reproducible-builds) | after each release (rebuild of the tag, compared with the release), weekly and when the build files change on `main`, and PRs that change the build files (built twice, compared) (`Reproducible build`) |
| Static analysis | CodeQL (see [below](#codeql)), OpenSSF Scorecard, ORT | CodeQL: every PR, every push to `main` and weekly; a finding that is not on the accepted list fails the job (`CodeQL`). Scorecard: on `main`; ORT: when a manifest changes |

"Prose" is the short list of paths that no job reads; which jobs a
change needs is decided by `scripts/ci_changes.sh`, and the checks a
pull request must pass are the gate jobs: see
[CONTRIBUTING](../CONTRIBUTING.md#required-checks). To see what CI will
do with a change:

    git diff --name-only --no-renames main...HEAD | scripts/ci_changes.sh classify

You need: Rust (the version in `.github/workflows/ci.yml`; at least the
`rust-version` in `Cargo.toml`), and for some checks `cargo-deny`, `cargo-vet`,
`cargo-llvm-cov`, `reuse`, Java 21, a nightly Rust toolchain, and Docker or Podman
([below](#docker-or-podman)). Before you
open a pull request, run at least format, lint, tests, `cargo deny
check` and `cargo vet --locked`.

## Docker or Podman

The scripts that start containers (`examples/demos/two-party/demo-compose.sh`,
`scripts/edc-interop.sh`, `scripts/gaiax-e2e.sh`) run with Docker or with
Podman. `scripts/container-engine.sh` chooses, and only that file:

1. `CONTAINER_ENGINE=docker` or `CONTAINER_ENGINE=podman` in the
   environment: that engine. Any other value is an error.
2. Otherwise Docker, if the `docker` command exists and `docker info`
   answers.
3. Otherwise Podman, if the `podman` command exists and `podman info`
   answers.

A `docker` command that is Podman under another name counts as Podman.
The commands in this file and in the other documents are written with
`docker`; with Podman, write `podman` instead (`podman build`,
`podman compose`, `podman run`).

CI runs everything with Docker. The Podman runs are done by hand:

| Script | Podman run that passed |
|---|---|
| `examples/demos/two-party/demo-compose.sh` | 2026-11-22, tag `v1.0.0-rc.1` (release images): rootless Podman 4.9.3 (netavark 1.4.0, aardvark-dns 1.4.0), Docker Compose 5.7.2, Pop!\_OS 24.04 LTS on amd64; about 7 seconds; 36 of 40 runs passed (the other 4: see below) |
| `examples/demos/two-party/demo-compose.sh` | 2026-11-20, commit `d857330` (0.13.0 before the tag): Podman 6.1.4 with machine 5.8.5 (libkrun, rootless, 6 CPUs, 8 GiB), Docker Compose 5.6.1, macOS 26.7.3 on Apple Silicon, bash 5.3, OpenSSL 3.5; 7 seconds, with the three `:ci` images as arguments |
| `scripts/gaiax-e2e.sh` | the same day and machine; 44 seconds |
| `scripts/edc-interop.sh` | the same day and machine; 12 minutes, most of it the Gradle builds of EDC |

**What Podman needs**

- **Docker Compose** as the provider of `podman compose`: the
  `docker-compose` binary (macOS: `brew install docker-compose`).
  `podman compose` only starts a provider and prefers `docker-compose`
  to `podman-compose`
  ([podman-compose(1)](https://github.com/containers/podman/blob/main/docs/source/markdown/podman-compose.1.md.in)).
  The scripts refuse `podman-compose`: its `run` has no `--quiet`
  (podman-compose 1.6.0), and it has not been tried with these Compose
  files.
- **On Linux, rootless Podman with its API socket and container DNS.**
  Ubuntu 24.04 and Pop!\_OS 24.04 ship Podman 4.9.3, which has
  `podman compose` (added in 4.7.0). netavark and aardvark-dns, which
  give the containers their names in DNS, are not dependencies of the
  `podman` package there, so name them:
  `sudo apt install podman netavark aardvark-dns slirp4netns uidmap`.
  `podman compose` points Docker Compose at the API socket, so start it:
  `systemctl --user enable --now podman.socket`. Install the
  `docker-compose` binary from the
  [Docker Compose releases](https://github.com/docker/compose/releases)
  into `/usr/local/bin` (the Docker Engine is not needed).
  **Known failure with Podman 4.9.3:** about one run in ten fails at the
  first catalog request (502 from the consumer; the provider logs
  `DSP request not checked: a service the check needs is unavailable`,
  `client error (Connect)`): for about 20 seconds after the containers
  start, the provider cannot reach the consumer's identity component by
  its name. Run the script again. Each try gives up after 5 seconds, the
  DNS timeout, so the name lookup (aardvark-dns 1.4.0) is the likely
  cause; we have not proven it. Asking again for 15 seconds did not help.
  Ubuntu 26.04 ships Podman 5.7.0 with aardvark-dns 1.16.0; not tried yet.
- **On macOS, a Podman machine** that runs (`podman machine start`), with
  the repository under your home folder: the machine mounts `/Users`,
  `/private` and `/var/folders`
  ([containers.conf(5)](https://github.com/containers/common/blob/main/docs/containers.conf.5.md),
  `[machine]`, `volumes`), and the scripts mount only files inside the
  repository (`run/` folders).
- **Enough memory for the EDC interop test.** A new machine has 2048 MiB.
  The test builds four EDC runtimes with Gradle, then runs four JVMs
  (three before 0.14), Memgraph, the compliance service (Node.js),
  Vault, three web servers and three Portitor components. With three
  JVMs it passed on a machine with 6 CPUs and 8 GiB of memory; with
  four it has not been run with Podman yet. Our estimate for the least,
  made for three, is 4 CPUs, 8 GiB of
  memory, 40 GiB of disk
  (`podman machine init --cpus 4 --memory 8192 --disk-size 40`, or
  `podman machine set` on a stopped machine). The Compose demo and the
  Gaia-X test should fit in 4 GiB (also an estimate).
- **bash 4.4 or later** for every script in `scripts/` (they check it
  first, `scripts/require-bash.sh`). macOS has bash 3.2 as `/bin/bash`:
  `brew install bash`; the scripts start with `#!/usr/bin/env bash`, so
  the newer one is used once it comes first in `PATH`. Also `jq`, and for
  the two interop tests `python3` with `cryptography`. Homebrew's
  Python refuses `pip install` outside a virtual environment (PEP 668):
  `python3 -m venv .venv && .venv/bin/pip install cryptography`, then
  put `.venv/bin` first in `PATH`. The scripts were run with OpenSSL 3
  only. On macOS, `/usr/bin/openssl` is LibreSSL: `brew install openssl@3`
  and put its `bin` first in `PATH`.
  `scripts/bench.sh` and `scripts/measure.sh` run on Linux only (they
  read `/proc`).
- **Linux binaries for the machine's architecture.** The `Dockerfile`
  copies static binaries from `dist/<arch>/`. On an Apple Silicon Mac
  the Podman machine is arm64: build `aarch64-unknown-linux-musl` and
  copy to `dist/arm64/`. One way that needs nothing but the engine (it is what the
  release workflow does on an arm64 runner; about 8 minutes on the Mac
  above):

  ```sh
  podman run --rm -v "$PWD:/src" -w /src docker.io/library/rust:1.97.0 sh -c '
    apt-get update -qq && apt-get install -y -qq musl-tools &&
    rustup target add "$(uname -m)-unknown-linux-musl" &&
    cargo build --release --locked --target "$(uname -m)-unknown-linux-musl" \
      -p portitor-connector -p portitor-identity -p portitor-issuer'
  mkdir -p dist/arm64
  cp target/aarch64-unknown-linux-musl/release/portitor-{connector,identity,issuer} dist/arm64/
  podman build --format docker --target portitor-connector -t portitor-connector:ci .
  podman build --format docker --target portitor-identity -t portitor-identity:ci .
  podman build --format docker --target portitor-issuer -t portitor-issuer:ci .
  ```

**Known differences, and what the repository does about them**

| Difference | Handled by |
|---|---|
| Podman needs the registry in an image name unless short names are configured ([containers-registries.conf(5)](https://github.com/containers/image/blob/main/docs/containers-registries.conf.5.md), "Short-Name Aliasing"). | Every pulled image is written in full (`docker.io/library/nginx:...`) in the Compose files and Dockerfiles. Images built here (`portitor-connector:ci`) are found locally. |
| `podman build` writes images in OCI format, and Podman reads a health check only from an image in Docker format ([libimage `inspect.go`](https://github.com/containers/common/blob/main/libimage/inspect.go); `--format` in podman-build(1)). The same holds for a pulled image with an OCI manifest. | No script relies on an image's `HEALTHCHECK`: `examples/demos/two-party/compose.yaml` has its own `healthcheck:` for the services others wait for. To keep the image's check, build with `podman build --format docker`; for `podman run`, pass `--health-cmd '["/portitor-connector", "health"]'`. |
| Docker's name server inside a container is `127.0.0.11`; Podman's (aardvark-dns) listens on the network's gateway. | The `tls` web server of the two interop tests takes its `resolver` from the container's `/etc/resolv.conf` (the nginx image's `NGINX_ENTRYPOINT_LOCAL_RESOLVERS`). |
| Rootless Podman (the default, also in a Podman machine): root in the container is the user who runs Podman; any other user id cannot write into that user's folders. | The demo bundle is built in a container as `0:0` with rootless Podman and as your user otherwise (`engine_bind_mount_user` in `scripts/container-engine.sh`). The scripts make the files they mount readable by all (`chmod a+rX`), as before: the images run as user 65532. |
| `podman compose` prints a line about its provider on each call. | The scripts set `PODMAN_COMPOSE_WARNING_LOGS=false`. |
| Docker Compose builds through Podman without BuildKit (`podman compose` sets `DOCKER_BUILDKIT=0`). | The Dockerfiles use no BuildKit-only syntax. If the build of the EDC images fails under Podman, build them with `podman build --target <controlplane, dataplane, identityhub or issuerservice> -t portitor-interop/edc-<target>:1.0.0-RC8 interop/edc/runtimes` and run `scripts/edc-interop.sh --no-build`. |

Not needed, so not used: `host.docker.internal` (no container calls the
host), ports below 1024 on the host (all published ports are 18000 and
up, on `127.0.0.1`), SELinux labels on mounts (`:z`). Not handled and
not run: Podman on a Linux host with SELinux enforcing, where the mounted `run/` folders
would need `:z`; and a Docker CLI that talks to a Podman socket
(`DOCKER_HOST`), which the scripts take for Docker.

## Building the images locally

The container scripts take the released images, or the ones you name.
To test your own changes, build the three images as CI does. The
`Dockerfile` copies static binaries from `dist/<arch>/`:

```sh
mkdir -p dist/amd64
cargo build --release --target x86_64-unknown-linux-musl -p portitor-connector -p portitor-identity -p portitor-issuer
cp target/x86_64-unknown-linux-musl/release/portitor-{connector,identity,issuer} dist/amd64/
docker build --target portitor-connector -t portitor-connector:ci .
docker build --target portitor-identity -t portitor-identity:ci .
docker build --target portitor-issuer -t portitor-issuer:ci .
```

On an arm64 machine (an Apple Silicon Mac), see "What Podman needs"
above for a build that needs nothing but the container engine.

The release build is reproducible: `scripts/build-release.sh` gives the
same binaries, bit for bit, from any checkout path, and the workflow
`Reproducible build` checks it, images included (see
[RELEASING](RELEASING.md#reproducible-builds)).

## Unit and integration tests

```sh
cargo test --workspace
cargo test -p portitor-connector --features tck   # also the TCK scripts
```

**Changed one, check the other three.** The negotiation and transfer
logic exists four times in near-copies: provider and consumer, for
negotiations and for transfers
([TD-42](RISKS_AND_TECHNICAL_DEBTS.md#td-42)). A fix or a new test for
one of them most often applies to the other three; the table of files is
in [READING-THE-CODE](READING-THE-CODE.md#the-three-siblings).

Where the tests are:

- **The freeze guard** (`tests/frozen_interface.rs` in the three
  binaries' crates): the HTTP routes and environment variables listed in
  [STABILITY.md](STABILITY.md) are the ones the code has. A test fails
  when a listed name leaves the code, and when the code gains a name
  that is not listed. The top of each file says what to do then. The
  tools are in `crates/portitor-runtime/src/testing/frozen.rs`.
- **Domain crates** (`crates/portitor-dsp`, `crates/portitor-dcp`,
  `crates/portitor-dps`): pure functions
  and ports, tested without network or disk. Examples: the negotiation state
  machine is checked for every (state, message, sender) combination;
  `tests/negotiation_spec.rs` parses every example of the DSP
  specification, writes it back unchanged and validates it against the
  official JSON Schema (`tests/schemas`, copied unchanged from the
  specification). `portitor-dps` does the same for Data Plane Signaling
  (`tests/spec.rs`); five of that specification's examples fail its own
  schemas, so the test checks they fail, fixes them the smallest way, and
  checks again.
- **Connector** (`crates/portitor-connector`): HTTP tests drive the real router
  in memory (`tower::ServiceExt::oneshot`), with fake adapters for what
  leaves the process. `tests/dsp_negotiation.rs`, for example, records the
  messages the provider would send instead of sending them;
  `tests/dsp_consumer.rs` does the same for the consumer side, with a fake
  provider, and checks every message against the DSP JSON Schemas.
  `tests/dsp_transfer.rs` does the same for transfers as provider, with a
  fake data plane and a fixed agreement; its last tests use the built-in
  data plane and fetch data from an upstream served on a local port (no
  network needed). `tests/end_to_end.rs` runs two connectors in one
  process, joined by a fake network: the consumer negotiates, requests a
  transfer, fetches the data from the provider's built-in data plane and
  completes the transfer. `tests/credential_constraints.rs` does the same
  with an offer that needs a credential claim (ADR 0019) and fake DCP
  ports that stand for the credential services: a holder without the
  claim is terminated, one with it gets an agreement and a transfer, and
  once its credential is revoked a new transfer does not start; a
  credential service that does not answer ends the negotiation, and an
  offer from the provider with a credential constraint is not accepted.
  Its last test gives the provider the real DCP verifier, against a
  consumer identity and an issuer served on a local port: once the
  issuer sets the membership credential's bit in its status list, a new
  negotiation is refused, a reissued credential held next to the
  revoked one is still refused (DCP 5.4.3, step 8), and once the revoked
  one is deleted the consumer is admitted again. The
  SQLite store is tested on an in-memory database and on a real file.
  Gaia-X compliance (ADR 0022): `src/auth/dcp/tests.rs` admits a caller
  with a compliance credential and its linked `gx:LegalPerson`, reads the
  label for a policy, and refuses a tampered or someone else's legal
  person credential, an untrusted or forged compliance service and an
  expired compliance credential.
- **Dataset harvest** (connector, ADR 0020): `src/harvest/tests.rs`
  runs harvests with fixed sources and the in-memory store: add, update,
  unchanged snapshot, a rejected entry that keeps the previous good
  version, a duplicate `@id`, withdrawal and return, a source that
  fails, and the check again when the rules change (hide, show again).
  `src/store/harvest.rs` tests the SQLite adapter (one transaction per
  run, 100 jobs kept). `tests/end_to_end.rs`,
  `a_withdrawn_dataset_keeps_its_transfers`: a dataset removed from the
  file leaves the catalog and refuses new contract requests, while a
  transfer under the agreement already made still gets its data.
  `tests/management_api.rs` tests `GET /v1/harvest-jobs` (pages, newest
  first) and one job with its errors.
- **Bundle** (ADR 0005): `crates/portitor-dcp/tests/bundle_manifest_schema.rs`
  checks examples and broken manifests against both
  `docs/schemas/bundle-manifest.schema.json` and the Rust type.
  `crates/portitor-issuer/tests/bundle.rs` builds and signs a bundle
  (`bundle build`), refuses RDF/XML and paths that leave the folder,
  serves built bundles, and lists the bundle key in the DID document.
  `crates/portitor-connector/tests/bundle_source.rs` takes the bundle an
  issuer on a local port serves: a newer version is taken without a
  restart, an older one never, a new version that fails a check leaves
  the rules in use, a fixed version never changes, the cache is used
  when the authority is down, an error without a cache, a key the DID
  document does not list or another authority or version is refused,
  and a folder is checked like a download. `tests/bundle_cache.rs`
  covers when the cache may replace a download.
- **SHACL engine** (`crates/portitor-shacl`): `tests/w3c.rs` runs the
  W3C SHACL core test cases for the parts the engine implements (copied
  unchanged into `tests/w3c`), comparing every result;
  `tests/refusal.rs` checks that each SHACL term the engine does not
  implement is refused, naming the term and the shape;
  `tests/engine.rs` covers constraints, paths, targets and JSON-LD to
  RDF; `tests/healthdcat.rs` loads the HealthDCAT-AP release 8 and
  DCAT-AP 3.0.0 shapes for each access level, checks valid and invalid
  samples (Turtle and DSP JSON-LD), and compares with the EU ITB
  validator's results on the 0.6 spike samples. Run one with
  `cargo test -p portitor-shacl --test healthdcat`.
- **SHACL in the connector** (dataset check and message guard):
  `src/shacl/tests.rs` builds small bundles in process: a dataset that
  passes states the profile, a violation rejects it with the report,
  each access level has its own shapes, a dataset without access rights,
  with unknown ones or with a relative `@id` is rejected, entries are
  read with the DSP context only (a bundle context file does not change
  that), a context file for a DSP URL is not used, and the guard checks
  messages against the message shapes and refuses unknown context URLs.
  `tests/shacl_checks.rs` runs them through HTTP with the HealthDCAT-AP
  release 8 shapes (full-IRI datasets): valid datasets served with
  `dct:conformsTo` under the plain DSP `@context`, invalid ones in the
  harvest job with their SHACL report, an inbound message that breaks the
  message shapes answered 400, an outbound one not sent, and a new bundle
  version that hides a failing dataset while its transfers keep working.
  `src/outbox.rs` checks that a message the rules refuse is logged as a
  rule breach, not as an unreachable counter-party. Run them with
  `cargo test -p portitor-connector --lib shacl`,
  `cargo test -p portitor-connector --test shacl_checks` and
  `cargo test -p portitor-connector --lib outbox`.
- `tests/dsp_catalog.rs` also checks that every path in
  [docs/api/openapi.json](api/openapi.json) is served.
- `tests/management_api.rs` tests the management API and checks that
  [docs/api/management-openapi.json](api/management-openapi.json) is the
  description the code generates. After changing the API, regenerate it:
  `PORTITOR_UPDATE_OPENAPI=1 cargo test -p portitor-connector --test management_api`.
- **DCP crate** (`crates/portitor-dcp`): `tests/spec.rs` parses the DCP
  v1.0.1 examples into the Portitor types and checks what Portitor writes
  back against the official JSON Schemas; `tests/gaia_x_ps256.rs` checks
  PS256 signatures as Gaia-X uses them. Unit tests next to the code cover
  the Gaia-X credentials and the check of the compliance service's answer
  (`gaiax_sign.rs`), the digests (`gaiax.rs`, RFC 8785 examples) and the
  certificate reader (`x509.rs`, with `test-only-*` certificates in
  `tests/fixtures/`).
- **Identity** (`crates/portitor-identity`), the same in-memory router
  tests: `tests/credential_service.rs` (DID document, STS, presentation
  API, Storage API and offers; every path in
  [docs/api/identity-openapi.json](api/identity-openapi.json) is served),
  `tests/issuance.rs` (the holder side of issuance against a fake issuer),
  `tests/gaiax.rs` (Gaia-X onboarding against a fake compliance service
  over HTTP: the presentation it gets, the stored and served credentials,
  a wrong digest refused, the runs in the management API, a certificate
  chain for another key or a bad participant file stopping the start),
  and `tests/management_api.rs` (API key, credential requests and
  credentials; also checks
  [identity-management-openapi.json](api/identity-management-openapi.json),
  regenerated the same way as the connector's).
- **Issuer** (`crates/portitor-issuer`): `tests/issuer.rs` runs the issuer
  end to end with fake neighbours (DID documents from memory, what it sends
  to holders recorded): requests, decisions, delivery, offers, revocation
  and status lists, and the paths of
  [issuer-openapi.json](api/issuer-openapi.json). Regenerate
  [issuer-management-openapi.json](api/issuer-management-openapi.json)
  with `cargo test -p portitor-issuer --test issuer -- --ignored write_management_description`.
- **Runtime and time** (`crates/portitor-runtime`, `crates/portitor-time`):
  unit tests next to the code (`cargo test -p portitor-runtime --lib`).
- **Health checks** (ADR 0023): `crates/portitor-runtime/src/health.rs`
  tests the `HealthStatus` answers, the time limit per check and the
  `health` subcommand. `crates/portitor-connector/tests/health.rs` and
  `crates/portitor-identity/tests/health.rs` check that the routes are
  open without the API key, that readiness follows the component's own
  dependencies (a removed SQLite file makes it not ready), and that the
  management API keeps its key; for the issuer it is
  `health_checks_its_own_dependencies_without_the_key` in
  `crates/portitor-issuer/tests/issuer.rs`.
- **Telemetry export** (ADR 0023):
  `crates/portitor-runtime/tests/telemetry_export.rs` runs a child
  process against a fake OTLP/HTTP receiver: nothing is sent and log
  lines are unchanged without an endpoint; with one, server spans carry
  the route template and never the path, client spans and the STS call
  share the trace, no trace context crosses a DSP-like call, and no
  secret or `Authorization` value appears in any export.
  `cargo test -p portitor-runtime --test telemetry_export`.
- **Request size limits** (R-18): `crates/portitor-runtime/src/request_size.rs`
  tests the settings and their bounds, and the lower of the port's and
  the route's limit. `tests/request_size.rs` in the connector and the
  identity, and `request_bodies_over_the_limits_get_413` in the issuer's
  `tests/issuer.rs`, send each limited route a body at its limit and one
  byte over, with and without `Content-Length`.
- **Status list cache** (ADR 0016, amended): the unit tests in
  `crates/portitor-connector/src/auth/dcp/status_list_cache.rs`, and the
  verifier tests in `src/auth/dcp/tests.rs` (at 0 and 300 seconds, a
  list that ends sooner, a failed fetch, a revocation seen once the list
  is stale). `cargo test -p portitor-connector --lib auth::dcp`.
- **DID document cache** (ADR 0016, amended in 0.12): the unit tests in
  `crates/portitor-runtime/src/did.rs` (at 0, 300 and 3600 seconds, the
  bounds of `PORTITOR_DID_CACHE_SECONDS`),
  `the_did_cache_setting_is_bounded` in the connector's `src/auth.rs`
  and the verifier tests in `src/auth/dcp/tests.rs` (a removed key is
  refused at once only at 0).
  `cargo test -p portitor-runtime --lib did`.
- **Issuer metadata and offers against the DCP schemas** (TD-33):
  `the_metadata_answer_passes_the_dcp_schema`,
  `the_definitions_files_of_the_repository_are_valid` and the offer
  tests in the issuer's `tests/issuer.rs` check what the issuer itself
  answers and sends, with the schema copies in
  `crates/portitor-dcp/tests/schemas/`.
- **Strict policy reader** (TD-32):
  `crates/portitor-dsp/src/policy/strict_tests.rs` reads offers and
  agreements with an unknown field, action, operand or operator at each
  level, a field given twice, `null`, one object in place of a list and
  other values of `@type`. `src/harvest/tests.rs` in the connector checks
  that only the dataset with such an offer is left out; the unknown-term
  tests in `tests/dsp_negotiation.rs` and `tests/dsp_consumer.rs` check
  the 400 answer that names the term.
  `cargo test -p portitor-dsp --lib policy`.
- **Repeated transfer request** (TD-34):
  `crates/portitor-dsp/src/transfer/provider/tests.rs` (the same request
  again, one that differs in agreement, format or callback, another
  caller, two copies at once, a repeat after the transfer ended), the
  `repeated_request` tests in the connector's `tests/dsp_transfer.rs`
  (one transfer and one token, also in the SQLite store) and the
  migration tests in `src/store.rs`.
- **Refused callers and DSP details** (TD-36): `every_dsp_route_refuses_a_caller_without_a_token`
  and `every_dsp_route_answers_503_while_the_callers_check_cannot_finish`
  in the connector's `tests/dsp_transfer.rs` go through the list
  `DSP_ROUTES` (404 on negotiation and transfer routes and callbacks,
  401 on the catalog). A new DSP route must be added to that list. The
  agreement timestamp is tested in
  `crates/portitor-dsp/src/negotiation/consumer/tests.rs` and
  `tests/dsp_consumer.rs`, the `consumerPid` of a counter-request in
  `tests/dsp_negotiation.rs`.
- **Reading a provider's catalog** (TD-38): the connector's
  `tests/management_catalog.rs`. A Portitor provider in the same process
  answers the two management calls (the answers pass the DSP JSON
  Schemas), and a small HTTP server plays a provider that refuses (401,
  404, 503), cannot be reached, sends pages (`Link` headers), more than
  the limit, or something that is not a catalog. Also: every refused
  request body, `additionalScopes` in the token, an `http://` address
  without `PORTITOR_ALLOW_HTTP`, and HTTP 403 documented on
  every protected route of the OpenAPI description (ADR 0025).
  `cargo test -p portitor-connector --test management_catalog`.
- **Keys of another DID** (DCP 5.4.3): `crates/portitor-dcp/src/credential/tests.rs`
  and `a_credential_signed_with_another_dids_key_refuses_the_caller` in
  the connector's `src/auth/dcp/tests.rs`; for the holder side,
  `an_answer_signed_with_another_dids_key_is_refused_and_nothing_is_stored`
  in the identity's `tests/gaiax.rs`.
- **Panics** (TD-41): `crates/portitor-runtime/src/panics.rs` (a 500
  answer as `application/problem+json`, other requests go on, supervised
  tasks start again after the pause) and `src/serve.rs` (both servers
  over real sockets). `panics_are_counted_and_logged_without_their_message`
  in `tests/telemetry_export.rs` checks the two metrics. The background
  tasks have their own tests: `src/health.rs`, `src/harvest/tests.rs`
  and the outbox test in `tests/dsp_negotiation.rs` in the connector,
  `a_run_that_panics_does_not_block_the_next_run` in the identity's
  `tests/gaiax.rs`. A store keeps working after a panic that held its
  lock: `src/store/poison_tests.rs` in the connector, the identity and
  the issuer.
- **Database migrations**: `crates/portitor-connector/src/store.rs` opens
  a file as 0.11 wrote it, with data, and checks the indexes of 0.12
  (migrations 9 to 11) and that the lookups use them.
- **Settings that stop the start**: `tests/health.rs` in the identity and
  `tests/issuer.rs` in the issuer run the binary with a
  `PORTITOR_DID_CACHE_SECONDS` out of bounds; the issuer also with a
  definitions file without a valid `credentialSchema`.
- **Tests marked `#[ignore = "bug: ..."]`** describe a behaviour the code
  does not have yet; the reason names it. Remove the mark with the fix.
  `grep -rn 'ignore = "bug' crates` lists them.
- **Retired keys** (TD-05): `crates/portitor-identity/tests/key_rotation.rs`,
  and `a_key_change_with_the_old_key_retired_leaves_no_gap` in the
  issuer's `tests/issuer.rs`: what the old key signed verifies while it
  is listed, and stops once it is removed.
- **Outbox retries** (ADR 0007, amended): `src/outbox.rs` in the
  connector checks which failures are tried again and that any other
  answer ends the tries. `cargo test -p portitor-connector --lib outbox`.

Rules for new tests:

- A protocol behaviour cites the specification section it tests.
- No network access and no fixed ports in unit tests; bind to port 0 if a
  test needs a server.
- Test keys only in `crates/*/tests/fixtures/test-only-*`. Never a real
  secret.
- `unwrap()` is allowed in tests only; add
  `#![allow(clippy::unwrap_used)]` to the test file or module.

## Coverage

```sh
cargo install cargo-llvm-cov
cargo llvm-cov --workspace --summary-only
```

New code should keep line coverage at 80 % or more. Coverage counts only
the default build, so code behind the `tck` feature is tested by the
command above but not counted.

## DSP conformance (TCK)

The Eclipse DSP Technology Compatibility Kit (TCK) acts as the other
connector and checks that Portitor follows DSP 2025-1. The `TCK` workflow
builds it from source at a pinned commit. Required today: metadata (MET),
catalog (CAT), contract negotiation as provider (CN:01 to CN:03) and as
consumer (CN_C:01 to CN_C:03), and the transfer process as provider
(TP:01 to TP:03) and as consumer (TP_C:01 to TP_C:03). The TCK itself
disables TP:02-04 and TP_C:02-04.

In remote mode the TCK expects the connector to act differently per
dataset or agreement (for example, terminate instead of agree), and starts
each consumer test with a POST to `/tck/negotiations` or `/tck/transfers`.
The TCK build also holds the agreements of the provider transfer tests
and sends transfer start messages 300 ms late, to avoid a race in the TCK
(ADR 0014). Portitor does both only
when built with the test-only `tck` feature (`src/tck.rs`); release builds
never contain it. CI also runs `cargo test` and `cargo clippy` with that
feature.

To run it locally:

```sh
# 1. Get the TCK: build it (see the workflow), or download the released jar
#    and check it against the .sha1 file next to it on Maven Central
curl -LO https://repo1.maven.org/maven2/org/eclipse/dataspacetck/dsp/dsp-tck/1.0.2/dsp-tck-1.0.2-all.jar
curl -L https://repo1.maven.org/maven2/org/eclipse/dataspacetck/dsp/dsp-tck/1.0.2/dsp-tck-1.0.2-all.jar.sha1; echo
sha1sum dsp-tck-1.0.2-all.jar

# 2. Start Portitor with the TCK datasets; the tck build takes the TCK's
#    fixed header instead of DCP (PORTITOR_TCK_AUTHORIZATION)
cargo build -p portitor-connector --features tck
PORTITOR_BIND=127.0.0.1:8080 PORTITOR_PUBLIC_URL=http://127.0.0.1:8080 \
PORTITOR_PARTICIPANT_ID='did:web:localhost%3A8080' PORTITOR_TCK_AUTHORIZATION='Bearer dsp-tck-only' \
PORTITOR_DATASETS_FILE=tck/datasets.json PORTITOR_ALLOW_HTTP=true \
PORTITOR_DB_FILE=/tmp/portitor-tck.db target/debug/portitor-connector &

# 3. Run it and check the required tests
java -jar dsp-tck-1.0.2-all.jar -config tck/dsp-tck.properties > tck.log
python3 scripts/tck_check.py tck.log
```

The TCK runs without a bundle. Its datasets (`tck/datasets.json`) have
relative ids and no HealthDCAT-AP metadata, so a bundle with dataset
shapes would reject every one of them and the catalog tests could not
run; that is the expected behaviour, not a conformance result. A bundle
with message shapes only would need an authority (an issuer with a DID
document and a bundle key) in the TCK job, and would show only that the
guard lets valid TCK messages through; the guard itself is tested in
`tests/shacl_checks.rs`, and the demos and the EDC interop test run DSP
flows with a bundle on.

To add a TCK group: add its datasets to `tck/datasets.json`, its
properties to `tck/dsp-tck.properties`, scripted behaviour (if any) to
`src/tck.rs`, and its test ids to `REQUIRED` in `scripts/tck_check.py`.

## DCP conformance (TCK)

The Eclipse DCP TCK v1.2.1 checks the identity component
(`portitor-identity`) as a credential service, the connector as a
verifier, and the issuer (below). For the identity component it runs the
presentation tests (DCP sections 4 and 5) and the holder side of issuance (Storage API and
credential offers, section 6), 54 tests. The TCK plays the verifier and
the issuer, serves their DID documents itself, and gets its ID tokens from
the identity component's STS. Before each test it delivers two test
credentials through the Storage API, so the run needs the TCK's issuer in
`PORTITOR_TRUSTED_ISSUERS` and a credential request with the TCK's
correlation id (`dataspacetck.credentials.correlation.id` in
`tck/dcp-tck.properties`). The three presentation definition tests are
disabled in the TCK itself. The `TCK` workflow downloads the released TCK
jar, checks its SHA-256, and runs it against the normal build of
`portitor-identity` (building the TCK from source needs a Java 17
toolchain as well as 21).

To run it locally:

```sh
# 1. Get the TCK and check it against the .sha1 file next to it
curl -LO https://repo1.maven.org/maven2/org/eclipse/dataspacetck/dcp/dcp-tck/1.2.1/dcp-tck-1.2.1-all.jar
curl -L https://repo1.maven.org/maven2/org/eclipse/dataspacetck/dcp/dcp-tck/1.2.1/dcp-tck-1.2.1-all.jar.sha1; echo
sha1sum dcp-tck-1.2.1-all.jar

# 2. Start the identity component with the TCK's issuer trusted
cargo build -p portitor-identity
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out /tmp/id-key.pem
openssl rand -hex 32 > /tmp/id-sts.secret
openssl rand -hex 32 > /tmp/id-api.key
PORTITOR_PARTICIPANT_ID='did:web:localhost%3A8090' PORTITOR_BIND=127.0.0.1:8090 \
PORTITOR_CREDENTIAL_SERVICE_URL=http://localhost:8090 PORTITOR_IDENTITY_KEY_FILE=/tmp/id-key.pem \
PORTITOR_STS_CLIENT_SECRET_FILE=/tmp/id-sts.secret PORTITOR_MANAGEMENT_API_KEY_FILE=/tmp/id-api.key \
PORTITOR_TRUSTED_ISSUERS='did:web:127.0.0.1%3A8083:issuer' PORTITOR_ALLOW_HTTP=true \
PORTITOR_DB_FILE=/tmp/portitor-identity-tck.db target/debug/portitor-identity &

# 3. Record the requests the test credentials answer: one per correlation id
#    in the properties file, as a delivery closes its request
for pid in $(sed -n 's/^dataspacetck\.credentials\.correlation\.id[^=]*=//p' tck/dcp-tck.properties); do
  curl -sf -X POST localhost:8091/v1/credential-requests -H "Authorization: Bearer $(cat /tmp/id-api.key)" \
    -H 'Content-Type: application/json' \
    -d "{\"issuerDid\": \"did:web:127.0.0.1%3A8083:issuer\", \"holderPid\": \"$pid\"}"
done

# 4. Run it and check that all 54 tests passed
DATASPACETCK_STS_CLIENT_SECRET=$(cat /tmp/id-sts.secret) \
  java -jar dcp-tck-1.2.1-all.jar -config tck/dcp-tck.properties > dcp-tck.log
python3 scripts/dcp_tck_check.py dcp-tck.log
```

### The verifier tests

The same TCK checks the connector as a verifier (package
`verification.presentation.verifier`, 22 tests): it plays the prover and
the issuer, serves their DIDs itself, and starts each test with a catalog
request carrying the prover's ID token; it expects 2xx when the caller
should be admitted and 4xx otherwise. The connector's own identity is a
`portitor-identity`: its STS signs the connector's presentation queries,
which the TCK checks against that identity's DID document. The run uses
`tck/dcp-verifier-tck.properties` and the normal connector build:

```sh
# the TCK jar as above; the identity component of step 2 on port 8090
# (management API not needed), then the connector
cargo build -p portitor-connector
PORTITOR_PARTICIPANT_ID='did:web:localhost%3A8090' PORTITOR_TRUSTED_ISSUERS='did:web:127.0.0.1%3A8083:issuer' \
PORTITOR_ALLOW_HTTP=true PORTITOR_BIND=127.0.0.1:8080 PORTITOR_PUBLIC_URL=http://127.0.0.1:8080 \
PORTITOR_DATASETS_FILE=tck/datasets.json PORTITOR_DB_FILE=/tmp/portitor-verifier-tck.db \
PORTITOR_STS_URL=http://localhost:8090/sts/token PORTITOR_STS_CLIENT_SECRET_FILE=/tmp/id-sts.secret \
  target/debug/portitor-connector &
java -jar dcp-tck-1.2.1-all.jar -config tck/dcp-verifier-tck.properties > dcp-verifier-tck.log
python3 scripts/dcp_tck_check.py dcp-verifier-tck.log 22
```

### The issuer tests

The same TCK checks `portitor-issuer` with its `issuance.issuer` package,
41 tests: endpoint discovery, issuer metadata, credential requests and
their status (with every token check of DCP 4.3), and what is delivered
to the holder (one `CredentialMessage` per request, signed credentials,
the holder's access token sent back). The TCK plays the holder
(`did:web:127.0.0.1%3A8083:holder`) and receives the deliveries itself,
so its DID is registered as a participant before the run (the issuer
answers 401 to any other DID, which the TCK's "other holder" and
"unresolvable subject" tests accept as the 4xx they expect). The TCK
holder gets about twenty requests issued, so the script raises
`PORTITOR_ISSUER_MAX_ACTIVE_PER_HOLDER` to 100. The issuer
refuses a repeated `holderPid`, so `tck/dcp-issuer-tck.properties` gives
each test that must be accepted its own. `tck/issuer-credentials.json`
defines two `vc11-sl2021/jwt` credentials, as the TCK's test 6.4.1
expects. One script does it all, locally and in CI:

```sh
# the TCK jar as above; uses ports 8070, 8071 and 8083 (ISSUER_PORT,
# MANAGEMENT_PORT and TCK_PORT change them), and writes
# dcp-issuer-tck.log and issuer.log in the current directory
scripts/dcp-issuer-tck.sh dcp-tck-1.2.1-all.jar
# the same issuer as a bundle authority: it serves the demo bundle and its
# DID document also lists <did>#bundle-key; all 41 tests must pass too
BUNDLE_KEY=1 scripts/dcp-issuer-tck.sh dcp-tck-1.2.1-all.jar
```

CI runs both.

## EDC interop test

`scripts/edc-interop.sh` runs a small dataspace in Docker Compose
(`interop/edc/compose.yaml`): a Portitor participant (`portitor-connector`
and `portitor-identity`, `did:web:portitor.gaiax.test`) and an Eclipse
Dataspace Components participant (control plane, data plane, and
IdentityHub with its STS, all v1.0.0-RC8, `did:web:edc.gaiax.test`), both
members by a `MembershipCredential` from the dataspace's
`portitor-issuer`. From 0.13 it also has the local Gaia-X compliance
service of the [Gaia-X end-to-end test](#gaia-x-end-to-end-test)
(gx-compliance v2.16.0 with Memgraph 3.9.0, a test notary and a test CA).
From 0.14 a fourth EDC runtime, the Issuer Service of EDC IdentityHub,
is a second issuer
([A second issuer](#a-second-issuer-the-edc-issuer-service)).

One command runs it all, and it exits non-zero at the first failed
check. The steps, numbered as in the script's header comment, and what
each must show (steps 1 and 2 make the keys and start Compose):

| # | Step | Expected result |
|---|---|---|
| 3 | Each identity asks the issuer for its credential through its own API, and the issuer delivers it (DCP 6.4, 6.5): the IdentityHub through its Identity API (`POST .../participants/{id}/credentials/request`, in the package `v1.unstable` in RC8), Portitor's identity through its management API (`POST /v1/credential-requests`). The issuer registered both first | both requests reach `ISSUED`; each identity holds one `MembershipCredential`, the IdentityHub's from `did:web:issuer%3A8070` |
| 4 | EDC accepts Portitor's identity: the EDC provider's catalog, read through the Portitor connector's management API (`POST /v1/catalog/request`; EDC queries Portitor's credential service) | HTTP 200, and the catalog has an offer for `edc-interop` |
| 5 | Portitor consumer, EDC provider: negotiation, pull transfer, fetch from the EDC data plane | negotiation `FINALIZED`, transfer `STARTED`, the bytes equal `data/edc-provider.csv` |
| 6 | EDC consumer, Portitor provider: catalog, negotiation, pull transfer (EDC's data plane fetches from Portitor's). The issuer serves the HealthDCAT-AP demo bundle and the Portitor connector follows it, so its datasets (`config/portitor-datasets.json`) are checked against the shapes | EDC read `dct:conformsTo` and the HealthDCAT-AP metadata as full IRIs; negotiation `FINALIZED`, transfer `STARTED`, the bytes equal `data/portitor-provider.csv` |
| 7a | Gaia-X, Portitor: its identity did its onboarding by itself (ADR 0021) | its DID document, read over HTTPS, has `alg` `ES256` and `x5u`; the newest compliance run is `SUCCEEDED`; it holds a `gx:ParticipantComplianceCredential` from `did:web:compliance.gaiax.test` |
| 7b | Gaia-X, the offer: the Portitor provider's second dataset (`.../portitor-interop-gaiax`) is offered only with `ParticipantComplianceCredential.gx:labelLevel` `eq` `SC` (ADR 0022) | the offer in the catalog EDC read still has that left operand and `SC` |
| 7b | Gaia-X, refused: the EDC consumer, which holds no compliance credential, negotiates that offer | the EDC negotiation ends `TERMINATED`, and its `errorDetail` has no `code=`: Portitor took the request (the caller is a member) and then sent a `ContractNegotiationTerminationMessage`, not an HTTP error |
| 7c | Gaia-X, the EDC participant's onboarding: `scripts/gaiax_test_onboarding.py` signs its `gx:LegalPerson` and `gx:Issuer` credentials and the presentation with the participant's key; the script sends it to the compliance service | the service answers with a VC-JWT whose `id` is the one asked for, issuer `did:web:compliance.gaiax.test`, type `gx:ParticipantComplianceCredential`, `gx:labelLevel` `SC` |
| 7c | Gaia-X, the IdentityHub stores the compliance credential and the legal person credential (Identity API, `POST .../participants/{id}/credentials`) | it lists exactly one credential of each type |
| 7c | Gaia-X, admitted: EDC negotiates the same offer again and pulls the data | negotiation `FINALIZED`, transfer `STARTED`, the bytes equal `data/portitor-provider-gaiax.csv` |
| 8 | A second issuer, the EDC Issuer Service: steps 8a to 8g, [below](#a-second-issuer-the-edc-issuer-service) | see there |
| 9 | Revocation both ways (step 8 before 0.14): `portitor-issuer` revokes one participant's `MembershipCredential` (`POST /v1/credentials/{id}/revoke`), the other refuses it | EDC answers Portitor's catalog request with 401 (the management call then answers 502 and names the 401); EDC's negotiation with Portitor ends `TERMINATED` with `code=404` in its `errorDetail` (DSP 2025-1 HTTPS bindings, "Unauthorized Access"; from 0.12, first seen in CI on 2026-11-20 against RC7) |

EDC always sends a `filter` in its `CatalogRequestMessage`: an EDC
`QuerySpec` with no filter expressions (offset 0, limit 50). Portitor
treats such a filter as "no filter" and answers with the whole catalog.

```sh
# first build the Portitor images ("Building the images locally" above);
# the EDC images are built from interop/edc/runtimes (Maven Central, no
# EDC images are published); needs Docker or Podman, curl, jq, openssl
# and python3 with `cryptography`
scripts/edc-interop.sh            # KEEP=1 leaves it running; --no-build reuses the EDC images
```

It ends with `== Done`. With `LOGS=FILE` a failed run writes every
container's whole log to that file (CI keeps it as the artifact
`edc-interop-logs`). It needs Internet access, as the Gaia-X test does:
Maven Central for the EDC build, and the Gaia-X Registry, `w3id.org` and
`www.w3.org` for the compliance service.

### What is pinned

Every image and every library is pinned, so a later run uses the same
software:

| What | Version | Pinned by |
|---|---|---|
| EDC Connector and IdentityHub (with its Issuer Service) | v1.0.0-RC8 | `edcVersion` in `interop/edc/runtimes/gradle.properties`; the SHA-256 of every plugin and library in `gradle/verification-metadata.xml` |
| Eclipse Data Plane SDK (the test's data plane) | 1.2.1 | `dataplaneSdkVersion`, and the same file |
| Build and runtime images of the EDC runtimes | `gradle:9.8.2-jdk21`, `eclipse-temurin:21.0.13_9-jre-noble` | digest, in `interop/edc/runtimes/Dockerfile` |
| Gaia-X compliance service | gx-compliance v2.16.0 | digest, in `interop/gaiax/compose.yaml` (the EDC test's Compose file takes the service from there with `extends`) |
| Its graph database | Memgraph 3.9.0 | digest, same file |
| Vault | `hashicorp/vault:2.2.1` | digest, in `interop/edc/compose.yaml` |
| Web servers (`tls`, `data`, `identity-api-jwks`) | `nginx:1.29-alpine` | digest, same file |
| `fetch` | `curlimages/curl:8.11.1` | digest, same file |
| Portitor | the commit under test | built from the checkout; a release is named with `PORTITOR_IMAGE`, `PORTITOR_IDENTITY_IMAGE` and `PORTITOR_ISSUER_IMAGE` |

Not pinned: what the compliance service loads from the Gaia-X Registry at
run time (shapes, ontology, contexts), hence the nightly run.

### Last run that passed

| Date | Portitor | EDC | Compliance service | Where |
|---|---|---|---|---|
| 2026-11-20 | 0.13.0, release branch commit `4c8c6d0` (pull request 72, before the tag) | v1.0.0-RC8 (Connector, IdentityHub) | gx-compliance v2.16.0 | GitHub Actions with Docker, [run 44734964661](https://github.com/brunopacheco1/portitor/actions/runs/44734964661) |
| not run yet | 0.14, with step 8 (EDC Issuer Service) | v1.0.0-RC8 (Connector, IdentityHub, Issuer Service) | gx-compliance v2.16.0 | not run yet |

That run was before step 8 (the EDC Issuer Service, 0.14) existed.
Step 8 in Compose: not run yet. It was run on localhost only, against
the real EDC jars and Portitor binaries, without Vault, TLS, the bundle
and the data plane: every check passed except the two data pulls.

Before 0.13 (without the Gaia-X steps): the nightly run of 2026-11-20 on
`main` (after 0.12.0), EDC v1.0.0-RC7, workflow `EDC interop` run 55.

### What EDC needs

Why the test is set up as it is (all read from the EDC v1.0.0-RC8
sources, Connector and IdentityHub):

- **Scope.** EDC IdentityHub accepts only
  `org.eclipse.dspace.dcp.vc.type:<type>:<read|all|*>` (with the
  operation), so the control plane asks for
  `org.eclipse.dspace.dcp.vc.type:MembershipCredential:read`
  (`edc.iam.dcp.scopes.*` in `config/edc-controlplane.properties`) and
  Portitor's `PORTITOR_DCP_SCOPES` is the same string. Reported upstream:
  https://github.com/eclipse-edc/IdentityHub/issues/1278
- **Trust.** `edc.iam.trustedissuer.authority.id=did:web:issuer%3A8070`
  (the issuer), and `PORTITOR_TRUSTED_ISSUERS` on both Portitor
  components.
- **Credential request.** The IdentityHub sends the
  `CredentialRequestMessage` with a token from its own STS, without a
  `token` claim, and accepts the issuer's delivery only for a request it
  sent to that issuer, with the type and profile it asked for
  (`MembershipCredential`, `vc20-bssl/jwt`). It finds the issuer service
  in the issuer's DID document (`IssuerService`).
- **Status lists.** EDC needs an `id` on every `credentialStatus`, and
  reads status lists again after one second here
  (`edc.iam.credential.revocation.cache.validity`), so the revocation
  check sees a new list at once. Portitor's connector fetches them on
  every check here (`PORTITOR_STATUS_LIST_CACHE_SECONDS=0`).
- **Data plane.** EDC 1.0 has no data plane of its own; the test's is
  built with the Eclipse Data Plane SDK 1.2.1 (`runtimes/dataplane`), and
  serves one file. Control plane and data plane register with each other
  without authorization (test-only).
- **Dependency verification.** Gradle checks the SHA-256 of every
  plugin and library it downloads against
  `interop/edc/runtimes/gradle/verification-metadata.xml` and stops on a
  mismatch or an unknown file. After changing a version in
  `gradle.properties` or the build files, write it again with the same
  Gradle as the Dockerfile and review the diff:

  ```sh
  docker run --rm -v "$PWD/interop/edc/runtimes:/src" -w /src \
    docker.io/library/gradle:9.8.2-jdk21 gradle --no-daemon --write-verification-metadata sha256 shadowJar
  ```
- **IdentityHub admin.** The Identity API (the IdentityHub's "oauth2"
  variant) takes a JWT checked against a key set. The script makes a
  test-only RSA key for each run, signs five-minute RS256 tokens with
  `openssl` (`identity_api_token` in `scripts/edc-interop.sh`), and
  serves the key set from the `identity-api-jwks` web server.
- **Public names.** The Gaia-X compliance service resolves `did:web`
  over HTTPS only, so both participants have a public name answered by
  the `tls` service (nginx, `config/tls.conf`) with a certificate from
  the run's test CA. EDC's Java does not trust that CA: the EDC runtimes
  resolve `did:web` over plain HTTP (`edc.iam.did.web.use.https=false`),
  on port 80 of the same names, and the service endpoints in both DID
  documents stay plain HTTP inside the network.

### Gaia-X with EDC: what the test shows, and what it does not

What it shows: an EDC participant can **hold** a Gaia-X compliance
credential in its IdentityHub and **present** it over DCP, and a Portitor
provider admits it by the checks of ADR 0022 (the credential comes from
the trusted compliance service, and the `gx:LegalPerson` credential it
names by digest is in the same presentation, issued and signed by the
caller). This uses only what EDC v1.0.0-RC8 has, set by configuration:

- **The key.** The Gaia-X compliance service needs `alg` and `x5u` (the
  key's certificate chain) in the signer's DID document. The IdentityHub
  takes a key that already exists: `publicKeyJwk` and `privateKeyAlias`
  in the `KeyDescriptor` of the participant manifest
  (`KeyPairServiceImpl`, IdentityHub), and publishes the JWK as given
  (`DidDocumentServiceImpl`). The script puts the private key in the
  Vault first, as a JWK.
- **The DID document.** The IdentityHub finds it by the `Host` header
  (`DidWebController`); the `tls` service passes it on. The certificate
  chain is a static file next to it.
- **Storing.** `POST /v1/participants/{id}/credentials` of the Identity
  API takes a credential that was issued elsewhere
  (`VerifiableCredentialsApiController`, format `VC2_0_JOSE`).
- **Scopes.** The control plane asks for the two Gaia-X types when a
  policy names `ParticipantComplianceCredential`
  (`edc.iam.dcp.scopes.gaiaxcompliance.*` and `gaiaxlegalperson.*`, type
  `POLICY`, `DynamicScopeExtractor`). The IdentityHub maps the scope's
  type to the credential's type `gx:...` with
  `edc.identityhub.discriminator.*`
  (`config/edc-identityhub.properties`).

What it does not show:

- **The EDC IdentityHub does not do the Gaia-X onboarding.** It has no
  code for it. `scripts/gaiax_test_onboarding.py` (test only) signs the
  `gx:LegalPerson` and `gx:Issuer` credentials and the presentation with
  the EDC participant's key, as an operator's tool would, in the form a
  Portitor identity writes (`crates/portitor-dcp/src/gaiax_sign.rs`).
- **EDC as the provider does not ask Portitor for Gaia-X compliance.**
  Not done, for two reasons read in the RC8 sources. EDC refuses a
  presentation with a credential whose subject is not the presenter's DID
  (`HasValidSubjectIds` in `VerifiableCredentialValidationServiceImpl`);
  the subject of a compliance credential is `<credential id>#cs`
  (ADR 0022). And the control plane the test builds
  (`controlplane-dcp-bom`) has no policy function that reads a
  credential's claims; EDC has one with CEL expressions
  (`decentralized-claims-cel`), but only in its `controlplane-virtual-*`
  BOMs.
- **No real Gaia-X trust anchor**, as in the Gaia-X test: the compliance
  service runs in its test mode, and the notary and the CA are made for
  the run.

### A second issuer: the EDC Issuer Service

Step 8 of `scripts/edc-interop.sh` adds a second issuer to the data
space: the Issuer Service of EDC IdentityHub v1.0.0-RC8
(`did:web:edc-issuer.gaiax.test`, the fourth runtime in
`interop/edc/runtimes`). It issues a plain test credential, the
`InteropTestCredential`, with one claim, `level`. The type is made up for
this test; it is not a standard. Both sides trust the issuer
(`PORTITOR_TRUSTED_ISSUERS`, `edc.iam.trustedissuer.edcissuer.id`), and
each offers data only with `InteropTestCredential.level` `eq` `basic`.

| # | Step | Expected result |
|---|---|---|
| 8a | The Issuer Service gets its participant context (Identity API) and, through its admin API, both participants as holders, an attestation and the credential definition | its DID document, over HTTPS and HTTP, names its key as `assertionMethod` and an `IssuerService`; its issuer metadata lists `interop-test-credential` |
| 8b | The offers: Portitor's third dataset (`.../portitor-interop-issuer`), and a second offer of the EDC asset | each catalog, as the other side read it, has the left operand `InteropTestCredential.level` |
| 8c | Refused: before anyone holds the credential, each consumer negotiates the other's offer | both negotiations end `TERMINATED` (one still open after 60 seconds fails the step); EDC has no `FINALIZED` negotiation more than before |
| 8d | Portitor's identity asks the Issuer Service (`POST /v1/credential-requests`); the EDC IdentityHub does the same (Identity API) | both requests `ISSUED`; each holds one `InteropTestCredential` from `did:web:edc-issuer.gaiax.test`; its status list bit is not set |
| 8e | EDC negotiates Portitor's offer and pulls | `FINALIZED`; the connector's log has a new DEBUG line `status list fetched and verified` for the Issuer Service's list (`RUST_LOG` in `interop/edc/compose.yaml`); `STARTED`, the bytes equal `data/portitor-provider-issuer.csv` |
| 8f | Portitor negotiates EDC's offer and pulls | `FINALIZED`, the agreement has the constraint, EDC has one `FINALIZED` negotiation more than before, `STARTED`, the bytes equal `data/edc-provider.csv` |
| 8g | The Issuer Service revokes the EDC participant's credential (`POST .../credentials/{id}/revoke`); EDC negotiates Portitor's offer again | the status list bit is set; the negotiation ends `TERMINATED` with no `code=`; and one of two: Portitor's log has a new `credential refused` line with the type and `revoked`, or (a warning, not a failure) the IdentityHub's record of the credential changed its `state`, so it presented none. With neither, the step fails |

Step 9 is the revocation by `portitor-issuer` (before 0.14: step 8).
The script's header comment uses the same letters.

What the test shows: `portitor-identity` gets a credential from an EDC
Issuer Service over DCP; the Portitor connector verifies a credential an
EDC issuer signed and reads its Bitstring Status List; EDC accepts that
credential when Portitor's identity presents it.

What it does not show, and what to know:

- **Request status (DCP 6.8).** EDC v1.0.0-RC8 answers a credential
  request with a `Location` that leaves out the path of its issuance API,
  so the status is not found there. `GET /v1/credential-requests/{holderPid}`
  then shows `issuerStatusError`. The credential arrives all the same. The
  script prints this as a known gap.
- **Key type.** An EDC participant context that makes its own key gets an
  Ed25519 key. Portitor verifies ES256 and PS256, so the test gives the
  Issuer Service a P-256 key.
- **Validity.** An EDC IdentityHub asks for a new credential every minute
  once the one it holds has less than seven days left. The test's
  credential is valid for a year.
- **Who notices the revocation.** The IdentityHub keeps status lists for
  15 minutes, so it still presents the revoked credential and Portitor
  refuses it. If the IdentityHub saw the revocation first, it would present
  nothing; the script says which happened.
- **The EDC provider's constraint** is checked by a test extension
  (`CredentialClaimPolicyExtension`). EDC has no such function of its own.
- Suspension is not tested: the admin API answers 501 for it in RC8.

By hand (`KEEP=1`): the Issuer Service's ports are in the table of
[The EDC interop run by hand](#the-edc-interop-run-by-hand).

### From RC7 to RC8

Read in the diff of the two tags (Connector and IdentityHub). Nothing the
test uses changed its path or its shape; the management API v4 and the
Identity API paths are the same. What changed near the test:

- the `controlplane-dcp-bom` no longer has the Linked Data proof modules
  (credentials as JWT only; the test used JWT already);
- a `credentialStatus` of an unknown type is now refused by default
  (`edc.iam.credential.revocation.unknowntypes.ignore`); Portitor's
  issuer writes `BitstringStatusListEntry`, which EDC knows;
- a status list that cannot be fetched is now a failure EDC tries again;
- an initial `ContractRequestMessage` sent twice with the same
  `consumerPid` gets the first negotiation back;
- the IdentityHub's changes are in the issuer service's holder API,
  which the test does not use.

### The EDC interop run by hand

`KEEP=1 scripts/edc-interop.sh` leaves everything running, with the keys
in `interop/edc/run/`. The run ends after the revocation step, so both
`MembershipCredential`s are revoked by then: reading works, a new
negotiation is refused. To try negotiations by hand, stop the script
before that step, or ask the issuer for new credentials (step 3 of the
table).

| What | URL on the host | Key |
|---|---|---|
| Portitor connector, DSP | `http://127.0.0.1:18080/dsp/2025-1` | a DCP token |
| Portitor connector, management API | `http://127.0.0.1:18081` | `interop/edc/run/portitor/connector-management.key` (Bearer) |
| Portitor identity (DID document, STS, credential service) | `http://127.0.0.1:18190` | STS client secret: `interop/edc/run/portitor/sts-client-secret` |
| Portitor identity, management API | `http://127.0.0.1:18191` | `interop/edc/run/portitor/identity-management.key` (Bearer) |
| Issuer, management API | `http://127.0.0.1:18371` | `interop/edc/run/issuer/management.key` (Bearer) |
| Issuer (DID document, issuer service, status lists, bundle) | `http://127.0.0.1:18372` | none |
| EDC control plane, management API | `http://127.0.0.1:18181/api/management` | header `x-api-key: edc-interop-test-key` |
| EDC IdentityHub, Identity API | `http://127.0.0.1:18281/api/identity/v1` | a JWT signed with `interop/edc/run/identity-api-key.pem` (below) |
| EDC data plane | `http://127.0.0.1:18180/api` | none |
| EDC Issuer Service: Identity API, DCP issuer service, admin API, status lists | `http://127.0.0.1:18381`, `:18382`, `:18385`, `:18386` | Identity API as the IdentityHub's |

The public names (`portitor.gaiax.test`, `edc.gaiax.test`,
`compliance.gaiax.test`, `notary.gaiax.test`) and the EDC DSP endpoint
(`http://edc-controlplane:8082/api/dsp/2025-1`) exist only inside the
Compose network; the `fetch` service reaches them and trusts the test CA.

```sh
COMPOSE="docker compose -f interop/edc/compose.yaml" # or podman compose
ID_KEY="authorization: Bearer $(cat interop/edc/run/portitor/identity-management.key)"
CONN_KEY="authorization: Bearer $(cat interop/edc/run/portitor/connector-management.key)"
EDC_KEY='x-api-key: edc-interop-test-key'
EDC_CONTEXT='"@context": ["https://w3id.org/edc/connector/management/v2"]'

# both DID documents, as the compliance service reads them
$COMPOSE run --rm -q fetch -sS https://portitor.gaiax.test/.well-known/did.json | jq
$COMPOSE run --rm -q fetch -sS https://edc.gaiax.test/.well-known/did.json | jq

# Portitor: the compliance runs and the credentials it holds
curl -s -H "$ID_KEY" http://127.0.0.1:18191/v1/gaiax/compliance | jq
curl -s -H "$ID_KEY" http://127.0.0.1:18191/v1/credentials | jq '.[] | {types, issuer}'

# Portitor: its negotiations (as consumer of the EDC offer)
curl -s -H "$CONN_KEY" http://127.0.0.1:18081/v1/negotiations | jq '.negotiations[] | {consumerPid, state}'

# EDC: the Portitor provider's catalog, with both datasets and their offers
curl -s -X POST http://127.0.0.1:18181/api/management/v4/catalog/request -H "$EDC_KEY" \
  -H 'content-type: application/json' -d "{$EDC_CONTEXT, \"@type\": \"CatalogRequest\",
  \"counterPartyAddress\": \"http://portitor-connector:8080/dsp/2025-1\",
  \"counterPartyId\": \"did:web:portitor.gaiax.test\", \"protocol\": \"dataspace-protocol-http:2025-1\"}" | jq

# EDC: its negotiations (one TERMINATED and one FINALIZED for the Gaia-X offer)
curl -s -X POST http://127.0.0.1:18181/api/management/v4/contractnegotiations/request -H "$EDC_KEY" \
  -H 'content-type: application/json' -d "{$EDC_CONTEXT, \"@type\": \"QuerySpec\"}" |
  jq '.[] | {id: .["@id"], state, errorDetail}'

# EDC IdentityHub: the credentials it holds. The Identity API takes a
# five-minute RS256 token signed with the run's key (identity_api_token
# in scripts/edc-interop.sh)
b64() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
NOW=$(date +%s)
INPUT="$(printf '{"alg":"RS256","typ":"JWT","kid":"edc-interop-admin-key"}' | b64).$(printf \
  '{"iss":"edc-interop-admin","sub":"admin","scope":"identity-api:admin","jti":"%s","iat":%s,"nbf":%s,"exp":%s}' \
  "$(openssl rand -hex 16)" "$NOW" "$NOW" "$((NOW + 300))" | b64)"
TOKEN="$INPUT.$(printf '%s' "$INPUT" | openssl dgst -sha256 -sign interop/edc/run/identity-api-key.pem | b64)"
curl -s -H "authorization: Bearer $TOKEN" http://127.0.0.1:18281/api/identity/v1/participants/edc/credentials |
  jq '.[] | {type: .verifiableCredential.credential.type, issuer: .issuerId}'

# the EDC participant's Gaia-X credentials, as the test wrote and received them
cut -d. -f2 interop/edc/run/edc/www/credentials/gaiax/compliance | base64 -d 2>/dev/null | jq
```

The [Bruno](https://www.usebruno.com/) collection in `_http/` has the
environment `edc-interop` (`_http/environments/edc-interop.bru`) with
these URLs for the Portitor participant and the issuer. Put
`interop/edc/run/portitor/connector-management.key` in the secret
`apiKey`, `interop/edc/run/portitor/sts-client-secret` in `stsSecret`,
`interop/edc/run/portitor/identity-management.key` in `identityApiKey`
and `interop/edc/run/issuer/management.key` in `issuerApiKey`. The
`identity`, `issuer` and `auth` folders and the read requests of
`management` fit this run. `management/Start negotiation`,
`management/Request catalog` and `management/Request dataset` do not:
they send `{{dsp}}` as the provider's address, which here is a URL on
the host that the connector's container cannot reach. The DSP folders call
the Portitor provider with a token from the Portitor participant's own
STS, so the participant calls itself; this was not tried. The collection
has no requests for EDC's APIs.

Clean up with `docker compose -f interop/edc/compose.yaml down --volumes`
and `rm -rf interop/edc/run/`.

## Gaia-X end-to-end test

`scripts/gaiax-e2e.sh` runs a small dataspace in Docker Compose
(`interop/gaiax/compose.yaml`) with a local Gaia-X compliance service:
gx-compliance v2.16.0 with Memgraph 3.9.0, both pinned by digest, in the
test mode Gaia-X documents for local testing
([developer-and-local-testing.md](https://gitlab.com/gaia-x/lab/compliance/gx-compliance/-/blob/development/docs/developer-and-local-testing.md)).
The participants are Portitor identities and connectors, members by a
`MembershipCredential` from a `portitor-issuer`:

- provider (`did:web:provider.gaiax.test`) and consumer
  (`did:web:consumer.gaiax.test`), both with Gaia-X on (ADR 0021): a
  certificate for the identity key (`PORTITOR_IDENTITY_CERTIFICATE_CHAIN_FILE`),
  a participant file (`PORTITOR_GAIAX_PARTICIPANT_FILE`) and a
  registration number credential (`gx:VatID`);
- an outsider (`did:web:outsider-identity%3A8090`), a member without
  Gaia-X.

For each run the script makes a test CA and, from it, the certificates of
the identity keys, of the compliance service (RSA, PS256) and of the
public names. An nginx service (`tls`) answers for the `*.gaiax.test`
names over HTTPS and HTTP: the compliance service resolves `did:web` over
HTTPS only and trusts the test CA through `NODE_EXTRA_CA_CERTS`; the
Portitor components trust it through `SSL_CERT_DIR` (as in
[DEPLOYING](DEPLOYING.md#private-cas)). The registration number
credentials are signed by `scripts/gaiax_test_notary.py`, a stand-in for
a Gaia-X notary with its own `did:web`.

It checks, in this order:

1. the compliance service's DID resolves over HTTPS; each Gaia-X
   identity's DID document has `alg: ES256` and an `x5u` that serves the
   chain it was given;
2. each identity asks the issuer for its `MembershipCredential`;
3. the consumer's identity runs its onboarding (`POST /v1/gaiax/compliance`)
   and then holds a `gx:ParticipantComplianceCredential` issued by
   `did:web:compliance.gaiax.test`;
4. the consumer negotiates the provider's offer, whose constraint is
   `ParticipantComplianceCredential.gx:labelLevel` `eq` `SC` (ADR 0022;
   the provider trusts the service through
   `PORTITOR_GAIAX_COMPLIANCE_ISSUERS`): `FINALIZED`;
5. the outsider negotiates the same offer: `TERMINATED`.

What it proves: a Portitor identity can build the presentation a Gaia-X
compliance service accepts, the service's answer is stored and presented
over DCP, and a Portitor provider checks it (issuer, signature, the link
to the caller's `gx:LegalPerson`) before it agrees.

What it does not prove:

- **No real Gaia-X trust anchor.** `DISABLE_SIGNATURE_CHECK=true`: the
  service does not check certificate chains against the Gaia-X Registry,
  so any certificate from the test CA is accepted. `production=false`: it
  accepts a registration number credential from any issuer, so the test
  notary's credentials prove nothing.
- Not the Gaia-X Lab (`compliance.lab.gaia-x.eu`) nor a GXDCH notary; a
  test against the Lab with a public `did:web` is future work
  ([spike 0.8](spikes/0.8-gaia-x-compliance.md)).
- Not EDC: this test has only Portitor participants. From 0.13 the
  [EDC interop test](#edc-interop-test) has the Gaia-X steps with EDC.

It needs Internet access: the compliance service loads the Gaia-X shapes,
ontology and JSON-LD contexts from `registry.lab.gaia-x.eu` (through
`w3id.org`) and the credentials context from `www.w3.org`, with Node.js
clients that ignore `HTTPS_PROXY`. A change there can break the test
without a change in Portitor, hence the nightly run.

```sh
# first build the Portitor images ("Building the images locally" above);
# needs Docker or Podman, curl, jq, openssl and python3 with `cryptography`
scripts/gaiax-e2e.sh            # KEEP=1 leaves it running, with interop/gaiax/run/
```

## How to test Gaia-X compliance

Three levels, from fast to complete. None needs a real Gaia-X service; a
run against the Gaia-X Lab with a public `did:web` is future work
([R-12](RISKS_AND_TECHNICAL_DEBTS.md#r-12)).

**1. Unit and integration tests** (seconds, no Docker):

```sh
cargo test -p portitor-dcp gaiax          # JCS digests (RFC 8785), Gaia-X credentials, answer check
cargo test -p portitor-identity --test gaiax   # onboarding against a fake compliance service
cargo test -p portitor-connector auth::dcp     # ADR 0022 checks: admitted, refused, skipped
cargo +nightly fuzz run gaiax_compliance -- -max_total_time=60   # see Fuzzing
```

**2. The end-to-end test** (about 5 minutes; Docker, Internet):
[build the Portitor images](#building-the-images-locally), then run
`scripts/gaiax-e2e.sh`. It ends with `== Done` when the compliant consumer
gets a contract (`FINALIZED`) and the outsider is refused (`TERMINATED`);
see [Gaia-X end-to-end test](#gaia-x-end-to-end-test) for what it sets up.

**3. By hand, on the running dataspace.** `KEEP=1 scripts/gaiax-e2e.sh`
leaves everything running, with the keys in `interop/gaiax/run/`. From
the repository root:

```sh
# the management API keys of the consumer's identity and connector
ID_KEY="authorization: Bearer $(cat interop/gaiax/run/consumer/identity-management.key)"
CONN_KEY="authorization: Bearer $(cat interop/gaiax/run/consumer/connector-management.key)"

# the onboarding runs (outcome, error with the service's reason) and a new one
curl -s -H "$ID_KEY" http://127.0.0.1:18492/v1/gaiax/compliance | jq
curl -s -X POST -H "$ID_KEY" http://127.0.0.1:18492/v1/gaiax/compliance

# the credentials the identity holds: membership and the four Gaia-X ones
curl -s -H "$ID_KEY" http://127.0.0.1:18492/v1/credentials | jq '.[] | {types, issuer}'

# what the compliance service and verifiers read: the key's certificate
# chain (named by x5u in the DID document) and the credentials sent
curl -s http://127.0.0.1:18490/.well-known/x509CertificateChain.pem | openssl x509 -noout -subject -enddate
curl -s http://127.0.0.1:18490/credentials/gaiax/legal-person | cut -d. -f2 | base64 -d 2>/dev/null | jq

# the consumer's negotiations (the script's are FINALIZED)
curl -s -H "$CONN_KEY" http://127.0.0.1:18482/v1/negotiations | jq '.negotiations[] | {consumerPid, state}'

# the outsider's negotiation of the same offer: TERMINATED, as it holds no
# compliance credential (the provider logs a refused credential only when
# one fails its checks: `credential refused`)
OUT_KEY="authorization: Bearer $(cat interop/gaiax/run/outsider/connector-management.key)"
curl -s -H "$OUT_KEY" http://127.0.0.1:18483/v1/negotiations | jq '.negotiations[] | {consumerPid, state}'
```

Things worth trying:

- **A failed run.** `docker compose -f interop/gaiax/compose.yaml stop compliance`,
  start a run, and read the runs: it fails, the credentials of the last
  good run stay, and it is retried with growing delays. Start the service
  again and run once more.
- **Another offer.** Edit `interop/gaiax/config/datasets.json` (for
  example `gx:labelLevel` `eq` `L1`) and restart `provider-connector`: the
  consumer's negotiation is now `TERMINATED`, as its label is `SC`.
- **Another compliance service.** Set the provider's
  `PORTITOR_GAIAX_COMPLIANCE_ISSUERS` to another DID and restart it: the
  compliance credential is skipped and the consumer is refused for this
  offer, but still admitted to offers that need only membership.

Clean up with `docker compose -f interop/gaiax/compose.yaml down -v` and
`rm -rf interop/gaiax/run/`.

## Fuzzing

Parsers and state machines are fuzzed with `cargo-fuzz` (libFuzzer). It
needs a nightly toolchain:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run dsp_negotiation -- -max_total_time=60
```

Some targets link the bundled SQLite (through `portitor-connector` and
`portitor-runtime`). Build them with Clang, as ClusterFuzzLite does: with
GCC as `cc`, AddressSanitizer stops them at start with an
`odr-violation` in `sqlite3.c`.

```sh
CC=clang CXX=clang++ cargo +nightly fuzz build -O --debug-assertions
```

Targets (`fuzz/fuzz_targets`): `dsp_messages`, `dsp_negotiation`, `dsp_transfer`,
`odrl_policy`, `dcp_token_parse`, `dcp_token_validate`, `dcp_messages`,
`dcp_status_list`, `dcp_credentials`, `dcp_access_token`, `dcp_did_document`,
`dcp_x509` (X.509 chains, PEM and DER), `bundle_manifest` (`latest.json`,
the bundle manifest and its signature), `bundle_rules` (the bundle's
files and the message guard, as the connector reads them), `shacl_load`
(Turtle shapes loaded by `portitor-shacl`), `shacl_jsonld` (JSON-LD to
RDF with local contexts), `shacl_datatypes` (lexical forms for
`sh:datatype`), `harvest_datasets` (the harvested datasets file),
`dps_messages` (Data Plane Signaling messages and states), `time_parse`
(`xsd:date` and `xsd:dateTime`), `http_inputs` (`Authorization`
headers and URLs other participants choose) and `gaiax_compliance` (the RFC 8785 JCS form and
`digestSRI` of any JSON, the Gaia-X compliance credential's list and the
answer check; the JCS form must not change when applied again). Every
parser of input from outside the process gets a target; the list of
parsers and targets is in
[SECURITY-ASSURANCE](SECURITY-ASSURANCE.md#parsers-and-fuzz-targets). In CI, ClusterFuzzLite fuzzes every pull request briefly
(`cflite_pr.yml`) and runs longer each week (`cflite_batch.yml`). A crash
is a bug: add the input that caused it as a regression test.

## CodeQL

The workflow `CodeQL` (`.github/workflows/codeql.yml`) analyses the Rust
code and the GitHub Actions workflows with GitHub's default queries, on
every pull request, on every push to `main` and once a week. Rust is
analysed without a build (`build-mode: none`).

The analysis itself succeeds whenever it ran, with or without findings.
So its last step runs `scripts/codeql-gate.sh <rust|actions> <folder with .sarif>`,
which fails the job on every result that is not in
`.github/codeql/accepted-findings.json`, whatever its severity. The list
names a rule, a file, how many findings of that rule the file may have,
and the reason. One finding more than the count fails. A count that is
higher than what was found is only reported: a pull request run may
hold fewer findings than a run on `main`.

The accepted list today: six findings of the rule `rust/unsafe-deserialization`, entered as false positives with their reasons; the maintainer dismissed the six alerts on GitHub on 2026-11-21.

What to do with a finding is in
[CONTRIBUTING](../CONTRIBUTING.md#pull-requests).

To run it by hand, download the CodeQL bundle from
<https://github.com/github/codeql-action/releases>, unpack it, and from
the repository root:

```sh
codeql database create codeql-db --language=rust --build-mode=none
mkdir -p codeql-out
codeql database analyze codeql-db \
  codeql/rust-queries:codeql-suites/rust-code-scanning.qls \
  --format=sarif-latest --output=codeql-out/rust.sarif
scripts/codeql-gate.sh rust codeql-out
```

The script needs `jq`. It ends with 0 when nothing new was found, 1 on
a finding that is not accepted, 2 on a wrong call or a missing file.
Do not commit `codeql-db` or `codeql-out`.

Last local run: CodeQL bundle 2.27.5, Rust: 6 findings, all
on the accepted list.

## Manual tests

### HealthDCAT-AP demo bundle

`examples/bundle/healthdcat-ap-r8` holds the sources of a demo schema and
vocabulary bundle (ADR 0005): the HealthDCAT-AP release 8 shapes for each
access level, the DCAT-AP 3.0.0 shapes, reduced code lists and a JSON-LD
context. Build and sign it with a new demo bundle key:

```sh
scripts/demo-bundle.sh target/demo-bundle
# AUTHORITY and VERSION change the defaults, which match
# the issuer of examples/demos/two-party/demo.sh (did:web:127.0.0.1%3A18060)
```

It writes `target/demo-bundle/bundles/<version>/` for
`PORTITOR_ISSUER_BUNDLE_DIR` and the public key
`target/demo-bundle/bundle-key.jwk` for `PORTITOR_ISSUER_BUNDLE_KEY_FILE`.
A connector then needs `PORTITOR_BUNDLE_URL=<issuer>/bundles/` (the
bundles folder, not a version folder) and `PORTITOR_BUNDLE_AUTHORITY`;
`PORTITOR_BUNDLE_VERSION` only to keep one version
([DEPLOYING](DEPLOYING.md#schema-and-vocabulary-bundle)).
`examples/healthdcat-datasets.json` has a public and a restricted dataset
that pass the shapes and one, `bed-occupancy`, that does not (7
violations): its `shacl` error shows in `GET /v1/harvest-jobs`. Both demos ([Two connectors](#two-connectors))
and the EDC interop test run with this bundle.

To make the reduced code lists again, convert the sources that
`scripts/reduce_code_lists.py` lists to N-Triples, then run it (its
header says how).

### Without an identity component

A connector needs an identity component and a credential for every DSP
request (ADR 0016), which is more than a quick test wants. The `tck`
build (test-only) also accepts one fixed header instead,
`PORTITOR_TCK_AUTHORIZATION`, and sends it on its own requests. Start it
with the example datasets:

```sh
PORTITOR_PARTICIPANT_ID='did:web:localhost%3A8080' PORTITOR_PUBLIC_URL=http://localhost:8080 \
PORTITOR_DATASETS_FILE=examples/datasets.json PORTITOR_ALLOW_HTTP=true \
PORTITOR_TCK_AUTHORIZATION=manual-test \
PORTITOR_DB_FILE=/tmp/portitor-manual.db cargo run -p portitor-connector --features tck
```

and send `-H 'authorization: manual-test'` with each DSP request below.

The endpoints are described in [docs/api/openapi.json](api/openapi.json)
(OpenAPI 3.1). Any OpenAPI tool that resolves relative `$ref`s from the
repository can show it and send requests. The [Bruno](https://www.usebruno.com/)
collection in `_http/` has these requests ready, with the `single` and
`two-connectors` environments of the [VS Code set-up](#debugging-in-vs-code),
which runs real DCP instead of the `tck` build: the collection gets a
token from the consumer's STS before every DSP request. With `curl` and
the `tck` build:

```sh
curl -s localhost:8080/.well-known/dspace-version
curl -s -X POST localhost:8080/dsp/2025-1/catalog/request \
  -H 'authorization: manual-test' -H 'content-type: application/json' \
  -d '{"@context":["https://w3id.org/dspace/2025/1/context.jsonld"],"@type":"CatalogRequestMessage"}'
```

A negotiation needs a consumer that accepts the provider's callbacks.
This small server answers 200 to every POST and prints it:

```sh
python3 -c '
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        print(self.path, self.rfile.read(int(self.headers["Content-Length"])).decode())
        self.send_response(200); self.end_headers()
HTTPServer(("127.0.0.1", 9000), H).serve_forever()' &
```

Then request an offer from the catalog. Copy the offer's terms unchanged
from the catalog response, or the provider terminates (ADR 0009); this one
is the `weather-stations` offer of the example file:

```sh
curl -s -X POST localhost:8080/dsp/2025-1/negotiations/request \
  -H 'authorization: manual-test' -H 'content-type: application/json' -d '{
  "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
  "@type": "ContractRequestMessage",
  "consumerPid": "urn:uuid:my-test-1",
  "offer": {"@id": "weather-stations:offer:use", "@type": "Offer",
            "target": "weather-stations", "permission": [{"action": "use"}]},
  "callbackAddress": "http://127.0.0.1:9000"}'
```

The fake consumer prints the agreement. Send the verification
(`ContractAgreementVerificationMessage` to
`/dsp/2025-1/negotiations/<providerPid>/agreement/verification`) and the
provider sends the `FINALIZED` event. `GET
/dsp/2025-1/negotiations/<providerPid>` shows the state at each step.

### Two connectors

`examples/demos/two-party/demo.sh` runs a small dataspace on this machine, with DCP,
and checks the result. It needs `curl`, `jq`, `openssl` and Python 3.

```sh
examples/demos/two-party/demo.sh                      # builds the debug binaries
examples/demos/two-party/demo.sh <portitor-connector> [<portitor-identity> [<portitor-issuer>]]
```

What it sets up:

- Each participant (provider and consumer) is a connector and its own
  `portitor-identity`. A `portitor-issuer` with a new key is the
  dataspace's authority (credential definitions in
  `examples/issuer-credentials.json`).
- The script registers both participants at the issuer (claims:
  `memberOf`, and `processing: "eu"` for the consumer). Each identity
  asks the issuer for its credentials through its own management API
  (`POST /v1/credential-requests`); the script waits until the request
  is `ISSUED`, since the issuer decides and delivers in the background.
  The provider gets a `MembershipCredential`, the consumer also a
  `DataProcessorCredential`. The credential definitions copy `memberOf`
  and `role` into the `MembershipCredential`, and `processing` into the
  `DataProcessorCredential`. A demo step that needs another claim
  registers the participant with it (`register` in
  `scripts/participants.sh`).
- The issuer is also the bundle authority: the script builds and signs
  the HealthDCAT-AP demo bundle (`scripts/demo-bundle.sh`), the issuer
  serves it and lists the bundle key, and the provider follows its
  newest version (`PORTITOR_BUNDLE_URL`, `_AUTHORITY`) and offers
  `examples/healthdcat-datasets.json`.
- Both connectors set
  `PORTITOR_POLICY_CREDENTIALS=DataProcessorCredential`, and
  `PORTITOR_STATUS_LIST_CACHE_SECONDS=0` so that the revocation step
  sees a revocation at once. The EDC interop test and the DCP TCK run
  set `0` too.

What it checks, in this order (`examples/demos/two-party/demo-steps.sh`):

1. A catalog request without a token is refused (401).
2. One made through the consumer's management API
   (`POST /v1/catalog/request`, for which the connector gets a token
   from its STS) is answered. Under the plain DSP `@context`, the two
   valid datasets carry `dct:conformsTo` with the release 8 profile and
   their metadata as full IRIs, and `bed-occupancy` is missing. The
   provider's `GET /v1/harvest-jobs/{id}` shows it rejected by the
   `shacl` check with its SHACL report.
3. The `hospital-admissions` offer copied from the catalog negotiates to
   `FINALIZED`; the same offer id without its constraint is
   `TERMINATED`.
4. The restricted `cancer-registry-extract` offer for members (the
   `memberOf` claim of the demo `MembershipCredential`, ADR 0019) is
   `FINALIZED`; its offer for auditors, a `role` claim the demo
   credential does not have, is `TERMINATED`.
5. A transfer under the first agreement: the script fetches the file
   with the token, checks it equals
   `examples/data/hospital-admissions.csv`, completes the transfer and
   checks that the token is refused afterwards. Then it transfers the
   restricted dataset under the members' agreement.
6. A second credential type (ADR 0019): the `hospital-admissions` offer
   for EU processors checks the consumer's `DataProcessorCredential`;
   it negotiates to `FINALIZED` and a transfer starts. The issuer
   revokes that credential (`POST /v1/credentials/{id}/revoke`), and a
   new transfer request under the same agreement is `TERMINATED`.

**The same in Docker Compose.**
`examples/demos/two-party/demo-compose.sh [connector image [identity image [issuer image]]]`
runs the same checks against the services in
`examples/demos/two-party/compose.yaml`: the issuer, a provider and a consumer
(each a connector with its identity), and a small web server that holds
the provider's data. It uses the released images or the ones you name.
The [quickstart](https://bruno.pacheco.lu/portitor/#try) on the website
walks through it.

- It writes the keys and secrets into `examples/demos/two-party/run/` first,
  and builds the demo bundle with the issuer image into
  `examples/demos/two-party/run/issuer/bundle/`.
- It stops everything at the end. `KEEP=1` leaves everything running:
  the consumer's management API is on `127.0.0.1:18091` (key in
  `examples/demos/two-party/run/consumer/management.key`), the provider's, with
  the harvest jobs, on `127.0.0.1:18081` (key in
  `examples/demos/two-party/run/provider/management.key`), the issuer's on
  `127.0.0.1:18071` (key in `examples/demos/two-party/run/issuer/management.key`).
- Running it again after `KEEP=1` works: the script first removes the
  containers and volumes of the last run
  (`docker compose down --volumes --remove-orphans`), then `run/`.
- The images must be 0.9 or later: the Compose file starts each service
  once the ones it needs are healthy (`condition: service_healthy`),
  which needs the image's `health` subcommand.
- To try your own changes, [build the images](#building-the-images-locally)
  and name them:
  `examples/demos/two-party/demo-compose.sh portitor-connector:ci portitor-identity:ci portitor-issuer:ci`.

CI runs both scripts on every pull request, with the binaries and
images it has just built.

**By hand.** Start the Compose demo and keep it running, then use the
consumer's management API. The provider is
`http://provider:8080/dsp/2025-1`, `did:web:provider-identity%3A8090`:

```sh
KEEP=1 examples/demos/two-party/demo-compose.sh
docker compose -f examples/demos/two-party/compose.yaml down --volumes   # when done
```

Start a negotiation from the consumer's management API (the requests
are in [DEPLOYING](DEPLOYING.md#management-api)) and read it with
`GET 127.0.0.1:18091/v1/negotiations/<consumerPid>`. Within a second its
state is `FINALIZED` and it holds the agreement. Then request a transfer
under that agreement with `POST 127.0.0.1:18091/v1/transfers` and
`{"agreementId": "<agreement @id>"}`; the HealthDCAT-AP datasets have a
data source, so it gets `STARTED` and a `dataAddress` to fetch from (on
`127.0.0.1:18080`).

### Two data spaces

`scripts/two-data-spaces.sh shared` and `scripts/two-data-spaces.sh
separate` run one organisation in two dataspaces on this machine, with
one connector per dataspace
([DEPLOYING](DEPLOYING.md#two-data-spaces),
[ADR 0024](adr/0024-several-data-spaces.md)). Each dataspace has its
own issuer and one consumer that trusts only that issuer. `shared`
gives the organisation one identity for both connectors; `separate`
gives it one identity per dataspace. The script checks that the whole
exchange works inside each dataspace (catalog, negotiation, transfer,
data), that a member of one dataspace gets 401 from the connector of
the other, and, for `shared`, that a verifier of one dataspace
receives both membership credentials. It needs curl, jq, openssl and
python3, uses ports 19070 to 19511, and is not run by CI.

Last run by hand that passed: 2026-11-20, both ways, on one machine
over plain HTTP, without a reverse proxy, a bundle or Gaia-X.

### Observability stack (by hand)

`examples/demos/two-party/observability.yaml` adds an OpenTelemetry Collector,
Jaeger and Prometheus to the Compose demo
([ADR 0023](adr/0023-observability.md), section 6). It sets
`OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318` on the issuer,
both identities and both connectors; the collector
(`examples/demos/two-party/otel-collector.yaml`) sends the traces to Jaeger and
the metrics to Prometheus's OTLP endpoint. It is started by hand; CI
does not run it. Stop a demo that is still running first, with the
`docker compose ... down --volumes` command above.

```sh
EXTRA_COMPOSE=examples/demos/two-party/observability.yaml KEEP=1 examples/demos/two-party/demo-compose.sh
```

The components send metrics every 60 s by default
(`OTEL_METRIC_EXPORT_INTERVAL`). The overlay sets it to 10 s
(`OTEL_METRIC_EXPORT_INTERVAL: "10000"` in its `x-otel` block), so
with `KEEP=1` the metrics are in Prometheus about 10 s after the demo's
calls.

- Jaeger UI: <http://127.0.0.1:16686>, pick a Portitor service.
- Prometheus UI: <http://127.0.0.1:9090>. Prometheus turns OpenTelemetry
  names into its own: dots become underscores, the unit is added as a
  suffix and a histogram is stored as `_bucket`, `_sum` and `_count`
  series ([`translation_strategy`](https://github.com/prometheus/prometheus/blob/v3.16.0/docs/configuration/configuration.md),
  default `UnderscoreEscapingWithSuffixes`). `service.name` becomes the
  `job` label and `service.instance.id` the `instance` label (the
  overlay sets it to `authority`, `provider` or `consumer`, so the two
  connectors stay apart). So `http.server.request.duration` (seconds)
  is:

```promql
http_server_request_duration_seconds_count
histogram_quantile(0.95, sum by (le, job, instance) (rate(http_server_request_duration_seconds_bucket[5m])))
```

Stop and remove everything, volumes included:

```sh
docker compose -f examples/demos/two-party/compose.yaml -f examples/demos/two-party/observability.yaml down --volumes
```

## Debugging in VS Code

The repository ships shared VS Code files in `.vscode/`: debug
configurations (`launch.json`), tasks (`tasks.json`) and the recommended
extensions (`extensions.json`). Install the recommended
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
and [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)
when VS Code asks. Then open Run and Debug, pick a configuration and press F5.

Every connector needs its identity component, a credential and a
trusted issuer (DCP, ADR 0016). The task **portitor: identities**
(`scripts/dev-identities.sh`) provides them, as `examples/demos/two-party/demo.sh` does:
it starts a `portitor-issuer` on `127.0.0.1:9100` (management
`127.0.0.1:9104`, key in `target/vscode-dcp/issuer/management.key`) and
two `portitor-identity` processes, each with a `MembershipCredential` it
asked the issuer for, and serves `examples/data` on `127.0.0.1:9105`, the
upstream data of the `weather-stations` dataset:

| Participant | DID | Identity (DID document, STS, credential service) | Identity management |
|---|---|---|---|
| provider | `did:web:127.0.0.1%3A9102` | `127.0.0.1:9102` | `127.0.0.1:9103` |
| consumer | `did:web:127.0.0.1%3A9203` | `127.0.0.1:9203` | `127.0.0.1:9204` |

It keeps running until you stop it, and starts afresh each time (new
keys); restart the connectors after restarting it. Keys, STS client
secrets and logs are in `target/vscode-dcp/`.

| Configuration | What it starts |
|---|---|
| Connector (single, :8080, management :8081) | one connector with `examples/datasets.json`, as the provider participant; starts the identities first; the `single` environment of `_http/` |
| Connector: provider (:9101) / consumer (:9201, management :9202) | the two connectors of [Two connectors](#two-connectors), each with its identity; start the identities task first; the `two-connectors` environment |
| Two connectors (provider + consumer) | the identities, then both connectors; stopping one connector stops both |
| Connector: TCK build (:8080) | the connector with the `tck` feature and `tck/datasets.json`, for the [TCK](#dsp-conformance-tck); no identity, it takes the header `authorization: manual-test` |
| Tests: *crate* (unit) | the unit tests of one crate |
| Tests: *crate* *file* | one integration test file in `crates/*/tests` |

CodeLLDB runs `cargo run` or `cargo test` for the configuration and
starts the result under the debugger, so it always debugs a fresh build.
Databases and the management API key are kept in `target/`
(`target/vscode-*.db`, `target/vscode-management.key`); a task creates the
key the first time. In the `_http/` collection, put that key in the
`apiKey` secret and the consumer's STS client secret
(`target/vscode-dcp/consumer/sts-client-secret`) in `stsSecret`; the
collection then gets a fresh token from the consumer's STS before each
DSP request (`auth/STS token` shows one). For the `identity` and
`issuer` folders, put the identity's management key
(`target/vscode-dcp/<provider or consumer>/identity-management.key`:
the provider's with `single`, the consumer's with `two-connectors`) in
`identityApiKey` and `target/vscode-dcp/issuer/management.key` in
`issuerApiKey`. Delete the `.db` files to start from an empty store.

The collection's folders:

| Folder | Requests |
|---|---|
| `metadata`, `catalog`, `negotiation-provider`, `transfer-provider` | DSP requests to the provider, each with a fresh DCP token; also a catalog request and a negotiation read without a valid token (401 and 404), a repeated transfer request (the same answer again) and the two negotiation messages the provider always refuses with 400 (it never sends an offer) |
| `auth` | one token from the consumer's STS, to look at |
| `management` | the consumer's management API, which is not DSP: read the provider's catalog and one dataset, start and read negotiations, start, read, complete and terminate transfers, harvest jobs, the health checks, the OpenAPI description |
| `data` | fetch the data of a started transfer from the provider's data plane, with the token of its data address |
| `identity` | an identity's management API: credentials, credential requests to the issuer, offers, Gaia-X compliance runs, the health checks; and its public DID document |
| `issuer` | the issuer's management API: participants, offers and issued credentials, the health checks; and its public DID document, metadata and status list |

`transfer-provider` needs an agreement in the variable `agreementId`:
`management/Get negotiation` saves it in `two-connectors`; its
`folder.bru` says how to get one in `single`. Two requests need an id
you copy by hand into the environment, because they destroy something:
`credentialIdToDelete` (`identity/Delete credential`) and
`credentialIdToRevoke` (`issuer/Revoke credential`). Not in the
collection: the consumer's callback routes (only the provider of the
negotiation or transfer may call them), the credential service and
issuer service routes that need another participant's token, and the
`tck` routes.

A transfer in `two-connectors`: `Start negotiation`, `Get negotiation`
(saves the agreement id), `Start transfer`, `Get transfer` (saves the
data address), `data/Fetch data`, `Complete transfer`. The provider
fetches the `weather-stations` data from the small web server that
`scripts/dev-identities.sh` runs on `127.0.0.1:9105` (`examples/data`,
the dataset's `portitor:source`). With the Bruno CLI, a step needs
about a second after the one before (`bru run ... --delay 1000`).

To debug one test, use the **Debug** link that rust-analyzer shows above
each `#[test]`, or add the test name to `"args"` of a test configuration.
A new test file in `crates/*/tests` gets its own configuration in
`launch.json`.

Personal settings go in `.vscode/settings.json`, which git ignores. If
CodeLLDB cannot find `cargo` (common on macOS when VS Code is not started
from a terminal), set `lldb.cargo` there or in your user settings to the
full path, for example `/Users/<you>/.cargo/bin/cargo`.

Fuzz targets need a nightly toolchain and are not in `launch.json`.

## Footprint

`scripts/measure.sh` measures size, start time, memory and latency
against the targets in ADR 0002, and what DCP costs per DSP request
(ADR 0016). It starts a provider (connector and identity) and a
consumer identity, with credentials from a `portitor-issuer`, as
`examples/demos/two-party/demo.sh` does; it reports the issuer's binary size and memory
too.
The load tool is the `dsp-load` example of the connector: it sends
catalog requests from 50 clients, each with a fresh token from the
consumer's STS (a general load tool cannot, since a DCP token may be used
once). With `TCK_CONNECTOR`, a connector built with the `tck` feature
runs the same load with a fixed header, without DCP. Every wait is
bounded; it needs `curl`, `openssl` and Python 3.

```sh
scripts/measure.sh                   # builds the release binaries and the tools
cargo build --release -p portitor-connector --features tck
cp target/release/portitor-connector /tmp/portitor-connector-tck
cargo build --release -p portitor-connector -p portitor-identity -p portitor-issuer
TCK_CONNECTOR=/tmp/portitor-connector-tck \
  scripts/measure.sh target/release/portitor-connector
```

The results of each release are in ADR 0002, section 7, and from 0.12
in [BENCHMARKS](BENCHMARKS.md).

`scripts/bench.sh` measures the reference load of BENCHMARKS: the data
plane under many, large and long pulls, the stored history, the
catalog, negotiations at one per second, and whether negotiations slow
the data plane down. Its load tool is the `bench-load` example of the
connector (a data source that makes its bytes up, contracts through a
consumer's management API, timed pulls, timed catalog requests). Each
part starts its own participants with empty databases; the whole run
takes about 20 minutes and needs 2 GB of free memory.

```sh
scripts/bench.sh                     # every part
BENCH_LARGE_GB=1 BENCH_HISTORY=500 BENCH_RUNS=1 scripts/bench.sh large history   # a quick look
```
