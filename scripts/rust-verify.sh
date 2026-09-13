#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_dir/build-lifecycle.sh"

mote_create_build_dir cargo
mote_install_cleanup_traps
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p catalog-bench --test benchmark_smoke
