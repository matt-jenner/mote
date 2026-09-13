#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_dir/build-lifecycle.sh"

[ "$#" -gt 0 ] || {
	printf '%s\n' "usage: $0 COMMAND [ARG ...]" >&2
	exit 2
}

mote_create_build_dir cargo
mote_install_cleanup_traps
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR"
"$@"
