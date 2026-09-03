import fs from "node:fs/promises";
import path from "node:path";
import { openSync } from "fontkit";
import { BRAND, PATHS } from "./config.mjs";

const XMLNS = "http://www.w3.org/2000/svg";
const TRACKING = -35;

function escapeXml(value) {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll('"', "&quot;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;");
}

function frame({
	layer,
	x,
	y,
	width,
	height,
	radius,
	colour,
	stroke,
	cropCorner,
	cropCut,
}) {
	if (cropCorner) {
		const right = x + width;
		const bottom = y + height;
		const pathData =
			cropCorner === "top-right"
				? `M ${right} ${y + cropCut} V ${bottom - radius} Q ${right} ${bottom} ${right - radius} ${bottom} H ${x + radius} Q ${x} ${bottom} ${x} ${bottom - radius} V ${y + radius} Q ${x} ${y} ${x + radius} ${y} H ${right - cropCut}`
				: `M ${x} ${bottom - cropCut} V ${y + radius} Q ${x} ${y} ${x + radius} ${y} H ${right - radius} Q ${right} ${y} ${right} ${y + radius} V ${bottom - radius} Q ${right} ${bottom} ${right - radius} ${bottom} H ${x + cropCut}`;
		return `<path data-layer="${layer}" data-crop-corner="${cropCorner}" data-x="${x}" data-y="${y}" data-width="${width}" data-height="${height}" d="${pathData}" fill="none" stroke="${colour}" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round"/>`;
	}
	return `<rect data-layer="${layer}" x="${x}" y="${y}" width="${width}" height="${height}" rx="${radius}" fill="none" stroke="${colour}" stroke-width="${stroke}" stroke-linejoin="round"/>`;
}

function palette(mode, monochrome) {
	if (monochrome) {
		const colour = mode === "dark" ? BRAND.colors.white : BRAND.colors.graphite;
		return { rear: colour, front: colour, centre: colour };
	}
	return {
		rear: mode === "dark" ? BRAND.colors.white : BRAND.colors.graphite,
		front: BRAND.colors.green,
		centre: BRAND.colors.green,
	};
}

function symbolArtwork({ mode, monochrome = false, small = false }) {
	const colours = palette(mode, monochrome);
	const stroke = small ? 112 : 96;
	const centre = small ? 108 : 84;
	const cropCut = small ? 144 : 128;
	return [
		frame({
			layer: "rear",
			x: 368,
			y: 240,
			width: 416,
			height: 416,
			radius: 42,
			colour: colours.rear,
			stroke,
			cropCorner: "top-right",
			cropCut,
		}),
		frame({
			layer: "front",
			x: 240,
			y: 368,
			width: 416,
			height: 416,
			radius: 42,
			colour: colours.front,
			stroke,
			cropCorner: "bottom-left",
			cropCut,
		}),
		`<rect data-layer="centre" x="${512 - centre / 2}" y="${512 - centre / 2}" width="${centre}" height="${centre}" rx="12" fill="${colours.centre}"/>`,
	].join("");
}

function tileNode(mode) {
	const fill = mode === "dark" ? BRAND.colors.darkTile : BRAND.colors.graphite;
	return `<rect data-layer="tile" x="48" y="48" width="928" height="928" rx="224" fill="${fill}"/>`;
}

export function symbolSvg({
	mode = "light",
	tile = false,
	maskable = false,
	monochrome = false,
	size = 1024,
} = {}) {
	const small = size <= 24;
	const scale = maskable ? 0.72 : tile ? 0.82 : 0.9;
	const offset = (1024 - 1024 * scale) / 2;
	const tileMarkup = tile ? tileNode(mode) : "";
	const safeZone = maskable
		? '<rect data-safe-zone="205 205 614 614" x="205" y="205" width="614" height="614" fill="none"/>'
		: "";
	const bounds = maskable ? ' data-artwork-bounds="205 205 614 614"' : "";
	const artworkMode = tile && !monochrome ? "dark" : mode;
	return `<svg xmlns="${XMLNS}" viewBox="0 0 1024 1024" width="${size}" height="${size}" role="img" aria-label="Mote symbol">
  ${tileMarkup}
  <g${bounds} transform="translate(${offset} ${offset}) scale(${scale})">${symbolArtwork({ mode: artworkMode, monochrome, small })}</g>
  ${safeZone}
</svg>`;
}

let fredoka;

function fredokaFont() {
	if (!fredoka) {
		const base = openSync(PATHS.sourceFont);
		fredoka = base.getVariation({
			wght: BRAND.wordmark.weight,
			wdth: BRAND.wordmark.width,
		});
	}
	return fredoka;
}

