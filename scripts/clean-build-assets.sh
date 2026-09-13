#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
. "$script_dir/build-lifecycle.sh"
. "$script_dir/hosted-smoke-assets.sh"

remove_generated_path() {
	candidate=$1
	prefix=$2
	if [ -e "$candidate" ]; then
		mote_remove_managed_path "$candidate" "$prefix"
		printf '%s\n' "removed: $candidate"
	else
		printf '%s\n' "absent: $candidate"
	fi
}

clean_checkout() {
	checkout=$1
	case "$checkout" in
		"$repository_root") ;;
		"$repository_root"/.worktrees/*)
			[ "$(dirname -- "$checkout")" = "$repository_root/.worktrees" ] || {
				printf '%s\n' "refusing nested worktree path: $checkout" >&2
				return 1
			}
			;;
		*)
			printf '%s\n' "refusing checkout outside repository: $checkout" >&2
			return 1
			;;
	esac

	remove_generated_path "$checkout/target" "$checkout"
	remove_generated_path "$checkout/apps/desktop/src-tauri/target" "$checkout"
	remove_generated_path "$checkout/apps/interface/dist" "$checkout"
	remove_generated_path "$checkout/apps/interface/.vite" "$checkout"
	remove_generated_path "$checkout/node_modules/.vite" "$checkout"
	remove_generated_path "$checkout/node_modules/.vite-temp" "$checkout"
	remove_generated_path "$checkout/apps/interface/node_modules/.vite" "$checkout"
	remove_generated_path "$checkout/apps/interface/node_modules/.vite-temp" "$checkout"
	remove_generated_path "$checkout/apps/interface/tsconfig.tsbuildinfo" "$checkout"
}

clean_checkout "$repository_root"
if [ -d "$repository_root/.worktrees" ]; then
	for checkout in "$repository_root"/.worktrees/*; do
		[ -d "$checkout" ] || continue
		clean_checkout "$checkout"
	done
fi

remove_generated_path "$repository_root/build/flatpak" "$repository_root"
remove_generated_path "$repository_root/.flatpak-builder" "$repository_root"
remove_generated_path "$repository_root/dist/flatpak/repo" "$repository_root"
mote_reap_stale_builds

container_engine=${CONTAINER_ENGINE:-podman}
if command -v "$container_engine" >/dev/null 2>&1 && "$container_engine" info >/dev/null 2>&1; then
	mote_reap_stale_smoke_assets "$container_engine"
else
	printf '%s\n' "skipping container cleanup: $container_engine is unavailable"
fi
