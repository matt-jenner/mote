#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
run_id="photo-viewer-smoke-$(date +%s)-$$"
container_name="${run_id}-app"
network_name="${run_id}-network"
data_volume="${run_id}-data"
cache_volume="${run_id}-cache"
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/${run_id}.XXXXXX")
source_dir="${temporary_root}/photos"
state_dir="${temporary_root}/browser-state"
baseline_metadata="${temporary_root}/source-metadata.before"
baseline_hashes="${temporary_root}/source-hashes.before"
source_mode=0555
cleanup_started=false

cleanup() {
	if [ "$cleanup_started" = true ]; then
		return
	fi
	cleanup_started=true
	trap - HUP INT TERM
	if [ -d "$source_dir" ]; then
		chmod u+rwx "$source_dir" >/dev/null 2>&1 || true
		find "$source_dir" -type d -exec chmod u+rwx {} \; >/dev/null 2>&1 || true
	fi
	podman rm --force "$container_name" >/dev/null 2>&1 || true
	podman network rm "$network_name" >/dev/null 2>&1 || true
	podman volume rm "$data_volume" >/dev/null 2>&1 || true
	podman volume rm "$cache_volume" >/dev/null 2>&1 || true
	case "$temporary_root" in
		"${TMPDIR:-/tmp}"/photo-viewer-smoke-*) rm -rf -- "$temporary_root" ;;
		*) printf '%s\n' "refusing to remove unexpected path: $temporary_root" >&2 ;;
	esac
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

metadata_manifest() {
	destination=$1
	if stat --version >/dev/null 2>&1; then
		find "$source_dir" -type f -exec stat -c '%n|%s|%Y' {} \; | LC_ALL=C sort >"$destination"
	else
		find "$source_dir" -type f -exec stat -f '%N|%z|%m' {} \; | LC_ALL=C sort >"$destination"
	fi
}

hash_manifest() {
	destination=$1
	if command -v shasum >/dev/null 2>&1; then
		find "$source_dir" -type f -exec shasum -a 256 {} \; | LC_ALL=C sort >"$destination"
	else
		find "$source_dir" -type f -exec sha256sum {} \; | LC_ALL=C sort >"$destination"
	fi
}

assert_source_unchanged() {
	label=$1
	current_metadata="${temporary_root}/source-metadata.current"
	current_hashes="${temporary_root}/source-hashes.current"
	metadata_manifest "$current_metadata"
	hash_manifest "$current_hashes"
	if ! cmp -s "$baseline_metadata" "$current_metadata"; then
		printf '%s\n' "source metadata changed after $label" >&2
		diff -u "$baseline_metadata" "$current_metadata" >&2 || true
		exit 1
	fi
	if ! cmp -s "$baseline_hashes" "$current_hashes"; then
		printf '%s\n' "source bytes changed after $label" >&2
		diff -u "$baseline_hashes" "$current_hashes" >&2 || true
		exit 1
	fi
	printf '%s\n' "source manifests unchanged after $label"
}

wait_for_health() {
	base_url=$1
	attempt=0
	while [ "$attempt" -lt 60 ]; do
		if curl --fail --silent --show-error "${base_url}/healthz" >"${temporary_root}/health.json" 2>/dev/null; then
			return 0
		fi
		attempt=$((attempt + 1))
		sleep 1
	done
	printf '%s\n' "health check did not pass within 60 seconds" >&2
	podman logs "$container_name" >&2 || true
	return 1
}

start_container() {
	host_port=${1:-}
	if [ -n "$host_port" ]; then
		publish="127.0.0.1:${host_port}:8080"
	else
		publish="127.0.0.1::8080"
	fi
	podman run --detach \
		--name "$container_name" \
		--network "$network_name" \
		--read-only \
		--security-opt no-new-privileges \
		--publish "$publish" \
		--env PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
		--env PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
		--env PHOTO_VIEWER_SOURCE_ROOT=/photos \
		--env PHOTO_VIEWER_BIND=0.0.0.0:8080 \
		--volume "${source_dir}:/photos:ro,Z" \
		--volume "${data_volume}:/var/lib/photo-viewer:U" \
		--volume "${cache_volume}:/var/cache/photo-viewer:U" \
		localhost/photo-viewer:dev >/dev/null
	port_mapping=$(podman port "$container_name" 8080/tcp | sed -n '1p')
	case "$port_mapping" in
		127.0.0.1:*) ;;
		*) printf '%s\n' "unexpected loopback mapping: $port_mapping" >&2; exit 1 ;;
	esac
	mapped_port=${port_mapping##*:}
	base_url="http://127.0.0.1:${mapped_port}"
	wait_for_health "$base_url"
	printf '%s\n' "$mapped_port"
}

run_browser_phase() {
	phase_name=$1
	PHOTO_VIEWER_BASE_URL="$base_url" \
	PHOTO_VIEWER_PHASE="$phase_name" \
	PHOTO_VIEWER_STATE_DIR="$state_dir" \
		npm run test:hosted
}

