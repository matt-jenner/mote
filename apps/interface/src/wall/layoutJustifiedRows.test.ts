import { describe, expect, it } from "vitest";
import type { WallAsset } from "../services/photoService";
import {
	type JustifiedLayoutOptions,
	LAYOUT_GEOMETRY_TOLERANCE,
	layoutJustifiedRows,
} from "./layoutJustifiedRows";

function assets(aspectRatios: readonly number[]): WallAsset[] {
	return aspectRatios.map((aspectRatio, index) => ({
		id: `asset-${index + 1}`,
		displayName: `asset-${index + 1}.jpg`,
		mediaKind: "jpeg",
		provisionalOrder: index + 1,
		capturedAtUtc: null,
		dateState: "provisional",
		width: aspectRatio * 100,
		height: 100,
		representativeRgb: null,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
		rating: null,
	}));
}

function options(
	overrides: Partial<JustifiedLayoutOptions> = {},
): JustifiedLayoutOptions {
	return {
		containerWidth: 1000,
		targetRowHeight: 220,
		gap: 4,
		layoutComplete: false,
		...overrides,
	};
}

describe("layoutJustifiedRows", () => {
	it("fills complete rows without changing source aspect ratios", () => {
		const rows = layoutJustifiedRows(assets([1.5, 1, 2, 0.75]), {
			containerWidth: 1000,
			targetRowHeight: 220,
			gap: 4,
			layoutComplete: false,
		});

		expect(rows).toHaveLength(1);
		expect(rows[0]?.width).toBeCloseTo(1000, 5);
		for (const item of rows[0]?.items ?? []) {
			expect(item.width / item.height).toBeCloseTo(
				item.asset.width / item.asset.height,
				5,
			);
		}
	});

	it("holds an incomplete row until the source completes", () => {
		expect(
			layoutJustifiedRows(assets([1]), options({ layoutComplete: false })),
		).toEqual([]);
		expect(
			layoutJustifiedRows(assets([1]), options({ layoutComplete: true }))[0]
				?.justified,
		).toBe(false);
	});

	it("accounts for gaps and leaves tile geometry unchanged when a derivative arrives", () => {
		const source = assets([1.5, 0.75, 2]);
		const before = layoutJustifiedRows(
			source,
			options({ containerWidth: 900 }),
		);
		const refined = source.map((asset, index) =>
			index === 0
				? {
						...asset,
						wallThumbnail: {
							assetId: asset.id,
							kind: "wallThumbnail" as const,
							key: "ready",
						},
					}
				: asset,
		);
		const after = layoutJustifiedRows(
			refined,
			options({ containerWidth: 900 }),
		);

		expect(
			after.map((row) => row.items.map(({ width, height }) => [width, height])),
		).toEqual(
			before.map((row) =>
				row.items.map(({ width, height }) => [width, height]),
			),
		);
		expect(
			before[0]?.items.reduce((sum, item) => sum + item.width, 0),
		).toBeCloseTo(900 - 8, 5);
	});

	it("rejects non-positive source dimensions", () => {
		const source = assets([1])[0];
		if (!source) throw new Error("expected a fixture asset");
		expect(() =>
			layoutJustifiedRows([{ ...source, width: 0 }], options()),
		).toThrow("Wall asset dimensions must be positive");
	});

	it("rejects non-finite geometry options", () => {
		expect(() =>
			layoutJustifiedRows(assets([1]), options({ containerWidth: Number.NaN })),
		).toThrow("Wall layout geometry must be finite and positive");
		expect(() =>
			layoutJustifiedRows(assets([1]), options({ gap: -1 })),
		).toThrow("Wall layout gap must be non-negative");
	});

	it("leaves a completed final row left-aligned at the target height", () => {
		const rows = layoutJustifiedRows(
			assets([1, 2]),
			options({ layoutComplete: true }),
		);

		expect(rows).toHaveLength(1);
		expect(rows[0]?.justified).toBe(false);
		expect(rows[0]?.height).toBe(220);
		expect(rows[0]?.items[0]?.left).toBe(0);
		expect(rows[0]?.items[1]?.left).toBeGreaterThan(220);
		expect(rows[0]?.width).toBeCloseTo(220 * 3 + 4, 5);
	});

	it("keeps ultra-narrow tiles wide enough for controls without overflowing", () => {
		const single = layoutJustifiedRows(assets([0.25]), {
			containerWidth: 366,
			targetRowHeight: 150,
			gap: 4,
			layoutComplete: true,
		});
		expect(single[0]?.items[0]?.width).toBeGreaterThanOrEqual(44);
		expect(single[0]?.width).toBeLessThanOrEqual(366);

		const rows = layoutJustifiedRows(assets(Array(10).fill(0.25)), {
			containerWidth: 320,
			targetRowHeight: 150,
			gap: 4,
			layoutComplete: true,
		});
		expect(rows.length).toBeGreaterThan(1);
		for (const row of rows) {
			expect(row.items.every((item) => item.width >= 44)).toBe(true);
			expect(row.width).toBeLessThanOrEqual(320);
			const last = row.items.at(-1);
			expect((last?.left ?? 0) + (last?.width ?? 0)).toBeLessThanOrEqual(320);
		}
	});

	it("keeps the final complete tile on the edge while clamping narrow items", () => {
		const fixture = assets([1])[0];
		if (!fixture) throw new Error("expected a fixture asset");
		const dimensions: readonly (readonly [number, number])[] = [
			[4461, 4210],
			[1394, 2162],
			[2855, 1077],
			[451, 4113],
			[3049, 780],
		];
		const source = dimensions.map(([width, height], index) => ({
			...fixture,
			id: `review-${index}`,
			width,
			height,
		}));
		const row = layoutJustifiedRows(source, options())[0];
		if (!row) throw new Error("expected a justified row");
		const last = row.items.at(-1);
		if (!last) throw new Error("expected the final tile");

		expect(row.justified).toBe(true);
		expect(last.left + last.width).toBe(1000);
		expect(row.items.every((item) => item.width >= 44)).toBe(true);
	});

	it("rejects an aspect ratio that underflows to zero", () => {
		const source = assets([1])[0];
		if (!source) throw new Error("expected a fixture asset");
		expect(() =>
			layoutJustifiedRows(
				[{ ...source, width: Number.MIN_VALUE, height: 2 }],
				options(),
			),
		).toThrow("Wall asset dimensions must be positive");
	});

	it("uses the documented tolerance for fractional container geometry", () => {
		const fixture = assets([1])[0];
		if (!fixture) throw new Error("expected a fixture asset");
		const dimensions: readonly (readonly [number, number])[] = [
			[3713, 4000],
			[1396, 1080],
			[1153, 630],
			[988, 1741],
			[766, 1649],
			[3223, 413],
		];
		const source = dimensions.map(([width, height], index) => ({
			...fixture,
			id: `fractional-${index}`,
			width,
			height,
		}));
		const row = layoutJustifiedRows(source, {
			containerWidth: 1000.1,
			targetRowHeight: 107.18,
			gap: 4.1,
			layoutComplete: true,
		})[0];
		if (!row) throw new Error("expected a justified row");
		const last = row.items.at(-1);
		if (!last) throw new Error("expected the final tile");

		expect(Math.abs(last.left + last.width - 1000.1)).toBeLessThanOrEqual(
			LAYOUT_GEOMETRY_TOLERANCE,
		);
		expect(row.items.every((item) => item.width >= 44)).toBe(true);
		for (let index = 1; index < row.items.length; index += 1) {
			const previous = row.items[index - 1];
			const current = row.items[index];
			if (!previous || !current) throw new Error("expected adjacent tiles");
			expect(
				Math.abs(current.left - previous.left - previous.width - 4.1),
			).toBeLessThanOrEqual(LAYOUT_GEOMETRY_TOLERANCE);
		}
	});
});
