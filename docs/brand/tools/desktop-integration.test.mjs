import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import test from "node:test";

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
