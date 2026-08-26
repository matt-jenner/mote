import type { WallAsset } from "../services/photoService";

export interface ViewerNeighbourIds {
	immediate: string[];
	idle: string[];
}

export interface ViewerFilmstripWindow {
	start: number;
	/** Exclusive end index. */
	end: number;
}

/** Derive a bounded odd number of filmstrip items from the usable width. */
export function viewerFilmstripCapacity(
	viewportWidth: number,
	itemWidth = viewportWidth <= 639 ? 56 : 64,
	gap = 8,
): number {
	if (!Number.isFinite(viewportWidth) || viewportWidth <= 0) return 1;
	const measured = Math.floor((viewportWidth + gap) / (itemWidth + gap));
	const bounded = Math.min(31, Math.max(1, measured));
	return bounded % 2 === 0 ? Math.max(1, bounded - 1) : bounded;
}

/** Return the position of an asset in the ordered wall, or -1 when absent. */
export function findViewerIndex(
	items: readonly WallAsset[],
	assetId: string,
): number {
	return items.findIndex((item) => item.id === assetId);
}

/**
 * Split the bounded neighbourhood around an asset into adjacent and idle IDs.
 * Iterating the source range keeps both result lists in wall order.
 */
export function viewerNeighbourIds(
	items: readonly WallAsset[],
	currentIndex: number,
	idleRadius = 2,
): ViewerNeighbourIds {
	const immediate: string[] = [];
	const idle: string[] = [];
	const index = Math.trunc(currentIndex);
	const radius = Math.max(0, Math.trunc(idleRadius));

	if (index < 0 || index >= items.length || radius === 0) {
		return { immediate, idle };
	}

	const start = Math.max(0, index - radius);
	const end = Math.min(items.length, index + radius + 1);
	for (let position = start; position < end; position += 1) {
		if (position === index) continue;
		const assetId = items[position]?.id;
		if (assetId === undefined) continue;
		if (Math.abs(position - index) === 1) immediate.push(assetId);
		else idle.push(assetId);
	}

	return { immediate, idle };
}

/**
 * Return an inclusive-radius filmstrip range as [start, end), with the
 * current item retained and no more than 2 * radius + 1 items.
 */
export function viewerFilmstripWindow(
	items: readonly WallAsset[],
	currentIndex: number,
	radius: number,
): ViewerFilmstripWindow {
	const count = items.length;
	if (count === 0) return { start: 0, end: 0 };

	const boundedRadius = Math.max(0, Math.trunc(radius));
	const maxItems = boundedRadius * 2 + 1;
	const index = Math.min(count - 1, Math.max(0, Math.trunc(currentIndex)));
	let start = Math.max(0, index - boundedRadius);
	let end = Math.min(count, index + boundedRadius + 1);

	if (end - start < maxItems) {
		if (start === 0) end = Math.min(count, maxItems);
		else if (end === count) start = Math.max(0, count - maxItems);
	}

	return { start, end };
}

/** Return whether a viewer at the loaded end should request the next page. */
export function shouldLoadViewerPage(
	currentIndex: number,
	itemCount: number,
	hasNextCursor: boolean,
	isLoading: boolean,
): boolean {
	if (itemCount <= 0 || !hasNextCursor || isLoading) return false;
	return Math.trunc(currentIndex) >= Math.max(0, itemCount - 5);
}
