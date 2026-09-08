import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { XMLParser } from "fast-xml-parser";
import { parse } from "yaml";

const APP_ID = "io.github.matt_jenner.mote";
const root = path.resolve(import.meta.dirname, "../..");
const packaging = path.join(root, "packaging/flatpak");

function read(filename) {
	return fs.readFileSync(path.join(packaging, filename), "utf8");
}

function sha256(filename) {
	return createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
}

test("Flatpak guide covers both local distributions and defers CI", () => {
	const guide = read("README.md");
	for (const text of [
		"Omarchy or Arch Linux",
		"Fedora",
		"npm run flatpak -- package",
		"npm run flatpak -- install",
		"io.github.matt_jenner.mote",
		"GitHub Actions is deferred",
		"portal",
	]) {
		assert.ok(
			guide.toLowerCase().includes(text.toLowerCase()),
			`missing guide text: ${text}`,
		);
	}
});

test("desktop entry launches Mote with the permanent application ID", () => {
	const desktop = read(`${APP_ID}.desktop`);
	for (const line of [
		"[Desktop Entry]",
		"Type=Application",
		"Name=Mote",
		"Exec=mote",
		`Icon=${APP_ID}`,
		"Terminal=false",
		"Categories=Graphics;Photography;",
	]) {
		assert.ok(
			desktop.split(/\r?\n/).includes(line),
			`missing desktop entry line: ${line}`,
		);
	}
});

test("AppStream metadata matches the Flatpak and desktop IDs", () => {
	const parsed = new XMLParser({
		ignoreAttributes: false,
		attributeNamePrefix: "@",
	}).parse(read(`${APP_ID}.metainfo.xml`));
	const component = parsed.component;
	assert.equal(component["@type"], "desktop-application");
	assert.equal(component.id, APP_ID);
	assert.equal(component.name, "Mote");
	assert.equal(component.metadata_license, "CC0-1.0");
	assert.equal(component.launchable["@type"], "desktop-id");
	assert.equal(component.launchable["#text"], `${APP_ID}.desktop`);
	assert.equal(component.url["@type"], "homepage");
	assert.equal(component.url["#text"], "https://github.com/matt-jenner/mote");
});

test("approved Linux icons cover Flatpak desktop integration", () => {
	for (const size of [
		"16x16",
		"24x24",
		"32x32",
		"48x48",
		"64x64",
		"128x128",
		"256x256",
		"512x512",
	]) {
		assert.ok(
			fs.existsSync(
				path.join(root, `docs/brand/icons/linux/hicolor/${size}/apps/mote.png`),
			),
		);
	}
	assert.ok(
		fs.existsSync(
			path.join(root, "docs/brand/icons/linux/hicolor/scalable/apps/mote.svg"),
		),
	);
	assert.ok(
		fs.existsSync(
			path.join(
				root,
				"docs/brand/icons/linux/hicolor/scalable/apps/mote-symbolic.svg",
			),
		),
	);
});

test("manifest pins the approved runtime and build SDKs", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	assert.equal(manifest.id, APP_ID);
	assert.equal(manifest.runtime, "org.gnome.Platform");
	assert.equal(manifest["runtime-version"], "49");
	assert.equal(manifest.sdk, "org.gnome.Sdk");
	assert.equal(manifest.command, "mote");
	assert.equal(manifest["default-branch"], "stable");
	assert.deepEqual(manifest["sdk-extensions"], [
		"org.freedesktop.Sdk.Extension.node24",
		"org.freedesktop.Sdk.Extension.rust-stable",
	]);
});

test("manifest grants display acceleration without host file or network access", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	assert.deepEqual(manifest["finish-args"], [
		"--socket=wayland",
		"--socket=fallback-x11",
		"--device=dri",
		"--share=ipc",
	]);
	const serialized = JSON.stringify(manifest["finish-args"]);
	assert.doesNotMatch(
		serialized,
		/--filesystem|--share=network|--socket=session-bus|--socket=system-bus/,
	);
});

test("manifest builds npm and Cargo offline and installs matching metadata", () => {
	const manifest = parse(read(`${APP_ID}.yml`));
	const module = manifest.modules.find(({ name }) => name === "mote");
	assert.equal(module.buildsystem, "simple");
	assert.equal(module["build-options"].env.CARGO_NET_OFFLINE, "true");
	assert.equal(module["build-options"].env.npm_config_offline, "true");
	assert.match(module["build-options"]["append-path"], /node24/);
	assert.match(module["build-options"]["append-path"], /rust-stable/);
	const commands = module["build-commands"].join("\n");
	assert.match(commands, /npm ci --offline/);
	assert.match(commands, /desktop:build -- --no-bundle --ci/);
	assert.match(commands, /target\/release\/photo-viewer-desktop/);
	assert.ok(commands.includes(`${APP_ID}.desktop`));
	assert.ok(commands.includes(`${APP_ID}.metainfo.xml`));
	assert.match(
		commands,
		/for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512/,
	);
	assert.ok(
		commands.includes(`/app/share/icons/hicolor/\${size}/apps/${APP_ID}.png`),
	);
	assert.ok(
		commands.includes(`/app/share/icons/hicolor/scalable/apps/${APP_ID}.svg`),
	);
	assert.ok(
		commands.includes(
			`/app/share/icons/hicolor/scalable/apps/${APP_ID}-symbolic.svg`,
		),
	);
	assert.ok(
		module.sources.some((source) => source === "generated/cargo-sources.json"),
	);
	assert.ok(
		module.sources.some((source) => source === "generated/node-sources.json"),
	);
});

test("generated Flatpak sources are nonempty and match both lockfiles", () => {
	const generated = path.join(packaging, "generated");
	const sourceLock = JSON.parse(
		fs.readFileSync(path.join(generated, "source-lock.json"), "utf8"),
	);
	assert.equal(
		sourceLock.lockfiles["package-lock.json"],
		sha256(path.join(root, "package-lock.json")),
	);
	assert.equal(
		sourceLock.lockfiles["apps/desktop/src-tauri/Cargo.lock"],
		sha256(path.join(root, "apps/desktop/src-tauri/Cargo.lock")),
	);
	assert.match(
		sourceLock.generator.repository,
		/^https:\/\/github\.com\/flatpak\/flatpak-builder-tools(?:\.git)?$/,
	);
	assert.match(sourceLock.generator.commit, /^[0-9a-f]{40}$/);
	for (const filename of ["node-sources.json", "cargo-sources.json"]) {
		const sources = JSON.parse(
			fs.readFileSync(path.join(generated, filename), "utf8"),
		);
		assert.ok(Array.isArray(sources));
		assert.ok(sources.length > 0);
	}
});
