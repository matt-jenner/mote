import { afterEach, describe, expect, it, vi } from "vitest";
import { emptySavedFolders } from "../folders/savedFolders";
import {
	hostedClientStorageKey,
	hostedPreferencesStorageKey,
} from "./browserPreferences";
import {
	createHttpPhotoService,
	type EventSourceLike,
} from "./httpPhotoService";
import { PhotoServiceError, type WallUpdate } from "./photoService";

class MemoryStorage implements Storage {
	private readonly values = new Map<string, string>();
	readonly reads: string[] = [];
	readonly writes: Array<[string, string]> = [];

	get length(): number {
		return this.values.size;
	}

	clear(): void {
		this.values.clear();
	}

	getItem(key: string): string | null {
		this.reads.push(key);
		return this.values.get(key) ?? null;
	}

	peek(key: string): string | null {
		return this.values.get(key) ?? null;
	}

	key(index: number): string | null {
		return [...this.values.keys()][index] ?? null;
	}

	removeItem(key: string): void {
		this.values.delete(key);
	}

	setItem(key: string, value: string): void {
		this.values.set(key, value);
		this.writes.push([key, value]);
	}
}

type EventListener = (event: MessageEvent<string>) => void;

class FakeEventSource implements EventSourceLike {
	readonly listeners = new Map<string, Set<EventListener>>();
	closed = false;
	onopen: ((event: Event) => void) | null = null;
	onerror: ((event: Event) => void) | null = null;

	constructor(readonly url: string) {}

	addEventListener(type: string, listener: EventListener): void {
		const listeners = this.listeners.get(type) ?? new Set<EventListener>();
		listeners.add(listener);
		this.listeners.set(type, listeners);
	}

	removeEventListener(type: string, listener: EventListener): void {
		this.listeners.get(type)?.delete(listener);
	}

	close(): void {
		this.closed = true;
	}

	open(): void {
		this.onopen?.(new Event("open"));
	}

	fail(): void {
		this.onerror?.(new Event("error"));
	}

	emit(update: unknown, lastEventId: string): void {
		this.emitRaw(JSON.stringify(update), lastEventId);
	}

	emitRaw(data: string, lastEventId: string): void {
		const event = {
			data,
			lastEventId,
		} as MessageEvent<string>;
		for (const listener of this.listeners.get("wallUpdate") ?? []) {
			listener(event);
		}
	}
}

interface RecordedRequest {
	url: string;
	init: RequestInit | undefined;
}

const bootstrapResponse = {
	capabilities: {
		folderBrowser: true,
		video: false,
		originalDownloads: false,
	},
	sourceAvailable: true,
};

const selectionSummary = {
	id: "selection-a",
	sourceId: "source-a",
	displayName: "Iceland",
	breadcrumbs: [
		{ name: "Trips", path: "Trips" },
		{ name: "Iceland", path: "Trips/Iceland" },
	],
	availability: "available",
};

const secondSelectionSummary = {
	id: "selection-b",
	sourceId: "source-b",
	displayName: "Alps",
	breadcrumbs: [
		{ name: "Trips", path: "Trips" },
		{ name: "Alps", path: "Trips/Alps" },
	],
	availability: "rootOffline",
};

const representativeAsset = {
	id: "asset-a",
	displayName: "Aurora.jpg",
	mediaKind: "jpeg",
	provisionalOrder: 7,
	capturedAtUtc: "2025-01-02T03:04:05Z",
	dateState: "settled",
	width: 1600,
	height: 900,
	representativeRgb: 0x123456,
	shapeState: "ready",
	availability: "available",
	warning: { code: "screenPreviewUnavailable", retryable: true },
	wallThumbnail: {
		assetId: "asset-a",
		kind: "wallThumbnail",
		key: "wall/cache-a",
	},
	screenPreview: {
		assetId: "asset-a",
		kind: "screenPreview",
		key: "screen/cache-a",
	},
	rating: 4,
};

const emptyWallPage = {
	items: [],
	nextCursor: null,
	orderState: "settled",
	sourceWarnings: [],
	totalCount: 0,
	previewCounts: { wallReady: 0, screenReady: 0 },
};

function json(value: unknown, status = 200): Response {
	return new Response(JSON.stringify(value), {
		status,
		headers: { "content-type": "application/json" },
	});
}

function noContent(): Response {
	return new Response(null, { status: 204 });
}

function savedPreferences(
	overrides: Partial<{
		selectionId: string | null;
		breadcrumbs: Array<{ name: string; path: string }>;
		appearance: "system" | "light" | "dark";
		galleryScope: "currentFolder" | "includeSubfolders";
		sortDirection: "oldestFirst" | "newestFirst";
	}> = {},
): MemoryStorage {
	const storage = new MemoryStorage();
	storage.setItem(
		hostedPreferencesStorageKey,
		JSON.stringify({
			selectionId: null,
			breadcrumbs: [],
			appearance: "system",
			galleryScope: "includeSubfolders",
			sortDirection: "oldestFirst",
			...overrides,
		}),
	);
	storage.writes.length = 0;
	return storage;
}

function requestBody(request: RecordedRequest): unknown {
	if (typeof request.init?.body !== "string") return null;
	return JSON.parse(request.init.body);
}

function recordedState(request: RecordedRequest): unknown {
	const body = requestBody(request);
	return typeof body === "object" && body !== null && "state" in body
		? body.state
		: null;
}

