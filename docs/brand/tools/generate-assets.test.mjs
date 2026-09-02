import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { XMLValidator } from "fast-xml-parser";
import { openSync } from "fontkit";
import sharp from "sharp";
import { expectedOutputs, PLATFORM_SIZES } from "./config.mjs";

const root = path.resolve("docs/brand");

async function findFiles(directory, extension) {
	const files = [];
	const queue = [directory];
	while (queue.length > 0) {
		const current = queue.pop();
		for (const entry of await fs.readdir(current, { withFileTypes: true })) {
			const filename = path.join(current, entry.name);
			if (entry.isDirectory()) queue.push(filename);
			else if (entry.name.endsWith(extension)) files.push(filename);
		}
	}
	return files;
}

test("every declared platform output exists", async () => {
	for (const output of expectedOutputs()) await fs.access(path.resolve(output));
});

test("Windows and Linux PNG dimensions match their filenames", async () => {
	const files = [
		...PLATFORM_SIZES.windows.map((size) => [
			`icons/windows/png/mote-${size}.png`,
			size,
		]),
		...PLATFORM_SIZES.linux.map((size) => [
			`icons/linux/hicolor/${size}x${size}/apps/mote.png`,
			size,
		]),
	];
	for (const [relative, size] of files) {
		const metadata = await sharp(path.join(root, relative)).metadata();
		assert.equal(metadata.width, size);
		assert.equal(metadata.height, size);
		assert.equal(metadata.hasAlpha, true);
	}
});

test("every SVG is valid XML and portable wordmarks contain no live text", async () => {
	const svgFiles = await findFiles(root, ".svg");
	assert.ok(svgFiles.length >= 15);
	for (const filename of svgFiles) {
		const svg = await fs.readFile(filename, "utf8");
		assert.equal(XMLValidator.validate(svg), true, filename);
		if (/wordmark|lockup/.test(filename)) {
			assert.match(svg, /<path\b/);
			assert.doesNotMatch(svg, /<text\b/i);
		}
	}
});

test("Windows ICO contains a readable PNG entry for every required size", async () => {
	const ico = await fs.readFile(path.join(root, "icons/windows/Mote.ico"));
	assert.equal(ico.readUInt16LE(0), 0);
	assert.equal(ico.readUInt16LE(2), 1);
	const count = ico.readUInt16LE(4);
	const sizes = [];
	for (let index = 0; index < count; index += 1) {
		const entry = 6 + index * 16;
		const width = ico[entry] || 256;
		const height = ico[entry + 1] || 256;
		assert.equal(width, height);
		assert.equal(ico.readUInt16LE(entry + 6), 32);
		const length = ico.readUInt32LE(entry + 8);
		const offset = ico.readUInt32LE(entry + 12);
		const image = ico.subarray(offset, offset + length);
		const isPng = image
			.subarray(0, 8)
			.equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
		if (isPng) {
			const metadata = await sharp(image).metadata();
			assert.equal(metadata.width, width);
			assert.equal(metadata.height, height);
		} else {
			assert.equal(image.readUInt32LE(0), 40);
			assert.equal(image.readInt32LE(4), width);
			assert.equal(image.readInt32LE(8) / 2, height);
			assert.equal(image.readUInt16LE(14), 32);
		}
		sizes.push(width);
	}
	assert.deepEqual(
		sizes.sort((a, b) => a - b),
		PLATFORM_SIZES.windows,
	);
});

test("macOS ICNS contains the complete modern PNG representation set", async () => {
	const icns = await fs.readFile(path.join(root, "icons/macos/Mote.icns"));
	assert.equal(icns.subarray(0, 4).toString("ascii"), "icns");
	assert.equal(icns.readUInt32BE(4), icns.length);
	const types = [];
	let offset = 8;
	while (offset < icns.length) {
		const type = icns.subarray(offset, offset + 4).toString("ascii");
		const length = icns.readUInt32BE(offset + 4);
		assert.ok(length > 8);
		const image = icns.subarray(offset + 8, offset + length);
		assert.equal(image.subarray(1, 4).toString("ascii"), "PNG");
		types.push(type);
		offset += length;
	}
	assert.deepEqual(types, [
		"icp4",
		"ic11",
		"icp5",
		"ic12",
		"ic07",
		"ic13",
		"ic08",
		"ic14",
		"ic09",
		"ic10",
	]);
});

test("PWA manifest declarations match real standard, maskable, and monochrome assets", async () => {
	const manifestPath = path.join(root, "icons/web/manifest-icons.json");
	const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
	assert.deepEqual(
		new Set(manifest.icons.map((icon) => icon.purpose)),
		new Set(["any", "maskable", "monochrome"]),
	);
	for (const icon of manifest.icons) {
		const filename = path.join(root, "icons/web", icon.src);
		await fs.access(filename);
		if (icon.type === "image/png") {
			const declared = Number(icon.sizes.split("x")[0]);
			const metadata = await sharp(filename).metadata();
			assert.equal(metadata.width, declared);
			assert.equal(metadata.height, declared);
		} else {
			assert.equal(icon.type, "image/svg+xml");
			assert.equal(icon.sizes, "any");
		}
	}
});

test("font package preserves names, both variation axes, and the OFL", async () => {
	const ttf = openSync(path.join(root, "fonts/Fredoka-Variable.ttf"));
	const woff2 = openSync(path.join(root, "fonts/Fredoka-Variable.woff2"));
	assert.equal(woff2.familyName, ttf.familyName);
	assert.equal(woff2.fullName, ttf.fullName);
	assert.deepEqual(woff2.variationAxes, ttf.variationAxes);
	assert.deepEqual(woff2.head.modified, ttf.head.modified);
	assert.ok(ttf.variationAxes.wght);
	assert.ok(ttf.variationAxes.wdth);
	const license = await fs.readFile(path.join(root, "fonts/OFL.txt"), "utf8");
	assert.match(license, /SIL OPEN FONT LICENSE Version 1\.1/);
});

test("print and preview exports have the approved formats and dimensions", async () => {
	const required = [
		"print/mote-lockup-light.svg",
		"print/mote-lockup-dark.svg",
		"print/mote-lockup-light-3000.png",
		"print/mote-lockup-dark-3000.png",
		"print/mote-brand-sheet-a4.pdf",
		"previews/mote-asset-contact-sheet.png",
		"previews/mote-small-size-check.png",
	];
	for (const relative of required) await fs.access(path.join(root, relative));

	for (const relative of [
		"print/mote-lockup-light-3000.png",
		"print/mote-lockup-dark-3000.png",
	]) {
		const metadata = await sharp(path.join(root, relative)).metadata();
		assert.equal(metadata.width, 3000);
		assert.equal(metadata.hasAlpha, true);
	}
	const contact = await sharp(
		path.join(root, "previews/mote-asset-contact-sheet.png"),
	).metadata();
	assert.deepEqual([contact.width, contact.height], [2400, 1600]);
	const small = await sharp(
		path.join(root, "previews/mote-small-size-check.png"),
	).metadata();
	assert.deepEqual([small.width, small.height], [1800, 600]);
});
