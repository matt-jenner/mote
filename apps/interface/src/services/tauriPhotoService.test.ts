import { describe, expect, it, vi } from "vitest";
import { createPickListStore } from "../picks/usePickList";
import { createInMemoryPhotoService } from "./inMemoryPhotoService";
import {
	type CopyProgress,
	type CopyResult,
	type DerivativeRequest,
	PhotoServiceError,
	type WallUpdate,
} from "./photoService";
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
	selectionId: "sample-selection",
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
			rating: null,
		},
	],
	orderState: "provisional",
	generation: 1,
	progress: { discovered: 1, shaped: 1, enriched: 0, total: 1 },
};

const sampleProgressUpdate: WallUpdate = {
	kind: "progress",
	selectionId: "sample-selection",
	generation: 1,
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
	it("distinguishes selection dismissal from worker cancellation and forwards cancel", async () => {
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		for (const kind of ["selectionCancelled", "copyCancelled"] as const) {
			const service = createTauriPhotoService(
				async <T>(command: string, args?: Record<string, unknown>) => {
					calls.push([command, args]);
					return { kind } as T;
				},
				undefined,
				(listener) => new FakeChannel(listener),
			);
			expect(await service.copyPickedOriginals(null, () => {})).toEqual({
				kind,
			});
			await service.cancelOriginalCopy();
			expect(calls.at(-1)).toEqual(["cancel_original_copy", undefined]);
		}
	});
	it("owns one pending copy globally and preserves partial state when retry is cancelled", async () => {
		const memory = createInMemoryPhotoService();
		let release!: (result: CopyResult) => void;
		let progress!: (event: CopyProgress) => void;
		const ids: Array<readonly string[] | null> = [];
		const store = createPickListStore({
			...memory,
			getPicks: () => ({
				revision: 0,
				persistenceError: null,
				items: ["one", "two"].map((assetId) => ({
					assetId,
					sourceFolderId: "folder",
					sourceLabel: "Folder",
					asset: null,
				})),
			}),
			copyPickedOriginals: (batch, listener) => {
				ids.push(batch);
				progress = listener;
				return new Promise((resolve) => {
					release = resolve;
				});
			},
		});
		const operation = store.getState().copyOriginals();
		void store.getState().copyOriginals();
		expect(ids).toEqual([null]);
		expect(store.getState().copy.phase).toBe("choosing");
		progress({
			completed: 1,
			total: 2,
			item: {
				assetId: "one",
				status: "copied",
				destinationName: "one.jpg",
				errorCode: null,
			},
		});
		expect(store.getState().copy).toMatchObject({
			phase: "copying",
			completed: 1,
			total: 2,
		});
		release({
			kind: "complete",
			copiedCount: 1,
			failedCount: 1,
			warningCode: null,
			items: [
				{
					assetId: "one",
					status: "copied",
					destinationName: "one.jpg",
					errorCode: null,
				},
				{
					assetId: "two",
					status: "failed",
					destinationName: null,
					errorCode: "source_unavailable",
				},
			],
		});
		await operation;
		expect(store.getState().copy).toMatchObject({
			phase: "partial",
			failedAssetIds: ["two"],
			copiedCount: 1,
		});
		expect(store.getState().announcement).toBe("Copied 1 of 2");
		const retry = store.getState().copyOriginals();
		expect(ids).toEqual([null, ["two"]]);
		release({ kind: "selectionCancelled" });
		await retry;
		expect(store.getState().copy).toMatchObject({
			phase: "partial",
			failedAssetIds: ["two"],
			copiedCount: 1,
		});
	});
	it("copies a frozen ID list with ordered channel progress and opens the last destination", async () => {
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const received: CopyProgress[] = [];
		const item = {
			assetId: "one",
			status: "copied" as const,
			destinationName: "one.jpg",
			errorCode: null,
		};
		const result = {
			kind: "complete",
			items: [item],
			copiedCount: 1,
			failedCount: 0,
			warningCode: null,
		};
		const service = createTauriPhotoService(
			async <T>(command: string, args?: Record<string, unknown>) => {
				calls.push([command, args]);
				if (command === "copy_picked_originals") {
					const channel = args?.onEvent as FakeChannel<CopyProgress>;
					channel.emit({ completed: 0, total: 1, item: null });
					channel.emit({ completed: 1, total: 1, item });
					return result as T;
				}
				return undefined as T;
			},
			undefined,
			(listener) => new FakeChannel(listener),
		);
		const ids = ["one"];
		const copying = service.copyPickedOriginals(ids, (event) =>
			received.push(event),
		);
		ids.push("later-pick");
		expect(await copying).toEqual(result);
		expect(calls[0]?.[1]?.assetIds).toEqual(["one"]);
		expect(received).toEqual([
			{ completed: 0, total: 1, item: null },
			{ completed: 1, total: 1, item },
		]);
		await service.showLastCopyDestination();
		expect(calls[1]).toEqual(["show_last_copy_destination", undefined]);
	});

	it("preserves picker cancellation and bounds copy errors without leaking native paths", async () => {
		const cancelled = createTauriPhotoService(
			async <T>() => ({ kind: "selectionCancelled" }) as T,
			undefined,
			(listener) => new FakeChannel(listener),
		);
		expect(await cancelled.copyPickedOriginals(null, () => {})).toEqual({
			kind: "selectionCancelled",
		});
		for (const [code, message] of [
			["copyInProgress", "An original copy is already in progress."],
			[
				"copyDestinationIsSource",
				"Choose a destination outside your source folders.",
			],
			["copyDestinationUnavailable", "The copy destination is unavailable."],
			["copyDestinationMissing", "The destination folder no longer exists."],
			["copyPreparationFailed", "Mote could not prepare these originals."],
		] as const) {
			const service = createTauriPhotoService(
				async () => {
					throw { code, message };
				},
				undefined,
				(listener) => new FakeChannel(listener),
			);
			await expect(
				service.copyPickedOriginals(null, () => {}),
			).rejects.toMatchObject({ code, message });
		}
		const unsafe = createTauriPhotoService(
			async () => {
				throw {
					code: "copyDestinationUnavailable",
					message: "/private/secret",
				};
			},
			undefined,
			(listener) => new FakeChannel(listener),
		);
		await expect(unsafe.showLastCopyDestination()).rejects.toMatchObject({
			code: "internal",
			message: "Mote could not complete that request.",
		});
	});
	it("keeps native folder selection isolated from hosted browser methods", async () => {
		const service = createTauriPhotoService(recordingInvoke([]));

		expect(service.capabilities).toEqual({
			chooseFolder: true,
			folderSelection: "native",
			locateFolder: false,
			originalAction: "copy",
		});
		expect(service.folderBrowserState()).toEqual({
			breadcrumbs: [],
			initialPath: "",
		});
		expect(service.initialSortDirection()).toBe("oldestFirst");
		service.rememberSortDirection("newestFirst");
		expect(service.initialSortDirection()).toBe("newestFirst");
		await expect(service.listFolders("Trips")).rejects.toMatchObject({
			code: "unsupportedCapability",
			message: "This host uses its system folder picker.",
		});
		await expect(service.selectFolder("Trips")).rejects.toMatchObject({
			code: "unsupportedCapability",
			message: "This host uses its system folder picker.",
		});
	});

	it("maps persisted settings to their native commands", async () => {
		const responses: unknown[] = [
			{
				settings: {
					appearance: "system",
					galleryScope: "includeSubfolders",
				},
				activeSource: null,
			},
			{ kind: "cancelled" },
			{
				settings: { appearance: "dark", galleryScope: "includeSubfolders" },
				activeSource: null,
			},
			{
				settings: { appearance: "dark", galleryScope: "currentFolder" },
				activeSource: null,
			},
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

		await expect(service.getBootstrapState()).resolves.toMatchObject({
			accentColor: "system",
		});
		await service.chooseFolder();
		await service.updateAppearance("dark");
		await service.updateGalleryScope("currentFolder");

		expect(calls).toEqual([
			["get_bootstrap_state", undefined],
			["choose_folder", undefined],
			["update_appearance", { appearance: "dark" }],
			["update_gallery_scope", { scope: "currentFolder" }],
		]);
	});

	it("maps pick snapshots, mutations, and derivative requests to native commands", async () => {
		const nativeSnapshots = [
			{
				revision: 4,
				items: [
					{
						assetId: "asset-a",
						sourceFolderId: "folder-a",
						sourceLabel: "Family",
						asset: null,
					},
				],
			},
			{ revision: 5, items: [] },
			{ revision: 6, items: [] },
			{ revision: 7, items: [] },
			{ revision: 8, items: [] },
			{ revision: 9, items: [] },
		];
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			calls.push([command, args]);
			if (command === "request_pick_derivatives") return undefined as T;
			return nativeSnapshots.shift() as T;
		};
		const service = createTauriPhotoService(invoke);
		const revisions: number[] = [];
		const stop = service.watchPicks((snapshot) => {
			revisions.push(snapshot.revision);
		});

		await expect(service.loadPicks()).resolves.toMatchObject({
			revision: 4,
			persistenceError: null,
		});
		await service.addPick({
			assetId: "asset-b",
			sourceFolderId: "folder-b",
			sourceLabel: "Trips",
		});
		await service.removePick("asset-a");
		await service.clearPicks();
		await service.restorePicks([
			{
				assetId: "asset-a",
				sourceFolderId: "folder-a",
				sourceLabel: "Family",
			},
		]);
		await service.loadPicks();
		await service.requestPickDerivatives({
			assetIds: ["asset-a", "asset-b"],
			priority: "visible",
			kind: "screenPreview",
		});
		stop();

		expect(service.getPicks()).toMatchObject({
			revision: 9,
			persistenceError: null,
		});
		expect(revisions).toEqual([4, 5, 6, 7, 8, 9]);
		expect(calls).toEqual([
			["list_photo_picks", undefined],
			["add_photo_pick", { assetId: "asset-b", sourceFolderId: "folder-b" }],
			["remove_photo_pick", { assetId: "asset-a" }],
			["clear_photo_picks", undefined],
			[
				"restore_photo_picks",
				{
					references: [
						{
							assetId: "asset-a",
							sourceFolderId: "folder-a",
							sourceLabel: "Family",
						},
					],
				},
			],
			["list_photo_picks", undefined],
			[
				"request_pick_derivatives",
				{
					request: {
						assetIds: ["asset-a", "asset-b"],
						priority: "visible",
						kind: "screenPreview",
					},
				},
			],
		]);
		expect(service.originalDownloadUrl("asset-a")).toBeNull();
	});

	it("restores more than 250 picks tail-first and publishes only the complete order", async () => {
		const references = Array.from({ length: 501 }, (_, index) => ({
			assetId: `asset-${index}`,
			sourceFolderId: `folder-${index}`,
			sourceLabel: `Source ${index}`,
		}));
		let nativeItems: Array<(typeof references)[number] & { asset: null }> = [];
		let revision = 0;
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			calls.push([command, args]);
			const chunk = (args?.references ?? []) as typeof references;
			nativeItems = [
				...chunk.map((reference) => ({ ...reference, asset: null })),
				...nativeItems,
			];
			revision += 1;
			return { revision, items: structuredClone(nativeItems) } as T;
		};
		const service = createTauriPhotoService(invoke);
		const published: number[] = [];
		service.watchPicks((snapshot) => published.push(snapshot.revision));

		const restored = await service.restorePicks(references);

		expect(
			calls.map(([command, args]) => [
				command,
				((args?.references ?? []) as typeof references).map(
					(reference) => reference.assetId,
				),
			]),
		).toEqual([
			["restore_photo_picks", ["asset-500"]],
			[
				"restore_photo_picks",
				Array.from({ length: 250 }, (_, index) => `asset-${index + 250}`),
			],
			[
				"restore_photo_picks",
				Array.from({ length: 250 }, (_, index) => `asset-${index}`),
			],
		]);
		expect(
			calls.every(
				([, args]) =>
					((args?.references ?? []) as typeof references).length <= 250,
			),
		).toBe(true);
		expect(restored.items.map((item) => item.assetId)).toEqual(
			references.map((reference) => reference.assetId),
		);
		expect(published).toEqual([3]);
	});

	it("resyncs after a later restore chunk fails without publishing partial responses", async () => {
		const references = Array.from({ length: 251 }, (_, index) => ({
			assetId: `asset-${index}`,
			sourceFolderId: "folder-a",
			sourceLabel: "Family",
		}));
		const calls: string[] = [];
		let restoreCalls = 0;
		const invoke: InvokeCommand = async <T>(command: string) => {
			calls.push(command);
			if (command === "list_photo_picks") {
				return {
					revision: 1,
					items: [{ ...references[250], asset: null }],
				} as T;
			}
			restoreCalls += 1;
			if (restoreCalls === 2) {
				throw {
					code: "assetNotFound",
					message: "That photo is no longer available.",
				};
			}
			return {
				revision: 1,
				items: [{ ...references[250], asset: null }],
			} as T;
		};
		const service = createTauriPhotoService(invoke);
		const published: string[][] = [];
		service.watchPicks((snapshot) =>
			published.push(snapshot.items.map((item) => item.assetId)),
		);

		await expect(service.restorePicks(references)).rejects.toMatchObject({
			code: "assetNotFound",
			message: "That photo is no longer available.",
		});

		expect(calls).toEqual([
			"restore_photo_picks",
			"restore_photo_picks",
			"list_photo_picks",
		]);
		expect(service.getPicks().items.map((item) => item.assetId)).toEqual([
			"asset-250",
		]);
		expect(published).toEqual([["asset-250"]]);
	});

	it("requests more than 250 pick derivatives in sequential bounded chunks", async () => {
		const calls: DerivativeRequest[] = [];
		let active = 0;
		let maximumActive = 0;
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			if (command !== "request_pick_derivatives") return undefined as T;
			active += 1;
			maximumActive = Math.max(maximumActive, active);
			calls.push(structuredClone(args?.request as DerivativeRequest));
			await Promise.resolve();
			active -= 1;
			return undefined as T;
		};
		const service = createTauriPhotoService(invoke);
		const assetIds = Array.from(
			{ length: 501 },
			(_, index) => `asset-${index}`,
		);

		await service.requestPickDerivatives({
			assetIds,
			priority: "nearViewport",
			kind: "wallThumbnail",
		});

		expect(calls.map((request) => request.assetIds)).toEqual([
			assetIds.slice(0, 250),
			assetIds.slice(250, 500),
			assetIds.slice(500),
		]);
		expect(calls.every((request) => request.assetIds.length <= 250)).toBe(true);
		expect(maximumActive).toBe(1);
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

	it("keeps derivative-unavailable failures on the bounded native allowlist", async () => {
		const invoke: InvokeCommand = async () => {
			throw {
				code: "derivativeUnavailable",
				message: "Some requested previews could not be generated.",
			};
		};
		const service = createTauriPhotoService(invoke);

		const error = await service
			.requestDerivatives({
				assetIds: ["asset-a"],
				priority: "visible",
				kind: "screenPreview",
			})
			.catch((reason: unknown) => reason);

		expect(error).toBeInstanceOf(PhotoServiceError);
		expect(error).toMatchObject({
			code: "derivativeUnavailable",
			message: "Some requested previews could not be generated.",
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
				message: "Mote could not complete that request.",
			});
			expect((error as Error).message).not.toContain("/Users/private");
		}
	});

	it("maps wall operations and ordered channel updates", async () => {
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const firstReceived: WallUpdate[] = [];
		const secondReceived: WallUpdate[] = [];
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
		const stopFirst = service.watchWallUpdates((event) =>
			firstReceived.push(event),
		);
		const stopSecond = service.watchWallUpdates((event) =>
			secondReceived.push(event),
		);

		await service.queryWall({
			cursor: null,
			limit: 100,
			direction: "oldestFirst",
		});
		await service.requestDerivatives({
			assetIds: ["asset-a"],
			priority: "visible",
			kind: "screenPreview",
		});
		await service.setWallInteraction(true);

		expect(calls).toEqual([
			["watch_wall_updates", { onEvent: channels[0] }],
			["watch_wall_updates", { onEvent: channels[1] }],
			[
				"query_wall",
				{ request: { cursor: null, limit: 100, direction: "oldestFirst" } },
			],
			[
				"request_derivatives",
				{
					request: {
						assetIds: ["asset-a"],
						priority: "visible",
						kind: "screenPreview",
					},
				},
			],
			["set_wall_interaction", { active: true }],
		]);
		expect(channels).toHaveLength(2);
		expect(JSON.stringify(calls[0]?.[1])).toBe(
			'{"onEvent":"__CHANNEL__:fake"}',
		);
		expect(channels.map((channel) => channel.toJSON())).toEqual([
			"__CHANNEL__:fake",
			"__CHANNEL__:fake",
		]);
		expect(
			service.derivativeUrl({
				assetId: "asset-a",
				kind: "wallThumbnail",
				key: "abc",
			}),
		).toBe("photo-derivative://localhost/asset-a/wallThumbnail/abc");
		channels[0]?.emit(sampleCatalogBatch);
		channels[1]?.emit(sampleCatalogBatch);
		expect(firstReceived).toEqual([
			{ kind: "resyncRequired", selectionId: "" },
			sampleCatalogBatch,
		]);
		expect(secondReceived).toEqual([
			{ kind: "resyncRequired", selectionId: "" },
			sampleCatalogBatch,
		]);
		stopFirst();
		channels[0]?.emit(sampleProgressUpdate);
		channels[1]?.emit(sampleProgressUpdate);
		expect(firstReceived).toEqual([
			{ kind: "resyncRequired", selectionId: "" },
			sampleCatalogBatch,
		]);
		expect(secondReceived).toEqual([
			{ kind: "resyncRequired", selectionId: "" },
			sampleCatalogBatch,
			sampleProgressUpdate,
		]);
		stopSecond();
	});

	it("turns wall-update registration failure into a bounded resync", async () => {
		const invoke: InvokeCommand = async <T>(command: string) => {
			if (command === "watch_wall_updates") throw new Error("listen failed");
			return undefined as T;
		};
		const received: WallUpdate[] = [];
		const service = createTauriPhotoService(
			invoke,
			(listener) => new FakeChannel(listener),
		);

		const stop = service.watchWallUpdates((update) => received.push(update));
		await new Promise<void>((resolve) => setTimeout(resolve, 0));

		expect(received).toEqual([{ kind: "resyncRequired", selectionId: "" }]);
		stop();
	});

	it("resyncs exactly once after the first successful wall-update registration", async () => {
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const received: WallUpdate[] = [];
		const service = createTauriPhotoService(
			async <T>(command: string, args?: Record<string, unknown>) => {
				calls.push([command, args]);
				return (
					command === "watch_wall_updates" ? "initial-subscription" : undefined
				) as T;
			},
			(listener) => new FakeChannel(listener),
		);

		const stop = service.watchWallUpdates((update) => received.push(update));
		await new Promise<void>((resolve) => setTimeout(resolve, 0));

		expect(received).toEqual([{ kind: "resyncRequired", selectionId: "" }]);
		stop();
		expect(calls).toContainEqual([
			"unwatch_wall_updates",
			{ subscriptionId: "initial-subscription" },
		]);
	});

	it("keeps registration failure delivery synchronous with unsubscribe", async () => {
		const invoke: InvokeCommand = async <T>(command: string) => {
			if (command === "watch_wall_updates") throw new Error("listen failed");
			return undefined as T;
		};
		const received: WallUpdate[] = [];
		const service = createTauriPhotoService(
			invoke,
			(listener) => new FakeChannel(listener),
		);

		const stop = service.watchWallUpdates((update) => received.push(update));
		stop();
		await new Promise<void>((resolve) => setTimeout(resolve, 0));

		expect(received).toEqual([]);
	});

	it("retries failed registrations with bounded fake-timer backoff", async () => {
		vi.useFakeTimers();
		try {
			const calls: string[] = [];
			let registrationAttempts = 0;
			const channels: FakeChannel<WallUpdate>[] = [];
			const invoke: InvokeCommand = async <T>(command: string) => {
				calls.push(command);
				if (command === "watch_wall_updates") {
					registrationAttempts += 1;
					if (registrationAttempts < 4) throw new Error("listen failed");
					return "wall-subscription-recovered" as T;
				}
				return undefined as T;
			};
			const received: WallUpdate[] = [];
			const service = createTauriPhotoService(invoke, (listener) => {
				const channel = new FakeChannel(listener);
				channels.push(channel);
				return channel;
			});
			const stop = service.watchWallUpdates((update) => received.push(update));
			await vi.runAllTicks();
			await vi.advanceTimersByTimeAsync(0);
			expect(calls).toEqual(["watch_wall_updates"]);
			expect(received).toHaveLength(1);

			await vi.advanceTimersByTimeAsync(99);
			expect(calls).toEqual(["watch_wall_updates"]);
			await vi.advanceTimersByTimeAsync(1);
			await vi.runAllTicks();
			expect(calls).toEqual(["watch_wall_updates", "watch_wall_updates"]);
			expect(received).toHaveLength(2);

			await vi.advanceTimersByTimeAsync(199);
			expect(calls).toHaveLength(2);
			await vi.advanceTimersByTimeAsync(1);
			await vi.runAllTicks();
			expect(calls).toHaveLength(3);
			expect(received).toHaveLength(3);

			await vi.advanceTimersByTimeAsync(299);
			expect(calls).toHaveLength(3);
			channels[0]?.emit(sampleCatalogBatch);
			expect(received).toHaveLength(4);
			await vi.advanceTimersByTimeAsync(1);
			await vi.runAllTicks();
			expect(calls).toHaveLength(4);
			expect(received).toEqual([
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
				sampleCatalogBatch,
				{ kind: "resyncRequired", selectionId: "" },
			]);
			channels[0]?.emit(sampleProgressUpdate);
			expect(received).toEqual([
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
				sampleCatalogBatch,
				{ kind: "resyncRequired", selectionId: "" },
				sampleProgressUpdate,
			]);

			stop();
			await vi.advanceTimersByTimeAsync(10_000);
			expect(
				calls.filter((command) => command === "watch_wall_updates"),
			).toHaveLength(4);
			expect(calls.at(-1)).toBe("unwatch_wall_updates");
		} finally {
			vi.useRealTimers();
		}
	});

	it("unwatches a recovered registration when resync stops synchronously", async () => {
		vi.useFakeTimers();
		try {
			let attempts = 0;
			const calls: Array<[string, Record<string, unknown> | undefined]> = [];
			const channels: FakeChannel<WallUpdate>[] = [];
			const invoke: InvokeCommand = async <T>(
				command: string,
				args?: Record<string, unknown>,
			) => {
				calls.push([command, args]);
				if (command === "watch_wall_updates") {
					attempts += 1;
					if (attempts === 1) throw new Error("listen failed");
					return "recovered-subscription" as T;
				}
				return undefined as T;
			};
			let stop: () => void = () => undefined;
			const received: WallUpdate[] = [];
			const service = createTauriPhotoService(invoke, (listener) => {
				const channel = new FakeChannel(listener);
				channels.push(channel);
				return channel;
			});
			stop = service.watchWallUpdates((update) => {
				received.push(update);
				if (update.kind === "resyncRequired" && attempts === 2) stop();
			});

			await vi.runAllTicks();
			await vi.advanceTimersByTimeAsync(100);
			await vi.runAllTicks();

			expect(attempts).toBe(2);
			expect(received).toEqual([
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
			]);
			expect(calls.map(([command]) => command)).toEqual([
				"watch_wall_updates",
				"watch_wall_updates",
				"unwatch_wall_updates",
			]);
			expect(calls[2]?.[1]).toEqual({
				subscriptionId: "recovered-subscription",
			});
			channels[0]?.emit(sampleProgressUpdate);
			await vi.advanceTimersByTimeAsync(10_000);
			expect(received).toHaveLength(2);
			expect(
				calls.filter(([command]) => command === "unwatch_wall_updates"),
			).toHaveLength(1);
		} finally {
			vi.useRealTimers();
		}
	});

	it("does not retry or leak when the recovered resync listener throws", async () => {
		vi.useFakeTimers();
		try {
			let attempts = 0;
			const calls: string[] = [];
			const invoke: InvokeCommand = async <T>(command: string) => {
				calls.push(command);
				if (command === "watch_wall_updates") {
					attempts += 1;
					if (attempts === 1) throw new Error("listen failed");
					return "throwing-subscription" as T;
				}
				return undefined as T;
			};
			const received: WallUpdate[] = [];
			const service = createTauriPhotoService(
				invoke,
				(listener) => new FakeChannel(listener),
			);
			service.watchWallUpdates((update) => {
				received.push(update);
				if (update.kind === "resyncRequired" && attempts === 2) {
					throw new Error("consumer failed");
				}
			});

			await vi.runAllTicks();
			await vi.advanceTimersByTimeAsync(100);
			await vi.runAllTicks();
			await vi.advanceTimersByTimeAsync(10_000);

			expect(received).toEqual([
				{ kind: "resyncRequired", selectionId: "" },
				{ kind: "resyncRequired", selectionId: "" },
			]);
			expect(calls).toEqual([
				"watch_wall_updates",
				"watch_wall_updates",
				"unwatch_wall_updates",
			]);
		} finally {
			vi.useRealTimers();
		}
	});

	it("unwatches a registration that resolves after synchronous unsubscribe", async () => {
		let resolveWatch!: (subscriptionId: string) => void;
		const watchPromise = new Promise<string>((resolve) => {
			resolveWatch = resolve;
		});
		const calls: Array<[string, Record<string, unknown> | undefined]> = [];
		const invoke: InvokeCommand = async <T>(
			command: string,
			args?: Record<string, unknown>,
		) => {
			calls.push([command, args]);
			if (command === "watch_wall_updates") return watchPromise as T;
			return undefined as T;
		};
		const service = createTauriPhotoService(
			invoke,
			(listener) => new FakeChannel(listener),
		);

		const stop = service.watchWallUpdates(() => undefined);
		stop();
		resolveWatch("wall-subscription-race");
		await new Promise<void>((resolve) => setTimeout(resolve, 0));

		expect(calls[0]?.[0]).toBe("watch_wall_updates");
		expect(JSON.stringify(calls[0]?.[1])).toBe(
			'{"onEvent":"__CHANNEL__:fake"}',
		);
		expect(calls[1]).toEqual([
			"unwatch_wall_updates",
			{ subscriptionId: "wall-subscription-race" },
		]);
	});
});

