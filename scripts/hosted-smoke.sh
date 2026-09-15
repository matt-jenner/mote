#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_dir/scripts/hosted-smoke-assets.sh"
. "$project_dir/scripts/cargo-feature-mode.sh"

usage() {
	printf '%s\n' "usage: $0 [--no-heic] [CONTAINER_BUILD_ARGS ...]" >&2
}

if [ "${1:-}" = --no-heic ]; then
	mote_disable_heic
	shift
fi
for argument in "$@"; do
	case "$argument" in
		--*heic* | --*heif*)
			usage
			exit 2
			;;
	esac
done

container_engine=${CONTAINER_ENGINE:-podman}
container_engine_name=${container_engine##*/}
runtime_uid=$(id -u)
runtime_gid=$(id -g)
run_id="photo-viewer-smoke-$(date +%s)-$$"
container_name="${run_id}-app"
network_name="${run_id}-network"
image_name="localhost/mote-smoke:${run_id}"
asset_label="io.github.matt-jenner.mote.asset=hosted-smoke"
owner_label="io.github.matt-jenner.mote.owner-pid=$$"
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/${run_id}.XXXXXX")
CARGO_TARGET_DIR="$temporary_root/cargo-target"
export CARGO_TARGET_DIR
runtime_root="${project_dir}/runtime/${run_id}"
source_dir="${temporary_root}/photos"
state_dir="${temporary_root}/browser-state"
data_dir="${runtime_root}/data"
cache_dir="${runtime_root}/cache"
baseline_metadata="${temporary_root}/source-metadata.before"
baseline_hashes="${temporary_root}/source-hashes.before"
source_mode=0555
cleanup_started=false

cleanup() {
	exit_status=$?
	if [ "$cleanup_started" = true ]; then
		return
	fi
	cleanup_started=true
	trap - EXIT HUP INT TERM
	if [ "$exit_status" -ne 0 ]; then
		printf '%s\n' "hosted smoke failed; container log follows" >&2
		"$container_engine" logs "$container_name" >&2 2>/dev/null || true
	fi
	if [ -d "$source_dir" ]; then
		chmod u+rwx "$source_dir" >/dev/null 2>&1 || true
		find "$source_dir" -type d -exec chmod u+rwx {} \; >/dev/null 2>&1 || true
	fi
	"$container_engine" rm --force "$container_name" >/dev/null 2>&1 || true
	"$container_engine" network rm "$network_name" >/dev/null 2>&1 || true
	if ! "$container_engine" image rm "$image_name" >/dev/null 2>&1; then
		printf '%s\n' "retained smoke image still referenced by a container: $image_name" >&2
		[ "$exit_status" -ne 0 ] || exit_status=1
	fi
	case "$temporary_root" in
		"${TMPDIR:-/tmp}"/photo-viewer-smoke-*) rm -rf -- "$temporary_root" ;;
		*) printf '%s\n' "refusing to remove unexpected path: $temporary_root" >&2 ;;
	esac
	case "$runtime_root" in
		"${project_dir}"/runtime/photo-viewer-smoke-*) rm -rf -- "$runtime_root" ;;
		*) printf '%s\n' "refusing to remove unexpected path: $runtime_root" >&2 ;;
	esac
	exit "$exit_status"
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

assert_external_storage() {
	if [ ! -f "${data_dir}/catalog.sqlite" ]; then
		printf '%s\n' "catalogue was not written to host path: $data_dir" >&2
		exit 1
	fi
	if ! find "$cache_dir" -type f -name '*.jpg' -print -quit | grep -q .; then
		printf '%s\n' "JPEG derivative cache was not written to host path: $cache_dir" >&2
		exit 1
	fi
	printf '%s\n' "catalogue and derivative cache are present under $runtime_root"
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
	"$container_engine" logs "$container_name" >&2 || true
	return 1
}

start_container() {
	host_port=${1:-}
	set --
	case "$container_engine_name" in
		podman | podman-remote) set -- --userns=keep-id ;;
	esac
	if [ -n "$host_port" ]; then
		publish="127.0.0.1:${host_port}:8080"
	else
		publish="127.0.0.1::8080"
	fi
	"$container_engine" run --detach \
		"$@" \
		--name "$container_name" \
		--label "$asset_label" \
		--label "$owner_label" \
		--network "$network_name" \
		--user "${runtime_uid}:${runtime_gid}" \
		--read-only \
		--security-opt no-new-privileges \
		--publish "$publish" \
		--env PHOTO_VIEWER_DATA_DIR=/var/lib/photo-viewer \
		--env PHOTO_VIEWER_CACHE_DIR=/var/cache/photo-viewer \
		--env PHOTO_VIEWER_SOURCE_ROOT=/photos \
		--env PHOTO_VIEWER_BIND=0.0.0.0:8080 \
		--volume "${source_dir}:/photos:ro,Z" \
		--volume "${data_dir}:/var/lib/photo-viewer:Z" \
		--volume "${cache_dir}:/var/cache/photo-viewer:Z" \
		"$image_name" >/dev/null
	port_mapping=$("$container_engine" port "$container_name" 8080/tcp | sed -n '1p')
	case "$port_mapping" in
		127.0.0.1:*) ;;
		*) printf '%s\n' "unexpected loopback mapping: $port_mapping" >&2; exit 1 ;;
	esac
	mapped_port=${port_mapping##*:}
	base_url="http://127.0.0.1:${mapped_port}"
	wait_for_health "$base_url"
	printf '%s\n' "$mapped_port"
}

