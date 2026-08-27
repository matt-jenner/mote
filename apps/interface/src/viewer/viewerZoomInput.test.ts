import { describe, expect, it } from "vitest";
import { classifyViewerWheel } from "./viewerZoomInput";

describe("classifyViewerWheel", () => {
	it("zooms only for modified wheel input over the viewer", () => {
		expect(
			classifyViewerWheel({
				deltaX: 0,
				deltaY: -80,
				ctrlKey: true,
				metaKey: false,
				mode: "fit",
			}),
		).toMatchObject({ kind: "zoom" });
	});

	it("pans ordinary wheel input only while zoomed", () => {
		expect(
			classifyViewerWheel({
				deltaX: 20,
				deltaY: 40,
				ctrlKey: false,
				metaKey: false,
				mode: "zoomed",
			}),
		).toEqual({ kind: "pan", delta: { x: -20, y: -40 } });
		expect(
			classifyViewerWheel({
				deltaX: 0,
				deltaY: 40,
				ctrlKey: false,
				metaKey: false,
				mode: "fit",
			}),
		).toEqual({ kind: "none" });
	});
});
