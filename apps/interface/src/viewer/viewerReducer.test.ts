import { describe, expect, it } from "vitest";

import {
	initialViewerState,
	type ViewerState,
	viewerReducer,
} from "./viewerReducer";

describe("viewerReducer", () => {
	it("opens at the selected asset and increments the preview generation on navigation", () => {
		const opened = viewerReducer(initialViewerState, {
			type: "open",
			assetId: "b",
			anchor: { assetId: "b", scrollTop: 640 },
		});
		const moved = viewerReducer(opened, { type: "select", assetId: "c" });

		expect(opened).toMatchObject({
			open: true,
			currentAssetId: "b",
			returnAnchor: { assetId: "b", scrollTop: 640 },
		});
		expect(opened.previewGeneration).toBe(1);
		expect(moved.currentAssetId).toBe("c");
		expect(moved.previewGeneration).toBe(opened.previewGeneration + 1);
	});

	it("keeps information open across navigation and resets it after close", () => {
		const open = viewerReducer(initialViewerState, {
			type: "open",
			assetId: "a",
			anchor: { assetId: "a", scrollTop: 20 },
		});
		const info = viewerReducer(open, { type: "setInfoOpen", open: true });

		expect(viewerReducer(info, { type: "select", assetId: "b" }).infoOpen).toBe(
			true,
		);
		const closed = viewerReducer(info, { type: "close" });
		expect(closed.infoOpen).toBe(false);
		expect(closed).toEqual({
			...initialViewerState,
			previewGeneration: info.previewGeneration + 1,
		});
	});

	it("advances the preview generation when opening, selecting, closing, and reopening", () => {
		const opened = viewerReducer(initialViewerState, {
			type: "open",
			assetId: "a",
			anchor: { assetId: "a", scrollTop: 20 },
		});
		const selected = viewerReducer(opened, { type: "select", assetId: "b" });
		const closed = viewerReducer(selected, { type: "close" });
		const reopened = viewerReducer(closed, {
			type: "open",
			assetId: "c",
			anchor: { assetId: "c", scrollTop: 640 },
		});

		expect(opened.previewGeneration).toBe(1);
		expect(selected.previewGeneration).toBe(2);
		expect(closed.previewGeneration).toBe(3);
		expect(reopened.previewGeneration).toBe(4);
	});

	it("shows and hides primary controls without changing the filmstrip", () => {
		const hidden = viewerReducer(initialViewerState, { type: "hideControls" });
		const shown = viewerReducer(hidden, { type: "showControls" });

		expect(hidden).toMatchObject({
			controlsVisible: false,
			filmstripVisible: true,
		});
		expect(shown).toMatchObject({
			controlsVisible: true,
			filmstripVisible: true,
		});
	});

	it("toggles touch controls together", () => {
		const hidden = viewerReducer(initialViewerState, {
			type: "toggleTouchControls",
		});
		const shown = viewerReducer(hidden, { type: "toggleTouchControls" });

		expect(hidden).toMatchObject({
			controlsVisible: false,
			filmstripVisible: false,
		});
		expect(shown).toMatchObject({
			controlsVisible: true,
			filmstripVisible: true,
		});
	});

	it("repairs a mixed visibility state when touch controls are toggled", () => {
		const mixed = viewerReducer(initialViewerState, { type: "hideControls" });
		const paired = viewerReducer(mixed, { type: "toggleTouchControls" });

		expect(mixed).toMatchObject({
			controlsVisible: false,
			filmstripVisible: true,
		});
		expect(paired).toMatchObject({
			controlsVisible: true,
			filmstripVisible: true,
		});
	});

	it("select changes only the current asset and preview generation", () => {
		const state: ViewerState = {
			...initialViewerState,
			open: true,
			currentAssetId: "a",
			returnAnchor: { assetId: "a", scrollTop: 12 },
			infoOpen: true,
			controlsVisible: false,
			filmstripVisible: false,
			previewGeneration: 3,
		};

		expect(viewerReducer(state, { type: "select", assetId: "b" })).toEqual({
			...state,
			currentAssetId: "b",
			previewGeneration: 4,
		});
	});
});
