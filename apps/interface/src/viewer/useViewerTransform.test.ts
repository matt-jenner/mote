import { describe, expect, it } from "vitest";
import {
	reconcileViewerTransform,
	transitionViewerTransform,
} from "./useViewerTransform";
import { deriveViewerTransform, resetViewerTransform } from "./viewerTransform";

const context = {
	viewportWidth: 1200,
	viewportHeight: 800,
	imageWidth: 6000,
	imageHeight: 4000,
	naturalWidth: 1200,
	naturalHeight: 800,
};

describe("viewer transform controller transitions", () => {
	it("resets to fit when the asset revision changes", () => {
		const zoomed = transitionViewerTransform(resetViewerTransform(8), {
			type: "zoomAt",
			scale: 2,
			anchor: { x: 600, y: 400 },
			context,
		});
		const reset = transitionViewerTransform(zoomed, {
			type: "assetRevision",
			assetRevision: 9,
		});
		expect(reset).toEqual(resetViewerTransform(9));
	});

	it("keeps scale and focal point when natural preview dimensions improve", () => {
		const zoomed = transitionViewerTransform(resetViewerTransform(8), {
			type: "zoomAt",
			scale: 1.5,
			anchor: { x: 700, y: 350 },
			context,
		});
		const refined = reconcileViewerTransform(zoomed, {
			...context,
			naturalWidth: 4096,
			naturalHeight: 2731,
		});
		expect(refined.scale).toBe(zoomed.scale);
		expect(refined.focal).toEqual(zoomed.focal);
	});

	it("preserves the focal point while the drawable viewport rotates", () => {
		const zoomed = transitionViewerTransform(resetViewerTransform(8), {
			type: "zoomAt",
			scale: 1.5,
			anchor: { x: 700, y: 350 },
			context,
		});
		const rotated = reconcileViewerTransform(zoomed, {
			...context,
			viewportWidth: 800,
			viewportHeight: 1200,
		});
		expect(rotated.focal).toEqual(zoomed.focal);
	});

	it("does not enlarge a decoded derivative smaller than the fitted frame", () => {
		const state = reconcileViewerTransform(resetViewerTransform(8), {
			...context,
			naturalWidth: 1,
			naturalHeight: 1,
		});
		expect(
			deriveViewerTransform(state, {
				...context,
				naturalWidth: 1,
				naturalHeight: 1,
			}).scale,
		).toBe(1);
	});
});
