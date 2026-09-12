import { createBrowserSavedFolders } from "../folders/browserSavedFolders";
import {
	cloneSavedFolders,
	emptySavedFolders,
	type FolderAccess,
	folderLabel,
	type SavedFolderSnapshot,
	sourceIsUnavailable,
} from "../folders/savedFolders";
import { createSelectionIntent } from "../folders/selectionIntent";
import { type BrowserPicks, createBrowserPicks } from "../picks/browserPicks";
import type { PickListSnapshot, PickReference } from "../picks/pickList";
import {
	type BrowserPreferences,
	createBrowserPreferences,
} from "./browserPreferences";
import type {
	Appearance,
	BootstrapState,
	ChooseFolderResult,
	DerivativeReference,
	DerivativeRequest,
	FolderBreadcrumb,
	FolderEntry,
	FolderListing,
	GalleryScope,
	PhotoService,
	ScanProgressDto,
	SortDirection,
	SourceAvailability,
	SourceSummary,
	WallAsset,
	WallPage,
	WallUpdate,
	WallWarningState,
} from "./photoService";
import { PhotoServiceError } from "./photoService";

export interface EventSourceLike {
	onopen: ((event: Event) => void) | null;
	onerror: ((event: Event) => void) | null;
	addEventListener(
		type: string,
		listener: (event: MessageEvent<string>) => void,
	): void;
	removeEventListener(
		type: string,
		listener: (event: MessageEvent<string>) => void,
	): void;
	close(): void;
}

export interface HttpPhotoService extends PhotoService {
	dispose(): void;
}

export interface HttpPhotoServiceOptions {
	localStorage?: Storage;
	sessionStorage?: Storage;
	fetch?: typeof globalThis.fetch;
	eventSourceFactory?: (url: string) => EventSourceLike;
	randomUuid?: () => string;
}

interface SelectionSummary {
	folderId: string;
	path: string;
	id: string;
	sourceId: string;
	displayName: string;
	breadcrumbs: FolderBreadcrumb[];
	availability: SourceAvailability;
}

interface ActiveWatch {
	listener: (update: WallUpdate) => void;
	stream: EventSourceLike | null;
	retryTimer: ReturnType<typeof setTimeout> | null;
	retryDelay: number;
	active: boolean;
	resyncDelivered: boolean;
}

const internalErrorMessage = "Mote could not complete that request.";
const retryStartMs = 250;
const retryMaximumMs = 5_000;
const interactionRefreshMs = 10_000;
const maximumU64 = 18_446_744_073_709_551_615n;

const publicErrorMessages: Readonly<Record<string, string>> = {
	invalidRequest: "That request is not valid.",
	invalidFolderPath: "That folder path is not valid.",
	folderUnavailable: "That folder is unavailable.",
	folderUnreadable: "That folder cannot be read.",
	sourceUnavailable: "The photo source is unavailable.",
	notFound: "That selection is not available.",
	invalidCursor: "That cursor is not valid.",
	invalidLimit: "That wall page limit is not valid.",
	derivativeUnavailable: "The requested derivative is not currently available.",
	derivativeFailed: "The requested derivative could not be generated.",
};

class DecodeError extends Error {}

function internalError(): PhotoServiceError {
	return new PhotoServiceError("internal", internalErrorMessage);
}

function record(
	value: unknown,
	allowedKeys?: readonly string[],
): Record<string, unknown> {
	if (typeof value !== "object" || value === null || Array.isArray(value)) {
		throw new DecodeError("expected object");
	}
	const result = value as Record<string, unknown>;
	if (
		allowedKeys &&
		Object.keys(result).some((key) => !allowedKeys.includes(key))
	) {
		throw new DecodeError("unexpected object key");
	}
	return result;
}

function stringValue(value: unknown): string {
	if (typeof value !== "string") throw new DecodeError("expected string");
	return value;
}

function booleanValue(value: unknown): boolean {
	if (typeof value !== "boolean") throw new DecodeError("expected boolean");
	return value;
}

function integerValue(value: unknown): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value)) {
		throw new DecodeError("expected integer");
	}
	return value;
}

function enumValue<T extends string>(value: unknown, values: readonly T[]): T {
	if (typeof value !== "string" || !values.includes(value as T)) {
		throw new DecodeError("expected enum");
	}
	return value as T;
}

function arrayValue<T>(value: unknown, decode: (entry: unknown) => T): T[] {
	if (!Array.isArray(value)) throw new DecodeError("expected array");
	return value.map(decode);
}

function decodeBreadcrumb(value: unknown): FolderBreadcrumb {
	const item = record(value, ["name", "path"]);
	return { name: stringValue(item.name), path: stringValue(item.path) };
}

function decodeFolderEntry(value: unknown): FolderEntry {
	const item = record(value, ["name", "path"]);
	return { name: stringValue(item.name), path: stringValue(item.path) };
}

