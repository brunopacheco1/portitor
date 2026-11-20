<!--
SPDX-FileCopyrightText: Portitor contributors

SPDX-License-Identifier: Apache-2.0
-->

# Governance

Portitor is a small project with a single maintainer. This document says who
decides what, how to take part, and how the project would continue if the
maintainer stopped.

## Roles

| Role | Who | Responsibilities |
|---|---|---|
| **Maintainer** | Bruno Pacheco ([@brunopacheco1](https://github.com/brunopacheco1)) | Reviews and merges pull requests; decides the roadmap; publishes releases; answers security reports ([SECURITY.md](SECURITY.md)); keeps CI, dependencies and the website healthy; enforces the [code of conduct](CODE_OF_CONDUCT.md). |
| **Contributor** | Anyone who opens an issue, a pull request or a review | Follows [CONTRIBUTING.md](CONTRIBUTING.md) and the code of conduct. Contributors keep the copyright of their work, licensed under Apache-2.0. |
| **Reviewer** | Any contributor | Reviews pull requests. Reviews from people other than a change's author are welcome on every pull request, including the maintainer's own. |

There is one maintainer today, so every change still needs that person's
approval (`.github/CODEOWNERS`). **A second maintainer is wanted:** someone
who has contributed and reviewed for a while can ask, or be invited; the
maintainer adds them to CODEOWNERS, gives them write access, and records it
here.

## Decisions

- **Day to day:** in issues and pull requests, in the open. The maintainer
  decides when there is no agreement, and says why.
- **Larger decisions** (language, components, protocols, crypto,
  dependencies, licence) are recorded as ADRs in [docs/adr/](docs/adr/),
  proposed in a pull request first.
- **Compatibility:** versions follow [Semantic Versioning](https://semver.org/).
  Until 1.0, minor versions may change behaviour and configuration; every
  change is listed in [CHANGELOG.md](CHANGELOG.md).

## Releases

The maintainer releases by pushing a signed `v*` tag; CI builds, signs and
publishes the images ([docs/RELEASING.md](docs/RELEASING.md)). Security
fixes follow [SECURITY.md](SECURITY.md).

## Continuity

If the maintainer can no longer continue, the project should go on with
minimal interruption: within a week, someone should be able to triage
issues, accept changes and publish a release.

- **A successor for the repository:** the maintainer has set a trusted
  person as account successor in GitHub's account settings (GitHub's
  account successor feature). If the maintainer is no longer able to,
  that person can take over the repositories, with their issues, CI and
  website.
- **What does not move with the repository:** release tags are signed
  with the maintainer's personal key, and the images are published in
  the container registry under the maintainer's account
  ([docs/RELEASING.md](docs/RELEASING.md)). A successor would sign tags
  with a new key, and publish the images under their own account if the
  registry path cannot be kept. They would say so in the release notes
  and in the documents that tell users how to check a release. The
  images themselves are signed keyless (cosign with the workflow's
  identity), so no signing secret has to be handed over.
- **The code** is Apache-2.0 and public: anyone may fork it and carry on.
- **Everything else** (SonarCloud, the OpenSSF Best Practices entry, the
  website on GitHub Pages) is tied to the repository and moves with it.

## Changing this document

Changes to governance are proposed in a pull request and decided by the
maintainers.
