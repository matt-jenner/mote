#!/bin/sh
set -eu

usage() {
	printf '%s\n' "usage: $0 [--prefix DIR | --app Mote.app] [--binary FILE] [--arch ARCH|universal] [--no-heic]" >&2
	exit 2
}
fail() { printf '%s\n' "$*" >&2; exit 1; }
prefix=
app=
binary=
arch=$(uname -m)
disabled=0
while [ "$#" -gt 0 ]; do
	case "$1" in
		--prefix | --app | --binary | --arch)
			[ "$#" -ge 2 ] || usage
			case "$1" in --prefix) prefix=$2 ;; --app) app=$2 ;; --binary) binary=$2 ;; --arch) arch=$2 ;; esac
			shift 2 ;;
		--no-heic) disabled=1; shift ;;
		*) usage ;;
	esac
done
[ -n "$prefix$app$binary" ] || usage
[ -z "$prefix" ] || [ -z "$app" ] || usage
system=$(uname -s)
case "$system" in Darwin | Linux) ;; *) fail "unsupported inspection host: $system" ;; esac
case "$arch" in '' | *[!a-zA-Z0-9_-]*) usage ;; esac
library_dir=
scan_root=$prefix
if [ -n "$prefix" ]; then
	[ -d "$prefix" ] || fail "missing prefix: $prefix"
	library_dir="$prefix/lib"
elif [ -n "$app" ]; then
	[ -d "$app/Contents/MacOS" ] || fail "missing app: $app"
	scan_root=$app
	library_dir="$app/Contents/Frameworks"
	binary=$(find "$app/Contents/MacOS" -type f -perm -111 -print)
	[ "$(printf '%s\n' "$binary" | wc -l | tr -d ' ')" = 1 ] || fail 'expected one app executable'
fi
[ -z "$binary" ] || [ -f "$binary" ] || fail "missing binary: $binary"

# Header declarations such as heif_encoding.h are part of the stable ABI.
# Reject codec implementations and tools, not libheif's encoder-query API.
forbidden='x265|x264|kvazaar|rav1e|svt.?av1|vvenc|uvg266|heif-enc|enc265|en265_|get_encoder_plugin_'
if [ -n "$scan_root" ]; then
	artifacts=$(find "$scan_root" \( -type f -o -type l \) \( -name '*.dylib' -o -name '*.so*' -o -name '*.a' -o -path '*/bin/*' \) -print)
	if printf '%s\n' "$artifacts" | grep -Ei "$forbidden|plugin|\.a$" >/dev/null; then
		fail "unexpected encoder, static library, or plugin in $scan_root"
	fi
	if [ "$disabled" -eq 1 ] && find "$scan_root" \( -iname '*libheif*' -o -iname '*libde265*' \) -print | grep -q .; then
		fail "HEIF-disabled artifact contains native decoder files: $scan_root"
	fi
fi

inspect() {
	inspect_file=$1
	[ -f "$inspect_file" ] || fail "missing runtime library: $inspect_file"
	if [ "$system" = Darwin ]; then
		if [ "$arch" = universal ]; then
			lipo "$inspect_file" -verify_arch arm64 x86_64
		else
			lipo "$inspect_file" -verify_arch "$arch"
		fi
		dependencies=$(otool -arch all -L "$inspect_file")
		load_commands=$(otool -arch all -l "$inspect_file")
		rpaths=$(printf '%s\n' "$load_commands" | awk '/cmd LC_RPATH/{next_path=1;next} next_path && $1=="path"{print $2;next_path=0}')
		printf '%s\n' "$rpaths" | while IFS= read -r rpath; do
			case "$rpath" in '' | @loader_path | @executable_path/../Frameworks) ;; *) fail "nonportable runtime search path: $rpath" ;; esac
		done
		symbols=$(nm -a "$inspect_file")
	else
		header=$(readelf -h "$inspect_file")
		case "$arch" in
			x86_64) machine='Advanced Micro Devices X86-64' ;;
			aarch64 | arm64) machine=AArch64 ;;
			*) fail "unsupported ELF architecture: $arch" ;;
		esac
		printf '%s\n' "$header" | grep -F "$machine" >/dev/null || fail "missing $arch architecture: $inspect_file"
		dependencies=$(ldd "$inspect_file" 2>&1) || fail "cannot resolve dependencies: $inspect_file"
		printf '%s\n' "$dependencies" | grep -q 'not found' && fail "missing runtime dependency: $inspect_file"
		load_commands=$(readelf -d "$inspect_file")
		if printf '%s\n' "$load_commands" | grep -E '\((NEEDED|SONAME)\).*\[[^]]*/' >/dev/null; then
			fail "absolute ELF dependency or identity: $inspect_file"
		fi
		if printf '%s\n' "$load_commands" | grep -E '\((RPATH|RUNPATH)\)' | grep -v '\[\$ORIGIN\]' >/dev/null; then
			fail "nonportable runtime search path: $inspect_file"
		fi
		symbols=$(nm --defined-only "$inspect_file")
	fi
	printf '%s\n' "$dependencies"
	if printf '%s\n' "$dependencies" "$symbols" | grep -Ei "$forbidden" >/dev/null; then
		fail "encoder implementation or dependency in $inspect_file"
	fi
	if [ "$disabled" -eq 1 ] && printf '%s\n' "$dependencies" | grep -Ei 'libheif|libde265' >/dev/null; then
		fail "HEIF-disabled binary links a decoder: $inspect_file"
	fi
	# Release binaries must not retain source/build directory strings either.
	if [ "$inspect_file" != "$binary" ] && strings "$inspect_file" | grep -v '^/tmp/libheif-XXXXXX$' | grep -E '/(Users|home|private/(tmp|var)|tmp)/|mote-build-heic-native\.' >/dev/null; then
		fail "absolute build path in $inspect_file"
	fi
	if [ "$system" = Darwin ]; then
		printf '%s\n' "$dependencies" | awk '/^[[:space:]]/{print $1}' | while IFS= read -r dependency; do
			case "$dependency" in
				/usr/lib/* | /System/Library/*) ;;
				@rpath/*)
					[ -n "$library_dir" ] && [ -e "$library_dir/${dependency#@rpath/}" ] || fail "missing runtime dependency: $dependency" ;;
				@loader_path/*)
					[ -e "$(dirname "$inspect_file")/${dependency#@loader_path/}" ] || fail "missing runtime dependency: $dependency" ;;
				@executable_path/*)
					[ -n "$binary" ] && [ -e "$(dirname "$binary")/${dependency#@executable_path/}" ] || fail "missing runtime dependency: $dependency" ;;
				*) fail "absolute or unsupported dependency: $dependency" ;;
			esac
		done
	fi
}

if [ "$disabled" -eq 0 ] && [ -n "$library_dir" ]; then
	case "$system" in Darwin) suffix=dylib ;; Linux) suffix=so ;; esac
	inspect "$library_dir/libde265.$suffix"
	inspect "$library_dir/libheif.$suffix"
	printf '%s\n' "$dependencies" | grep -q libde265 || fail 'libheif is not dynamically linked to libde265'
fi
if [ -n "$binary" ]; then
	inspect "$binary"
	if [ "$disabled" -eq 0 ]; then
		printf '%s\n' "$dependencies" | grep -q libheif || fail 'enabled binary is not dynamically linked to libheif'
	fi
fi
printf '%s\n' 'native dependency inspection passed'
