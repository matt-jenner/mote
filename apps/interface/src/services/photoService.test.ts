import { describe, expect, it } from "vitest";
import {
	createInMemoryPhotoService,
	type InMemoryWallFixture,
} from "./inMemoryPhotoService";
import type { WallAsset, WallUpdate } from "./photoService";

const fixtureAsset: WallAsset = {
	id: "asset-a",
	displayName: "asset-a.jpg",
	mediaKind: "jpeg",
	provisionalOrder: 1,
	capturedAtUtc: null,
	dateState: "provisional",
	width: 640,
	height: 480,
	representativeRgb: null,
	shapeState: "ready",
	availability: "available",
	warning: null,
	wallThumbnail: null,
	screenPreview: null,
};

const fixtureAssets: readonly InMemoryWallFixture[] = [fixtureAsset];

const datedFixtures: readonly InMemoryWallFixture[] = [
	{
		...fixtureAsset,
		id: "asset-z",
		displayName: "Zeta.jpg",
		provisionalOrder: 4,
		capturedAtUtc: "2024-01-01T00:00:00Z",
	},
	{
		...fixtureAsset,
		id: "asset-b",
		displayName: "Same.jpg",
		provisionalOrder: 2,
		capturedAtUtc: "2024-01-01T00:00:00Z",
	},
	{
		...fixtureAsset,
		id: "asset-a",
		displayName: "Same.jpg",
		provisionalOrder: 3,
		capturedAtUtc: "2024-01-01T00:00:00Z",
	},
	{
		...fixtureAsset,
		id: "asset-new",
		displayName: "Newest.jpg",
		provisionalOrder: 1,
		capturedAtUtc: "2025-01-01T00:00:00Z",
	},
];

const thumbnailReference: WallAsset["wallThumbnail"] = {
	assetId: "asset-a",
	kind: "wallThumbnail",
	key: "thumb-a",
};

const sampleProgressUpdate: WallUpdate = {
	kind: "progress",
	progress: { discovered: 1, shaped: 1, enriched: 1, total: 1 },
};

