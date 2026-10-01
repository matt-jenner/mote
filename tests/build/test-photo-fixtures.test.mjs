import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import sharp from "sharp";

const repositoryRoot = path.resolve(import.meta.dirname, "../..");
const generator = path.join(repositoryRoot, "scripts/generate-test-photos.mjs");
const names = [
	"city.jpg",
	"coast.jpg",
	"forest.jpg",
	"interior.jpg",
	"mountain.jpg",
	"portrait.jpg",
];

function generate(output) {
	execFileSync(process.execPath, [generator, "--output", output], {
		cwd: repositoryRoot,
		stdio: "pipe",
	});
}

test("generates deterministic local JPEG fixtures with both orientations", async () => {
	const first = mkdtempSync(path.join(os.tmpdir(), "mote-test-photos-first-"));
	const second = mkdtempSync(
		path.join(os.tmpdir(), "mote-test-photos-second-"),
	);
	generate(first);
	generate(second);

	const dimensions = [];
	for (const name of names) {
		const relative = path.join("demo-photos", name);
		const firstBytes = readFileSync(path.join(first, relative));
		const secondBytes = readFileSync(path.join(second, relative));
		assert.deepEqual(firstBytes, secondBytes, `${name} changed between runs`);
		assert.deepEqual([...firstBytes.subarray(0, 3)], [0xff, 0xd8, 0xff]);
		const metadata = await sharp(firstBytes).metadata();
		dimensions.push([metadata.width, metadata.height]);
		assert.ok(
			Math.max(metadata.width, metadata.height) >= 1536 &&
				Math.min(metadata.width, metadata.height) >= 1024,
			`${name} must be large enough to exercise viewer zoom controls`,
		);
	}

	assert.ok(dimensions.some(([width, height]) => width > height));
	assert.ok(dimensions.some(([width, height]) => height > width));
});
