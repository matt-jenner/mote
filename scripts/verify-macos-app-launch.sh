#!/bin/sh
set -eu

[ "$#" = 1 ] || {
	printf '%s\n' "usage: $0 APP" >&2
	exit 2
}

app=$1
plist="$app/Contents/Info.plist"
[ -f "$plist" ] || {
	printf '%s\n' "macOS app is missing Contents/Info.plist: $app" >&2
	exit 1
}

executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")
binary="$app/Contents/MacOS/$executable"
[ -x "$binary" ] || {
	printf '%s\n' "macOS app executable is missing or not executable: $binary" >&2
	exit 1
}

log=$(mktemp "${TMPDIR:-/tmp}/mote-macos-launch.XXXXXX")
pid=

read_process_state() {
	raw_state=$(ps -p "$pid" -o state= 2>/dev/null) || return 1
	printf '%s' "$raw_state" | tr -d '[:space:]'
}

terminate_process() {
	[ -n "$pid" ] || return 0
	if kill -0 "$pid" 2>/dev/null; then
		kill -TERM "$pid" 2>/dev/null || true
		attempt=0
		while kill -0 "$pid" 2>/dev/null; do
			process_state=$(read_process_state || true)
			case "$process_state" in
				Z*) break ;;
			esac
			attempt=$((attempt + 1))
			if [ "$attempt" -ge 20 ]; then
				kill -KILL "$pid" 2>/dev/null || true
				break
			fi
			sleep 0.1
		done
	fi
	wait "$pid" 2>/dev/null || true
	pid=
}

cleanup() {
	status=$?
	trap - EXIT HUP INT TERM
	terminate_process
	rm -f -- "$log"
	exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

PHOTO_VIEWER_PROFILE="release-launch-check-$$" "$binary" >"$log" 2>&1 &
pid=$!
sleep "${MOTE_MACOS_LAUNCH_SECONDS:-3}"

process_state=$(read_process_state || true)
case "$process_state" in
	Z*) process_exited=true ;;
	'')
		if kill -0 "$pid" 2>/dev/null; then
			process_exited=false
		else
			process_exited=true
		fi
		;;
	*) process_exited=false ;;
esac
if [ "$process_exited" = true ]; then
	exit_status=0
	wait "$pid" || exit_status=$?
	pid=
	cat "$log" >&2
	printf '%s\n' "macOS app exited during startup with status $exit_status" >&2
	exit 1
fi

terminate_process
printf '%s\n' "macOS app launch check passed: $app"