afterEach(() => {
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe("HTTP PhotoService", () => {
	it("waits for bootstrap root identity before reading browser pick storage", async () => {
		const localStorage = savedPreferences();
		let releaseBootstrap!: (response: Response) => void;
		let markRequested!: () => void;
		const requested = new Promise<void>((resolve) => {
			markRequested = resolve;
		});
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch: async () => {
				markRequested();
				return new Promise<Response>((resolve) => {
					releaseBootstrap = resolve;
				});
			},
			randomUuid: () => "client-a",
		});

		const loading = service.getBootstrapState();
		await requested;
		expect(
			localStorage.reads.some((key) => key.startsWith("mote.picks.v1.")),
		).toBe(false);
		releaseBootstrap(json({ ...bootstrapResponse, rootId: "root-a" }));
		await loading;
		expect(localStorage.reads).toContain("mote.picks.v1.root-a");
		service.dispose();
	});

	it("maps the bootstrap original-download capability without probing a download URL", async () => {
		const fetch = vi.fn(async () =>
			json({
				capabilities: {
					folderBrowser: true,
					video: false,
					originalDownloads: true,
				},
				sourceAvailable: true,
			}),
		);
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch,
			randomUuid: () => "client-a",
		});

		await service.getBootstrapState();

		expect(service.capabilities.originalAction).toBe("download");
		expect(fetch).toHaveBeenCalledOnce();
		expect(fetch).toHaveBeenCalledWith("/api/v1/bootstrap", undefined);
		service.dispose();
	});

	it("hydrates root-scoped picks by source folder without replacing the active selection", async () => {
		const localStorage = savedPreferences();
		const calls: RecordedRequest[] = [];
		const assets = {
			"asset-a": representativeAsset,
			"asset-b": {
				...representativeAsset,
				id: "asset-b",
				displayName: "Glacier.jpg",
				wallThumbnail: null,
				screenPreview: null,
			},
		};
		const fetch = vi.fn(
			async (input: RequestInfo | URL, init?: RequestInit) => {
				const url = String(input);
				calls.push({ url, init });
				if (url === "/api/v1/bootstrap")
					return json({ ...bootstrapResponse, rootId: "root-a" });
				if (url === "/api/v1/selections")
					return json(
						{
							...selectionSummary,
							folderId: "active-folder",
							sourceId: "root-a",
						},
						201,
					);
				if (url.endsWith("/assets")) {
					const body = requestBody({ url, init }) as { assetIds: string[] };
					return json(
						body.assetIds.map((id) => assets[id as keyof typeof assets]),
					);
				}
				if (url.endsWith("/derivatives")) return noContent();
				if (url.includes("/wall?")) return json(emptyWallPage);
				throw new Error(`unexpected request ${url}`);
			},
		);
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: (url) => new FakeEventSource(url),
			randomUuid: () => "client-a",
		});

		expect(service.getPicks()).toEqual({
			revision: 0,
			items: [],
			persistenceError: null,
		});
		expect(
			localStorage.reads.some((key) => key.startsWith("mote.picks.v1.")),
		).toBe(false);
		expect(localStorage.peek("mote.picks.v1.root-a")).toBeNull();
		await service.getBootstrapState();
		expect(localStorage.reads).toContain("mote.picks.v1.root-a");
		await service.selectFolder("Trips/Iceland");
		await service.addPick({
			assetId: "asset-a",
			sourceFolderId: "folder-one",
			sourceLabel: "Stored family label",
		});
		await service.addPick({
			assetId: "asset-b",
			sourceFolderId: "folder-two",
			sourceLabel: "Stored trip label",
		});
		await expect
			.poll(() => service.getPicks().items.map((item) => item.asset?.id))
			.toEqual(["asset-a", "asset-b"]);
		const snapshot = await service.loadPicks();
		await service.requestPickDerivatives({
			assetIds: ["asset-b", "asset-a"],
			priority: "nearViewport",
			kind: "wallThumbnail",
		});
		await service.queryWall({
			cursor: null,
			limit: 10,
			direction: "oldestFirst",
		});

		expect(
			snapshot.items.map(({ assetId, sourceLabel, asset }) => ({
				assetId,
				sourceLabel,
				resolvedId: asset?.id,
			})),
		).toEqual([
			{
				assetId: "asset-a",
				sourceLabel: "Stored family label",
				resolvedId: "asset-a",
			},
			{
				assetId: "asset-b",
				sourceLabel: "Stored trip label",
				resolvedId: "asset-b",
			},
		]);
		const pickCalls = calls.filter(
			(call) =>
				call.url.endsWith("/assets") || call.url.endsWith("/derivatives"),
		);
		expect(
			// Mutations now hydrate automatically as well as on an explicit refresh.
			// Compare each distinct request contract, including its source and IDs.
			[
				...new Map(
					pickCalls.map(({ url, init }) => {
						const request = [url, requestBody({ url, init })];
						return [JSON.stringify(request), request];
					}),
				).values(),
			],
		).toEqual([
			[
				"/api/v1/selections/selection-folder-one/assets",
				{ assetIds: ["asset-a"] },
			],
			[
				"/api/v1/selections/selection-folder-two/assets",
				{ assetIds: ["asset-b"] },
			],
			[
				"/api/v1/selections/selection-folder-two/derivatives",
				{
					scope: "includeSubfolders",
					request: {
						assetIds: ["asset-b"],
						priority: "nearViewport",
						kind: "wallThumbnail",
					},
				},
			],
			[
				"/api/v1/selections/selection-folder-one/derivatives",
				{
					scope: "includeSubfolders",
					request: {
						assetIds: ["asset-a"],
						priority: "nearViewport",
						kind: "wallThumbnail",
					},
				},
			],
		]);
		expect(calls.at(-1)?.url).toContain("/selections/selection-a/wall?");
		expect(service.capabilities.originalAction).toBe("none");
		expect(service.originalDownloadUrl("asset-a")).toBeNull();
		service.dispose();
	});

	it("does not let an older hydration clear assets resolved for newer picks", async () => {
		const localStorage = savedPreferences();
		let resolveOlderRequest!: (response: Response) => void;
		let markOlderRequestFinished!: () => void;
		const olderRequest = new Promise<Response>((resolve) => {
			resolveOlderRequest = resolve;
		});
		const olderRequestFinished = new Promise<void>((resolve) => {
			markOlderRequestFinished = resolve;
		});
		let assetRequestCount = 0;
		const secondAsset = {
			...representativeAsset,
			id: "asset-b",
			displayName: "Glacier.jpg",
		};
		const fetch = vi.fn(
			async (input: RequestInfo | URL, init?: RequestInit) => {
				const url = String(input);
				if (url === "/api/v1/bootstrap")
					return json({ ...bootstrapResponse, rootId: "root-a" });
				if (url.endsWith("/assets")) {
					assetRequestCount += 1;
					if (assetRequestCount === 1) {
						return olderRequest.then((response) => {
							markOlderRequestFinished();
							return response;
						});
					}
					const body = requestBody({ url, init }) as { assetIds: string[] };
					return json(
						body.assetIds.map((id) =>
							id === representativeAsset.id ? representativeAsset : secondAsset,
						),
					);
				}
				throw new Error(`unexpected request ${url}`);
			},
		);
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch,
			randomUuid: () => "client-a",
		});

		await service.getBootstrapState();
		await service.addPick({
			assetId: representativeAsset.id,
			sourceFolderId: "folder-one",
			sourceLabel: "Family",
		});
		await expect.poll(() => assetRequestCount).toBe(1);
		await service.addPick({
			assetId: secondAsset.id,
			sourceFolderId: "folder-one",
			sourceLabel: "Family",
		});
		await expect
			.poll(() => service.getPicks().items.map((item) => item.asset?.id))
			.toEqual(["asset-a", "asset-b"]);

		resolveOlderRequest(json([representativeAsset]));
		await olderRequestFinished;
		await new Promise<void>((resolve) => setTimeout(resolve, 0));

		expect(service.getPicks().items.map((item) => item.asset?.id)).toEqual([
			"asset-a",
			"asset-b",
		]);
		service.dispose();
	});

	it("refreshes its pick snapshot when another tab writes the same root", async () => {
		const localStorage = new MemoryStorage();
		const storageListeners: Array<(event: StorageEvent) => void> = [];
		vi.stubGlobal(
			"addEventListener",
			(type: string, listener: EventListener) => {
				if (type === "storage")
					storageListeners.push(
						listener as unknown as (event: StorageEvent) => void,
					);
			},
		);
		vi.stubGlobal("removeEventListener", () => undefined);
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch: async () => json({ ...bootstrapResponse, rootId: "root-a" }),
			randomUuid: () => "client-a",
		});
		await service.getBootstrapState();
		const revisions: number[] = [];
		service.watchPicks((snapshot) => revisions.push(snapshot.revision));
		localStorage.setItem(
			"mote.picks.v1.root-a",
			JSON.stringify({
				revision: 7,
				items: [
					{
						assetId: "asset-a",
						sourceFolderId: "folder-a",
						sourceLabel: "Family",
					},
				],
			}),
		);

		for (const listener of storageListeners)
			listener({
				key: "mote.picks.v1.root-a",
				storageArea: localStorage,
			} as unknown as StorageEvent);

		expect(service.getPicks()).toMatchObject({
			revision: 7,
			items: [{ assetId: "asset-a", sourceLabel: "Family", asset: null }],
		});
		expect(revisions).toEqual([7]);
		service.dispose();
	});
	it("restores independent browser and tab state against one server", async () => {
		const firstLocal = savedPreferences({
			selectionId: "selection-a",
			breadcrumbs: selectionSummary.breadcrumbs,
			appearance: "dark",
			galleryScope: "currentFolder",
			sortDirection: "newestFirst",
		});
		const secondLocal = savedPreferences({
			selectionId: "selection-b",
			breadcrumbs: secondSelectionSummary.breadcrumbs,
			appearance: "light",
			galleryScope: "includeSubfolders",
			sortDirection: "oldestFirst",
		});
		const firstSession = new MemoryStorage();
		const secondSession = new MemoryStorage();
		const streams: FakeEventSource[] = [];
		const fetch = vi.fn(async (input: RequestInfo | URL) => {
			switch (String(input)) {
				case "/api/v1/bootstrap":
					return json(bootstrapResponse);
				case "/api/v1/selections/selection-a":
					return json(selectionSummary);
				case "/api/v1/selections/selection-b":
					return json(secondSelectionSummary);
				default:
					throw new Error(`unexpected request ${String(input)}`);
			}
		});
		const create = (
			localStorage: MemoryStorage,
			sessionStorage: MemoryStorage,
			clientId: string,
		) =>
			createHttpPhotoService({
				localStorage,
				sessionStorage,
				fetch,
				eventSourceFactory: (url) => {
					const stream = new FakeEventSource(url);
					streams.push(stream);
					return stream;
				},
				randomUuid: () => clientId,
			});
		const first = create(firstLocal, firstSession, "client-a");
		const second = create(secondLocal, secondSession, "client-b");

		await expect(first.getBootstrapState()).resolves.toMatchObject({
			settings: { appearance: "dark", galleryScope: "currentFolder" },
			activeSource: { id: "source-a", selectionId: "selection-a" },
		});
		await expect(second.getBootstrapState()).resolves.toMatchObject({
			settings: {
				appearance: "light",
				galleryScope: "includeSubfolders",
			},
			activeSource: { id: "source-b", selectionId: "selection-b" },
		});
		expect(first.initialSortDirection()).toBe("newestFirst");
		expect(second.initialSortDirection()).toBe("oldestFirst");
		const firstBrowserState = first.folderBrowserState();
		const secondBrowserState = second.folderBrowserState();
		expect(firstBrowserState).toEqual({
			breadcrumbs: selectionSummary.breadcrumbs,
			initialPath: "Trips/Iceland",
		});
		expect(secondBrowserState).toEqual({
			breadcrumbs: secondSelectionSummary.breadcrumbs,
			initialPath: "Trips/Alps",
		});
		const firstLeaf = firstBrowserState.breadcrumbs.at(-1);
		if (!firstLeaf) throw new Error("expected first restored breadcrumb");
		firstLeaf.path = "mutated";
		const secondLeaf = secondBrowserState.breadcrumbs.at(-1);
		if (!secondLeaf) throw new Error("expected second restored breadcrumb");
		secondLeaf.path = "mutated";
		expect(first.folderBrowserState()).toEqual({
			breadcrumbs: selectionSummary.breadcrumbs,
			initialPath: "Trips/Iceland",
		});
		expect(second.folderBrowserState()).toEqual({
			breadcrumbs: secondSelectionSummary.breadcrumbs,
			initialPath: "Trips/Alps",
		});
		const stopFirst = first.watchWallUpdates(() => undefined);
		const stopSecond = second.watchWallUpdates(() => undefined);
		expect(streams.map((stream) => stream.url)).toEqual([
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=currentFolder",
			"/api/v1/selections/selection-b/events?clientId=client-b&scope=includeSubfolders",
		]);
		expect(firstSession.getItem(hostedClientStorageKey)).toBe("client-a");
		expect(secondSession.getItem(hostedClientStorageKey)).toBe("client-b");
		stopFirst();
		stopSecond();
	});

	it("restores browser-owned state and maps origin-relative gallery calls", async () => {
		const calls: RecordedRequest[] = [];
		const localStorage = savedPreferences({
			selectionId: "selection-a",
			breadcrumbs: selectionSummary.breadcrumbs,
			appearance: "dark",
			galleryScope: "currentFolder",
			sortDirection: "newestFirst",
		});
		const fetch = vi.fn(
			async (input: RequestInfo | URL, init?: RequestInit) => {
				const url = String(input);
				calls.push({ url, init });
				if (url === "/api/v1/bootstrap") return json(bootstrapResponse);
				if (url === "/api/v1/selections/selection-a")
					return json(selectionSummary);
				if (url.startsWith("/api/v1/folders?"))
					return json({
						path: "Trips & Tours/Iceland",
						breadcrumbs: selectionSummary.breadcrumbs,
						children: [],
						imageCount: 27,
					});
				if (url.includes("/wall?")) return json(emptyWallPage);
				if (url.endsWith("/derivatives") || url.endsWith("/interaction"))
					return noContent();
				throw new Error(`unexpected request ${url}`);
			},
		);
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		await expect(service.getBootstrapState()).resolves.toEqual({
			savedFolders: emptySavedFolders(),
			settings: { appearance: "dark", galleryScope: "currentFolder" },
			activeSource: {
				id: "source-a",
				selectionId: "selection-a",
				displayName: "Iceland",
				availability: "available",
			},
		});
		expect(service.initialSortDirection()).toBe("newestFirst");
		const folder = await service.listFolders("Trips & Tours/Iceland");
		expect(folder.imageCount).toBe(27);
		await service.queryWall({
			cursor: "cursor/value",
			limit: 100,
			direction: "newestFirst",
		});
		await service.requestDerivatives({
			assetIds: ["asset-a"],
			priority: "visible",
			kind: "wallThumbnail",
		});
		await service.setWallInteraction(false);

		expect(calls[2]?.url).toBe(
			"/api/v1/folders?path=Trips%20%26%20Tours%2FIceland&includeImageCount=true",
		);
		expect(calls[3]?.url).toBe(
			"/api/v1/selections/selection-a/wall?scope=currentFolder&direction=newestFirst&limit=100&cursor=cursor%2Fvalue",
		);
		const derivativeCall = calls[4];
		const interactionCall = calls[5];
		if (!derivativeCall || !interactionCall) {
			throw new Error("expected derivative and interaction requests");
		}
		expect(requestBody(derivativeCall)).toEqual({
			scope: "currentFolder",
			request: {
				assetIds: ["asset-a"],
				priority: "visible",
				kind: "wallThumbnail",
			},
		});
		expect(requestBody(interactionCall)).toEqual({
			clientId: "client-a",
			scope: "currentFolder",
			state: "idle",
		});
		expect(
			service.derivativeUrl({
				assetId: "asset-a",
				kind: "wallThumbnail",
				key: "cache/key",
			}),
		).toBe("/api/v1/derivatives/cache%2Fkey");
	});

	it("carries the hosted runtime accent into browser state", async () => {
		const fetch = vi.fn(async () =>
			json({
				...bootstrapResponse,
				rootId: null,
				accentColor: "#7C3AED",
			}),
		);
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		await expect(service.getBootstrapState()).resolves.toMatchObject({
			accentColor: "#7C3AED",
		});
	});

	it("tolerates a legacy folder response without an image count", async () => {
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () =>
				json({ path: "Trips", breadcrumbs: [], children: [] }),
			),
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		await expect(service.listFolders("Trips")).resolves.toMatchObject({
			imageCount: null,
		});
	});

	it("stores a selected ID and structured breadcrumbs atomically", async () => {
		const localStorage = savedPreferences();
		const fetch = vi.fn(async (input: RequestInfo | URL) => {
			if (String(input) === "/api/v1/selections")
				return json(selectionSummary, 201);
			throw new Error(`unexpected request ${String(input)}`);
		});
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		const result = await service.selectFolder("Trips/Iceland");

		expect(result).toMatchObject({
			kind: "selected",
			state: { activeSource: { selectionId: "selection-a" } },
		});
		expect(localStorage.writes).toHaveLength(1);
		expect(JSON.parse(localStorage.writes[0]?.[1] ?? "null")).toMatchObject({
			selectionId: "selection-a",
			breadcrumbs: selectionSummary.breadcrumbs,
		});
		expect(service.folderBrowserState()).toEqual({
			breadcrumbs: selectionSummary.breadcrumbs,
			initialPath: "Trips/Iceland",
		});
		expect(fetch).toHaveBeenCalledWith(
			"/api/v1/selections",
			expect.objectContaining({
				method: "POST",
				body: JSON.stringify({ path: "Trips/Iceland" }),
			}),
		);
	});

	it("clears only a stale selection ID after a 404 and keeps recovery breadcrumbs", async () => {
		const localStorage = savedPreferences({
			selectionId: "selection-missing",
			breadcrumbs: selectionSummary.breadcrumbs,
			appearance: "light",
			galleryScope: "currentFolder",
			sortDirection: "newestFirst",
		});
		const fetch = vi.fn(async (input: RequestInfo | URL) => {
			if (String(input) === "/api/v1/bootstrap") return json(bootstrapResponse);
			return json(
				{ code: "notFound", message: "native path must not escape" },
				404,
			);
		});
		const service = createHttpPhotoService({
			localStorage,
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		await expect(service.getBootstrapState()).resolves.toEqual({
			savedFolders: emptySavedFolders(),
			settings: { appearance: "light", galleryScope: "currentFolder" },
			activeSource: null,
		});
		const browserState = service.folderBrowserState();
		expect(browserState).toEqual({
			breadcrumbs: selectionSummary.breadcrumbs,
			initialPath: "Trips/Iceland",
		});
		const recoveredLeaf = browserState.breadcrumbs[1];
		if (!recoveredLeaf)
			throw new Error("expected the recovered leaf breadcrumb");
		recoveredLeaf.path = "mutated";
		expect(service.folderBrowserState().initialPath).toBe("Trips/Iceland");
		expect(JSON.parse(localStorage.writes.at(-1)?.[1] ?? "null")).toMatchObject(
			{
				selectionId: null,
				breadcrumbs: selectionSummary.breadcrumbs,
				appearance: "light",
				galleryScope: "currentFolder",
				sortDirection: "newestFirst",
			},
		);
	});

	it("maps server failures to fixed public errors", async () => {
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () =>
				json(
					{
						code: "invalidCursor",
						message: "catalog failed at /private/catalog.sqlite",
					},
					400,
				),
			),
			eventSourceFactory: () => new FakeEventSource("unused"),
			randomUuid: () => "client-a",
		});

		const error = await service
			.queryWall({ cursor: "bad", limit: 100, direction: "oldestFirst" })
			.catch((reason: unknown) => reason);

		expect(error).toBeInstanceOf(PhotoServiceError);
		expect(error).toMatchObject({
			code: "invalidCursor",
			message: "That cursor is not valid.",
		});
		expect((error as Error).message).not.toContain("/private");
	});

	it.each([
		{
			name: "bootstrap without an original-download capability",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({
					capabilities: { folderBrowser: true, video: false },
					sourceAvailable: true,
				}),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.getBootstrapState(),
		},
		{
			name: "bootstrap with a non-boolean original-download capability",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({
					capabilities: {
						folderBrowser: true,
						video: false,
						originalDownloads: "yes",
					},
					sourceAvailable: true,
				}),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.getBootstrapState(),
		},
		{
			name: "extra folder field",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({
					path: "",
					breadcrumbs: [],
					children: [],
					nativePath: "/photos",
				}),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.listFolders(""),
		},
		{
			name: "negative folder image count",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({ path: "", breadcrumbs: [], children: [], imageCount: -1 }),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.listFolders(""),
		},
		{
			name: "fractional folder image count",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({ path: "", breadcrumbs: [], children: [], imageCount: 1.5 }),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.listFolders(""),
		},
		{
			name: "unknown wall enum",
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			fetch: async () =>
				json({
					items: [{ ...representativeAsset, mediaKind: "mov" }],
					nextCursor: null,
					orderState: "settled",
					sourceWarnings: [],
				}),
			act: (service: ReturnType<typeof createHttpPhotoService>) =>
				service.queryWall({
					cursor: null,
					limit: 1,
					direction: "oldestFirst",
				}),
		},
	])(
		"strictly rejects $name success DTOs",
		async ({ localStorage, fetch, act }) => {
			const service = createHttpPhotoService({
				localStorage,
				sessionStorage: new MemoryStorage(),
				fetch: vi.fn(fetch),
				eventSourceFactory: () => new FakeEventSource("unused"),
				randomUuid: () => "client-a",
			});

			await expect(act(service)).rejects.toEqual(
				expect.objectContaining({
					code: "internal",
					message: "Mote could not complete that request.",
				}),
			);
		},
	);

	it("turns malformed and extra SSE payloads into one authoritative resync", () => {
		const streams: FakeEventSource[] = [];
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () => noContent()),
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		const received: WallUpdate[] = [];
		const stop = service.watchWallUpdates((update) => received.push(update));

		streams[0]?.emitRaw("{not-json", "1");
		streams[0]?.emit(
			{
				kind: "progress",
				selectionId: "selection-a",
				generation: 1,
				progress: {
					discovered: 1,
					shaped: 1,
					enriched: 0,
					directTotal: 1,
					total: 1,
				},
				nativePath: "/photos/private",
			},
			"2",
		);
		streams[0]?.emit({ kind: "futureUpdate", selectionId: "selection-a" }, "3");

		expect(received).toEqual([
			{ kind: "resyncRequired", selectionId: "selection-a" },
		]);
		stop();
	});

	it("preserves one monotonic u64 replay cursor and resets it across contexts", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () => noContent()),
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		const stop = service.watchWallUpdates(() => undefined);
		const progress = {
			kind: "progress",
			selectionId: "selection-a",
			generation: 1,
			progress: {
				discovered: 1,
				shaped: 1,
				enriched: 0,
				directTotal: 1,
				total: 1,
			},
		};
		const reconnect = async (expected: string) => {
			streams.at(-1)?.fail();
			await vi.advanceTimersByTimeAsync(250);
			expect(streams.at(-1)?.url).toBe(expected);
			streams.at(-1)?.open();
		};

		streams[0]?.emit(progress, "9");
		streams[0]?.emit(progress, "9");
		streams[0]?.emit(progress, "8");
		streams[0]?.emit(progress, "010");
		await reconnect(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=9",
		);
		streams.at(-1)?.emit(progress, "18446744073709551616");
		await reconnect(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=9",
		);
		streams.at(-1)?.emit(progress, "");
		streams.at(-1)?.emit(progress, "12x");
		await reconnect(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=9",
		);
		streams.at(-1)?.emit(progress, "18446744073709551615");
		await reconnect(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=18446744073709551615",
		);

		await service.updateGalleryScope("currentFolder");
		expect(streams.at(-1)?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=currentFolder",
		);
		streams.at(-1)?.emit({ ...progress, generation: 2 }, "3");
		await reconnect(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=currentFolder&afterEventId=3",
		);
		await service.updateGalleryScope("includeSubfolders");
		expect(streams.at(-1)?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders",
		);
		stop();
	});

	it("starts replay fresh after replacing the selected folder", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async (input: RequestInfo | URL) => {
				if (String(input) === "/api/v1/selections")
					return json(secondSelectionSummary, 201);
				return noContent();
			}),
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		const selectionAProgress = {
			kind: "progress",
			selectionId: "selection-a",
			generation: 1,
			progress: {
				discovered: 1,
				shaped: 1,
				enriched: 0,
				directTotal: 1,
				total: 1,
			},
		};
		const stopFirst = service.watchWallUpdates(() => undefined);
		streams[0]?.emit(selectionAProgress, "7");

		await service.selectFolder("Trips/Alps");

		expect(streams[0]?.closed).toBe(true);
		stopFirst();
		const stopSecond = service.watchWallUpdates(() => undefined);
		expect(streams[1]?.url).toBe(
			"/api/v1/selections/selection-b/events?clientId=client-a&scope=includeSubfolders",
		);
		streams[1]?.emit(
			{ ...selectionAProgress, selectionId: "selection-b", generation: 2 },
			"3",
		);
		streams[1]?.fail();
		await vi.advanceTimersByTimeAsync(250);
		expect(streams[2]?.url).toBe(
			"/api/v1/selections/selection-b/events?clientId=client-a&scope=includeSubfolders&afterEventId=3",
		);
		stopSecond();
	});

	it("accepts only canonical zero as the initial replay cursor", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () => noContent()),
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		const stop = service.watchWallUpdates(() => undefined);
		const progress = {
			kind: "progress",
			selectionId: "selection-a",
			generation: 1,
			progress: {
				discovered: 0,
				shaped: 0,
				enriched: 0,
				directTotal: 0,
				total: 0,
			},
		};

		streams[0]?.emit(progress, "00");
		streams[0]?.fail();
		await vi.advanceTimersByTimeAsync(250);

		expect(streams[1]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders",
		);
		streams[1]?.open();
		streams[1]?.emit(progress, "0");
		streams[1]?.fail();
		await vi.advanceTimersByTimeAsync(250);
		expect(streams[2]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=0",
		);
		stop();
	});

	it("does not replace a valid replay cursor with a decimal above u64 max", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch: vi.fn(async () => noContent()),
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		const stop = service.watchWallUpdates(() => undefined);
		const progress = {
			kind: "progress",
			selectionId: "selection-a",
			generation: 1,
			progress: {
				discovered: 1,
				shaped: 1,
				enriched: 0,
				directTotal: 1,
				total: 1,
			},
		};

		streams[0]?.emit(progress, "10");
		streams[0]?.emit(progress, "18446744073709551616");
		streams[0]?.fail();
		await vi.advanceTimersByTimeAsync(250);

		expect(streams[1]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=10",
		);
		stop();
	});

	it("caps reconnect backoff at five seconds and stops queued retries", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const create = () =>
			createHttpPhotoService({
				localStorage: savedPreferences({ selectionId: "selection-a" }),
				sessionStorage: new MemoryStorage(),
				fetch: vi.fn(async () => noContent()),
				eventSourceFactory: (url) => {
					const stream = new FakeEventSource(url);
					streams.push(stream);
					return stream;
				},
				randomUuid: () => `client-${streams.length}`,
			});
		const service = create();
		const stop = service.watchWallUpdates(() => undefined);
		for (const delay of [250, 500, 1_000, 2_000, 4_000, 5_000, 5_000]) {
			const count = streams.length;
			streams.at(-1)?.fail();
			await vi.advanceTimersByTimeAsync(delay - 1);
			expect(streams).toHaveLength(count);
			await vi.advanceTimersByTimeAsync(1);
			expect(streams).toHaveLength(count + 1);
		}
		streams.at(-1)?.fail();
		stop();
		await vi.advanceTimersByTimeAsync(10_000);
		expect(streams).toHaveLength(8);

		const disposable = create();
		disposable.watchWallUpdates(() => undefined);
		streams.at(-1)?.fail();
		disposable.dispose();
		const afterDispose = streams.length;
		await vi.advanceTimersByTimeAsync(10_000);
		expect(streams).toHaveLength(afterDispose);
	});

	it("keeps selection identity consistent across nonempty wall and event DTOs", async () => {
		const streams: FakeEventSource[] = [];
		const calls: string[] = [];
		const fetch = vi.fn(async (input: RequestInfo | URL) => {
			const url = String(input);
			calls.push(url);
			if (url === "/api/v1/bootstrap") return json(bootstrapResponse);
			if (url === "/api/v1/selections/selection-a")
				return json(selectionSummary);
			if (url.includes("/wall?"))
				return json({
					items: [representativeAsset],
					nextCursor: "cursor-a",
					orderState: "settled",
					sourceWarnings: [{ code: "sourceOffline", retryable: true }],
					totalCount: 1,
					previewCounts: { wallReady: 1, screenReady: 1 },
				});
			throw new Error(`unexpected request ${url}`);
		});
		const service = createHttpPhotoService({
			localStorage: savedPreferences({ selectionId: "selection-a" }),
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});

		const state = await service.getBootstrapState();
		const page = await service.queryWall({
			cursor: null,
			limit: 1,
			direction: "oldestFirst",
		});
		const received: WallUpdate[] = [];
		const stop = service.watchWallUpdates((update) => received.push(update));
		streams[0]?.emit(
			{
				kind: "catalogBatch",
				selectionId: "selection-a",
				assets: [representativeAsset],
				orderState: "settled",
				generation: 2,
				progress: {
					discovered: 1,
					shaped: 1,
					enriched: 1,
					directTotal: 1,
					total: 1,
				},
			},
			"2",
		);

		expect(state.activeSource).toEqual({
			id: "source-a",
			selectionId: "selection-a",
			displayName: "Iceland",
			availability: "available",
		});
		expect(page).toEqual({
			items: [representativeAsset],
			nextCursor: "cursor-a",
			orderState: "settled",
			totalCount: 1,
			previewCounts: { wallReady: 1, screenReady: 1 },
			sourceWarnings: [{ code: "sourceOffline", retryable: true }],
		});
		expect(received).toEqual([
			{
				kind: "catalogBatch",
				selectionId: "selection-a",
				assets: [representativeAsset],
				orderState: "settled",
				generation: 2,
				progress: {
					discovered: 1,
					shaped: 1,
					enriched: 1,
					directTotal: 1,
					total: 1,
				},
			},
		]);
		expect(calls[2]).toContain("/api/v1/selections/selection-a/wall?");
		expect(streams[0]?.url).toContain("/api/v1/selections/selection-a/events?");
		stop();
	});

	it("reconnects SSE with pair-scoped replay and emits one resync per gap", async () => {
		vi.useFakeTimers();
		const streams: FakeEventSource[] = [];
		const fetch = vi.fn(async (input: RequestInfo | URL) => {
			if (String(input) === "/api/v1/selections")
				return json(selectionSummary, 201);
			return noContent();
		});
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		await service.selectFolder("Trips/Iceland");
		const received: WallUpdate[] = [];
		const stop = service.watchWallUpdates((update) => received.push(update));

		expect(streams[0]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders",
		);
		streams[0]?.emit(
			{ kind: "resyncRequired", selectionId: "selection-a" },
			"4",
		);
		streams[0]?.emit(
			{ kind: "resyncRequired", selectionId: "selection-a" },
			"5",
		);
		expect(received).toEqual([
			{ kind: "resyncRequired", selectionId: "selection-a" },
		]);
		streams[0]?.emit(
			{
				kind: "progress",
				selectionId: "selection-a",
				generation: 1,
				progress: {
					discovered: 1,
					shaped: 1,
					enriched: 0,
					directTotal: 1,
					total: 1,
				},
			},
			"6",
		);
		streams[0]?.fail();
		await vi.advanceTimersByTimeAsync(249);
		expect(streams).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(1);
		expect(streams[1]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=includeSubfolders&afterEventId=6",
		);

		await service.updateGalleryScope("currentFolder");
		expect(streams[1]?.closed).toBe(true);
		expect(streams[2]?.url).toBe(
			"/api/v1/selections/selection-a/events?clientId=client-a&scope=currentFolder",
		);
		stop();
		expect(streams[2]?.closed).toBe(true);
	});

	it("refreshes active interaction leases and cancels them on stream disposal", async () => {
		vi.useFakeTimers();
		const calls: RecordedRequest[] = [];
		const streams: FakeEventSource[] = [];
		const fetch = vi.fn(
			async (input: RequestInfo | URL, init?: RequestInit) => {
				const url = String(input);
				calls.push({ url, init });
				if (url === "/api/v1/selections") return json(selectionSummary, 201);
				return noContent();
			},
		);
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: (url) => {
				const stream = new FakeEventSource(url);
				streams.push(stream);
				return stream;
			},
			randomUuid: () => "client-a",
		});
		await service.selectFolder("Trips/Iceland");
		const stop = service.watchWallUpdates(() => undefined);
		await service.setWallInteraction(true);
		const interactionBodies = () =>
			calls
				.filter((call) => call.url.endsWith("/interaction"))
				.map(requestBody);
		expect(interactionBodies()).toEqual([
			{
				clientId: "client-a",
				scope: "includeSubfolders",
				state: "active",
			},
		]);

		streams[0]?.open();
		await vi.runAllTicks();
		expect(interactionBodies()).toHaveLength(2);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(interactionBodies()).toHaveLength(3);
		stop();
		await vi.advanceTimersByTimeAsync(30_000);
		expect(interactionBodies()).toHaveLength(3);

		await service.setWallInteraction(false);
		expect(interactionBodies().at(-1)).toEqual({
			clientId: "client-a",
			scope: "includeSubfolders",
			state: "idle",
		});
		service.dispose();
	});

	it("cancels interaction refresh on idle, selection replacement, and disposal", async () => {
		vi.useFakeTimers();
		const calls: RecordedRequest[] = [];
		let selectionCalls = 0;
		const fetch = vi.fn(
			async (input: RequestInfo | URL, init?: RequestInit) => {
				const url = String(input);
				calls.push({ url, init });
				if (url === "/api/v1/selections") {
					selectionCalls += 1;
					return json(
						selectionCalls === 1 ? selectionSummary : secondSelectionSummary,
						201,
					);
				}
				return noContent();
			},
		);
		const service = createHttpPhotoService({
			localStorage: savedPreferences(),
			sessionStorage: new MemoryStorage(),
			fetch,
			eventSourceFactory: (url) => new FakeEventSource(url),
			randomUuid: () => "client-a",
		});
		const activeCalls = () =>
			calls.filter(
				(call) =>
					call.url.endsWith("/interaction") && recordedState(call) === "active",
			).length;
		await service.selectFolder("Trips/Iceland");
		await service.setWallInteraction(true);
		await vi.advanceTimersByTimeAsync(10_000);
		expect(activeCalls()).toBe(2);
		await service.setWallInteraction(false);
		const afterIdle = activeCalls();
		await vi.advanceTimersByTimeAsync(20_000);
		expect(activeCalls()).toBe(afterIdle);

		await service.setWallInteraction(true);
		await service.selectFolder("Trips/Alps");
		const afterSelection = activeCalls();
		await vi.advanceTimersByTimeAsync(20_000);
		expect(activeCalls()).toBe(afterSelection);

		await service.setWallInteraction(true);
		service.dispose();
		const afterDispose = activeCalls();
		await vi.advanceTimersByTimeAsync(20_000);
		expect(activeCalls()).toBe(afterDispose);
	});
});

