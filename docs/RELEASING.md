<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Releasing

All components share one version (mono repo). Pushing a signed tag `vX.Y.Z`
runs [`.github/workflows/release.yml`](../.github/workflows/release.yml), which:
1. checks the tag is annotated, signed by the maintainer (keys from
   `github.com/<owner>.gpg`), on a commit of `main`, equal to `version`
   in `Cargo.toml`, and listed in `CHANGELOG.md`;
2. tests the workspace and builds static musl binaries on native amd64 and
   arm64 runners;
3. builds one multi-arch image per component and pushes it to
   `ghcr.io/brunopacheco1/<component>:X.Y.Z`. From 0.6 there are three:

   | Image | Ports | State |
   |---|---|---|
   | `ghcr.io/brunopacheco1/portitor-connector` | 8080 DSP, 8081 management | `/data/portitor.db` |
   | `ghcr.io/brunopacheco1/portitor-identity` | 8090 DID document, STS, credential service; 8091 management | `/data/portitor-identity.db` |
   | `ghcr.io/brunopacheco1/portitor-issuer` | 8070 DID document, DCP issuer service, status lists; 8071 management | `/data/portitor-issuer.db` |

   ([ADR 0003](adr/0003-component-split.md), [ADR 0018](adr/0018-issuer-component.md));
4. signs each image with cosign (keyless, the workflow's identity);
5. attaches build provenance (SLSA v1) and a CycloneDX SBOM to each image
   as GitHub attestations;
6. creates a GitHub release with the changelog section, the image
   digests, the binaries, one SBOM and one provenance bundle per component
   (`<component>-X.Y.Z.cdx.json`, `<component>-X.Y.Z.intoto.jsonl`) and
   `THIRD-PARTY-NOTICES.txt` (all components).

The components are listed once, in `COMPONENTS` at the top of the
workflow; each one is a build target of the same name in the single
`Dockerfile` (`docker build --target portitor-identity .`). The image
steps (build, push, sign, attest) run as a matrix over that list, one job
per image, and the GitHub release is created only when every image job
succeeded. Adding a component means: a binary crate, a target in the
`Dockerfile`, a line in `.dockerignore`, and its name in `COMPONENTS`.

A pull request that changes the release workflow, the root `Cargo.toml`,
the `Dockerfile`, `.dockerignore` or the notices script runs the same
build as a dry run: nothing is pushed or signed. The release pull request
sets the version in `Cargo.toml`, so it always gets a dry run: the arm64
build and tests, the SBOMs and the notices are checked before the tag.

## One-time setup (owner)

1. **Settings → Environments → New environment** `release`. Restrict it to
   tags matching `v*`, and under **Required reviewers** add yourself, so
   that no image is pushed or signed before you approve the run. Each
   image job uses the environment; one **Review deployments** approval
   covers all images of a run. Before you approve, check on the run's page
   that the tag is the one you pushed and that its commit is on `main`.
2. **Settings → Rules → Rulesets → New tag ruleset**, target tags
   matching `v*`: **Restrict creations**, **Restrict updates** and
   **Restrict deletions**, with yourself (the repository admin role) as
   the only one on the bypass list. Nobody else, and no token of a
   workflow, can then create or move a release tag.
3. These two settings are the control over who can publish a release.
   The workflow's own checks of the tag (signed, on `main`, same version
   as `Cargo.toml` and `CHANGELOG.md`) guard against mistakes only: on a
   tag push GitHub takes `release.yml` from the tagged commit, so a
   commit that removes the checks would run without them. Without the
   required reviewer and the ruleset, anyone who can push a `v*` tag
   could publish signed images from a commit that was never on `main`.
4. No signing secrets are needed: cosign keyless signing and attestations
   use the workflow's OIDC token. Images are pushed with the workflow's
   `GITHUB_TOKEN`.
5. After the first release of each image, make its package public:
   **Packages → portitor-connector** (and, from 0.5, **portitor-identity**;
   from 0.6, **portitor-issuer**)
   **→ Package settings → Change visibility → Public**, and link it to this
   repository if GitHub did not. A new package starts private, so pulling
   it fails until this is done.

## Documentation review

Every release checks its documentation against the code before the
version is set, as part of the release's quality review (on the release
branch, next to the readability review). Fix what is wrong in the release
pull request, or record why it stays.

- `python3 scripts/check_docs.py` passes (CI runs it too): the C4
  diagrams in `docs/diagrams/` show every crate and image, and name only
  files and types that exist; every OpenAPI file has the new version.
- The C4 diagrams ([ARCHITECTURE.md](ARCHITECTURE.md), "Diagrams") still
  tell the truth: new components, ports, adapters, containers or outside
  systems are drawn, removed ones are gone. Edit them in draw.io.
