import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const verifier = path.join(root, "scripts/verify-macos-app-launch.sh");

function appFixture(executable) {
	const directory = fs.mkdtempSync(path.join(os.tmpdir(), "mote-launch-test-"));
	const app = path.join(directory, "Mote.app");
	const contents = path.join(app, "Contents");
	const macOS = path.join(contents, "MacOS");
	fs.mkdirSync(macOS, { recursive: true });
	fs.writeFileSync(
		path.join(contents, "Info.plist"),
		`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleExecutable</key><string>photo-viewer-desktop</string></dict></plist>
`,
	);
	const binary = path.join(macOS, "photo-viewer-desktop");
	fs.writeFileSync(binary, executable);
	fs.chmodSync(binary, 0o755);
	return { app, directory };
}

test("macOS launch verification accepts an app that remains running", {
	skip: process.platform !== "darwin",
}, () => {
	const fixture = appFixture("#!/bin/sh\nexec /bin/sleep 300\n");
	try {
		const result = spawnSync(verifier, [fixture.app], {
			env: { ...process.env, MOTE_MACOS_LAUNCH_SECONDS: "0.5" },
			encoding: "utf8",
		});
		assert.equal(result.error, undefined, result.error?.message);
		assert.equal(result.status, 0, result.stderr);
		assert.match(result.stdout, /macOS app launch check passed/);
	} finally {
		fs.rmSync(fixture.directory, { recursive: true, force: true });
	}
});

test("macOS launch verification rejects an app that exits during startup", {
	skip: process.platform !== "darwin",
}, () => {
	const fixture = appFixture(
		"#!/bin/sh\nprintf 'simulated loader failure\\n' >&2\nexit 42\n",
	);
	try {
		const result = spawnSync(verifier, [fixture.app], {
			env: { ...process.env, MOTE_MACOS_LAUNCH_SECONDS: "0.5" },
			encoding: "utf8",
		});
		assert.equal(result.error, undefined, result.error?.message);
		assert.notEqual(result.status, 0);
		assert.match(result.stderr, /simulated loader failure/);
		assert.match(result.stderr, /exited during startup/);
	} finally {
		fs.rmSync(fixture.directory, { recursive: true, force: true });
	}
});

test("macOS launch verification falls back when process inspection is unavailable", {
	skip: process.platform !== "darwin",
}, () => {
	const fixture = appFixture("#!/bin/sh\nexec /bin/sleep 1\n");
	const fakeBin = path.join(fixture.directory, "bin");
	fs.mkdirSync(fakeBin);
	fs.writeFileSync(path.join(fakeBin, "ps"), "#!/bin/sh\nexit 1\n");
	fs.chmodSync(path.join(fakeBin, "ps"), 0o755);
	try {
		const result = spawnSync(verifier, [fixture.app], {
			env: {
				...process.env,
				MOTE_MACOS_LAUNCH_SECONDS: "0.1",
				PATH: `${fakeBin}:${process.env.PATH}`,
			},
			encoding: "utf8",
		});
		assert.equal(result.error, undefined, result.error?.message);
		assert.equal(result.status, 0, result.stderr);
		assert.match(result.stdout, /macOS app launch check passed/);
	} finally {
		fs.rmSync(fixture.directory, { recursive: true, force: true });
	}
});
