import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const lifecycle = path.join(root, "scripts/build-lifecycle.sh");

function fixture() {
	return fs.mkdtempSync(path.join(os.tmpdir(), "mote-lifecycle-test-"));
}

function run(source, tmpdir) {
	return spawnSync("sh", ["-c", `. "${lifecycle}"\n${source}`], {
		env: { ...process.env, TMPDIR: tmpdir },
		encoding: "utf8",
	});
}

function waitForFile(file, timeout = 5_000) {
	const started = Date.now();
	return new Promise((resolve, reject) => {
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
}

test("managed build directory is removed after success", () => {
	const tmpdir = fixture();
	try {
		const result = run(
			'mote_create_build_dir unit\nprintf \'%s\' "$MOTE_BUILD_DIR" > "$TMPDIR/path"\nmote_install_cleanup_traps',
			tmpdir,
		);
		assert.equal(result.status, 0, result.stderr);
		const build = fs.readFileSync(path.join(tmpdir, "path"), "utf8");
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("cleanup preserves a wrapped failure status", () => {
	const tmpdir = fixture();
	try {
		const result = run(
			'mote_create_build_dir unit\nprintf \'%s\' "$MOTE_BUILD_DIR" > "$TMPDIR/path"\nmote_install_cleanup_traps\nexit 23',
			tmpdir,
		);
		assert.equal(result.status, 23, result.stderr);
		const build = fs.readFileSync(path.join(tmpdir, "path"), "utf8");
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("TERM removes the managed build directory and returns 143", async () => {
	const tmpdir = fixture();
	const pathFile = path.join(tmpdir, "path");
	try {
		const child = spawn(
			"sh",
			[
				"-c",
				`. "${lifecycle}"\nmote_create_build_dir unit\nprintf '%s' "$MOTE_BUILD_DIR" > "$TMPDIR/path"\nmote_install_cleanup_traps\nwhile :; do sleep 1; done`,
			],
			{ env: { ...process.env, TMPDIR: tmpdir }, stdio: "ignore" },
		);
		await waitForFile(pathFile);
		const build = fs.readFileSync(pathFile, "utf8");
		child.kill("SIGTERM");
		const status = await new Promise((resolve) => child.once("exit", resolve));
		assert.equal(status, 143);
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("stale reaping preserves a live PID lease", () => {
	const tmpdir = fixture();
	const build = path.join(tmpdir, "mote-build-unit.live");
	fs.mkdirSync(build);
	fs.writeFileSync(path.join(build, ".mote-owner-pid"), `${process.pid}\n`);
	try {
		const result = run("mote_reap_stale_builds", tmpdir);
		assert.equal(result.status, 0, result.stderr);
		assert.equal(fs.existsSync(build), true);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("stale reaping removes a dead PID lease", () => {
	const tmpdir = fixture();
	const build = path.join(tmpdir, "mote-build-unit.dead");
	fs.mkdirSync(build);
	fs.writeFileSync(path.join(build, ".mote-owner-pid"), "999999999\n");
	try {
		const result = run("mote_reap_stale_builds", tmpdir);
		assert.equal(result.status, 0, result.stderr);
		assert.equal(fs.existsSync(build), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("stale reaping skips a malformed lease", () => {
	const tmpdir = fixture();
	const build = path.join(tmpdir, "mote-build-unit.invalid");
	fs.mkdirSync(build);
	fs.writeFileSync(path.join(build, ".mote-owner-pid"), "not-a-pid\n");
	try {
		const result = run("mote_reap_stale_builds", tmpdir);
		assert.equal(result.status, 0, result.stderr);
		assert.match(result.stderr, /invalid owner/);
		assert.equal(fs.existsSync(build), true);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("managed removal rejects paths outside the exact prefix", () => {
	const tmpdir = fixture();
	const managedRoot = path.join(tmpdir, "managed");
	const outside = path.join(tmpdir, "outside");
	fs.mkdirSync(managedRoot);
	fs.mkdirSync(outside);
	try {
		const result = run(
			`mote_remove_managed_path "${outside}" "${managedRoot}"`,
			tmpdir,
		);
		assert.notEqual(result.status, 0);
		assert.match(result.stderr, /unmanaged path/);
		assert.equal(fs.existsSync(outside), true);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("ephemeral Cargo wrapper removes the target and returns the child status", () => {
	const tmpdir = fixture();
	const bin = path.join(tmpdir, "bin");
	fs.mkdirSync(bin);
	const command = path.join(bin, "fake-command");
	fs.writeFileSync(
		command,
		'#!/bin/sh\nprintf \'%s\' "$CARGO_TARGET_DIR" > "$TMPDIR/cargo-path"\nmkdir -p "$CARGO_TARGET_DIR"\nexit 19\n',
	);
	fs.chmodSync(command, 0o755);
	try {
		const result = spawnSync(
			path.join(root, "scripts/with-ephemeral-cargo-target.sh"),
			[command],
			{ env: { ...process.env, TMPDIR: tmpdir }, encoding: "utf8" },
		);
		assert.equal(result.status, 19, result.stderr);
		const target = fs.readFileSync(path.join(tmpdir, "cargo-path"), "utf8");
		assert.equal(fs.existsSync(target), false);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

test("Rust verification shares one temporary target and removes it", () => {
	const tmpdir = fixture();
	const bin = path.join(tmpdir, "bin");
	const log = path.join(tmpdir, "cargo-paths");
	fs.mkdirSync(bin);
	const cargo = path.join(bin, "cargo");
	fs.writeFileSync(
		cargo,
		'#!/bin/sh\nprintf \'%s|%s\\n\' "$CARGO_TARGET_DIR" "$*" >> "$TMPDIR/cargo-paths"\nmkdir -p "$CARGO_TARGET_DIR"\ntouch "$CARGO_TARGET_DIR/output"\n',
	);
	fs.chmodSync(cargo, 0o755);
	try {
		const result = spawnSync(path.join(root, "scripts/rust-verify.sh"), [], {
			env: {
				...process.env,
				TMPDIR: tmpdir,
				PATH: `${bin}:${process.env.PATH}`,
			},
			encoding: "utf8",
		});
		assert.equal(result.status, 0, result.stderr);
		const calls = fs.readFileSync(log, "utf8").trim().split("\n");
		assert.equal(calls.length, 4);
		const targets = new Set(calls.map((call) => call.split("|")[0]));
		assert.equal(targets.size, 1);
		const [target] = targets;
		assert.equal(fs.existsSync(target), false);
		assert.match(calls[0], /\|fmt --all --check$/);
		assert.match(
			calls[1],
			/\|clippy --workspace --all-targets --all-features -- -D warnings$/,
		);
		assert.match(calls[2], /\|test --workspace --all-features$/);
		assert.match(calls[3], /\|test -p catalog-bench --test benchmark_smoke$/);
	} finally {
		fs.rmSync(tmpdir, { recursive: true, force: true });
	}
});

function desktopFixture({
	existingMarker = "accepted",
	buildStatus = 0,
	invalid = "",
	inspectionStatus = 0,
	signStatus = 0,
} = {}) {
	const repository = fixture();
	const scripts = path.join(repository, "scripts");
	const bin = path.join(repository, "bin");
	fs.mkdirSync(scripts);
	fs.mkdirSync(bin);
	for (const name of [
		"build-lifecycle.sh",
		"cargo-feature-mode.sh",
		"macos-deployment-target.sh",
	]) {
		const source = path.join(root, "scripts", name);
		if (fs.existsSync(source)) {
			fs.copyFileSync(source, path.join(scripts, name));
		}
	}
	const config = path.join(
		repository,
		"apps/desktop/src-tauri/tauri.conf.json",
	);
	fs.mkdirSync(path.dirname(config), { recursive: true });
	fs.copyFileSync(
		path.join(root, "apps/desktop/src-tauri/tauri.conf.json"),
		config,
	);
	fs.writeFileSync(
		path.join(bin, "plutil"),
		`#!${process.execPath}\nconst fs = require('fs'); console.log(JSON.parse(fs.readFileSync(process.argv.at(-1))).bundle.macOS.minimumSystemVersion);\n`,
	);
	fs.chmodSync(path.join(bin, "plutil"), 0o755);
	const desktopBuild = path.join(root, "scripts/desktop-build.sh");
	if (fs.existsSync(desktopBuild)) {
		fs.copyFileSync(desktopBuild, path.join(scripts, "desktop-build.sh"));
		fs.chmodSync(path.join(scripts, "desktop-build.sh"), 0o755);
	}
	const stable = path.join(repository, "dist/macos/Mote.app");
	const heicPackaging = path.join(repository, "packaging/heic");
	const licensePackaging = path.join(repository, "packaging/licenses");
	fs.mkdirSync(heicPackaging, { recursive: true });
	fs.mkdirSync(licensePackaging, { recursive: true });
	fs.copyFileSync(
		path.join(root, "THIRD_PARTY_NOTICES.md"),
		path.join(repository, "THIRD_PARTY_NOTICES.md"),
	);
	for (const name of ["LGPL-3.0-or-later.txt", "libheif.md", "libde265.md"])
		fs.copyFileSync(
			path.join(root, "packaging/licenses", name),
			path.join(licensePackaging, name),
		);
	for (const name of ["README.md", "decode-only.cmake"])
		fs.copyFileSync(
			path.join(root, "packaging/heic", name),
			path.join(heicPackaging, name),
		);
	fs.writeFileSync(
		path.join(heicPackaging, "build-unix.sh"),
		`#!/bin/sh
prefix="$PWD/build/heic-native/macos-universal"
mkdir -p "$prefix/lib/pkgconfig"
printf 'heif' > "$prefix/lib/libheif.dylib"
printf 'de265' > "$prefix/lib/libde265.dylib"
printf 'export MOTE_HEIC_PREFIX="%s"\\n' "$prefix"
printf 'built' >> "$PWD/native-build-log"
`,
	);
	fs.writeFileSync(
		path.join(heicPackaging, "verify-native-deps.sh"),
		`#!/bin/sh
printf '%s\\n' "$*" >> "$PWD/native-inspection-log"
case "$*" in *--app*)
  [ "${inspectionStatus}" = 0 ] || exit "${inspectionStatus}"
  app=$2
  case "$*" in
    *--no-heic*) [ ! -e "$app/Contents/Frameworks/libheif.dylib" ] && [ ! -e "$app/Contents/Frameworks/libde265.dylib" ] ;;
    *) [ -f "$app/Contents/Frameworks/libheif.dylib" ] && [ -f "$app/Contents/Frameworks/libde265.dylib" ] ;;
  esac ;;
esac
`,
	);
	for (const name of ["build-unix.sh", "verify-native-deps.sh"])
		fs.chmodSync(path.join(heicPackaging, name), 0o755);
	for (const name of ["install_name_tool", "codesign", "lipo"]) {
		fs.writeFileSync(
			path.join(bin, name),
			`#!/bin/sh\nprintf '%s\\n' "$*" >> "$PWD/${name}-log"\nexit ${name === "codesign" ? signStatus : 0}\n`,
		);
		fs.chmodSync(path.join(bin, name), 0o755);
	}
	fs.writeFileSync(path.join(bin, "otool"), "#!/bin/sh\nprintf ''\n");
	fs.chmodSync(path.join(bin, "otool"), 0o755);
	fs.mkdirSync(stable, { recursive: true });
	fs.writeFileSync(path.join(stable, "marker"), existingMarker);
	fs.writeFileSync(
		path.join(bin, "npm"),
		`#!/bin/sh
printf '%s' "$CARGO_TARGET_DIR" > "$TMPDIR/cargo-path"
printf '%s\n' "$@" > "$TMPDIR/npm-argv"
if [ "${buildStatus}" -ne 0 ]; then exit "${buildStatus}"; fi
case " $* " in
  *" --target universal-apple-darwin "*) bundle="$CARGO_TARGET_DIR/universal-apple-darwin/release/bundle/macos/Mote.app" ;;
  *) bundle="$CARGO_TARGET_DIR/release/bundle/macos/Mote.app" ;;
esac
mkdir -p "$bundle/Contents/MacOS"
printf 'plist' > "$bundle/Contents/Info.plist"
printf 'new' > "$bundle/marker"
printf '#!/bin/sh\n' > "$bundle/Contents/MacOS/photo-viewer-desktop"
chmod +x "$bundle/Contents/MacOS/photo-viewer-desktop"
	mkdir -p "$PWD/apps/interface/dist" "$PWD/apps/interface/node_modules/.vite" "$PWD/apps/interface/node_modules/.vite-temp"
printf 'generated' > "$PWD/apps/interface/dist/index.html"
printf 'generated' > "$PWD/apps/interface/node_modules/.vite/cache"
printf 'generated' > "$PWD/apps/interface/node_modules/.vite-temp/cache"
printf 'generated' > "$PWD/apps/interface/tsconfig.tsbuildinfo"
if [ "${invalid}" = duplicate ]; then
  cp "$bundle/Contents/MacOS/photo-viewer-desktop" "$bundle/Contents/MacOS/duplicate"
fi
if [ "${invalid}" = missing-plist ]; then
  rm "$bundle/Contents/Info.plist"
fi
`,
	);
	fs.chmodSync(path.join(bin, "npm"), 0o755);
	fs.writeFileSync(path.join(bin, "ditto"), '#!/bin/sh\ncp -R "$1" "$2"\n');
	fs.chmodSync(path.join(bin, "ditto"), 0o755);
	return repository;
}

function runDesktopBuild(repository, mode, args = [], environment = {}) {
	return spawnSync(
		path.join(repository, "scripts/desktop-build.sh"),
		[mode, ...args],
		{
			cwd: repository,
			env: {
				...process.env,
				TMPDIR: repository,
				PATH: `${path.join(repository, "bin")}:${process.env.PATH}`,
				...environment,
			},
			encoding: "utf8",
		},
	);
}

function desktopBuildArguments(repository) {
	return fs
		.readFileSync(path.join(repository, "npm-argv"), "utf8")
		.trimEnd()
		.split("\n");
}

function assertDesktopTempsRemoved(repository) {
	const cargoPath = path.join(repository, "cargo-path");
	if (fs.existsSync(cargoPath)) {
		assert.equal(fs.existsSync(fs.readFileSync(cargoPath, "utf8")), false);
	}
	const output = path.join(repository, "dist/macos");
	const entries = fs.existsSync(output) ? fs.readdirSync(output) : [];
	assert.deepEqual(
		entries.filter((entry) => entry.startsWith(".Mote.app.")),
		[],
	);
	assert.deepEqual(
		fs
			.readdirSync(repository)
			.filter((entry) => entry.startsWith("mote-build-desktop.")),
		[],
	);
	for (const relative of [
		"apps/interface/dist",
		"apps/interface/node_modules/.vite",
		"apps/interface/node_modules/.vite-temp",
		"apps/interface/tsconfig.tsbuildinfo",
	]) {
		assert.equal(
			fs.existsSync(path.join(repository, relative)),
			false,
			relative,
		);
	}
}

test("external native prefix is checked against the app deployment target before desktop compilation", () => {
	const repository = desktopFixture();
	try {
		const config = path.join(
			repository,
			"apps/desktop/src-tauri/tauri.conf.json",
		);
		const app = JSON.parse(fs.readFileSync(config, "utf8"));
		app.bundle.macOS.minimumSystemVersion = "12.3.4";
		fs.writeFileSync(config, JSON.stringify(app));
		const prefix = path.join(repository, "external prefix");
		fs.mkdirSync(prefix);
		const inspector = path.join(
			repository,
			"packaging/heic/verify-native-deps.sh",
		);
		fs.writeFileSync(
			inspector,
			`#!/bin/sh\nprintf '%s\\n' "$@" > "$PWD/external-inspection-argv"\nexit 54\n`,
		);
		const result = runDesktopBuild(repository, "universal", [], {
			MOTE_HEIC_PREFIX: prefix,
		});
		assert.equal(result.status, 54, result.stderr);
		assert.deepEqual(
			fs
				.readFileSync(path.join(repository, "external-inspection-argv"), "utf8")
				.trim()
				.split("\n"),
			[
				"--prefix",
				prefix,
				"--arch",
				"universal",
				"--deployment-target",
				"12.3.4",
			],
		);
		assert.equal(
			fs.existsSync(path.join(repository, "native-build-log")),
			false,
		);
		assert.equal(fs.existsSync(path.join(repository, "npm-argv")), false);
		assert.equal(
			fs.readFileSync(
				path.join(repository, "dist/macos/Mote.app/marker"),
				"utf8",
			),
			"accepted",
		);
		assertDesktopTempsRemoved(repository);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("external prefix with a newer macOS minimum fails real inspection before desktop compilation", () => {
	const repository = desktopFixture();
	try {
		const prefix = path.join(repository, "external prefix");
		fs.mkdirSync(path.join(prefix, "lib"), { recursive: true });
		for (const name of ["libheif", "libde265"])
			fs.writeFileSync(path.join(prefix, "lib", `${name}.dylib`), "shared");
		fs.copyFileSync(
			path.join(root, "packaging/heic/verify-native-deps.sh"),
			path.join(repository, "packaging/heic/verify-native-deps.sh"),
		);
		for (const [name, source] of [
			[
				"uname",
				'#!/bin/sh\ncase "$1" in -s) echo Darwin ;; -m) echo arm64 ;; esac\n',
			],
			["lipo", "#!/bin/sh\necho arm64\n"],
			[
				"otool",
				'#!/bin/sh\nprintf "Load command 0\\n      cmd LC_BUILD_VERSION\\n platform 1\\n    minos 26.0\\n"\n',
			],
		]) {
			fs.writeFileSync(path.join(repository, "bin", name), source);
			fs.chmodSync(path.join(repository, "bin", name), 0o755);
		}
		const result = runDesktopBuild(repository, "native", [], {
			MOTE_HEIC_PREFIX: prefix,
		});
		assert.equal(result.status, 1, result.stderr);
		assert.match(
			result.stderr,
			/minimum 26\.0 exceeds deployment target 11\.0/,
		);
		assert.equal(
			fs.existsSync(path.join(repository, "native-build-log")),
			false,
		);
		assert.equal(fs.existsSync(path.join(repository, "npm-argv")), false);
		assert.equal(
			fs.readFileSync(
				path.join(repository, "dist/macos/Mote.app/marker"),
				"utf8",
			),
			"accepted",
		);
		assertDesktopTempsRemoved(repository);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("failed desktop build preserves the stable app", () => {
	const repository = desktopFixture({ buildStatus: 17 });
	try {
		const result = runDesktopBuild(repository, "native");
		assert.equal(result.status, 17, result.stderr);
		assert.equal(
			fs.readFileSync(
				path.join(repository, "dist/macos/Mote.app/marker"),
				"utf8",
			),
			"accepted",
		);
		assertDesktopTempsRemoved(repository);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

for (const mode of ["native", "universal"]) {
	test(`successful ${mode} desktop build replaces the stable app`, () => {
		const repository = desktopFixture();
		try {
			const result = runDesktopBuild(repository, mode);
			assert.equal(result.status, 0, result.stderr);
			assert.equal(
				fs.readFileSync(
					path.join(repository, "dist/macos/Mote.app/marker"),
					"utf8",
				),
				"new",
			);
			assertDesktopTempsRemoved(repository);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

for (const mode of ["native", "universal"]) {
	test(`HEIC ${mode} desktop mode translates the Mote flag and keeps Tauri arguments`, () => {
		const repository = desktopFixture();
		try {
			const result = runDesktopBuild(repository, mode, [
				"--no-heic",
				"--config=libheif.toml",
				"-v",
				"--ci",
			]);
			assert.equal(result.status, 0, result.stderr);
			const arguments_ = desktopBuildArguments(repository);
			assert.equal(arguments_.includes("--no-heic"), false);
			assert.deepEqual(
				arguments_.filter((argument) =>
					["--no-default-features", "--features", "mote-defaults"].includes(
						argument,
					),
				),
				["--no-default-features", "--features", "mote-defaults"],
			);
			assert.equal(arguments_.includes("--config=libheif.toml"), true);
			assert.equal(arguments_.includes("-v"), true);
			assert.equal(arguments_.at(-1), "--ci");
			assertDesktopTempsRemoved(repository);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

test("HEIC default desktop mode adds no Cargo feature arguments", () => {
	const repository = desktopFixture();
	try {
		const result = runDesktopBuild(repository, "native", ["--ci"]);
		assert.equal(result.status, 0, result.stderr);
		const arguments_ = desktopBuildArguments(repository);
		assert.deepEqual(
			arguments_.filter((argument) =>
				["--no-default-features", "--features", "mote-defaults"].includes(
					argument,
				),
			),
			[],
		);
		assert.equal(arguments_.at(-1), "--ci");
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("enabled desktop bundle stages and signs decoder dylibs before publication", () => {
	const repository = desktopFixture();
	try {
		const result = runDesktopBuild(repository, "universal");
		assert.equal(result.status, 0, result.stderr);
		for (const name of ["libheif", "libde265"]) {
			assert.ok(
				fs.existsSync(
					path.join(
						repository,
						`dist/macos/Mote.app/Contents/Frameworks/${name}.dylib`,
					),
				),
			);
		}
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
						"dist/macos/Mote.app/Contents/Resources/licenses",
						name,
					),
				),
				name,
			);
		assert.equal(
			fs.readFileSync(
				path.join(
					repository,
					"dist/macos/Mote.app/Contents/Resources/licenses/decode-only.cmake",
				),
				"utf8",
			),
			fs.readFileSync(
				path.join(repository, "packaging/heic/decode-only.cmake"),
				"utf8",
			),
		);
		assert.match(
			fs.readFileSync(path.join(repository, "native-inspection-log"), "utf8"),
			/--app .*\.Mote\.app\.next.*--arch universal/,
		);
		assert.match(
			fs.readFileSync(path.join(repository, "install_name_tool-log"), "utf8"),
			/-add_rpath @executable_path\/\.\.\/Frameworks/,
		);
		assert.match(
			fs.readFileSync(path.join(repository, "codesign-log"), "utf8"),
			/--verify/,
		);
		assertDesktopTempsRemoved(repository);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("disabled desktop builds skip native compilation and inspect the staged app", () => {
	const repository = desktopFixture();
	try {
		const result = runDesktopBuild(repository, "universal", ["--no-heic"]);
		assert.equal(result.status, 0, result.stderr);
		assert.equal(
			fs.existsSync(path.join(repository, "native-build-log")),
			false,
		);
		assert.match(
			fs.readFileSync(path.join(repository, "native-inspection-log"), "utf8"),
			/--app .*--no-heic/,
		);
		assert.equal(
			fs.existsSync(
				path.join(
					repository,
					"dist/macos/Mote.app/Contents/Resources/licenses",
				),
			),
			false,
		);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

for (const [name, options] of [
	["native inspection", { inspectionStatus: 53 }],
	["native signing", { signStatus: 53 }],
]) {
	test(`failed ${name} keeps the accepted app and removes desktop staging`, () => {
		const repository = desktopFixture(options);
		try {
			const result = runDesktopBuild(repository, "universal");
			assert.equal(result.status, 53, result.stderr);
			assert.equal(
				fs.readFileSync(
					path.join(repository, "dist/macos/Mote.app/marker"),
					"utf8",
				),
				"accepted",
			);
			assertDesktopTempsRemoved(repository);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

for (const args of [
	["--no-heic", "--no-heic"],
	["--no-heicc"],
	["--no-hiec"],
	["--no-HEIC"],
	["--no-heif"],
]) {
	test(`HEIC desktop mode rejects invalid Mote arguments: ${args.join(" ")}`, () => {
		const repository = desktopFixture();
		try {
			const result = runDesktopBuild(repository, "native", args);
			assert.equal(result.status, 2);
			assert.match(result.stderr, /usage:/);
			assert.equal(fs.existsSync(path.join(repository, "npm-argv")), false);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

for (const args of [
	["--no-heic", "--all-features"],
	["--no-heic", "--features", "heic"],
	["--no-heic", "--features=heic"],
	["--no-default-features"],
	["--no-heic", "-f", "heic"],
	["--no-heic", "-fheic"],
	["--no-heic", "-f=heic"],
]) {
	test(`desktop Mote mode rejects Cargo feature overrides: ${args.join(" ")}`, () => {
		const repository = desktopFixture();
		try {
			const result = runDesktopBuild(repository, "native", args);
			assert.equal(result.status, 2);
			assert.match(result.stderr, /usage:/);
			assert.equal(fs.existsSync(path.join(repository, "npm-argv")), false);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

for (const invalid of ["duplicate", "missing-plist"]) {
	test(`invalid ${invalid} desktop build preserves the stable app`, () => {
		const repository = desktopFixture({ invalid });
		try {
			const result = runDesktopBuild(repository, "native");
			assert.notEqual(result.status, 0);
			assert.equal(
				fs.readFileSync(
					path.join(repository, "dist/macos/Mote.app/marker"),
					"utf8",
				),
				"accepted",
			);
			assertDesktopTempsRemoved(repository);
		} finally {
			fs.rmSync(repository, { recursive: true, force: true });
		}
	});
}

test("legacy build assets cleanup is strictly repository-scoped", () => {
	const repository = fixture();
	const scripts = path.join(repository, "scripts");
	const tmpdir = path.join(repository, "temporary");
	const outside = fs.mkdtempSync(
		path.join(os.tmpdir(), "mote-data-preserved-"),
	);
	fs.mkdirSync(scripts);
	fs.mkdirSync(tmpdir);
	for (const name of ["build-lifecycle.sh", "hosted-smoke-assets.sh"]) {
		fs.copyFileSync(path.join(root, "scripts", name), path.join(scripts, name));
	}
	const cleanup = path.join(root, "scripts/clean-build-assets.sh");
	if (fs.existsSync(cleanup)) {
		fs.copyFileSync(cleanup, path.join(scripts, "clean-build-assets.sh"));
		fs.chmodSync(path.join(scripts, "clean-build-assets.sh"), 0o755);
	}
	const removable = [
		"target",
		"apps/desktop/src-tauri/target",
		"apps/interface/dist",
		"apps/interface/.vite",
		"node_modules/.vite",
		"node_modules/.vite-temp",
		"apps/interface/node_modules/.vite",
		"apps/interface/node_modules/.vite-temp",
		"build/flatpak",
		"build/heic-native",
		".flatpak-builder",
		"dist/flatpak/repo",
		".worktrees/example/target",
		".worktrees/example/apps/desktop/src-tauri/target",
		".worktrees/example/node_modules/.vite",
		".worktrees/example/node_modules/.vite-temp",
		".worktrees/example/apps/interface/node_modules/.vite",
		".worktrees/example/apps/interface/node_modules/.vite-temp",
		".worktrees/example/build/heic-native",
		".worktrees/example/build/flatpak",
		".worktrees/example/.flatpak-builder",
		"packaging/flatpak/.generated.next.999999999",
		"packaging/flatpak/.generated.previous.999999999",
		"runtime/photo-viewer-smoke-100-999999999",
	];
	for (const relative of removable) {
		fs.mkdirSync(path.join(repository, relative), { recursive: true });
		fs.writeFileSync(path.join(repository, relative, "generated"), "build");
	}
	const generatedFiles = [
		"apps/interface/tsconfig.tsbuildinfo",
		".worktrees/example/apps/interface/tsconfig.tsbuildinfo",
	];
	for (const relative of generatedFiles) {
		fs.mkdirSync(path.dirname(path.join(repository, relative)), {
			recursive: true,
		});
		fs.writeFileSync(path.join(repository, relative), "build");
	}
	for (const relative of [
		"build/heic-native/cache/download.partial",
		"build/heic-native/extracted/source.c",
		"build/heic-native/.macos-arm64.stage/library",
		"build/heic-native/.macos-arm64.backup/library",
		"build/heic-native/macos-arm64/lib/libheif.dylib",
		".worktrees/example/build/heic-native/cache/download.partial",
		".worktrees/example/build/heic-native/linux-x86_64/lib/libheif.so",
	]) {
		fs.mkdirSync(path.dirname(path.join(repository, relative)), {
			recursive: true,
		});
		fs.writeFileSync(path.join(repository, relative), "generated");
	}
	const stableApp = path.join(repository, "dist/macos/Mote.app");
	fs.mkdirSync(path.join(repository, "packaging/flatpak/generated"), {
		recursive: true,
	});
	fs.writeFileSync(
		path.join(repository, "packaging/flatpak/generated/source-lock.json"),
		"keep source lock",
	);
	const stableFlatpak = path.join(
		repository,
		"dist/flatpak/Mote-0.1.0-x86_64.flatpak",
	);
	fs.mkdirSync(stableApp, { recursive: true });
	fs.writeFileSync(path.join(stableApp, "installed-copy"), "keep");
	fs.writeFileSync(stableFlatpak, "keep");
	const sourcePhoto = path.join(repository, "source-media/iphone.heic");
	const untrackedDistFile = path.join(repository, "dist/user-owned-photo.heic");
	fs.mkdirSync(path.dirname(sourcePhoto), { recursive: true });
	fs.writeFileSync(sourcePhoto, "source bytes");
	fs.writeFileSync(untrackedDistFile, "untracked dist bytes");
	fs.writeFileSync(path.join(outside, "application-data"), "keep");
	const deadBuild = path.join(tmpdir, "mote-build-unit.dead");
	const liveBuild = path.join(tmpdir, "mote-build-unit.live");
	fs.mkdirSync(deadBuild);
	fs.mkdirSync(liveBuild);
	fs.writeFileSync(path.join(deadBuild, ".mote-owner-pid"), "999999999\n");
	fs.writeFileSync(path.join(liveBuild, ".mote-owner-pid"), `${process.pid}\n`);
	const fakeEngine = path.join(repository, "fake-podman");
	const engineLog = path.join(repository, "engine.log");
	fs.writeFileSync(
		fakeEngine,
		`#!/bin/sh\nprintf '%s\\n' "$*" >> "${engineLog}"\nexit 0\n`,
	);
	fs.chmodSync(fakeEngine, 0o755);
	try {
		const result = spawnSync(path.join(scripts, "clean-build-assets.sh"), [], {
			cwd: repository,
			env: {
				...process.env,
				TMPDIR: tmpdir,
				CONTAINER_ENGINE: fakeEngine,
			},
			encoding: "utf8",
		});
		assert.equal(result.status, 0, result.stderr);
		for (const relative of removable) {
			assert.equal(
				fs.existsSync(path.join(repository, relative)),
				false,
				relative,
			);
		}
		for (const relative of generatedFiles) {
			assert.equal(
				fs.existsSync(path.join(repository, relative)),
				false,
				relative,
			);
		}
		assert.equal(fs.existsSync(stableApp), true);
		assert.equal(
			fs.readFileSync(
				path.join(repository, "packaging/flatpak/generated/source-lock.json"),
				"utf8",
			),
			"keep source lock",
		);
		assert.equal(fs.readFileSync(stableFlatpak, "utf8"), "keep");
		assert.equal(fs.readFileSync(sourcePhoto, "utf8"), "source bytes");
		assert.equal(
			fs.readFileSync(untrackedDistFile, "utf8"),
			"untracked dist bytes",
		);
		assert.equal(
			fs.readFileSync(path.join(outside, "application-data"), "utf8"),
			"keep",
		);
		assert.equal(fs.existsSync(deadBuild), false);
		assert.equal(fs.existsSync(liveBuild), true);
		const calls = fs.readFileSync(engineLog, "utf8");
		assert.match(calls, /info/);
		assert.match(calls, /ps -a/);
		assert.doesNotMatch(calls, /system prune|volume prune|image prune/);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
		fs.rmSync(outside, { recursive: true, force: true });
	}
});

test("package commands route local builds through managed asset lifecycles", () => {
	const packageJson = JSON.parse(
		fs.readFileSync(path.join(root, "package.json"), "utf8"),
	);
	assert.equal(packageJson.scripts["rust:verify"], "sh scripts/rust-verify.sh");
	assert.equal(
		packageJson.scripts["desktop:dev"],
		"sh scripts/with-ephemeral-cargo-target.sh npm exec --workspace @photo-viewer/desktop -- tauri dev",
	);
	assert.equal(
		packageJson.scripts["desktop:build"],
		"sh scripts/desktop-build.sh native",
	);
	assert.equal(
		packageJson.scripts["desktop:build:universal"],
		"sh scripts/desktop-build.sh universal",
	);
	assert.equal(
		packageJson.scripts["clean:build-assets"],
		"sh scripts/clean-build-assets.sh",
	);
	assert.equal(
		packageJson.scripts["clean:build"],
		"sh scripts/clean-build-assets.sh",
	);
	for (const [command, child] of [
		["typecheck", "npm run typecheck --workspace @photo-viewer/interface"],
		["test", "npm run test --workspace @photo-viewer/interface"],
		[
			"test:browser",
			"npm run test:browser --workspace @photo-viewer/interface",
		],
	]) {
		assert.equal(
			packageJson.scripts[command],
			`sh scripts/with-ephemeral-interface-assets.sh ${child}`,
		);
	}
});

test("macOS workflow packages the stable repository app as a short-lived manual DMG", () => {
	const workflow = fs.readFileSync(
		path.join(root, ".github/workflows/build-macos.yml"),
		"utf8",
	);
	assert.match(workflow, /app="dist\/macos\/Mote\.app"/);
	assert.doesNotMatch(workflow, /target\/.*Mote\.app/);
	assert.match(workflow, /lipo .* -verify_arch arm64 x86_64/);
	assert.match(workflow, /hdiutil create/);
	assert.match(workflow, /retention-days: 1/);
	assert.doesNotMatch(workflow, /\.zip/);
});

test("interface command wrapper removes generated caches and preserves status", () => {
	const repository = fixture();
	const scripts = path.join(repository, "scripts");
	const bin = path.join(repository, "bin");
	fs.mkdirSync(scripts);
	fs.mkdirSync(bin);
	fs.copyFileSync(lifecycle, path.join(scripts, "build-lifecycle.sh"));
	const wrapper = path.join(root, "scripts/with-ephemeral-interface-assets.sh");
	if (fs.existsSync(wrapper)) {
		fs.copyFileSync(
			wrapper,
			path.join(scripts, "with-ephemeral-interface-assets.sh"),
		);
		fs.chmodSync(
			path.join(scripts, "with-ephemeral-interface-assets.sh"),
			0o755,
		);
	}
	const command = path.join(bin, "fake-interface-command");
	fs.writeFileSync(
		command,
		'#!/bin/sh\nmkdir -p "$PWD/apps/interface/dist" "$PWD/apps/interface/node_modules/.vite" "$PWD/apps/interface/node_modules/.vite-temp" "$PWD/node_modules/.vite" "$PWD/node_modules/.vite-temp"\ntouch "$PWD/apps/interface/dist/index.html" "$PWD/apps/interface/node_modules/.vite/cache" "$PWD/apps/interface/node_modules/.vite-temp/cache" "$PWD/node_modules/.vite/cache" "$PWD/node_modules/.vite-temp/cache" "$PWD/apps/interface/tsconfig.tsbuildinfo"\nexit 21\n',
	);
	fs.chmodSync(command, 0o755);
	const preserved = path.join(repository, "application-data");
	fs.writeFileSync(preserved, "keep");
	try {
		const result = spawnSync(
			path.join(scripts, "with-ephemeral-interface-assets.sh"),
			[command],
			{ cwd: repository, encoding: "utf8" },
		);
		assert.equal(result.status, 21, result.stderr);
		for (const relative of [
			"apps/interface/dist",
			"apps/interface/node_modules/.vite",
			"apps/interface/node_modules/.vite-temp",
			"node_modules/.vite",
			"node_modules/.vite-temp",
			"apps/interface/tsconfig.tsbuildinfo",
		]) {
			assert.equal(
				fs.existsSync(path.join(repository, relative)),
				false,
				relative,
			);
		}
		assert.equal(fs.readFileSync(preserved, "utf8"), "keep");
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("interface cleanup waits for another live wrapped command", async () => {
	const repository = fixture();
	const scripts = path.join(repository, "scripts");
	const bin = path.join(repository, "bin");
	const tmpdir = path.join(repository, "temporary");
	fs.mkdirSync(scripts);
	fs.mkdirSync(bin);
	fs.mkdirSync(tmpdir);
	for (const name of [
		"build-lifecycle.sh",
		"with-ephemeral-interface-assets.sh",
	]) {
		fs.copyFileSync(path.join(root, "scripts", name), path.join(scripts, name));
		fs.chmodSync(path.join(scripts, name), 0o755);
	}
	const leaderCommand = path.join(bin, "leader");
	const followerCommand = path.join(bin, "follower");
	const cache = path.join(
		repository,
		"apps/interface/node_modules/.vite/cache",
	);
	fs.writeFileSync(
		leaderCommand,
		'#!/bin/sh\nmkdir -p "$PWD/apps/interface/node_modules/.vite"\ntouch "$PWD/apps/interface/node_modules/.vite/cache" "$TMPDIR/leader-ready"\nwhile [ ! -f "$TMPDIR/release" ]; do sleep 0.05; done\n',
	);
	fs.writeFileSync(
		followerCommand,
		'#!/bin/sh\nmkdir -p "$PWD/apps/interface/node_modules/.vite"\ntouch "$PWD/apps/interface/node_modules/.vite/cache"\n',
	);
	fs.chmodSync(leaderCommand, 0o755);
	fs.chmodSync(followerCommand, 0o755);
	const wrapper = path.join(scripts, "with-ephemeral-interface-assets.sh");
	const environment = { ...process.env, TMPDIR: tmpdir };
	const leader = spawn(wrapper, [leaderCommand], {
		cwd: repository,
		env: environment,
		stdio: "ignore",
	});
	try {
		await waitForFile(path.join(tmpdir, "leader-ready"));
		const follower = spawnSync(wrapper, [followerCommand], {
			cwd: repository,
			env: environment,
			encoding: "utf8",
		});
		assert.equal(follower.status, 0, follower.stderr);
		assert.equal(fs.existsSync(cache), true);
		fs.writeFileSync(path.join(tmpdir, "release"), "go");
		const status = await new Promise((resolve) => leader.once("exit", resolve));
		assert.equal(status, 0);
		assert.equal(fs.existsSync(cache), false);
	} finally {
		if (leader.exitCode === null) leader.kill("SIGTERM");
		fs.rmSync(repository, { recursive: true, force: true });
	}
});
