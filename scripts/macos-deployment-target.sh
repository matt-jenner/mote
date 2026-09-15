#!/bin/sh
set -eu

# Tauri's app contract is authoritative for bundled native dependencies too.
deployment_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
deployment_target=$(plutil -extract bundle.macOS.minimumSystemVersion raw -o - \
	"$deployment_script_dir/../apps/desktop/src-tauri/tauri.conf.json")
printf '%s\n' "$deployment_target" | grep -E '^[0-9]+(\.[0-9]+)?(\.[0-9]+)?$' >/dev/null || {
	printf '%s\n' 'invalid app macOS deployment target' >&2
	exit 1
}
printf '%s\n' "$deployment_target"
