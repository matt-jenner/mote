import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
for (const [mode, dependency, artifact, success] of [
	[
		"enabled",
		"libheif.so.1 => /usr/local/lib/libheif.so.1\nlibde265.so.0 => /usr/local/lib/libde265.so.0",
		"libheif.so.1",
		true,
	],
	["disabled", "libc.so.6 => /lib/libc.so.6", null, true],
	["disabled", "libheif.so.1 => /usr/local/lib/libheif.so.1", null, false],
	["disabled", "libc.so.6 => /lib/libc.so.6", "libde265.so.0", false],
	["enabled", "libheif.so.1 => not found", "libheif.so.1", false],
	[
		"enabled",
		"libheif.so.1 => /usr/local/lib/libheif.so.1\nlibde265.so.0 => /usr/local/lib/libde265.so.0",
		"heif-enc",
		false,
	],
]) {
	test(`Linux runtime inspection ${mode}, ${artifact}, ${dependency}: ${success}`, () => {
		const directory = fs.mkdtempSync(
			path.join(os.tmpdir(), "mote-runtime-test-"),
		);
		try {
			const bin = path.join(directory, "tools");
			const prefix = path.join(directory, "app");
			fs.mkdirSync(bin);
			fs.mkdirSync(path.join(prefix, "lib"), { recursive: true });
			fs.writeFileSync(path.join(prefix, "server"), "server");
			if (mode === "enabled")
				for (const name of ["libheif.so.1", "libde265.so.0"])
					fs.writeFileSync(path.join(prefix, "lib", name), "library");
			if (artifact)
				fs.writeFileSync(path.join(prefix, "lib", artifact), "library");
			for (const [tool, output] of [
				["ldd", dependency],
				["readelf", "(NEEDED) Shared library: [libc.so.6]"],
			])
				fs.writeFileSync(
					path.join(bin, tool),
					`#!/bin/sh\nprintf '%s\\n' '${output}'\n`,
					{ mode: 0o755 },
				);
			const result = spawnSync(
				"sh",
				[
					path.join(root, "packaging/heic/verify-linux-runtime.sh"),
					prefix,
					path.join(prefix, "server"),
					mode,
				],
				{
					encoding: "utf8",
					env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
				},
			);
			assert.equal(result.status === 0, success, result.stderr);
		} finally {
			fs.rmSync(directory, { recursive: true, force: true });
		}
	});
}
