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
	sourceComplete: boolean;
	settled: boolean;
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
			sourceComplete: boolean;
	  }
	| { type: "derivativesReady"; derivatives: readonly DerivativeReference[] }
	| { type: "metadataSettled"; assets: readonly WallAsset[] }
	| { type: "setDirection"; direction: SortDirection };

export const initialWallState: WallState = {
	items: [],
	cursor: null,
	orderState: "provisional",
	direction: "oldestFirst",
	scrollEpoch: 0,
	sourceComplete: false,
	settled: false,
};

interface MergeResult {
	items: WallAsset[];
	changed: boolean;
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
		left.warning === right.warning &&
		left.wallThumbnail === right.wallThumbnail &&
		left.screenPreview === right.screenPreview
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

function sameDerivative(
	left: DerivativeReference | null,
	right: DerivativeReference,
): boolean {
	return (
		left?.assetId === right.assetId &&
		left.kind === right.kind &&
		left.key === right.key
	);
}

export function wallReducer(state: WallState, action: WallAction): WallState {
	switch (action.type) {
		case "catalogBatch": {
			const merged = mergeAssets(state.items, action.assets);
			if (!merged.changed && state.orderState === action.orderState)
				return state;
			const sorted =
				action.orderState === "provisional"
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			return {
				...state,
				items,
				orderState: action.orderState,
			};
		}
		case "pageLoaded": {
			const merged = mergeAssets(state.items, action.assets);
			const sorted =
				action.orderState === "provisional"
					? sortProvisional(merged.items)
					: merged.items;
			const items = reuseSequence(state.items, sorted);
			const sourceComplete = state.sourceComplete || action.sourceComplete;
			if (
				!merged.changed &&
				state.orderState === action.orderState &&
				state.cursor === action.nextCursor &&
				state.sourceComplete === sourceComplete
			) {
				return state;
			}
			return {
				...state,
				items,
				cursor: action.nextCursor,
				orderState: action.orderState,
				sourceComplete,
			};
		}
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
			if (state.settled) return state;
			const merged = mergeAssets([], action.assets);
			return {
				...state,
				items: merged.items,
				orderState: "settled",
				sourceComplete: true,
				settled: true,
			};
		}
		case "setDirection": {
			if (state.direction === action.direction) return state;
			return {
				...state,
				items: [],
				cursor: null,
				direction: action.direction,
				scrollEpoch: state.scrollEpoch + 1,
			};
		}
	}
}
