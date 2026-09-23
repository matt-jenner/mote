#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_dir/build-lifecycle.sh"

if [ -n "${MOTE_HEIC_PREFIX:-}" ]; then
	export PKG_CONFIG_PATH="$MOTE_HEIC_PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
	case $(uname -s) in
		Darwin) export DYLD_LIBRARY_PATH="$MOTE_HEIC_PREFIX/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" ;;
		Linux) export LD_LIBRARY_PATH="$MOTE_HEIC_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ;;
	esac
fi

mote_create_build_dir cargo
mote_install_cleanup_traps
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR"

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p catalog-bench --test benchmark_smoke