describe("PhotoService contract", () => {
	it("persists a selected source and appearance for the adapter lifetime", async () => {
		const service = createInMemoryPhotoService({
			selectedFolderName: "Iceland 2025",
		});
		expect((await service.getBootstrapState()).activeSource).toBeNull();

		const chosen = await service.chooseFolder();
		expect(chosen.kind).toBe("selected");
		if (chosen.kind !== "selected") throw new Error("expected selected folder");
		expect(chosen.state.activeSource?.displayName).toBe("Iceland 2025");

		const updated = await service.updateAppearance("dark");
		expect(updated.settings.appearance).toBe("dark");
		expect((await service.getBootstrapState()).activeSource?.displayName).toBe(
			"Iceland 2025",
		);
	});

	it("represents picker cancellation without throwing", async () => {
		const service = createInMemoryPhotoService({ cancelFolderPicker: true });
		await expect(service.chooseFolder()).resolves.toEqual({
			kind: "cancelled",
		});
	});

	it("emits geometry before metadata settles and stops after unsubscribe", async () => {
		const service = createInMemoryPhotoService({
			wallAssets: fixtureAssets,
			geometryDelayMs: 0,
			thumbnailDelayMs: 50,
			metadataDelayMs: 100,
		});
		const events: WallUpdate[] = [];
		const stop = service.watchWallUpdates((event) => events.push(event));
		await service.startFixtureScan();
		expect(events[0]?.kind).toBe("catalogBatch");
		expect(events.some((event) => event.kind === "metadataSettled")).toBe(
			false,
		);
		await service.finishFixtureScan();
		expect(events.at(-1)?.kind).toBe("metadataSettled");
		stop();
		service.emitForTest(sampleProgressUpdate);
		expect(events.at(-1)?.kind).toBe("metadataSettled");
	});

	it("settles newest ordering with ascending equal-date tie breakers", async () => {
		const service = createInMemoryPhotoService({ wallAssets: datedFixtures });
		await service.finishFixtureScan();

		const oldest = await service.queryWall({
			cursor: null,
			limit: 10,
			direction: "oldestFirst",
		});
		const newest = await service.queryWall({
			cursor: null,
			limit: 10,
			direction: "newestFirst",
		});

		expect(oldest.items.map((asset) => asset.id)).toEqual([
			"asset-a",
			"asset-b",
			"asset-z",
			"asset-new",
		]);
		expect(newest.items.map((asset) => asset.id)).toEqual([
			"asset-new",
			"asset-a",
			"asset-b",
			"asset-z",
		]);
	});

	it("honors bounded cursors and rejects invalid wall limits", async () => {
		const service = createInMemoryPhotoService({ wallAssets: datedFixtures });
		await service.finishFixtureScan();
		const first = await service.queryWall({
			cursor: null,
			limit: 2,
			direction: "oldestFirst",
		});
		const second = await service.queryWall({
			cursor: first.nextCursor,
			limit: 2,
			direction: "oldestFirst",
		});

		expect(first.items.map((asset) => asset.id)).toEqual([
			"asset-a",
			"asset-b",
		]);
		expect(first.nextCursor).toBe("2");
		expect(second.items.map((asset) => asset.id)).toEqual([
			"asset-z",
			"asset-new",
		]);
		expect(second.nextCursor).toBeNull();
		await expect(
			service.queryWall({ cursor: null, limit: 0, direction: "oldestFirst" }),
		).rejects.toMatchObject({
			code: "invalidLimit",
			message: "The requested wall page is not valid.",
		});
		await expect(
			service.queryWall({ cursor: null, limit: 251, direction: "oldestFirst" }),
		).rejects.toMatchObject({ code: "invalidLimit" });
	});

	it("clones query results and delivered updates", async () => {
		const service = createInMemoryPhotoService({ wallAssets: fixtureAssets });
		const page = await service.queryWall({
			cursor: null,
			limit: 10,
			direction: "oldestFirst",
		});
		const firstItem = page.items[0];
		if (!firstItem) throw new Error("expected a fixture item");
		firstItem.displayName = "mutated by caller";
		const retained = await service.queryWall({
			cursor: null,
			limit: 10,
			direction: "oldestFirst",
		});
		expect(retained.items[0]?.displayName).toBe("asset-a.jpg");

		const first: WallUpdate[] = [];
		const second: WallUpdate[] = [];
		service.watchWallUpdates((update) => {
			first.push(update);
			if (update.kind === "progress") update.progress.enriched = 999;
		});
		service.watchWallUpdates((update) => second.push(update));
		service.emitForTest(sampleProgressUpdate);
		expect(first[0]).toMatchObject({
			kind: "progress",
			progress: { enriched: 999 },
		});
		expect(second[0]).toEqual(sampleProgressUpdate);
	});

	it("publishes geometry before derivatives and settles once for concurrent finishes", async () => {
		const fixture: InMemoryWallFixture = {
			...fixtureAsset,
			wallThumbnail: thumbnailReference,
			derivativeUrls: { wallThumbnail: "https://fixtures.invalid/thumb-a.jpg" },
		};
		const service = createInMemoryPhotoService({
			wallAssets: [fixture],
			geometryDelayMs: 0,
			thumbnailDelayMs: 0,
			metadataDelayMs: 0,
		});
		const events: WallUpdate[] = [];
		service.watchWallUpdates((update) => events.push(update));

		const start = service.startFixtureScan();
		const firstFinish = service.finishFixtureScan();
		await start;
		expect(events.map((event) => event.kind)).toEqual(["catalogBatch"]);
		const secondFinish = service.finishFixtureScan();
		await Promise.all([firstFinish, secondFinish]);
		expect(events.map((event) => event.kind)).toEqual([
			"catalogBatch",
			"derivativesReady",
			"metadataSettled",
		]);
		expect(service.derivativeUrl(thumbnailReference)).toBe(
			"https://fixtures.invalid/thumb-a.jpg",
		);
	});
});
