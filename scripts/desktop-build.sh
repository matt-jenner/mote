#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
. "$script_dir/build-lifecycle.sh"
. "$script_dir/cargo-feature-mode.sh"

usage() {
	printf '%s\n' "usage: $0 native|universal [--no-heic] [TAURI_ARGS ...]" >&2
}

mode=${1:-}
case "$mode" in
	native | universal) shift ;;
	*)
		usage
		exit 2
		;;
esac

if [ "${1:-}" = --no-heic ]; then
	mote_disable_heic
	shift
fi
for argument in "$@"; do
	if mote_is_invalid_heic_flag "$argument"; then
		usage
		exit 2
	fi
	case "$argument" in
		-f* | --all-features | --features | --features=* | --no-default-features)
			usage
			exit 2
			;;
	esac
done
case "$MOTE_CARGO_FEATURE_ARGS" in
	'') ;;
	'--no-default-features --features mote-defaults')
		set -- --no-default-features --features mote-defaults "$@"
		;;
	*)
		printf '%s\n' "unsupported Mote Cargo feature mode" >&2
		exit 2
		;;
esac

output_dir="$repository_root/dist/macos"
interface_dir="$repository_root/apps/interface"
stable_app="$output_dir/Mote.app"
staging_app="$output_dir/.Mote.app.next.$$"
backup_app="$output_dir/.Mote.app.previous.$$"

validate_app() {
	app=$1
	[ -f "$app/Contents/Info.plist" ] || {
		printf '%s\n' "desktop bundle is missing Contents/Info.plist: $app" >&2
		return 1
	}
	[ -d "$app/Contents/MacOS" ] || {
		printf '%s\n' "desktop bundle is missing Contents/MacOS: $app" >&2
		return 1
	}
	executable_count=$(find "$app/Contents/MacOS" -type f -perm -111 -print | wc -l | tr -d '[:space:]')
	[ "$executable_count" = 1 ] || {
		printf '%s\n' "desktop bundle must contain exactly one executable: $app" >&2
		return 1
	}
}

desktop_cleanup_on_exit() {
	command_status=$?
	trap - EXIT HUP INT TERM
	cleanup_status=0
	if [ -e "$staging_app" ]; then
		mote_remove_managed_path "$staging_app" "$output_dir" || cleanup_status=$?
	fi
	if [ -e "$backup_app" ]; then
		if [ ! -e "$stable_app" ]; then
			mv -- "$backup_app" "$stable_app" || cleanup_status=$?
		else
			mote_remove_managed_path "$backup_app" "$output_dir" || cleanup_status=$?
		fi
	fi
	for generated_path in \
		"$interface_dir/dist" \
		"$interface_dir/.vite" \
		"$interface_dir/node_modules/.vite" \
		"$interface_dir/node_modules/.vite-temp" \
		"$interface_dir/tsconfig.tsbuildinfo"; do
		if [ -e "$generated_path" ]; then
			mote_remove_managed_path "$generated_path" "$interface_dir" || cleanup_status=$?
		fi
	done
	mote_cleanup_build_dir || cleanup_status=$?
	if [ "$command_status" -ne 0 ]; then
		exit "$command_status"
	fi
	exit "$cleanup_status"
}

mote_create_build_dir desktop
trap desktop_cleanup_on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export CARGO_TARGET_DIR="$MOTE_BUILD_DIR/cargo-target"
mkdir -p "$CARGO_TARGET_DIR" "$output_dir"

native_arch=$(uname -m)
[ "$mode" != universal ] || native_arch=universal
if [ "$MOTE_HEIC_MODE" = enabled ]; then
	if [ -z "${MOTE_HEIC_PREFIX:-}" ]; then
		native_environment=$("$repository_root/packaging/heic/build-unix.sh" --platform macos --arch "$native_arch")
		eval "$native_environment"
	fi
	"$repository_root/packaging/heic/verify-native-deps.sh" --prefix "$MOTE_HEIC_PREFIX" --arch "$native_arch"
	export PKG_CONFIG_PATH="$MOTE_HEIC_PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
	export DYLD_LIBRARY_PATH="$MOTE_HEIC_PREFIX/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
	[ "$mode" != universal ] || export PKG_CONFIG_ALLOW_CROSS=1
