import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { parse } from "yaml";

const root = path.resolve(import.meta.dirname, "../..");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");

const materialSwitches = (source) =>
	[
		...source.matchAll(
			/-D(BUILD_(?:SHARED_LIBS|TESTING|DOCUMENTATION|DEVELOPMENT_TOOLS)|ENABLE_[A-Za-z0-9_]+|WITH_[A-Za-z0-9_]+|USE_IWYU|FORCE_FULL_VISIBILITY)=([A-Za-z0-9_+.-]+)/g,
		),
	].map((match) => `${match[1]}=${match[2]}`);

function documentedSwitches(library) {
	const guide = read("packaging/heic/README.md");
	const body = guide.match(
		new RegExp(
			`<!-- BEGIN ${library} material switches -->([\\s\\S]*?)<!-- END ${library} material switches -->`,
		),
	)?.[1];
	assert.ok(body, `missing ${library} material-switch inventory`);
	return materialSwitches(body).sort();
}

test("the material-switch parser covers every BUILD policy control", () => {
	assert.deepEqual(
		materialSwitches(
			[
				"-DBUILD_SHARED_LIBS=ON",
				"-DBUILD_TESTING=OFF",
				"-DBUILD_DOCUMENTATION=OFF",
				"-DBUILD_DEVELOPMENT_TOOLS=OFF",
				"-DCMAKE_BUILD_TYPE=Release",
			].join("\n"),
		),
		[
			"BUILD_SHARED_LIBS=ON",
			"BUILD_TESTING=OFF",
			"BUILD_DOCUMENTATION=OFF",
			"BUILD_DEVELOPMENT_TOOLS=OFF",
		],
	);
});

function nativePins() {
	return Object.fromEntries(
		read("packaging/heic/versions.env")
			.split(/\r?\n/)
			.filter((line) => line && !line.startsWith("#"))
			.map((line) => line.split("=", 2)),
	);
}

test("dependency notices reproduce every verified native source pin", () => {
	const pins = nativePins();
	const lock = JSON.parse(
		read("packaging/flatpak/generated/source-lock.json"),
	).native;
	const notice = read("THIRD_PARTY_NOTICES.md");
	const records = {
		LIBHEIF: read("packaging/licenses/libheif.md"),
		LIBDE265: read("packaging/licenses/libde265.md"),
	};

	for (const [key, lockKey] of [
		["LIBHEIF", "libheif"],
		["LIBDE265", "libde265"],
	]) {
		assert.equal(lock[lockKey].version, pins[`${key}_VERSION`]);
		assert.equal(lock[lockKey].url, pins[`${key}_URL`]);
		assert.equal(lock[lockKey].sha256, pins[`${key}_SHA256`]);
		for (const value of [
			pins[`${key}_VERSION`],
			pins[`${key}_URL`],
			pins[`${key}_SHA256`],
		]) {
			assert.ok(records[key].includes(value), `${key} omits ${value}`);
			assert.ok(notice.includes(value), `notices omit ${value}`);
		}
		assert.match(records[key], /LGPL-3\.0-or-later/);
		assert.match(records[key], /copyright/i);
	}
});

test("compliance records enumerate patches, decode-only switches, and source offer", () => {
	const heif = read("packaging/licenses/libheif.md");
	const de265 = read("packaging/licenses/libde265.md");
	const rebuild = read("packaging/heic/README.md");
	const notice = read("THIRD_PARTY_NOTICES.md");

	assert.match(
		heif,
		/Patches applied[\s\S]*packaging\/heic\/decode-only\.cmake/i,
	);
	assert.match(de265, /Patches applied:\s*none\b/i);
	for (const option of [
		"-DBUILD_SHARED_LIBS=ON",
		"-DWITH_LIBDE265=ON",
		"-DWITH_LIBDE265_PLUGIN=OFF",
		"-DWITH_X265=OFF",
		"-DENABLE_PLUGIN_LOADING=OFF",
		"-DENABLE_ENCODER=OFF",
	])
		assert.ok(`${heif}\n${de265}\n${rebuild}`.includes(option), option);
	assert.match(rebuild, /build-unix\.sh/);
	assert.match(rebuild, /build-windows\.ps1/);
	assert.match(rebuild, /macOS[\s\S]*Contents\/Frameworks/i);
	assert.match(rebuild, /Windows[\s\S]*(?:heif\.dll|libde265\.dll)/i);
	assert.match(rebuild, /Linux[\s\S]*\/usr\/local\/lib/i);
	assert.match(rebuild, /relink|replace/i);
	assert.match(
		`${notice}\n${rebuild}`,
		/complete corresponding source|source archives/i,
	);
	assert.match(`${notice}\n${rebuild}`, /decod(?:e|ing)[ -]only/i);
	assert.match(
		`${notice}\n${rebuild}`,
		/x265[\s\S]*(?:absent|not shipped|disabled)/i,
	);
	assert.match(`${notice}\n${rebuild}`, /patent/i);
	assert.doesNotMatch(
		`${notice}\n${rebuild}`,
		/(?:open[ -]source|non[ -]commercial)[^\n.]{0,80}(?:eliminates|resolves|waives)[^\n.]{0,40}patent/i,
	);
});

test("the complete LGPLv3 and incorporated GPLv3 text is shipped", () => {
	const licence = read("packaging/licenses/LGPL-3.0-or-later.txt");
	assert.match(
		licence,
		/GNU LESSER GENERAL PUBLIC LICENSE\s+Version 3, 29 June 2007/,
	);
	assert.match(licence, /GNU GENERAL PUBLIC LICENSE\s+Version 3, 29 June 2007/);
	assert.match(
		licence,
		/This version of the GNU Lesser General Public License incorporates/,
	);
	assert.match(licence, /How to Apply These Terms to Your New Programs/);
	assert.ok(licence.length > 30_000, "licence text is unexpectedly truncated");
});

