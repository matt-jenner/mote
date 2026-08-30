import { afterEach, describe, expect, it, vi } from "vitest";
import { hostedPreferencesStorageKey } from "./browserPreferences";
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
		const event = {
			data: JSON.stringify(update),
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

const emptyWallPage = {
	items: [],
	nextCursor: null,
	orderState: "settled",
	sourceWarnings: [],
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

afterEach(() => {
	vi.useRealTimers();
});

describe("HTTP PhotoService", () => {
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
		await service.listFolders("Trips & Tours/Iceland");
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
			"/api/v1/folders?path=Trips%20%26%20Tours%2FIceland",
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
				progress: { discovered: 1, shaped: 1, enriched: 0, total: 1 },
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
});
