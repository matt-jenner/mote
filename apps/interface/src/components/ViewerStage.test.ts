import { describe, expect, it } from "vitest";
import { drawableViewerBox, fitViewerFrame } from "./ViewerStage";

describe("viewer stage geometry", () => {
	it("subtracts safe padding before fitting the drawable image box", () => {
		expect(
			drawableViewerBox(1440, 1024, {
				top: 44,
				right: 44,
				bottom: 44,
				left: 44,
			}),
		).toEqual({ width: 1352, height: 936 });
		expect(
			drawableViewerBox(390, 844, {
				top: 12,
				right: 24,
				bottom: 34,
				left: 8,
			}),
		).toEqual({ width: 358, height: 798 });
		expect(
			drawableViewerBox(844, 390, {
				top: 24,
				right: 34,
				bottom: 12,
				left: 8,
			}),
		).toEqual({ width: 802, height: 354 });
	});

	it("preserves aspect ratio inside desktop and phone bounds", () => {
		const desktop = fitViewerFrame(1352, 936, 1200, 800);
		expect(desktop).toEqual({ width: 1352, height: 901.3333333333334 });
		const portrait = fitViewerFrame(358, 798, 800, 1200);
		expect(portrait.width / portrait.height).toBeCloseTo(2 / 3);
		expect(portrait.width).toBeLessThanOrEqual(358);
		expect(portrait.height).toBeLessThanOrEqual(798);
		const landscape = fitViewerFrame(802, 354, 1200, 800);
		expect(landscape.width / landscape.height).toBeCloseTo(3 / 2);
		expect(landscape.width).toBeLessThanOrEqual(802);
		expect(landscape.height).toBeLessThanOrEqual(354);
	});
});