test("every enabled package path installs the same compliance set", () => {
	const required = [
		"THIRD_PARTY_NOTICES.md",
		"LGPL-3.0-or-later.txt",
		"libheif.md",
		"libde265.md",
		"HEIC-REBUILD.md",
		"decode-only.cmake",
	];
	const desktop = read("scripts/desktop-build.sh");
	const windows = read("packaging/heic/build-windows.ps1");
	const container = read("Containerfile");
	for (const name of required) {
		assert.ok(desktop.includes(name), `macOS package omits ${name}`);
		assert.ok(windows.includes(name), `Windows staging omits ${name}`);
		assert.ok(container.includes(name), `hosted image omits ${name}`);
	}
	assert.match(desktop, /Contents\/Resources\/licenses/);
	assert.match(windows, /share[\\/]licenses[\\/]mote/i);
	assert.match(container, /\/usr\/share\/licenses\/mote/);
	assert.match(
		desktop,
		/cp "\$repository_root\/packaging\/heic\/decode-only\.cmake" "\$licenses\/decode-only\.cmake"/,
	);
	assert.match(
		windows,
		/Copy-Item \(Join-Path \$PSScriptRoot 'decode-only\.cmake'\) \(Join-Path \$licenseDestination 'decode-only\.cmake'\)/,
	);
	assert.match(
		container,
		/install -Dm0644 packaging\/heic\/decode-only\.cmake \/runtime\/usr\/share\/licenses\/mote\/decode-only\.cmake/,
	);

	const template = parse(
		read("packaging/flatpak/io.github.matt_jenner.mote.yml"),
	);
	const heif = template.modules.find(({ name }) => name === "libheif");
	const installed = (heif["post-install"] ?? []).join("\n");
	const availableSources = new Set(
		heif.sources
			.filter((source) => source.path)
			.map((source) => source["dest-filename"] ?? path.basename(source.path)),
	);
	for (const name of required) {
		assert.ok(availableSources.has(name), `Flatpak source omits ${name}`);
		assert.ok(installed.includes(name), `Flatpak install omits ${name}`);
	}
	assert.match(
		installed,
		/\/app\/share\/licenses\/io\.github\.matt_jenner\.mote/,
	);
});

test("the shipped rebuild record matches every material builder switch", () => {
	const unix = read("packaging/heic/build-unix.sh");
	const windows = read("packaging/heic/build-windows.ps1");
	const windowsCommon = windows.match(
		/\$common = @\(([\s\S]*?)\)\s*\n\s*\$de265Build/,
	)?.[1];
	assert.ok(windowsCommon, "cannot isolate common Windows configuration");
	const manifest = parse(
		read("packaging/flatpak/io.github.matt_jenner.mote.yml"),
	);
	for (const [library, nextUnix, nextWindows] of [
		["libde265", "cmake --build", "Invoke-Native 'cmake'"],
		["libheif", "grep -E", "Invoke-Native 'cmake'"],
	]) {
		const unixBlock = unix.match(
			new RegExp(
				`configure_cmake ${library}([\\s\\S]*?)${nextUnix.replaceAll(" ", "\\s+")}`,
			),
		)?.[1];
		const windowsBlock = windows.match(
			new RegExp(
				`Configure-Native '${library}'([\\s\\S]*?)${nextWindows.replaceAll(" ", "\\s+")}`,
			),
		)?.[1];
		assert.ok(unixBlock, `cannot isolate Unix ${library} configuration`);
		assert.ok(windowsBlock, `cannot isolate Windows ${library} configuration`);
		const expected = documentedSwitches(library);
		for (const [builder, options] of [
			["Unix", materialSwitches(unixBlock).sort()],
			["Windows", materialSwitches(`${windowsCommon}\n${windowsBlock}`).sort()],
			[
				"Flatpak",
				materialSwitches(
					manifest.modules
						.find(({ name }) => name === library)
						["config-opts"].join("\n"),
				).sort(),
			],
		])
			assert.deepEqual(options, expected, `${builder} ${library} drifted`);
	}
});

test("disabled Flatpak rendering omits HEIC libraries and their notices", () => {
	const renderer = read("scripts/render-flatpak-manifest.mjs");
	assert.match(renderer, /filter\(\(\{ name \}\) => name === "mote"\)/);
	const template = parse(
		read("packaging/flatpak/io.github.matt_jenner.mote.yml"),
	);
	const appCommands = template.modules
		.find(({ name }) => name === "mote")
		["build-commands"].join("\n");
	assert.doesNotMatch(
		appCommands,
		/LGPL-3\.0-or-later|libheif\.md|libde265\.md/,
	);
});

test("artifact audit rejects encoder implementations but permits ABI declarations", () => {
	for (const file of [
		"packaging/heic/verify-native-deps.sh",
		"packaging/heic/verify-native-deps.ps1",
		"packaging/heic/verify-linux-runtime.sh",
	]) {
		const verifier = read(file);
		assert.match(verifier, /x265/);
		assert.match(verifier, /heif-enc/);
		assert.match(verifier, /plugin/i);
		assert.doesNotMatch(verifier, /forbidden[^\n]*heif_encoder/i);
	}
	const record = read("packaging/heic/README.md");
	assert.match(record, /generic exported encoder API symbols/i);
	assert.match(record, /encoder (?:implementation|library|plugin|tool)/i);
});
