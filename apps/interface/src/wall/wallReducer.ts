import type {
	DerivativeReference,
	OrderState,
	SortDirection,
	WallAsset,
} from "../services/photoService";

export interface WallState {
	items: WallAsset[];
	cursor: string | null;
	orderState: OrderState;
	direction: SortDirection;
	scrollEpoch: number;
	scanComplete: boolean;
	pagesExhausted: boolean;
	settled: boolean;
	activeRequest: PageRequest | null;
	sourceGeneration: number;
	error: string | null;
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
	  }
	| {
			type: "pageRequestStarted";
			requestId: WallRequestId;
			requestCursor: string | null;
			requestEpoch: number;
			sourceGeneration?: number;
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
	| { type: "resetSource"; sourceGeneration: number }
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
	  }
	| { type: "setDirection"; direction: SortDirection };

export const initialWallState: WallState = {
	items: [],
	cursor: null,
	orderState: "provisional",
	direction: "oldestFirst",
	scrollEpoch: 0,
	scanComplete: false,
	pagesExhausted: false,
	settled: false,
	activeRequest: null,
	sourceGeneration: 0,
	error: null,
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
			return { ...state, activeRequest };
		}
		case "catalogBatch": {
			const merged = mergeAssets(state.items, action.assets);
			const orderState = state.settled ? "settled" : action.orderState;
			if (!merged.changed && state.orderState === orderState) return state;
			const sorted =
				!state.settled && action.orderState === "provisional"
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			return {
				...state,
				items,
				orderState,
			};
		}
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
			const merged = firstPage
				? mergeAssets([], action.assets)
				: mergeAssets(state.items, action.assets);
			const orderState =
				state.settled || settledPage ? "settled" : action.orderState;
			const sorted =
				!state.settled && !settledPage && action.orderState === "provisional"
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			const pagesExhausted = action.nextCursor === null;
			if (
				!merged.changed &&
				state.orderState === orderState &&
				state.cursor === action.nextCursor &&
				state.pagesExhausted === pagesExhausted &&
				state.activeRequest === null
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
				settled: state.settled || settledPage,
				activeRequest: null,
				error: null,
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
			return { ...state, activeRequest: null, error: action.error };
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
			if (state.settled) return { ...state, activeRequest: null };
			const merged = mergeAssets([], action.assets);
			return {
				...state,
				items: merged.items,
				cursor: action.nextCursor,
				orderState: "settled",
				scanComplete: true,
				pagesExhausted: action.nextCursor === null,
				settled: true,
				activeRequest: null,
				error: null,
			};
		}
		case "resetSource":
			return {
				...initialWallState,
				sourceGeneration: action.sourceGeneration,
			};
		case "retryStarted":
			return state.error ? { ...state, error: null } : state;
		case "setDirection": {
			if (state.direction === action.direction) return state;
			return {
				...state,
				items: [],
				cursor: null,
				pagesExhausted: false,
				activeRequest: null,
				error: null,
				direction: action.direction,
				scrollEpoch: state.scrollEpoch + 1,
			};
		}
	}
}
