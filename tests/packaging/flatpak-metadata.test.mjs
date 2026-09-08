import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { XMLParser } from "fast-xml-parser";

const APP_ID = "io.github.matt_jenner.mote";
const root = path.resolve(import.meta.dirname, "../..");
const packaging = path.join(root, "packaging/flatpak");

function read(filename) {
	return fs.readFileSync(path.join(packaging, filename), "utf8");
}

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
