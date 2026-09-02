import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { XMLValidator } from "fast-xml-parser";
import { openSync } from "fontkit";
import pngToIco from "png-to-ico";
import sharp from "sharp";
import { BRAND, expectedOutputs, PATHS, PLATFORM_SIZES } from "./config.mjs";
import { exportSvgSources, horizontalLockupSvg, symbolSvg } from "./svg.mjs";

const execFileAsync = promisify(execFile);
const generatedDirectories = [
	"source",
	"svg",
	"fonts",
	"icons",
	"print",
	"previews",
];

async function renderPng(svg, size, output) {
	await fs.mkdir(path.dirname(output), { recursive: true });
	await sharp(Buffer.from(svg))
		.resize(size, size)
		.png({ compressionLevel: 9 })
		.withMetadata({ icc: "srgb" })
		.toFile(output);
}

async function renderWidthPng(svg, width, output) {
	await fs.mkdir(path.dirname(output), { recursive: true });
	await sharp(Buffer.from(svg))
		.resize({ width })
		.png({ compressionLevel: 9 })
		.withMetadata({ icc: "srgb" })
		.toFile(output);
}

async function writeText(output, contents) {
	await fs.mkdir(path.dirname(output), { recursive: true });
	await fs.writeFile(output, `${contents.trim()}\n`, "utf8");
}

async function writeIco(pngPaths, output) {
	await fs.mkdir(path.dirname(output), { recursive: true });
	await fs.writeFile(output, await pngToIco(pngPaths));
}

async function writeModernIcns(iconset, output) {
	const representations = [
		["icp4", "icon_16x16.png"],
		["ic11", "icon_16x16@2x.png"],
		["icp5", "icon_32x32.png"],
		["ic12", "icon_32x32@2x.png"],
		["ic07", "icon_128x128.png"],
		["ic13", "icon_128x128@2x.png"],
		["ic08", "icon_256x256.png"],
		["ic14", "icon_256x256@2x.png"],
		["ic09", "icon_512x512.png"],
		["ic10", "icon_512x512@2x.png"],
	];
	const chunks = [];
	for (const [type, filename] of representations) {
		const image = await fs.readFile(path.join(iconset, filename));
		const header = Buffer.alloc(8);
		header.write(type, 0, 4, "ascii");
		header.writeUInt32BE(image.length + 8, 4);
		chunks.push(header, image);
	}
	const body = Buffer.concat(chunks);
	const header = Buffer.alloc(8);
	header.write("icns", 0, 4, "ascii");
	header.writeUInt32BE(body.length + 8, 4);
	await fs.writeFile(output, Buffer.concat([header, body]));
}

async function copyFontPackage(stage) {
	const fontDir = path.join(stage, "fonts");
	await fs.mkdir(fontDir, { recursive: true });
	await fs.copyFile(
		PATHS.sourceFont,
		path.join(fontDir, "Fredoka-Variable.ttf"),
	);
	await fs.copyFile(PATHS.sourceLicense, path.join(fontDir, "OFL.txt"));
	await execFileAsync(process.env.BRAND_PYTHON ?? "python3", [
		path.resolve("docs/brand/tools/convert_font.py"),
		path.join(fontDir, "Fredoka-Variable.ttf"),
		path.join(fontDir, "Fredoka-Variable.woff2"),
	]);
}

function macIconSvg(mode) {
	return symbolSvg({ mode, tile: true, size: 1024 })
		.replace(
			/(<svg[^>]*>)/,
			'$1<defs><filter id="tile-shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="18" stdDeviation="22" flood-color="#000000" flood-opacity="0.22"/></filter></defs>',
		)
		.replace(
			'data-layer="tile"',
			'data-layer="tile" filter="url(#tile-shadow)"',
		);
}

