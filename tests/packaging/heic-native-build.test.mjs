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

const nativeFixture = () => {
	const repository = fs.mkdtempSync(path.join(os.tmpdir(), "mote-heic-build-"));
	const packaging = path.join(repository, "packaging/heic");
	const scripts = path.join(repository, "scripts");
	const bin = path.join(repository, "bin");
	const temporary = path.join(repository, "temporary");
	const log = path.join(repository, "tool-log");
	for (const directory of [packaging, scripts, bin, temporary, log]) {
		fs.mkdirSync(directory, { recursive: true });
	}
	for (const file of ["build-unix.sh", "versions.env"]) {
		fs.copyFileSync(
			path.join(root, "packaging/heic", file),
			path.join(packaging, file),
		);
	}
	fs.chmodSync(path.join(packaging, "build-unix.sh"), 0o755);
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

const runNativeBuilder = (fixture, overrides = {}) =>
	spawnSync(
		path.join(fixture.packaging, "build-unix.sh"),
		["--platform", "macos", "--arch", "arm64"],
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

const waitForFile = (file, timeout = 5_000) =>
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
	try {
		const first = runNativeBuilder(fixture);
		assert.equal(first.status, 0, first.stderr);
		const marker = path.join(fixturePrefix(fixture), "previous");
		fs.writeFileSync(marker, "keep");
		const ready = path.join(fixture.repository, "publication-ready");
		const child = spawn(
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
			builderSource.replace(
				activation,
				() => `${activation}kill -TERM "$$"\n`,
			),
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
