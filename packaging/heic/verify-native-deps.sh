#!/bin/sh
set -eu

usage() {
	printf '%s\n' "usage: $0 [--prefix DIR | --app Mote.app] [--binary FILE] [--arch ARCH|universal] [--deployment-target VERSION] [--no-heic]" >&2
	exit 2
}
fail() { printf '%s\n' "$*" >&2; exit 1; }
prefix=
app=
binary=
arch=$(uname -m)
disabled=0
deployment_target=
while [ "$#" -gt 0 ]; do
	case "$1" in
		--prefix | --app | --binary | --arch | --deployment-target)
			[ "$#" -ge 2 ] || usage
			case "$1" in --prefix) prefix=$2 ;; --app) app=$2 ;; --binary) binary=$2 ;; --arch) arch=$2 ;; --deployment-target) deployment_target=$2 ;; esac
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
if [ "$system" = Darwin ]; then
	if [ -z "$deployment_target" ]; then
		inspection_script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
		deployment_target=$(sh "$inspection_script_dir/../../scripts/macos-deployment-target.sh")
	fi
	printf '%s\n' "$deployment_target" | grep -E '^[0-9]+(\.[0-9]+)?(\.[0-9]+)?$' >/dev/null || fail 'invalid macOS deployment target'
fi
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

inspect_deployment_target() {
	deployment_file=$1
	deployment_slices=$(lipo -archs "$deployment_file")
	[ -n "$deployment_slices" ] || fail "missing architecture for deployment inspection: $deployment_file"
	for deployment_slice in $deployment_slices; do
		case "$deployment_slice" in arm64 | x86_64) ;; *) fail "unsupported deployment architecture: $deployment_slice" ;; esac
		# Query one slice at a time: a universal header alone cannot establish
		# that both slices carry a compatible macOS load command.
		deployment_commands=$(otool -arch "$deployment_slice" -l "$deployment_file")
		deployment_minimum=$(printf '%s\n' "$deployment_commands" | awk '
			$1 == "cmd" { kind = $2; platform = "" }
			kind == "LC_BUILD_VERSION" && $1 == "platform" { platform = tolower($2) }
			kind == "LC_BUILD_VERSION" && $1 == "minos" {
				if (platform != "1" && platform != "macos") exit 1
				print $2; count++
			}
			kind == "LC_VERSION_MIN_MACOSX" && $1 == "version" { print $2; count++ }
			END { if (count != 1) exit 1 }
		') || fail "missing or invalid macOS minimum/platform for $deployment_slice: $deployment_file"
		# Numeric component comparison, padding absent minor/patch with zero.
		# Lexical and decimal-number comparisons misorder 11.10 and 11.9.
		awk -v minimum="$deployment_minimum" -v expected="$deployment_target" 'BEGIN {
			if (minimum !~ /^[0-9]+([.][0-9]+)?([.][0-9]+)?$/) exit 1
			split(minimum, actual, "."); split(expected, target, ".")
			for (i = 1; i <= 3; i++) {
				if (actual[i] + 0 < target[i] + 0) exit 0
				if (actual[i] + 0 > target[i] + 0) exit 1
			}
		}' || fail "macOS minimum $deployment_minimum exceeds deployment target $deployment_target or is invalid ($deployment_slice): $deployment_file"
	done
}

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
	embedded_path=
	if [ "$inspect_file" != "$binary" ]; then
		embedded_path=$(strings "$inspect_file" | grep -Ev '/tmp/libheif-X+$' | grep -E '/(Users|home|private/(tmp|var)|tmp)/|mote-build-heic-native\.' | head -n 1 || true)
	fi
	if [ -n "$embedded_path" ]; then
		reported_path=$embedded_path
		case "$reported_path" in "$PWD"/*) reported_path="<checkout>/${reported_path#"$PWD"/}" ;; esac
		if [ -n "${MOTE_BUILD_DIR:-}" ]; then
			case "$reported_path" in "$MOTE_BUILD_DIR"/*) reported_path="<native-work>/${reported_path#"$MOTE_BUILD_DIR"/}" ;; esac
		fi
		fail "absolute build path $reported_path in $inspect_file"
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

if [ "$system" = Darwin ] && [ -n "$scan_root" ]; then
	find "$scan_root" \( -type f -o -type l \) -name '*.dylib' -print | while IFS= read -r deployment_library; do
		inspect_deployment_target "$deployment_library"
	done
fi
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