assert_supported_engine() {
	if [ "$container_engine_name" = docker ]; then
		security_options=$("$container_engine" info --format '{{json .SecurityOptions}}')
		case "$security_options" in
			*rootless* | *name=userns*)
				printf '%s\n' "Docker user-namespace remapping is not supported by this bind-mount smoke test" >&2
				exit 1
				;;
		esac
	fi
}

assert_image_runtime_user() {
	configured_user=$("$container_engine" image inspect \
		--format '{{.Config.User}}' "$image_name")
	if [ "$configured_user" != "10001:10001" ]; then
		printf '%s\n' "unexpected image runtime user: $configured_user" >&2
		exit 1
	fi
	printf '%s\n' "image defaults to runtime user $configured_user"
}

run_browser_phase() {
	phase_name=$1
	PHOTO_VIEWER_BASE_URL="$base_url" \
	PHOTO_VIEWER_PHASE="$phase_name" \
	PHOTO_VIEWER_STATE_DIR="$state_dir" \
	PHOTO_VIEWER_WEB_ROOT="$browser_web_root" \
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

command -v "$container_engine" >/dev/null 2>&1 || { printf '%s\n' "$container_engine is required" >&2; exit 1; }
command -v curl >/dev/null 2>&1 || { printf '%s\n' "curl is required" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { printf '%s\n' "node is required" >&2; exit 1; }
assert_supported_engine
mote_reap_stale_smoke_assets "$container_engine"

mkdir -p \
	"$source_dir/A/child" \
	"$source_dir/B" \
	"$source_dir/Nested/Album/grandchild" \
	"$state_dir" \
	"$data_dir" \
	"$cache_dir"
chmod 0777 "$data_dir" "$cache_dir"
cp "$project_dir/apps/interface/public/demo-photos/mountain.jpg" "$source_dir/A/a-01.jpg"
cp "$project_dir/apps/interface/public/demo-photos/coast.jpg" "$source_dir/A/a-02.jpg"
cp "$project_dir/apps/interface/public/demo-photos/forest.jpg" "$source_dir/A/child/a-child-uncached.jpg"
cp "$project_dir/apps/interface/public/demo-photos/interior.jpg" "$source_dir/B/b-01.jpg"
cp "$project_dir/apps/interface/public/demo-photos/portrait.jpg" "$source_dir/B/b-02.jpg"
cp "$project_dir/apps/interface/public/demo-photos/city.jpg" "$source_dir/Nested/Album/album-current.jpg"
cp "$project_dir/apps/interface/public/demo-photos/mountain.jpg" "$source_dir/Nested/Album/grandchild/album-descendant.jpg"
touch -t 202001010101 "$source_dir/A/a-01.jpg"
touch -t 202001020101 "$source_dir/A/a-02.jpg"
touch -t 202001030101 "$source_dir/A/child/a-child-uncached.jpg"
touch -t 202001050101 "$source_dir/B/b-01.jpg"
touch -t 202001040101 "$source_dir/B/b-02.jpg"
touch -t 202001060101 "$source_dir/Nested/Album/album-current.jpg"
touch -t 202001070101 "$source_dir/Nested/Album/grandchild/album-descendant.jpg"
ln -s /etc "$source_dir/Escape"
find "$source_dir" -type f -exec chmod 0444 {} \;
find "$source_dir" -type d -exec chmod "$source_mode" {} \;
metadata_manifest "$baseline_metadata"
hash_manifest "$baseline_hashes"

cd "$project_dir"
printf '%s\n' "building $image_name with $container_engine"
set -- --rm --force-rm "$@"
case "$container_engine_name" in
	podman | podman-remote) set -- "$@" --layers=false ;;
esac
"$container_engine" build "$@" \
	--build-arg MOTE_HEIC="$MOTE_HEIC_MODE" \
	--label "$asset_label" \
	--label "$owner_label" \
	--tag "$image_name" \
	--file Containerfile .
assert_image_runtime_user

browser_web_root="$temporary_root/web"
sh "$project_dir/scripts/with-ephemeral-interface-assets.sh" \
	npm run web:build -- --outDir "$browser_web_root"

"$container_engine" network create \
	--label "$asset_label" \
	--label "$owner_label" \
	"$network_name" >/dev/null

mapped_port=$(start_container)
base_url="http://127.0.0.1:${mapped_port}"
printf '%s\n' "hosted service: $base_url"
assert_source_unchanged "initial startup"
run_browser_phase beforeRestart
assert_external_storage
assert_source_unchanged "two-browser browse, scan, sort, scope, and viewer phase"

"$container_engine" stop -t 10 "$container_name" >/dev/null
"$container_engine" rm "$container_name" >/dev/null
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

printf '%s\n' "hosted smoke passed: host-mounted catalogue and cache, nested folder selection, scoped browsing, independent browsers, restart restoration, stable ETags, offline cache, request rejection, unchanged source"
