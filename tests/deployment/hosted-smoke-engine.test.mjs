import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const repositoryRoot = path.resolve(import.meta.dirname, "../..");
const smokeScript = path.join(repositoryRoot, "scripts/hosted-smoke.sh");

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
