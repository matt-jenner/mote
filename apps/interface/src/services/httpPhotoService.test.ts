import { afterEach, describe, expect, it, vi } from "vitest";
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
	readonly writes: Array<[string, string]> = [];

	get length(): number {
		return this.values.size;
	}

	clear(): void {
		this.values.clear();
	}

	getItem(key: string): string | null {
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
	capabilities: { folderBrowser: true, video: false },
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
});

describe("HTTP PhotoService", () => {
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
			name: "malformed bootstrap",
			localStorage: savedPreferences(),
			fetch: async () =>
				json({
					capabilities: { folderBrowser: "yes", video: false },
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
					message: "Photo Viewer could not complete that request.",
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
