#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
interface_dir="$repository_root/apps/interface"
. "$script_dir/build-lifecycle.sh"

[ "$#" -gt 0 ] || {
	printf '%s\n' "usage: $0 COMMAND [ARG ...]" >&2
	exit 2
}

mote_create_build_dir interface

cleanup_interface_assets_on_exit() {
	command_status=$?
	trap - EXIT HUP INT TERM
	cleanup_status=0
	mote_cleanup_build_dir || cleanup_status=$?
	for active_build in "$MOTE_BUILD_TMP_ROOT"/mote-build-interface.*; do
		[ -d "$active_build" ] || continue
		owner_file="$active_build/.mote-owner-pid"
		if IFS= read -r owner_pid <"$owner_file" 2>/dev/null; then
			case "$owner_pid" in
				'' | *[!0-9]*) ;;
				*)
					if kill -0 "$owner_pid" 2>/dev/null; then
						if [ "$command_status" -ne 0 ]; then
							exit "$command_status"
						fi
						exit "$cleanup_status"
					fi
					;;
			esac
		fi
	done
	for generated_path in \
		"$repository_root/node_modules/.vite" \
		"$repository_root/node_modules/.vite-temp" \
		"$interface_dir/dist" \
		"$interface_dir/.vite" \
		"$interface_dir/node_modules/.vite" \
		"$interface_dir/node_modules/.vite-temp" \
		"$interface_dir/tsconfig.tsbuildinfo"; do
		[ -e "$generated_path" ] || continue
		case "$generated_path" in
			"$repository_root/node_modules/.vite" | "$repository_root/node_modules/.vite-temp") prefix=$repository_root ;;
			*) prefix=$interface_dir ;;
		esac
		mote_remove_managed_path "$generated_path" "$prefix" || cleanup_status=$?
	done
	if [ "$command_status" -ne 0 ]; then
		exit "$command_status"
	fi
	exit "$cleanup_status"
}

trap cleanup_interface_assets_on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
"$@"
