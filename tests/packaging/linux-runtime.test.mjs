import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const smoke = fs.readFileSync(
	path.join(root, "scripts/hosted-smoke.sh"),
	"utf8",
);
const imageInspectionRoot = smoke.match(
	/-s -- (\/\S*) \/usr\/local\/bin\/photo-server/,
)?.[1];

for (const artifact of [
	null,
	"usr/lib/libheif.so.1",
	"lib/libde265.so.0",
	"usr/bin/heif-enc",
	"usr/include/libheif/heif.h",
]) {
	test(`hosted whole-image inspection ${artifact ?? "accepts Debian runtime and prunes virtual mounts"}`, () => {
		const directory = fs.mkdtempSync(
			path.join(os.tmpdir(), "mote-runtime-test-"),
		);
		try {
			const prefix = path.join(directory, "image");
			const bin = path.join(directory, "tools");
			fs.mkdirSync(bin);
			for (const name of [
				"usr/local/bin/photo-server",
				"usr/lib/libc.so.6",
				"etc/ld.so.conf",
				"var/lib/dpkg/status",
				"usr/share/doc/libc6/copyright",
				"proc/1/root/libheif.so.1",
				"sys/kernel/libde265.so.0",
				"dev/heif-enc",
				"run/libheif.so.1",
				"tmp/libde265.so.0",
				"var/tmp/libheif.so.1",
				...(artifact ? [artifact] : []),
			]) {
				fs.mkdirSync(path.dirname(path.join(prefix, name)), {
					recursive: true,
				});
				fs.writeFileSync(path.join(prefix, name), "fixture");
			}
			fs.symlinkSync("usr/bin", path.join(prefix, "bin"));
			for (const [name, output] of [
				["ldd", "libc.so.6 => /usr/lib/libc.so.6"],
				["readelf", "(NEEDED) Shared library: [libc.so.6]"],
			])
				fs.writeFileSync(
					path.join(bin, name),
					`#!/bin/sh\nprintf '%s\\n' '${output}'\n`,
					{ mode: 0o755 },
				);
			assert.ok(imageInspectionRoot, "hosted inspection root must be explicit");
			const result = spawnSync(
				"sh",
				[
					path.join(root, "packaging/heic/verify-linux-runtime.sh"),
					path.join(prefix, imageInspectionRoot),
					path.join(prefix, "usr/local/bin/photo-server"),
					"disabled",
				],
				{
					encoding: "utf8",
					env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
				},
			);
			if (artifact) {
				assert.notEqual(
					result.status,
					0,
					`accepted unlinked shipped artifact ${artifact}`,
				);
				assert.match(
					result.stderr,
					/disabled runtime contains decoder files|unexpected encoder or development file/,
				);
			} else {
				assert.equal(result.status, 0, result.stderr);
				assert.equal(result.stderr, "");
				// Exercise staging roots independently of the final-image command.
				const staged = spawnSync(
					"sh",
					[
						path.join(root, "packaging/heic/verify-linux-runtime.sh"),
						`${prefix}/`,
						path.join(prefix, "usr/local/bin/photo-server"),
						"disabled",
					],
					{
						encoding: "utf8",
						env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
					},
				);
				assert.equal(staged.status, 0, staged.stderr);
				assert.equal(staged.stderr, "");
			}
		} finally {
			fs.rmSync(directory, { recursive: true, force: true });
		}
	});
}
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
