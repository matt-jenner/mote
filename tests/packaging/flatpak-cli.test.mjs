import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const script = path.join(root, "scripts/flatpak.sh");

test("Flatpak helper exposes the complete local packaging interface", () => {
	const help = execFileSync("bash", [script, "help"], {
		cwd: root,
		encoding: "utf8",
	});
	for (const command of ["check", "package", "install", "run", "inspect"]) {
		assert.match(help, new RegExp(`^  ${command}(?: |$)`, "m"));
	}
	assert.doesNotMatch(help, /^ {2}(?:build|bundle)(?: |$)/m);
});

test("Flatpak helper is valid Bash and never escalates privileges", () => {
	execFileSync("bash", ["-n", script], { cwd: root });
	const source = fs.readFileSync(script, "utf8");
	assert.doesNotMatch(source, /\bsudo\b/);
	assert.match(source, /--noninteractive/);
	assert.match(
		source,
		/--runtime-repo=https:\/\/flathub\.org\/repo\/flathub\.flatpakrepo/,
	);
	assert.match(source, /--show-permissions/);
	assert.match(source, /--state-dir="\$state_dir"/);
	assert.match(source, /--delete-build-dirs/);
});

test("Flatpak helper preserves the application and bundle naming contract", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /app_id="io\.github\.matt_jenner\.mote"/);
	assert.match(source, /Mote-%s-%s\.flatpak/);
	assert.match(source, /MOTE_FLATPAK_ARCH/);
	assert.match(source, /require\(process\.argv\[1\]\)\.version/);
});

test("Flatpak helper updates the current user's installed bundle", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /flatpak install --user --noninteractive --or-update/);
});

test("Flatpak validation runs from the repository root", () => {
	const source = fs.readFileSync(script, "utf8");
	assert.match(source, /cd -- "\$repository_root"/);
	assert.match(source, /npm run test:flatpak/);
});

function flatpakFixture(bundleStatus = 0) {
	const directory = fs.mkdtempSync(
		path.join(os.tmpdir(), "mote-flatpak-test-"),
	);
	const bin = path.join(directory, "bin");
	const tmpdir = path.join(directory, "tmp");
	const bundles = path.join(directory, "bundles");
	const log = path.join(directory, "calls.log");
	fs.mkdirSync(bin);
	fs.mkdirSync(tmpdir);
	fs.mkdirSync(bundles);
	const executable = (name, source) => {
		const target = path.join(bin, name);
		fs.writeFileSync(target, source);
		fs.chmodSync(target, 0o755);
	};
	executable(
		"flatpak-builder",
		`#!/bin/sh
printf '%s|%s\n' "\${MOTE_HEIC:-}" "$*" >> "$MOTE_FLATPAK_TEST_LOG"
for argument in "$@"; do
  case "$argument" in
    --repo=*) repo=$(printf '%s' "$argument" | cut -c 8-); mkdir -p "$repo" ;;
    */build) mkdir -p "$argument" ;;
  esac
done
exit 0
`,
	);
	executable(
		"flatpak",
		'#!/bin/sh\nif [ "$1" = --default-arch ]; then printf \'%s\\n\' x86_64; exit 0; fi\nif [ "$1" = build-bundle ]; then\n  if [ ' +
			bundleStatus +
			" -ne 0 ]; then exit " +
			bundleStatus +
			'; fi\n  for argument in "$@"; do\n    case "$argument" in *.flatpak) printf \'bundle\' > "$argument" ;; esac\n  done\nfi\nexit 0\n',
	);
	for (const tool of ["desktop-file-validate", "appstreamcli", "npm"]) {
		executable(tool, "#!/bin/sh\nexit 0\n");
	}
	return { directory, bin, tmpdir, bundles, log };
}

function runFlatpakFixture(fixtureDirectory, args = []) {
	return spawnSync("bash", [script, "package", ...args], {
		cwd: root,
		env: {
			...process.env,
			PATH: `${fixtureDirectory.bin}:${process.env.PATH}`,
			TMPDIR: fixtureDirectory.tmpdir,
			MOTE_FLATPAK_BUNDLE_DIR: fixtureDirectory.bundles,
			MOTE_FLATPAK_ARCH: "x86_64",
			MOTE_FLATPAK_TEST_LOG: fixtureDirectory.log,
		},
		encoding: "utf8",
	});
}

