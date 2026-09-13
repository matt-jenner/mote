import type {
	DerivativeReference,
	OrderState,
	ScanProgressDto,
	SortDirection,
	WallAsset,
	WallPreviewCounts,
	WallWarningState,
} from "../services/photoService";

export interface WallState {
	items: WallAsset[];
	totalCount: number | null;
	indexedCount: number | null;
	previewCounts: WallPreviewCounts | null;
	previewCountVersion: number;
	cursor: string | null;
	orderState: OrderState;
	direction: SortDirection;
	scrollEpoch: number;
	scanComplete: boolean;
	scanActive: boolean;
	pagesExhausted: boolean;
	settledGeneration: number | null;
	sourceWarnings: Record<string, NonNullable<WallAsset["warning"]>>;
	sourceWarningTombstones: Record<string, string>;
	assetWarnings: Record<string, NonNullable<WallAsset["warning"]>>;
	warningTombstones: Record<string, string>;
	selectionId: string | null;
	activeRequest: PageRequest | null;
	sourceGeneration: number;
	error: string | null;
	scanProgress: ScanProgressDto | null;
	scanProgressGeneration: number | null;
	streamedAssetGeneration: number | null;
	streamedAssetIds: Record<string, true>;
	sortPending: boolean;
	derivativeRetrying: boolean;
}

export type WallRequestId = string | number;

export interface PageRequest {
	id: WallRequestId;
	cursor: string | null;
	epoch: number;
	previewCountVersion: number;
}

export type WallAction =
	| {
			type: "catalogBatch";
			assets: readonly WallAsset[];
			orderState: OrderState;
			selectionId?: string;
			generation?: number;
			progress?: ScanProgressDto;
	  }
	| {
			type: "pageLoaded";
			assets: readonly WallAsset[];
			totalCount?: number;
			indexedCount?: number;
			previewCounts?: WallPreviewCounts;
			orderState: OrderState;
			nextCursor: string | null;
			requestCursor: string | null;
			requestEpoch: number;
			requestId: WallRequestId;
			sourceGeneration?: number;
			sourceWarnings?: readonly WallWarningState[];
	  }
	| {
			type: "pageRequestStarted";
			requestId: WallRequestId;
			requestCursor: string | null;
			requestEpoch: number;
			sourceGeneration?: number;
	  }
	| {
			type: "progress";
			selectionId: string;
			generation: number;
			progress: ScanProgressDto;
	  }
	| {
			type: "scanSettled";
			selectionId: string;
			generation: number;
	  }
	| {
			type: "derivativeRetrying";
			sourceGeneration: number;
			retrying: boolean;
	  }
	| {
			type: "pageRequestFailed";
			requestId: WallRequestId;
			requestCursor: string | null;
			requestEpoch: number;
			error: string;
			sourceGeneration?: number;
	  }
	| {
			type: "wallError";
			error: string;
			sourceGeneration?: number;
	  }
	| { type: "sourceUnavailable"; sourceGeneration?: number }
	| { type: "resetSource"; sourceGeneration: number; selectionId?: string }
	| { type: "retryStarted" }
	| {
			type: "derivativesReady";
			derivatives: readonly DerivativeReference[];
			previewCounts?: WallPreviewCounts | null;
	  }
	| {
			type: "metadataSettled";
			assets: readonly WallAsset[];
			totalCount?: number;
			indexedCount?: number;
			previewCounts?: WallPreviewCounts;
			nextCursor: string | null;
			requestEpoch: number;
			requestCursor: string | null;
			requestId: WallRequestId;
			sourceGeneration?: number;
			generation?: number;
			sourceWarnings?: readonly WallWarningState[];
	  }
	| {
			type: "warning";
			selectionId?: string;
			sourceId: string;
			assetId: string | null;
			warning: NonNullable<WallAsset["warning"]>;
	  }
	| {
			type: "warningCleared";
			selectionId?: string;
			sourceId: string;
			assetId: string | null;
			code: string;
	  }
	| { type: "resyncRequired"; selectionId: string }
	| { type: "setDirection"; direction: SortDirection };

export const initialWallState: WallState = {
	items: [],
	totalCount: null,
	indexedCount: null,
	previewCounts: null,
	previewCountVersion: 0,
	cursor: null,
	orderState: "provisional",
	direction: "oldestFirst",
	scrollEpoch: 0,
	scanComplete: false,
	scanActive: false,
	pagesExhausted: false,
	settledGeneration: null,
	sourceWarnings: {},
	sourceWarningTombstones: {},
	assetWarnings: {},
	warningTombstones: {},
	selectionId: null,
	activeRequest: null,
	sourceGeneration: 0,
	error: null,
	scanProgress: null,
	scanProgressGeneration: null,
	streamedAssetGeneration: null,
	streamedAssetIds: {},
	sortPending: false,
	derivativeRetrying: false,
};

