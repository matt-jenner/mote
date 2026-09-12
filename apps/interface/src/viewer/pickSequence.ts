import type { PickItem } from "../picks/pickList";
import type { WallAsset } from "../services/photoService";

/** Assets which can be reviewed, kept in the pick list's insertion order. */
export function hydratePickSequence(items: readonly PickItem[]): WallAsset[] {
	return items.flatMap((item) => (item.asset ? [item.asset] : []));
}

/**
 * Select a stable successor before the current item leaves a pick review.
 * Stale, asset-less rows stay in the list but cannot take part in review.
 */
export function nextPickAfterRemoval(
	items: readonly PickItem[],
	assetId: string,
): string | null {
	const index = items.findIndex((item) => item.assetId === assetId);
	if (index < 0) return null;
	for (const item of items.slice(index + 1)) {
		if (item.asset) return item.asset.id;
	}
	for (const item of items.slice(0, index).reverse()) {
		if (item.asset) return item.asset.id;
	}
	return null;
}