function flatpakBuildCall(fixtureDirectory) {
	return fs
		.readFileSync(fixtureDirectory.log, "utf8")
		.trimEnd()
		.split("\n")
		.find((line) => line.includes("--force-clean"));
}

test("Flatpak package retains one bundle and removes intermediates", () => {
	const fixtureDirectory = flatpakFixture();
	const oldBundle = path.join(
		fixtureDirectory.bundles,
		"Mote-0.0.9-x86_64.flatpak",
	);
	fs.writeFileSync(oldBundle, "old");
	try {
		const result = runFlatpakFixture(fixtureDirectory);
		assert.equal(result.status, 0, result.stderr);
		const bundles = fs
			.readdirSync(fixtureDirectory.bundles)
			.filter((entry) => entry.endsWith(".flatpak"));
		assert.equal(bundles.length, 1);
		assert.notEqual(bundles[0], path.basename(oldBundle));
		assert.ok(
			fs.statSync(path.join(fixtureDirectory.bundles, bundles[0])).size > 0,
		);
		assert.deepEqual(
			fs
				.readdirSync(fixtureDirectory.tmpdir)
				.filter((entry) => entry.startsWith("mote-build-flatpak.")),
			[],
		);
	} finally {
		fs.rmSync(fixtureDirectory.directory, { recursive: true, force: true });
	}
});

test("failed Flatpak package preserves the previous bundle", () => {
	const fixtureDirectory = flatpakFixture(29);
	const oldBundle = path.join(
		fixtureDirectory.bundles,
		"Mote-0.0.9-x86_64.flatpak",
	);
	fs.writeFileSync(oldBundle, "old");
	try {
		const result = runFlatpakFixture(fixtureDirectory);
		assert.equal(result.status, 29, result.stderr);
		assert.equal(fs.readFileSync(oldBundle, "utf8"), "old");
		assert.deepEqual(
			fs
				.readdirSync(fixtureDirectory.tmpdir)
				.filter((entry) => entry.startsWith("mote-build-flatpak.")),
			[],
		);
	} finally {
		fs.rmSync(fixtureDirectory.directory, { recursive: true, force: true });
	}
});

test("HEIC Flatpak package passes the default mode explicitly", () => {
	const fixtureDirectory = flatpakFixture();
	try {
		const result = runFlatpakFixture(fixtureDirectory);
		assert.equal(result.status, 0, result.stderr);
		assert.match(flatpakBuildCall(fixtureDirectory), /^enabled\|/);
	} finally {
		fs.rmSync(fixtureDirectory.directory, { recursive: true, force: true });
	}
});

test("HEIC Flatpak package consumes the Mote flag and preserves builder arguments", () => {
	const fixtureDirectory = flatpakFixture();
	try {
		const result = runFlatpakFixture(fixtureDirectory, [
			"--no-heic",
			"--disable-rofiles-fuse",
		]);
		assert.equal(result.status, 0, result.stderr);
		const buildCall = flatpakBuildCall(fixtureDirectory);
		assert.match(buildCall, /^disabled\|/);
		assert.doesNotMatch(buildCall, /--no-heic/);
		assert.match(buildCall, /--disable-rofiles-fuse/);
	} finally {
		fs.rmSync(fixtureDirectory.directory, { recursive: true, force: true });
	}
});

for (const args of [["--no-heic", "--no-heic"], ["--no-heicc"]]) {
	test(`HEIC Flatpak package rejects invalid Mote arguments: ${args.join(" ")}`, () => {
		const fixtureDirectory = flatpakFixture();
		try {
			const result = runFlatpakFixture(fixtureDirectory, args);
			assert.equal(result.status, 2);
			assert.match(result.stderr, /usage:/);
			assert.equal(fs.existsSync(fixtureDirectory.log), false);
		} finally {
			fs.rmSync(fixtureDirectory.directory, {
				recursive: true,
				force: true,
			});
		}
	});
}

test("Flatpak documentation exposes only the managed package command", () => {
	const documentation = fs.readFileSync(
		path.join(root, "packaging/flatpak/README.md"),
		"utf8",
	);
	assert.match(documentation, /npm run flatpak -- package/);
	assert.doesNotMatch(documentation, /npm run flatpak -- (?:build|bundle)/);
});
