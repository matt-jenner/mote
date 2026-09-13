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
		rating: null,
	};
}

function reduce(state: typeof initialWallState, action: WallAction) {
	return wallReducer(state, action);
}

function thumbnail(id: string) {
	return { assetId: id, kind: "wallThumbnail" as const, key: `${id}-thumb` };
}

function loadedState(items: WallAsset[]) {
	return {
		...initialWallState,
		items,
		selectionId: "selection-a",
		sourceGeneration: 1,
		scanComplete: true,
		pagesExhausted: true,
		orderState: "settled" as const,
	};
}

function activeState() {
	return {
		...initialWallState,
		selectionId: "selection-a",
		sourceGeneration: 1,
	};
}

describe("wallReducer", () => {
	it("sorts known provisional dates immediately and keeps undated photos last", () => {
		const old = {
			...wallAsset("old", 1, 1),
			capturedAtUtc: "2024-01-01T12:00:00Z",
			wallThumbnail: thumbnail("old"),
		};
		const undated = {
			...wallAsset("undated", 1, 2),
			wallThumbnail: thumbnail("undated"),
		};
		const recent = {
			...wallAsset("recent", 1, 3),
			capturedAtUtc: "2024-03-01T12:00:00Z",
			wallThumbnail: thumbnail("recent"),
		};
		const state = {
			...activeState(),
			items: [old, undated, recent],
		};

		const sorted = reduce(state, {
			type: "setDirection",
			direction: "newestFirst",
		});

		expect(sorted.items.map((item) => item.id)).toEqual([
			"recent",
			"old",
			"undated",
		]);
		expect(sorted.items.map((item) => item.wallThumbnail?.key)).toEqual([
			"recent-thumb",
			"old-thumb",
			"undated-thumb",
		]);
	});

	it("repositions a provisional photo when its capture date arrives", () => {
		const old = {
			...wallAsset("old", 1, 1),
			capturedAtUtc: "2024-01-01T12:00:00Z",
		};
		const pending = wallAsset("pending", 1, 2);
		const state = {
			...activeState(),
			direction: "newestFirst" as const,
			items: [old, pending],
		};

		const updated = reduce(state, {
			type: "catalogBatch",
			assets: [
				{
					...pending,
					capturedAtUtc: "2024-03-01T12:00:00Z",
				},
			],
			orderState: "provisional",
		});

		expect(updated.items.map((item) => item.id)).toEqual(["pending", "old"]);
	});

	it("keeps populated thumbnails visible until the replacement sort page arrives", () => {
		const populated = loadedState([
			{ ...wallAsset("old", 1, 1), wallThumbnail: thumbnail("old") },
			{ ...wallAsset("new", 1, 2), wallThumbnail: thumbnail("new") },
		]);
		const pending = wallReducer(populated, {
			type: "setDirection",
			direction: "newestFirst",
		});
		expect(pending.items.map((item) => item.id)).toEqual(["old", "new"]);
		expect(pending.items.every((item) => item.wallThumbnail !== null)).toBe(
			true,
		);
		expect(pending.sortPending).toBe(true);
	});

	it("stores the latest scan counters for the active selection and generation", () => {
		const next = wallReducer(activeState(), {
			type: "progress",
			selectionId: "selection-a",
			generation: 3,
			progress: { discovered: 20, shaped: 12, enriched: 4, total: 20 },
		});
		expect(next.scanProgress).toEqual({
			discovered: 20,
			shaped: 12,
			enriched: 4,
			total: 20,
		});
	});

	it("uses the filtered total when an empty wall settles", () => {
		const progressing = reduce(activeState(), {
			type: "progress",
			selectionId: "selection-a",
			generation: 3,
			progress: { discovered: 29, shaped: 0, enriched: 0, total: 29 },
		});
		const requested = reduce(progressing, {
			type: "pageRequestStarted",
			requestId: "settled-empty",
			requestCursor: null,
			requestEpoch: progressing.scrollEpoch,
		});
		const settled = reduce(requested, {
			type: "pageLoaded",
			assets: [],
			totalCount: 0,
			previewCounts: { wallReady: 0, screenReady: 0 },
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: requested.scrollEpoch,
			requestId: "settled-empty",
		});

		expect(settled.scanComplete).toBe(true);
		expect(settled.totalCount).toBe(0);
	});

	it("does not regress a live preview count when an older wall page arrives", () => {
		const initial = {
			...activeState(),
			previewCounts: { wallReady: 1_033, screenReady: 149 },
		};
		const requested = reduce(initial, {
			type: "pageRequestStarted",
			requestId: "stale-count-page",
			requestCursor: null,
			requestEpoch: initial.scrollEpoch,
		});
		const live = reduce(requested, {
			type: "derivativesReady",
			derivatives: [],
			previewCounts: { wallReady: 1_034, screenReady: 149 },
		});
		const paged = reduce(live, {
			type: "pageLoaded",
			assets: [],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: live.scrollEpoch,
			requestId: "stale-count-page",
			previewCounts: { wallReady: 1_033, screenReady: 149 },
		});

		expect(paged.previewCounts).toEqual({
			wallReady: 1_034,
			screenReady: 149,
		});
	});

	it("accepts a lower authoritative count from a page requested after live updates", () => {
		const live = reduce(
			{
				...activeState(),
				previewCounts: { wallReady: 1_033, screenReady: 149 },
			},
			{
				type: "derivativesReady",
				derivatives: [],
				previewCounts: { wallReady: 1_034, screenReady: 149 },
			},
		);
		const requested = reduce(live, {
			type: "pageRequestStarted",
			requestId: "newer-count-page",
			requestCursor: null,
			requestEpoch: live.scrollEpoch,
		});
		const paged = reduce(requested, {
			type: "pageLoaded",
			assets: [],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: requested.scrollEpoch,
			requestId: "newer-count-page",
			previewCounts: { wallReady: 1_000, screenReady: 140 },
		});

		expect(paged.previewCounts).toEqual({
			wallReady: 1_000,
			screenReady: 140,
		});
	});

	it("stores progress attached to an unchanged catalog batch", () => {
		const current = reduce(activeState(), {
			type: "catalogBatch",
			assets: [wallAsset("same", 1)],
			orderState: "provisional",
			selectionId: "selection-a",
			generation: 2,
			progress: { discovered: 1, shaped: 1, enriched: 0, total: 1 },
		});
		const next = reduce(current, {
			type: "catalogBatch",
			assets: [wallAsset("same", 1)],
			orderState: "provisional",
			selectionId: "selection-a",
			generation: 2,
			progress: { discovered: 1, shaped: 1, enriched: 1, total: 1 },
		});
		expect(next.scanProgress).toEqual({
			discovered: 1,
			shaped: 1,
			enriched: 1,
			total: 1,
		});
	});

	it("keeps the retained sort sequence while a catalog update races replacement", () => {
		const retained = loadedState([
			{ ...wallAsset("old", 1), wallThumbnail: thumbnail("old") },
		]);
		const pending = reduce(retained, {
			type: "setDirection",
			direction: "newestFirst",
		});
		const raced = reduce(pending, {
			type: "catalogBatch",
			assets: [wallAsset("catalog-race", 1)],
			orderState: "provisional",
			selectionId: "selection-a",
		});
		expect(raced.items.map((item) => item.id)).toEqual(["old"]);
		expect(raced.sortPending).toBe(true);
	});

	it("atomically replaces a sorted wall from metadata settlement and merges references", () => {
		const retained = loadedState([
			{
				...wallAsset("old", 1),
				wallThumbnail: thumbnail("old"),
				screenPreview: {
					assetId: "old",
					kind: "screenPreview",
					key: "old-screen",
				},
			},
		]);
		const pending = reduce(retained, {
			type: "setDirection",
			direction: "newestFirst",
		});
		const request = reduce(pending, {
			type: "pageRequestStarted",
			requestId: "settled-replacement",
			requestCursor: null,
			requestEpoch: pending.scrollEpoch,
		});
		const settled = reduce(request, {
			type: "metadataSettled",
			assets: [wallAsset("old", 1)],
			nextCursor: null,
			requestEpoch: pending.scrollEpoch,
			requestCursor: null,
			requestId: "settled-replacement",
		});
		expect(settled.items.map((item) => item.id)).toEqual(["old"]);
		expect(settled.items[0]?.wallThumbnail?.key).toBe("old-thumb");
		expect(settled.items[0]?.screenPreview?.key).toBe("old-screen");
		expect(settled.sortPending).toBe(false);
	});

	it("does not regress scan progress from an older catalog generation", () => {
		const current = reduce(activeState(), {
			type: "progress",
			selectionId: "selection-a",
			generation: 3,
			progress: { discovered: 30, shaped: 20, enriched: 10, total: 30 },
		});
		const stale = reduce(current, {
			type: "catalogBatch",
			assets: [wallAsset("stale", 1)],
			orderState: "provisional",
			selectionId: "selection-a",
			generation: 2,
			progress: { discovered: 20, shaped: 12, enriched: 4, total: 20 },
		});
		expect(stale.scanProgress).toEqual({
			discovered: 30,
			shaped: 20,
			enriched: 10,
			total: 30,
		});
	});

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

	it("preserves the accepted sort direction when the source resets", () => {
		const newest = reduce(initialWallState, {
			type: "setDirection",
			direction: "newestFirst",
		});

		const reset = reduce(newest, {
			type: "resetSource",
			sourceGeneration: 2,
			selectionId: "selection-b",
		});

		expect(reset.direction).toBe("newestFirst");
		expect(reset.selectionId).toBe("selection-b");
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

	it("keeps a loaded wall and marks its assets offline after source loss", () => {
		const cached = {
			...wallAsset("cached", 1),
			wallThumbnail: thumbnail("cached"),
			screenPreview: {
				assetId: "cached",
				kind: "screenPreview" as const,
				key: "cached-screen",
			},
		};
		const uncached = wallAsset("uncached", 1);

		const offline = reduce(loadedState([cached, uncached]), {
			type: "sourceUnavailable",
			sourceGeneration: 1,
		});

		expect(offline.error).toBeNull();
		expect(offline.items).toEqual(
			[cached, uncached].map((item) => ({
				...item,
				availability: "rootOffline",
				warning: { code: "sourceUnavailable", retryable: true },
			})),
		);
		expect(offline.items[0]?.wallThumbnail).toEqual(thumbnail("cached"));
		expect(offline.items[0]?.screenPreview?.key).toBe("cached-screen");
	});

	it("clears a synthetic source warning when an authoritative update recovers", () => {
		const available = wallAsset("recovered", 1);
		const offline = reduce(loadedState([available]), {
			type: "sourceUnavailable",
			sourceGeneration: 1,
		});
		const requested = reduce(offline, {
			type: "pageRequestStarted",
			requestId: "recovery-page",
			requestCursor: null,
			requestEpoch: offline.scrollEpoch,
			sourceGeneration: 1,
		});
		const recovered = reduce(requested, {
			type: "pageLoaded",
			assets: [available],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: requested.scrollEpoch,
			requestId: "recovery-page",
			sourceGeneration: 1,
		});

		expect(recovered.items[0]?.availability).toBe("available");
		expect(recovered.items[0]?.warning).toBeNull();
		expect(recovered.assetWarnings).not.toHaveProperty("recovered");
	});

	it("clears only synthetic source warnings when metadata settles", () => {
		const recovered = wallAsset("metadata-recovered", 1);
		const warning = { code: "metadataWarning", retryable: false };
		const warned = { ...wallAsset("metadata-warned", 1), warning };
		const offline = reduce(loadedState([recovered, warned]), {
			type: "sourceUnavailable",
			sourceGeneration: 1,
		});
		const requested = reduce(offline, {
			type: "pageRequestStarted",
			requestId: "metadata-recovery",
			requestCursor: null,
			requestEpoch: offline.scrollEpoch,
			sourceGeneration: 1,
		});
		const settled = reduce(requested, {
			type: "metadataSettled",
			assets: [recovered, warned],
			nextCursor: null,
			requestEpoch: requested.scrollEpoch,
			requestCursor: null,
			requestId: "metadata-recovery",
			sourceGeneration: 1,
			generation: 2,
		});

		expect(settled.items[0]?.warning).toBeNull();
		expect(settled.assetWarnings).not.toHaveProperty("metadata-recovered");
		expect(settled.items[1]?.warning).toEqual(warning);
		expect(settled.assetWarnings["metadata-warned"]).toEqual(warning);
	});

	it("keeps a synthetic source warning through a catalog replay", () => {
		const available = wallAsset("replayed-offline", 1);
		const offline = reduce(loadedState([available]), {
			type: "sourceUnavailable",
			sourceGeneration: 1,
		});
		const replayed = reduce(offline, {
			type: "catalogBatch",
			selectionId: "selection-a",
			orderState: "settled",
			assets: [available],
		});

		expect(replayed.items[0]?.availability).toBe("rootOffline");
		expect(replayed.items[0]?.warning).toEqual({
			code: "sourceUnavailable",
			retryable: true,
		});
		expect(replayed.assetWarnings["replayed-offline"]).toEqual({
			code: "sourceUnavailable",
			retryable: true,
		});
	});

	it("ignores source loss from an earlier source generation", () => {
		const reset = reduce(loadedState([wallAsset("old", 1)]), {
			type: "resetSource",
			sourceGeneration: 2,
			selectionId: "selection-b",
		});
		const stale = reduce(reset, {
			type: "sourceUnavailable",
			sourceGeneration: 1,
		});

		expect(stale).toBe(reset);
	});

	it("marks source unavailability terminal without inventing page exhaustion", () => {
		const source = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
		});
		const request = reduce(source, {
			type: "pageRequestStarted",
			requestId: "cached-offline",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 4,
		});
		const errored = reduce(request, {
			type: "wallError",
			sourceGeneration: 4,
			error: "Source unavailable. Try again.",
		});

		expect(errored.scanComplete).toBe(true);
		expect(errored.pagesExhausted).toBe(false);
		expect(errored.activeRequest).toEqual(request.activeRequest);
	});

	it("preserves terminal source knowledge through resync and its provisional replacement", () => {
		const source = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
			selectionId: "selection-a",
		});
		const firstRequest = reduce(source, {
			type: "pageRequestStarted",
			requestId: "before-resync",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 4,
		});
		const firstPage = reduce(firstRequest, {
			type: "pageLoaded",
			assets: [wallAsset("stale", 1)],
			orderState: "provisional",
			nextCursor: "stale-next",
			requestCursor: null,
			requestEpoch: 0,
			requestId: "before-resync",
			sourceGeneration: 4,
		});
		const unavailable = reduce(firstPage, {
			type: "wallError",
			sourceGeneration: 4,
			error: "Source unavailable. Try again.",
		});
		const resync = reduce(unavailable, {
			type: "resyncRequired",
			selectionId: "selection-a",
		});

		expect(resync.scanComplete).toBe(true);
		expect(resync.items).toEqual([]);
		expect(resync.cursor).toBeNull();
		expect(resync.pagesExhausted).toBe(false);
		expect(resync.activeRequest).toBeNull();

		const replacementRequest = reduce(resync, {
			type: "pageRequestStarted",
			requestId: "replacement",
			requestCursor: null,
			requestEpoch: 1,
			sourceGeneration: 4,
		});
		const replacement = reduce(replacementRequest, {
			type: "pageLoaded",
			assets: [
				{
					...wallAsset("offline-child", 1),
					availability: "rootOffline",
					wallThumbnail: null,
					screenPreview: null,
				},
			],
			orderState: "provisional",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 1,
			requestId: "replacement",
			sourceGeneration: 4,
		});

		expect(replacement.scanComplete).toBe(true);
		expect(replacement.pagesExhausted).toBe(true);
		expect(isWallLayoutComplete(replacement)).toBe(true);
	});

	it("preserves scan completion only when resetting the same non-null selection", () => {
		const selected = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
			selectionId: "selection-a",
		});
		const complete = reduce(selected, {
			type: "wallError",
			sourceGeneration: 4,
			error: "Source unavailable. Try again.",
		});
		const sameSelection = reduce(complete, {
			type: "resetSource",
			sourceGeneration: 5,
			selectionId: "selection-a",
		});
		const differentSelection = reduce(complete, {
			type: "resetSource",
			sourceGeneration: 5,
			selectionId: "selection-b",
		});
		const noSelection = reduce(complete, {
			type: "resetSource",
			sourceGeneration: 5,
		});

		expect(sameSelection.scanComplete).toBe(true);
		expect(differentSelection.scanComplete).toBe(false);
		expect(noSelection.scanComplete).toBe(false);
	});

	it("accepts a cached page after a source error races its active request", () => {
		const source = reduce(initialWallState, {
			type: "resetSource",
			sourceGeneration: 4,
			selectionId: "selection-a",
		});
		const request = reduce(source, {
			type: "pageRequestStarted",
			requestId: "cached-offline",
			requestCursor: null,
			requestEpoch: 0,
			sourceGeneration: 4,
		});
		const errored = reduce(request, {
			type: "wallError",
			sourceGeneration: 4,
			error: "Source unavailable. Try again.",
		});
		expect(errored.activeRequest).toEqual(request.activeRequest);

		const cachedAsset = {
			...wallAsset("offline", 1),
			availability: "rootOffline" as const,
			wallThumbnail: thumbnail("offline"),
			screenPreview: {
				assetId: "offline",
				kind: "screenPreview" as const,
				key: "offline-screen",
			},
		};
		const loaded = reduce(errored, {
			type: "pageLoaded",
			assets: [cachedAsset],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: 0,
			requestId: "cached-offline",
			sourceGeneration: 4,
		});
		expect(loaded.items).toEqual([cachedAsset]);
		expect(loaded.activeRequest).toBeNull();
		expect(loaded.error).toBeNull();
	});

	it("does not let a replayed catalog batch upgrade offline availability", () => {
		const offline = {
			...wallAsset("child", 1),
			availability: "rootOffline" as const,
		};
		const replayed = reduce(loadedState([offline]), {
			type: "catalogBatch",
			selectionId: "selection-a",
			orderState: "settled",
			assets: [
				{
					...wallAsset("child", 1),
					representativeRgb: 0x123456,
					wallThumbnail: thumbnail("child"),
					screenPreview: {
						assetId: "child",
						kind: "screenPreview",
						key: "child-screen",
					},
				},
			],
		});

		expect(replayed.items[0]).toMatchObject({
			availability: "rootOffline",
			representativeRgb: 0x123456,
			wallThumbnail: { key: "child-thumb" },
			screenPreview: { key: "child-screen" },
		});

		const request = reduce(replayed, {
			type: "pageRequestStarted",
			requestId: "authoritative-recovery",
			requestCursor: null,
			requestEpoch: replayed.scrollEpoch,
			sourceGeneration: replayed.sourceGeneration,
		});
		const recovered = reduce(request, {
			type: "pageLoaded",
			assets: [wallAsset("child", 1)],
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: request.scrollEpoch,
			requestId: "authoritative-recovery",
			sourceGeneration: request.sourceGeneration,
		});
		expect(recovered.items[0]?.availability).toBe("available");
	});

	it("keeps a conservative unavailable catalog replay over available state", () => {
		const replayed = reduce(loadedState([wallAsset("child", 1)]), {
			type: "catalogBatch",
			selectionId: "selection-a",
			orderState: "settled",
			assets: [
				{
					...wallAsset("child", 1),
					availability: "rootOffline",
				},
			],
		});

		expect(replayed.items[0]?.availability).toBe("rootOffline");
	});

	it("merges idempotently, refines in place, and settles without reordering", () => {
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
		expect(settled.items.map((item) => item.id)).toEqual(["b", "a"]);
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
			cursor: null,
			scrollEpoch: settled.scrollEpoch + 1,
			direction: "newestFirst",
		});
		expect(reversed.items).toEqual(refined.items);
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

	it("settles the ordered first page without removing a loaded provisional item", () => {
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

		expect(provisional.items.map((item) => item.id)).toEqual([
			"provisional",
			"b",
			"a",
		]);
		expect(provisional.cursor).toBe("provisional-next");
		expect(provisional.pagesExhausted).toBe(false);
		expect(provisional.scanComplete).toBe(true);
		expect(provisional.orderState).toBe("settled");
		expect(provisional.settledGeneration).toBe(1);
	});

	it("patches settled metadata without dropping loaded pages or regressing pagination", () => {
		const loaded = Array.from({ length: 150 }, (_, index) => ({
			...wallAsset(`asset-${index}`, 1, index + 1),
			displayName: `Provisional ${index}`,
		}));
		const loadedIds = loaded.map(({ id }) => id);
		const current = {
			...activeState(),
			items: loaded,
			cursor: null,
			pagesExhausted: true,
		};
		const requested = reduce(current, {
			type: "pageRequestStarted",
			requestId: "settle-loaded-pages",
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
		});
		const settlement = loaded
			.slice(0, 100)
			.reverse()
			.map((item) => ({
				...item,
				displayName: `Settled ${item.id}`,
				dateState: "settled" as const,
				wallThumbnail: thumbnail(item.id),
			}));

		const settled = reduce(requested, {
			type: "metadataSettled",
			assets: settlement,
			totalCount: 150,
			nextCursor: "settlement-page-2",
			requestEpoch: current.scrollEpoch,
			requestCursor: null,
			requestId: "settle-loaded-pages",
		});

		expect(settled.items).toHaveLength(150);
		expect(settled.items.map(({ id }) => id)).toEqual(loadedIds);
		expect(settled.items.slice(0, 100)).toEqual(
			expect.arrayContaining(
				loaded.slice(0, 100).map((item) =>
					expect.objectContaining({
						id: item.id,
						displayName: `Settled ${item.id}`,
						dateState: "settled",
						wallThumbnail: thumbnail(item.id),
					}),
				),
			),
		);
		expect(settled.items.slice(100)).toEqual(loaded.slice(100));
		expect(settled.cursor).toBeNull();
		expect(settled.pagesExhausted).toBe(true);
	});

	it("patches the same settled generation without dropping unloaded pages", () => {
		const firstPage = wallAsset("first-page", 1, 1);
		const unloadedPage = wallAsset("unloaded-page", 1, 2);
		const loaded = [firstPage, unloadedPage];
		const current = {
			...activeState(),
			items: loaded,
			totalCount: 2,
			cursor: null,
			pagesExhausted: true,
			orderState: "settled" as const,
			settledGeneration: 4,
		};
		const requested = reduce(current, {
			type: "pageRequestStarted",
			requestId: "same-generation",
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
		});
		const patched = reduce(requested, {
			type: "metadataSettled",
			assets: [
				{
					...firstPage,
					displayName: "Updated first page.jpg",
					wallThumbnail: thumbnail("first-page"),
				},
			],
			totalCount: 1,
			nextCursor: null,
			requestEpoch: current.scrollEpoch,
			requestCursor: null,
			requestId: "same-generation",
			generation: 4,
		});

		expect(patched.items.map(({ id }) => id)).toEqual([
			"first-page",
			"unloaded-page",
		]);
		expect(patched.items[0]).toMatchObject({
			displayName: "Updated first page.jpg",
			wallThumbnail: thumbnail("first-page"),
		});
		expect(patched.totalCount).toBe(2);
	});

	it("keeps later loaded pages when the current settled generation reloads page one", () => {
		const firstPage = wallAsset("first-page", 1, 1);
		const laterPage = wallAsset("later-page", 1, 2);
		const loaded = [firstPage, laterPage];
		const current = {
			...activeState(),
			items: loaded,
			totalCount: 2,
			cursor: null,
			pagesExhausted: true,
			orderState: "settled" as const,
			settledGeneration: 4,
		};
		const requested = reduce(current, {
			type: "pageRequestStarted",
			requestId: "reload-current-page-one",
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
		});
		const reloaded = reduce(requested, {
			type: "pageLoaded",
			assets: [
				{
					...firstPage,
					displayName: "Reloaded first page.jpg",
				},
			],
			totalCount: 1,
			orderState: "settled",
			nextCursor: null,
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
			requestId: "reload-current-page-one",
		});

		expect(reloaded.items.map(({ id }) => id)).toEqual([
			"first-page",
			"later-page",
		]);
		expect(reloaded.items[0]?.displayName).toBe("Reloaded first page.jpg");
		expect(reloaded.totalCount).toBe(2);
	});

	it("lets a newer complete settled generation remove absent assets and lower total", () => {
		const current = {
			...activeState(),
			items: [wallAsset("removed", 1, 1), wallAsset("retained", 1, 2)],
			totalCount: 2,
			cursor: null,
			pagesExhausted: true,
			orderState: "settled" as const,
			settledGeneration: 4,
		};
		const requested = reduce(current, {
			type: "pageRequestStarted",
			requestId: "new-complete-generation",
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
		});
		const replaced = reduce(requested, {
			type: "metadataSettled",
			assets: [wallAsset("retained", 1, 2)],
			totalCount: 1,
			nextCursor: null,
			requestEpoch: current.scrollEpoch,
			requestCursor: null,
			requestId: "new-complete-generation",
			generation: 5,
		});

		expect(replaced.items.map(({ id }) => id)).toEqual(["retained"]);
		expect(replaced.totalCount).toBe(1);
		expect(replaced.settledGeneration).toBe(5);
		expect(replaced.pagesExhausted).toBe(true);
	});

	it("keeps an advanced nonterminal cursor when settlement covers only earlier items", () => {
		const loaded = Array.from({ length: 150 }, (_, index) =>
			wallAsset(`asset-${index}`, 1, index + 1),
		);
		const current = {
			...activeState(),
			items: loaded,
			cursor: "page-3",
			pagesExhausted: false,
		};
		const requested = reduce(current, {
			type: "pageRequestStarted",
			requestId: "settle-earlier-page",
			requestCursor: null,
			requestEpoch: current.scrollEpoch,
		});

		const settled = reduce(requested, {
			type: "metadataSettled",
			assets: loaded.slice(0, 100),
			totalCount: 150,
			nextCursor: "page-2",
			requestEpoch: current.scrollEpoch,
			requestCursor: null,
			requestId: "settle-earlier-page",
		});

		expect(settled.items.map((item) => item.id)).toEqual(
			loaded.map((item) => item.id),
		);
		expect(settled.cursor).toBe("page-3");
		expect(settled.pagesExhausted).toBe(false);
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

	it("replaces each newer complete scan generation and patches the current one", () => {
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
		expect(duplicate.items.map((item) => item.id)).toEqual(["second", "stale"]);

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
			rating: null,
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
		expect(reset.items).toEqual(complete.items);
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