it("ignores an older native check snapshot after a newer removal", async () => {
	const { emptySavedFolders } = await import("../folders/savedFolders");
	const entry = {
		id: "saved-a",
		folderId: "folder-a",
		name: "A",
		displayPath: "/A",
		customLabel: null,
	};
	const old = {
		...emptySavedFolders(),
		entries: [entry],
		activeEntryId: entry.id,
		revision: 1,
	};
	const removed = {
		...emptySavedFolders(),
		hasOpenedFolder: true,
		revision: 2,
	};
	let release!: (value: unknown) => void;
	const invoke = async <T>(command: string): Promise<T> => {
		if (command === "check_saved_folders")
			return new Promise((resolve) => {
				release = resolve as (value: unknown) => void;
			});
		return {
			settings: { appearance: "system", galleryScope: "includeSubfolders" },
			activeSource: null,
			savedFolders: command === "remove_saved_folder" ? removed : old,
		} as T;
	};
	const service = createTauriPhotoService(invoke);
	await service.getBootstrapState();
	const checking = service.checkSavedFolders([entry.id]);
	await service.removeSavedFolder(entry.id);
	release({
		settings: { appearance: "system", galleryScope: "includeSubfolders" },
		activeSource: null,
		savedFolders: old,
	});
	await checking;
	expect(service.getSavedFolders().entries).toEqual([]);
	expect(service.getSavedFolders().activeEntryId).toBeNull();
});