export function isWallLayoutComplete(
	state: Pick<WallState, "scanComplete" | "pagesExhausted">,
): boolean {
	return state.scanComplete && state.pagesExhausted;
}

function mergePreviewCounts(
	current: WallPreviewCounts | null,
	incoming: WallPreviewCounts | null | undefined,
): WallPreviewCounts | null {
	if (!incoming) return current;
	if (!current) return incoming;
	return {
		wallReady: Math.max(current.wallReady, incoming.wallReady),
		screenReady: Math.max(current.screenReady, incoming.screenReady),
	};
}

function observedPreviewCounts(items: readonly WallAsset[]): WallPreviewCounts {
	return {
		wallReady: items.filter((item) => item.wallThumbnail !== null).length,
		screenReady: items.filter((item) => item.screenPreview !== null).length,
	};
}

function samePreviewCounts(
	left: WallPreviewCounts | null,
	right: WallPreviewCounts | null,
): boolean {
	return (
		left === right ||
		(left !== null &&
			right !== null &&
			left.wallReady === right.wallReady &&
			left.screenReady === right.screenReady)
	);
}

function pagePreviewCounts(
	state: WallState,
	incoming: WallPreviewCounts | undefined,
): WallPreviewCounts | null {
	if (!incoming) return state.previewCounts;
	if (state.activeRequest?.previewCountVersion === state.previewCountVersion)
		return incoming;
	return mergePreviewCounts(state.previewCounts, incoming);
}

function matchesActiveRequest(
	state: WallState,
	requestId: WallRequestId,
	requestCursor: string | null,
	requestEpoch: number,
): boolean {
	const active = state.activeRequest;
	return (
		active !== null &&
		active.id === requestId &&
		active.cursor === requestCursor &&
		active.epoch === requestEpoch &&
		requestEpoch === state.scrollEpoch
	);
}

function matchesSource(state: WallState, sourceGeneration?: number): boolean {
	return (
		sourceGeneration === undefined ||
		sourceGeneration === state.sourceGeneration
	);
}

function matchesSelection(state: WallState, selectionId?: string): boolean {
	return (
		selectionId === undefined ||
		state.selectionId === null ||
		selectionId === state.selectionId
	);
}

interface MergeResult {
	items: WallAsset[];
	changed: boolean;
}

function sameWarning(
	left: WallAsset["warning"],
	right: WallAsset["warning"],
): boolean {
	return left?.code === right?.code && left?.retryable === right?.retryable;
}

const maxWarningTombstones = 128;

function warningKey(assetId: string, code: string): string {
	return `${assetId}\u0000${code}`;
}

function requestToken(request: PageRequest | null): string | null {
	return request === null
		? null
		: `${String(request.id)}\u0000${request.epoch}\u0000${request.cursor ?? ""}`;
}

function addBoundedTombstone(
	tombstones: Record<string, string>,
	key: string,
	token: string | null,
): Record<string, string> {
	if (token === null) return tombstones;
	const next = { ...tombstones, [key]: token };
	const keys = Object.keys(next);
	while (keys.length > maxWarningTombstones) {
		const oldest = keys.shift();
		if (oldest !== undefined) delete next[oldest];
	}
	return next;
}

function sameDerivative(
	left: WallAsset["wallThumbnail"] | WallAsset["screenPreview"],
	right: WallAsset["wallThumbnail"] | WallAsset["screenPreview"],
): boolean {
	return (
		left?.assetId === right?.assetId &&
		left?.kind === right?.kind &&
		left?.key === right?.key
	);
}

function sameAsset(left: WallAsset, right: WallAsset): boolean {
	return (
		left.id === right.id &&
		left.displayName === right.displayName &&
		left.mediaKind === right.mediaKind &&
		left.provisionalOrder === right.provisionalOrder &&
		left.capturedAtUtc === right.capturedAtUtc &&
		left.dateState === right.dateState &&
		left.width === right.width &&
		left.height === right.height &&
		left.representativeRgb === right.representativeRgb &&
		left.shapeState === right.shapeState &&
		left.availability === right.availability &&
		sameWarning(left.warning, right.warning) &&
		sameDerivative(left.wallThumbnail, right.wallThumbnail) &&
		sameDerivative(left.screenPreview, right.screenPreview)
	);
}

