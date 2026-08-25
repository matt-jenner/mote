import { describe, expect, it } from "vitest";
import type { WallAsset } from "../services/photoService";
import {
	initialWallState,
	isWallLayoutComplete,
	type WallAction,
	wallReducer,
} from "./wallReducer";

function wallAsset(
	id: string,
	aspectRatio: number,
	provisionalOrder = 1,
): WallAsset {
	return {
		id,
		displayName: `${id}.jpg`,
		mediaKind: "jpeg",
		provisionalOrder,
		capturedAtUtc: null,
		dateState: "provisional",
		width: aspectRatio * 100,
		height: 100,
		representativeRgb: null,
		shapeState: "ready",
		availability: "available",
		warning: null,
		wallThumbnail: null,
		screenPreview: null,
	};
}

function reduce(state: typeof initialWallState, action: WallAction) {
	return wallReducer(state, action);
}

describe("wallReducer", () => {
	it("merges idempotently, refines in place, and resets once at settlement", () => {
		const provisional = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [
				wallAsset("a", 1, 2),
				wallAsset("b", 2, 1),
				wallAsset("a", 1, 2),
			],
			orderState: "provisional",
		});
		expect(provisional.items.map((item) => item.id)).toEqual(["b", "a"]);

		const refined = reduce(provisional, {
			type: "derivativesReady",
			derivatives: [{ assetId: "a", kind: "wallThumbnail", key: "ready" }],
		});
		expect(refined.items[1]).toMatchObject({
			id: "a",
			width: provisional.items[1]?.width,
		});
		expect(refined.items[1]?.wallThumbnail?.key).toBe("ready");

		const settled = reduce(refined, {
			type: "metadataSettled",
			assets: refined.items.slice().reverse(),
		});
		expect(settled.items.map((item) => item.id)).toEqual(["a", "b"]);
		expect(
			reduce(settled, {
				type: "metadataSettled",
				assets: settled.items.slice().reverse(),
			}),
		).toBe(settled);

		const reversed = reduce(settled, {
			type: "setDirection",
			direction: "newestFirst",
		});
		expect(reversed).toMatchObject({
			items: [],
			cursor: null,
			scrollEpoch: settled.scrollEpoch + 1,
			direction: "newestFirst",
		});
	});

	it("keeps the first provisional order for duplicate IDs and merges only changed fields", () => {
		const first = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [wallAsset("a", 1, 1), wallAsset("b", 1, 2)],
			orderState: "provisional",
		});
		const secondAsset = {
			...wallAsset("a", 2, 99),
			displayName: "new-name.jpg",
		};
		const second = reduce(first, {
			type: "catalogBatch",
			assets: [secondAsset],
			orderState: "provisional",
		});

		expect(second.items.map((item) => item.id)).toEqual(["a", "b"]);
		expect(second.items[0]).toMatchObject({
			id: "a",
			displayName: "new-name.jpg",
			width: 200,
		});
		expect(second.items[0]?.provisionalOrder).toBe(1);
	});

	it("loads paged batches with cursors and explicit source completion", () => {
		const firstPage = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("a", 1, 1)],
			orderState: "provisional",
			nextCursor: "page-2",
		});
		expect(firstPage.cursor).toBe("page-2");
		expect(firstPage.pagesExhausted).toBe(false);
		expect(firstPage.scanComplete).toBe(false);
		expect(isWallLayoutComplete(firstPage)).toBe(false);

		const settledFirstPage = reduce(firstPage, {
			type: "metadataSettled",
			assets: firstPage.items,
		});
		expect(settledFirstPage.scanComplete).toBe(true);
		expect(settledFirstPage.pagesExhausted).toBe(false);
		expect(isWallLayoutComplete(settledFirstPage)).toBe(false);

		const terminalPage = reduce(settledFirstPage, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
			nextCursor: null,
		});
		expect(terminalPage.cursor).toBeNull();
		expect(terminalPage.pagesExhausted).toBe(true);
		expect(terminalPage.scanComplete).toBe(true);
		expect(isWallLayoutComplete(terminalPage)).toBe(true);
		expect(terminalPage.items.map((item) => item.id)).toEqual(["a", "b"]);
	});

	it("keeps source completion and settlement latched across streamed updates", () => {
		const settled = reduce(initialWallState, {
			type: "metadataSettled",
			assets: [wallAsset("a", 1, 1)],
		});
		const streamed = reduce(settled, {
			type: "catalogBatch",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
		});
		expect(streamed.settled).toBe(true);
		expect(streamed.scanComplete).toBe(true);
		expect(streamed.pagesExhausted).toBe(false);
		expect(
			reduce(streamed, {
				type: "metadataSettled",
				assets: [wallAsset("c", 1, 3)],
			}),
		).toBe(streamed);
	});

	it("returns the same state and structural references for an identical catalog batch", () => {
		const asset = wallAsset("a", 1, 1);
		const state = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [asset],
			orderState: "provisional",
		});
		expect(
			reduce(state, {
				type: "catalogBatch",
				assets: [],
				orderState: "provisional",
			}),
		).toBe(state);
		const repeated = reduce(state, {
			type: "catalogBatch",
			assets: [asset],
			orderState: "provisional",
		});

		expect(repeated).toBe(state);
		expect(repeated.items).toBe(state.items);
		expect(repeated.items[0]).toBe(state.items[0]);
	});

	it("keeps settled order irreversible across provisional stream updates", () => {
		const settled = reduce(initialWallState, {
			type: "metadataSettled",
			assets: [wallAsset("b", 1, 1), wallAsset("a", 1, 2)],
		});
		expect(settled.items.map((item) => item.id)).toEqual(["b", "a"]);

		const emptyStream = reduce(settled, {
			type: "catalogBatch",
			assets: [],
			orderState: "provisional",
		});
		expect(emptyStream).toBe(settled);

		const streamed = reduce(emptyStream, {
			type: "catalogBatch",
			assets: [wallAsset("c", 1, 0)],
			orderState: "provisional",
		});
		expect(streamed.orderState).toBe("settled");
		expect(streamed.items.map((item) => item.id)).toEqual(["b", "a", "c"]);

		const paged = reduce(streamed, {
			type: "pageLoaded",
			assets: [wallAsset("d", 1, -1)],
			orderState: "provisional",
			nextCursor: null,
		});
		expect(paged.orderState).toBe("settled");
		expect(paged.items.map((item) => item.id)).toEqual(["b", "a", "c", "d"]);
	});

	it("treats cloned nested warning and derivative records as a semantic no-op", () => {
		const asset: WallAsset = {
			...wallAsset("a", 1, 1),
			warning: { code: "unreadable", retryable: true },
			wallThumbnail: { assetId: "a", kind: "wallThumbnail", key: "thumb" },
			screenPreview: { assetId: "a", kind: "screenPreview", key: "screen" },
		};
		const state = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [asset],
			orderState: "provisional",
		});
		const cloned = reduce(state, {
			type: "catalogBatch",
			assets: [
				{
					...asset,
					warning: { code: "unreadable", retryable: true },
					wallThumbnail: {
						assetId: "a",
						kind: "wallThumbnail",
						key: "thumb",
					},
					screenPreview: {
						assetId: "a",
						kind: "screenPreview",
						key: "screen",
					},
				},
			],
			orderState: "provisional",
		});

		expect(cloned).toBe(state);
		expect(cloned.items).toBe(state.items);
		expect(cloned.items[0]).toBe(state.items[0]);
	});

	it("resets paging exhaustion but retains scan completion when direction changes", () => {
		const complete = reduce(
			reduce(initialWallState, {
				type: "metadataSettled",
				assets: [wallAsset("a", 1, 1)],
			}),
			{
				type: "pageLoaded",
				assets: [],
				orderState: "settled",
				nextCursor: null,
			},
		);
		expect(isWallLayoutComplete(complete)).toBe(true);
		const reset = reduce(complete, {
			type: "setDirection",
			direction: "newestFirst",
		});

		expect(reset.scanComplete).toBe(true);
		expect(reset.pagesExhausted).toBe(false);
		expect(reset.cursor).toBeNull();
		expect(reset.items).toEqual([]);
		expect(isWallLayoutComplete(reset)).toBe(false);
	});

	it("replaces only matching derivative reference fields", () => {
		const asset = {
			...wallAsset("a", 1),
			wallThumbnail: {
				assetId: "a",
				kind: "wallThumbnail" as const,
				key: "old-thumb",
			},
			screenPreview: {
				assetId: "a",
				kind: "screenPreview" as const,
				key: "old-screen",
			},
		};
		const state = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [asset],
			orderState: "provisional",
		});
		const refined = reduce(state, {
			type: "derivativesReady",
			derivatives: [{ assetId: "a", kind: "screenPreview", key: "new-screen" }],
		});

		expect(refined.items[0]?.wallThumbnail?.key).toBe("old-thumb");
		expect(refined.items[0]?.screenPreview?.key).toBe("new-screen");
	});
});
