#!/bin/sh

mote_build_tmp_root() {
	raw_root=${MOTE_BUILD_TMP_ROOT:-${TMPDIR:-/tmp}}
	case "$raw_root" in
		'' | '/')
			printf '%s\n' "refusing unsafe Mote temporary root: $raw_root" >&2
			return 1
			;;
	esac
	[ -d "$raw_root" ] || {
		printf '%s\n' "Mote temporary root does not exist: $raw_root" >&2
		return 1
	}
	canonical_root=$(CDPATH= cd -- "$raw_root" && pwd -P)
	if [ -n "${HOME:-}" ]; then
		canonical_home=$(CDPATH= cd -- "$HOME" && pwd -P)
		[ "$canonical_root" != "$canonical_home" ] || {
			printf '%s\n' "refusing home directory as Mote temporary root" >&2
			return 1
		}
	fi
	printf '%s\n' "$canonical_root"
}

mote_remove_managed_path() {
	candidate=$1
	prefix=$2
	case "$candidate" in
		"$prefix"/*) ;;
		*)
			printf '%s\n' "refusing to remove unmanaged path: $candidate" >&2
			return 1
			;;
	esac
	[ -n "$candidate" ] && [ "$candidate" != "/" ] && [ "$candidate" != "$prefix" ] || {
		printf '%s\n' "refusing to remove unsafe path: $candidate" >&2
		return 1
	}
	rm -rf -- "$candidate"
}

mote_reap_stale_builds() {
	root=$(mote_build_tmp_root) || return 1
	for candidate in "$root"/mote-build-*; do
		[ -d "$candidate" ] || continue
		owner_file="$candidate/.mote-owner-pid"
		if ! IFS= read -r owner_pid <"$owner_file" 2>/dev/null; then
			printf '%s\n' "skipping Mote build with missing owner: $candidate" >&2
			continue
		fi
		case "$owner_pid" in
			'' | *[!0-9]*)
				printf '%s\n' "skipping Mote build with invalid owner: $candidate" >&2
				continue
				;;
		esac
		if kill -0 "$owner_pid" 2>/dev/null; then
			continue
		fi
		mote_remove_managed_path "$candidate" "$root" || return 1
	done
}

mote_create_build_dir() {
	kind=$1
	case "$kind" in
		'' | *[!a-z0-9-]*)
			printf '%s\n' "invalid Mote build kind: $kind" >&2
			return 1
			;;
	esac
	MOTE_BUILD_TMP_ROOT=$(mote_build_tmp_root) || return 1
	export MOTE_BUILD_TMP_ROOT
	mote_reap_stale_builds || return 1
	MOTE_BUILD_DIR=$(mktemp -d "$MOTE_BUILD_TMP_ROOT/mote-build-$kind.XXXXXX") || return 1
	export MOTE_BUILD_DIR
	printf '%s\n' "$$" >"$MOTE_BUILD_DIR/.mote-owner-pid"
}

mote_cleanup_build_dir() {
	[ -n "${MOTE_BUILD_DIR:-}" ] || return 0
	directory=$MOTE_BUILD_DIR
	MOTE_BUILD_DIR=
	export MOTE_BUILD_DIR
	mote_remove_managed_path "$directory" "$MOTE_BUILD_TMP_ROOT"
}

mote_cleanup_on_exit() {
	command_status=$?
	trap - EXIT HUP INT TERM
	cleanup_status=0
	mote_cleanup_build_dir || cleanup_status=$?
	if [ "$command_status" -ne 0 ]; then
		exit "$command_status"
	fi
	exit "$cleanup_status"
}

mote_install_cleanup_traps() {
	trap mote_cleanup_on_exit EXIT
	trap 'exit 129' HUP
	trap 'exit 130' INT
	trap 'exit 143' TERM
}
