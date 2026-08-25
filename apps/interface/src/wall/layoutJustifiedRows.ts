import type { WallAsset } from "../services/photoService";

export interface JustifiedLayoutOptions {
	containerWidth: number;
	targetRowHeight: number;
	gap: number;
	layoutComplete: boolean;
}

/** Maximum CSS-pixel error accepted when checking fractional row geometry. */
export const LAYOUT_GEOMETRY_TOLERANCE = 1e-9;

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
	const widths: number[] = [];
	for (let index = 0; index < assets.length; index += 1) {
		const ratio = aspectRatios[index];
		if (ratio === undefined) {
			throw new Error("Wall layout assets changed during measurement");
		}
		const width =
			justified && index === assets.length - 1
				? availableWidth - widths.reduce((sum, value) => sum + value, 0)
				: ratio * rowHeight;
		if (!Number.isFinite(width) || width <= 0) {
			throw new Error("Wall tile geometry must be finite and positive");
		}
		widths.push(width);
	}

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
		width: justified
			? options.containerWidth
			: widths.reduce((sum, value) => sum + value, 0) +
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
