import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const repositoryRoot = path.resolve(import.meta.dirname, "../../..");
const tauriConfig = JSON.parse(
	fs.readFileSync(
		path.join(repositoryRoot, "apps/desktop/src-tauri/tauri.conf.json"),
		"utf8",
	),
);

const shippingIconDir = "apps/desktop/src-tauri/icons";
const macosBrandIconDir = "docs/brand/icons/macos";

function sha256(filename) {
	return createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
}

test("Tauri ships the approved Mote PNG icon", () => {
	assert.equal(
		sha256(`${shippingIconDir}/icon.png`),
		sha256(`${macosBrandIconDir}/icon-1024.png`),
	);
});

test("Tauri ships the approved Mote ICNS icon", () => {
	assert.equal(
		sha256(`${shippingIconDir}/icon.icns`),
		sha256(`${macosBrandIconDir}/Mote.icns`),
	);
});

test("Tauri uses the permanent Mote application identifier", () => {
	assert.equal(tauriConfig.identifier, "io.github.matt_jenner.mote");
});
