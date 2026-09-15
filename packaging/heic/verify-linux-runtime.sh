#!/bin/sh
set -eu

[ "$#" = 3 ] || { echo 'usage: verify-linux-runtime.sh ROOT BINARY enabled|disabled' >&2; exit 2; }
runtime_root=$1
runtime_binary=$2
runtime_mode=$3
case "$runtime_mode" in enabled|disabled) ;; *) exit 2 ;; esac
fail() { printf '%s\n' "$*" >&2; exit 1; }
[ -d "$runtime_root" ] && [ -f "$runtime_binary" ] || fail 'missing runtime root or binary'
runtime_root=$(CDPATH= cd -- "$runtime_root" && pwd -P)
runtime_scan_prefix=${runtime_root%/}
# Scan every shipped location, including /usr/lib, /lib, /usr/bin and /opt.
# Do not follow symlinks or cross mounts, and prune virtual/transient trees
# before descending. The same paths are relative to a staging root such as
# /runtime; a trailing slash does not change the exclusions.
runtime_files=$(find -P "$runtime_root" -xdev \
	\( -path "$runtime_scan_prefix/proc" -o -path "$runtime_scan_prefix/sys" \
	-o -path "$runtime_scan_prefix/dev" -o -path "$runtime_scan_prefix/run" \
	-o -path "$runtime_scan_prefix/tmp" -o -path "$runtime_scan_prefix/var/run" \
	-o -path "$runtime_scan_prefix/var/tmp" \) -prune -o \
	\( -type f -o -type l \) -print)
forbidden='x265|x264|kvazaar|rav1e|svt.?av1|vvenc|uvg266|heif-enc|enc265|libheif/plugins'
if printf '%s\n' "$runtime_files" | grep -Ei "$forbidden|/include/|/pkgconfig/|/cmake/|\.a$|/lib(heif|de265)\.so$"; then
	fail 'unexpected encoder or development file in runtime'
fi
if [ "$runtime_mode" = disabled ]; then
	if printf '%s\n' "$runtime_files" | grep -Ei 'libheif|libde265'; then fail 'disabled runtime contains decoder files'; fi
else
	for library in libheif libde265; do
		printf '%s\n' "$runtime_files" | grep -E "/$library\.so\.[0-9]" >/dev/null || fail "missing runtime library: $library"
	done
fi
inspect_runtime_file() {
	runtime_dependencies=$(ldd "$1" 2>&1) || fail "cannot inspect dependencies: $1"
	if printf '%s\n' "$runtime_dependencies" | grep -Ei "$forbidden|not found"; then fail 'forbidden or unresolved runtime dependency'; fi
	if [ "$runtime_mode" = disabled ]; then
		if printf '%s\n' "$runtime_dependencies" | grep -Ei 'libheif|libde265'; then fail 'disabled binary references decoder'; fi
	fi
	# The build SDK/CI host supplies readelf. The slim runtime deliberately has
	# no binutils; the smoke still verifies actual loader resolution there.
	if command -v readelf >/dev/null 2>&1; then
		runtime_dynamic=$(readelf -d "$1") || fail "cannot inspect ELF metadata: $1"
		if printf '%s\n' "$runtime_dynamic" | grep -E '\((NEEDED|SONAME)\).*\[[^]]*/'; then fail 'absolute ELF dependency'; fi
		if [ "$runtime_mode" = disabled ] && printf '%s\n' "$runtime_dynamic" | grep -Ei 'libheif|libde265'; then fail 'disabled ELF references decoder'; fi
	fi
}
inspect_runtime_file "$runtime_binary"
if [ "$runtime_mode" = enabled ]; then
	for library in libheif libde265; do
		printf '%s\n' "$runtime_dependencies" | grep -F "$library.so." >/dev/null || fail "binary does not load $library"
	done
	printf '%s\n' "$runtime_files" | grep -E '/lib(heif|de265)\.so\.[0-9][^/]*$' | while IFS= read -r library; do inspect_runtime_file "$library"; done
fi
printf '%s\n' "Linux $runtime_mode runtime inspection passed"
