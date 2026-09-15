import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const readRepoFile = (...segments) =>
	fs.readFileSync(path.join(root, ...segments), "utf8");

const parseVersions = (source) =>
	Object.fromEntries(
		source
			.split(/\r?\n/)
			.filter((line) => line && !line.startsWith("#"))
			.map((line) => {
				const separator = line.indexOf("=");
				assert.notEqual(separator, -1, `invalid versions.env line: ${line}`);
				return [line.slice(0, separator), line.slice(separator + 1)];
			}),
	);

const versions = parseVersions(
	readRepoFile("packaging", "heic", "versions.env"),
);

const writeExecutable = (file, source) => {
	fs.writeFileSync(file, source);
	fs.chmodSync(file, 0o755);
};

const nativeFixture = ({ spaces = false } = {}) => {
	const repository = fs.mkdtempSync(
		path.join(os.tmpdir(), spaces ? "mote heic build " : "mote-heic-build-"),
	);
	const packaging = path.join(repository, "packaging/heic");
	const scripts = path.join(repository, "scripts");
	const bin = path.join(repository, "bin");
	const temporary = path.join(
		repository,
		spaces ? "managed temporary files" : "temporary",
	);
	const log = path.join(repository, "tool-log");
	for (const directory of [packaging, scripts, bin, temporary, log]) {
		fs.mkdirSync(directory, { recursive: true });
	}
	for (const file of [
		"build-unix.sh",
		"versions.env",
		"verify-native-deps.sh",
		"compiler-path-maps.cmake",
	]) {
		if (!fs.existsSync(path.join(root, "packaging/heic", file))) continue;
		fs.copyFileSync(
			path.join(root, "packaging/heic", file),
			path.join(packaging, file),
		);
	}
	fs.chmodSync(path.join(packaging, "build-unix.sh"), 0o755);
	if (fs.existsSync(path.join(packaging, "verify-native-deps.sh")))
		fs.chmodSync(path.join(packaging, "verify-native-deps.sh"), 0o755);
	fs.copyFileSync(
		path.join(root, "scripts/build-lifecycle.sh"),
		path.join(scripts, "build-lifecycle.sh"),
	);

	writeExecutable(
		path.join(bin, "uname"),
		`#!/bin/sh
case "$1" in
  -s) printf '%s\n' Darwin ;;
  -m) printf '%s\n' arm64 ;;
  *) exit 2 ;;
esac
`,
	);
	writeExecutable(
		path.join(bin, "lipo"),
		`#!/bin/sh
if [ "$1" = -create ]; then
  printf '%s\\n' "$*" >> "$FAKE_TOOL_LOG/lipo"
  while [ "$1" != -output ]; do shift; done
  [ "\${FAKE_LIPO_FAIL:-0}" = 0 ] || exit 43
  printf 'universal' > "$2"
else
  [ "\${FAKE_MISSING_ARCH:-0}" = 0 ] || exit 44
  printf 'arm64 x86_64\\n'
fi
`,
	);
	writeExecutable(path.join(bin, "install_name_tool"), "#!/bin/sh\nexit 0\n");
	writeExecutable(
		path.join(bin, "otool"),
		`#!/bin/sh
for argument in "$@"; do file=$argument; done
printf '%s:\\n' "$file"
case "$*" in
  *' -l '*|'-l '*) printf '' ;;
  *)
    printf '\\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\\n'
    case "$file" in *libheif*) printf '\\t@rpath/libde265.dylib (compatibility version 0.0.0)\\n' ;; esac
    [ -z "\${FAKE_DEPENDENCY:-}" ] || printf '\\t%s (compatibility version 0.0.0)\\n' "$FAKE_DEPENDENCY"
    ;;
esac
`,
	);
	writeExecutable(
		path.join(bin, "nm"),
		`#!/bin/sh
printf '%s\\n' "\${FAKE_SYMBOL:-000 T _de265_decode}"
[ "$1" = -gU ] || printf '%s\\n' "\${FAKE_LOCAL_SYMBOL:-}"
`,
	);
	writeExecutable(
		path.join(bin, "cc"),
		`#!/bin/sh
while [ "$1" != -o ]; do shift; done
printf '#!/bin/sh\\n[ "\${FAKE_ENCODERS:-0}" = 0 ] || exit 45\\n' > "$2"
chmod +x "$2"
`,
	);
	writeExecutable(
		path.join(bin, "curl"),
		`#!/bin/sh
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=$2; shift 2 ;;
    https://*) url=$1; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  *libde265*) printf 'de265' > "$output" ;;
  *libheif*) printf 'heif' > "$output" ;;
  *) exit 40 ;;
esac
printf '%s\n' "$url" >> "$FAKE_TOOL_LOG/curl"
`,
	);
	writeExecutable(
		path.join(bin, "shasum"),
		`#!/bin/sh
for argument in "$@"; do file=$argument; done
case "$(cat "$file")" in
  de265) checksum=${versions.LIBDE265_SHA256} ;;
  heif) checksum=${versions.LIBHEIF_SHA256} ;;
  *) checksum=0000000000000000000000000000000000000000000000000000000000000000 ;;
esac
printf 'verify %s\n' "$file" >> "$FAKE_TOOL_LOG/events"
printf '%s  %s\n' "$checksum" "$file"
`,
	);
	writeExecutable(
		path.join(bin, "tar"),
		`#!/bin/sh
destination=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -C) destination=$2; shift 2 ;;
    *) shift ;;
  esac
done
mkdir -p "$destination"
printf 'extract %s\n' "$destination" >> "$FAKE_TOOL_LOG/events"
printf '%s\n' "$destination" >> "$FAKE_TOOL_LOG/tar"
`,
	);
	writeExecutable(
		path.join(bin, "cmake"),
		`#!/bin/sh
case "$1" in
  -DSOURCE_DIR=*) exit 0 ;;
  --build)
    build=$2
    case "$build" in
      *"\${FAKE_CMAKE_FAIL_BUILD:-never}") exit 31 ;;
    esac
    exit 0
    ;;
  --install)
    build=$2
    prefix=$(cat "$build/install-prefix")
    installed="\${DESTDIR}$prefix"
    mkdir -p "$installed/lib/pkgconfig"
    case "$build" in
      *libde265-build*)
        printf 'shared de265' > "$installed/lib/libde265.dylib"
        printf 'Version: 1.1.1\n' > "$installed/lib/pkgconfig/libde265.pc"
        ;;
      *libheif-build*)
        printf 'shared heif' > "$installed/lib/libheif.dylib"
        printf 'Version: 1.23.4\nRequires.private: libde265\n' > "$installed/lib/pkgconfig/libheif.pc"
        ;;
    esac
    exit 0
    ;;
esac
build=
prefix=
previous=
for argument in "$@"; do
  if [ "$previous" = B ]; then build=$argument; previous=; continue; fi
  case "$argument" in
    -B) previous=B ;;
    -DCMAKE_INSTALL_PREFIX=*) prefix=\${argument#*=} ;;
  esac
done
mkdir -p "$build"
printf '%s' "$prefix" > "$build/install-prefix"
printf '%s\n' 'libde265 HEVC decoder                 : + built-in'
printf '%s\n' 'x265 HEVC encoder                     : - disabled'
`,
	);
	writeExecutable(
		path.join(bin, "mv"),
		`#!/bin/sh
source_path=$1
destination=$2
printf '%s|%s\n' "$source_path" "$destination" >> "$FAKE_TOOL_LOG/mv"
case "$source_path" in
  *stage*) case "$destination" in
  */build/heic-native/macos-*)
    if [ "\${FAKE_MV_FAIL_PUBLICATION:-0}" = 1 ]; then exit 41; fi
    if [ "\${FAKE_MV_PAUSE_PUBLICATION:-0}" = 1 ]; then
      : > "$FAKE_PUBLICATION_READY"
      trap 'exit 143' HUP INT TERM
      while :; do sleep 1; done
    fi
    ;;
  esac ;;
esac
exec /bin/mv "$@"
`,
	);

	return { repository, packaging, bin, temporary, log };
};