function decodeFolderListing(value: unknown): FolderListing {
	const listing = record(value, [
		"path",
		"breadcrumbs",
		"children",
		"imageCount",
	]);
	const imageCount =
		listing.imageCount === undefined ? null : integerValue(listing.imageCount);
	if (imageCount !== null && imageCount < 0)
		throw new DecodeError("expected non-negative integer");
	return {
		path: stringValue(listing.path),
		breadcrumbs: arrayValue(listing.breadcrumbs, decodeBreadcrumb),
		children: arrayValue(listing.children, decodeFolderEntry),
		imageCount,
	};
}

function decodeAvailability(value: unknown): SourceAvailability {
	return enumValue(value, [
		"available",
		"rootOffline",
		"missing",
		"unreadable",
	]);
}

function decodeSelectionSummary(value: unknown): SelectionSummary {
	const summary = record(value, [
		"folderId",
		"path",
		"id",
		"sourceId",
		"displayName",
		"breadcrumbs",
		"availability",
	]);
	return {
		id: stringValue(summary.id),
		folderId:
			summary.folderId === undefined
				? stringValue(summary.id).replace(/^selection-/, "")
				: stringValue(summary.folderId),
		path:
			summary.path === undefined
				? (arrayValue(summary.breadcrumbs, decodeBreadcrumb).at(-1)?.path ?? "")
				: stringValue(summary.path),
		sourceId: stringValue(summary.sourceId),
		displayName: stringValue(summary.displayName),
		breadcrumbs: arrayValue(summary.breadcrumbs, decodeBreadcrumb),
		availability: decodeAvailability(summary.availability),
	};
}

function decodeBootstrap(value: unknown): string | null {
	const bootstrap = record(value, [
		"capabilities",
		"sourceAvailable",
		"rootId",
	]);
	const capabilities = record(bootstrap.capabilities, [
		"folderBrowser",
		"video",
	]);
	booleanValue(capabilities.folderBrowser);
	booleanValue(capabilities.video);
	booleanValue(bootstrap.sourceAvailable);
	return bootstrap.rootId == null ? null : stringValue(bootstrap.rootId);
}

function decodeFolderAccess(value: unknown): FolderAccess {
	const item = record(value, [
		"folderId",
		"state",
		"generation",
		"retryAfterMs",
	]);
	return {
		folderId: stringValue(item.folderId),
		state: enumValue(item.state, [
			"unknown",
			"checking",
			"available",
			"missing",
			"unreadable",
			"rootOffline",
			"unverified",
		]),
		generation: integerValue(item.generation),
		retryAfterMs: integerValue(item.retryAfterMs),
	};
}

function decodeWarning(value: unknown): WallWarningState {
	const warning = record(value, ["code", "retryable"]);
	return {
		code: stringValue(warning.code),
		retryable: booleanValue(warning.retryable),
	};
}

function decodeDerivativeReference(value: unknown): DerivativeReference {
	const reference = record(value, ["assetId", "kind", "key"]);
	return {
		assetId: stringValue(reference.assetId),
		kind: enumValue(reference.kind, ["wallThumbnail", "screenPreview"]),
		key: stringValue(reference.key),
	};
}

function nullable<T>(value: unknown, decode: (value: unknown) => T): T | null {
	return value === null ? null : decode(value);
}

function decodeWallAsset(value: unknown): WallAsset {
	const asset = record(value, [
		"id",
		"displayName",
		"mediaKind",
		"provisionalOrder",
		"capturedAtUtc",
		"dateState",
		"width",
		"height",
		"representativeRgb",
		"shapeState",
		"availability",
		"warning",
		"wallThumbnail",
		"screenPreview",
		"rating",
	]);
	return {
		id: stringValue(asset.id),
		displayName: stringValue(asset.displayName),
		mediaKind: enumValue(asset.mediaKind, [
			"jpeg",
			"png",
			"tiff",
			"heif",
			"webp",
			"avif",
			"raw",
			"video",
			"unknown",
		]),
		provisionalOrder: integerValue(asset.provisionalOrder),
		capturedAtUtc: nullable(asset.capturedAtUtc, stringValue),
		dateState: enumValue(asset.dateState, ["provisional", "settled"]),
		width: integerValue(asset.width),
		height: integerValue(asset.height),
		representativeRgb: nullable(asset.representativeRgb, integerValue),
		shapeState: enumValue(asset.shapeState, ["ready", "fallback"]),
		availability: decodeAvailability(asset.availability),
		warning: nullable(asset.warning, decodeWarning),
		wallThumbnail: nullable(asset.wallThumbnail, decodeDerivativeReference),
		screenPreview: nullable(asset.screenPreview, decodeDerivativeReference),
		rating: nullable(asset.rating, integerValue),
	};
}

