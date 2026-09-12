import type { WallAsset } from "../services/photoService";

export interface JustifiedLayoutOptions {
	containerWidth: number;
	targetRowHeight: number;
	gap: number;
	layoutComplete: boolean;
}

/** Maximum CSS-pixel error accepted when checking fractional row geometry. */
export const LAYOUT_GEOMETRY_TOLERANCE = 1e-9;
const MINIMUM_TILE_WIDTH = 44;
const INLINE_CONTROLS_MINIMUM_WIDTH = 82;
const INLINE_CONTROLS_MINIMUM_HEIGHT = 50;
const STACKED_CONTROLS_MINIMUM_HEIGHT = 70;

export interface PositionedWallAsset {
	asset: WallAsset;
	left: number;
	width: number;
	height: number;
}

export interface JustifiedRow {
	items: PositionedWallAsset[];
	width: number;
	height: number;
	justified: boolean;
}

function assertFiniteGeometry(options: JustifiedLayoutOptions): void {
	if (
		!Number.isFinite(options.containerWidth) ||
		!Number.isFinite(options.targetRowHeight) ||
		!Number.isFinite(options.gap) ||
		options.containerWidth <= 0 ||
		options.targetRowHeight <= 0
	) {
		throw new Error("Wall layout geometry must be finite and positive");
	}
	if (options.gap < 0) {
		throw new Error("Wall layout gap must be non-negative");
	}
}

function assertAssetDimensions(asset: WallAsset): void {
	const aspectRatio = asset.width / asset.height;
	if (
		!Number.isFinite(asset.width) ||
		!Number.isFinite(asset.height) ||
		asset.width <= 0 ||
		asset.height <= 0 ||
		!Number.isFinite(aspectRatio) ||
		aspectRatio <= 0
	) {
		throw new Error("Wall asset dimensions must be positive");
	}
}

function minimumControlHeight(asset: WallAsset): number {
	const aspectRatio = asset.width / asset.height;
	const inlineHeight = Math.max(
		INLINE_CONTROLS_MINIMUM_HEIGHT,
		INLINE_CONTROLS_MINIMUM_WIDTH / aspectRatio,
	);
	const stackedHeight = Math.max(
		STACKED_CONTROLS_MINIMUM_HEIGHT,
		MINIMUM_TILE_WIDTH / aspectRatio,
	);
	return Math.min(inlineHeight, stackedHeight);
}

function rowGeometry(
	assets: readonly WallAsset[],
	options: JustifiedLayoutOptions,
): {
	aspectRatios: number[];
	availableWidth: number;
	maximumHeight: number;
	minimumHeight: number;
} {
	const aspectRatios = assets.map((asset) => asset.width / asset.height);
	const sumOfAspectRatios = aspectRatios.reduce((sum, ratio) => sum + ratio, 0);
	if (!Number.isFinite(sumOfAspectRatios) || sumOfAspectRatios <= 0) {
		throw new Error("Wall asset dimensions must be positive");
	}
	const availableWidth =
		options.containerWidth - options.gap * (assets.length - 1);
	if (!Number.isFinite(availableWidth) || availableWidth <= 0) {
		throw new Error("Wall layout geometry leaves no room for assets");
	}
	return {
		aspectRatios,
		availableWidth,
		maximumHeight: availableWidth / sumOfAspectRatios,
		minimumHeight: Math.max(...assets.map(minimumControlHeight)),
	};
}

function rowFitsControls(
	assets: readonly WallAsset[],
	options: JustifiedLayoutOptions,
): boolean {
	const { maximumHeight, minimumHeight } = rowGeometry(assets, options);
	return maximumHeight + LAYOUT_GEOMETRY_TOLERANCE >= minimumHeight;
}

function makeRow(
	assets: readonly WallAsset[],
	options: JustifiedLayoutOptions,
	justified: boolean,
): JustifiedRow {
	const { aspectRatios, maximumHeight, minimumHeight } = rowGeometry(
		assets,
		options,
	);
	const rowHeight = justified
		? maximumHeight
		: Math.min(maximumHeight, Math.max(options.targetRowHeight, minimumHeight));
	if (!Number.isFinite(rowHeight) || rowHeight <= 0) {
		throw new Error("Wall tile geometry must be finite and positive");
	}
	const widths = aspectRatios.map((ratio) => ratio * rowHeight);
	if (widths.some((width) => !Number.isFinite(width) || width <= 0))
		throw new Error("Wall tile geometry must be finite and positive");

	const items: PositionedWallAsset[] = [];
	let left = 0;
	for (let index = 0; index < assets.length; index += 1) {
		const width =
			justified && index === assets.length - 1
				? options.containerWidth - left
				: widths[index];
		const asset = assets[index];
		if (width === undefined || asset === undefined || width <= 0) {
			throw new Error("Wall layout assets changed during positioning");
		}
		items.push({ asset, left, width, height: rowHeight });
		left += width + options.gap;
	}

	return {
		items,
		width:
			widths.reduce((sum, width) => sum + width, 0) +
			options.gap * (assets.length - 1),
		height: rowHeight,
		justified,
	};
}

export function layoutJustifiedRows(
	assets: readonly WallAsset[],
	options: JustifiedLayoutOptions,
): JustifiedRow[] {
	assertFiniteGeometry(options);
	for (const asset of assets) assertAssetDimensions(asset);

	const rows: JustifiedRow[] = [];
	let candidate: WallAsset[] = [];
	let candidateAspectRatio = 0;

	for (const asset of assets) {
		candidate.push(asset);
		candidateAspectRatio += asset.width / asset.height;
		if (candidate.length > 1 && !rowFitsControls(candidate, options)) {
			const nextAsset = candidate.pop();
			if (!nextAsset)
				throw new Error("Wall layout assets changed during row formation");
			rows.push(makeRow(candidate, options, false));
			candidate = [nextAsset];
			candidateAspectRatio = nextAsset.width / nextAsset.height;
		}
		const availableWidth =
			options.containerWidth - options.gap * (candidate.length - 1);
		if (!Number.isFinite(availableWidth) || availableWidth <= 0) {
			throw new Error("Wall layout geometry leaves no room for assets");
		}
		const candidateHeight = availableWidth / candidateAspectRatio;
		if (
			candidateHeight <= options.targetRowHeight &&
			rowFitsControls(candidate, options)
		) {
			rows.push(makeRow(candidate, options, true));
			candidate = [];
			candidateAspectRatio = 0;
		}
	}

	if (candidate.length > 0 && options.layoutComplete) {
		rows.push(makeRow(candidate, options, false));
	}

	return rows;
}