const fixturePrefix = ({ repository }) =>
	path.join(repository, "build/heic-native/macos-arm64");

const fixtureEnvironment = (fixture, overrides = {}) => ({
	...process.env,
	PATH: `${fixture.bin}:/usr/bin:/bin`,
	TMPDIR: fixture.temporary,
	FAKE_TOOL_LOG: fixture.log,
	...overrides,
});

const runNativeBuilder = (fixture, overrides = {}, arch = "arm64") =>
	spawnSync(
		path.join(fixture.packaging, "build-unix.sh"),
		["--platform", "macos", "--arch", arch],
		{
			cwd: fixture.repository,
			env: fixtureEnvironment(fixture, overrides),
			encoding: "utf8",
		},
	);

const assertNoEphemeralNativePaths = (fixture) => {
	assert.deepEqual(
		fs
			.readdirSync(fixture.temporary)
			.filter((entry) => entry.startsWith("mote-build-heic-native.")),
		[],
	);
	const nativeRoot = path.join(fixture.repository, "build/heic-native");
	assert.deepEqual(
		fs
			.readdirSync(nativeRoot)
			.filter((entry) => /\.(?:stage|backup)\./.test(entry)),
		[],
	);
};

const waitForFile = (file, timeout = 30_000) =>
	new Promise((resolve, reject) => {
		const started = Date.now();
		const poll = () => {
			if (fs.existsSync(file)) {
				resolve();
				return;
			}
			if (Date.now() - started >= timeout) {
				reject(new Error(`timed out waiting for ${file}`));
				return;
			}
			setTimeout(poll, 20);
		};
		poll();
	});