function decodeWallPage(value: unknown): WallPage {
	const page = record(value, [
		"items",
		"nextCursor",
		"orderState",
		"sourceWarnings",
		"totalCount",
		"previewCounts",
	]);
	return {
		items: arrayValue(page.items, decodeWallAsset),
		nextCursor: nullable(page.nextCursor, stringValue),
		orderState: enumValue(page.orderState, ["provisional", "settled"]),
		sourceWarnings: arrayValue(page.sourceWarnings, decodeWarning),
		totalCount: integerValue(page.totalCount),
		previewCounts: decodePreviewCounts(page.previewCounts),
	};
}

function decodeResolvedAssets(value: unknown): Array<WallAsset | null> {
	return arrayValue(value, (asset) => nullable(asset, decodeWallAsset));
}

function decodePreviewCounts(value: unknown) {
	const counts = record(value, ["wallReady", "screenReady"]);
	return {
		wallReady: integerValue(counts.wallReady),
		screenReady: integerValue(counts.screenReady),
	};
}

function decodeProgress(value: unknown): ScanProgressDto {
	const progress = record(value, [
		"discovered",
		"shaped",
		"enriched",
		"directTotal",
		"total",
	]);
	return {
		discovered: integerValue(progress.discovered),
		shaped: integerValue(progress.shaped),
		enriched: integerValue(progress.enriched),
		directTotal: nullable(progress.directTotal, integerValue),
		total: nullable(progress.total, integerValue),
	};
}

function decodeWallUpdate(value: unknown): WallUpdate {
	const update = record(value);
	const kind = stringValue(update.kind);
	switch (kind) {
		case "catalogBatch": {
			const item = record(value, [
				"kind",
				"selectionId",
				"sourceId",
				"assets",
				"orderState",
				"generation",
				"progress",
			]);
			const sourceId =
				item.sourceId === undefined ? undefined : stringValue(item.sourceId);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				...(sourceId === undefined ? {} : { sourceId }),
				assets: arrayValue(item.assets, decodeWallAsset),
				orderState: enumValue(item.orderState, ["provisional", "settled"]),
				generation: integerValue(item.generation),
				progress: decodeProgress(item.progress),
			};
		}
		case "derivativesReady": {
			const item = record(value, [
				"kind",
				"selectionId",
				"derivatives",
				"previewCounts",
			]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				derivatives: arrayValue(item.derivatives, decodeDerivativeReference),
				previewCounts: nullable(item.previewCounts, decodePreviewCounts),
			};
		}
		case "metadataSettled": {
			const item = record(value, [
				"kind",
				"selectionId",
				"sourceId",
				"generation",
			]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				sourceId: stringValue(item.sourceId),
				generation: integerValue(item.generation),
			};
		}
		case "progress": {
			const item = record(value, [
				"kind",
				"selectionId",
				"generation",
				"progress",
			]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				generation: integerValue(item.generation),
				progress: decodeProgress(item.progress),
			};
		}
		case "sourceUnavailable": {
			const item = record(value, ["kind", "selectionId", "sourceId"]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				sourceId: stringValue(item.sourceId),
			};
		}
		case "warning": {
			const item = record(value, [
				"kind",
				"selectionId",
				"sourceId",
				"assetId",
				"warning",
			]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				sourceId: stringValue(item.sourceId),
				assetId: nullable(item.assetId, stringValue),
				warning: decodeWarning(item.warning),
			};
		}
		case "warningCleared": {
			const item = record(value, [
				"kind",
				"selectionId",
				"sourceId",
				"assetId",
				"code",
			]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				sourceId: stringValue(item.sourceId),
				assetId: nullable(item.assetId, stringValue),
				code: stringValue(item.code),
			};
		}
		case "resyncRequired": {
			const item = record(value, ["kind", "selectionId"]);
			return { kind, selectionId: stringValue(item.selectionId) };
		}
		default:
			throw new DecodeError("unknown wall update");
	}
}

function cloneBreadcrumbs(
	breadcrumbs: readonly FolderBreadcrumb[],
): FolderBreadcrumb[] {
	return breadcrumbs.map(({ name, path }) => ({ name, path }));
}

function sourceFromSummary(summary: SelectionSummary): SourceSummary {
	return {
		id: summary.sourceId,
		selectionId: summary.id,
		displayName: summary.displayName,
		availability: summary.availability,
	};
}

function responseError(code: string): PhotoServiceError {
	const message = publicErrorMessages[code];
	return message ? new PhotoServiceError(code, message) : internalError();
}

async function errorFromResponse(
	response: Response,
): Promise<PhotoServiceError> {
	try {
		const body = record(await response.json(), ["code", "message"]);
		stringValue(body.message);
		return responseError(stringValue(body.code));
	} catch {
		return internalError();
	}
}

