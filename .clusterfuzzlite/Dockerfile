# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0

# Builder image for ClusterFuzzLite: nightly Rust with cargo-fuzz.
# Pinned by digest; Dependabot proposes updates.
FROM gcr.io/oss-fuzz-base/base-builder-rust@sha256:2216f0ac0a289a0e8974917779c3f744c354067dd81e0c8f2fc53bc1a9a573db
COPY . $SRC/portitor
WORKDIR $SRC/portitor
COPY .clusterfuzzlite/build.sh $SRC/
