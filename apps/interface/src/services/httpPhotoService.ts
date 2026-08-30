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

const internalErrorMessage = "Photo Viewer could not complete that request.";
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
	const listing = record(value, ["path", "breadcrumbs", "children"]);
	return {
		path: stringValue(listing.path),
		breadcrumbs: arrayValue(listing.breadcrumbs, decodeBreadcrumb),
		children: arrayValue(listing.children, decodeFolderEntry),
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
		"id",
		"sourceId",
		"displayName",
		"breadcrumbs",
		"availability",
	]);
	return {
		id: stringValue(summary.id),
		sourceId: stringValue(summary.sourceId),
		displayName: stringValue(summary.displayName),
		breadcrumbs: arrayValue(summary.breadcrumbs, decodeBreadcrumb),
		availability: decodeAvailability(summary.availability),
	};
}

function decodeBootstrap(value: unknown): void {
	const bootstrap = record(value, ["capabilities", "sourceAvailable"]);
	const capabilities = record(bootstrap.capabilities, [
		"folderBrowser",
		"video",
	]);
	booleanValue(capabilities.folderBrowser);
	booleanValue(capabilities.video);
	booleanValue(bootstrap.sourceAvailable);
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
	]);
	return {
		items: arrayValue(page.items, decodeWallAsset),
		nextCursor: nullable(page.nextCursor, stringValue),
		orderState: enumValue(page.orderState, ["provisional", "settled"]),
		sourceWarnings: arrayValue(page.sourceWarnings, decodeWarning),
	};
}

function decodeProgress(value: unknown): ScanProgressDto {
	const progress = record(value, ["discovered", "shaped", "enriched", "total"]);
	return {
		discovered: integerValue(progress.discovered),
		shaped: integerValue(progress.shaped),
		enriched: integerValue(progress.enriched),
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
			const item = record(value, ["kind", "selectionId", "derivatives"]);
			return {
				kind,
				selectionId: stringValue(item.selectionId),
				derivatives: arrayValue(item.derivatives, decodeDerivativeReference),
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

	const bootstrapState = (): BootstrapState => ({
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

	const service: HttpPhotoService = {
		capabilities: {
			chooseFolder: true,
			folderSelection: "hosted",
			locateFolder: false,
		},
		async getBootstrapState() {
			await requestJson("/api/v1/bootstrap", decodeBootstrap);
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
				`/api/v1/folders?path=${encodeURIComponent(path)}`,
				decodeFolderListing,
			);
		},
		async selectFolder(path: string): Promise<ChooseFolderResult> {
			const summary = await requestJson(
				"/api/v1/selections",
				decodeSelectionSummary,
				{
					method: "POST",
					headers: { "content-type": "application/json" },
					body: JSON.stringify({ path }),
				},
			);
			if (stored.selectionId !== summary.id) stopSelectionResources();
			stored = preferences.update({
				selectionId: summary.id,
				breadcrumbs: cloneBreadcrumbs(summary.breadcrumbs),
			});
			activeSource = sourceFromSummary(summary);
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
		},
	};

	return service;
}
