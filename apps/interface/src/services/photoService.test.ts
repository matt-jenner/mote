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
});
