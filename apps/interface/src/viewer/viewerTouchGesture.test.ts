import { describe, expect, it } from "vitest";
import {
	DOUBLE_TAP_MAX_DISTANCE,
	DOUBLE_TAP_WINDOW_MS,
	isViewerDoubleTap,
	pinchScale,
	pinchSnapshot,
} from "./viewerTouchGesture";

describe("viewer touch gesture helpers", () => {
	it("derives pinch midpoint and relative scale", () => {
		const start = pinchSnapshot([
			{ pointerId: 1, x: 100, y: 100 },
			{ pointerId: 2, x: 200, y: 100 },
		]);
		const current = pinchSnapshot([
			{ pointerId: 1, x: 50, y: 100 },
			{ pointerId: 2, x: 250, y: 100 },
		]);
		expect(start?.midpoint).toEqual({ x: 150, y: 100 });
		if (!start || !current) throw new Error("pinch snapshots missing");
		expect(pinchScale(start, current)).toBe(2);
	});

	it("accepts a nearby second tap within the exact double-tap window", () => {
		expect(DOUBLE_TAP_WINDOW_MS).toBe(280);
		expect(DOUBLE_TAP_MAX_DISTANCE).toBe(32);
		expect(
			isViewerDoubleTap(
				{ at: 1000, point: { x: 120, y: 160 } },
				{ at: 1220, point: { x: 132, y: 168 } },
			),
		).toBe(true);
	});

	it("rejects late or distant second taps", () => {
		expect(
			isViewerDoubleTap(
				{ at: 1000, point: { x: 120, y: 160 } },
				{ at: 1281, point: { x: 120, y: 160 } },
			),
		).toBe(false);
		expect(
			isViewerDoubleTap(
				{ at: 1000, point: { x: 120, y: 160 } },
				{ at: 1220, point: { x: 153, y: 184 } },
			),
		).toBe(false);
	});

	it("returns no pinch snapshot without two pointers", () => {
		expect(pinchSnapshot([{ pointerId: 1, x: 1, y: 2 }])).toBeNull();
	});
});
