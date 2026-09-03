import assert from "node:assert/strict";
import test from "node:test";
import sharp from "sharp";
import { horizontalLockupSvg, symbolSvg, wordmarkSvg } from "./svg.mjs";

function layerRect(svg, layer) {
	const match = svg.match(
		new RegExp(`<(?:rect|path) data-layer="${layer}"[^>]+>`),
	);
	assert.ok(match, `${layer} rectangle is present`);
	const tag = match[0];
	const numberAttribute = (name, fallback) => {
		const attribute = tag.match(new RegExp(`(?:data-)?${name}="([0-9.]+)"`));
		if (!attribute) return fallback;
		return Number(attribute[1]);
	};
	return {
		x: numberAttribute("x"),
		y: numberAttribute("y"),
		width: numberAttribute("width"),
		height: numberAttribute("height"),
		stroke: numberAttribute("stroke-width", 0),
	};
}

async function horizontalArtworkMetrics(svg) {
	const { data, info } = await sharp(Buffer.from(svg))
		.ensureAlpha()
		.raw()
		.toBuffer({ resolveWithObject: true });
	const activeColumns = [];
	for (let x = 0; x < info.width; x += 1) {
		for (let y = 0; y < info.height; y += 1) {
			if (data[(y * info.width + x) * info.channels + 3] > 8) {
				activeColumns.push(x);
				break;
			}
		}
	}
	const gaps = [];
	for (let index = 1; index < activeColumns.length; index += 1) {
		const width = activeColumns[index] - activeColumns[index - 1] - 1;
		if (width > 0) {
			gaps.push({
				left: activeColumns[index - 1],
				right: activeColumns[index],
				width,
			});
		}
	}
	const separation = gaps.sort((a, b) => b.width - a.width)[0];
	assert.ok(separation, "symbol and wordmark have a visible separation");

	function bounds(fromX, toX) {
		let minY = info.height;
		let maxY = -1;
		for (let y = 0; y < info.height; y += 1) {
			for (let x = fromX; x <= toX; x += 1) {
				if (data[(y * info.width + x) * info.channels + 3] > 8) {
					minY = Math.min(minY, y);
					maxY = Math.max(maxY, y);
				}
			}
		}
		return { centreY: (minY + maxY) / 2 };
	}

	return {
		gap: separation.width,
		symbol: bounds(0, separation.left),
		wordmark: bounds(separation.right, info.width - 1),
	};
}