it("migrates the legacy hosted selection into the root-scoped folder list", async () => {
	const local = savedPreferences({ selectionId: "selection-a" });
	const service = createHttpPhotoService({
		localStorage: local,
		sessionStorage: new MemoryStorage(),
		fetch: async (input) => {
			const url = String(input);
			if (url.endsWith("bootstrap"))
				return json({ ...bootstrapResponse, rootId: "source-a" });
			if (url.endsWith("/access"))
				return json({
					folderId: "a",
					state: "available",
					generation: 1,
					retryAfterMs: 5000,
				});
			return json(selectionSummary);
		},
	});
	const state = await service.getBootstrapState();
	expect(state.savedFolders.entries).toHaveLength(1);
	expect(state.activeSource?.displayName).toBe("Iceland");
	service.dispose();
});

it("keeps a newer selection when an earlier startup restoration finishes late", async () => {
	const local = new MemoryStorage();
	const session = new MemoryStorage();
	const { createBrowserSavedFolders } = await import(
		"../folders/browserSavedFolders"
	);
	const folders = createBrowserSavedFolders(local, session, "source-a");
	const one = folders.save({
		folderId: "a",
		name: "Iceland",
		displayPath: "Trips/Iceland",
	});
	folders.select(one.id);
	folders.markMigrated();
	let release!: (response: Response) => void;
	let started!: () => void;
	const checking = new Promise<void>((resolve) => {
		started = resolve;
	});
	const service = createHttpPhotoService({
		localStorage: local,
		sessionStorage: session,
		fetch: async (input) => {
			const url = String(input);
			if (url.endsWith("bootstrap"))
				return json({ ...bootstrapResponse, rootId: "source-a" });
			if (url.endsWith("/access")) {
				started();
				return new Promise<Response>((resolve) => {
					release = resolve;
				});
			}
			return json({
				...secondSelectionSummary,
				sourceId: "source-a",
				availability: "available",
			});
		},
	});
	const restoring = service.getBootstrapState();
	await checking;
	await service.selectFolder("Trips/Alps");
	release(
		json({
			folderId: "a",
			state: "available",
			generation: 1,
			retryAfterMs: 5000,
		}),
	);
	const state = await restoring;
	expect(state.activeSource?.displayName).toBe("Alps");
	expect(service.getSavedFolders().activeEntryId).not.toBeNull();
	service.dispose();
});

