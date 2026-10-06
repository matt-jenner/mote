import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const builder = path.join(root, "scripts/desktop-build-windows.mjs");

test("package command exposes the managed Windows desktop build", () => {
	const packageJson = JSON.parse(
		fs.readFileSync(path.join(root, "package.json"), "utf8"),
	);
	assert.equal(
		packageJson.scripts["desktop:build:windows"],
		"node scripts/desktop-build-windows.mjs",
	);
});

test("Windows build preserves the runner Path while adding decoder DLLs", () => {
	const source = fs.readFileSync(builder, "utf8");
	assert.match(source, /Object\.keys\(environment\)\.find\(/);
	assert.match(source, /name\.toLowerCase\(\) === "path"/);
	assert.match(source, /const currentPath = environment\[pathName\]/);
	assert.doesNotMatch(source, /environment\.PATH\s*=/);
});

function writeExecutable(file, contents) {
	fs.writeFileSync(file, contents);
	fs.chmodSync(file, 0o755);
}

function fixture() {
	const repository = fs.mkdtempSync(
		path.join(os.tmpdir(), "mote-windows-build-"),
	);
	for (const directory of [
		"scripts",
		"packaging/heic",
		"packaging/licenses",
		"apps/desktop/src-tauri",
		"docs/brand/icons/windows",
		"apps/interface",
		"node_modules/@tauri-apps/cli",
		"bin",
		"tmp",
	]) {
		fs.mkdirSync(path.join(repository, directory), { recursive: true });
	}
	fs.copyFileSync(
		builder,
		path.join(repository, "scripts/desktop-build-windows.mjs"),
	);
	fs.writeFileSync(
		path.join(repository, "apps/desktop/src-tauri/tauri.conf.json"),
		JSON.stringify({ version: "0.1.0" }),
	);
	for (const file of [
		"THIRD_PARTY_NOTICES.md",
		"packaging/licenses/LGPL-3.0-or-later.txt",
		"packaging/licenses/libheif.md",
		"packaging/licenses/libde265.md",
		"packaging/heic/README.md",
		"packaging/heic/decode-only.cmake",
		"packaging/heic/build-windows.ps1",
		"packaging/heic/verify-native-deps.ps1",
		"docs/brand/icons/windows/Mote.ico",
	]) {
		fs.mkdirSync(path.dirname(path.join(repository, file)), {
			recursive: true,
		});
		fs.writeFileSync(path.join(repository, file), file);
	}
	const log = path.join(repository, "commands.jsonl");
	writeExecutable(
		path.join(repository, "bin/pwsh"),
		`#!/bin/sh
printf '%s\n' "$(node -e 'console.log(JSON.stringify({tool:"pwsh",args:process.argv.slice(1)}))' -- "$@")" >> "$FAKE_TOOL_LOG"
case "$*" in
  *build-windows.ps1*)
    prefix="$PWD/build/heic-native/windows-x64/installed/x64-windows"
    mkdir -p "$prefix/bin" "$prefix/lib"
    printf dll > "$prefix/bin/heif.dll"
    printf dll > "$prefix/bin/libde265.dll"
    printf lib > "$prefix/lib/heif.lib"
    printf lib > "$prefix/lib/de265.lib"
    ;;
esac
`,
	);
	writeExecutable(
		path.join(repository, "node_modules/@tauri-apps/cli/tauri.js"),
		`#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");
fs.appendFileSync(process.env.FAKE_TOOL_LOG, JSON.stringify({ tool: "tauri", args: process.argv.slice(2) }) + "\\n");
const target = process.env.CARGO_TARGET_DIR;
fs.mkdirSync(path.join(process.cwd(), "apps/interface/dist"), { recursive: true });
fs.writeFileSync(path.join(process.cwd(), "apps/interface/dist/index.html"), "generated");
if (process.env.FAKE_TAURI_FAIL === "1") process.exit(31);
fs.mkdirSync(path.join(target, "release/bundle/nsis"), { recursive: true });
fs.writeFileSync(path.join(target, "release/photo-viewer-desktop.exe"), "binary");
fs.writeFileSync(path.join(target, "release/bundle/nsis/Mote_0.1.0_x64-setup.exe"), "installer");
`,
	);
	return { repository, log };
}

function runBuild(repository, log, arguments_ = [], extraEnv = {}) {
	return spawnSync(
		process.execPath,
		[path.join(repository, "scripts/desktop-build-windows.mjs"), ...arguments_],
		{
			cwd: repository,
			env: {
				...process.env,
				npm_execpath: path.join(repository, "missing-npm-cli.cjs"),
				...extraEnv,
				FAKE_TOOL_LOG: log,
				MOTE_BUILD_TMP_ROOT: path.join(repository, "tmp"),
				PATH: `${path.join(repository, "bin")}${path.delimiter}${process.env.PATH}`,
			},
			encoding: "utf8",
		},
	);
}

function readCommands(log) {
	return fs
		.readFileSync(log, "utf8")
		.trim()
		.split("\n")
		.map((line) => JSON.parse(line));
}

test("successful Windows build publishes the versioned installer and removes ephemeral assets", () => {
	assert.ok(fs.existsSync(builder), "Windows desktop builder is missing");
	const { repository, log } = fixture();
	try {
		const result = runBuild(repository, log, ["--ci"]);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		assert.equal(
			fs.readFileSync(
				path.join(repository, "dist/windows/Mote-0.1.0-windows-x64-setup.exe"),
				"utf8",
			),
			"installer",
		);
		assert.deepEqual(fs.readdirSync(path.join(repository, "tmp")), []);
		assert.equal(
			fs.existsSync(path.join(repository, "apps/interface/dist")),
			false,
		);
		const commands = readCommands(log);
		const tauri = commands.find((command) => command.tool === "tauri");
		assert.ok(tauri.args.includes("--ci"));
		const override = JSON.parse(tauri.args[tauri.args.indexOf("--config") + 1]);
		assert.deepEqual(override.bundle.targets, ["nsis"]);
		assert.deepEqual(override.bundle.windows, {
			certificateThumbprint: null,
			signCommand: null,
			timestampUrl: null,
		});
		assert.ok(
			override.bundle.icon[0].endsWith("docs/brand/icons/windows/Mote.ico"),
		);
		assert.deepEqual(
			Object.values(override.bundle.resources).sort(),
			[
				"heif.dll",
				"libde265.dll",
				"licenses/HEIC-REBUILD.md",
				"licenses/LGPL-3.0-or-later.txt",
				"licenses/THIRD_PARTY_NOTICES.md",
				"licenses/decode-only.cmake",
				"licenses/libde265.md",
				"licenses/libheif.md",
			].sort(),
		);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("failed Windows build preserves the accepted installer and removes ephemeral assets", () => {
	assert.ok(fs.existsSync(builder), "Windows desktop builder is missing");
	const { repository, log } = fixture();
	const destination = path.join(
		repository,
		"dist/windows/Mote-0.1.0-windows-x64-setup.exe",
	);
	try {
		fs.mkdirSync(path.dirname(destination), { recursive: true });
		fs.writeFileSync(destination, "accepted");
		const result = runBuild(repository, log, [], { FAKE_TAURI_FAIL: "1" });
		assert.notEqual(result.status, 0, result.stdout + result.stderr);
		assert.equal(fs.readFileSync(destination, "utf8"), "accepted");
		assert.deepEqual(fs.readdirSync(path.join(repository, "tmp")), []);
		assert.equal(
			fs.existsSync(path.join(repository, "apps/interface/dist")),
			false,
		);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("no-HEIC Windows build skips native compilation and verifies the disabled binary", () => {
	assert.ok(fs.existsSync(builder), "Windows desktop builder is missing");
	const { repository, log } = fixture();
	try {
		const result = runBuild(repository, log, ["--no-heic"]);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		const commands = readCommands(log);
		const powershellCommands = commands.filter(
			(command) => command.tool === "pwsh",
		);
		assert.equal(
			powershellCommands.some((command) =>
				command.args.some((argument) => argument.endsWith("build-windows.ps1")),
			),
			false,
		);
		assert.ok(
			powershellCommands.some(
				(command) =>
					command.args.some((argument) =>
						argument.endsWith("verify-native-deps.ps1"),
					) && command.args.includes("-NoHeic"),
			),
			JSON.stringify(commands),
		);
		const tauri = commands.find((command) => command.tool === "tauri");
		assert.ok(tauri.args.includes("--no-default-features"));
		assert.deepEqual(
			tauri.args.slice(
				tauri.args.indexOf("--features"),
				tauri.args.indexOf("--features") + 2,
			),
			["--features", "mote-defaults"],
		);
		const override = JSON.parse(tauri.args[tauri.args.indexOf("--config") + 1]);
		assert.equal(override.bundle.resources, undefined);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("Windows build reaps abandoned managed directories and preserves a live build", () => {
	assert.ok(fs.existsSync(builder), "Windows desktop builder is missing");
	const { repository, log } = fixture();
	const stale = path.join(repository, "tmp/mote-build-windows-stale");
	const live = path.join(repository, "tmp/mote-build-windows-live");
	try {
		for (const [directory, owner] of [
			[stale, "999999999"],
			[live, String(process.pid)],
		]) {
			fs.mkdirSync(directory);
			fs.writeFileSync(path.join(directory, ".mote-owner-pid"), `${owner}\n`);
			fs.writeFileSync(path.join(directory, "asset"), "generated");
		}
		const result = runBuild(repository, log);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		assert.equal(fs.existsSync(stale), false);
		assert.equal(fs.existsSync(live), true);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});

test("Windows build invokes Tauri through Node without an npm command shim", () => {
	assert.ok(fs.existsSync(builder), "Windows desktop builder is missing");
	const { repository, log } = fixture();
	try {
		const result = runBuild(repository, log);
		assert.equal(result.status, 0, result.stdout + result.stderr);
		const commands = readCommands(log);
		assert.ok(commands.some((command) => command.tool === "tauri"));
		assert.equal(
			commands.some((command) => command.tool === "npm"),
			false,
		);
	} finally {
		fs.rmSync(repository, { recursive: true, force: true });
	}
});