async function generateMac(stage) {
	const directory = path.join(stage, "icons/macos");
	const iconset = path.join(directory, "Mote.iconset");
	await fs.mkdir(iconset, { recursive: true });
	await renderPng(
		macIconSvg("light"),
		1024,
		path.join(directory, "icon-1024.png"),
	);
	await renderPng(
		macIconSvg("dark"),
		1024,
		path.join(directory, "icon-1024-dark.png"),
	);

	const entries = [
		[16, "icon_16x16.png"],
		[32, "icon_16x16@2x.png"],
		[32, "icon_32x32.png"],
		[64, "icon_32x32@2x.png"],
		[128, "icon_128x128.png"],
		[256, "icon_128x128@2x.png"],
		[256, "icon_256x256.png"],
		[512, "icon_256x256@2x.png"],
		[512, "icon_512x512.png"],
		[1024, "icon_512x512@2x.png"],
	];
	await Promise.all(
		entries.map(([size, filename]) =>
			renderPng(
				symbolSvg({ mode: "light", tile: true, size }),
				size,
				path.join(iconset, filename),
			),
		),
	);
	await writeModernIcns(iconset, path.join(directory, "Mote.icns"));
}

async function generateWindows(stage) {
	const directory = path.join(stage, "icons/windows");
	const pngDirectory = path.join(directory, "png");
	const pngs = [];
	for (const size of PLATFORM_SIZES.windows) {
		const output = path.join(pngDirectory, `mote-${size}.png`);
		await renderPng(
			symbolSvg({ mode: "light", tile: true, size }),
			size,
			output,
		);
		pngs.push(output);
	}
	await writeIco(pngs, path.join(directory, "Mote.ico"));
}

async function generateLinux(stage) {
	const hicolor = path.join(stage, "icons/linux/hicolor");
	for (const size of PLATFORM_SIZES.linux) {
		await renderPng(
			symbolSvg({ mode: "light", tile: true, size }),
			size,
			path.join(hicolor, `${size}x${size}/apps/mote.png`),
		);
	}
	const scalable = path.join(hicolor, "scalable/apps");
	await writeText(
		path.join(scalable, "mote.svg"),
		symbolSvg({ mode: "light", tile: true }),
	);
	await writeText(
		path.join(scalable, "mote-symbolic.svg"),
		symbolSvg({ mode: "light", monochrome: true }),
	);
	await writeText(
		path.join(scalable, "mote-symbolic-dark.svg"),
		symbolSvg({ mode: "dark", monochrome: true }),
	);
}

function adaptiveFaviconSvg() {
	return symbolSvg({ mode: "light", tile: true })
		.replace(
			/(<svg[^>]*>)/,
			`$1<style>:root{--mote-tile:${BRAND.colors.graphite};--mote-rear:${BRAND.colors.graphite}}@media(prefers-color-scheme:dark){:root{--mote-tile:${BRAND.colors.darkTile};--mote-rear:${BRAND.colors.white}}}</style>`,
		)
		.replace(`fill="${BRAND.colors.graphite}"`, 'fill="var(--mote-tile)"')
		.replace(`stroke="${BRAND.colors.graphite}"`, 'stroke="var(--mote-rear)"');
}

async function generateWeb(stage) {
	const directory = path.join(stage, "icons/web");
	await fs.mkdir(directory, { recursive: true });

	const standard = [192, 512];
	for (const size of standard) {
		await renderPng(
			symbolSvg({ mode: "light", tile: true, size }),
			size,
			path.join(directory, `icon-${size}.png`),
		);
		await renderPng(
			symbolSvg({ mode: "light", tile: true, maskable: true, size }),
			size,
			path.join(directory, `maskable-${size}.png`),
		);
	}
	await renderPng(
		symbolSvg({ mode: "light", tile: true, size: 180 }),
		180,
		path.join(directory, "apple-touch-icon-180.png"),
	);

	const faviconPngs = [];
	for (const size of [16, 32, 48]) {
		const filename = path.join(directory, `.favicon-${size}.png`);
		await renderPng(
			symbolSvg({ mode: "light", tile: true, size }),
			size,
			filename,
		);
		faviconPngs.push(filename);
	}
	await writeIco(faviconPngs, path.join(directory, "favicon.ico"));
	await Promise.all(faviconPngs.map((filename) => fs.unlink(filename)));

	await writeText(path.join(directory, "favicon.svg"), adaptiveFaviconSvg());
	await writeText(
		path.join(directory, "service-monochrome.svg"),
		symbolSvg({ mode: "light", monochrome: true }),
	);
	await writeText(
		path.join(directory, "manifest-icons.json"),
		JSON.stringify(
			{
				icons: [
					{
						src: "icon-192.png",
						sizes: "192x192",
						type: "image/png",
						purpose: "any",
					},
					{
						src: "icon-512.png",
						sizes: "512x512",
						type: "image/png",
						purpose: "any",
					},
					{
						src: "maskable-192.png",
						sizes: "192x192",
						type: "image/png",
						purpose: "maskable",
					},
					{
						src: "maskable-512.png",
						sizes: "512x512",
						type: "image/png",
						purpose: "maskable",
					},
					{
						src: "service-monochrome.svg",
						sizes: "any",
						type: "image/svg+xml",
						purpose: "monochrome",
					},
				],
			},
			null,
			2,
		),
	);
}