probe_rejected() {
	label=$1
	request_target=$2
	status=$(curl --path-as-is --silent --output "${temporary_root}/probe-body" --write-out '%{http_code}' "${base_url}${request_target}")
	case "$status" in
		4??) printf '%s\n' "rejected $label with HTTP $status" ;;
		*) printf '%s\n' "$label probe was not rejected: HTTP $status" >&2; exit 1 ;;
	esac
}

wait_for_degraded_health() {
	selection_id=$(node -e 'const fs=require("node:fs"); const value=JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(value.a.selectionId);' "${state_dir}/hosted-record.json")
	attempt=0
	while [ "$attempt" -lt 60 ]; do
		curl --silent --output "${temporary_root}/offline-folder.json" \
			"${base_url}/api/v1/folders?path=" || true
		curl --silent --output "${temporary_root}/offline-wall.json" \
			"${base_url}/api/v1/selections/${selection_id}/wall?scope=currentFolder&direction=oldestFirst&limit=100" || true
		if curl --fail --silent --show-error "${base_url}/healthz" >"${temporary_root}/health-offline.json" 2>/dev/null \
			&& grep -Eq '"status"[[:space:]]*:[[:space:]]*"degraded"' "${temporary_root}/health-offline.json"; then
			return 0
		fi
		attempt=$((attempt + 1))
		sleep 1
	done
	printf '%s\n' "health did not become degraded after source loss" >&2
	cat "${temporary_root}/health-offline.json" >&2 2>/dev/null || true
	return 1
}

command -v podman >/dev/null 2>&1 || { printf '%s\n' "podman is required" >&2; exit 1; }
command -v curl >/dev/null 2>&1 || { printf '%s\n' "curl is required" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { printf '%s\n' "node is required" >&2; exit 1; }

mkdir -p "$source_dir/A/child" "$source_dir/B" "$state_dir"
cp "$project_dir/apps/interface/public/demo-photos/mountain.jpg" "$source_dir/A/a-01.jpg"
cp "$project_dir/apps/interface/public/demo-photos/coast.jpg" "$source_dir/A/a-02.jpg"
cp "$project_dir/apps/interface/public/demo-photos/forest.jpg" "$source_dir/A/child/a-child-uncached.jpg"
cp "$project_dir/apps/interface/public/demo-photos/interior.jpg" "$source_dir/B/b-01.jpg"
cp "$project_dir/apps/interface/public/demo-photos/portrait.jpg" "$source_dir/B/b-02.jpg"
touch -t 202001010101 "$source_dir/A/a-01.jpg"
touch -t 202001020101 "$source_dir/A/a-02.jpg"
touch -t 202001030101 "$source_dir/A/child/a-child-uncached.jpg"
touch -t 202001050101 "$source_dir/B/b-01.jpg"
touch -t 202001040101 "$source_dir/B/b-02.jpg"
ln -s /etc "$source_dir/Escape"
find "$source_dir" -type f -exec chmod 0444 {} \;
find "$source_dir" -type d -exec chmod "$source_mode" {} \;
metadata_manifest "$baseline_metadata"
hash_manifest "$baseline_hashes"

cd "$project_dir"
printf '%s\n' "building localhost/photo-viewer:dev"
podman build --tag localhost/photo-viewer:dev --file Containerfile .

podman network create "$network_name" >/dev/null
podman volume create "$data_volume" >/dev/null
podman volume create "$cache_volume" >/dev/null

mapped_port=$(start_container)
base_url="http://127.0.0.1:${mapped_port}"
printf '%s\n' "hosted service: $base_url"
assert_source_unchanged "initial startup"
run_browser_phase beforeRestart
assert_source_unchanged "two-browser browse, scan, sort, scope, and viewer phase"

podman stop --time 10 "$container_name" >/dev/null
podman rm "$container_name" >/dev/null
mapped_port=$(start_container "$mapped_port")
base_url="http://127.0.0.1:${mapped_port}"
assert_source_unchanged "retained-volume restart"
run_browser_phase afterRestart
assert_source_unchanged "browser-state and ETag restoration"

chmod 000 "$source_dir"
wait_for_degraded_health
run_browser_phase offline
chmod "$source_mode" "$source_dir"
assert_source_unchanged "offline cached wall and viewer reads"

probe_rejected "encoded traversal" "/api/v1/folders?path=%2e%2e"
probe_rejected "absolute folder" "/api/v1/folders?path=%2Fetc"
probe_rejected "NUL folder" "/api/v1/folders?path=%00"
over_limit=$(node -e 'process.stdout.write("x".repeat(4097))')
probe_rejected "over-limit folder" "/api/v1/folders?path=${over_limit}"
probe_rejected "repeated folder parameter" "/api/v1/folders?path=A&path=B"
probe_rejected "file-as-folder" "/api/v1/folders?path=A%2Fa-01.jpg"
probe_rejected "symlink escape" "/api/v1/folders?path=Escape"
assert_source_unchanged "traversal and limit probes"

printf '%s\n' "hosted smoke passed: independent browsers, restart restoration, stable ETags, offline cache, request rejection, unchanged source"
