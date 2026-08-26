import { describe, expect, it } from "vitest";
import { chooseViewerViewport } from "./viewerViewport";

describe("chooseViewerViewport", () => {
	it("prefers the visual viewport when it is available", () => {
		expect(
			chooseViewerViewport(
				{ width: 844, height: 390 },
				{ width: 900, height: 500 },
			),
		).toEqual({ width: 844, height: 390 });
	});

	it("falls back to document dimensions when visual viewport is absent", () => {
		expect(chooseViewerViewport(null, { width: 900, height: 500 })).toEqual({
			width: 900,
			height: 500,
		});
	});
});
