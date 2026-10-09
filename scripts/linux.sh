#!/usr/bin/env bash
# Runs a cargo command for the Linux target, in a podman container with the
# repository mounted, so the Linux-only code compiles and tests on Windows.
#
#   scripts/linux.sh clippy --workspace --all-targets --all-features --locked
#   scripts/linux.sh test --workspace --locked
#   scripts/linux.sh mutants -p sungma-clock -f crates/sungma-clock/src/linux.rs
#
# Named volumes keep the toolchain, the cargo registry and the build output
# between runs, apart from the Windows target directory. Builds use four
# jobs to fit the podman machine's memory.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd -W 2>/dev/null || pwd)"

MSYS_NO_PATHCONV=1 exec podman run --rm \
  -v "$repo:/work" \
  -v sungma-cargo:/usr/local/cargo/registry \
  -v sungma-rustup:/usr/local/rustup \
  -v sungma-target:/target \
  -e CARGO_TARGET_DIR=/target \
  -e CARGO_BUILD_JOBS=4 \
  -e RUSTFLAGS="${RUSTFLAGS:--D warnings}" \
  -w /work \
  docker.io/library/rust:1 \
  cargo "$@"
