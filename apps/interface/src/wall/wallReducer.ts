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
}

export type WallAction =
	| {
			type: "catalogBatch";
			assets: readonly WallAsset[];
			orderState: OrderState;
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
};

function mergeAssets(
	current: readonly WallAsset[],
	incoming: readonly WallAsset[],
): WallAsset[] {
	const byId = new Map<string, WallAsset>();
	for (const asset of current) byId.set(asset.id, asset);
	for (const asset of incoming) {
		const previous = byId.get(asset.id);
		byId.set(asset.id, previous ? { ...previous, ...asset } : asset);
	}
	return [...byId.values()];
}

function sortProvisional(assets: readonly WallAsset[]): WallAsset[] {
	return [...assets].sort(
		(left, right) =>
			left.provisionalOrder - right.provisionalOrder ||
			left.id.localeCompare(right.id),
	);
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
			const items =
				action.orderState === "provisional" ? sortProvisional(merged) : merged;
			return {
				...state,
				items,
				orderState: action.orderState,
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
			if (state.orderState === "settled") return state;
			return {
				...state,
				items: mergeAssets([], action.assets),
				orderState: "settled",
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
