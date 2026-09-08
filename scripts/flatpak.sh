#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
app_id="io.github.matt_jenner.mote"
manifest="$repository_root/packaging/flatpak/$app_id.yml"
build_dir="${MOTE_FLATPAK_BUILD_DIR:-$repository_root/build/flatpak}"
repo_dir="${MOTE_FLATPAK_REPO_DIR:-$repository_root/dist/flatpak/repo}"
bundle_dir="${MOTE_FLATPAK_BUNDLE_DIR:-$repository_root/dist/flatpak}"

usage() {
  cat <<'EOF'
usage: scripts/flatpak.sh COMMAND

  check    validate tools, runtimes, manifest, and desktop metadata
  build    build and export Mote to the repository-local Flatpak repository
  bundle   create the single-file Flatpak bundle from that repository
  package  run build followed by bundle
  install  install or update the bundle for the current user
  run      launch the installed Mote Flatpak
  inspect  show installed application metadata and sandbox permissions
  help     show this help
EOF
}

require_command() {
  command -v "$1" >/dev/null || {
    echo "required command not found: $1" >&2
    exit 1
  }
}

flatpak_arch() {
  if [[ -n "${MOTE_FLATPAK_ARCH:-}" ]]; then
    printf '%s\n' "$MOTE_FLATPAK_ARCH"
  else
    flatpak --default-arch
  fi
}

app_version() {
  node -p 'require(process.argv[1]).version' \
    "$repository_root/apps/desktop/src-tauri/tauri.conf.json"
}

bundle_path() {
  printf '%s/Mote-%s-%s.flatpak\n' "$bundle_dir" "$(app_version)" "$(flatpak_arch)"
}

check_requirements() {
  for command in flatpak flatpak-builder node desktop-file-validate appstreamcli; do
    require_command "$command"
  done
  for runtime in \
    org.gnome.Platform//49 \
    org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.node24//25.08 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08; do
    flatpak info "$runtime" >/dev/null || {
      echo "required Flatpak runtime is not installed: $runtime" >&2
      exit 1
    }
  done
  flatpak-builder --show-manifest "$manifest" >/dev/null
  desktop-file-validate "$repository_root/packaging/flatpak/$app_id.desktop"
  appstreamcli validate --no-net "$repository_root/packaging/flatpak/$app_id.metainfo.xml"
  (
    cd -- "$repository_root"
    npm run test:flatpak
  )
}

build() {
  check_requirements
  mkdir -p "$build_dir" "$repo_dir" "$bundle_dir"
  flatpak-builder --force-clean --repo="$repo_dir" "$build_dir" "$manifest"
}

bundle() {
  require_command flatpak
  require_command node
  mkdir -p "$bundle_dir"
  flatpak build-bundle \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    --arch="$(flatpak_arch)" \
    "$repo_dir" "$(bundle_path)" "$app_id" stable
  printf 'bundle: %s\n' "$(bundle_path)"
}

install_bundle() {
  require_command flatpak
  flatpak install --user --noninteractive --or-update -y "$(bundle_path)"
}

case "${1:-help}" in
  check) check_requirements ;;
  build) build ;;
  bundle) bundle ;;
  package) build; bundle ;;
  install) install_bundle ;;
  run) require_command flatpak; flatpak run "$app_id" ;;
  inspect)
    require_command flatpak
    flatpak info "$app_id"
    flatpak info --show-permissions "$app_id"
    ;;
  help|-h|--help) usage ;;
  *) usage >&2; exit 2 ;;
esac
