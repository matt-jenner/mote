import { describe, expect, it } from "vitest";
import {
	deriveViewerTransform,
	imagePointAtViewportPoint,
	panViewerBy,
	resetViewerTransform,
	viewerZoomStep,
	zoomViewerAt,
} from "./viewerTransform";

const landscape = {
	viewportWidth: 1200,
	viewportHeight: 800,
	imageWidth: 6000,
	imageHeight: 4000,
	naturalWidth: 4096,
	naturalHeight: 2731,
};

describe("viewer transform geometry", () => {
	it("fits the image and caps zoom at one derivative pixel per CSS pixel", () => {
		const geometry = deriveViewerTransform(resetViewerTransform(1), landscape);
		expect(geometry.fitWidth).toBeCloseTo(1200);
		expect(geometry.fitHeight).toBeCloseTo(800);
		expect(geometry.maxScale).toBeCloseTo(4096 / 1200);
		expect(geometry.mode).toBe("fit");
	});

	it("keeps a small derivative at fit instead of enlarging cached pixels", () => {
		const geometry = deriveViewerTransform(resetViewerTransform(1), {
			...landscape,
			naturalWidth: 600,
			naturalHeight: 400,
		});
		expect(geometry.maxScale).toBe(1);
	});

	it("keeps the image point under the pointer fixed while zooming", () => {
		const initial = resetViewerTransform(4);
		const anchor = { x: 900, y: 250 };
		const before = imagePointAtViewportPoint(initial, landscape, anchor);
		const zoomed = zoomViewerAt(initial, landscape, 2, anchor);
		const after = imagePointAtViewportPoint(zoomed, landscape, anchor);
		expect(after.x).toBeCloseTo(before.x, 6);
		expect(after.y).toBeCloseTo(before.y, 6);
	});

	it("clamps panning at every edge without exposing empty canvas", () => {
		const zoomed = zoomViewerAt(resetViewerTransform(4), landscape, 2, {
			x: 600,
			y: 400,
		});
		for (const [delta, expectedX, expectedY] of [
			[{ x: 10000, y: -10000 }, 600, -400],
			[{ x: -10000, y: 10000 }, -600, 400],
		] as const) {
			const moved = panViewerBy(zoomed, landscape, delta);
			const geometry = deriveViewerTransform(moved, landscape);
			expect(geometry.translateX).toBe(expectedX);
			expect(geometry.translateY).toBe(expectedY);
		}
	});

	it("preserves the normalized focal point after rotation", () => {
		const zoomed = zoomViewerAt(resetViewerTransform(4), landscape, 2, {
			x: 850,
			y: 300,
		});
		const rotated = deriveViewerTransform(zoomed, {
			...landscape,
			viewportWidth: 800,
			viewportHeight: 1200,
		});
		expect(rotated.focal).toEqual(
			deriveViewerTransform(zoomed, landscape).focal,
		);
	});

	it("derives the visible normalized rectangle for the navigator", () => {
		const zoomed = zoomViewerAt(resetViewerTransform(4), landscape, 2, {
			x: 600,
			y: 400,
		});
		const { visibleImageRect } = deriveViewerTransform(zoomed, landscape);
		expect(visibleImageRect).toMatchObject({
			x: 0.25,
			y: 0.25,
			width: 0.5,
			height: 0.5,
		});
	});

	it("steps down from the effective scale after a resize lowers the maximum", () => {
		const zoomed = zoomViewerAt(resetViewerTransform(4), landscape, 3, {
			x: 600,
			y: 400,
		});
		const resized = {
			...landscape,
			viewportWidth: 2400,
			viewportHeight: 1600,
		};
		const effective = deriveViewerTransform(zoomed, resized);
		expect(effective.scale).toBeCloseTo(4096 / 2400);
		const stepped = viewerZoomStep(zoomed, resized, -1);
		expect(deriveViewerTransform(stepped, resized).scale).toBeCloseTo(
			4096 / 2400 / 1.25,
		);
	});

	it("normalizes stale zoom state to fit when a resize removes zoom room", () => {
		const zoomed = zoomViewerAt(resetViewerTransform(4), landscape, 2, {
			x: 600,
			y: 400,
		});
		const resized = {
			...landscape,
			viewportWidth: 6000,
			viewportHeight: 4000,
		};
		expect(panViewerBy(zoomed, resized, { x: 10, y: 10 })).toEqual({
			assetRevision: 4,
			scale: 1,
			focal: { x: 0.5, y: 0.5 },
		});
	});

	it.each([
		{
			name: "reset",
			run: () => resetViewerTransform(Number.NaN),
			expectation: { assetRevision: 0, scale: 1, focal: { x: 0.5, y: 0.5 } },
		},
		{
			name: "derive",
			run: () =>
				deriveViewerTransform(resetViewerTransform(7), {
					...landscape,
					viewportWidth: Number.NaN,
				}),
			expectation: {
				mode: "fit",
				fitWidth: 0,
				fitHeight: 0,
				scale: 1,
				maxScale: 1,
				translateX: 0,
				translateY: 0,
				focal: { x: 0.5, y: 0.5 },
				visibleImageRect: { x: 0, y: 0, width: 0, height: 0 },
			},
		},
		{
			name: "point conversion",
			run: () =>
				imagePointAtViewportPoint(resetViewerTransform(7), landscape, {
					x: Number.POSITIVE_INFINITY,
					y: 400,
				}),
			expectation: { x: 0.5, y: 0.5 },
		},
		{
			name: "zoom",
			run: () =>
				zoomViewerAt(resetViewerTransform(7), landscape, Number.NaN, {
					x: 600,
					y: 400,
				}),
			expectation: { assetRevision: 7, scale: 1, focal: { x: 0.5, y: 0.5 } },
		},
		{
			name: "pan",
			run: () =>
				panViewerBy(resetViewerTransform(7), landscape, {
					x: Number.NEGATIVE_INFINITY,
					y: 0,
				}),
			expectation: { assetRevision: 7, scale: 1, focal: { x: 0.5, y: 0.5 } },
		},
		{
			name: "zoom step",
			run: () =>
				viewerZoomStep(
					resetViewerTransform(7),
					{
						...landscape,
						viewportHeight: Number.POSITIVE_INFINITY,
					},
					1,
				),
			expectation: { assetRevision: 7, scale: 1, focal: { x: 0.5, y: 0.5 } },
		},
	])(
		"returns a fit-safe result for non-finite $name input",
		({ run, expectation }) => {
			expect(run()).toMatchObject(expectation);
		},
	);
});
