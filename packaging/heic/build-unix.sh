#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
. "$repository_root/scripts/build-lifecycle.sh"
. "$script_dir/versions.env"

usage() {
	printf '%s\n' "usage: $0 --platform macos|linux --arch ARCH" >&2
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
[ "$host_arch" = "$arch" ] || {
	printf '%s\n' "this builder only supports the current architecture ($host_arch), not $arch" >&2
	exit 2
}

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
case "$platform" in
	macos) require_tool shasum ;;
	linux) require_tool sha256sum ;;
esac

native_root="$repository_root/build/heic-native"
prefix="$repository_root/build/heic-native/${platform}-${arch}"
archive_cache="$native_root/cache"
mkdir -p "$archive_cache"

mote_create_build_dir heic-native
mote_install_cleanup_traps

checksum_archive() {
	case "$platform" in
		macos) shasum -a 256 "$1" | awk '{print $1}' ;;
		linux) sha256sum "$1" | awk '{print $1}' ;;
	esac
}

verify_archive() {
	archive=$1
	expected=$2
	actual=$(checksum_archive "$archive")
	[ "$actual" = "$expected" ] || {
		printf '%s\n' "SHA-256 mismatch for $archive: expected $expected, got $actual" >&2
		return 1
	}
}

extract_archive() {
	archive=$1
	destination=$2
	mkdir -p "$destination"
	tar -xzf "$archive" -C "$destination" --strip-components=1
}

prune_archive_cache() {
	for candidate in "$archive_cache"/*; do
		[ -e "$candidate" ] || continue
		case "$(basename -- "$candidate")" in
			"$LIBDE265_SHA256.tar.gz" | "$LIBHEIF_SHA256.tar.gz") ;;
			*) mote_remove_managed_path "$candidate" "$archive_cache" ;;
		esac
	done
}

download_archive() {
	url=$1
	expected=$2
	archive="$archive_cache/$expected.tar.gz"
	if [ -f "$archive" ]; then
		if verify_archive "$archive" "$expected"; then
			return 0
		fi
		mote_remove_managed_path "$archive" "$archive_cache"
	fi
	temporary="$MOTE_BUILD_DIR/download-$expected.partial"
	curl --fail --location --proto '=https' --tlsv1.2 --output "$temporary" "$url"
	verify_archive "$temporary" "$expected"
	mv "$temporary" "$archive"
}

configure_cmake() {
	name=$1
	shift
	log="$MOTE_BUILD_DIR/$name-configure.log"
	status=0
	cmake --warn-uninitialized -Werror=dev "$@" >"$log" 2>&1 || status=$?
	cat "$log" >&2
	if grep -E 'Manually-specified variables were not used by the project|Unknown CMake command|Unknown argument' "$log" >/dev/null; then
		printf '%s\n' "CMake rejected or ignored a requested build switch for $name" >&2
		return 1
	fi
	[ "$status" -eq 0 ] || return "$status"
}

prune_archive_cache
download_archive "$LIBDE265_URL" "$LIBDE265_SHA256"
download_archive "$LIBHEIF_URL" "$LIBHEIF_SHA256"

de265_source="$MOTE_BUILD_DIR/libde265-$LIBDE265_VERSION"
heif_source="$MOTE_BUILD_DIR/libheif-$LIBHEIF_VERSION"
de265_build="$MOTE_BUILD_DIR/libde265-build"
heif_build="$MOTE_BUILD_DIR/libheif-build"
stage_root="$MOTE_BUILD_DIR/stage"
staged_prefix="$stage_root$prefix"

extract_archive "$archive_cache/$LIBDE265_SHA256.tar.gz" "$de265_source"
extract_archive "$archive_cache/$LIBHEIF_SHA256.tar.gz" "$heif_source"

configure_cmake libde265 \
	-S "$de265_source" \
	-B "$de265_build" \
	-DCMAKE_BUILD_TYPE=Release \
	-DCMAKE_INSTALL_PREFIX="$prefix" \
	-DBUILD_SHARED_LIBS=ON \
	-DENABLE_DECODER=OFF \
	-DENABLE_ENCODER=OFF \
	-DENABLE_SDL=OFF \
	-DENABLE_SHERLOCK265=OFF \
	-DENABLE_INTERNAL_DEVELOPMENT_TOOLS=OFF \
	-DWITH_FUZZERS=OFF \
	-DUSE_IWYU=OFF \
	-DFORCE_FULL_VISIBILITY=OFF
cmake --build "$de265_build" --target de265 --parallel >&2
DESTDIR="$stage_root" cmake --install "$de265_build" >&2

case "$platform" in
	macos) de265_library="$staged_prefix/lib/libde265.dylib" ;;
	linux) de265_library="$staged_prefix/lib/libde265.so" ;;
esac
[ -e "$de265_library" ] || {
	printf '%s\n' "libde265 was not staged at $de265_library" >&2
	exit 1
}

configure_cmake libheif \
	-S "$heif_source" \
	-B "$heif_build" \
	-DCMAKE_BUILD_TYPE=Release \
	-DCMAKE_INSTALL_PREFIX="$prefix" \
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
	-DLIBDE265_INCLUDE_DIR="$staged_prefix/include" \
	-DLIBDE265_LIBRARY="$de265_library" \
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

cmake --build "$heif_build" --target heif --parallel >&2
DESTDIR="$stage_root" cmake --install "$heif_build" >&2

case "$platform" in
	macos)
		find "$staged_prefix/lib" -type f -name 'libde265*.dylib' -print -quit | grep -q .
		find "$staged_prefix/lib" -type f -name 'libheif*.dylib' -print -quit | grep -q .
		;;
	linux)
		find "$staged_prefix/lib" -type f -name 'libde265.so*' -print -quit | grep -q .
		find "$staged_prefix/lib" -type f -name 'libheif.so*' -print -quit | grep -q .
		;;
esac
[ -f "$staged_prefix/lib/pkgconfig/libheif.pc" ]
if find "$staged_prefix/lib" -iname '*x265*' -o -iname '*plugin*' | grep -q .; then
	printf '%s\n' "unexpected encoder or plugin staged below $staged_prefix" >&2
	exit 1
fi
[ ! -e "$staged_prefix/bin/heif-enc" ]
[ ! -e "$staged_prefix/bin/enc265" ]

if [ -e "$prefix" ]; then
	mote_remove_managed_path "$prefix" "$native_root"
fi
mkdir -p "$(dirname -- "$prefix")"
mv "$staged_prefix" "$prefix"

shell_quote() {
	printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

pkg_config_path="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
printf 'export PKG_CONFIG_PATH='
shell_quote "$pkg_config_path"
printf '\n'
case "$platform" in
	macos)
		dyld_library_path="$prefix/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
		printf 'export DYLD_LIBRARY_PATH='
		shell_quote "$dyld_library_path"
		printf '\n'
		;;
	linux)
		ld_library_path="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
		printf 'export LD_LIBRARY_PATH='
		shell_quote "$ld_library_path"
		printf '\n'
		;;
esac
