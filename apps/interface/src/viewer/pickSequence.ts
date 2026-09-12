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
	const sequence = hydratePickSequence(items);
	const index = sequence.findIndex((item) => item.id === assetId);
	if (index < 0) return null;
	return sequence[index + 1]?.id ?? sequence[index - 1]?.id ?? null;
}
