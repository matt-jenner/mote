import type {
	DerivativeReference,
	OrderState,
	ScanProgressDto,
	SortDirection,
	WallAsset,
	WallWarningState,
} from "../services/photoService";

export interface WallState {
	items: WallAsset[];
	cursor: string | null;
	orderState: OrderState;
	direction: SortDirection;
	scrollEpoch: number;
	scanComplete: boolean;
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
	sortPending: boolean;
	derivativeRetrying: boolean;
}

export type WallRequestId = string | number;

export interface PageRequest {
	id: WallRequestId;
	cursor: string | null;
	epoch: number;
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
	| { type: "resetSource"; sourceGeneration: number; selectionId?: string }
	| { type: "retryStarted" }
	| { type: "derivativesReady"; derivatives: readonly DerivativeReference[] }
	| {
			type: "metadataSettled";
			assets: readonly WallAsset[];
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
	cursor: null,
	orderState: "provisional",
	direction: "oldestFirst",
	scrollEpoch: 0,
	scanComplete: false,
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
	sortPending: false,
	derivativeRetrying: false,
};

export function isWallLayoutComplete(
	state: Pick<WallState, "scanComplete" | "pagesExhausted">,
): boolean {
	return state.scanComplete && state.pagesExhausted;
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
): WallAsset[] {
	const byId = new Map(current.map((asset) => [asset.id, asset]));
	return incoming.map((asset) => {
		const previous = byId.get(asset.id);
		if (!previous) return asset;
		return {
			...asset,
			wallThumbnail: asset.wallThumbnail ?? previous.wallThumbnail,
			screenPreview: asset.screenPreview ?? previous.screenPreview,
		};
	});
}

interface RememberedWarningsResult {
	assets: WallAsset[];
	warnings: Record<string, NonNullable<WallAsset["warning"]>>;
	tombstones: Record<string, string>;
	changed: boolean;
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

function sortProvisional(assets: readonly WallAsset[]): WallAsset[] {
	return [...assets].sort(
		(left, right) =>
			left.provisionalOrder - right.provisionalOrder ||
			left.id.localeCompare(right.id),
	);
}

function reuseSequence(previous: WallAsset[], next: WallAsset[]): WallAsset[] {
	return sameSequence(previous, next) ? previous : next;
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
				!progressChanged
			)
				return state;
			const sorted =
				state.settledGeneration === null && action.orderState === "provisional"
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			return {
				...state,
				items,
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
			};
		}
		case "progress": {
			if (!matchesSelection(state, action.selectionId)) return state;
			if (
				state.scanProgressGeneration !== null &&
				action.generation < state.scanProgressGeneration
			)
				return state;
			return {
				...state,
				scanProgress: action.progress,
				scanProgressGeneration: action.generation,
			};
		}
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
			const remembered = rememberWarnings(
				state.assetWarnings,
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
			const replacementPage =
				firstPage && state.sortPending
					? mergeDerivativeReferences(state.items, remembered.assets)
					: remembered.assets;
			const merged = firstPage
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
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			const pagesExhausted = action.nextCursor === null;
			if (
				!merged.changed &&
				state.orderState === orderState &&
				state.cursor === action.nextCursor &&
				state.pagesExhausted === pagesExhausted &&
				state.activeRequest === null &&
				!remembered.changed &&
				!rememberedSource.changed
			) {
				return state;
			}
			return {
				...state,
				items,
				cursor: action.nextCursor,
				orderState,
				pagesExhausted,
				scanComplete: state.scanComplete || settledPage,
				settledGeneration:
					settledPage && state.settledGeneration === null
						? 1
						: state.settledGeneration,
				assetWarnings: remembered.changed
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
			return { ...state, scanComplete: true, error: action.error };
		case "derivativesReady": {
			if (action.derivatives.length === 0 || state.items.length === 0)
				return state;
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
			return changed ? { ...state, items } : state;
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
				generation <= state.settledGeneration
			)
				return { ...state, activeRequest: null };
			const remembered = rememberWarnings(
				state.assetWarnings,
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
			);
			const merged = mergeAssets([], replacementPage);
			return {
				...state,
				items: merged.items,
				cursor: action.nextCursor,
				orderState: "settled",
				scanComplete: true,
				pagesExhausted: action.nextCursor === null,
				settledGeneration: generation,
				assetWarnings: remembered.changed
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
				sourceGeneration: state.sourceGeneration,
				selectionId: action.selectionId,
			};
		case "retryStarted":
			return state.error ? { ...state, error: null } : state;
		case "setDirection": {
			if (state.direction === action.direction) return state;
			return {
				...state,
				items: state.items,
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
