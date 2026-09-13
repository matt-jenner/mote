#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
. "$script_dir/build-lifecycle.sh"

mode=${1:-}
case "$mode" in
	native | universal) shift ;;
	*)
		printf '%s\n' "usage: $0 native|universal [TAURI_ARGS ...]" >&2
		exit 2
		;;
esac

output_dir="$repository_root/dist/macos"
interface_dir="$repository_root/apps/interface"
stable_app="$output_dir/Mote.app"
staging_app="$output_dir/.Mote.app.next.$$"
backup_app="$output_dir/.Mote.app.previous.$$"

validate_app() {
	app=$1
	[ -f "$app/Contents/Info.plist" ] || {
		printf '%s\n' "desktop bundle is missing Contents/Info.plist: $app" >&2
		return 1
	}
	[ -d "$app/Contents/MacOS" ] || {
		printf '%s\n' "desktop bundle is missing Contents/MacOS: $app" >&2
		return 1
	}
	executable_count=$(find "$app/Contents/MacOS" -type f -perm -111 -print | wc -l | tr -d '[:space:]')
	[ "$executable_count" = 1 ] || {
		printf '%s\n' "desktop bundle must contain exactly one executable: $app" >&2
		return 1
	}
}

desktop_cleanup_on_exit() {
	command_status=$?
	trap - EXIT HUP INT TERM
	cleanup_status=0
	if [ -e "$staging_app" ]; then
		mote_remove_managed_path "$staging_app" "$output_dir" || cleanup_status=$?
	fi
	if [ -e "$backup_app" ]; then
		if [ ! -e "$stable_app" ]; then
			mv -- "$backup_app" "$stable_app" || cleanup_status=$?
		else
			mote_remove_managed_path "$backup_app" "$output_dir" || cleanup_status=$?
		fi
	fi
	for generated_path in \
		"$interface_dir/dist" \
		"$interface_dir/.vite" \
		"$interface_dir/node_modules/.vite" \
		"$interface_dir/node_modules/.vite-temp" \
		"$interface_dir/tsconfig.tsbuildinfo"; do
		if [ -e "$generated_path" ]; then
			mote_remove_managed_path "$generated_path" "$interface_dir" || cleanup_status=$?
		fi
	done
	mote_cleanup_build_dir || cleanup_status=$?
	if [ "$command_status" -ne 0 ]; then
		exit "$command_status"
	fi
	exit "$cleanup_status"
}

mote_create_build_dir desktop
trap desktop_cleanup_on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR" "$output_dir"

if [ "$mode" = universal ]; then
	npm exec --workspace @photo-viewer/desktop -- tauri build \
		--target universal-apple-darwin --bundles app "$@"
	candidate="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle/macos/Mote.app"
else
	npm exec --workspace @photo-viewer/desktop -- tauri build --bundles app "$@"
	candidate="$CARGO_TARGET_DIR/release/bundle/macos/Mote.app"
fi

validate_app "$candidate"
ditto "$candidate" "$staging_app"
validate_app "$staging_app"

if [ -e "$stable_app" ]; then
	mv -- "$stable_app" "$backup_app"
fi
mv -- "$staging_app" "$stable_app"
if [ -e "$backup_app" ]; then
	mote_remove_managed_path "$backup_app" "$output_dir"
fi

printf '%s\n' "macOS app: $stable_app"
