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

function constrainWidths(
	rawWidths: readonly number[],
	targetWidth: number,
	minimumWidth: number,
): number[] {
	const widths = rawWidths.map(() => 0);
	const flexible = new Set(rawWidths.keys());
	let remainingWidth = targetWidth;
	while (flexible.size > 0) {
		const remainingRawWidth = [...flexible].reduce(
			(sum, index) => sum + (rawWidths[index] ?? 0),
			0,
		);
		const scale = remainingWidth / remainingRawWidth;
		const newlyClamped = [...flexible].filter(
			(index) => (rawWidths[index] ?? 0) * scale < minimumWidth,
		);
		if (newlyClamped.length === 0) {
			for (const index of flexible)
				widths[index] = (rawWidths[index] ?? 0) * scale;
			break;
		}
		for (const index of newlyClamped) {
			widths[index] = minimumWidth;
			remainingWidth -= minimumWidth;
			flexible.delete(index);
		}
	}
	const widthTotal = widths.reduce((sum, width) => sum + width, 0);
	const adjustmentIndex = widths.reduce(
		(widest, width, index) => (width > (widths[widest] ?? 0) ? index : widest),
		0,
	);
	widths[adjustmentIndex] =
		(widths[adjustmentIndex] ?? 0) + targetWidth - widthTotal;
	return widths;
}

function makeRow(
	assets: readonly WallAsset[],
	options: JustifiedLayoutOptions,
	justified: boolean,
): JustifiedRow {
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

	const rowHeight = justified
		? availableWidth / sumOfAspectRatios
		: options.targetRowHeight;
	if (!Number.isFinite(rowHeight) || rowHeight <= 0) {
		throw new Error("Wall tile geometry must be finite and positive");
	}
	const rawWidths = aspectRatios.map((ratio) => ratio * rowHeight);
	const minimumWidth = Math.min(
		MINIMUM_TILE_WIDTH,
		availableWidth / assets.length,
	);
	const targetWidth = justified
		? availableWidth
		: Math.min(
				availableWidth,
				Math.max(
					rawWidths.reduce((sum, width) => sum + width, 0),
					minimumWidth * assets.length,
				),
			);
	const widths = constrainWidths(rawWidths, targetWidth, minimumWidth);
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
		width: targetWidth + options.gap * (assets.length - 1),
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
	const maximumItemsPerRow = Math.max(
		1,
		Math.floor(
			(options.containerWidth + options.gap) /
				(MINIMUM_TILE_WIDTH + options.gap),
		),
	);

	for (const asset of assets) {
		if (candidate.length >= maximumItemsPerRow) {
			rows.push(makeRow(candidate, options, true));
			candidate = [];
			candidateAspectRatio = 0;
		}
		candidate.push(asset);
		candidateAspectRatio += asset.width / asset.height;
		const availableWidth =
			options.containerWidth - options.gap * (candidate.length - 1);
		if (!Number.isFinite(availableWidth) || availableWidth <= 0) {
			throw new Error("Wall layout geometry leaves no room for assets");
		}
		const candidateHeight = availableWidth / candidateAspectRatio;
		if (candidateHeight <= options.targetRowHeight) {
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