function svgDataUri(svg) {
	return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
}

async function generateContactSheet(stage, lightLockup, darkLockup) {
	const lightSymbol = symbolSvg({ mode: "light", tile: true });
	const maskable = symbolSvg({ mode: "light", tile: true, maskable: true });
	const monoDark = symbolSvg({ mode: "light", monochrome: true });
	const monoLight = symbolSvg({ mode: "dark", monochrome: true });
	const sheet = `<svg xmlns="http://www.w3.org/2000/svg" width="2400" height="1600" viewBox="0 0 2400 1600">
    <rect width="2400" height="1600" fill="${BRAND.colors.white}"/>
    <style>text{font-family:Arial,sans-serif;fill:${BRAND.colors.graphite}}.label{font-size:28px;font-weight:700;letter-spacing:3px}.small{font-size:24px}</style>
    <rect x="96" y="84" width="48" height="8" rx="4" fill="${BRAND.colors.green}"/>
    <text x="166" y="103" class="label">MOTE ASSET CONTACT SHEET</text>
    <text x="96" y="166" class="small">Light, dark, application, maskable and monochrome assets</text>
    <rect x="96" y="220" width="1060" height="360" rx="28" fill="#FFFFFF" stroke="${BRAND.colors.grey}"/>
    <text x="140" y="274" class="label">LIGHT</text>
    <image x="140" y="310" width="950" height="220" preserveAspectRatio="xMinYMid meet" href="${svgDataUri(lightLockup)}"/>
    <rect x="1244" y="220" width="1060" height="360" rx="28" fill="${BRAND.colors.graphite}"/>
    <text x="1288" y="274" class="label" fill="${BRAND.colors.white}" style="fill:${BRAND.colors.white}">DARK</text>
    <image x="1288" y="310" width="950" height="220" preserveAspectRatio="xMinYMid meet" href="${svgDataUri(darkLockup)}"/>
    <text x="96" y="660" class="label">APPLICATION AND SERVICE ICONS</text>
    <rect x="96" y="704" width="500" height="500" rx="34" fill="#FFFFFF" stroke="${BRAND.colors.grey}"/>
    <image x="146" y="754" width="400" height="400" href="${svgDataUri(lightSymbol)}"/>
    <text x="96" y="1248" class="small">Desktop / standard PWA</text>
    <rect x="682" y="704" width="500" height="500" rx="34" fill="#FFFFFF" stroke="${BRAND.colors.grey}"/>
    <image x="732" y="754" width="400" height="400" href="${svgDataUri(maskable)}"/>
    <text x="682" y="1248" class="small">Maskable PWA safe zone</text>
    <rect x="1268" y="704" width="500" height="500" rx="34" fill="#FFFFFF" stroke="${BRAND.colors.grey}"/>
    <image x="1318" y="754" width="400" height="400" href="${svgDataUri(monoDark)}"/>
    <text x="1268" y="1248" class="small">Monochrome / light surface</text>
    <rect x="1854" y="704" width="450" height="500" rx="34" fill="${BRAND.colors.graphite}"/>
    <image x="1879" y="754" width="400" height="400" href="${svgDataUri(monoLight)}"/>
    <text x="1854" y="1248" class="small">Monochrome / dark surface</text>
    <text x="96" y="1340" class="label">APPROVED PALETTE</text>
    ${[
			[BRAND.colors.green, "#45A06B"],
			[BRAND.colors.graphite, "#171A1F"],
			[BRAND.colors.white, "#F7F8FA"],
			[BRAND.colors.grey, "#B9C1C9"],
		]
			.map(
				([colour, label], index) =>
					`<rect x="${96 + index * 576}" y="1380" width="510" height="92" rx="20" fill="${colour}" stroke="${BRAND.colors.grey}"/><text x="${116 + index * 576}" y="1530" class="small">${label}</text>`,
			)
			.join("")}
  </svg>`;
	await fs.mkdir(path.join(stage, "previews"), { recursive: true });
	await sharp(Buffer.from(sheet))
		.png({ compressionLevel: 9 })
		.withMetadata({ icc: "srgb" })
		.toFile(path.join(stage, "previews/mote-asset-contact-sheet.png"));
}

