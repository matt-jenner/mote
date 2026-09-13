import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
	chmodSync,
	mkdtempSync,
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

test("hosted smoke gives standalone browser fixtures an isolated web root", () => {
	const script = readFileSync(smokeScript, "utf8");
	assert.match(
		script,
		/with-ephemeral-interface-assets\.sh["']? \\\n\s+npm run web:build -- --outDir/,
	);
	assert.match(script, /PHOTO_VIEWER_WEB_ROOT="\$browser_web_root"/);
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

test("hosted smoke removes its unique image after a failed build", () => {
	const fixtureDirectory = mkdtempSync(path.join(os.tmpdir(), "mote-engine-"));
	const fakeEngine = path.join(fixtureDirectory, "podman");
	const log = path.join(fixtureDirectory, "calls.log");
	writeFileSync(
		fakeEngine,
		`#!/bin/sh
printf '%s\n' "$*" >>"${log}"
if [ "$1" = info ]; then printf '%s\n' '[]'; exit 0; fi
if [ "$1" = build ]; then exit 31; fi
exit 0
`,
	);
	chmodSync(fakeEngine, 0o755);
	try {
		const result = spawnSync(smokeScript, [], {
			cwd: repositoryRoot,
			env: { ...process.env, CONTAINER_ENGINE: fakeEngine },
			encoding: "utf8",
		});
		assert.equal(result.status, 31, result.stderr);
		const calls = readFileSync(log, "utf8");
		const tag = calls.match(/--tag (localhost\/mote-smoke:[^ ]+)/)?.[1];
		assert.ok(tag);
		assert.match(calls, /build .*--layers=false/);
		assert.match(calls, new RegExp(`image rm ${tag.replaceAll("-", "\\-")}`));
		assert.doesNotMatch(calls, /system prune|volume prune|image prune/);
	} finally {
		rmSync(fixtureDirectory, { recursive: true, force: true });
	}
});

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
