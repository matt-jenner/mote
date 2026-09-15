#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repository_root/scripts/build-lifecycle.sh"
[[ $# = 1 ]] || { echo 'usage: update-flatpak-sources.sh --offline | /path/to/flatpak-builder-tools' >&2; exit 2; }
output_root="$repository_root/packaging/flatpak/generated"
stage="$repository_root/packaging/flatpak/.generated.next.$$"
backup="$repository_root/packaging/flatpak/.generated.previous.$$"
active=false
complete=false
mote_create_build_dir flatpak-sources
cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  if [[ "$active" = true && "$complete" = false && -d "$backup" ]]; then
    [[ ! -e "$output_root" ]] || mote_remove_managed_path "$output_root" "$repository_root/packaging/flatpak"
    mv "$backup" "$output_root"
  fi
  for candidate in "$stage" "$backup"; do
    [[ ! -e "$candidate" ]] || mote_remove_managed_path "$candidate" "$repository_root/packaging/flatpak"
  done
  mote_cleanup_build_dir
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
candidate_root="$MOTE_BUILD_DIR/generated"
for record in "$output_root"/* "$output_root"/.[!.]* "$output_root"/..?*; do
  [[ -e "$record" || -L "$record" ]] || continue
  case "${record##*/}" in
    cargo-sources.json|node-sources.json|source-lock.json)
      [[ -f "$record" && ! -L "$record" ]] || { echo "unexpected generated record: $record" >&2; exit 1; } ;;
    *) echo "unexpected generated record: $record" >&2; exit 1 ;;
  esac
done
mkdir "$candidate_root"
for record in cargo-sources.json node-sources.json source-lock.json; do
  install -m0644 "$output_root/$record" "$candidate_root/$record"
done
if [[ "$1" != --offline ]]; then
  tools_root="$(cd -- "$1" && pwd)"
  [[ -f "$tools_root/cargo/flatpak-cargo-generator.py" ]]
  command -v flatpak-node-generator >/dev/null
  mkdir -p "$MOTE_BUILD_DIR/apps/desktop" "$MOTE_BUILD_DIR/apps/interface"
  for file in package.json package-lock.json apps/desktop/package.json apps/interface/package.json; do
    install -m0644 "$repository_root/$file" "$MOTE_BUILD_DIR/$file"
  done
  flatpak-node-generator --no-requests-cache \
    --node-sdk-extension org.freedesktop.Sdk.Extension.node24//25.08 \
    --output "$candidate_root/node-sources.json" npm "$MOTE_BUILD_DIR/package-lock.json"
  node --input-type=module - "$candidate_root/source-lock.json" "$(git -C "$tools_root" remote get-url origin)" "$(git -C "$tools_root" rev-parse HEAD)" <<'NODE'
import fs from "node:fs";
const [file, repository, commit] = process.argv.slice(2);
fs.writeFileSync(file, JSON.stringify({ generator: { repository, commit } }));
NODE
fi
node "$repository_root/scripts/flatpak-source-lock.mjs" "$repository_root" "$candidate_root"
mkdir "$stage"
for record in cargo-sources.json node-sources.json source-lock.json; do
  install -m0644 "$candidate_root/$record" "$stage/$record"
done
active=true
mv "$output_root" "$backup"
mv "$stage" "$output_root"
complete=true
printf '%s\n' 'updated Flatpak sources from both Cargo locks, npm lock, and native pins'
