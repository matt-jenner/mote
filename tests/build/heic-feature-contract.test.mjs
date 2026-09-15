import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../..");

function cargoMetadata(manifestPath) {
	return JSON.parse(
		execFileSync(
			"cargo",
			[
				"metadata",
				"--format-version",
				"1",
				"--no-deps",
				"--manifest-path",
				manifestPath,
			],
			{ cwd: root, encoding: "utf8" },
		),
	);
}

function packageNamed(metadata, name) {
	const package_ = metadata.packages.find(
		(candidate) => candidate.name === name,
	);
	assert.ok(package_, `Cargo metadata must contain ${name}`);
	return package_;
}

function assertMoteDefaultsMirror(package_, name) {
	const expected = package_.features.default
		.filter((feature) => feature !== "heic" && feature !== "mote-defaults")
		.toSorted();
	const actual = package_.features["mote-defaults"].toSorted();
	assert.deepEqual(actual, expected, `${name} non-HEIC defaults drifted`);
}

const rootMetadata = cargoMetadata(path.join(root, "Cargo.toml"));
const desktopMetadata = cargoMetadata(
	path.join(root, "apps/desktop/src-tauri/Cargo.toml"),
);
const serverPackage = packageNamed(rootMetadata, "photo-server");
const desktopPackage = packageNamed(desktopMetadata, "photo-viewer-desktop");
const codecPackage = packageNamed(rootMetadata, "photo-codec");

test("HEIC is a default feature with a stable non-HEIC bundle", () => {
	for (const [name, package_] of [
		["server", serverPackage],
		["desktop", desktopPackage],
	]) {
		assert.deepEqual(package_.features.default, ["mote-defaults", "heic"]);
		assert.ok(package_.features["mote-defaults"], `${name}: mote-defaults`);
		assert.ok(package_.features.heic, `${name}: heic`);
	}
});

test("mote-defaults mirrors every non-HEIC entry-crate default", () => {
	assertMoteDefaultsMirror(serverPackage, "server");
	assertMoteDefaultsMirror(desktopPackage, "desktop");
});

test("commented feature strings cannot satisfy mote-defaults drift", () => {
	const fixture = fs.mkdtempSync(
		path.join(os.tmpdir(), "mote-feature-contract-"),
	);
	try {
		fs.mkdirSync(path.join(fixture, "src"));
		fs.writeFileSync(path.join(fixture, "src/lib.rs"), "");
		fs.writeFileSync(
			path.join(fixture, "Cargo.toml"),
			`[package]
name = "comment-regression"
version = "0.1.0"
edition = "2024"

[features]
default = ["mote-defaults", "heic", "normal-feature"]
mote-defaults = [
  # "normal-feature"
]
heic = []
normal-feature = []
`,
		);
		const metadata = cargoMetadata(path.join(fixture, "Cargo.toml"));
		const package_ = packageNamed(metadata, "comment-regression");
		assert.throws(
			() => assertMoteDefaultsMirror(package_, "comment regression"),
			/comment regression non-HEIC defaults drifted/,
		);
	} finally {
		fs.rmSync(fixture, { recursive: true, force: true });
	}
});

test("the native binding is optional and never embedded", () => {
	const binding = codecPackage.dependencies.find(
		(dependency) => dependency.name === "libheif-rs",
	);
	assert.ok(binding);
	assert.equal(binding.optional, true);
	assert.equal(binding.uses_default_features, false);
	assert.deepEqual(binding.features, ["v1_23"]);
	assert.equal(
		binding.features.some((feature) => feature.includes("embedded-libheif")),
		false,
	);
});

test("internal dependencies do not activate defaults implicitly", () => {
	for (const package_ of [
		...rootMetadata.packages,
		...desktopMetadata.packages,
	]) {
		for (const dependency of package_.dependencies.filter(
			(candidate) =>
				candidate.source === null && candidate.name.startsWith("photo-"),
		)) {
			assert.equal(
				dependency.uses_default_features,
				false,
				`${package_.name}: ${dependency.name}`,
			);
		}
	}
});
