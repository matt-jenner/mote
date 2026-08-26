import { describe, expect, it } from "vitest";
import type { WallAsset } from "../services/photoService";
import { buildViewerPreviewPlan } from "./viewerPreviewPlan";

function asset(id: string): WallAsset {
	return {
		id,
		displayName: id,
		mediaKind: "jpeg",
		provisionalOrder: 0,
		capturedAtUtc: null,
		dateState: "provisional",
		width: 1200,
		height: 800,
		representativeRgb: null,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
		rating: null,
	};
}

const fiveAssets = ["a", "b", "c", "d", "e"].map(asset);

describe("buildViewerPreviewPlan", () => {
	it("requests the current preview visibly and immediate neighbours near the viewport", () => {
		expect(buildViewerPreviewPlan(fiveAssets, 2)).toEqual([
			{
				assetIds: ["c"],
				priority: "visible",
				kind: "screenPreview",
			},
			{
				assetIds: ["b", "d"],
				priority: "nearViewport",
				kind: "screenPreview",
			},
			{
				assetIds: ["a", "e"],
				priority: "nearViewport",
				kind: "screenPreview",
				idle: true,
			},
		]);
	});

	it("bounds an invalid current index without inventing asset IDs", () => {
		expect(buildViewerPreviewPlan(fiveAssets, -1)).toEqual([]);
		expect(buildViewerPreviewPlan([], 0)).toEqual([]);
	});
});
