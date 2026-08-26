import type { DerivativeRequest, WallAsset } from "../services/photoService";

export type ViewerPreviewPlan = DerivativeRequest & { idle?: boolean };

/**
 * Build the bounded set of screen-preview requests for the open viewer.
 * The planner is deliberately pure: browser scheduling belongs to the hook.
 */
export function buildViewerPreviewPlan(
	assets: readonly WallAsset[],
	currentIndex: number,
): ViewerPreviewPlan[] {
	const index = Math.trunc(currentIndex);
	if (index < 0 || index >= assets.length) return [];

	const current = assets[index];
	if (!current) return [];
	const immediate: string[] = [];
	const idle: string[] = [];
	for (let distance = 1; distance <= 2; distance += 1) {
		const left = assets[index - distance];
		const right = assets[index + distance];
		if (distance === 1) {
			if (left) immediate.push(left.id);
			if (right) immediate.push(right.id);
		} else {
			if (left) idle.push(left.id);
			if (right) idle.push(right.id);
		}
	}

	const plan: ViewerPreviewPlan[] = [
		{ assetIds: [current.id], priority: "visible", kind: "screenPreview" },
	];
	if (immediate.length > 0) {
		plan.push({
			assetIds: immediate,
			priority: "nearViewport",
			kind: "screenPreview",
		});
	}
	if (idle.length > 0) {
		plan.push({
			assetIds: idle,
			priority: "nearViewport",
			kind: "screenPreview",
			idle: true,
		});
	}
	return plan;
}