function sameSequence(
	left: readonly WallAsset[],
	right: readonly WallAsset[],
): boolean {
	return (
		left.length === right.length &&
		left.every((asset, index) => asset === right[index])
	);
}

function mergeAssets(
	current: readonly WallAsset[],
	incoming: readonly WallAsset[],
): MergeResult {
	const byId = new Map<string, WallAsset>();
	for (const asset of current) byId.set(asset.id, asset);
	let changed = false;
	for (const asset of incoming) {
		const previous = byId.get(asset.id);
		if (!previous) {
			byId.set(asset.id, asset);
			changed = true;
			continue;
		}
		const merged = {
			...previous,
			...asset,
			provisionalOrder: previous.provisionalOrder,
		};
		if (!sameAsset(previous, merged)) {
			byId.set(asset.id, merged);
			changed = true;
		}
	}
	return { items: [...byId.values()], changed };
}

function mergeSettledAssets(
	current: readonly WallAsset[],
	settled: readonly WallAsset[],
): MergeResult {
	return mergeAssets(current, settled);
}

function replaceSettledAssets(
	current: readonly WallAsset[],
	settled: readonly WallAsset[],
): MergeResult {
	const merged = mergeAssets(current, settled);
	const byId = new Map(merged.items.map((asset) => [asset.id, asset]));
	const seen = new Set<string>();
	const items = settled.flatMap((asset) => {
		if (seen.has(asset.id)) return [];
		seen.add(asset.id);
		const retained = byId.get(asset.id);
		return retained ? [retained] : [];
	});
	return { items, changed: !sameSequence(current, items) };
}

function preserveCatalogAvailability(
	current: readonly WallAsset[],
	incoming: readonly WallAsset[],
): WallAsset[] {
	const byId = new Map(current.map((asset) => [asset.id, asset]));
	return incoming.map((asset) => {
		const previous = byId.get(asset.id);
		if (
			previous === undefined ||
			previous.availability === "available" ||
			asset.availability !== "available"
		) {
			return asset;
		}
		return { ...asset, availability: previous.availability };
	});
}

function mergeDerivativeReferences(
	current: readonly WallAsset[],
	incoming: readonly WallAsset[],
	preferCurrent = false,
): WallAsset[] {
	const byId = new Map(current.map((asset) => [asset.id, asset]));
	return incoming.map((asset) => {
		const previous = byId.get(asset.id);
		if (!previous) return asset;
		return {
			...asset,
			wallThumbnail: preferCurrent
				? (previous.wallThumbnail ?? asset.wallThumbnail)
				: (asset.wallThumbnail ?? previous.wallThumbnail),
			screenPreview: preferCurrent
				? (previous.screenPreview ?? asset.screenPreview)
				: (asset.screenPreview ?? previous.screenPreview),
		};
	});
}

interface RememberedWarningsResult {
	assets: WallAsset[];
	warnings: Record<string, NonNullable<WallAsset["warning"]>>;
	tombstones: Record<string, string>;
	changed: boolean;
}

function clearRecoveredSourceWarnings(
	current: Record<string, NonNullable<WallAsset["warning"]>>,
	incoming: readonly WallAsset[],
): Record<string, NonNullable<WallAsset["warning"]>> {
	let next = current;
	for (const asset of incoming) {
		if (
			asset.availability !== "available" ||
			asset.warning !== null ||
			current[asset.id]?.code !== "sourceUnavailable"
		) {
			continue;
		}
		if (next === current) next = { ...current };
		delete next[asset.id];
	}
	return next;
}

function rememberWarnings(
	current: Record<string, NonNullable<WallAsset["warning"]>>,
	currentTombstones: Record<string, string>,
	incoming: readonly WallAsset[],
	token: string | null,
	consumeToken: boolean,
): RememberedWarningsResult {
	const warnings = { ...current };
	const tombstones = { ...currentTombstones };
	let changed = false;
	const assets: WallAsset[] = incoming.map((asset) => {
		const incomingKey = asset.warning
			? warningKey(asset.id, asset.warning.code)
			: null;
		const isTombstoned =
			incomingKey !== null &&
			token !== null &&
			tombstones[incomingKey] === token;
		if (asset.warning && !isTombstoned) {
			if (!sameWarning(warnings[asset.id] ?? null, asset.warning)) {
				warnings[asset.id] = asset.warning;
				changed = true;
			}
		}
		const warning = warnings[asset.id];
		if (isTombstoned) return { ...asset, warning: warning ?? null };
		return warning !== undefined && asset.warning === null
			? { ...asset, warning }
			: asset;
	});
	if (consumeToken && token !== null) {
		for (const [key, value] of Object.entries(tombstones)) {
			if (value === token) delete tombstones[key];
		}
	}
	return { assets, warnings, tombstones, changed };
}

