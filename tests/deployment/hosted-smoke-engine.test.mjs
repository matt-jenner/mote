import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	chmodSync,
	existsSync,
	mkdtempSync,
	readdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const repositoryRoot = path.resolve(import.meta.dirname, "../..");
const smokeScript = path.join(repositoryRoot, "scripts/hosted-smoke.sh");
const smokeAssets = path.join(repositoryRoot, "scripts/hosted-smoke-assets.sh");

test("HEIC container separates native build and runtime copies and supports disabled builds", () => {
	const source = readFileSync(
		path.join(repositoryRoot, "Containerfile"),
		"utf8",
	);
	assert.match(source, /AS native-build/);
	assert.match(source, /AS runtime-copy/);
	assert.match(source, /ARG MOTE_HEIC=enabled/);
	assert.match(source, /build-unix.sh --platform linux/);
	assert.match(source, /--no-default-features --features mote-defaults/);
	assert.match(source, /COPY --from=runtime-copy \/runtime\//);
	assert.doesNotMatch(source, /apt-get install[^\n]*(?:x265|heif-enc|libheif)/);
});

test("HEIC hosted smoke uses cleared source media and inspects both package modes", () => {
	const source = readFileSync(smokeScript, "utf8");
	assert.match(source, /fixtures\/heif\/iphone-8bit.heic/);
	assert.match(source, /PHOTO_VIEWER_HEIC_MODE="\$MOTE_HEIC_MODE"/);
	assert.match(source, /verify-linux-runtime.sh/);
	assert.match(source, /run --rm --interactive --name "\$container_name"/);
	const browser = readFileSync(
		path.join(repositoryRoot, "tests/hosted/heic.spec.ts"),
		"utf8",
	);
	for (const contract of [
		/wallThumbnail/,
		/screenPreview/,
		/afterRestart/,
		/image\/jpeg/,
		/totalAssets/,
		/totalPhotos/,
	])
		assert.match(browser, contract);
});

test("HEIC container Cargo command builds defaults or only mote-defaults", () => {
	const source = readFileSync(
		path.join(repositoryRoot, "Containerfile"),
		"utf8",
	).replaceAll(/\\\n/g, "");
	const command = source
		.split("\n")
		.find((line) => line.startsWith("RUN if") && line.includes("cargo build"))
		?.slice(4);
	assert.ok(command);
	const directory = mkdtempSync(
		path.join(os.tmpdir(), "mote-container-command-"),
	);
	try {
		writeFileSync(
			path.join(directory, "cargo"),
			"#!/bin/sh\nprintf '%s\\n' \"$@\"\n",
			{ mode: 0o755 },
		);
		for (const [mode, expected] of [
			["enabled", []],
			["disabled", ["--no-default-features", "--features", "mote-defaults"]],
		]) {
			const result = spawnSync("sh", ["-c", command], {
				encoding: "utf8",
				env: {
					...process.env,
					PATH: `${directory}:${process.env.PATH}`,
					MOTE_HEIC: mode,
				},
			});
			assert.equal(result.status, 0, result.stderr);
			assert.deepEqual(result.stdout.trim().split("\n"), [
				"build",
				"--locked",
				"--release",
				"--jobs",
				"2",
				"-p",
				"photo-server",
				...expected,
			]);
		}
	} finally {
		rmSync(directory, { recursive: true, force: true });
	}
});

test("hosted smoke gives standalone browser fixtures an isolated web root", () => {
	const script = readFileSync(smokeScript, "utf8");
	assert.match(
		script,
		/with-ephemeral-interface-assets\.sh["']? \\\n\s+npm run web:build -- --outDir/,
	);
	assert.match(script, /PHOTO_VIEWER_WEB_ROOT="\$browser_web_root"/);
	assert.match(script, /CARGO_TARGET_DIR="\$temporary_root\/cargo-target"/);
	assert.match(script, /export CARGO_TARGET_DIR/);
});

test("hosted smoke validates the selected container engine", () => {
	const missingEngine = "photo-viewer-missing-container-engine";
	const result = spawnSync(smokeScript, [], {
		cwd: repositoryRoot,
		env: {
			...process.env,
			CONTAINER_ENGINE: missingEngine,
		},
		encoding: "utf8",
	});

	assert.notEqual(result.status, 0);
	assert.match(result.stderr, new RegExp(`${missingEngine} is required`));
});

test("hosted smoke rejects Docker daemon user-namespace remapping", () => {
	const fixtureDirectory = mkdtempSync(
		path.join(os.tmpdir(), "photo-viewer-docker-fixture-"),
	);
	const fakeDocker = path.join(fixtureDirectory, "docker");
	writeFileSync(
		fakeDocker,
		"#!/bin/sh\nif [ \"$1\" = info ]; then\n\tprintf '%s\\n' '[\"name=userns\"]'\n\texit 0\nfi\nexit 99\n",
	);
	chmodSync(fakeDocker, 0o755);

	try {
		const result = spawnSync(smokeScript, [], {
			cwd: repositoryRoot,
			env: {
				...process.env,
				CONTAINER_ENGINE: fakeDocker,
			},
			encoding: "utf8",
		});

		assert.notEqual(result.status, 0);
		assert.match(result.stderr, /user-namespace remapping is not supported/);
	} finally {
		rmSync(fixtureDirectory, { recursive: true, force: true });
	}
});

function failingBuildEngineFixture() {
	const directory = mkdtempSync(path.join(os.tmpdir(), "mote-engine-"));
	const engine = path.join(directory, "podman");
	const log = path.join(directory, "calls.log");
	writeFileSync(
		engine,
		`#!/bin/sh
printf '%s\n' "$*" >>"${log}"
if [ "$1" = info ]; then printf '%s\n' '[]'; exit 0; fi
if [ "$1" = build ]; then exit 31; fi
exit 0
`,
	);
	chmodSync(engine, 0o755);
	return { directory, engine, log };
}

function runFailingSmokeBuild(fixture, args = []) {
	return spawnSync(smokeScript, args, {
		cwd: repositoryRoot,
		env: {
			...process.env,
			CONTAINER_ENGINE: fixture.engine,
			TMPDIR: fixture.directory,
		},
		encoding: "utf8",
	});
}

test("hosted smoke removes its unique image after a failed build", () => {
	const fixture = failingBuildEngineFixture();
	try {
		const result = runFailingSmokeBuild(fixture);
		assert.equal(result.status, 31, result.stderr);
		const calls = readFileSync(fixture.log, "utf8");
		const tag = calls.match(/--tag (localhost\/mote-smoke:[^ ]+)/)?.[1];
		assert.ok(tag);
		assert.match(calls, /build .*--layers=false/);
		assert.match(calls, /build .*--build-arg MOTE_HEIC=enabled/);
		assert.match(calls, new RegExp(`image rm ${tag.replaceAll("-", "\\-")}`));
		assert.doesNotMatch(calls, /system prune|volume prune|image prune/);
		assert.deepEqual(
			readdirSync(fixture.directory).filter((entry) =>
				entry.startsWith("photo-viewer-smoke-"),
			),
			[],
		);
	} finally {
		rmSync(fixture.directory, { recursive: true, force: true });
	}
});

test("HEIC hosted smoke consumes the Mote flag and preserves build arguments", () => {
	const fixture = failingBuildEngineFixture();
	try {
		const result = runFailingSmokeBuild(fixture, [
			"--no-heic",
			"--pull=never",
			"--build-arg=HEIC_CACHE=/cache/heic",
			"--label=decoder=libheif",
		]);
		assert.equal(result.status, 31, result.stderr);
		const buildCall = readFileSync(fixture.log, "utf8")
			.split("\n")
			.find((line) => line.startsWith("build "));
		assert.ok(buildCall);
		assert.match(buildCall, /--build-arg MOTE_HEIC=disabled/);
		assert.match(buildCall, /--pull=never/);
		assert.match(buildCall, /--build-arg=HEIC_CACHE=\/cache\/heic/);
		assert.match(buildCall, /--label=decoder=libheif/);
		assert.doesNotMatch(buildCall, /--no-heic/);
	} finally {
		rmSync(fixture.directory, { recursive: true, force: true });
	}
});

for (const args of [
	["--no-heic", "--no-heic"],
	["--no-heicc"],
	["--no-hiec"],
	["--no-HEIC"],
	["--no-heif"],
]) {
	test(`HEIC hosted smoke rejects invalid Mote arguments: ${args.join(" ")}`, () => {
		const fixture = failingBuildEngineFixture();
		try {
			const result = runFailingSmokeBuild(fixture, args);
			assert.equal(result.status, 2);
			assert.match(result.stderr, /usage:/);
			assert.equal(existsSync(fixture.log), false);
		} finally {
			rmSync(fixture.directory, { recursive: true, force: true });
		}
	});
}

test("stale smoke cleanup removes only dead-owner assets", () => {
	const fixtureDirectory = mkdtempSync(path.join(os.tmpdir(), "mote-engine-"));
	const fakeEngine = path.join(fixtureDirectory, "podman");
	const log = path.join(fixtureDirectory, "calls.log");
	const liveRun = `photo-viewer-smoke-100-${process.pid}`;
	const stoppedLiveRun = `photo-viewer-smoke-101-${process.pid}`;
	const deadRun = "photo-viewer-smoke-102-999999999";
	writeFileSync(
		fakeEngine,
		`#!/bin/sh
printf '%s\n' "$*" >>"${log}"
if [ "$1" = ps ]; then
  printf '%s\n' '${liveRun}-app' '${stoppedLiveRun}-app' '${deadRun}-app'
elif [ "$1" = network ] && [ "$2" = ls ]; then
  printf '%s\n' '${liveRun}-network' '${stoppedLiveRun}-network' '${deadRun}-network'
elif [ "$1" = images ]; then
  printf '%s\n' 'localhost/mote-smoke:${liveRun}' 'localhost/mote-smoke:${stoppedLiveRun}' 'localhost/mote-smoke:${deadRun}'
fi
exit 0
`,
	);
	chmodSync(fakeEngine, 0o755);
	try {
		const result = spawnSync(
			"sh",
			[
				"-c",
				`. "${smokeAssets}"\nmote_reap_stale_smoke_assets "${fakeEngine}"`,
			],
			{ encoding: "utf8" },
		);
		assert.equal(result.status, 0, result.stderr);
		const calls = readFileSync(log, "utf8");
		assert.match(calls, new RegExp(`rm ${deadRun}-app`));
		assert.match(calls, new RegExp(`network rm ${deadRun}-network`));
		assert.match(calls, new RegExp(`image rm localhost/mote-smoke:${deadRun}`));
		assert.doesNotMatch(calls, new RegExp(`rm ${liveRun}-app`));
		assert.doesNotMatch(calls, new RegExp(`rm ${stoppedLiveRun}-app`));
		assert.doesNotMatch(calls, /system prune|volume prune|image prune/);
	} finally {
		rmSync(fixtureDirectory, { recursive: true, force: true });
	}
});
