import { describe, expect, it } from "vitest";
import {
	navigatorImageBounds,
	navigatorPointToFocal,
	navigatorViewportStyle,
} from "./ViewerNavigator";

describe("viewer navigator geometry", () => {
	it("maps navigator points to clamped normalized focal coordinates", () => {
		expect(
			navigatorPointToFocal(
				{ x: 180, y: 70 },
				{ left: 80, top: 20, width: 200, height: 100 },
			),
		).toEqual({ x: 0.5, y: 0.5 });
		expect(
			navigatorPointToFocal(
				{ x: 400, y: -20 },
				{ left: 80, top: 20, width: 200, height: 100 },
			),
		).toEqual({ x: 1, y: 0 });
	});

	it("fits the thumbnail inside navigator bounds without distorting its aspect ratio", () => {
		expect(navigatorImageBounds(200, 120, 1200, 800)).toEqual({
			left: 10,
			top: 0,
			width: 180,
			height: 120,
		});
		expect(navigatorImageBounds(200, 120, 800, 1200)).toEqual({
			left: 60,
			top: 0,
			width: 80,
			height: 120,
		});
	});

	it("maps a normalized visible rectangle to thumbnail percentages", () => {
		expect(
			navigatorViewportStyle({ x: 0.25, y: 0.125, width: 0.5, height: 0.75 }),
		).toEqual({
			left: "25%",
			top: "12.5%",
			width: "50%",
			height: "75%",
		});
	});
});