interface RememberedSourceWarningsResult {
	warnings: Record<string, NonNullable<WallAsset["warning"]>>;
	tombstones: Record<string, string>;
	changed: boolean;
}

function rememberSourceWarnings(
	current: Record<string, NonNullable<WallAsset["warning"]>>,
	currentTombstones: Record<string, string>,
	incoming: readonly WallWarningState[] | undefined,
	token: string | null,
	consumeToken: boolean,
): RememberedSourceWarningsResult {
	const tombstones = { ...currentTombstones };
	if (incoming === undefined) {
		if (consumeToken && token !== null) {
			for (const [key, value] of Object.entries(tombstones)) {
				if (value === token) delete tombstones[key];
			}
		}
		return { warnings: current, tombstones, changed: false };
	}
	const warnings = { ...current };
	let changed = false;
	for (const warning of incoming) {
		if (token !== null && currentTombstones[warning.code] === token) continue;
		if (!sameWarning(warnings[warning.code] ?? null, warning)) {
			warnings[warning.code] = warning;
			changed = true;
		}
	}
	if (consumeToken && token !== null) {
		for (const [key, value] of Object.entries(tombstones)) {
			if (value === token) delete tombstones[key];
		}
	}
	return { warnings, tombstones, changed };
}

function sortProgressive(
	assets: readonly WallAsset[],
	direction: SortDirection,
): WallAsset[] {
	return [...assets].sort((left, right) => {
		if (left.capturedAtUtc !== null && right.capturedAtUtc !== null) {
			const dateOrder = left.capturedAtUtc.localeCompare(right.capturedAtUtc);
			if (dateOrder !== 0)
				return direction === "newestFirst" ? -dateOrder : dateOrder;
		} else if (left.capturedAtUtc !== null) {
			return -1;
		} else if (right.capturedAtUtc !== null) {
			return 1;
		}
		return (
			left.provisionalOrder - right.provisionalOrder ||
			left.id.localeCompare(right.id)
		);
	});
}

function reuseSequence(previous: WallAsset[], next: WallAsset[]): WallAsset[] {
	return sameSequence(previous, next) ? previous : next;
}

interface StreamedAssetState {
	generation: number | null;
	ids: Record<string, true>;
	changed: boolean;
}

function rememberStreamedAssets(
	currentGeneration: number | null,
	currentIds: Record<string, true>,
	generation: number | undefined,
	assets: readonly WallAsset[],
): StreamedAssetState {
	if (
		generation === undefined ||
		(currentGeneration !== null && generation < currentGeneration)
	) {
		return { generation: currentGeneration, ids: currentIds, changed: false };
	}
	if (currentGeneration === null || generation > currentGeneration) {
		return {
			generation,
			ids: Object.fromEntries(assets.map((asset) => [asset.id, true])),
			changed: true,
		};
	}
	let ids = currentIds;
	for (const asset of assets) {
		if (ids[asset.id]) continue;
		if (ids === currentIds) ids = { ...currentIds };
		ids[asset.id] = true;
	}
	return { generation, ids, changed: ids !== currentIds };
}