- [ARCHITECTURE.md](ARCHITECTURE.md), [DEPLOYING.md](DEPLOYING.md),
  [OPERATING.md](OPERATING.md), [TESTING.md](TESTING.md) and the README
  match what the release does (settings, ports, images, flows, logs,
  metrics, upgrade notes). The settings and routes have their home in
  DEPLOYING and the OpenAPI files; the README only links to them.
- [READING-THE-CODE.md](READING-THE-CODE.md) names only files and
  functions that exist (`check_docs.py` does not check paths in
  Markdown), and the [ADR index](adr/README.md) has a row for every
  ADR, with its current status.
- The OpenAPI files in `docs/api/`: the generated management descriptions
  are current (their tests fail otherwise); the hand-written
  `openapi.json` (DSP), `identity-openapi.json` (DID document, STS,
  DCP credential service) and `issuer-openapi.json` (DID document, DCP
  issuer service, status lists) list every route (their tests check that each
  listed route is served) and describe what changed.
- [STABILITY.md](STABILITY.md) (from 0.13, the freeze): its lists of
  routes and settings are those of the code. The freeze guard
  (`tests/frozen_interface.rs` in the three binaries' crates, part of
  `cargo test`) fails when a listed route or setting leaves the code,
  and when the code gains one that is not listed. A new route or setting
  gets its line there in the same pull request; from 1.0 a listed one is
  removed or renamed only in 2.0. The guard does not check the other
  promises (command line, file formats, database, "What 1.0 does not
  do"): read them against the release.
- [TESTING.md](TESTING.md): the "last run that passed" tables (container
  scripts, EDC interop test) name a run of this release, or say that
  none passed yet.
- The website (`site/index.html`, published with the API pages by
  `pages.yml`) shows the release's features and the roadmap state.
- The quickstart ("Try it" on the website, and `scripts/demo-compose.sh`
  in TESTING) works with the release's images: its prerequisites, the
  `git clone --branch` tag, the commands and the ports it names.
- [ROADMAP.md](ROADMAP.md) and the [CHANGELOG](../CHANGELOG.md) say what
  shipped.
- Supply chain: `cargo vet --locked` passes; count the exemptions in
  `supply-chain/config.toml` and note the number in R-04 (it should
  shrink).
- [RISKS_AND_TECHNICAL_DEBTS.md](RISKS_AND_TECHNICAL_DEBTS.md): each open
  item is checked against the release's code and gets its new status;
  what the release raised (ADR consequences, spike risks, review
  findings, "not in this release" lists) is added with a new id.

## Releasing a version

1. In a pull request, set `version` in `Cargo.toml` (`[workspace.package]`)
   to `X.Y.Z`, rename `## [Unreleased]` in `CHANGELOG.md` to
   `## [X.Y.Z] - YYYY-MM-DD`, add a fresh empty `## [Unreleased]` above it,
   set `info.version` in `docs/api/openapi.json`,
   `docs/api/identity-openapi.json` and `docs/api/issuer-openapi.json`, regenerate
   `docs/api/management-openapi.json`,
   `docs/api/identity-management-openapi.json`
   (`PORTITOR_UPDATE_OPENAPI=1 cargo test -p portitor-connector --test management_api`
   and the same with `-p portitor-identity`) and
   `docs/api/issuer-management-openapi.json`
   (`cargo test -p portitor-issuer --test issuer -- --ignored write_management_description`),
   update the fuzz lock file (`cargo update -w` in `fuzz/`),
   set the default image tags in `examples/compose/compose.yaml` and the
   `git clone --branch` tag in `site/index.html` to `X.Y.Z` (`vX.Y.Z`),
   and merge.
2. Tag the commit that the merge put on `main` with your personal key
   and push the tag:
   ```sh
   git switch main && git pull
   git tag -s vX.Y.Z -m "Portitor X.Y.Z"
   git push origin vX.Y.Z
   ```
   The tag must be on a commit of `main`: the workflow refuses a tag on
   any other commit, so a signed tag put on an unmerged branch by
   mistake publishes nothing. Pull requests are squashed, so the commit
   to tag is the squash commit on `main` (what `git pull` just fetched),
   never the head of the release pull request: that commit is not on
   `main` after a squash merge, and the check refuses it. This check is
   a guard against mistakes; what controls who can release is in
   "One-time setup", steps 1 to 3. A release from a maintenance branch
   needs the rule widened in `release.yml` first (job `verify`).
3. Watch **Actions → Release**. When it is green, the images and the GitHub
   release are published.
4. Run the website's quickstart ("Try it" in `site/index.html`) on a
   clean machine, with no Portitor images or clone on it, so that it
   pulls the new images. Time it, and fix the website if a command or
   the time it gives is wrong.

The tag comes last: the check of the tag reads `Cargo.toml` and
`CHANGELOG.md` at the tagged commit, so a tag pushed before the release
pull request (step 1) is merged fails at once, with
`Cargo.toml has version ..., the tag says ...`. Nothing is built or
published then. Delete the tag (on GitHub and in your clone), merge the
release pull request, and tag its squash commit:

```sh
git push origin :refs/tags/vX.Y.Z   # the tag ruleset lets the admin delete it
git tag -d vX.Y.Z
```

### Release candidates

A release candidate is a version with a pre-release part, such as
`1.0.0-rc.1` (tag `v1.0.0-rc.1`). It follows the same steps, with three
differences:

- The GitHub release is marked as a pre-release, so it never becomes the
  "latest" release (the workflow does it when the version has a `-`).
- `examples/compose/compose.yaml` takes the candidate's image tags, so a
  clone of the candidate's tag runs its images, but the website's
  `git clone --branch` tag stays on the last stable release: the
  quickstart is for users. Run the quickstart of step 4 with the
  candidate's clone instead (`git clone --branch vX.Y.Z-rc.N`).
- The final release (`X.Y.Z`) gets its own release pull request: the
  candidate's CHANGELOG section stays, and the new section says what
  changed since the candidate (or that nothing did).

## Checking a release

Check every image of the release the same way:

```sh
for image in portitor-connector portitor-identity portitor-issuer; do
  # signature: signed by this repository's release workflow, from a tag
  cosign verify "ghcr.io/brunopacheco1/$image:X.Y.Z" \
    --certificate-identity-regexp '^https://github.com/brunopacheco1/portitor/\.github/workflows/release\.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
  # build provenance (SLSA v1)
  gh attestation verify "oci://ghcr.io/brunopacheco1/$image:X.Y.Z" --repo brunopacheco1/portitor
  # SBOM attestation (CycloneDX)
  gh attestation verify "oci://ghcr.io/brunopacheco1/$image:X.Y.Z" --repo brunopacheco1/portitor \
    --predicate-type https://cyclonedx.org/bom
done
```

- The digests printed by `cosign verify` must match the ones in the
  release notes.
- The release tag is signed with the maintainer's key; GitHub publishes
  the public key:

  ```sh
  curl -fsSL https://github.com/brunopacheco1.gpg | gpg --import
  git verify-tag vX.Y.Z
  ```
- Binaries attached to the release (from the first release after
  0.14.0): each has a signed build provenance, checked with
  `gh attestation verify portitor-connector-X.Y.Z-linux-amd64 --repo brunopacheco1/portitor`
  (the same for the other five). Their SHA-256 is also printed in the
  release run's log by `scripts/build-release.sh`.
- SBOMs: attached to the GitHub release (one per component) and to each
  image as an attestation.

## Reproducible builds

A release can be rebuilt from its tag, and the rebuild gives the same
bytes: the same binaries as the GitHub release, and the same image
digests as on GHCR.

How the build is kept reproducible:

- the binaries are built by one script, `scripts/build-release.sh`, in
  the release and in every rebuild. It pins the Rust version (through
  `RUST_TOOLCHAIN` in the workflows) and every crate (`--locked`), maps
  the checkout and Cargo folders to fixed names in the binaries
  (`--remap-path-prefix` for Rust, `-ffile-prefix-map` for the C code of
  ring and SQLite), and sets `SOURCE_DATE_EPOCH` to the commit time;
- the images are built by a pinned BuildKit, with `SOURCE_DATE_EPOCH`
  set to the commit time and `rewrite-timestamp=true`, so the image
  config, the history and every file in the layers carry the commit
  time, not the time of the build;
- the C compiler comes from the runner image (`ubuntu-24.04`, package
  `musl-tools`), so a rebuild runs on the same runner image.

The check, `.github/workflows/reproducible.yml` (badge "Reproducible
build" in the README):

- after every release, it rebuilds the tag on fresh runners and
  compares each binary with the GitHub release and each image digest
  with GHCR; it can be started by hand for a tag (`workflow_dispatch`,
  input `tag`);
- on `main` (weekly, and when the build files change) and on pull
  requests that change the build, it builds the same commit twice, on
  separate runners, and compares the two.

The job summary lists each SHA-256 and digest side by side; the run
fails on any difference, and keeps both sets of digests as artifacts.

This is a check on GitHub's runners by this project, not by an
independent rebuilder: on 2026-11-21 no public rebuilder was found for
Rust binaries or container images published on GitHub and GHCR
(Reproducible Central covers Maven Central only, rebuilderd covers Linux
distribution packages, OSS Rebuild covers popular npm, PyPI and
crates.io packages). Anyone can repeat the rebuild: check out the tag on
an `ubuntu-24.04` machine with Rust `RUST_TOOLCHAIN` and `musl-tools`,
run `scripts/build-release.sh x86_64-unknown-linux-musl amd64`, and
compare `sha256sum dist/amd64/*` with the files of the release. Releases
up to 0.14.0 were built before this script and do not rebuild to the
same bytes.

If the release fails after the tag is pushed, fix the cause, delete the tag
(locally and with `git push origin :refs/tags/vX.Y.Z`), and tag again,
unless an image was already pushed; then use a new patch number.
