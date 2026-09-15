#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repository_root/scripts/build-lifecycle.sh"
. "$repository_root/scripts/cargo-feature-mode.sh"
app_id="io.github.matt_jenner.mote"
manifest="$repository_root/packaging/flatpak/$app_id.yml"
bundle_dir="${MOTE_FLATPAK_BUNDLE_DIR:-$repository_root/dist/flatpak}"

usage() {
  cat <<'EOF'
usage: scripts/flatpak.sh COMMAND [--no-heic] [FLATPAK_BUILDER_ARGS ...]

  check    validate tools, runtimes, manifest, and desktop metadata
  package  build one bundle and remove all intermediate assets
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
  MOTE_HEIC="$MOTE_HEIC_MODE" flatpak-builder --show-manifest "$manifest" >/dev/null
  desktop-file-validate "$repository_root/packaging/flatpak/$app_id.desktop"
  appstreamcli validate --no-net "$repository_root/packaging/flatpak/$app_id.metainfo.xml"
  (
    cd -- "$repository_root"
    npm run test:flatpak
  )
}

package_bundle() {
  check_requirements
  mote_create_build_dir flatpak
  build_dir="$MOTE_BUILD_DIR/build"
  repo_dir="$MOTE_BUILD_DIR/repo"
  state_dir="$MOTE_BUILD_DIR/state"
  candidate_bundle="$MOTE_BUILD_DIR/$(basename "$(bundle_path)")"
  staging_bundle="$bundle_dir/.Mote.flatpak.next.$$"
  backup_dir="$bundle_dir/.Mote.flatpak.previous.$$"
  promotion_complete=false

  flatpak_cleanup_on_exit() {
    command_status=$?
    trap - EXIT HUP INT TERM
    cleanup_status=0
    if [[ -e "$staging_bundle" ]]; then
      mote_remove_managed_path "$staging_bundle" "$bundle_dir" || cleanup_status=$?
    fi
    if [[ -d "$backup_dir" ]]; then
      if [[ "$promotion_complete" = true ]]; then
        mote_remove_managed_path "$backup_dir" "$bundle_dir" || cleanup_status=$?
      else
        for backed_up in "$backup_dir"/Mote-*.flatpak; do
          [[ -f "$backed_up" ]] || continue
          mv -- "$backed_up" "$bundle_dir/$(basename "$backed_up")" || cleanup_status=$?
        done
        mote_remove_managed_path "$backup_dir" "$bundle_dir" || cleanup_status=$?
      fi
    fi
    mote_cleanup_build_dir || cleanup_status=$?
    if [[ "$command_status" -ne 0 ]]; then
      exit "$command_status"
    fi
    exit "$cleanup_status"
  }

  trap flatpak_cleanup_on_exit EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM

  mkdir -p "$build_dir" "$repo_dir" "$state_dir" "$bundle_dir"
  MOTE_HEIC="$MOTE_HEIC_MODE" flatpak-builder "$@" --force-clean --delete-build-dirs \
    --state-dir="$state_dir" \
    --repo="$repo_dir" \
    "$build_dir" "$manifest"
  flatpak build-bundle \
    --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    --arch="$(flatpak_arch)" \
    "$repo_dir" "$candidate_bundle" "$app_id" stable

  [[ -f "$candidate_bundle" && -s "$candidate_bundle" ]] || {
    echo "Flatpak bundle was not created: $candidate_bundle" >&2
    return 1
  }

  mv -- "$candidate_bundle" "$staging_bundle"
  mkdir -- "$backup_dir"
  for existing_bundle in "$bundle_dir"/Mote-*.flatpak; do
    [[ -f "$existing_bundle" ]] || continue
    mv -- "$existing_bundle" "$backup_dir/$(basename "$existing_bundle")"
  done
  mv -- "$staging_bundle" "$(bundle_path)"
  promotion_complete=true
  mote_remove_managed_path "$backup_dir" "$bundle_dir"
  printf 'bundle: %s\n' "$(bundle_path)"
}

install_bundle() {
  require_command flatpak
  flatpak install --user --noninteractive --or-update -y "$(bundle_path)"
}

command=${1:-help}
if [[ $# -gt 0 ]]; then
  shift
fi
if [[ ${1:-} == --no-heic ]]; then
  mote_disable_heic
  shift
fi
for argument in "$@"; do
  case "$argument" in
    --*heic* | --*heif*) usage >&2; exit 2 ;;
  esac
done

case "$command" in
  check)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    check_requirements
    ;;
  package) package_bundle "$@" ;;
  install)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    install_bundle
    ;;
  run)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    require_command flatpak
    flatpak run "$app_id"
    ;;
  inspect)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    require_command flatpak
    flatpak info "$app_id"
    flatpak info --show-permissions "$app_id"
    ;;
  help|-h|--help)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    usage
    ;;
  *) usage >&2; exit 2 ;;
esac
