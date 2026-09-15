#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
. "$repository_root/scripts/build-lifecycle.sh"
. "$script_dir/versions.env"

usage() {
	printf '%s\n' "usage: $0 --platform macos|linux --arch ARCH|universal" >&2
}

platform=
arch=
while [ "$#" -gt 0 ]; do
	case "$1" in
		--platform)
			[ "$#" -ge 2 ] || {
				usage
				exit 2
			}
			platform=$2
			shift 2
			;;
		--arch)
			[ "$#" -ge 2 ] || {
				usage
				exit 2
			}
			arch=$2
			shift 2
			;;
		*)
			printf '%s\n' "unknown argument: $1" >&2
			usage
			exit 2
			;;
	esac
done

case "$platform" in
	macos) expected_system=Darwin ;;
	linux) expected_system=Linux ;;
	*)
		usage
		exit 2
		;;
esac
case "$arch" in
	'' | *[!A-Za-z0-9_-]*)
		printf '%s\n' "invalid architecture: $arch" >&2
		exit 2
		;;
esac

host_system=$(uname -s)
host_arch=$(uname -m)
[ "$host_system" = "$expected_system" ] || {
	printf '%s\n' "requested $platform build on $host_system" >&2
	exit 2
}
if [ "$platform" = macos ]; then
	case "$arch" in arm64 | x86_64 | universal) ;; *) usage; exit 2 ;; esac
elif [ "$host_arch" != "$arch" ]; then
	printf '%s\n' "this builder only supports the current architecture ($host_arch), not $arch" >&2
	exit 2
fi

require_tool() {
	command -v "$1" >/dev/null 2>&1 || {
		printf '%s\n' "required build tool is unavailable: $1" >&2
		exit 1
	}
}

require_tool cmake
require_tool curl
require_tool grep
require_tool tar
require_tool cc
case "$platform" in
	macos) require_tool shasum; require_tool lipo; require_tool install_name_tool ;;
	linux) require_tool sha256sum ;;
esac

heic_native_root="$repository_root/build/heic-native"
heic_install_prefix="$repository_root/build/heic-native/${platform}-${arch}"
heic_archive_cache="$heic_native_root/cache"
heic_publish_stage_root="$heic_native_root/.${platform}-${arch}.stage.$$"
heic_publish_backup="$heic_native_root/.${platform}-${arch}.backup.$$"
heic_publication_active=0
heic_publication_committed=0
heic_install_existed=0
mkdir -p "$heic_archive_cache"

mote_create_build_dir heic-native

heic_cleanup_on_exit() {
	heic_command_status=$?
	trap - EXIT HUP INT TERM
	heic_cleanup_status=0
	if [ "$heic_publication_active" -eq 1 ] && [ "$heic_publication_committed" -eq 0 ]; then
		if [ -e "$heic_publish_backup" ]; then
			if [ -e "$heic_install_prefix" ]; then
				mote_remove_managed_path "$heic_install_prefix" "$heic_native_root" || heic_cleanup_status=$?
			fi
			mv "$heic_publish_backup" "$heic_install_prefix" || heic_cleanup_status=$?
		elif [ "$heic_install_existed" -eq 0 ] && [ -e "$heic_install_prefix" ]; then
			mote_remove_managed_path "$heic_install_prefix" "$heic_native_root" || heic_cleanup_status=$?
		fi
	elif [ -e "$heic_publish_backup" ]; then
		mote_remove_managed_path "$heic_publish_backup" "$heic_native_root" || heic_cleanup_status=$?
fi
	if [ -e "$heic_publish_stage_root" ]; then
		mote_remove_managed_path "$heic_publish_stage_root" "$heic_native_root" || heic_cleanup_status=$?
	fi
	mote_cleanup_build_dir || heic_cleanup_status=$?
	if [ "$heic_command_status" -ne 0 ]; then
		exit "$heic_command_status"
	fi
	exit "$heic_cleanup_status"
}

