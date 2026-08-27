import { describe, expect, it } from "vitest";
import {
	deriveViewerTransform,
	imagePointAtViewportPoint,
	panViewerBy,
	resetViewerTransform,
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
		const moved = panViewerBy(zoomed, landscape, { x: 10000, y: -10000 });
		const geometry = deriveViewerTransform(moved, landscape);
		expect(geometry.translateX).toBe(
			(geometry.fitWidth * geometry.scale - 1200) / 2,
		);
		expect(geometry.translateY).toBe(
			-(geometry.fitHeight * geometry.scale - 800) / 2,
		);
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
});