it("keeps the wall cleared when a newer check overtakes the removal response", async () => {
	const { emptySavedFolders } = await import("../folders/savedFolders");
	const entry = {
		id: "saved-a",
		folderId: "folder-a",
		name: "A",
		displayPath: "/A",
		customLabel: null,
	};
	const settings = { appearance: "system", galleryScope: "includeSubfolders" };
	const initial = {
		settings,
		activeSource: { id: "A", selectionId: "selection-a", displayName: "A" },
		savedFolders: {
			...emptySavedFolders(),
			entries: [entry],
			activeEntryId: entry.id,
			revision: 1,
		},
	};
	const removed = {
		settings,
		activeSource: null,
		savedFolders: {
			...emptySavedFolders(),
			hasOpenedFolder: true,
			revision: 2,
		},
	};
	let release!: (value: unknown) => void;
	const invoke = async <T>(command: string): Promise<T> => {
		if (command === "remove_saved_folder")
			return new Promise((resolve) => {
				release = resolve as (value: unknown) => void;
			});
		if (command === "check_saved_folders")
			return {
				...removed,
				savedFolders: { ...removed.savedFolders, revision: 3 },
			} as T;
		return initial as T;
	};
	const service = createTauriPhotoService(invoke);
	await service.getBootstrapState();
	const removing = service.removeSavedFolder(entry.id);
	await service.checkSavedFolders([]);
	release(removed);
	const result = await removing;
	expect(result.activeSource).toBeNull();
	expect(result.savedFolders.activeEntryId).toBeNull();
	expect(result.savedFolders.revision).toBe(3);
});