it("does not recreate an entry removed in another tab during its selection POST", async () => {
	const local = new MemoryStorage();
	const session = new MemoryStorage();
	const { createBrowserSavedFolders } = await import(
		"../folders/browserSavedFolders"
	);
	const folders = createBrowserSavedFolders(local, session, "source-a");
	const one = folders.save({
		folderId: "a",
		name: "Iceland",
		displayPath: "Trips/Iceland",
	});
	folders.select(null);
	folders.markMigrated();
	let release!: (response: Response) => void;
	let started!: () => void;
	const posting = new Promise<void>((resolve) => {
		started = resolve;
	});
	const service = createHttpPhotoService({
		localStorage: local,
		sessionStorage: session,
		fetch: async (input) => {
			const url = String(input);
			if (url.endsWith("bootstrap"))
				return json({ ...bootstrapResponse, rootId: "source-a" });
			if (url.endsWith("/access"))
				return json({
					folderId: "a",
					state: "available",
					generation: 1,
					retryAfterMs: 5000,
				});
			started();
			return new Promise<Response>((resolve) => {
				release = resolve;
			});
		},
	});
	await service.getBootstrapState();
	const opening = service.activateSavedFolder(one.id);
	await posting;
	createBrowserSavedFolders(local, new MemoryStorage(), "source-a").remove(
		one.id,
	);
	release(json(selectionSummary));
	expect((await opening).kind).toBe("cancelled");
	expect(service.getSavedFolders().entries).toEqual([]);
	service.dispose();
});