fi

if [ "$mode" = universal ]; then
	npm exec --workspace @photo-viewer/desktop -- tauri build \
		--target universal-apple-darwin --bundles app "$@"
	candidate="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle/macos/Mote.app"
else
	npm exec --workspace @photo-viewer/desktop -- tauri build --bundles app "$@"
	candidate="$CARGO_TARGET_DIR/release/bundle/macos/Mote.app"
fi

validate_app "$candidate"
ditto "$candidate" "$staging_app"
validate_app "$staging_app"

if [ "$MOTE_HEIC_MODE" = enabled ]; then
	frameworks="$staging_app/Contents/Frameworks"
	licenses="$staging_app/Contents/Resources/licenses"
	mkdir -p "$frameworks" "$licenses"
	cp -L "$MOTE_HEIC_PREFIX/lib/libheif.dylib" "$frameworks/libheif.dylib"
	cp -L "$MOTE_HEIC_PREFIX/lib/libde265.dylib" "$frameworks/libde265.dylib"
	cp "$repository_root/THIRD_PARTY_NOTICES.md" "$licenses/THIRD_PARTY_NOTICES.md"
	cp "$repository_root/packaging/licenses/LGPL-3.0-or-later.txt" "$licenses/LGPL-3.0-or-later.txt"
	cp "$repository_root/packaging/licenses/libheif.md" "$licenses/libheif.md"
	cp "$repository_root/packaging/licenses/libde265.md" "$licenses/libde265.md"
	cp "$repository_root/packaging/heic/README.md" "$licenses/HEIC-REBUILD.md"
	cp "$repository_root/packaging/heic/decode-only.cmake" "$licenses/decode-only.cmake"
	executable=$(find "$staging_app/Contents/MacOS" -type f -perm -111 -print)
	install_name_tool -id @rpath/libheif.dylib "$frameworks/libheif.dylib"
	install_name_tool -id @rpath/libde265.dylib "$frameworks/libde265.dylib"
	for linked_file in "$executable" "$frameworks/libheif.dylib" "$frameworks/libde265.dylib"; do
		otool -arch all -L "$linked_file" | awk '/^[[:space:]]/{print $1}' | sort -u | while IFS= read -r dependency; do
			case "$dependency" in
				*/libheif*.dylib) install_name_tool -change "$dependency" @rpath/libheif.dylib "$linked_file" ;;
				*/libde265*.dylib) install_name_tool -change "$dependency" @rpath/libde265.dylib "$linked_file" ;;
			esac
		done
	done
	if ! otool -l "$executable" | grep -F 'path @executable_path/../Frameworks ' >/dev/null; then
		install_name_tool -add_rpath @executable_path/../Frameworks "$executable"
	fi
	"$repository_root/packaging/heic/verify-native-deps.sh" --app "$staging_app" --arch "$native_arch"
	# Changing a Mach-O invalidates its signature. Sign nested libraries first,
	# then the bundle, preserving Tauri's executable entitlements and flags.
	for library in "$frameworks/libheif.dylib" "$frameworks/libde265.dylib"; do
		codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" "$library"
	done
	codesign --force --sign "${APPLE_SIGNING_IDENTITY:--}" --preserve-metadata=identifier,entitlements,flags "$staging_app"
	codesign --verify --deep --strict "$staging_app"
else
	"$repository_root/packaging/heic/verify-native-deps.sh" --app "$staging_app" --arch "$native_arch" --no-heic
fi

if [ -e "$stable_app" ]; then
	mv -- "$stable_app" "$backup_app"
fi
mv -- "$staging_app" "$stable_app"
if [ -e "$backup_app" ]; then
	mote_remove_managed_path "$backup_app" "$output_dir"
fi

printf '%s\n' "macOS app: $stable_app"
