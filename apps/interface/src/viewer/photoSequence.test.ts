import { describe, expect, it } from "vitest";
import type { WallAsset } from "../services/photoService";
import {
	findViewerIndex,
	shouldLoadViewerPage,
	viewerFilmstripCapacity,
	viewerFilmstripWindow,
	viewerNeighbourIds,
} from "./photoSequence";

function asset(id: string): WallAsset {
	return {
		id,
		displayName: `${id}.jpg`,
		mediaKind: "jpeg",
		provisionalOrder: 1,
		capturedAtUtc: null,
		dateState: "provisional",
		width: 100,
		height: 100,
		representativeRgb: null,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
		rating: null,
	};
}

const items = ["a", "b", "c", "d", "e"].map(asset);

describe("photoSequence", () => {
	it("finds an asset by its stable ID", () => {
		expect(findViewerIndex(items, "b")).toBe(1);
		expect(findViewerIndex(items, "missing")).toBe(-1);
	});

	it("returns immediate and idle neighbours in wall order", () => {
		expect(viewerNeighbourIds(items, 1, 2)).toEqual({
			immediate: ["a", "c"],
			idle: ["d"],
		});
	});

	it("bounds the filmstrip window to the requested radius", () => {
		const manyItems = Array.from({ length: 100 }, (_, index) =>
			asset(String(index)),
		);

		expect(viewerFilmstripWindow(manyItems, 50, 15)).toEqual({
			start: 35,
			end: 66,
		});
	});

	it("derives an odd filmstrip capacity from the measured viewport", () => {
		expect(viewerFilmstripCapacity(390)).toBe(5);
		expect(viewerFilmstripCapacity(844)).toBe(11);
		expect(viewerFilmstripCapacity(5000)).toBe(31);
	});

	it("loads another page within five items of the loaded end", () => {
		expect(shouldLoadViewerPage(96, 100, true, false)).toBe(true);
		expect(shouldLoadViewerPage(95, 100, true, false)).toBe(true);
		expect(shouldLoadViewerPage(94, 100, true, false)).toBe(false);
		expect(shouldLoadViewerPage(96, 100, false, false)).toBe(false);
		expect(shouldLoadViewerPage(96, 100, true, true)).toBe(false);
		expect(shouldLoadViewerPage(99, 100, true, true)).toBe(false);
	});
});
