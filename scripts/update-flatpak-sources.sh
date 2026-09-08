#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/update-flatpak-sources.sh /path/to/flatpak-builder-tools" >&2
  exit 2
fi

tools_root="$(cd -- "$1" && pwd)"
cargo_generator="$tools_root/cargo/flatpak-cargo-generator.py"
output_root="$repository_root/packaging/flatpak/generated"

command -v flatpak-node-generator >/dev/null || {
  echo "flatpak-node-generator is required" >&2
  exit 1
}
[[ -f "$cargo_generator" ]] || {
  echo "flatpak-cargo-generator.py was not found below $tools_root" >&2
  exit 1
}

generator_commit="$(git -C "$tools_root" rev-parse HEAD)"
generator_repository="$(git -C "$tools_root" remote get-url origin)"
temporary_root="$(mktemp -d)"
cleanup() {
  rm -rf -- "$temporary_root"
}
trap cleanup EXIT

mkdir -p "$temporary_root/apps/desktop" "$temporary_root/apps/interface" "$output_root"
install -m 0644 "$repository_root/package.json" "$temporary_root/package.json"
install -m 0644 "$repository_root/package-lock.json" "$temporary_root/package-lock.json"
install -m 0644 "$repository_root/apps/desktop/package.json" "$temporary_root/apps/desktop/package.json"
install -m 0644 "$repository_root/apps/interface/package.json" "$temporary_root/apps/interface/package.json"

flatpak-node-generator \
  --no-requests-cache \
  --node-sdk-extension org.freedesktop.Sdk.Extension.node24//25.08 \
  --output "$output_root/node-sources.json" \
  npm "$temporary_root/package-lock.json"

python3 "$cargo_generator" \
  "$repository_root/apps/desktop/src-tauri/Cargo.lock" \
  --output "$output_root/cargo-sources.json"

node --input-type=module - "$repository_root" "$generator_repository" "$generator_commit" <<'NODE'
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const [root, repository, commit] = process.argv.slice(2);
const digest = (filename) => createHash("sha256").update(readFileSync(path.join(root, filename))).digest("hex");
const lock = {
  generator: { repository, commit },
  lockfiles: {
    "package-lock.json": digest("package-lock.json"),
    "apps/desktop/src-tauri/Cargo.lock": digest("apps/desktop/src-tauri/Cargo.lock"),
  },
};
writeFileSync(
  path.join(root, "packaging/flatpak/generated/source-lock.json"),
  `${JSON.stringify(lock, null, 2)}\n`,
);
NODE
