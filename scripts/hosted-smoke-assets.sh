#!/bin/sh

mote_smoke_run_is_stale() {
	run_id=$1
	case "$run_id" in
		photo-viewer-smoke-*-*) ;;
		*) return 1 ;;
	esac
	owner_pid=${run_id##*-}
	case "$owner_pid" in
		'' | *[!0-9]*) return 1 ;;
	esac
	! kill -0 "$owner_pid" 2>/dev/null
}

mote_reap_stale_smoke_assets() {
	engine=$1
	asset_label="io.github.matt-jenner.mote.asset=hosted-smoke"

	"$engine" ps -a --filter "label=$asset_label" --format '{{.Names}}' 2>/dev/null |
		while IFS= read -r container_name; do
			case "$container_name" in
				photo-viewer-smoke-*-*-app) ;;
				*) continue ;;
			esac
			run_id=${container_name%-app}
			mote_smoke_run_is_stale "$run_id" || continue
			running=$("$engine" container inspect --format '{{.State.Running}}' "$container_name" 2>/dev/null || printf 'unknown')
			[ "$running" = false ] || [ -z "$running" ] || continue
			if "$engine" rm "$container_name" >/dev/null 2>&1; then
				printf '%s\n' "removed stale smoke container: $container_name"
			fi
		done

	"$engine" network ls --filter "label=$asset_label" --format '{{.Name}}' 2>/dev/null |
		while IFS= read -r network_name; do
			case "$network_name" in
				photo-viewer-smoke-*-*-network) ;;
				*) continue ;;
			esac
			run_id=${network_name%-network}
			mote_smoke_run_is_stale "$run_id" || continue
			if "$engine" network rm "$network_name" >/dev/null 2>&1; then
				printf '%s\n' "removed stale smoke network: $network_name"
			fi
		done

	"$engine" images --filter "label=$asset_label" --format '{{.Repository}}:{{.Tag}}' 2>/dev/null |
		while IFS= read -r image_name; do
			case "$image_name" in
				localhost/mote-smoke:photo-viewer-smoke-*-*) ;;
				*) continue ;;
			esac
			run_id=${image_name#localhost/mote-smoke:}
			mote_smoke_run_is_stale "$run_id" || continue
			if "$engine" image rm "$image_name" >/dev/null 2>&1; then
				printf '%s\n' "removed stale smoke image: $image_name"
			fi
		done
}