function outlinedRun(text) {
	const font = fredokaFont();
	const run = font.layout(text);
	let cursor = 0;
	const paths = run.glyphs.map((glyph, index) => {
		const pathData = glyph.path.toSVG();
		const transformed = `<path d="${pathData}" transform="translate(${cursor + run.positions[index].xOffset} ${-run.positions[index].yOffset}) scale(1 -1)"/>`;
		cursor += run.positions[index].xAdvance;
		if (index < run.glyphs.length - 1) cursor += TRACKING;
		return transformed;
	});
	return {
		ascent: font.ascent,
		descent: font.descent,
		paths: paths.join(""),
		width: cursor,
	};
}

function wordmarkGroup({ mode, x = 0, baseline = 0, scale = 1 }) {
	const run = outlinedRun(BRAND.name);
	const fill = mode === "dark" ? BRAND.colors.white : BRAND.colors.graphite;
	return {
		height: (run.ascent - run.descent) * scale,
		markup: `<g data-layer="wordmark" fill="${fill}" transform="translate(${x} ${baseline}) scale(${scale})">${run.paths}</g>`,
		width: run.width * scale,
	};
}

export async function wordmarkSvg({ mode = "light" } = {}) {
	const wordmark = wordmarkGroup({ mode, baseline: 974 });
	return `<svg xmlns="${XMLNS}" viewBox="0 0 ${wordmark.width} 1210" role="img" aria-label="${escapeXml(BRAND.name)} wordmark">
  ${wordmark.markup}
</svg>`;
}

export async function horizontalLockupSvg({ mode = "light" } = {}) {
	const symbolSize = 620;
	const symbolY = 46;
	const wordmarkX = 561;
	const wordmark = wordmarkGroup({
		mode,
		x: wordmarkX,
		baseline: 511,
		scale: 0.46,
	});
	const width = wordmarkX + wordmark.width + 48;
	const artwork = symbolArtwork({ mode, monochrome: false, small: false });
	return `<svg xmlns="${XMLNS}" viewBox="0 0 ${width} ${symbolSize}" role="img" aria-label="${escapeXml(BRAND.name)}">
  <g data-layer="symbol" transform="translate(0 ${symbolY}) scale(${symbolSize / 1024})">${artwork}</g>
  ${wordmark.markup}
</svg>`;
}

export async function stackedLockupSvg({ mode = "light" } = {}) {
	const symbolSize = 600;
	const gap = 96;
	const wordmark = wordmarkGroup({ mode, baseline: 980, scale: 0.42 });
	const width = Math.max(symbolSize, wordmark.width + 96);
	const symbolX = (width - symbolSize) / 2;
	const wordmarkX = (width - wordmark.width) / 2;
	const artwork = symbolArtwork({ mode, monochrome: false, small: false });
	return `<svg xmlns="${XMLNS}" viewBox="0 0 ${width} 1090" role="img" aria-label="${escapeXml(BRAND.name)}">
  <g data-layer="symbol" transform="translate(${symbolX} 0) scale(${symbolSize / 1024})">${artwork}</g>
  ${wordmark.markup.replace("translate(0 980)", `translate(${wordmarkX} ${symbolSize + gap + 284})`)}
</svg>`;
}

async function writeSvg(root, relative, markup) {
	const filename = path.join(root, relative);
	await fs.mkdir(path.dirname(filename), { recursive: true });
	await fs.writeFile(filename, `${markup.trim()}\n`, "utf8");
}

export async function exportSvgSources(root) {
	const files = [
		["source/mote-symbol-master.svg", symbolSvg({ mode: "light" })],
		["source/mote-wordmark-outlined.svg", await wordmarkSvg({ mode: "light" })],
		[
			"source/mote-lockup-master.svg",
			await horizontalLockupSvg({ mode: "light" }),
		],
		["svg/mote-symbol-light.svg", symbolSvg({ mode: "light" })],
		["svg/mote-symbol-dark.svg", symbolSvg({ mode: "dark" })],
		[
			"svg/mote-symbol-monochrome-dark.svg",
			symbolSvg({ mode: "light", monochrome: true }),
		],
		[
			"svg/mote-symbol-monochrome-light.svg",
			symbolSvg({ mode: "dark", monochrome: true }),
		],
		["svg/mote-wordmark-dark.svg", await wordmarkSvg({ mode: "light" })],
		["svg/mote-wordmark-light.svg", await wordmarkSvg({ mode: "dark" })],
		[
			"svg/mote-lockup-horizontal-light.svg",
			await horizontalLockupSvg({ mode: "light" }),
		],
		[
			"svg/mote-lockup-horizontal-dark.svg",
			await horizontalLockupSvg({ mode: "dark" }),
		],
		[
			"svg/mote-lockup-stacked-light.svg",
			await stackedLockupSvg({ mode: "light" }),
		],
		[
			"svg/mote-lockup-stacked-dark.svg",
			await stackedLockupSvg({ mode: "dark" }),
		],
	];
	await Promise.all(
		files.map(([relative, markup]) => writeSvg(root, relative, markup)),
	);
}
