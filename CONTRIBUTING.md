<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Contributing to Portitor

Thanks for helping. Bug reports, questions and pull requests are all welcome.

## Before you start

How the project is run is in [GOVERNANCE.md](GOVERNANCE.md); the design is
in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and the decisions in
[docs/adr/](docs/adr/). AI coding agents also follow [AGENTS.md](AGENTS.md).

- For anything larger than a small fix, open an issue first so we can agree
  on the approach. A change to an accepted decision needs a new ADR.
- Security problems: do **not** open a public issue; see [SECURITY.md](SECURITY.md).
- Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Building

Portitor needs the Rust toolchain pinned in CI (`RUST_TOOLCHAIN` in
`.github/workflows/ci.yml`) or newer, and `rust-version` from `Cargo.toml`
at least.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
cargo deny check   # licences, advisories, crate sources (deny.toml)
```

How to test, including the DSP conformance tests (TCK), fuzzing and
manual tests: [docs/TESTING.md](docs/TESTING.md).

## Code style

The build enforces the style, so a pull request that breaks it fails:
- **rustfmt** with default settings, which follow the
  [Rust Style Guide](https://doc.rust-lang.org/style-guide/).
- **Clippy** with `pedantic` lints; warnings are errors. `unsafe` is
  forbidden and `unwrap()` is denied outside tests (`Cargo.toml`,
  `[workspace.lints]`). Allow a lint only where it cannot be fixed, as
  narrowly as possible, with a comment saying why.
- **Hexagonal architecture** ([ADR 0004](docs/adr/0004-hexagonal-architecture.md)):
  domain crates have no I/O dependencies; CI checks it.
- **SonarCloud** reviews every pull request; its quality gate must pass.
- **Plain code.** Write for a maintainer who reads Rust now and then:
  full names rather than one letter, a `match` or a loop rather than a
  long chain of closures, and a comment where a pattern hides what the
  code does. [docs/RUST-IDIOMS.md](docs/RUST-IDIOMS.md) explains the
  patterns the code uses.
- **Docs with the code.** A change that adds, removes or renames a
  crate, image, port, adapter or HTTP route updates the C4 diagrams
  (`docs/diagrams/`, edited in draw.io), the OpenAPI files in `docs/api/`
  and the docs that describe it, in the same pull request.
  `scripts/check_docs.py` (in CI) catches part of it.

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Keep the change focused. **Test policy:** a change that adds
   functionality must add tests for it to the automated test suite
   (`cargo test`; the TCK or interop tests where a protocol is
   involved), and a bug fix must add a test that fails without the fix.
   A pull request without them is not merged. Protocol behaviour cites
   the spec section it implements.
3. New files need an SPDX header (copy an existing one), or are listed in
   `REUSE.toml`. CI runs [`reuse lint`](https://reuse.software).
4. New dependencies must have a licence on the allowlist in `deny.toml`
   (compatible with Apache-2.0); say in the pull request why the
   dependency is needed. Every new or bumped crate must also pass
   cargo-vet (below).
5. Describe what changed and why, and add a line to `CHANGELOG.md` under
   *Unreleased*.
6. A CodeQL finding fails the `CodeQL` job
   ([TESTING](docs/TESTING.md#codeql)): fix it. A false positive goes,
   with its reason, into `.github/codeql/accepted-findings.json` in the
   same pull request, and a maintainer dismisses the alert on GitHub
   with the same reason.
7. Commits must be signed, and `main` only changes through pull requests
   that pass the required checks, are up to date with `main`, and have
   one approval. `.github/CODEOWNERS` names who is asked to review; the
   ruleset does not require the approval to be a code owner's.

## Required checks

The ruleset on `main` requires five checks, by these names:

| Check | Workflow | It passes when |
|---|---|---|
| `CI gate` | `CI` (`ci.yml`) | `REUSE compliance` and `Documentation matches the code` passed, and `Lint, test and build` and `SonarQube Cloud` passed or were not needed |
| `TCK gate` | `TCK` (`tck.yml`) | the four TCK jobs passed or were not needed |
| `Dependencies gate` | `Dependencies` (`dependencies.yml`) | the dependency review, cargo-deny and cargo-vet passed or were not needed |
| `Analyze (rust)` | `CodeQL` (`codeql.yml`) | the analysis ran and has no finding outside the accepted list |
| `Analyze (actions)` | `CodeQL` (`codeql.yml`) | the same, for the workflow files |

"Not needed" is decided by `scripts/ci_changes.sh` from the changed
files:

- A change to prose only skips the build, the tests, the TCKs and the
  dependency checks. Prose is `site/`, `_http/`, `.vscode/`, the `*.md`
  files in the repository root, and the `*.md` files under `docs/`
  except `docs/STABILITY.md` and `docs/ARCHITECTURE.md`, which tests and
  `scripts/check_docs.py` read.
- A change to `fuzz/` or `.clusterfuzzlite/` only skips the build, the
  tests and the TCKs, not the dependency checks.
- Any other path runs everything. So do the weekly and the manual runs.

The licence check (`reuse lint`) and `scripts/check_docs.py` always run.
The script tests its own rule in every run
(`scripts/ci_changes.sh self-test`: the path lists, and the reading of
the changed files in a temporary git repository). Before you add a path
to the prose list, check that no test, script or workflow reads it, and
add a case to the self-test.

What a gate job decides is in `scripts/ci_gate.sh`, one script for the
three gates, which also tests its own rule in every run
(`scripts/ci_gate.sh self-test`). When a workflow gets a new job that
the gate must wait for, add it to the gate job's `needs` and to its
`NEEDED_JOBS` (or `ALWAYS_JOBS`).

Of a workflow that skips jobs, only the gate job can be a required
check. A job that is skipped on
purpose reports "skipped", and its name alone cannot tell that from
"passed after a failure upstream"; the gate job can. A workflow with a
path filter (`EDC interop`, `Gaia-X`, `ORT`, `Fuzzing (PR)`, `Website`,
the `Release` dry run) must never be required: on a pull request that
does not match the filter it reports nothing, and the pull request
would wait for ever. CodeQL runs on every pull request and skips
nothing, so its two jobs are required by their own names.

## Dependency audits (cargo-vet)

CI runs [cargo-vet](https://mozilla.github.io/cargo-vet/) (`cargo vet
--locked`, workflow `Dependencies`) on the root `Cargo.lock`
(the shipped crates). Every crate version must be audited or exempted, in
`supply-chain/`:

- `config.toml`: the audit sets we import, and the exemptions. Crates the
  shipped code uses, including build dependencies and macros, need
  `safe-to-deploy`; crates used only by tests need `safe-to-run`
  (cargo-vet's defaults).
- `audits.toml`: our own audits. An audit is a claim that **a person**
  read the code, so only a maintainer records one, under their own name;
  tools and AI agents never do.
- `imports.lock`: the imported audits, pinned, so CI checks against the
  same audits every time (`--locked` fetches no newer ones).

Imported sets (from [cargo-vet's registry](https://github.com/mozilla/cargo-vet/blob/main/registry.toml)):
Mozilla, Google, Bytecode Alliance, ISRG and Zcash. On 2026-11-16 they
cover 45 of the 204 crate versions; the other 159 are exempted at the
versions locked that day. An exemption means "trusted as it is, not
reviewed"; it does not cover a newer version.

Install it once with `cargo install cargo-vet --version 0.10.3 --locked`
(the version CI uses).

When cargo-vet fails on a pull request (usually a Dependabot bump):

1. `cargo vet` (without `--locked`) fetches newer imported audits; if one
   covers the new version, commit the updated `imports.lock` and
   `config.toml` (it may also drop exemptions no longer needed).
2. Otherwise `cargo vet suggest` lists what is missing, and
   `cargo vet diff <crate> <old> <new> --mode local` shows the change
   to read, in the terminal (without `--mode local` cargo-vet opens the
   website diff.rs, which did not find the page on 2026-11-20). After
   reading it, record it with `cargo vet certify <crate> <old> <new>`
   (a delta audit), and commit `audits.toml`.
3. If the change cannot be reviewed now, exempt that version with
   `cargo vet add-exemption <crate> <version> --criteria safe-to-deploy
   --notes "why"` (`safe-to-run` for a test-only crate), and say so in
   the pull request.

`fuzz/` has its own lock file and is not shipped, so cargo-vet does not
check it.

## Licensing of contributions

Portitor is licensed under the [Apache License 2.0](LICENSE). Unless you state
otherwise, any contribution you submit is licensed under the same terms, as
section 5 of the licence describes. No separate contributor agreement or
sign-off is required.