export function createHttpPhotoService(
	options: HttpPhotoServiceOptions = {},
): HttpPhotoService {
	const fetchRequest = options.fetch ?? globalThis.fetch.bind(globalThis);
	const eventSourceFactory =
		options.eventSourceFactory ?? ((url: string) => new EventSource(url));
	const preferences: BrowserPreferences = createBrowserPreferences({
		localStorage: options.localStorage ?? globalThis.localStorage,
		sessionStorage: options.sessionStorage ?? globalThis.sessionStorage,
		randomUuid: options.randomUuid ?? (() => globalThis.crypto.randomUUID()),
	});
	preferences.clientId();
	let stored = preferences.read();
	let activeSource: SourceSummary | null = null;
	let disposed = false;
	let interactionActive = false;
	let interactionTimer: ReturnType<typeof setInterval> | null = null;
	let replayContext: string | null = null;
	let replayId: string | null = null;
	let replayValue: bigint | null = null;
	const watches = new Set<ActiveWatch>();
	let folderStore: ReturnType<typeof createBrowserSavedFolders> | null = null;
	let pickStore: BrowserPicks | null = null;
	let unsubscribePickStore: (() => void) | null = null;
	let picks: PickListSnapshot = {
		revision: 0,
		items: [],
		persistenceError: null,
	};
	const pickListeners = new Set<(value: PickListSnapshot) => void>();
	const clonePicks = (value: PickListSnapshot): PickListSnapshot =>
		structuredClone(value);
	const publishPicks = (value: PickListSnapshot) => {
		picks = clonePicks(value);
		for (const listener of pickListeners) listener(clonePicks(picks));
		return clonePicks(picks);
	};
	let rootId: string | null = null;
	const folderListeners = new Set<(value: SavedFolderSnapshot) => void>();
	const access: Record<string, FolderAccess> = {};
	const deadlines = new Map<string, number>();
	const pendingChecks = new Map<string, Promise<void>>();
	const selectionIntent = createSelectionIntent();
	const savedSnapshot = () => ({
		...(folderStore?.read() ?? emptySavedFolders()),
		access: Object.fromEntries(
			Object.entries(access).map(([id, value]) => [id, { ...value }]),
		),
	});
	const publishFolders = () => {
		const value = savedSnapshot();
		for (const listener of folderListeners) listener(cloneSavedFolders(value));
		return value;
	};

	const bootstrapState = (): BootstrapState => ({
		savedFolders: savedSnapshot(),
		settings: {
			appearance: stored.appearance,
			galleryScope: stored.galleryScope,
		},
		activeSource: activeSource === null ? null : { ...activeSource },
	});

	const fetchResponse = async (
		url: string,
		init?: RequestInit,
	): Promise<Response> => {
		if (disposed) throw internalError();
		try {
			return await fetchRequest(url, init);
		} catch {
			throw internalError();
		}
	};

	const requestJson = async <T>(
		url: string,
		decode: (value: unknown) => T,
		init?: RequestInit,
	): Promise<T> => {
		const response = await fetchResponse(url, init);
		if (!response.ok) throw await errorFromResponse(response);
		try {
			return decode(await response.json());
		} catch (error) {
			if (error instanceof PhotoServiceError) throw error;
			throw internalError();
		}
	};

	const requestNoContent = async (
		url: string,
		body: unknown,
	): Promise<void> => {
		const response = await fetchResponse(url, {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify(body),
		});
		if (!response.ok) throw await errorFromResponse(response);
	};

	const selectionId = (): string => {
		if (stored.selectionId === null) {
			throw new PhotoServiceError(
				"selectionUnavailable",
				"Choose a photo folder first.",
			);
		}
		return stored.selectionId;
	};

	const interactionUrl = (): string =>
		`/api/v1/selections/${encodeURIComponent(selectionId())}/interaction`;

	const postInteraction = async (state: "active" | "idle"): Promise<void> =>
		requestNoContent(interactionUrl(), {
			clientId: preferences.clientId(),
			scope: stored.galleryScope,
			state,
		});

	const cancelInteraction = (): void => {
		if (interactionTimer !== null) {
			clearInterval(interactionTimer);
			interactionTimer = null;
		}
		interactionActive = false;
	};

	const currentReplayContext = (): string =>
		`${stored.selectionId ?? ""}\u0000${stored.galleryScope}`;

	const resetReplayForContext = (): void => {
		const context = currentReplayContext();
		if (replayContext === context) return;
		replayContext = context;
		replayId = null;
		replayValue = null;
	};

	const rememberReplayId = (value: string): void => {
		if (!/^(?:0|[1-9]\d{0,19})$/.test(value)) return;
		const parsed = BigInt(value);
		if (
			parsed > maximumU64 ||
			(replayValue !== null && parsed <= replayValue)
		) {
			return;
		}
		replayValue = parsed;
		replayId = parsed.toString();
	};

	const closeStream = (watch: ActiveWatch): void => {
		if (watch.retryTimer !== null) {
			clearTimeout(watch.retryTimer);
			watch.retryTimer = null;
		}
		if (watch.stream !== null) {
			watch.stream.onopen = null;
			watch.stream.onerror = null;
			watch.stream.close();
			watch.stream = null;
		}
	};

	const deliverResync = (watch: ActiveWatch): void => {
		if (watch.resyncDelivered || stored.selectionId === null) return;
		watch.resyncDelivered = true;
		watch.listener({
			kind: "resyncRequired",
			selectionId: stored.selectionId,
		});
	};

	const openWatch = (watch: ActiveWatch): void => {
		if (!watch.active || disposed || stored.selectionId === null) return;
		resetReplayForContext();
		const query = [
			`clientId=${encodeURIComponent(preferences.clientId())}`,
			`scope=${encodeURIComponent(stored.galleryScope)}`,
		];
		if (replayId !== null) {
			query.push(`afterEventId=${encodeURIComponent(replayId)}`);
		}
		const url = `/api/v1/selections/${encodeURIComponent(
			stored.selectionId,
		)}/events?${query.join("&")}`;
		const stream = eventSourceFactory(url);
		watch.stream = stream;
		const onUpdate = (event: MessageEvent<string>): void => {
			if (!watch.active || watch.stream !== stream) return;
			rememberReplayId(event.lastEventId);
			let update: WallUpdate;
			try {
				update = decodeWallUpdate(JSON.parse(event.data));
			} catch {
				deliverResync(watch);
				return;
			}
			if (update.kind === "resyncRequired") {
				deliverResync(watch);
				return;
			}
			if (
				update.kind === "sourceUnavailable" &&
				update.selectionId === activeSource?.selectionId
			) {
				const entry = savedSnapshot().entries.find(
					(e) => e.id === savedSnapshot().activeEntryId,
				);
				if (entry) {
					access[entry.folderId] = {
						folderId: entry.folderId,
						state: "rootOffline",
						generation: access[entry.folderId]?.generation ?? 0,
						retryAfterMs: 0,
					};
					deadlines.delete(entry.folderId);
					publishFolders();
				}
			}
			watch.resyncDelivered = false;
			try {
				watch.listener(update);
			} catch {
				watch.active = false;
				closeStream(watch);
				watches.delete(watch);
				cancelInteraction();
			}
		};
		stream.addEventListener("wallUpdate", onUpdate);
		stream.onopen = () => {
			if (!watch.active || watch.stream !== stream) return;
			watch.retryDelay = retryStartMs;
			if (interactionActive)
				void postInteraction("active").catch(() => undefined);
		};
		stream.onerror = () => {
			if (!watch.active || watch.stream !== stream) return;
			stream.removeEventListener("wallUpdate", onUpdate);
			stream.close();
			watch.stream = null;
			const delay = watch.retryDelay;
			watch.retryDelay = Math.min(watch.retryDelay * 2, retryMaximumMs);
			watch.retryTimer = setTimeout(() => {
				watch.retryTimer = null;
				openWatch(watch);
			}, delay);
		};
	};

	const reconnectWatches = (): void => {
		resetReplayForContext();
		for (const watch of watches) {
			closeStream(watch);
			watch.retryDelay = retryStartMs;
			watch.resyncDelivered = false;
			openWatch(watch);
		}
	};

	const stopSelectionResources = (): void => {
		cancelInteraction();
		for (const watch of watches) {
			watch.active = false;
			closeStream(watch);
		}
		watches.clear();
		replayContext = null;
		replayId = null;
		replayValue = null;
	};

	const storageChanged = (event: StorageEvent) => {
		pickStore?.handleStorageEvent(event);
		if (
			!folderStore ||
			(event.key !== null && !event.key.startsWith(folderStore.prefix))
		)
			return;
		if (event.newValue === null) selectionIntent.invalidate();
		const snapshot = folderStore.read();
		if (activeSource && snapshot.activeEntryId === null) {
			selectionIntent.invalidate();
			stopSelectionResources();
			activeSource = null;
			stored = preferences.update({ selectionId: null });
		} else if (activeSource) {
			const entry = snapshot.entries.find(
				(e) => e.id === snapshot.activeEntryId,
			);
			if (entry)
				activeSource = { ...activeSource, displayName: folderLabel(entry) };
		}
		publishFolders();
	};
	globalThis.addEventListener?.("storage", storageChanged);

	const service: HttpPhotoService = {
		getPicks: () => clonePicks(picks),
		watchPicks(listener) {
			pickListeners.add(listener);
			return () => pickListeners.delete(listener);
		},
		async loadPicks() {
			const store = pickStore;
			if (store === null) return clonePicks(picks);
			const storedPicks = store.read();
			const groups = new Map<string, string[]>();
			for (const item of storedPicks.items) {
				const ids = groups.get(item.sourceFolderId) ?? [];
				ids.push(item.assetId);
				groups.set(item.sourceFolderId, ids);
			}
			const resolved = new Map<string, WallAsset | null>();
			for (const [sourceFolderId, ids] of groups) {
				for (let offset = 0; offset < ids.length; offset += 250) {
					const assetIds = ids.slice(offset, offset + 250);
					const assets = await requestJson(
						`/api/v1/selections/${encodeURIComponent(`selection-${sourceFolderId}`)}/assets`,
						decodeResolvedAssets,
						{
							method: "POST",
							headers: { "content-type": "application/json" },
							body: JSON.stringify({ assetIds }),
						},
					);
					if (assets.length !== assetIds.length) throw internalError();
					for (let index = 0; index < assetIds.length; index += 1) {
						const id = assetIds[index];
						const asset = assets[index];
						if (id === undefined || asset === undefined) throw internalError();
						if (asset !== null && asset.id !== id) throw internalError();
						resolved.set(id, asset);
					}
				}
			}
			if (pickStore !== store) return clonePicks(picks);
			const latest = store.read();
			return publishPicks({
				...latest,
				items: latest.items.map((reference) => ({
					assetId: reference.assetId,
					sourceFolderId: reference.sourceFolderId,
					sourceLabel: reference.sourceLabel,
					asset: resolved.get(reference.assetId) ?? null,
				})),
			});
		},
		async addPick(reference: PickReference) {
			if (pickStore === null) return clonePicks(picks);
			pickStore.add(reference);
			return clonePicks(picks);
		},
		async removePick(assetId: string) {
			if (pickStore === null) return clonePicks(picks);
			pickStore.remove(assetId);
			return clonePicks(picks);
		},
		async clearPicks() {
			if (pickStore === null) return clonePicks(picks);
			pickStore.clear();
			return clonePicks(picks);
		},
		async restorePicks(cleared: readonly PickReference[]) {
			if (pickStore === null) return clonePicks(picks);
			pickStore.restore(cleared);
			return clonePicks(picks);
		},
		async requestPickDerivatives(request: DerivativeRequest) {
			const byId = new Map(
				picks.items.map((item) => [item.assetId, item.sourceFolderId]),
			);
			const groups = new Map<string, string[]>();
			for (const assetId of request.assetIds) {
				const sourceFolderId = byId.get(assetId);
				if (sourceFolderId === undefined)
					throw new PhotoServiceError(
						"assetNotFound",
						"That photo is no longer available.",
					);
				const ids = groups.get(sourceFolderId) ?? [];
				ids.push(assetId);
				groups.set(sourceFolderId, ids);
			}
			for (const [sourceFolderId, ids] of groups) {
				for (let offset = 0; offset < ids.length; offset += 250) {
					await requestNoContent(
						`/api/v1/selections/${encodeURIComponent(`selection-${sourceFolderId}`)}/derivatives`,
						{
							scope: "includeSubfolders",
							request: {
								...request,
								assetIds: ids.slice(offset, offset + 250),
							},
						},
					);
				}
			}
		},
		originalDownloadUrl: () => null,
		getSavedFolders: savedSnapshot,
		watchSavedFolders: (listener) => {
			folderListeners.add(listener);
			return () => {
				folderListeners.delete(listener);
			};
		},
		async renameSavedFolder(id, label) {
			const entry = savedSnapshot().entries.find((e) => e.id === id);
			if (
				!entry ||
				sourceIsUnavailable(access[entry.folderId]?.state ?? "unknown")
			)
				return bootstrapState();
			folderStore?.rename(id, label);
			if (savedSnapshot().activeEntryId === id && activeSource)
				activeSource = {
					...activeSource,
					displayName: label.trim() || entry.name,
				};
			publishFolders();
			return bootstrapState();
		},
		async removeSavedFolder(id) {
			const active = savedSnapshot().activeEntryId === id;
			folderStore?.remove(id);
			selectionIntent.invalidate();
			if (active) {
				stopSelectionResources();
				activeSource = null;
				stored = preferences.update({ selectionId: null });
			}
			publishFolders();
			return bootstrapState();
		},
		async clearActiveFolder() {
			selectionIntent.invalidate();
			stopSelectionResources();
			activeSource = null;
			stored = preferences.update({ selectionId: null });
			folderStore?.select(null);
			publishFolders();
			return bootstrapState();
		},
		async activateSavedFolder(id) {
			const intent = selectionIntent.begin();
			const entry = savedSnapshot().entries.find((e) => e.id === id);
			if (!entry) return { kind: "cancelled" };
			await service.checkSavedFolders([id]);
			if (
				!selectionIntent.isCurrent(intent) ||
				access[entry.folderId]?.state !== "available" ||
				!savedSnapshot().entries.some((e) => e.id === id)
			)
				return { kind: "cancelled" };
			return service.selectFolder(entry.displayPath);
		},
		async checkSavedFolders(ids) {
			await Promise.all(
				ids.slice(0, 256).map(async (id) => {
					const entry = savedSnapshot().entries.find((e) => e.id === id);
					if (!entry) return;
					const running = pendingChecks.get(entry.folderId);
					if (running) return running;
					if ((deadlines.get(entry.folderId) ?? 0) > Date.now()) return;
					const checking = (async () => {
						access[entry.folderId] = {
							folderId: entry.folderId,
							state: "checking",
							generation: access[entry.folderId]?.generation ?? 0,
							retryAfterMs: 0,
						};
						publishFolders();
						let result: FolderAccess;
						try {
							result = await requestJson(
								`/api/v1/selections/${encodeURIComponent(`selection-${entry.folderId}`)}/access`,
								decodeFolderAccess,
								{ signal: AbortSignal.timeout(5000) },
							);
							if (result.folderId !== entry.folderId) throw internalError();
						} catch (error) {
							result = {
								folderId: entry.folderId,
								state:
									error instanceof PhotoServiceError &&
									error.code === "notFound"
										? "missing"
										: "unverified",
								generation: access[entry.folderId]?.generation ?? 0,
								retryAfterMs: 5000,
							};
						}
						if (savedSnapshot().entries.some((e) => e.id === id)) {
							access[entry.folderId] = result;
							deadlines.set(
								entry.folderId,
								Date.now() + Math.max(0, result.retryAfterMs),
							);
						}
						publishFolders();
					})();
					pendingChecks.set(entry.folderId, checking);
					try {
						await checking;
					} finally {
						pendingChecks.delete(entry.folderId);
					}
				}),
			);
			return savedSnapshot();
		},
		capabilities: {
			chooseFolder: true,
			folderSelection: "hosted",
			locateFolder: false,
			originalAction: "none",
		},
		async getBootstrapState() {
			const bootstrapIntent = selectionIntent.begin();
			const nextRoot = await requestJson("/api/v1/bootstrap", decodeBootstrap);
			if (!selectionIntent.isCurrent(bootstrapIntent)) return bootstrapState();
			if (nextRoot !== null) {
				if (rootId !== nextRoot) {
					stopSelectionResources();
					activeSource = null;
					rootId = nextRoot;
					folderStore = createBrowserSavedFolders(
						options.localStorage ?? globalThis.localStorage,
						options.sessionStorage ?? globalThis.sessionStorage,
						rootId,
					);
					unsubscribePickStore?.();
					pickStore = createBrowserPicks(
						options.localStorage ?? globalThis.localStorage,
						rootId,
					);
					publishPicks(pickStore.read());
					unsubscribePickStore = pickStore.subscribe(publishPicks);
					for (const key of Object.keys(access)) delete access[key];
					deadlines.clear();
				}
				if (folderStore && !folderStore.migrated()) {
					const legacyId = stored.selectionId;
					if (legacyId && folderStore.read().entries.length === 0) {
						try {
							const legacy = await requestJson(
								`/api/v1/selections/${encodeURIComponent(legacyId)}`,
								decodeSelectionSummary,
							);
							if (!selectionIntent.isCurrent(bootstrapIntent))
								return bootstrapState();
							if (legacy.sourceId === rootId) {
								const entry = folderStore.save({
									folderId: legacy.folderId,
									name: legacy.displayName,
									displayPath: legacy.path,
								});
								folderStore.select(entry.id);
							}
						} catch (error) {
							if (
								!(error instanceof PhotoServiceError) ||
								error.code !== "notFound"
							)
								throw error;
						}
					}
					folderStore.markMigrated();
				}
				if (!selectionIntent.isCurrent(bootstrapIntent))
					return bootstrapState();
				const snapshot = savedSnapshot();
				const active = snapshot.entries.find(
					(e) => e.id === snapshot.activeEntryId,
				);
				if (active) {
					const restoring = service.activateSavedFolder(active.id);
					const restoreIntent = selectionIntent.current();
					const selected = await restoring;
					if (
						!selectionIntent.isCurrent(restoreIntent) &&
						selected.kind !== "selected"
					)
						return bootstrapState();
					if (selected.kind === "selected") return selected.state;
					folderStore?.select(null);
				}
				activeSource = null;
				stored = preferences.update({ selectionId: null });
				publishFolders();
				return bootstrapState();
			}
			if (stored.selectionId === null) {
				activeSource = null;
				return bootstrapState();
			}
			const id = stored.selectionId;
			const response = await fetchResponse(
				`/api/v1/selections/${encodeURIComponent(id)}`,
			);
			if (response.status === 404) {
				stopSelectionResources();
				stored = preferences.update({ selectionId: null });
				activeSource = null;
				return bootstrapState();
			}
			if (!response.ok) throw await errorFromResponse(response);
			let summary: SelectionSummary;
			try {
				summary = decodeSelectionSummary(await response.json());
			} catch {
				throw internalError();
			}
			activeSource = sourceFromSummary(summary);
			return bootstrapState();
		},
		async chooseFolder() {
			return { kind: "cancelled" };
		},
		async updateAppearance(appearance: Appearance) {
			stored = preferences.update({ appearance });
			return bootstrapState();
		},
		async updateGalleryScope(galleryScope: GalleryScope) {
			if (stored.galleryScope !== galleryScope) {
				stored = preferences.update({ galleryScope });
				reconnectWatches();
			}
			return bootstrapState();
		},
		async queryWall(request) {
			const query = [
				`scope=${encodeURIComponent(stored.galleryScope)}`,
				`direction=${encodeURIComponent(request.direction)}`,
				`limit=${encodeURIComponent(String(request.limit))}`,
			];
			if (request.cursor !== null) {
				query.push(`cursor=${encodeURIComponent(request.cursor)}`);
			}
			return requestJson(
				`/api/v1/selections/${encodeURIComponent(
					selectionId(),
				)}/wall?${query.join("&")}`,
				decodeWallPage,
			);
		},
		requestDerivatives(request: DerivativeRequest) {
			return requestNoContent(
				`/api/v1/selections/${encodeURIComponent(selectionId())}/derivatives`,
				{ scope: stored.galleryScope, request },
			);
		},
		async setWallInteraction(active: boolean) {
			if (active) {
				interactionActive = true;
				if (interactionTimer !== null) clearInterval(interactionTimer);
				interactionTimer = setInterval(() => {
					if (interactionActive) {
						void postInteraction("active").catch(() => undefined);
					}
				}, interactionRefreshMs);
				await postInteraction("active");
				return;
			}
			cancelInteraction();
			await postInteraction("idle");
		},
		watchWallUpdates(listener) {
			const watch: ActiveWatch = {
				listener,
				stream: null,
				retryTimer: null,
				retryDelay: retryStartMs,
				active: true,
				resyncDelivered: false,
			};
			watches.add(watch);
			openWatch(watch);
			return () => {
				if (!watch.active) return;
				watch.active = false;
				closeStream(watch);
				watches.delete(watch);
				cancelInteraction();
			};
		},
		derivativeUrl(reference: DerivativeReference) {
			return `/api/v1/derivatives/${encodeURIComponent(reference.key)}`;
		},
		listFolders(path: string) {
			return requestJson(
				`/api/v1/folders?path=${encodeURIComponent(path)}&includeImageCount=true`,
				decodeFolderListing,
			);
		},
		async selectFolder(path: string): Promise<ChooseFolderResult> {
			const intent = selectionIntent.begin();
			const expectedEntry = savedSnapshot().entries.find(
				(e) => e.displayPath === path,
			);
			const summary = await requestJson(
				"/api/v1/selections",
				decodeSelectionSummary,
				{
					method: "POST",
					headers: { "content-type": "application/json" },
					body: JSON.stringify({ path }),
				},
			);
			if (
				!selectionIntent.isCurrent(intent) ||
				(expectedEntry &&
					!savedSnapshot().entries.some((e) => e.id === expectedEntry.id))
			)
				return { kind: "cancelled" };
			if (rootId !== null && summary.sourceId !== rootId)
				throw new PhotoServiceError(
					"sourceChanged",
					"The photo source changed. Reload to choose a folder.",
				);
			if (stored.selectionId !== summary.id) stopSelectionResources();
			stored = preferences.update({
				selectionId: summary.id,
				breadcrumbs: cloneBreadcrumbs(summary.breadcrumbs),
			});
			activeSource = sourceFromSummary(summary);
			if (folderStore) {
				activeSource.availability = "available";
				const entry = folderStore.save({
					folderId: summary.folderId,
					name: summary.displayName,
					displayPath: summary.path,
				});
				folderStore.select(entry.id);
				activeSource.displayName = folderLabel(entry);
				access[entry.folderId] = {
					folderId: entry.folderId,
					state: "available",
					generation: access[entry.folderId]?.generation ?? 0,
					retryAfterMs: 0,
				};
				publishFolders();
			}
			return { kind: "selected", state: bootstrapState() };
		},
		folderBrowserState() {
			const breadcrumbs = cloneBreadcrumbs(stored.breadcrumbs);
			return {
				breadcrumbs,
				initialPath: breadcrumbs.at(-1)?.path ?? "",
			};
		},
		initialSortDirection() {
			return stored.sortDirection;
		},
		rememberSortDirection(sortDirection: SortDirection) {
			if (stored.sortDirection === sortDirection) return;
			stored = preferences.update({ sortDirection });
		},
		dispose() {
			if (disposed) return;
			stopSelectionResources();
			disposed = true;
			selectionIntent.invalidate();
			globalThis.removeEventListener?.("storage", storageChanged);
			unsubscribePickStore?.();
			unsubscribePickStore = null;
			pickListeners.clear();
			folderListeners.clear();
		},
	};

	return service;
}
