import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const generated = path.join(root, "packaging/flatpak/generated");

for (const stale of ["file", "archive", "extra-file"]) {
	test(`offline refresh rejects ${stale} pollution before accepting any output`, () => {
		const directory = fs.mkdtempSync(
			path.join(os.tmpdir(), "mote-source-test-"),
		);
		try {
			const checkout = path.join(directory, "checkout");
			for (const file of [
				"scripts/update-flatpak-sources.sh",
				"scripts/build-lifecycle.sh",
				"scripts/flatpak-source-lock.mjs",
				"package-lock.json",
				"Cargo.lock",
				"apps/desktop/src-tauri/Cargo.lock",
				"packaging/heic/versions.env",
			]) {
				fs.mkdirSync(path.dirname(path.join(checkout, file)), {
					recursive: true,
				});
				fs.copyFileSync(path.join(root, file), path.join(checkout, file));
			}
			const output = path.join(checkout, "packaging/flatpak/generated");
			fs.cpSync(generated, output, { recursive: true });
			if (stale === "extra-file")
				fs.writeFileSync(
					path.join(output, "stale-cache.tar.gz"),
					"not a source record",
				);
			else {
				const file = path.join(output, "node-sources.json");
				const sources = JSON.parse(fs.readFileSync(file, "utf8"));
				sources.push({
					type: stale,
					url:
						stale === "file"
							? "https://registry.npmjs.org/obsolete/-/obsolete-0.1.0.tgz"
							: "https://private.example/obsolete.tgz",
					sha512: "a".repeat(128),
					dest: "flatpak-node/npm-cache/_cacache/content-v2/sha512/aa/aa",
				});
				fs.writeFileSync(file, JSON.stringify(sources));
			}
			const before = fs
				.readdirSync(output)
				.map((file) => [
					file,
					fs.readFileSync(path.join(output, file), "utf8"),
				]);
			const result = spawnSync(
				"bash",
				[path.join(checkout, "scripts/update-flatpak-sources.sh"), "--offline"],
				{
					encoding: "utf8",
					env: { ...process.env, MOTE_BUILD_TMP_ROOT: directory },
				},
			);
			assert.notEqual(result.status, 0, `accepted ${stale} pollution`);
			assert.match(
				result.stderr,
				stale === "extra-file"
					? /unexpected generated record/
					: /obsolete npm source/,
			);
			for (const [file, contents] of before)
				assert.equal(
					fs.readFileSync(path.join(output, file), "utf8"),
					contents,
				);
			assert.deepEqual(fs.readdirSync(path.dirname(output)), ["generated"]);
			assert.deepEqual(
				fs
					.readdirSync(directory)
					.filter((name) => name.startsWith("mote-build-")),
				[],
			);
		} finally {
			fs.rmSync(directory, { recursive: true, force: true });
		}
	});
}

for (const interrupt of [false, true]) {
	test(`source publication ${interrupt ? "TERM" : "failure"} rolls back the previous records`, () => {
		const directory = fs.mkdtempSync(
			path.join(os.tmpdir(), "mote-source-test-"),
		);
		try {
			const checkout = path.join(directory, "checkout");
			for (const file of [
				"scripts/update-flatpak-sources.sh",
				"scripts/build-lifecycle.sh",
				"scripts/flatpak-source-lock.mjs",
				"package-lock.json",
				"Cargo.lock",
				"apps/desktop/src-tauri/Cargo.lock",
				"packaging/heic/versions.env",
			]) {
				fs.mkdirSync(path.dirname(path.join(checkout, file)), {
					recursive: true,
				});
				fs.copyFileSync(path.join(root, file), path.join(checkout, file));
			}
			const output = path.join(checkout, "packaging/flatpak/generated");
			fs.cpSync(generated, output, { recursive: true });
			const before = fs.readFileSync(
				path.join(output, "source-lock.json"),
				"utf8",
			);
			const bin = path.join(directory, "bin");
			fs.mkdirSync(bin);
			fs.writeFileSync(
				path.join(bin, "mv"),
				`#!/bin/sh\ncase "$1" in */.generated.next.*) ${interrupt ? 'kill -TERM "$PPID"; exit 48' : "exit 49"} ;; esac\nexec /bin/mv "$@"\n`,
				{ mode: 0o755 },
			);
			const result = spawnSync(
				"bash",
				[path.join(checkout, "scripts/update-flatpak-sources.sh"), "--offline"],
				{
					encoding: "utf8",
					env: {
						...process.env,
						PATH: `${bin}:${process.env.PATH}`,
						MOTE_BUILD_TMP_ROOT: directory,
					},
				},
			);
			assert.equal(result.status, interrupt ? 143 : 49, result.stderr);
			assert.equal(
				fs.readFileSync(path.join(output, "source-lock.json"), "utf8"),
				before,
			);
			assert.deepEqual(fs.readdirSync(path.dirname(output)), ["generated"]);
			assert.deepEqual(
				fs
					.readdirSync(directory)
					.filter((name) => name.startsWith("mote-build-")),
				[],
			);
		} finally {
			fs.rmSync(directory, { recursive: true, force: true });
		}
	});
}

