import { describe, expect, it } from "vitest";
import type { PickItem } from "../picks/pickList";
import type { WallAsset } from "../services/photoService";
import { hydratePickSequence, nextPickAfterRemoval } from "./pickSequence";

function asset(id: string): WallAsset {
	return {
		id,
		displayName: id.toUpperCase(),
		mediaKind: "jpeg",
		provisionalOrder: 1,
		capturedAtUtc: null,
		dateState: "settled",
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

function pick(assetId: string, value: WallAsset | null): PickItem {
	return {
		assetId,
		sourceFolderId: `${assetId}-folder`,
		sourceLabel: `${assetId} folder`,
		asset: value,
	};
}

describe("pickSequence", () => {
	it("keeps hydrated picks in their cross-folder insertion order", () => {
		const sequence = hydratePickSequence([
			pick("coast", asset("coast")),
			pick("stale", null),
			pick("forest", asset("forest")),
			pick("city", asset("city")),
		]);

		expect(sequence.map((item) => item.id)).toEqual([
			"coast",
			"forest",
			"city",
		]);
	});

	it("selects the next reviewable pick before removing the current one", () => {
		const items = [
			pick("coast", asset("coast")),
			pick("stale", null),
			pick("forest", asset("forest")),
			pick("city", asset("city")),
		];

		expect(nextPickAfterRemoval(items, "forest")).toBe("city");
		expect(nextPickAfterRemoval(items, "city")).toBe("forest");
		expect(nextPickAfterRemoval([pick("only", asset("only"))], "only")).toBe(
			null,
		);
	});

	it("keeps the old insertion position when the current pick becomes stale", () => {
		const items = [
			pick("coast", asset("coast")),
			pick("forest", null),
			pick("city", asset("city")),
		];

		expect(nextPickAfterRemoval(items, "forest")).toBe("city");
		expect(
			nextPickAfterRemoval(
				[pick("coast", asset("coast")), pick("forest", null)],
				"forest",
			),
		).toBe("coast");
	});
});
