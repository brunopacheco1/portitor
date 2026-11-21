#!/usr/bin/env bash
# SPDX-FileCopyrightText: Portitor contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Builds the release binaries, the same way every time, so a rebuild of a
# release gives the same bytes (reproducible build; docs/RELEASING.md,
# "Reproducible builds"). The release workflow and the rebuild check
# (.github/workflows/reproducible.yml) both call this script.
#
#   scripts/build-release.sh TARGET ARCH
#   scripts/build-release.sh x86_64-unknown-linux-musl amd64
#
# Writes dist/ARCH/portitor-{connector,identity,issuer}.
#
# What could differ between two builds, and how it is fixed:
# - the folder of the checkout and of Cargo's home end up in panic messages
#   and in the C objects of ring and SQLite: both are mapped to fixed names
#   (--remap-path-prefix for Rust, -ffile-prefix-map for C);
# - the time: SOURCE_DATE_EPOCH is the commit time, for any build script
#   that reads it;
# - the toolchain: the Rust version is pinned (RUST_TOOLCHAIN in the
#   workflows), and Cargo.lock with --locked pins every crate.
# The C compiler comes from the runner image (musl-tools), so a rebuild uses
# the same runner image as the release (ubuntu-24.04).
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 TARGET ARCH (for example x86_64-unknown-linux-musl amd64)" >&2
  exit 2
fi
target=$1
arch=$2
components="portitor-connector portitor-identity portitor-issuer"

root=$(git rev-parse --show-toplevel)
cd "$root"
cargo_home=${CARGO_HOME:-$HOME/.cargo}

SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
export SOURCE_DATE_EPOCH
export RUSTFLAGS="--remap-path-prefix=${root}=/portitor --remap-path-prefix=${cargo_home}=/cargo"
export CFLAGS="-ffile-prefix-map=${root}=/portitor -ffile-prefix-map=${cargo_home}=/cargo"
export CARGO_INCREMENTAL=0

mkdir -p "dist/${arch}"
for c in $components; do
  cargo build --release --locked --target "$target" -p "$c"
  cp "target/${target}/release/$c" "dist/${arch}/"
done
(cd "dist/${arch}" && sha256sum $components)