test("cold cache downloads verify before extraction and publish the platform prefix", () => {
	const fixture = nativeFixture();
	try {
		const result = runNativeBuilder(fixture);
		assert.equal(result.status, 0, result.stderr);
		assert.match(
			result.stdout,
			new RegExp(`${fixturePrefix(fixture)}/lib/pkgconfig`),
		);
		assert.equal(
			fs.readFileSync(path.join(fixture.log, "curl"), "utf8").trim().split("\n")
				.length,
			2,
		);
		assert.deepEqual(
			fs
				.readdirSync(path.join(fixture.repository, "build/heic-native/cache"))
				.sort(),
			[
				`${versions.LIBHEIF_SHA256}.tar.gz`,
				`${versions.LIBDE265_SHA256}.tar.gz`,
			].sort(),
		);
		const events = fs
			.readFileSync(path.join(fixture.log, "events"), "utf8")
			.trim()
			.split("\n");
		assert.equal(
			events.filter((event) => event.startsWith("verify ")).length,
			2,
		);
		assert.equal(
			events.filter((event) => event.startsWith("extract ")).length,
			2,
		);
		assert.ok(
			events.findLastIndex((event) => event.startsWith("verify ")) <
				events.findIndex((event) => event.startsWith("extract ")),
		);
		assert.equal(
			fs.readFileSync(
				path.join(fixturePrefix(fixture), "lib/libheif.dylib"),
				"utf8",
			),
			"shared heif",
		);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("repeated build reuses valid archives and replaces stale or corrupt cache entries", () => {
	const fixture = nativeFixture();
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const cache = path.join(fixture.repository, "build/heic-native/cache");
		fs.writeFileSync(path.join(cache, "stale.tar.gz"), "stale");
		fs.writeFileSync(
			path.join(cache, `${versions.LIBDE265_SHA256}.tar.gz`),
			"corrupt",
		);
		fs.writeFileSync(
			path.join(fixturePrefix(fixture), "previous"),
			"replace me",
		);

		const second = runNativeBuilder(fixture);
		assert.equal(second.status, 0, second.stderr);
		assert.equal(
			fs.readFileSync(path.join(fixture.log, "curl"), "utf8").trim().split("\n")
				.length,
			3,
		);
		assert.deepEqual(
			fs.readdirSync(cache).sort(),
			[
				`${versions.LIBHEIF_SHA256}.tar.gz`,
				`${versions.LIBDE265_SHA256}.tar.gz`,
			].sort(),
		);
		assert.equal(
			fs.existsSync(path.join(fixturePrefix(fixture), "previous")),
			false,
		);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("build failure removes temporary work and preserves the previous prefix", () => {
	const fixture = nativeFixture();
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const marker = path.join(fixturePrefix(fixture), "previous");
		fs.writeFileSync(marker, "keep");

		const failed = runNativeBuilder(fixture, {
			FAKE_CMAKE_FAIL_BUILD: "libheif-build",
		});
		assert.equal(failed.status, 31, failed.stderr);
		assert.equal(fs.readFileSync(marker, "utf8"), "keep");
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("publication failure rolls back the previous prefix", () => {
	const fixture = nativeFixture();
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const marker = path.join(fixturePrefix(fixture), "previous");
		fs.writeFileSync(marker, "keep");

		const failed = runNativeBuilder(fixture, { FAKE_MV_FAIL_PUBLICATION: "1" });
		assert.equal(failed.status, 41, failed.stderr);
		assert.equal(fs.readFileSync(marker, "utf8"), "keep");
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("TERM during publication rolls back and removes all temporary work", async () => {
	const fixture = nativeFixture();
	let child;
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const marker = path.join(fixturePrefix(fixture), "previous");
		fs.writeFileSync(marker, "keep");
		const ready = path.join(fixture.repository, "publication-ready");
		child = spawn(
			path.join(fixture.packaging, "build-unix.sh"),
			["--platform", "macos", "--arch", "arm64"],
			{
				cwd: fixture.repository,
				detached: true,
				env: fixtureEnvironment(fixture, {
					FAKE_MV_PAUSE_PUBLICATION: "1",
					FAKE_PUBLICATION_READY: ready,
				}),
				stdio: "ignore",
			},
		);
		await waitForFile(ready);
		process.kill(-child.pid, "SIGTERM");
		const status = await new Promise((resolve) => child.once("exit", resolve));
		assert.equal(status, 143);
		assert.equal(fs.readFileSync(marker, "utf8"), "keep");
		assertNoEphemeralNativePaths(fixture);
	} finally {
		if (child?.exitCode === null) process.kill(-child.pid, "SIGKILL");
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("TERM at publication activation preserves the previous prefix", () => {
	const fixture = nativeFixture();
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const marker = path.join(fixturePrefix(fixture), "previous");
		fs.writeFileSync(marker, "keep");
		const builder = path.join(fixture.packaging, "build-unix.sh");
		const builderSource = fs.readFileSync(builder, "utf8");
		const activation = "heic_publication_active=1\n";
		assert.equal(builderSource.split(activation).length, 2);
		fs.writeFileSync(
			builder,
			builderSource.replace(activation, () => `${activation}kill -TERM "$$"\n`),
		);

		const interrupted = runNativeBuilder(fixture);
		assert.equal(interrupted.status, 143, interrupted.stderr);
		assert.equal(fs.readFileSync(marker, "utf8"), "keep");
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("native decoder sources are exactly pinned and integrity checked", () => {
	assert.equal(versions.LIBDE265_VERSION, "1.1.1");
	assert.equal(versions.LIBHEIF_VERSION, "1.23.4");
	for (const dependency of ["LIBDE265", "LIBHEIF"]) {
		assert.match(versions[`${dependency}_URL`], /^https:\/\/[^\s]+\.tar\.gz$/);
		assert.match(versions[`${dependency}_SHA256`], /^[0-9a-f]{64}$/);
	}
});

test("Unix builder prints platform-specific caller environment", () => {
	const script = readRepoFile("packaging", "heic", "build-unix.sh");
	assert.match(script, /build\/heic-native\/\$\{platform\}-\$\{arch\}/);
	assert.match(script, /PKG_CONFIG_PATH/);
	assert.match(script, /DYLD_LIBRARY_PATH/);
	assert.match(script, /LD_LIBRARY_PATH/);
});

test("Unix builder configures only an in-process HEVC decoder", () => {
	const script = readRepoFile("packaging", "heic", "build-unix.sh");

	for (const option of [
		"BUILD_SHARED_LIBS=ON",
		"WITH_LIBDE265=ON",
		"WITH_LIBDE265_PLUGIN=OFF",
		"WITH_X265=OFF",
		"ENABLE_ENCODER=OFF",
		"ENABLE_DECODER=OFF",
		"ENABLE_SDL=OFF",
		"WITH_EXAMPLES=OFF",
		"BUILD_TESTING=OFF",
		"ENABLE_EXPERIMENTAL_FEATURES=OFF",
		"ENABLE_PLUGIN_LOADING=OFF",
		"WITH_AOM_DECODER=OFF",
		"WITH_AOM_ENCODER=OFF",
		"WITH_DAV1D=OFF",
		"WITH_FFMPEG_DECODER=OFF",
		"WITH_JPEG_DECODER=OFF",
		"WITH_JPEG_ENCODER=OFF",
		"WITH_KVAZAAR=OFF",
		"WITH_OpenH264_DECODER=OFF",
		"WITH_OpenJPEG_DECODER=OFF",
		"WITH_OpenJPEG_ENCODER=OFF",
		"WITH_OPENJPH_ENCODER=OFF",
		"WITH_RAV1E=OFF",
		"WITH_SvtEnc=OFF",
		"WITH_UNCOMPRESSED_CODEC=OFF",
		"WITH_UVG266=OFF",
		"WITH_VVDEC=OFF",
		"WITH_VVENC=OFF",
		"WITH_WEBCODECS=OFF",
		"WITH_X264=OFF",
	]) {
		assert.match(script, new RegExp(`-D${option}(?:\\s|")`), option);
	}

	assert.match(script, /--warn-uninitialized/);
	assert.match(script, /-Werror=dev/);
	assert.match(script, /Manually-specified variables were not used/);
});

test("native work and archive retention use the managed build lifecycle", () => {
	const script = readRepoFile("packaging", "heic", "build-unix.sh");
	const cleanup = readRepoFile("scripts", "clean-build-assets.sh");

	assert.match(script, /\. "\$repository_root\/scripts\/build-lifecycle\.sh"/);
	assert.match(script, /mote_create_build_dir heic-native/);
	assert.match(script, /trap heic_cleanup_on_exit EXIT/);
	assert.match(script, /mote_cleanup_build_dir/);
	assert.match(script, /prune_archive_cache/);
	assert.match(cleanup, /build\/heic-native/);
	assert.doesNotMatch(
		cleanup,
		/remove_generated_path "\$(?:checkout|repository_root)\/dist" /,
	);
});

test("Windows uses the same pins and decode-only dynamic policy as Unix", () => {
	const windows = readRepoFile("packaging/heic/build-windows.ps1");
	const unix = readRepoFile("packaging/heic/build-unix.sh");
	assert.match(windows, /versions\.env/);
	assert.match(windows, /Get-FileHash.*SHA256/);
	assert.match(windows, /VCPKGRS_DYNAMIC/);
	assert.match(windows, /VCPKG_ROOT/);
	assert.match(windows, /finally/);
	assert.match(windows, /decode-only\.cmake/);
	assert.match(unix, /decode-only\.cmake/);
	for (const option of unix.matchAll(
		/-D((?:WITH_|ENABLE_|BUILD_SHARED|BUILD_TESTING|BUILD_DOCUMENTATION|BUILD_DEVELOPMENT)[A-Za-z0-9_]*=(?:ON|OFF))/g,
	)) {
		assert.ok(windows.includes(`-D${option[1]}`), option[1]);
	}
	for (const script of [unix, windows]) {
		assert.doesNotMatch(script, /embedded-libheif|1\.23\.4|1\.1\.1/);
	}
	assert.ok(
		readRepoFile("crates/codec/src/lib.rs").includes(
			`libheif-${versions.LIBHEIF_VERSION}-libde265-${versions.LIBDE265_VERSION}-sdr-v1`,
		),
	);
});

test("Windows contracts distinguish the de265 import library from its DLL", () => {
	const builder = readRepoFile("packaging/heic/build-windows.ps1");
	const inspector = readRepoFile("packaging/heic/verify-native-deps.ps1");
	assert.doesNotMatch(builder, /libde265\.lib/);
	assert.match(builder, /'de265\.lib'/);
	assert.match(builder, /'libde265\.dll'/);
	assert.match(inspector, /@\('de265', 'libde265'\)/);
});

test("compiler path maps survive spaces in both checkout and managed temporary roots", () => {
	const fixture = nativeFixture({ spaces: true });
	try {
		const realCmake = spawnSync("sh", ["-c", "command -v cmake"], {
			encoding: "utf8",
		}).stdout.trim();
		assert.ok(
			realCmake,
			"CMake is required for the real compiler-argument regression",
		);
		const fakeCmake = path.join(fixture.bin, "cmake-fixture");
		fs.renameSync(path.join(fixture.bin, "cmake"), fakeCmake);
		writeExecutable(
			path.join(fixture.bin, "cmake"),
			`#!${process.execPath}
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const args = process.argv.slice(2);
const run = (command, argv) => {
  const result = spawnSync(command, argv, { encoding: 'utf8', env: { ...process.env, PATH: ${JSON.stringify(process.env.PATH)} } });
  if (result.status !== 0) { process.stderr.write(result.stdout + result.stderr); process.exit(result.status || 1); }
  return result.stdout;
};
if (args.includes('-B')) {
  const build = args[args.indexOf('-B') + 1];
  const source = path.join(build, 'path map check source');
  const nativeRoot = path.join(${JSON.stringify(fixture.repository)}, 'build/heic-native');
  const stage = path.join(nativeRoot, fs.readdirSync(nativeRoot).find(name => name.includes('.stage.')));
  fs.mkdirSync(source, { recursive: true });
  fs.writeFileSync(path.join(source, 'probe.c'), '#include <stdio.h>\\nint main(void) { puts(__FILE__); return 0; }\\n');
  fs.writeFileSync(path.join(stage, 'probe.cpp'), '#include <stdio.h>\\nint main() { puts(__FILE__); return 0; }\\n');
  fs.writeFileSync(path.join(source, 'CMakeLists.txt'), 'cmake_minimum_required(VERSION 3.16)\\nproject(pathmaps LANGUAGES C CXX)\\nadd_executable(c_probe probe.c)\\nadd_executable(cxx_probe "\${CPP_SOURCE}")\\n');
  const options = args.filter(arg => /^-D(CMAKE_C_FLAGS|CMAKE_CXX_FLAGS|CMAKE_PROJECT_INCLUDE|MOTE_HEIC_WORK_ROOT|MOTE_HEIC_STAGE_ROOT)=/.test(arg));
  const output = path.join(build, 'path map check output');
  run(${JSON.stringify(realCmake)}, ['-S', source, '-B', output, '-DCPP_SOURCE=' + path.join(stage, 'probe.cpp'), ...options]);
  run(${JSON.stringify(realCmake)}, ['--build', output]);
  for (const name of ['c_probe', 'cxx_probe']) {
    const filename = run(path.join(output, name), []).trim();
    const expected = name === 'c_probe' ? path.basename(build) + '/path map check source/probe.c' : 'probe.cpp';
    if (filename.replace(/^\\.\\//, '') !== expected) throw new Error('source path was not mapped: ' + filename);
    fs.appendFileSync(${JSON.stringify(path.join(fixture.log, "path-map-probes"))}, filename + '\\n');
  }
}
const result = spawnSync('sh', [${JSON.stringify(fakeCmake)}, ...args], { stdio: 'inherit' });
process.exit(result.status || 0);
`,
		);
		const result = runNativeBuilder(fixture);
		assert.equal(result.status, 0, result.stderr);
		assert.equal(
			fs
				.readFileSync(path.join(fixture.log, "path-map-probes"), "utf8")
				.trim()
				.split("\n").length,
			4,
		);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("decode-only source patch removes the unconditional mask encoder and fails closed on source drift", () => {
	const source = fs.mkdtempSync(path.join(os.tmpdir(), "mote-heic-patch-"));
	try {
		fs.mkdirSync(path.join(source, "libheif/plugins"), { recursive: true });
		const registry = path.join(source, "libheif/plugin_registry.cc");
		const plugins = path.join(source, "libheif/plugins/CMakeLists.txt");
		fs.writeFileSync(
			registry,
			'#include "plugins/encoder_mask.h"\n  register_encoder(get_encoder_plugin_mask());\nkeep_decoder();\n',
		);
		fs.writeFileSync(
			plugins,
			"target_sources(heif PRIVATE\n               encoder_mask.h\n               encoder_mask.cc\n               nalu_utils.cc)\n",
		);
		const patchSource = path.join(root, "packaging/heic/decode-only.cmake");
		const run = () =>
			spawnSync("cmake", [`-DSOURCE_DIR=${source}`, "-P", patchSource], {
				encoding: "utf8",
			});
		const patched = run();
		assert.equal(patched.status, 0, patched.stdout + patched.stderr);
		assert.equal(fs.readFileSync(registry, "utf8"), "keep_decoder();\n");
		assert.equal(
			fs.readFileSync(plugins, "utf8"),
			"target_sources(heif PRIVATE\n               nalu_utils.cc)\n",
		);
		assert.notEqual(
			run().status,
			0,
			"changed upstream source must require an explicit patch update",
		);
	} finally {
		fs.rmSync(source, { recursive: true, force: true });
	}
});

test("every CI platform has enabled fixtures and inspected disabled binaries", () => {
	const ci = readRepoFile(".github/workflows/ci.yml");
	assert.match(ci, /os: \[ubuntu-latest, macos-latest, windows-latest\]/);
	assert.match(ci, /heic: \[enabled, disabled\]/);
	for (const command of [
		/build-unix\.sh/,
		/build-windows\.ps1/,
		/--test heif_backend/,
		/cargo tree[^\n]*--no-default-features --features mote-defaults/,
		/verify-native-deps\.sh[^\n]*--no-heic/,
		/verify-native-deps\.ps1[^\n]*-NoHeic/,
		/clippy[^\n]*--all-features/,
		/test --workspace --all-features/,
	]) {
		assert.match(ci, command);
	}
	const macos = readRepoFile(".github/workflows/build-macos.yml");
	assert.match(
		macos,
		/uses: swatinem\/rust-cache@v2\n\s+with:\n\s+workspaces: apps\/desktop\/src-tauri -> target/,
	);
	assert.match(macos, /heic: \[enabled, disabled\]/);
	assert.match(macos, /build-unix\.sh[^\n]*--arch universal/);
	assert.match(macos, /desktop:build:universal[^\n]*--no-heic/);
	assert.match(macos, /verify-native-deps\.sh/);
});

test("universal builder merges both slices and leaves no per-architecture build trees", () => {
	const fixture = nativeFixture();
	try {
		writeExecutable(
			path.join(fixture.bin, "lipo"),
			`#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_TOOL_LOG/lipo"
if [ "$1" = -create ]; then
  while [ "$1" != -output ]; do shift; done
  printf 'universal' > "$2"
else
  printf 'arm64 x86_64\\n'
fi
`,
		);
		writeExecutable(
			path.join(fixture.bin, "install_name_tool"),
			"#!/bin/sh\nexit 0\n",
		);
		const result = spawnSync(
			path.join(fixture.packaging, "build-unix.sh"),
			["--platform", "macos", "--arch", "universal"],
			{ env: fixtureEnvironment(fixture), encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stderr);
		assert.equal(
			fs.readFileSync(
				path.join(
					fixture.repository,
					"build/heic-native/macos-universal/lib/libheif.dylib",
				),
				"utf8",
			),
			"universal",
		);
		assert.equal(
			fs
				.readFileSync(path.join(fixture.log, "lipo"), "utf8")
				.split("\n")
				.filter((line) => line.startsWith("-create")).length,
			2,
		);
		assertNoEphemeralNativePaths(fixture);
		assert.deepEqual(
			fs.readdirSync(path.join(fixture.repository, "build/heic-native")).sort(),
			["cache", "macos-universal"],
		);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("native inspectors exist for both platform families", () => {
	for (const file of ["verify-native-deps.sh", "verify-native-deps.ps1"]) {
		assert.ok(fs.existsSync(path.join(root, "packaging/heic", file)), file);
	}
});

for (const [name, overrides, remove] of [
	["missing runtime library", { FAKE_DEPENDENCY: "@rpath/missing.dylib" }],
	[
		"absolute build dependency",
		{ FAKE_DEPENDENCY: "/tmp/stage/libheif.dylib" },
	],
	[
		"encoder implementation symbol",
		{ FAKE_SYMBOL: "000 T _en265_new_encoder" },
	],
	["x265 dependency", { FAKE_DEPENDENCY: "@rpath/libx265.dylib" }],
	["missing architecture", { FAKE_MISSING_ARCH: "1" }],
	["missing libde265", {}, "libde265.dylib"],
	[
		"hidden mask encoder",
		{ FAKE_LOCAL_SYMBOL: "000 t __Z23get_encoder_plugin_maskv" },
	],
]) {
	test(`inspection rejects ${name}`, () => {
		const fixture = nativeFixture();
		try {
			const built = runNativeBuilder(fixture);
			assert.equal(built.status, 0, built.stderr);
			if (remove)
				fs.unlinkSync(path.join(fixturePrefix(fixture), "lib", remove));
			const result = spawnSync(
				"sh",
				[
					path.join(fixture.packaging, "verify-native-deps.sh"),
					"--prefix",
					fixturePrefix(fixture),
					"--arch",
					"universal",
				],
				{ env: fixtureEnvironment(fixture, overrides), encoding: "utf8" },
			);
			assert.notEqual(result.status, 0, result.stdout);
		} finally {
			fs.rmSync(fixture.repository, { recursive: true, force: true });
		}
	});
}

test("native build rejects a decoder with runtime encoders before publication", () => {
	const fixture = nativeFixture();
	try {
		const result = runNativeBuilder(fixture, { FAKE_ENCODERS: "1" });
		assert.notEqual(result.status, 0);
		assert.equal(fs.existsSync(fixturePrefix(fixture)), false);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("disabled inspection rejects bundled HEIF libraries even when unused", () => {
	const fixture = nativeFixture();
	try {
		assert.equal(runNativeBuilder(fixture).status, 0);
		const result = spawnSync(
			"sh",
			[
				path.join(fixture.packaging, "verify-native-deps.sh"),
				"--prefix",
				fixturePrefix(fixture),
				"--no-heic",
			],
			{ env: fixtureEnvironment(fixture), encoding: "utf8" },
		);
		assert.notEqual(result.status, 0);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("inspection permits the upstream temporary filename template but rejects embedded build paths", () => {
	const fixture = nativeFixture();
	try {
		assert.equal(runNativeBuilder(fixture).status, 0);
		const library = path.join(fixturePrefix(fixture), "lib/libheif.dylib");
		fs.writeFileSync(library, "/tmp/libheif-XXXXXX\n");
		const inspect = () =>
			spawnSync(
				"sh",
				[
					path.join(fixture.packaging, "verify-native-deps.sh"),
					"--prefix",
					fixturePrefix(fixture),
				],
				{ env: fixtureEnvironment(fixture), encoding: "utf8" },
			);
		assert.equal(inspect().status, 0);
		fs.writeFileSync(
			library,
			"/private/tmp/mote-build-heic-native.abc/source.cc\n",
		);
		assert.notEqual(inspect().status, 0);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("universal merge failure preserves the prior prefix and removes slices and partials", () => {
	const fixture = nativeFixture();
	try {
		const previous = path.join(
			fixture.repository,
			"build/heic-native/macos-universal",
		);
		fs.mkdirSync(previous, { recursive: true });
		fs.writeFileSync(path.join(previous, "previous"), "keep");
		const result = runNativeBuilder(
			fixture,
			{ FAKE_LIPO_FAIL: "1" },
			"universal",
		);
		assert.equal(result.status, 43, result.stderr);
		assert.equal(
			fs.readFileSync(path.join(previous, "previous"), "utf8"),
			"keep",
		);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

test("TERM during universal publication restores the old bundle and cleans both slices", async () => {
	const fixture = nativeFixture();
	let child;
	try {
		const previous = path.join(
			fixture.repository,
			"build/heic-native/macos-universal",
		);
		fs.mkdirSync(previous, { recursive: true });
		fs.writeFileSync(path.join(previous, "previous"), "keep");
		const ready = path.join(fixture.repository, "publication-ready");
		child = spawn(
			path.join(fixture.packaging, "build-unix.sh"),
			["--platform", "macos", "--arch", "universal"],
			{
				detached: true,
				stdio: "ignore",
				env: fixtureEnvironment(fixture, {
					FAKE_MV_PAUSE_PUBLICATION: "1",
					FAKE_PUBLICATION_READY: ready,
				}),
			},
		);
		await waitForFile(ready, 15000);
		process.kill(-child.pid, "SIGTERM");
		const status = await new Promise((resolve) => child.once("exit", resolve));
		assert.equal(status, 143);
		assert.equal(
			fs.readFileSync(path.join(previous, "previous"), "utf8"),
			"keep",
		);
		assertNoEphemeralNativePaths(fixture);
	} finally {
		if (child?.exitCode === null) process.kill(-child.pid, "SIGKILL");
		fs.rmSync(fixture.repository, { recursive: true, force: true });
	}
});

// JavaScript evaluates newline escapes here before PowerShell receives this
// script. PowerShell's single-quoted strings therefore contain real newlines.
const windowsFixtureScript = () => `
$ErrorActionPreference = 'Stop'
$global:buildPrefixes = @{}
$global:publicationFailures = 0
$global:rollbacks = 0
function global:Move-Item {
    param($LiteralPath, $Destination)
    $publishing = (Split-Path $LiteralPath -Leaf) -eq 'vcpkg'
    if ($publishing -and $env:MOTE_TEST_PUBLICATION) {
        $global:publicationFailures++
        if (-not (Test-Path (Join-Path (Split-Path $LiteralPath) 'previous/previous'))) { throw 'backup phase was not reached' }
        if ($env:MOTE_TEST_PUBLICATION -eq 'publication-interrupt') {
            Microsoft.PowerShell.Management\\Move-Item -LiteralPath $LiteralPath -Destination $Destination
            throw [System.OperationCanceledException]::new('interrupted after publication rename')
        }
        throw 'publication rename failed'
    }
    if ((Split-Path $LiteralPath -Leaf) -eq 'previous') { $global:rollbacks++ }
    Microsoft.PowerShell.Management\\Move-Item -LiteralPath $LiteralPath -Destination $Destination
}
function global:cl {
    $libraryPath = ($args | Where-Object { $_ -like '/LIBPATH:*' }) -replace '^/LIBPATH:', ''
    foreach ($library in ($args | Where-Object { $_ -like '*.lib' })) {
        if (-not (Test-Path (Join-Path $libraryPath $library))) { throw "missing probe import library: $library" }
    }
    $probe = ($args | Where-Object { $_ -like '/Fe:*' }) -replace '^/Fe:', ''
    $object = ($args | Where-Object { $_ -like '/Fo:*' }) -replace '^/Fo:', ''
    $source = "$probe.c"
    Set-Content $source '#include <stdlib.h>\nint main(void) { return getenv("MOTE_TEST_ENCODERS") != NULL; }'
    & cl.exe /nologo /MD $source "/Fe:$probe" "/Fo:$object"
}
function global:tar.exe { $global:LASTEXITCODE = 0 }
function global:Invoke-WebRequest { param($Uri, $OutFile); Set-Content $OutFile 'archive' }
function global:Get-FileHash { param($LiteralPath, $Algorithm); @{ Hash = if ($LiteralPath -like '*${versions.LIBDE265_SHA256}*') { '${versions.LIBDE265_SHA256}' } else { '${versions.LIBHEIF_SHA256}' } } }
function global:cmake {
    $global:LASTEXITCODE = 0
    if ($args -contains '-P') { return }
    if ($args[0] -eq '--build') { return }
    if ($args[0] -eq '--install') {
        $prefix = $global:buildPrefixes[$args[1]]
        New-Item -ItemType Directory -Force "$prefix/lib/pkgconfig", "$prefix/lib/cmake", "$prefix/bin" | Out-Null
        $import, $name = if ($args[1] -like '*de265-build') { @('de265', 'libde265') } else { @('heif', 'heif') }
        Set-Content "$prefix/lib/$import.lib" 'import library'
        Set-Content "$prefix/bin/$name.dll" 'shared library'
        Set-Content "$prefix/lib/pkgconfig/$name.pc" "prefix=$prefix"
        return
    }
    $build = $args[[array]::IndexOf($args, '-B') + 1]
    $prefix = ($args | Where-Object { $_ -like '-DCMAKE_INSTALL_PREFIX=*' }) -replace '^-DCMAKE_INSTALL_PREFIX=', ''
    $global:buildPrefixes[$build] = $prefix
    foreach ($library in ($args | Where-Object { $_ -like '-DLIBDE265_LIBRARY=*' })) {
        if (-not (Test-Path ($library -replace '^-DLIBDE265_LIBRARY=', ''))) { throw 'missing CMake de265 import library' }
    }
    # Use the production options with real CMake/MSVC, not a string assertion.
    # Both source roots contain spaces; /FC makes __FILE__ an absolute path.
    if (-not $global:pathMapsChecked) {
        $source = Join-Path $build 'path map check source'
        $output = Join-Path $build 'path map check output'
        New-Item -ItemType Directory -Force $source, $prefix | Out-Null
        $program = '#include <stdio.h>\nint main(void) { puts(__FILE__); return 0; }'
        Set-Content (Join-Path $source 'probe.c') $program
        Set-Content (Join-Path $prefix 'probe.cpp') $program
        Set-Content (Join-Path $source 'CMakeLists.txt') 'cmake_minimum_required(VERSION 3.16)\nproject(pathmaps LANGUAGES C CXX)\nadd_compile_options(/FC /WX)\nadd_executable(c_probe probe.c)\nadd_executable(cxx_probe "\${CPP_SOURCE}")'
        $maps = @($args | Where-Object { $_ -match '^-D(CMAKE_C_FLAGS|CMAKE_CXX_FLAGS|CMAKE_PROJECT_INCLUDE|MOTE_HEIC_WORK_ROOT|MOTE_HEIC_STAGE_ROOT)=' })
        & cmake.exe -G 'NMake Makefiles' -S $source -B $output "-DCPP_SOURCE=$(Join-Path $prefix 'probe.cpp')" @maps
        if ($LASTEXITCODE -ne 0) { throw 'path map configuration failed' }
        & cmake.exe --build $output
        if ($LASTEXITCODE -ne 0) { throw 'path map compilation failed' }
        foreach ($probe in @('c_probe.exe', 'cxx_probe.exe')) {
            $mapped = & (Join-Path $output $probe)
            if ($LASTEXITCODE -ne 0 -or [System.IO.Path]::IsPathRooted($mapped) -or $mapped -notmatch 'probe\\.c(pp)?$') { throw "unmapped compiler source path: $mapped" }
        }
        Remove-Item -LiteralPath (Join-Path $prefix 'probe.cpp')
        $global:pathMapsChecked = $true
    }
    if ($build -like '*heif-build' -and $env:MOTE_TEST_FAILURE) {
        if ($env:MOTE_TEST_FAILURE -eq 'interrupt') { throw [System.OperationCanceledException]::new('interrupted') }
        $global:LASTEXITCODE = 31
        return
    }
    'libde265 HEVC decoder : + built-in'
    'x265 HEVC encoder : - disabled'
}
function global:dumpbin {
    $global:LASTEXITCODE = 0
    $file = $args[-1]
    if (-not (Test-Path $file)) { throw "dumpbin input missing: $file" }
    switch ($args[1]) {
        '/headers' { '8664 machine (x64)'; if ($file -like '*heif.lib') { 'heif.dll' }; if ($file -like '*de265.lib') { 'libde265.dll' } }
        '/dependents' { '    KERNEL32.dll'; if ($file -like '*heif.dll') { '    libde265.dll' } }
        '/exports' { '    de265_decode' }
    }
}
$builder = Join-Path $PSScriptRoot 'packaging/heic/build-windows.ps1'
$env:GITHUB_ENV = ''
$global:pathMapsChecked = $false
& $builder
$nativeRoot = Join-Path $PSScriptRoot 'build/heic-native'
$marker = Join-Path $nativeRoot 'windows-x64/previous'
Set-Content $marker 'keep'
foreach ($failure in @('build', 'interrupt', 'runtime', 'publication', 'publication-interrupt')) {
    $env:MOTE_TEST_FAILURE = if ($failure -in @('build', 'interrupt')) { $failure } else { '' }
    $env:MOTE_TEST_PUBLICATION = if ($failure -like 'publication*') { $failure } else { '' }
    $env:MOTE_TEST_ENCODERS = if ($failure -eq 'runtime') { '1' } else { '' }
    $failed = $false
    try { & $builder } catch { $failed = $true }
    if (-not $failed) { throw 'failure was ignored' }
    if ((Get-Content $marker) -ne 'keep') { throw 'previous prefix lost' }
    if (@(Get-ChildItem $nativeRoot -Force | Where-Object Name -Like '*.stage.*').Count) { throw 'staging leaked' }
}
$env:MOTE_TEST_FAILURE = ''
$env:MOTE_TEST_ENCODERS = ''
$env:MOTE_TEST_PUBLICATION = ''
if ($global:publicationFailures -ne 2 -or $global:rollbacks -ne 2) { throw 'publication rollback phases not exercised' }
& $builder
if (Test-Path $marker) { throw 'old prefix was not replaced' }
if (@(Get-ChildItem $nativeRoot -Force | Where-Object Name -Like '*.stage.*').Count) { throw 'staging leaked' }
if (@(Get-ChildItem "$nativeRoot/cache").Count -ne 2) { throw 'cache is unbounded' }
`;

test("Windows fixture emits real newlines in its C, C++ and CMake inputs", () => {
	const assertMultilineInputs = (source) => {
		for (const [pattern, lines] of [
			[/Set-Content \$source '(#include <stdlib.h>[^']+)'/, 2],
			[/\$program = '(#include <stdio.h>[^']+)'/, 2],
			[/Set-Content \(Join-Path \$source 'CMakeLists.txt'\) '([^']+)'/, 5],
		]) {
			const emitted = source.match(pattern)?.[1];
			assert.equal(typeof emitted, "string");
			assert.equal(emitted.split("\n").length, lines);
			assert.doesNotMatch(emitted, /\\n/);
		}
		assert.match(source, /add_executable\(cxx_probe "\$\{CPP_SOURCE\}"\)/);
	};
	const source = windowsFixtureScript();
	assertMultilineInputs(source);
	// A double-escaped JavaScript newline would leave PowerShell a literal \\n.
	assert.throws(() => assertMultilineInputs(source.replaceAll("\n", "\\n")));
});

test("Windows builder publishes discoverable DLL metadata and cleans failed or interrupted work", {
	skip: process.platform !== "win32",
}, () => {
	const repository = fs.mkdtempSync(
		path.join(os.tmpdir(), "mote heic windows "),
	);
	try {
		const packaging = path.join(repository, "packaging/heic");
		const licenses = path.join(repository, "packaging/licenses");
		fs.mkdirSync(packaging, { recursive: true });
		fs.mkdirSync(licenses, { recursive: true });
		fs.copyFileSync(
			path.join(root, "THIRD_PARTY_NOTICES.md"),
			path.join(repository, "THIRD_PARTY_NOTICES.md"),
		);
		for (const name of ["LGPL-3.0-or-later.txt", "libheif.md", "libde265.md"])
			fs.copyFileSync(
				path.join(root, "packaging/licenses", name),
				path.join(licenses, name),
			);
		for (const name of [
			"versions.env",
			"build-windows.ps1",
			"verify-native-deps.ps1",
			"decode-only.cmake",
			"verify-decoder.c",
			"compiler-path-maps.cmake",
			"README.md",
		])
			fs.copyFileSync(
				path.join(root, "packaging/heic", name),
				path.join(packaging, name),
			);
		const script = path.join(repository, "test.ps1");
		fs.writeFileSync(script, windowsFixtureScript());
		const result = spawnSync("pwsh", ["-NoProfile", "-File", script], {
			cwd: repository,
			encoding: "utf8",
			timeout: 60_000,
		});
		assert.equal(result.status, 0, result.stdout + result.stderr);
		const status = fs.readFileSync(
			path.join(
				repository,
				"build/heic-native/windows-x64/installed/vcpkg/status",
			),
			"utf8",
		);
		assert.match(status, /Package: libheif/);
		assert.match(status, /Depends: libde265/);
		for (const name of [
			"THIRD_PARTY_NOTICES.md",
			"LGPL-3.0-or-later.txt",
			"libheif.md",
			"libde265.md",
			"HEIC-REBUILD.md",
			"decode-only.cmake",
		])
			assert.ok(
				fs.existsSync(
					path.join(
						repository,
						"build/heic-native/windows-x64/installed/x64-windows/share/licenses/mote",
						name,
					),
				),
				name,
			);
		assert.equal(
			fs.readFileSync(
				path.join(
					repository,
					"build/heic-native/windows-x64/installed/x64-windows/share/licenses/mote/decode-only.cmake",
				),
				"utf8",
			),
			fs.readFileSync(
				path.join(repository, "packaging/heic/decode-only.cmake"),
				"utf8",
			),
		);
		for (const [name, version, importLibrary, runtimeLibrary] of [
			["libheif", versions.LIBHEIF_VERSION, "heif", "heif"],
			["libde265", versions.LIBDE265_VERSION, "de265", "libde265"],
		]) {
			const manifest = fs.readFileSync(
				path.join(
					repository,
					`build/heic-native/windows-x64/installed/vcpkg/info/${name}_${version}_x64-windows.list`,
				),
				"utf8",
			);
			assert.match(
				manifest,
				new RegExp(`x64-windows/lib/${importLibrary}\\.lib`),
			);
			assert.match(
				manifest,
				new RegExp(`x64-windows/bin/${runtimeLibrary}\\.dll`),
			);
			for (const entry of manifest.trim().split(/\r?\n/)) {
				assert.ok(
					fs.existsSync(
						path.join(
							repository,
							"build/heic-native/windows-x64/installed",
							entry.trim(),
						),
					),
				);
			}
		}
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

for (const [name, environment] of [
	["wrong ELF architecture", { FAKE_ELF_MACHINE: "AArch64" }],
	["absolute ELF dependency", { FAKE_ELF_NEEDED: "/tmp/stage/libde265.so.0" }],
]) {
	test(`Linux inspection rejects ${name}`, () => {
		const fixture = nativeFixture();
		try {
			const prefix = fixturePrefix(fixture);
			fs.mkdirSync(path.join(prefix, "lib"), { recursive: true });
			for (const name of ["libheif.so", "libde265.so"])
				fs.writeFileSync(path.join(prefix, "lib", name), "shared library");
			writeExecutable(
				path.join(fixture.bin, "uname"),
				'#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; esac\n',
			);
			writeExecutable(
				path.join(fixture.bin, "ldd"),
				'#!/bin/sh\nprintf "libde265.so.0 => /runtime/libde265.so.0 (0x1)\\nlibc.so.6 => /lib/libc.so.6 (0x2)\\n"\n',
			);
			writeExecutable(
				path.join(fixture.bin, "readelf"),
				`#!/bin/sh
if [ "$1" = -h ]; then printf 'Machine: %s\\n' "\${FAKE_ELF_MACHINE:-Advanced Micro Devices X86-64}"; else
printf '0 (NEEDED) Shared library: [%s]\\n' "\${FAKE_ELF_NEEDED:-libde265.so.0}"
fi
`,
			);
			const inspect = (overrides) =>
				spawnSync(
					"sh",
					[
						path.join(fixture.packaging, "verify-native-deps.sh"),
						"--prefix",
						prefix,
					],
					{ env: fixtureEnvironment(fixture, overrides), encoding: "utf8" },
				);
			assert.equal(inspect({}).status, 0);
			assert.notEqual(inspect(environment).status, 0);
		} finally {
			fs.rmSync(fixture.repository, { recursive: true, force: true });
		}
	});
}