it("keeps legacy preferences retryable when migration lookup fails transiently", async () => {
	const local = savedPreferences({ selectionId: "selection-a" });
	const service = createHttpPhotoService({
		localStorage: local,
		sessionStorage: new MemoryStorage(),
		fetch: async (input) => {
			if (String(input).endsWith("bootstrap"))
				return json({ ...bootstrapResponse, rootId: "source-a" });
			throw Error("offline");
		},
	});
	await expect(service.getBootstrapState()).rejects.toBeInstanceOf(
		PhotoServiceError,
	);
	expect(
		JSON.parse(local.getItem(hostedPreferencesStorageKey) ?? "{}").selectionId,
	).toBe("selection-a");
	expect(local.getItem("mote.folders.v1.source-a.migrated")).not.toBe("1");
	service.dispose();
});

it("does not save a response from a changed hosted root into the previous root", async () => {
	const service = createHttpPhotoService({
		localStorage: new MemoryStorage(),
		sessionStorage: new MemoryStorage(),
		fetch: async (input) => {
			if (String(input).endsWith("bootstrap"))
				return json({ ...bootstrapResponse, rootId: "source-a" });
			return json({ ...selectionSummary, sourceId: "changed-root" });
		},
	});
	await service.getBootstrapState();
	await expect(service.selectFolder("Trips/Iceland")).rejects.toBeInstanceOf(
		PhotoServiceError,
	);
	expect(service.getSavedFolders().entries).toEqual([]);
	service.dispose();
});
