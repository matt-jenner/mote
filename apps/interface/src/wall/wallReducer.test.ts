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
			nextCursor: null,
			requestEpoch: 0,
		});
		expect(settled.items.map((item) => item.id)).toEqual(["a", "b"]);
		expect(
			reduce(settled, {
				type: "metadataSettled",
				assets: settled.items.slice().reverse(),
				nextCursor: null,
				requestEpoch: 0,
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
			requestCursor: null,
			requestEpoch: 0,
		});
		expect(firstPage.cursor).toBe("page-2");
		expect(firstPage.pagesExhausted).toBe(false);
		expect(firstPage.scanComplete).toBe(false);
		expect(isWallLayoutComplete(firstPage)).toBe(false);

		const settledFirstPage = reduce(firstPage, {
			type: "metadataSettled",
			assets: firstPage.items,
			nextCursor: firstPage.cursor,
			requestEpoch: firstPage.scrollEpoch,
		});
		expect(settledFirstPage.scanComplete).toBe(true);
		expect(settledFirstPage.pagesExhausted).toBe(false);
		expect(isWallLayoutComplete(settledFirstPage)).toBe(false);

		const terminalPage = reduce(settledFirstPage, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: firstPage.cursor,
			requestEpoch: firstPage.scrollEpoch,
		});
		expect(terminalPage.cursor).toBeNull();
		expect(terminalPage.pagesExhausted).toBe(true);
		expect(terminalPage.scanComplete).toBe(true);
		expect(isWallLayoutComplete(terminalPage)).toBe(true);
		expect(terminalPage.items.map((item) => item.id)).toEqual(["a", "b"]);
	});

	it("atomically settles the ordered first page and replaces its provisional cursor", () => {
		const provisional = reduce(
			reduce(initialWallState, {
				type: "pageLoaded",
				assets: [wallAsset("provisional", 1, 1)],
				orderState: "provisional",
				nextCursor: "provisional-next",
				requestCursor: null,
				requestEpoch: 0,
			}),
			{
				type: "metadataSettled",
				assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
				nextCursor: "settled-next",
				requestEpoch: 0,
			},
		);

		expect(provisional.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(provisional.cursor).toBe("settled-next");
		expect(provisional.pagesExhausted).toBe(false);
		expect(provisional.scanComplete).toBe(true);
		expect(provisional.orderState).toBe("settled");
		expect(provisional.settled).toBe(true);
	});

	it("starts settled from a cached nonterminal page and preserves server order", () => {
		const cached = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "cached-next",
			requestCursor: null,
			requestEpoch: 0,
		});
		expect(cached.settled).toBe(true);
		expect(cached.scanComplete).toBe(true);
		expect(cached.pagesExhausted).toBe(false);
		expect(cached.cursor).toBe("cached-next");
		expect(cached.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(isWallLayoutComplete(cached)).toBe(false);

		const reconciled = reduce(cached, {
			type: "catalogBatch",
			assets: [wallAsset("c", 1, 0)],
			orderState: "provisional",
		});
		expect(reconciled.orderState).toBe("settled");
		expect(reconciled.items.map((item) => item.id)).toEqual(["b", "a", "c"]);
	});

	it("marks a cached terminal settled page layout complete", () => {
		const cached = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
		});

		expect(cached.settled).toBe(true);
		expect(cached.scanComplete).toBe(true);
		expect(cached.pagesExhausted).toBe(true);
		expect(isWallLayoutComplete(cached)).toBe(true);
	});

	it("replaces a settled first page after direction reset in the current epoch", () => {
		const cached = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("old-b", 1, 2), wallAsset("old-a", 1, 1)],
			orderState: "settled",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
		});
		const reset = reduce(cached, {
			type: "setDirection",
			direction: "newestFirst",
		});
		const provisional = reduce(reset, {
			type: "catalogBatch",
			assets: [wallAsset("c", 1, 0)],
			orderState: "provisional",
		});
		const settledFirstPage = reduce(provisional, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "new-next",
			requestCursor: null,
			requestEpoch: reset.scrollEpoch,
		});

		expect(settledFirstPage.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(settledFirstPage.cursor).toBe("new-next");
		expect(settledFirstPage.scrollEpoch).toBe(reset.scrollEpoch);
	});

	it("ignores a stale first page response after direction reset by object identity", () => {
		const cached = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("old", 1, 1)],
			orderState: "settled",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
		});
		const reset = reduce(cached, {
			type: "setDirection",
			direction: "newestFirst",
		});
		const stale = reduce(reset, {
			type: "pageLoaded",
			assets: [wallAsset("stale", 1, 1)],
			orderState: "settled",
			nextCursor: "stale-next",
			requestCursor: null,
			requestEpoch: cached.scrollEpoch,
		});

		expect(stale).toBe(reset);
	});

	it("appends a current-epoch settled second page in server order", () => {
		const first = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "page-2",
			requestCursor: null,
			requestEpoch: 0,
		});
		const second = reduce(first, {
			type: "pageLoaded",
			assets: [wallAsset("d", 1, 4), wallAsset("c", 1, 3)],
			orderState: "settled",
			nextCursor: null,
			requestCursor: first.cursor,
			requestEpoch: first.scrollEpoch,
		});

		expect(second.items.map((item) => item.id)).toEqual(["b", "a", "d", "c"]);
	});

	it("ignores stale metadata settlement after direction reset", () => {
		const provisional = reduce(initialWallState, {
			type: "pageLoaded",
			assets: [wallAsset("old", 1, 1)],
			orderState: "provisional",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
		});
		const reset = reduce(provisional, {
			type: "setDirection",
			direction: "newestFirst",
		});
		const stale = reduce(reset, {
			type: "metadataSettled",
			assets: [wallAsset("stale", 1, 1)],
			nextCursor: null,
			requestEpoch: provisional.scrollEpoch,
		});

		expect(stale).toBe(reset);
	});

	it("keeps source completion and settlement latched across streamed updates", () => {
		const settled = reduce(initialWallState, {
			type: "metadataSettled",
			assets: [wallAsset("a", 1, 1)],
			nextCursor: null,
			requestEpoch: 0,
		});
		const streamed = reduce(settled, {
			type: "catalogBatch",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
		});
		expect(streamed.settled).toBe(true);
		expect(streamed.scanComplete).toBe(true);
		expect(streamed.pagesExhausted).toBe(true);
		expect(
			reduce(streamed, {
				type: "metadataSettled",
				assets: [wallAsset("c", 1, 3)],
				nextCursor: null,
				requestEpoch: 0,
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
			nextCursor: "page-2",
			requestEpoch: 0,
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
			requestCursor: "page-2",
			requestEpoch: 0,
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
				nextCursor: null,
				requestEpoch: 0,
			}),
			{
				type: "pageLoaded",
				assets: [],
				orderState: "settled",
				nextCursor: null,
				requestCursor: null,
				requestEpoch: 0,
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
