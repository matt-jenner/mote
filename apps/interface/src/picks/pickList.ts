import type { WallAsset } from "../services/photoService";

export interface PickReference {
	assetId: string;
	sourceFolderId: string;
	sourceLabel: string;
}

export interface PickItem extends PickReference {
	asset: WallAsset | null;
}

export interface PickListSnapshot {
	revision: number;
	items: PickItem[];
	persistenceError: string | null;
}

function hasAssetId(
	references: readonly PickReference[],
	assetId: string,
): boolean {
	return references.some((reference) => reference.assetId === assetId);
}

export function addPickReference(
	references: readonly PickReference[],
	reference: PickReference,
): PickReference[] {
	if (hasAssetId(references, reference.assetId)) return [...references];
	return [...references, { ...reference }];
}

export function removePickReference(
	references: readonly PickReference[],
	assetId: string,
): PickReference[] {
	return references.filter((reference) => reference.assetId !== assetId);
}

export function clearPickReferences(
	_references: readonly PickReference[],
): PickReference[] {
	return [];
}

export function restoreClearedPickReferences(
	cleared: readonly PickReference[],
	current: readonly PickReference[],
): PickReference[] {
	const restored: PickReference[] = [];
	for (const reference of cleared) {
		if (!hasAssetId(restored, reference.assetId))
			restored.push({ ...reference });
	}
	for (const reference of current) {
		if (!hasAssetId(restored, reference.assetId))
			restored.push({ ...reference });
	}
	return restored;
}