async function generateSmallSizeSheet(stage) {
	const width = 1800;
	const height = 600;
	const base =
		Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
    <rect width="${width}" height="${height}" fill="${BRAND.colors.white}"/>
    <style>text{font-family:Arial,sans-serif;fill:${BRAND.colors.graphite}}.label{font-size:28px;font-weight:700;letter-spacing:3px}.size{font-size:26px;font-weight:700}.note{font-size:20px}</style>
    <rect x="70" y="70" width="48" height="8" rx="4" fill="${BRAND.colors.green}"/>
    <text x="140" y="90" class="label">SMALL-SIZE OPTICAL CHECK</text>
    <text x="70" y="138" class="note">Nearest-neighbour enlargement with the original pixel asset below</text>
    ${[16, 24, 32, 48, 64]
			.map((size, index) => {
				const x = 70 + index * 342;
				return `<rect x="${x}" y="174" width="272" height="346" rx="26" fill="#FFFFFF" stroke="${BRAND.colors.grey}"/><text x="${x + 106}" y="490" class="size">${size}px</text>`;
			})
			.join("")}
  </svg>`);
	const composites = [];
	for (const [index, size] of [16, 24, 32, 48, 64].entries()) {
		const source = path.join(stage, `icons/windows/png/mote-${size}.png`);
		const enlarged = await sharp(source)
			.resize(200, 200, { kernel: "nearest" })
			.png()
			.toBuffer();
		composites.push({ input: enlarged, left: 106 + index * 342, top: 214 });
		composites.push({
			input: source,
			left: 206 + index * 342 - Math.floor(size / 2),
			top: 530,
		});
	}
	await sharp(base)
		.composite(composites)
		.png({ compressionLevel: 9 })
		.withMetadata({ icc: "srgb" })
		.toFile(path.join(stage, "previews/mote-small-size-check.png"));
}

async function generatePrintAndPreviews(stage) {
	const print = path.join(stage, "print");
	const previews = path.join(stage, "previews");
	await fs.mkdir(print, { recursive: true });
	await fs.mkdir(previews, { recursive: true });
	const light = await horizontalLockupSvg({ mode: "light" });
	const dark = await horizontalLockupSvg({ mode: "dark" });
	await writeText(path.join(print, "mote-lockup-light.svg"), light);
	await writeText(path.join(print, "mote-lockup-dark.svg"), dark);
	const lightPng = path.join(print, "mote-lockup-light-3000.png");
	const darkPng = path.join(print, "mote-lockup-dark-3000.png");
	await renderWidthPng(light, 3000, lightPng);
	await renderWidthPng(dark, 3000, darkPng);
	await generateContactSheet(stage, light, dark);
	await generateSmallSizeSheet(stage);

	const pdf = path.join(print, "mote-brand-sheet-a4.pdf");
	await execFileAsync(process.env.BRAND_PYTHON ?? "python3", [
		path.resolve("docs/brand/tools/generate_brand_sheet.py"),
		lightPng,
		darkPng,
		pdf,
	]);
	const fontCache = path.join(stage, ".fontconfig-cache");
	await fs.mkdir(path.join(fontCache, "fontconfig"), { recursive: true });
	const popplerEnvironment = { ...process.env, XDG_CACHE_HOME: fontCache };
	if (process.env.BRAND_FONTCONFIG_FILE) {
		popplerEnvironment.FONTCONFIG_FILE = process.env.BRAND_FONTCONFIG_FILE;
	}
	await execFileAsync(
		process.env.BRAND_PDFTOPPM ?? "pdftoppm",
		["-png", "-r", "150", pdf, path.join(previews, "mote-brand-sheet-a4")],
		{ env: popplerEnvironment },
	);
}

async function validateStage(stage) {
	for (const output of expectedOutputs()) {
		const relative = output.replace(/^docs\/brand\//, "");
		await fs.access(path.join(stage, relative));
	}
	const ttf = openSync(path.join(stage, "fonts/Fredoka-Variable.ttf"));
	const woff2 = openSync(path.join(stage, "fonts/Fredoka-Variable.woff2"));
	if (ttf.familyName !== woff2.familyName)
		throw new Error("WOFF2 family name changed");
	if (!woff2.variationAxes.wght || !woff2.variationAxes.wdth) {
		throw new Error("WOFF2 variation axes are incomplete");
	}
	for (const directory of ["source", "svg", "icons/linux", "icons/web"]) {
		const queue = [path.join(stage, directory)];
		while (queue.length > 0) {
			const current = queue.pop();
			for (const entry of await fs.readdir(current, { withFileTypes: true })) {
				const filename = path.join(current, entry.name);
				if (entry.isDirectory()) queue.push(filename);
				else if (entry.name.endsWith(".svg")) {
					const result = XMLValidator.validate(
						await fs.readFile(filename, "utf8"),
					);
					if (result !== true) throw new Error(`Invalid SVG: ${filename}`);
				}
			}
		}
	}
}

async function preserveManualFiles(stage, destination) {
	for (const relative of ["fonts/README.md"]) {
		const source = path.join(destination, relative);
		try {
			await fs.access(source);
			const target = path.join(stage, relative);
			await fs.mkdir(path.dirname(target), { recursive: true });
			await fs.copyFile(source, target);
		} catch (error) {
			if (error.code !== "ENOENT") throw error;
		}
	}
}

async function replaceGeneratedDirectories(stage, destination) {
	const token = `${process.pid}-${Date.now()}`;
	const swapped = [];
	try {
		for (const directory of generatedDirectories) {
			const current = path.join(destination, directory);
			const next = path.join(stage, directory);
			const backup = path.join(destination, `.${directory}.backup-${token}`);
			let hadCurrent = true;
			try {
				await fs.rename(current, backup);
			} catch (error) {
				if (error.code !== "ENOENT") throw error;
				hadCurrent = false;
			}
			await fs.rename(next, current);
			swapped.push({ current, backup, hadCurrent });
		}
	} catch (error) {
		for (const item of swapped.reverse()) {
			await fs.rm(item.current, { recursive: true, force: true });
			if (item.hadCurrent) await fs.rename(item.backup, item.current);
		}
		throw error;
	}
	await Promise.all(
		swapped
			.filter((item) => item.hadCurrent)
			.map((item) => fs.rm(item.backup, { recursive: true })),
	);
}

export async function generateAll({
	destination = PATHS.brandDir,
	keepStage = false,
} = {}) {
	const stage = await fs.mkdtemp(path.join(os.tmpdir(), "mote-brand-"));
	try {
		await exportSvgSources(stage);
		await copyFontPackage(stage);
		await Promise.all([
			generateMac(stage),
			generateWindows(stage),
			generateLinux(stage),
			generateWeb(stage),
		]);
		await generatePrintAndPreviews(stage);
		await validateStage(stage);
		await preserveManualFiles(stage, destination);
		await replaceGeneratedDirectories(stage, destination);
	} finally {
		if (!keepStage) await fs.rm(stage, { recursive: true, force: true });
	}
}

const currentFile = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === currentFile) {
	await generateAll();
}