test("offline source refresh covers both Cargo locks and refuses incomplete npm sources before publication", () => {
	const directory = fs.mkdtempSync(path.join(os.tmpdir(), "mote-source-test-"));
	try {
		fs.cpSync(generated, directory, { recursive: true });
		const args = [
			path.join(root, "scripts/flatpak-source-lock.mjs"),
			root,
			directory,
		];
		const success = spawnSync(process.execPath, args, { encoding: "utf8" });
		assert.equal(success.status, 0, success.stderr);
		assert.deepEqual(fs.readdirSync(directory).sort(), [
			"cargo-sources.json",
			"node-sources.json",
			"source-lock.json",
		]);
		assert.deepEqual(
			fs.readFileSync(path.join(directory, "node-sources.json")),
			fs.readFileSync(path.join(generated, "node-sources.json")),
			"generator browser/runtime sources must be preserved",
		);
		const cargo = JSON.parse(
			fs.readFileSync(path.join(directory, "cargo-sources.json")),
		);
		assert.ok(
			cargo.some(
				(source) =>
					source.url ===
					"https://static.crates.io/crates/libheif-rs/libheif-rs-3.0.0.crate",
			),
		);
		assert.ok(
			cargo.some(
				(source) =>
					source.url ===
					"https://static.crates.io/crates/libheif-sys/libheif-sys-5.3.1+1.23.1.crate",
			),
		);
		const stable = fs.readFileSync(
			path.join(directory, "source-lock.json"),
			"utf8",
		);
		fs.writeFileSync(
			path.join(directory, "unexpected-generator-output"),
			"extra",
		);
		const extra = spawnSync(process.execPath, args, { encoding: "utf8" });
		assert.notEqual(extra.status, 0);
		assert.match(extra.stderr, /unexpected generated record/);
		assert.equal(
			fs.readFileSync(path.join(directory, "source-lock.json"), "utf8"),
			stable,
		);
		fs.unlinkSync(path.join(directory, "unexpected-generator-output"));
		fs.writeFileSync(path.join(directory, "node-sources.json"), "[]");
		const failed = spawnSync(process.execPath, args, { encoding: "utf8" });
		assert.notEqual(failed.status, 0);
		assert.match(failed.stderr, /npm source/);
		assert.equal(
			fs.readFileSync(path.join(directory, "source-lock.json"), "utf8"),
			stable,
		);
	} finally {
		fs.rmSync(directory, { recursive: true, force: true });
	}
});

test("failed external source generation preserves every stable output and removes managed staging", () => {
	const directory = fs.mkdtempSync(path.join(os.tmpdir(), "mote-source-test-"));
	try {
		const bin = path.join(directory, "bin");
		const tools = path.join(directory, "tools");
		const checkout = path.join(directory, "checkout");
		for (const file of [
			"scripts/update-flatpak-sources.sh",
			"scripts/build-lifecycle.sh",
			"package.json",
			"package-lock.json",
			"apps/desktop/package.json",
			"apps/interface/package.json",
			"packaging/flatpak/generated/node-sources.json",
			"packaging/flatpak/generated/cargo-sources.json",
			"packaging/flatpak/generated/source-lock.json",
		]) {
			fs.mkdirSync(path.dirname(path.join(checkout, file)), {
				recursive: true,
			});
			fs.copyFileSync(path.join(root, file), path.join(checkout, file));
		}
		const generated = path.join(checkout, "packaging/flatpak/generated");
		fs.mkdirSync(bin);
		fs.mkdirSync(path.join(tools, "cargo"), { recursive: true });
		fs.writeFileSync(path.join(tools, "cargo/flatpak-cargo-generator.py"), "");
		for (const [name, body] of [
			["git", "exit 0"],
			[
				"flatpak-node-generator",
				'while [ $# -gt 0 ]; do if [ "$1" = --output ]; then shift; printf broken > "$1"; fi; shift; done\nexit 37',
			],
		]) {
			fs.writeFileSync(path.join(bin, name), `#!/bin/sh\n${body}\n`, {
				mode: 0o755,
			});
		}
		const before = fs
			.readdirSync(generated)
			.map((name) => [
				name,
				fs.readFileSync(path.join(generated, name), "utf8"),
			]);
		const result = spawnSync(
			"bash",
			[path.join(checkout, "scripts/update-flatpak-sources.sh"), tools],
			{
				encoding: "utf8",
				env: {
					...process.env,
					PATH: `${bin}:${process.env.PATH}`,
					MOTE_BUILD_TMP_ROOT: directory,
				},
			},
		);
		assert.equal(result.status, 37, result.stderr);
		for (const [name, contents] of before)
			assert.equal(
				fs.readFileSync(path.join(generated, name), "utf8"),
				contents,
			);
		assert.deepEqual(
			fs
				.readdirSync(directory)
				.filter((name) => name.startsWith("mote-build-")),
			[],
		);
	} finally {
		fs.rmSync(directory, { recursive: true, force: true });
	}
});
