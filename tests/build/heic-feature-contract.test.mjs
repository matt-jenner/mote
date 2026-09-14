import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");
const readManifest = (...segments) =>
	fs.readFileSync(path.join(root, ...segments), "utf8");

const serverManifest = readManifest("crates", "server", "Cargo.toml");
const desktopManifest = readManifest(
	"apps",
	"desktop",
	"src-tauri",
	"Cargo.toml",
);
const codecManifest = readManifest("crates", "codec", "Cargo.toml");
const internalManifests = [
	["catalog-bench", readManifest("crates", "catalog-bench", "Cargo.toml")],
	["catalog", readManifest("crates", "catalog", "Cargo.toml")],
	["metadata", readManifest("crates", "metadata", "Cargo.toml")],
	["indexer", readManifest("crates", "indexer", "Cargo.toml")],
	["cache", readManifest("crates", "cache", "Cargo.toml")],
	["core", readManifest("crates", "core", "Cargo.toml")],
	["app-service", readManifest("crates", "app-service", "Cargo.toml")],
	["server", serverManifest],
	["desktop", desktopManifest],
	["codec", codecManifest],
];

test("HEIC is a default feature with a stable non-HEIC bundle", () => {
	for (const manifest of [serverManifest, desktopManifest]) {
		assert.match(manifest, /default\s*=\s*\["mote-defaults",\s*"heic"\]/);
		assert.match(manifest, /mote-defaults\s*=\s*\[/);
		assert.match(manifest, /heic\s*=\s*\[/);
	}
});

test("the native binding is optional and never embedded", () => {
	assert.match(codecManifest, /libheif-rs.*optional\s*=\s*true/);
	assert.doesNotMatch(codecManifest, /embedded-libheif/);
});

test("internal dependencies do not activate defaults implicitly", () => {
	for (const [name, manifest] of internalManifests) {
		for (const line of manifest
			.split(/\r?\n/)
			.filter((line) => /photo-[a-z-]+\s*=/.test(line))) {
			assert.match(line, /default-features\s*=\s*false/, `${name}: ${line}`);
		}
	}
});
