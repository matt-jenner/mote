import { describe, expect, it } from "vitest";
import { classifyViewerGesture } from "./useViewerGestures";

describe("classifyViewerGesture", () => {
	it("classifies horizontal fit swipes as navigation", () => {
		expect(
			classifyViewerGesture({
				dx: -72,
				dy: 12,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("next");
		expect(
			classifyViewerGesture({
				dx: 72,
				dy: 12,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("previous");
	});

	it("preserves dominant vertical drawer movement for scrolling", () => {
		expect(
			classifyViewerGesture({
				dx: 18,
				dy: 90,
				viewState: "fit",
				inDrawer: true,
			}),
		).toBe("drawerScroll");
	});

	it("reserves horizontal movement in a zoomed state for panning", () => {
		expect(
			classifyViewerGesture({
				dx: -72,
				dy: 12,
				viewState: "zoomed",
				inDrawer: false,
			}),
		).toBe("pan");
	});

	it("classifies short movement as a tap", () => {
		expect(
			classifyViewerGesture({
				dx: 4,
				dy: 3,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("tap");
	});

	it("uses a 48-pixel threshold and strict horizontal dominance", () => {
		expect(
			classifyViewerGesture({
				dx: 47,
				dy: 0,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("tap");
		expect(
			classifyViewerGesture({
				dx: 48,
				dy: 38.4,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("none");
		expect(
			classifyViewerGesture({
				dx: 48.01,
				dy: 38.4,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("previous");
	});
});
