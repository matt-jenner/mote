import assert from "node:assert/strict";
import fs from "node:fs";
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

test("native decoder sources are exactly pinned and integrity checked", () => {
	const versions = parseVersions(
		readRepoFile("packaging", "heic", "versions.env"),
	);

	assert.equal(versions.LIBDE265_VERSION, "1.1.1");
	assert.equal(versions.LIBHEIF_VERSION, "1.23.4");
	for (const dependency of ["LIBDE265", "LIBHEIF"]) {
		assert.match(versions[`${dependency}_URL`], /^https:\/\/[^\s]+\.tar\.gz$/);
		assert.match(versions[`${dependency}_SHA256`], /^[0-9a-f]{64}$/);
	}
});

test("Unix builder verifies archives before extracting them", () => {
	const script = readRepoFile("packaging", "heic", "build-unix.sh");
	const verification = script.indexOf("verify_archive");
	const extraction = script.indexOf("extract_archive");

	assert.notEqual(verification, -1);
	assert.notEqual(extraction, -1);
	assert.ok(
		verification < extraction,
		"archive extraction precedes verification",
	);
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
	assert.match(script, /mote_install_cleanup_traps/);
	assert.match(script, /prune_archive_cache/);
	assert.match(cleanup, /build\/heic-native/);
	assert.doesNotMatch(
		cleanup,
		/remove_generated_path "\$(?:checkout|repository_root)\/dist" /,
	);
});
