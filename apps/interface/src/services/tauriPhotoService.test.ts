import { describe, expect, it } from "vitest";
import { PhotoServiceError, type WallUpdate } from "./photoService";
import {
	type ChannelFactory,
	createTauriPhotoService,
	type InvokeCommand,
} from "./tauriPhotoService";

class FakeChannel<T> {
	private handler: (response: T) => void;
	readonly serialized = "__CHANNEL__:fake";

	constructor(listener: (response: T) => void) {
		this.handler = listener;
	}

	set onmessage(handler: (response: T) => void) {
		this.handler = handler;
	}

	get onmessage(): (response: T) => void {
		return this.handler;
	}

	toJSON(): string {
		return this.serialized;
	}

	emit(response: T): void {
		this.handler(response);
	}
}

const sampleCatalogBatch: WallUpdate = {
	kind: "catalogBatch",
	assets: [
		{
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
		},
	],
	orderState: "provisional",
	progress: { discovered: 1, shaped: 1, enriched: 0, total: 1 },
};

const sampleProgressUpdate: WallUpdate = {
	kind: "progress",
	progress: { discovered: 1, shaped: 1, enriched: 1, total: 1 },
};

const recordingInvoke =
	(
		calls: Array<[string, Record<string, unknown> | undefined]>,
	): InvokeCommand =>
	async <T>(command: string, args?: Record<string, unknown>) => {
		calls.push([command, args]);
		return undefined as T;
	};

describe("Tauri PhotoService", () => {
	it("uses only the three checkpoint commands", async () => {
		const responses: unknown[] = [
			{ settings: { appearance: "system" }, activeSource: null },
			{ kind: "cancelled" },
			{ settings: { appearance: "dark" }, activeSource: null },
		];
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			calls.push([command, args]);
			return responses.shift() as T;
		};
		const service = createTauriPhotoService(invoke);

		await service.getBootstrapState();
		await service.chooseFolder();
		await service.updateAppearance("dark");

		expect(calls).toEqual([
			["get_bootstrap_state", undefined],
			["choose_folder", undefined],
			["update_appearance", { appearance: "dark" }],
		]);
	});

	it("converts a known native command failure to PhotoServiceError", async () => {
		const invoke: InvokeCommand = async () => {
			throw {
				code: "folderNotDirectory",
				message: "Choose a folder, not a file.",
			};
		};
		const service = createTauriPhotoService(invoke);

		const error = await service
			.chooseFolder()
			.catch((reason: unknown) => reason);

		expect(error).toBeInstanceOf(PhotoServiceError);
		expect(error).toMatchObject({
			code: "folderNotDirectory",
			message: "Choose a folder, not a file.",
		});
	});

	it("maps unknown native rejections to a fixed path-free internal error", async () => {
		const nativeDetail = "SQLite failed near /Users/private/Photo Library";
		for (const rejection of [
			new Error(nativeDetail),
			{ code: "unexpectedNativeFailure", message: nativeDetail },
			{ code: "folderNotDirectory", message: nativeDetail },
		]) {
			const invoke: InvokeCommand = async () => {
				throw rejection;
			};
			const service = createTauriPhotoService(invoke);

			const error = await service
				.getBootstrapState()
				.catch((reason: unknown) => reason);

			expect(error).toBeInstanceOf(PhotoServiceError);
			expect(error).toMatchObject({
				code: "internal",
				message: "Photo Viewer could not complete that request.",
			});
			expect((error as Error).message).not.toContain("/Users/private");
		}
	});

	it("maps wall operations and ordered channel updates", async () => {
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const received: WallUpdate[] = [];
		const channels: FakeChannel<WallUpdate>[] = [];
		const channelFactory: ChannelFactory = (listener) => {
			const channel = new FakeChannel(listener);
			channels.push(channel);
			return channel;
		};
		const service = createTauriPhotoService(
			recordingInvoke(calls),
			channelFactory,
		);
		const stop = service.watchWallUpdates((event) => received.push(event));

		await service.queryWall({
			cursor: null,
			limit: 100,
			direction: "oldestFirst",
		});
		await service.requestDerivatives({
			assetIds: ["asset-a"],
			priority: "visible",
		});
		await service.setWallInteraction(true);

		expect(calls.map(([name]) => name)).toEqual([
			"watch_wall_updates",
			"query_wall",
			"request_derivatives",
			"set_wall_interaction",
		]);
		expect(
			service.derivativeUrl({
				assetId: "asset-a",
				kind: "wallThumbnail",
				key: "abc",
			}),
		).toBe("photo-derivative://localhost/asset-a/wallThumbnail/abc");
		channels[0]?.emit(sampleCatalogBatch);
		expect(received).toEqual([sampleCatalogBatch]);
		stop();
		channels[0]?.emit(sampleProgressUpdate);
		expect(received).toEqual([sampleCatalogBatch]);
	});
});
