import { describe, expect, it } from "vitest";
import { controlHideDelay } from "./useViewerControls";

describe("viewer control timing", () => {
	it("uses the desktop and touch inactivity delays", () => {
		expect(controlHideDelay("mouse")).toBe(2500);
		expect(controlHideDelay("touch")).toBe(3500);
	});
});
