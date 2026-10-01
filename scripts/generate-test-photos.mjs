import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const repositoryRoot = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"..",
);
const specs = [
	["city.jpg", 1536, 1024, [36, 62, 86]],
	["coast.jpg", 1536, 1024, [32, 112, 148]],
	["forest.jpg", 1536, 1024, [42, 105, 66]],
	["interior.jpg", 1536, 1024, [132, 92, 62]],
	["mountain.jpg", 1536, 1024, [88, 104, 126]],
	["portrait.jpg", 1024, 1536, [144, 72, 92]],
];

function outputRoot() {
	const outputIndex = process.argv.indexOf("--output");
	if (outputIndex === -1) return path.join(repositoryRoot, "runtime/test-photos");
	const value = process.argv[outputIndex + 1];
	if (!value) throw new Error("--output requires a directory");
	return path.resolve(value);
}

function pixels(width, height, base, seed) {
	const bytes = Buffer.alloc(width * height * 3);
	for (let y = 0; y < height; y += 1) {
		for (let x = 0; x < width; x += 1) {
			const offset = (y * width + x) * 3;
			const band = ((x >> 4) + (y >> 4) + seed) % 5;
			bytes[offset] = (base[0] + x / 3 + band * 9) % 256;
			bytes[offset + 1] = (base[1] + y / 3 + band * 7) % 256;
			bytes[offset + 2] = (base[2] + (x + y) / 5 + band * 5) % 256;
		}
	}
	return bytes;
}

const directory = path.join(outputRoot(), "demo-photos");
await mkdir(directory, { recursive: true });

for (const [index, [name, width, height, base]] of specs.entries()) {
	await sharp(pixels(width, height, base, index), {
		raw: { width, height, channels: 3 },
	})
		.jpeg({ quality: 78, chromaSubsampling: "4:4:4", progressive: false })
		.toFile(path.join(directory, name));
}

process.stdout.write(`generated ${specs.length} test photos in ${directory}\n`);