export function wallReducer(state: WallState, action: WallAction): WallState {
	switch (action.type) {
		case "pageRequestStarted": {
			if (!matchesSource(state, action.sourceGeneration)) return state;
			if (action.requestEpoch !== state.scrollEpoch) return state;
			if (
				action.requestCursor !== null &&
				action.requestCursor !== state.cursor
			) {
				return state;
			}
			const activeRequest: PageRequest = {
				id: action.requestId,
				cursor: action.requestCursor,
				epoch: action.requestEpoch,
				previewCountVersion: state.previewCountVersion,
			};
			if (
				state.activeRequest?.id === activeRequest.id &&
				state.activeRequest.cursor === activeRequest.cursor &&
				state.activeRequest.epoch === activeRequest.epoch
			) {
				return state;
			}
			return {
				...state,
				activeRequest,
				warningTombstones: {},
				sourceWarningTombstones: {},
			};
		}
		case "catalogBatch": {
			if (!matchesSelection(state, action.selectionId)) return state;
			const streamed = rememberStreamedAssets(
				state.streamedAssetGeneration,
				state.streamedAssetIds,
				action.generation,
				action.assets,
			);
			const matchingAssets = state.sortPending
				? action.assets.filter((asset) =>
						state.items.some((current) => current.id === asset.id),
					)
				: action.assets;
			const catalogAssets = preserveCatalogAvailability(
				state.items,
				matchingAssets,
			);
			const remembered = rememberWarnings(
				state.assetWarnings,
				state.warningTombstones,
				catalogAssets,
				requestToken(state.activeRequest),
				false,
			);
			const merged = mergeAssets(state.items, remembered.assets);
			const orderState =
				state.settledGeneration !== null ? "settled" : action.orderState;
			const acceptsProgress =
				action.progress !== undefined &&
				(action.generation === undefined ||
					state.scanProgressGeneration === null ||
					action.generation >= state.scanProgressGeneration);
			const progressChanged =
				acceptsProgress &&
				(state.scanProgressGeneration !== action.generation ||
					JSON.stringify(state.scanProgress) !==
						JSON.stringify(action.progress));
			if (
				!merged.changed &&
				state.orderState === orderState &&
				!remembered.changed &&
				!progressChanged &&
				!streamed.changed
			)
				return state;
			const sorted =
				state.settledGeneration === null && action.orderState === "provisional"
					? sortProgressive(merged.items, state.direction)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			const indexedCount = Math.max(
				state.indexedCount ?? 0,
				action.progress?.indexedCount ?? action.progress?.shaped ?? 0,
				merged.items.length,
			);
			return {
				...state,
				items,
				scanActive: true,
				indexedCount,
				totalCount:
					acceptsProgress && action.progress?.total !== null
						? (action.progress?.total ?? state.totalCount)
						: state.totalCount,
				orderState,
				assetWarnings: remembered.changed
					? remembered.warnings
					: state.assetWarnings,
				warningTombstones: remembered.tombstones,
				scanProgress: acceptsProgress
					? (action.progress ?? state.scanProgress)
					: state.scanProgress,
				scanProgressGeneration: acceptsProgress
					? (action.generation ?? state.scanProgressGeneration)
					: state.scanProgressGeneration,
				streamedAssetGeneration: streamed.generation,
				streamedAssetIds: streamed.ids,
			};
		}
		case "progress": {
			if (!matchesSelection(state, action.selectionId)) return state;
			if (
				state.scanProgressGeneration !== null &&
				action.generation < state.scanProgressGeneration
			)
				return state;
			const startsNewStream =
				state.streamedAssetGeneration === null ||
				action.generation > state.streamedAssetGeneration;
			return {
				...state,
				scanActive: true,
				scanProgress: action.progress,
				scanProgressGeneration: action.generation,
				totalCount: action.progress.total ?? state.totalCount,
				indexedCount: Math.max(
					state.indexedCount ?? 0,
					action.progress.indexedCount ?? action.progress.shaped,
				),
				streamedAssetGeneration: startsNewStream
					? action.generation
					: state.streamedAssetGeneration,
				streamedAssetIds: startsNewStream ? {} : state.streamedAssetIds,
			};
		}
		case "scanSettled":
			if (!matchesSelection(state, action.selectionId)) return state;
			if (
				state.scanProgressGeneration !== null &&
				action.generation < state.scanProgressGeneration
			)
				return state;
			return state.scanActive ? { ...state, scanActive: false } : state;
		case "derivativeRetrying":
			if (!matchesSource(state, action.sourceGeneration)) return state;
			return state.derivativeRetrying === action.retrying
				? state
				: { ...state, derivativeRetrying: action.retrying };
		case "pageLoaded": {
			if (!matchesSource(state, action.sourceGeneration)) return state;
			if (
				!matchesActiveRequest(
					state,
					action.requestId,
					action.requestCursor,
					action.requestEpoch,
				)
			)
				return state;
			const firstPage = action.requestCursor === null;
			const settledPage = action.orderState === "settled";
			const recoveredWarnings = clearRecoveredSourceWarnings(
				state.assetWarnings,
				action.assets,
			);
			const remembered = rememberWarnings(
				recoveredWarnings,
				state.warningTombstones,
				action.assets,
				requestToken(state.activeRequest),
				true,
			);
			const rememberedSource = rememberSourceWarnings(
				state.sourceWarnings,
				state.sourceWarningTombstones,
				action.sourceWarnings,
				requestToken(state.activeRequest),
				true,
			);
			const liveUpdateRacedPage =
				state.activeRequest?.previewCountVersion !== state.previewCountVersion;
			const replacementPage =
				firstPage && (state.sortPending || liveUpdateRacedPage)
					? mergeDerivativeReferences(
							state.items,
							remembered.assets,
							liveUpdateRacedPage,
						)
					: remembered.assets;
			const preservesSettledRemainder =
				firstPage && state.settledGeneration !== null && !state.sortPending;
			const merged =
				firstPage && !preservesSettledRemainder
					? mergeAssets([], replacementPage)
					: mergeAssets(state.items, remembered.assets);
			const orderState =
				state.settledGeneration !== null || settledPage
					? "settled"
					: action.orderState;
			const sorted =
				state.settledGeneration === null &&
				!settledPage &&
				action.orderState === "provisional"
					? sortProgressive(merged.items, state.direction)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			const pagesExhausted = action.nextCursor === null;
			const totalCount = settledPage
				? preservesSettledRemainder
					? Math.max(
							state.totalCount ?? 0,
							action.totalCount ?? 0,
							merged.items.length,
						)
					: (action.totalCount ?? merged.items.length)
				: (state.scanProgress?.total ??
					action.totalCount ??
					Math.max(state.totalCount ?? 0, merged.items.length));
			const previewCounts = pagePreviewCounts(state, action.previewCounts);
			const indexedCount = Math.max(
				state.indexedCount ?? 0,
				action.indexedCount ?? 0,
				merged.items.length,
			);
			if (
				!merged.changed &&
				state.orderState === orderState &&
				state.cursor === action.nextCursor &&
				state.pagesExhausted === pagesExhausted &&
				state.totalCount === totalCount &&
				state.indexedCount === indexedCount &&
				samePreviewCounts(state.previewCounts, previewCounts) &&
				state.activeRequest === null &&
				!remembered.changed &&
				!rememberedSource.changed
			) {
				return state;
			}
			return {
				...state,
				items,
				totalCount,
				indexedCount,
				previewCounts,
				cursor: action.nextCursor,
				orderState,
				pagesExhausted,
				scanComplete: state.scanComplete || settledPage,
				settledGeneration:
					settledPage && state.settledGeneration === null
						? 1
						: state.settledGeneration,
				assetWarnings:
					remembered.changed || recoveredWarnings !== state.assetWarnings
						? remembered.warnings
						: state.assetWarnings,
				warningTombstones: remembered.tombstones,
				sourceWarnings: rememberedSource.warnings,
				sourceWarningTombstones: rememberedSource.tombstones,
				activeRequest: null,
				error: null,
				sortPending: firstPage ? false : state.sortPending,
				derivativeRetrying: firstPage ? false : state.derivativeRetrying,
			};
		}
		case "pageRequestFailed": {
			if (!matchesSource(state, action.sourceGeneration)) return state;
			if (
				!matchesActiveRequest(
					state,
					action.requestId,
					action.requestCursor,
					action.requestEpoch,
				)
			)
				return state;
			return { ...state, activeRequest: null, error: action.error };
		}
		case "wallError":
			if (!matchesSource(state, action.sourceGeneration)) return state;
			return {
				...state,
				scanComplete: true,
				scanActive: false,
				error: action.error,
			};
		case "sourceUnavailable": {
			if (!matchesSource(state, action.sourceGeneration)) return state;
			if (state.items.length === 0) {
				return {
					...state,
					scanComplete: true,
					scanActive: false,
					error: "Source unavailable. Try again.",
				};
			}
			const warning = { code: "sourceUnavailable", retryable: true };
			const assetWarnings = { ...state.assetWarnings };
			const items = state.items.map((asset) => {
				assetWarnings[asset.id] = warning;
				return {
					...asset,
					availability: "rootOffline" as const,
					warning,
				};
			});
			return {
				...state,
				items,
				assetWarnings,
				scanComplete: true,
				scanActive: false,
				error: null,
			};
		}
		case "derivativesReady": {
			if (action.derivatives.length === 0 || state.items.length === 0)
				return action.previewCounts
					? {
							...state,
							previewCounts: mergePreviewCounts(
								state.previewCounts,
								action.previewCounts,
							),
							previewCountVersion: state.previewCountVersion + 1,
						}
					: state;
			let changed = false;
			const derivativeByAssetAndKind = new Map(
				action.derivatives.map((derivative) => [
					`${derivative.assetId}\u0000${derivative.kind}`,
					derivative,
				]),
			);
			const items = state.items.map((asset) => {
				let nextAsset = asset;
				for (const kind of ["wallThumbnail", "screenPreview"] as const) {
					const derivative = derivativeByAssetAndKind.get(
						`${asset.id}\u0000${kind}`,
					);
					if (!derivative) continue;
					const field = kind;
					if (sameDerivative(nextAsset[field], derivative)) continue;
					changed = true;
					nextAsset = { ...nextAsset, [field]: derivative };
				}
				return nextAsset;
			});
			const previewCounts = mergePreviewCounts(
				mergePreviewCounts(
					state.previewCounts,
					changed ? observedPreviewCounts(items) : null,
				),
				action.previewCounts,
			);
			const previewCountChanged = !samePreviewCounts(
				state.previewCounts,
				previewCounts,
			);
			return changed || previewCounts !== state.previewCounts
				? {
						...state,
						...(changed ? { items } : {}),
						previewCounts,
						previewCountVersion:
							state.previewCountVersion +
							(action.previewCounts || previewCountChanged || changed ? 1 : 0),
					}
				: state;
		}
		case "metadataSettled": {
			if (!matchesSource(state, action.sourceGeneration)) return state;
			if (
				!matchesActiveRequest(
					state,
					action.requestId,
					action.requestCursor,
					action.requestEpoch,
				)
			)
				return state;
			const generation = action.generation ?? 1;
			if (
				state.settledGeneration !== null &&
				generation < state.settledGeneration
			)
				return { ...state, activeRequest: null };
			const recoveredWarnings = clearRecoveredSourceWarnings(
				state.assetWarnings,
				action.assets,
			);
			const remembered = rememberWarnings(
				recoveredWarnings,
				state.warningTombstones,
				action.assets,
				requestToken(state.activeRequest),
				true,
			);
			const rememberedSource = rememberSourceWarnings(
				state.sourceWarnings,
				state.sourceWarningTombstones,
				action.sourceWarnings,
				requestToken(state.activeRequest),
				true,
			);
			const replacementPage = mergeDerivativeReferences(
				state.items,
				remembered.assets,
				state.activeRequest?.previewCountVersion !== state.previewCountVersion,
			);
			const replacesCompleteGeneration =
				state.settledGeneration !== null &&
				generation > state.settledGeneration &&
				action.requestCursor === null;
			const settledIds = new Set(action.assets.map((asset) => asset.id));
			const protectsTrackedStream =
				replacesCompleteGeneration &&
				state.streamedAssetGeneration !== null &&
				state.streamedAssetGeneration >= generation;
			const streamedRemainder = protectsTrackedStream
				? state.items.filter(
						(item) =>
							state.streamedAssetIds[item.id] && !settledIds.has(item.id),
					)
				: [];
			const merged = replacesCompleteGeneration
				? replaceSettledAssets(state.items, [
						...replacementPage,
						...streamedRemainder,
					])
				: mergeSettledAssets(state.items, replacementPage);
			const items = reuseSequence(state.items, merged.items);
			const previewCounts = pagePreviewCounts(state, action.previewCounts);
			const preservesLoadedRemainder =
				!replacesCompleteGeneration &&
				state.items.some((current) => !settledIds.has(current.id));
			const preservePagination =
				!replacesCompleteGeneration &&
				(state.pagesExhausted || preservesLoadedRemainder);
			const preservesNewerStream =
				state.streamedAssetGeneration !== null &&
				state.streamedAssetGeneration > generation;
			return {
				...state,
				items,
				totalCount: replacesCompleteGeneration
					? (action.totalCount ?? merged.items.length)
					: Math.max(
							state.totalCount ?? 0,
							action.totalCount ?? 0,
							merged.items.length,
						),
				indexedCount: preservesNewerStream
					? Math.max(
							state.indexedCount ?? 0,
							action.indexedCount ?? 0,
							merged.items.length,
						)
					: (action.indexedCount ??
						Math.max(state.indexedCount ?? 0, merged.items.length)),
				previewCounts,
				cursor: preservePagination ? state.cursor : action.nextCursor,
				orderState: "settled",
				scanComplete: true,
				scanActive: preservesNewerStream ? state.scanActive : false,
				pagesExhausted: preservePagination
					? state.pagesExhausted
					: action.nextCursor === null,
				settledGeneration: Math.max(state.settledGeneration ?? 0, generation),
				streamedAssetGeneration: preservesNewerStream
					? state.streamedAssetGeneration
					: generation,
				streamedAssetIds: preservesNewerStream ? state.streamedAssetIds : {},
				assetWarnings:
					remembered.changed || recoveredWarnings !== state.assetWarnings
						? remembered.warnings
						: state.assetWarnings,
				warningTombstones: remembered.tombstones,
				sourceWarnings: rememberedSource.warnings,
				sourceWarningTombstones: rememberedSource.tombstones,
				activeRequest: null,
				error: null,
				sortPending: false,
				derivativeRetrying: false,
			};
		}
		case "resetSource":
			return {
				...initialWallState,
				direction: state.direction,
				scanActive:
					action.selectionId !== undefined &&
					action.selectionId === state.selectionId &&
					state.scanActive,
				scanComplete:
					state.selectionId !== null &&
					action.selectionId === state.selectionId &&
					state.scanComplete,
				sourceGeneration: action.sourceGeneration,
				selectionId: action.selectionId ?? null,
			};
		case "warning": {
			if (!matchesSelection(state, action.selectionId)) return state;
			if (action.assetId === null) {
				const sourceWarningTombstones = { ...state.sourceWarningTombstones };
				delete sourceWarningTombstones[action.warning.code];
				return {
					...state,
					sourceWarnings: {
						...state.sourceWarnings,
						[action.warning.code]: action.warning,
					},
					sourceWarningTombstones,
				};
			}
			if (
				action.warning.code === "wallThumbnailUnavailable" &&
				!action.warning.retryable
			) {
				const removed = state.items.find(
					(asset) => asset.id === action.assetId,
				);
				if (!removed) return state;
				const assetWarnings = { ...state.assetWarnings };
				delete assetWarnings[action.assetId];
				return {
					...state,
					items: state.items.filter((asset) => asset.id !== action.assetId),
					totalCount:
						state.totalCount === null
							? null
							: Math.max(0, state.totalCount - 1),
					previewCounts:
						state.previewCounts === null
							? null
							: {
									wallReady: Math.max(
										0,
										state.previewCounts.wallReady -
											(removed.wallThumbnail === null ? 0 : 1),
									),
									screenReady: Math.max(
										0,
										state.previewCounts.screenReady -
											(removed.screenPreview === null ? 0 : 1),
									),
								},
					assetWarnings,
				};
			}
			const assetWarnings = {
				...state.assetWarnings,
				[action.assetId]: action.warning,
			};
			const warningTombstones = { ...state.warningTombstones };
			delete warningTombstones[warningKey(action.assetId, action.warning.code)];
			const items = state.items.map((asset) =>
				asset.id === action.assetId
					? { ...asset, warning: action.warning }
					: asset,
			);
			return { ...state, assetWarnings, warningTombstones, items };
		}
		case "warningCleared": {
			if (!matchesSelection(state, action.selectionId)) return state;
			if (action.assetId === null) {
				const sourceWarnings = { ...state.sourceWarnings };
				delete sourceWarnings[action.code];
				const sourceWarningTombstones = addBoundedTombstone(
					state.sourceWarningTombstones,
					action.code,
					requestToken(state.activeRequest),
				);
				return { ...state, sourceWarnings, sourceWarningTombstones };
			}
			const current = state.assetWarnings[action.assetId];
			const assetWarnings = { ...state.assetWarnings };
			if (current?.code === action.code) delete assetWarnings[action.assetId];
			const warningTombstones = addBoundedTombstone(
				state.warningTombstones,
				warningKey(action.assetId, action.code),
				requestToken(state.activeRequest),
			);
			const items = state.items.map((asset) =>
				asset.id === action.assetId && asset.warning?.code === action.code
					? { ...asset, warning: null }
					: asset,
			);
			return { ...state, assetWarnings, warningTombstones, items };
		}
		case "resyncRequired":
			return {
				...initialWallState,
				direction: state.direction,
				scrollEpoch: state.scrollEpoch + 1,
				scanComplete: state.scanComplete,
				scanActive: state.scanActive,
				sourceGeneration: state.sourceGeneration,
				selectionId: action.selectionId,
			};
		case "retryStarted":
			return state.error ? { ...state, error: null } : state;
		case "setDirection": {
			if (state.direction === action.direction) return state;
			return {
				...state,
				items: sortProgressive(state.items, action.direction),
				cursor: null,
				pagesExhausted: false,
				activeRequest: null,
				error: null,
				direction: action.direction,
				scrollEpoch: state.scrollEpoch + 1,
				sortPending: true,
				derivativeRetrying: false,
			};
		}
	}
}