trap heic_cleanup_on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

checksum_archive() {
	case "$platform" in
		macos) shasum -a 256 "$1" | awk '{print $1}' ;;
		linux) sha256sum "$1" | awk '{print $1}' ;;
	esac
}

verify_archive() {
	heic_verify_path=$1
	heic_verify_expected=$2
	heic_verify_actual=$(checksum_archive "$heic_verify_path")
	[ "$heic_verify_actual" = "$heic_verify_expected" ] || {
		printf '%s\n' "SHA-256 mismatch for $heic_verify_path: expected $heic_verify_expected, got $heic_verify_actual" >&2
		return 1
	}
}

extract_archive() {
	heic_extract_path=$1
	heic_extract_destination=$2
	mkdir -p "$heic_extract_destination"
	tar -xzf "$heic_extract_path" -C "$heic_extract_destination" --strip-components=1
}

prune_archive_cache() {
	for heic_cache_entry in "$heic_archive_cache"/*; do
		[ -e "$heic_cache_entry" ] || continue
		case "$(basename -- "$heic_cache_entry")" in
			"$LIBDE265_SHA256.tar.gz" | "$LIBHEIF_SHA256.tar.gz") ;;
			*) mote_remove_managed_path "$heic_cache_entry" "$heic_archive_cache" ;;
		esac
	done
}

download_archive() {
	heic_download_url=$1
	heic_download_expected=$2
	heic_download_destination="$heic_archive_cache/$heic_download_expected.tar.gz"
	if [ -f "$heic_download_destination" ]; then
		if verify_archive "$heic_download_destination" "$heic_download_expected"; then
			return 0
		fi
		mote_remove_managed_path "$heic_download_destination" "$heic_archive_cache"
	fi
	heic_download_partial="$MOTE_BUILD_DIR/download-$heic_download_expected.partial"
	curl --fail --location --proto '=https' --tlsv1.2 --output "$heic_download_partial" "$heic_download_url"
	verify_archive "$heic_download_partial" "$heic_download_expected"
	mv "$heic_download_partial" "$heic_download_destination"
}

configure_cmake() {
	heic_configure_name=$1
	shift
	heic_configure_log="$MOTE_BUILD_DIR/$heic_configure_name-configure.log"
	heic_configure_status=0
	# Install-relative load paths also apply inside the temporary build tree.
	# No loader may depend on a staging directory that cleanup will remove.
	if [ "$platform" = macos ]; then
		set -- "$@" "-DCMAKE_OSX_ARCHITECTURES=$heic_slice_arch" \
			-DCMAKE_INSTALL_NAME_DIR=@rpath '-DCMAKE_INSTALL_RPATH=@loader_path'
	else
		set -- "$@" '-DCMAKE_INSTALL_RPATH=$ORIGIN'
	fi
	set -- "$@" -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
		"-DCMAKE_C_FLAGS=-ffile-prefix-map=$MOTE_BUILD_DIR=. -ffile-prefix-map=$heic_publish_stage_root=." \
		"-DCMAKE_CXX_FLAGS=-ffile-prefix-map=$MOTE_BUILD_DIR=. -ffile-prefix-map=$heic_publish_stage_root=."
	cmake --warn-uninitialized -Werror=dev "$@" >"$heic_configure_log" 2>&1 || heic_configure_status=$?
	cat "$heic_configure_log" >&2
	if grep -E 'Manually-specified variables were not used by the project|Unknown CMake command|Unknown argument' "$heic_configure_log" >/dev/null; then
		printf '%s\n' "CMake rejected or ignored a requested build switch for $heic_configure_name" >&2
		return 1
	fi
	[ "$heic_configure_status" -eq 0 ] || return "$heic_configure_status"
}

prune_archive_cache
download_archive "$LIBDE265_URL" "$LIBDE265_SHA256"
download_archive "$LIBHEIF_URL" "$LIBHEIF_SHA256"

heic_de265_source="$MOTE_BUILD_DIR/libde265-$LIBDE265_VERSION"
heic_heif_source="$MOTE_BUILD_DIR/libheif-$LIBHEIF_VERSION"
mkdir -p "$heic_publish_stage_root"
extract_archive "$heic_archive_cache/$LIBDE265_SHA256.tar.gz" "$heic_de265_source"
extract_archive "$heic_archive_cache/$LIBHEIF_SHA256.tar.gz" "$heic_heif_source"
cmake "-DSOURCE_DIR=$heic_heif_source" -P "$script_dir/decode-only.cmake" >&2

build_slice() {
heic_slice_arch=$1
heic_de265_build="$MOTE_BUILD_DIR/libde265-build"
heic_heif_build="$MOTE_BUILD_DIR/libheif-build"
heic_stage_root="$heic_publish_stage_root/$heic_slice_arch"
heic_staged_prefix="$heic_stage_root$heic_install_prefix"

configure_cmake libde265 \
	-S "$heic_de265_source" \
	-B "$heic_de265_build" \
	-DCMAKE_BUILD_TYPE=Release \
	-DCMAKE_INSTALL_PREFIX="$heic_install_prefix" \
	-DBUILD_SHARED_LIBS=ON \
	-DENABLE_DECODER=OFF \
	-DENABLE_ENCODER=OFF \
	-DENABLE_SDL=OFF \
	-DENABLE_SHERLOCK265=OFF \
	-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF \
	-DWITH_FUZZERS=OFF \
	-DUSE_IWYU=OFF \
	-DFORCE_FULL_VISIBILITY=OFF
cmake --build "$heic_de265_build" --target de265 --parallel >&2
DESTDIR="$heic_stage_root" cmake --install "$heic_de265_build" >&2

case "$platform" in
	macos) heic_de265_library="$heic_staged_prefix/lib/libde265.dylib" ;;
	linux) heic_de265_library="$heic_staged_prefix/lib/libde265.so" ;;
esac
[ -e "$heic_de265_library" ] || {
	printf '%s\n' "libde265 was not staged at $heic_de265_library" >&2
	exit 1
}

configure_cmake libheif \
	-S "$heic_heif_source" \
	-B "$heic_heif_build" \
	-DCMAKE_BUILD_TYPE=Release \
	-DCMAKE_INSTALL_PREFIX="$heic_install_prefix" \
	-DBUILD_SHARED_LIBS=ON \
	-DBUILD_TESTING=OFF \
	-DBUILD_DOCUMENTATION=OFF \
	-DBUILD_DEVELOPMENT_TOOLS=OFF \
	-DENABLE_COVERAGE=OFF \
	-DENABLE_EXPERIMENTAL_FEATURES=OFF \
	-DENABLE_MULTITHREADING_SUPPORT=ON \
	-DENABLE_PARALLEL_TILE_DECODING=ON \
	-DENABLE_PLUGIN_LOADING=OFF \
	-DWITH_LIBDE265=ON \
	-DWITH_LIBDE265_PLUGIN=OFF \
	-DLIBDE265_INCLUDE_DIR="$heic_staged_prefix/include" \
	-DLIBDE265_LIBRARY="$heic_de265_library" \
	-DWITH_X265=OFF \
	-DWITH_X265_PLUGIN=OFF \
	-DWITH_KVAZAAR=OFF \
	-DWITH_KVAZAAR_PLUGIN=OFF \
	-DWITH_UVG266=OFF \
	-DWITH_UVG266_PLUGIN=OFF \
	-DWITH_VVDEC=OFF \
	-DWITH_VVDEC_PLUGIN=OFF \
	-DWITH_VVENC=OFF \
	-DWITH_VVENC_PLUGIN=OFF \
	-DWITH_X264=OFF \
	-DWITH_X264_PLUGIN=OFF \
	-DWITH_OpenH264_DECODER=OFF \
	-DWITH_OpenH264_DECODER_PLUGIN=OFF \
	-DWITH_DAV1D=OFF \
	-DWITH_DAV1D_PLUGIN=OFF \
	-DWITH_AOM_DECODER=OFF \
	-DWITH_AOM_DECODER_PLUGIN=OFF \
	-DWITH_AOM_ENCODER=OFF \
	-DWITH_AOM_ENCODER_PLUGIN=OFF \
	-DWITH_SvtEnc=OFF \
	-DWITH_SvtEnc_PLUGIN=OFF \
	-DWITH_RAV1E=OFF \
	-DWITH_RAV1E_PLUGIN=OFF \
	-DWITH_JPEG_DECODER=OFF \
	-DWITH_JPEG_DECODER_PLUGIN=OFF \
	-DWITH_JPEG_ENCODER=OFF \
	-DWITH_JPEG_ENCODER_PLUGIN=OFF \
	-DWITH_OpenJPEG_DECODER=OFF \
	-DWITH_OpenJPEG_DECODER_PLUGIN=OFF \
	-DWITH_OpenJPEG_ENCODER=OFF \
	-DWITH_OpenJPEG_ENCODER_PLUGIN=OFF \
	-DWITH_FFMPEG_DECODER=OFF \
	-DWITH_FFMPEG_DECODER_PLUGIN=OFF \
	-DWITH_OPENJPH_ENCODER=OFF \
	-DWITH_OPENJPH_ENCODER_PLUGIN=OFF \
	-DWITH_UNCOMPRESSED_CODEC=OFF \
	-DWITH_WEBCODECS=OFF \
	-DWITH_LIBSHARPYUV=OFF \
	-DWITH_LIBSHARPYUV_INTERNAL=OFF \
	-DWITH_HEADER_COMPRESSION=OFF \
	-DWITH_EXAMPLES=OFF \
	-DWITH_EXAMPLE_HEIF_THUMB=OFF \
	-DWITH_EXAMPLE_HEIF_VIEW=OFF \
	-DWITH_GDK_PIXBUF=OFF \
	-DWITH_FUZZERS=OFF \
	-DWITH_REDUCED_VISIBILITY=ON

grep -E 'libde265 HEVC decoder[[:space:]]*: \+ built-in' "$MOTE_BUILD_DIR/libheif-configure.log" >/dev/null || {
	printf '%s\n' "libheif did not configure libde265 as an in-process decoder" >&2
	exit 1
}
grep -E 'x265 HEVC encoder[[:space:]]*: - disabled' "$MOTE_BUILD_DIR/libheif-configure.log" >/dev/null || {
	printf '%s\n' "libheif did not disable x265" >&2
	exit 1
}

cmake --build "$heic_heif_build" --target heif --parallel >&2
DESTDIR="$heic_stage_root" cmake --install "$heic_heif_build" >&2
mote_remove_managed_path "$heic_de265_build" "$MOTE_BUILD_DIR"
mote_remove_managed_path "$heic_heif_build" "$MOTE_BUILD_DIR"
}

if [ "$arch" = universal ]; then
	build_slice arm64
	build_slice x86_64
	heic_staged_prefix="$heic_publish_stage_root/merged"
	cp -R "$heic_publish_stage_root/arm64$heic_install_prefix" "$heic_staged_prefix"
	for heic_library in "$heic_staged_prefix/lib/"*.dylib; do
		[ ! -L "$heic_library" ] || continue
		heic_library_name=$(basename "$heic_library")
		lipo -create \
			"$heic_publish_stage_root/arm64$heic_install_prefix/lib/$heic_library_name" \
			"$heic_publish_stage_root/x86_64$heic_install_prefix/lib/$heic_library_name" \
			-output "$heic_library"
		lipo "$heic_library" -verify_arch arm64 x86_64
	done
else
	build_slice "$arch"
fi

if [ "$platform" = macos ]; then
	# Normalize IDs independently of upstream ABI filenames. Preserve the ABI
	# aliases for consumers that already linked against the upstream IDs.
	install_name_tool -id @rpath/libde265.dylib "$heic_staged_prefix/lib/libde265.dylib"
	install_name_tool -id @rpath/libheif.dylib \
		-change @rpath/libde265.0.dylib @rpath/libde265.dylib "$heic_staged_prefix/lib/libheif.dylib"
fi
LD_LIBRARY_PATH="$heic_staged_prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
	"$script_dir/verify-native-deps.sh" --prefix "$heic_staged_prefix" --arch "$arch" >&2

# Run against the staged shared libraries, before publication. The probe and
# its build-machine rpath are temporary and are never installed or bundled.
set --
if [ "$platform" = macos ]; then
	heic_probe_arch=$arch
	[ "$arch" != universal ] || heic_probe_arch=$host_arch
	set -- -arch "$heic_probe_arch"
fi
cc "$@" -I"$heic_staged_prefix/include" "$script_dir/verify-decoder.c" \
	-L"$heic_staged_prefix/lib" -lheif -lde265 "-Wl,-rpath,$heic_staged_prefix/lib" \
	-o "$MOTE_BUILD_DIR/verify-decoder" >&2
"$MOTE_BUILD_DIR/verify-decoder" "$LIBHEIF_VERSION" "$LIBDE265_VERSION" >&2

case "$platform" in
	macos)
		find "$heic_staged_prefix/lib" -type f -name 'libde265*.dylib' -print -quit | grep -q .
		find "$heic_staged_prefix/lib" -type f -name 'libheif*.dylib' -print -quit | grep -q .
		;;
	linux)
		find "$heic_staged_prefix/lib" -type f -name 'libde265.so*' -print -quit | grep -q .
		find "$heic_staged_prefix/lib" -type f -name 'libheif.so*' -print -quit | grep -q .
		;;
esac
[ -f "$heic_staged_prefix/lib/pkgconfig/libheif.pc" ]
if find "$heic_staged_prefix/lib" -iname '*x265*' -o -iname '*plugin*' | grep -q .; then
	printf '%s\n' "unexpected encoder or plugin staged below $heic_staged_prefix" >&2
	exit 1
fi
[ ! -e "$heic_staged_prefix/bin/heif-enc" ]
[ ! -e "$heic_staged_prefix/bin/enc265" ]

if [ -e "$heic_install_prefix" ]; then
	heic_install_existed=1
	fi
heic_publication_active=1
if [ "$heic_install_existed" -eq 1 ]; then
	mv "$heic_install_prefix" "$heic_publish_backup"
fi
heic_publish_status=0
mv "$heic_staged_prefix" "$heic_install_prefix" || heic_publish_status=$?
[ "$heic_publish_status" -eq 0 ] || exit "$heic_publish_status"
heic_publication_committed=1
if [ -e "$heic_publish_backup" ]; then
	mote_remove_managed_path "$heic_publish_backup" "$heic_native_root"
fi
heic_publication_active=0

shell_quote() {
	printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

pkg_config_path="$heic_install_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
printf 'export MOTE_HEIC_PREFIX='
shell_quote "$heic_install_prefix"
printf '\n'
printf 'export PKG_CONFIG_PATH='
shell_quote "$pkg_config_path"
printf '\n'
case "$platform" in
	macos)
		dyld_library_path="$heic_install_prefix/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
		printf 'export DYLD_LIBRARY_PATH='
		shell_quote "$dyld_library_path"
		printf '\n'
		;;
	linux)
		ld_library_path="$heic_install_prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
		printf 'export LD_LIBRARY_PATH='
		shell_quote "$ld_library_path"
		printf '\n'
		;;
esac