test("full-colour symbol contains two frames and the centre square", () => {
	const svg = symbolSvg({ mode: "light", tile: false, size: 1024 });
	const frameLayers = [
		...svg.matchAll(/data-layer="(rear|middle|front)"/g),
	].map(([, layer]) => layer);
	assert.deepEqual(frameLayers, ["rear", "front"]);
	assert.match(svg, /data-layer="centre"/);
	assert.match(svg, /#45A06B/i);
	assert.match(svg, /#171A1F/i);
});

test("two-frame artwork is centred on the symbol canvas", () => {
	const svg = symbolSvg({ mode: "light", tile: false, size: 1024 });
	const frames = ["rear", "front"].map((layer) => layerRect(svg, layer));
	const left = Math.min(...frames.map(({ x, stroke }) => x - stroke / 2));
	const top = Math.min(...frames.map(({ y, stroke }) => y - stroke / 2));
	const right = Math.max(
		...frames.map(({ x, width, stroke }) => x + width + stroke / 2),
	);
	const bottom = Math.max(
		...frames.map(({ y, height, stroke }) => y + height + stroke / 2),
	);
	assert.equal((left + right) / 2, 512);
	assert.equal((top + bottom) / 2, 512);
});

test("both photo frames are equal squares", () => {
	const svg = symbolSvg({ mode: "light", tile: false, size: 1024 });
	const rear = layerRect(svg, "rear");
	const front = layerRect(svg, "front");
	assert.equal(rear.width, rear.height);
	assert.equal(front.width, front.height);
	assert.equal(rear.width, front.width);
	assert.equal(rear.stroke, front.stroke);
});

test("centre dot stays clear of both frame strokes at every optical size", () => {
	for (const size of [16, 1024]) {
		const svg = symbolSvg({ mode: "light", tile: false, size });
		const dot = layerRect(svg, "centre");
		for (const layer of ["rear", "front"]) {
			const frame = layerRect(svg, layer);
			const inset = frame.stroke / 2;
			assert.ok(dot.x > frame.x + inset, `${size}px dot clears ${layer} left`);
			assert.ok(dot.y > frame.y + inset, `${size}px dot clears ${layer} top`);
			assert.ok(
				dot.x + dot.width < frame.x + frame.width - inset,
				`${size}px dot clears ${layer} right`,
			);
			assert.ok(
				dot.y + dot.height < frame.y + frame.height - inset,
				`${size}px dot clears ${layer} bottom`,
			);
		}
	}
});

test("approved symbol opens opposing exterior frame corners", async () => {
	const symbol = symbolSvg({
		mode: "light",
		tile: false,
		size: 1024,
	});
	assert.match(
		symbol,
		/<path data-layer="rear" data-crop-corner="top-right"[^>]+stroke-linecap="round"/,
	);
	assert.match(
		symbol,
		/<path data-layer="front" data-crop-corner="bottom-left"[^>]+stroke-linecap="round"/,
	);
	assert.doesNotMatch(symbol, /<rect data-layer="(?:rear|front)"/);
	const openPaths = [
		...symbol.matchAll(/data-crop-corner="[^"]+"[^>]+d="([^"]+)"/g),
	];
	assert.equal(openPaths.length, 2);
	for (const [, pathData] of openPaths) assert.doesNotMatch(pathData, /\bZ\b/i);

	const lockup = await horizontalLockupSvg({ mode: "dark" });
	assert.match(lockup, /data-crop-corner="top-right"/);
	assert.match(lockup, /data-crop-corner="bottom-left"/);
});

test("dark symbol reverses the rear frame and retains Mote green", () => {
	const svg = symbolSvg({ mode: "dark", tile: false, size: 1024 });
	assert.match(svg, /data-layer="rear"[^>]+stroke="#F7F8FA"/i);
	assert.match(svg, /data-layer="front"[^>]+stroke="#45A06B"/i);
});

test("full-colour tile keeps both frames visible against graphite", () => {
	const svg = symbolSvg({ mode: "light", tile: true, size: 1024 });
	assert.match(svg, /data-layer="tile"[^>]+fill="#171A1F"/i);
	assert.match(svg, /data-layer="rear"[^>]+stroke="#F7F8FA"/i);
	assert.match(svg, /data-layer="front"[^>]+stroke="#45A06B"/i);
});

test("maskable app icon keeps essential artwork inside the safe zone", () => {
	const svg = symbolSvg({
		mode: "light",
		tile: true,
		maskable: true,
		size: 1024,
	});
	assert.match(svg, /data-safe-zone="205 205 614 614"/);
	assert.match(svg, /data-artwork-bounds="205 205 614 614"/);
});

test("portable wordmarks and lockups use outlines instead of live text", async () => {
	const wordmark = await wordmarkSvg({ mode: "light" });
	const lockup = await horizontalLockupSvg({ mode: "dark" });
	for (const svg of [wordmark, lockup]) {
		assert.match(svg, /<path\b/);
		assert.doesNotMatch(svg, /<text\b/i);
	}
});

test("horizontal lockup uses the approved 88-unit visible gap", async () => {
	const metrics = await horizontalArtworkMetrics(
		await horizontalLockupSvg({ mode: "light" }),
	);
	assert.ok(metrics.gap >= 86 && metrics.gap <= 89, metrics);
});

test("horizontal lockup optically aligns the symbol and wordmark centres", async () => {
	const metrics = await horizontalArtworkMetrics(
		await horizontalLockupSvg({ mode: "light" }),
	);
	assert.ok(
		Math.abs(metrics.symbol.centreY - metrics.wordmark.centreY) <= 1,
		metrics,
	);
});

test("monochrome symbol keeps all layers in one requested colour", () => {
	const svg = symbolSvg({
		mode: "light",
		tile: false,
		monochrome: true,
		size: 1024,
	});
	const fillsAndStrokes = [
		...svg.matchAll(/(?:fill|stroke)="(#[0-9A-F]{6})"/gi),
	].map(([, colour]) => colour.toUpperCase());
	assert.equal(fillsAndStrokes.length, 3);
	assert.deepEqual(new Set(fillsAndStrokes), new Set(["#171A1F"]));
});
