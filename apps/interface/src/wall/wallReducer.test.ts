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
	it("resets a source and ignores stale failures from the prior source", () => {
		const request = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "a-1",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 1,
		});
		const reset = reduce(request, {
			type: "resetSource",
			sourceGeneration: 2,
		});
		const stale = reduce(reset, {
			type: "pageRequestFailed",
			requestId: "a-1",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 1,
			error: "old source failed",
		});
		expect(stale).toEqual(reset);
		expect(stale.items).toEqual([]);
	});

	it("accepts a generation-fenced first page after source reset", () => {
		const reset = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 1,
		});
		const started = reduce(reset, {
			type: "pageRequestStarted",
			requestId: "first",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 1,
		});
		const loaded = reduce(started, {
			type: "pageLoaded",
			assets: [wallAsset("one", 1)],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "first",
			sourceGeneration: 1,
		});
		expect(loaded.items.map((item) => item.id)).toEqual(["one"]);
	});

	it("clears a matching failed request and keeps a retryable error", () => {
		const source = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
		});
		const request = reduce(source, {
			type: "pageRequestStarted",
			requestId: "request-1",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 4,
		});
		const failed = reduce(request, {
			type: "pageRequestFailed",
			requestId: "request-1",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 4,
			error: "Try again",
		});
		expect(failed.activeRequest).toBeNull();
		expect(failed.error).toBe("Try again");
	});

	it("surfaces a source-scoped error even when no request is active", () => {
		const source = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
		});
		const errored = reduce(source, {
			type: "wallError",
			sourceGeneration: 4,
			error: "Source unavailable. Try again.",
		});
		expect(errored.error).toBe("Source unavailable. Try again.");
		expect(errored.activeRequest).toBeNull();
	});

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

		const settlementRequest = reduce(refined, {
			type: "pageRequestStarted",
			requestId: "settle-1",
			requestCursor: null,
			requestEpoch: refined.scrollEpoch,
		});
		const settled = reduce(settlementRequest, {
			type: "metadataSettled",
			assets: refined.items.slice().reverse(),
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "settle-1",
		});
		expect(settled.items.map((item) => item.id)).toEqual(["a", "b"]);
		const repeatedRequest = reduce(settled, {
			type: "pageRequestStarted",
			requestId: "settle-2",
			requestCursor: null,
			requestEpoch: settled.scrollEpoch,
		});
		const repeated = reduce(repeatedRequest, {
			type: "metadataSettled",
			assets: settled.items.slice().reverse(),
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "settle-2",
		});
		expect(repeated.items).toBe(settled.items);
		expect(repeated.activeRequest).toBeNull();

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
		const firstRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "first",
			requestCursor: null,
			requestEpoch: 0,
		});
		const firstPage = reduce(firstRequest, {
			type: "pageLoaded",
			assets: [wallAsset("a", 1, 1)],
			orderState: "provisional",
			nextCursor: "page-2",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "first",
		});
		expect(firstPage.cursor).toBe("page-2");
		expect(firstPage.pagesExhausted).toBe(false);
		expect(firstPage.scanComplete).toBe(false);
		expect(isWallLayoutComplete(firstPage)).toBe(false);

		const settledFirstPage = reduce(firstPage, {
			type: "pageRequestStarted",
			requestId: "settle-first",
			requestCursor: firstPage.cursor,
			requestEpoch: firstPage.scrollEpoch,
		});
		const settled = reduce(settledFirstPage, {
			type: "metadataSettled",
			assets: firstPage.items,
			nextCursor: firstPage.cursor,
			requestEpoch: firstPage.scrollEpoch,
			requestCursor: firstPage.cursor,
			requestId: "settle-first",
		});
		expect(settled.scanComplete).toBe(true);
		expect(settled.pagesExhausted).toBe(false);
		expect(isWallLayoutComplete(settled)).toBe(false);

		const terminalRequest = reduce(settled, {
			type: "pageRequestStarted",
			requestId: "terminal",
			requestCursor: settled.cursor,
			requestEpoch: settled.scrollEpoch,
		});
		const terminalPage = reduce(terminalRequest, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: settled.cursor,
			requestEpoch: settled.scrollEpoch,
			requestId: "terminal",
		});
		expect(terminalPage.cursor).toBeNull();
		expect(terminalPage.pagesExhausted).toBe(true);
		expect(terminalPage.scanComplete).toBe(true);
		expect(isWallLayoutComplete(terminalPage)).toBe(true);
		expect(terminalPage.items.map((item) => item.id)).toEqual(["a", "b"]);
	});

	it("atomically settles the ordered first page and replaces its provisional cursor", () => {
		const provisionalRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "provisional",
			requestCursor: null,
			requestEpoch: 0,
		});
		const provisionalPage = reduce(provisionalRequest, {
			type: "pageLoaded",
			assets: [wallAsset("provisional", 1, 1)],
			orderState: "provisional",
			nextCursor: "provisional-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "provisional",
		});
		const settlementRequest = reduce(provisionalPage, {
			type: "pageRequestStarted",
			requestId: "settled",
			requestCursor: null,
			requestEpoch: provisionalPage.scrollEpoch,
		});
		const provisional = reduce(settlementRequest, {
			type: "metadataSettled",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			nextCursor: "settled-next",
			requestEpoch: 0,
			requestCursor: null,
			requestId: "settled",
		});

		expect(provisional.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(provisional.cursor).toBe("settled-next");
		expect(provisional.pagesExhausted).toBe(false);
		expect(provisional.scanComplete).toBe(true);
		expect(provisional.orderState).toBe("settled");
		expect(provisional.settledGeneration).toBe(1);
	});

	it("starts settled from a cached nonterminal page and preserves server order", () => {
		const cachedRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "cached",
			requestCursor: null,
			requestEpoch: 0,
		});
		const cached = reduce(cachedRequest, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "cached-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "cached",
		});
		expect(cached.settledGeneration).toBe(1);
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
		const cachedRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "cached-terminal",
			requestCursor: null,
			requestEpoch: 0,
		});
		const cached = reduce(cachedRequest, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "cached-terminal",
		});

		expect(cached.settledGeneration).toBe(1);
		expect(cached.scanComplete).toBe(true);
		expect(cached.pagesExhausted).toBe(true);
		expect(isWallLayoutComplete(cached)).toBe(true);
	});

	it("replaces a settled first page after direction reset in the current epoch", () => {
		const cachedRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "cached-old",
			requestCursor: null,
			requestEpoch: 0,
		});
		const cached = reduce(cachedRequest, {
			type: "pageLoaded",
			assets: [wallAsset("old-b", 1, 2), wallAsset("old-a", 1, 1)],
			orderState: "settled",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "cached-old",
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
		const settledRequest = reduce(provisional, {
			type: "pageRequestStarted",
			requestId: "new-first",
			requestCursor: null,
			requestEpoch: reset.scrollEpoch,
		});
		const settledFirstPage = reduce(settledRequest, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "new-next",
			requestCursor: null,
			requestEpoch: reset.scrollEpoch,
			requestId: "new-first",
		});

		expect(settledFirstPage.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(settledFirstPage.cursor).toBe("new-next");
		expect(settledFirstPage.scrollEpoch).toBe(reset.scrollEpoch);
	});

	it("ignores a stale first page response after direction reset by object identity", () => {
		const cachedRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "old-page",
			requestCursor: null,
			requestEpoch: 0,
		});
		const cached = reduce(cachedRequest, {
			type: "pageLoaded",
			assets: [wallAsset("old", 1, 1)],
			orderState: "settled",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "old-page",
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
			requestId: "stale-old",
		});

		expect(stale).toBe(reset);
	});

	it("appends a current-epoch settled second page in server order", () => {
		const firstRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "first-settled",
			requestCursor: null,
			requestEpoch: 0,
		});
		const first = reduce(firstRequest, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "page-2",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "first-settled",
		});
		const secondRequest = reduce(first, {
			type: "pageRequestStarted",
			requestId: "second-settled",
			requestCursor: first.cursor,
			requestEpoch: first.scrollEpoch,
		});
		const second = reduce(secondRequest, {
			type: "pageLoaded",
			assets: [wallAsset("d", 1, 4), wallAsset("c", 1, 3)],
			orderState: "settled",
			nextCursor: null,
			requestCursor: first.cursor,
			requestEpoch: first.scrollEpoch,
			requestId: "second-settled",
		});

		expect(second.items.map((item) => item.id)).toEqual(["b", "a", "d", "c"]);
	});

	it("ignores stale metadata settlement after direction reset", () => {
		const provisionalRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "old-page",
			requestCursor: null,
			requestEpoch: 0,
		});
		const provisional = reduce(provisionalRequest, {
			type: "pageLoaded",
			assets: [wallAsset("old", 1, 1)],
			orderState: "provisional",
			nextCursor: "old-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "old-page",
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
			requestCursor: null,
			requestId: "stale-settle",
		});

		expect(stale).toBe(reset);
	});

	it("ignores a late provisional response after a same-epoch settlement requery wins", () => {
		const requestA = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "A",
			requestCursor: null,
			requestEpoch: 0,
		});
		const requestB = reduce(requestA, {
			type: "pageRequestStarted",
			requestId: "B",
			requestCursor: null,
			requestEpoch: 0,
		});
		const settled = reduce(requestB, {
			type: "metadataSettled",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			nextCursor: "settled-next",
			requestId: "B",
			requestCursor: null,
			requestEpoch: 0,
		});
		const late = reduce(settled, {
			type: "pageLoaded",
			assets: [wallAsset("late", 1, 0)],
			orderState: "provisional",
			nextCursor: "late-next",
			requestId: "A",
			requestCursor: null,
			requestEpoch: 0,
		});

		expect(settled.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(late).toBe(settled);
	});

	it("lets only the latest same-epoch settled first-page request install results", () => {
		const requestA = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: 1,
			requestCursor: null,
			requestEpoch: 0,
		});
		const requestB = reduce(requestA, {
			type: "pageRequestStarted",
			requestId: 2,
			requestCursor: null,
			requestEpoch: 0,
		});
		const current = reduce(requestB, {
			type: "pageLoaded",
			assets: [wallAsset("b", 1, 2), wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: null,
			requestId: 2,
			requestCursor: null,
			requestEpoch: 0,
		});
		const stale = reduce(current, {
			type: "pageLoaded",
			assets: [wallAsset("stale", 1, 0)],
			orderState: "settled",
			nextCursor: null,
			requestId: 1,
			requestCursor: null,
			requestEpoch: 0,
		});

		expect(current.items.map((item) => item.id)).toEqual(["b", "a"]);
		expect(stale).toBe(current);
	});

	it("ignores a stale continuation after a newer first-page request starts", () => {
		const firstRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "first",
			requestCursor: null,
			requestEpoch: 0,
		});
		const firstPage = reduce(firstRequest, {
			type: "pageLoaded",
			assets: [wallAsset("a", 1, 1)],
			orderState: "settled",
			nextCursor: "page-2",
			requestId: "first",
			requestCursor: null,
			requestEpoch: 0,
		});
		const continuationRequest = reduce(firstPage, {
			type: "pageRequestStarted",
			requestId: "continuation",
			requestCursor: firstPage.cursor,
			requestEpoch: 0,
		});
		const newerFirstRequest = reduce(continuationRequest, {
			type: "pageRequestStarted",
			requestId: "new-first",
			requestCursor: null,
			requestEpoch: 0,
		});
		const staleContinuation = reduce(newerFirstRequest, {
			type: "pageLoaded",
			assets: [wallAsset("stale", 1, 2)],
			orderState: "settled",
			nextCursor: null,
			requestId: "continuation",
			requestCursor: "page-2",
			requestEpoch: 0,
		});

		expect(staleContinuation).toBe(newerFirstRequest);
	});

	it("keeps source completion and settlement latched across streamed updates", () => {
		const settled = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "settle-stream",
			requestCursor: null,
			requestEpoch: 0,
		});
		const settledState = reduce(settled, {
			type: "metadataSettled",
			assets: [wallAsset("a", 1, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "settle-stream",
		});
		const streamed = reduce(settledState, {
			type: "catalogBatch",
			assets: [wallAsset("b", 1, 2)],
			orderState: "provisional",
		});
		expect(streamed.settledGeneration).toBe(1);
		expect(streamed.scanComplete).toBe(true);
		expect(streamed.pagesExhausted).toBe(true);
		expect(
			reduce(streamed, {
				type: "metadataSettled",
				assets: [wallAsset("c", 1, 3)],
				nextCursor: null,
				requestEpoch: 0,
				requestCursor: null,
				requestId: "stale-stream",
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
		const settlementRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "settled-order",
			requestCursor: null,
			requestEpoch: 0,
		});
		const settled = reduce(settlementRequest, {
			type: "metadataSettled",
			assets: [wallAsset("b", 1, 1), wallAsset("a", 1, 2)],
			nextCursor: "page-2",
			requestEpoch: 0,
			requestCursor: null,
			requestId: "settled-order",
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

		const pagedRequest = reduce(streamed, {
			type: "pageRequestStarted",
			requestId: "settled-page",
			requestCursor: "page-2",
			requestEpoch: streamed.scrollEpoch,
		});
		const paged = reduce(pagedRequest, {
			type: "pageLoaded",
			assets: [wallAsset("d", 1, -1)],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: "page-2",
			requestEpoch: 0,
			requestId: "settled-page",
		});
		expect(paged.orderState).toBe("settled");
		expect(paged.items.map((item) => item.id)).toEqual(["b", "a", "c", "d"]);
	});

	it("accepts one atomic replacement for each newer scan generation", () => {
		const firstRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "generation-1",
			requestCursor: null,
			requestEpoch: 0,
		});
		const first = reduce(firstRequest, {
			type: "metadataSettled",
			generation: 1,
			assets: [wallAsset("first", 1, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "generation-1",
		});
		const secondRequest = reduce(first, {
			type: "pageRequestStarted",
			requestId: "generation-2",
			requestCursor: null,
			requestEpoch: 0,
		});
		const second = reduce(secondRequest, {
			type: "metadataSettled",
			generation: 2,
			assets: [wallAsset("second", 2, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "generation-2",
		});
		expect(second.items.map((item) => item.id)).toEqual(["second"]);

		const duplicateRequest = reduce(second, {
			type: "pageRequestStarted",
			requestId: "generation-2-duplicate",
			requestCursor: null,
			requestEpoch: 0,
		});
		const duplicate = reduce(duplicateRequest, {
			type: "metadataSettled",
			generation: 2,
			assets: [wallAsset("stale", 3, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "generation-2-duplicate",
		});
		expect(duplicate.items.map((item) => item.id)).toEqual(["second"]);

		const staleRequest = reduce(second, {
			type: "pageRequestStarted",
			requestId: "generation-1-stale",
			requestCursor: null,
			requestEpoch: 0,
		});
		const stale = reduce(staleRequest, {
			type: "metadataSettled",
			generation: 1,
			assets: [wallAsset("stale", 4, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "generation-1-stale",
		});
		expect(stale.items.map((item) => item.id)).toEqual(["second"]);
	});

	it("stores and clears source and asset warnings", () => {
		const source = reduce(initialWallState, {
			type: "catalogBatch",
			assets: [wallAsset("warned", 1, 1)],
			orderState: "provisional",
		});
		const warned = reduce(source, {
			type: "warning",
			sourceId: "source",
			assetId: "warned",
			warning: { code: "derivativeUnavailable", retryable: true },
		});
		expect(warned.items[0]?.warning?.code).toBe("derivativeUnavailable");
		expect(warned.assetWarnings.warned?.code).toBe("derivativeUnavailable");

		const cleared = reduce(warned, {
			type: "warningCleared",
			sourceId: "source",
			assetId: "warned",
			code: "derivativeUnavailable",
		});
		expect(cleared.items[0]?.warning).toBeNull();
		expect(cleared.assetWarnings.warned).toBeUndefined();
		expect(cleared.warningTombstones).toEqual({});

		const earlyWarning = reduce(initialWallState, {
			type: "warning",
			sourceId: "source",
			assetId: "future",
			warning: { code: "derivativeUnavailable", retryable: true },
		});
		const materialized = reduce(earlyWarning, {
			type: "catalogBatch",
			assets: [wallAsset("future", 1, 1)],
			orderState: "provisional",
		});
		expect(materialized.items[0]?.warning?.code).toBe("derivativeUnavailable");
	});

	it("keeps source warning codes independent", () => {
		const withThumbnailWarning = reduce(initialWallState, {
			type: "warning",
			sourceId: "source",
			assetId: null,
			warning: { code: "wallThumbnailUnavailable", retryable: true },
		});
		const withBothWarnings = reduce(withThumbnailWarning, {
			type: "warning",
			sourceId: "source",
			assetId: null,
			warning: { code: "screenPreviewUnavailable", retryable: false },
		});
		expect(Object.keys(withBothWarnings.sourceWarnings)).toEqual([
			"wallThumbnailUnavailable",
			"screenPreviewUnavailable",
		]);

		const withScreenWarning = reduce(withBothWarnings, {
			type: "warningCleared",
			sourceId: "source",
			assetId: null,
			code: "wallThumbnailUnavailable",
		});
		expect(withScreenWarning.sourceWarnings).toEqual({
			screenPreviewUnavailable: {
				code: "screenPreviewUnavailable",
				retryable: false,
			},
		});
		expect(withScreenWarning.sourceWarningTombstones).toEqual({});
		const clear = reduce(withScreenWarning, {
			type: "warningCleared",
			sourceId: "source",
			assetId: null,
			code: "screenPreviewUnavailable",
		});
		expect(clear.sourceWarnings).toEqual({});
	});

	it("hydrates source warnings from a page snapshot without resurrecting a clear", () => {
		const request = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "source-snapshot",
			requestCursor: null,
			requestEpoch: 0,
		});
		const snapshot = reduce(request, {
			type: "pageLoaded",
			assets: [],
			sourceWarnings: [
				{ code: "wallThumbnailCacheUnavailable", retryable: true },
				{ code: "screenPreviewCacheUnavailable", retryable: true },
			],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "source-snapshot",
		});
		expect(Object.keys(snapshot.sourceWarnings)).toEqual([
			"wallThumbnailCacheUnavailable",
			"screenPreviewCacheUnavailable",
		]);

		const refreshRequest = reduce(snapshot, {
			type: "pageRequestStarted",
			requestId: "stale-source-snapshot",
			requestCursor: null,
			requestEpoch: snapshot.scrollEpoch,
		});
		const cleared = reduce(refreshRequest, {
			type: "warningCleared",
			sourceId: "source",
			assetId: null,
			code: "wallThumbnailCacheUnavailable",
		});
		const staleSnapshot = reduce(cleared, {
			type: "pageLoaded",
			assets: [],
			sourceWarnings: [
				{ code: "wallThumbnailCacheUnavailable", retryable: true },
				{ code: "screenPreviewCacheUnavailable", retryable: true },
			],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: snapshot.scrollEpoch,
			requestId: "stale-source-snapshot",
		});
		expect(staleSnapshot.sourceWarnings).toEqual({
			screenPreviewCacheUnavailable: {
				code: "screenPreviewCacheUnavailable",
				retryable: true,
			},
		});
	});

	it("does not resurrect an asset warning from a stale page after clear", () => {
		const warned = reduce(
			reduce(initialWallState, {
				type: "catalogBatch",
				assets: [wallAsset("race", 1)],
				orderState: "provisional",
			}),
			{
				type: "warning",
				sourceId: "source",
				assetId: "race",
				warning: { code: "derivativeUnavailable", retryable: true },
			},
		);
		const request = reduce(warned, {
			type: "pageRequestStarted",
			requestId: "stale-page",
			requestCursor: null,
			requestEpoch: warned.scrollEpoch,
		});
		const cleared = reduce(request, {
			type: "warningCleared",
			sourceId: "source",
			assetId: "race",
			code: "derivativeUnavailable",
		});
		const stalePage = reduce(cleared, {
			type: "pageLoaded",
			assets: [
				{
					...wallAsset("race", 1),
					warning: { code: "derivativeUnavailable", retryable: true },
				},
			],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: warned.scrollEpoch,
			requestId: "stale-page",
		});
		expect(stalePage.items[0]?.warning).toBeNull();
		expect(stalePage.assetWarnings.race).toBeUndefined();
		expect(
			stalePage.warningTombstones["race\u0000derivativeUnavailable"],
		).toBeUndefined();

		const liveWarning = reduce(stalePage, {
			type: "warning",
			sourceId: "source",
			assetId: "race",
			warning: { code: "derivativeUnavailable", retryable: true },
		});
		expect(liveWarning.items[0]?.warning?.code).toBe("derivativeUnavailable");
		expect(
			liveWarning.warningTombstones["race\u0000derivativeUnavailable"],
		).toBeUndefined();
	});

	it("fences asset warning clears by asset and warning code", () => {
		const assetState = reduce(
			reduce(initialWallState, {
				type: "catalogBatch",
				assets: [wallAsset("identity", 1)],
				orderState: "provisional",
			}),
			{
				type: "warning",
				sourceId: "source",
				assetId: "identity",
				warning: { code: "derivativeUnavailable", retryable: true },
			},
		);
		const request = reduce(assetState, {
			type: "pageRequestStarted",
			requestId: "identity-stale-page",
			requestCursor: null,
			requestEpoch: assetState.scrollEpoch,
		});
		const clearedDerivative = reduce(request, {
			type: "warningCleared",
			sourceId: "source",
			assetId: "identity",
			code: "derivativeUnavailable",
		});
		const unrelatedWarning = reduce(clearedDerivative, {
			type: "warning",
			sourceId: "source",
			assetId: "identity",
			warning: { code: "assetWarning", retryable: true },
		});
		const staleDerivative = reduce(unrelatedWarning, {
			type: "pageLoaded",
			assets: [
				{
					...wallAsset("identity", 1),
					warning: { code: "derivativeUnavailable", retryable: true },
				},
			],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: assetState.scrollEpoch,
			requestId: "identity-stale-page",
		});
		expect(staleDerivative.items[0]?.warning?.code).toBe("assetWarning");
		expect(
			staleDerivative.warningTombstones["identity\u0000derivativeUnavailable"],
		).toBeUndefined();
	});

	it("bounds and consumes request-scoped warning tombstones", () => {
		let state = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "bounded",
			requestCursor: null,
			requestEpoch: 0,
		});
		for (let index = 0; index < 1000; index += 1) {
			state = reduce(state, {
				type: "warningCleared",
				sourceId: "source",
				assetId: `asset-${index}`,
				code: `warning-${index}`,
			});
			state = reduce(state, {
				type: "warningCleared",
				sourceId: "source",
				assetId: null,
				code: `source-warning-${index}`,
			});
		}
		expect(Object.keys(state.warningTombstones).length).toBeLessThanOrEqual(
			128,
		);
		expect(
			Object.keys(state.sourceWarningTombstones).length,
		).toBeLessThanOrEqual(128);

		const settled = reduce(state, {
			type: "pageLoaded",
			assets: [],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "bounded",
			sourceWarnings: [],
		});
		expect(settled.warningTombstones).toEqual({});
		expect(settled.sourceWarningTombstones).toEqual({});
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
		const settlementRequest = reduce(initialWallState, {
			type: "pageRequestStarted",
			requestId: "complete-settle",
			requestCursor: null,
			requestEpoch: 0,
		});
		const settled = reduce(settlementRequest, {
			type: "metadataSettled",
			assets: [wallAsset("a", 1, 1)],
			nextCursor: null,
			requestEpoch: 0,
			requestCursor: null,
			requestId: "complete-settle",
		});
		const pageRequest = reduce(settled, {
			type: "pageRequestStarted",
			requestId: "complete-page",
			requestCursor: null,
			requestEpoch: settled.scrollEpoch,
		});
		const complete = reduce(pageRequest, {
			type: "pageLoaded",
			assets: [],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "complete-page",
		});
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
