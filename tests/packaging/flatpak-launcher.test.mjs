import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const launcher = path.resolve(
	import.meta.dirname,
	"../../packaging/flatpak/mote.sh",
);

function runLauncher(value, args = [], exitStatus = 0) {
	const directory = fs.mkdtempSync(path.join(os.tmpdir(), "mote-launcher-"));
	try {
		const binary = path.join(directory, "mote-bin");
		fs.writeFileSync(
			binary,
			'#!/bin/sh\nprintf "%s\\0" "$__NV_DISABLE_EXPLICIT_SYNC" "$@"\nexit "$MOTE_TEST_EXIT_STATUS"\n',
		);
		fs.chmodSync(binary, 0o755);
		const source = fs.readFileSync(launcher, "utf8");
		assert.match(source, /exec \/app\/bin\/mote-bin "\$@"/);
		const localLauncher = path.join(directory, "mote");
		fs.writeFileSync(
			localLauncher,
			source.replace("/app/bin/mote-bin", binary),
		);
		const env = { ...process.env, MOTE_TEST_EXIT_STATUS: String(exitStatus) };
		if (value === undefined) delete env.__NV_DISABLE_EXPLICIT_SYNC;
		else env.__NV_DISABLE_EXPLICIT_SYNC = value;
		const result = spawnSync("sh", [localLauncher, ...args], { env });
		if (result.error) throw result.error;
		return {
			status: result.status,
			values: result.stdout.toString().split("\0").slice(0, -1),
			stderr: result.stderr.toString(),
		};
	} finally {
		fs.rmSync(directory, { recursive: true, force: true });
	}
}

test("launcher defaults explicit sync off when unset", () => {
	const result = runLauncher(undefined);
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(result.values, ["1"]);
});

test("launcher preserves an explicit value", () => {
	const result = runLauncher("0");
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(result.values, ["0"]);
});

test("launcher preserves an explicitly empty value", () => {
	const result = runLauncher("");
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(result.values, [""]);
});

test("launcher forwards argument boundaries", () => {
	const result = runLauncher(undefined, ["one two", "", "*.jpg"]);
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(result.values, ["1", "one two", "", "*.jpg"]);
});

test("launcher returns the executable exit status", () => {
	const result = runLauncher(undefined, [], 37);
	assert.equal(result.status, 37, result.stderr);
});
