export interface ViewerPoint {
	x: number;
	y: number;
}

export interface ViewerTransformState {
	assetRevision: number;
	scale: number;
	focal: ViewerPoint;
}

export interface ViewerTransformContext {
	viewportWidth: number;
	viewportHeight: number;
	imageWidth: number;
	imageHeight: number;
	naturalWidth: number;
	naturalHeight: number;
}

export interface ViewerTransformGeometry {
	mode: "fit" | "zoomed";
	fitWidth: number;
	fitHeight: number;
	scale: number;
	maxScale: number;
	translateX: number;
	translateY: number;
	focal: ViewerPoint;
	visibleImageRect: { x: number; y: number; width: number; height: number };
}

export interface ViewerFrameRect {
	width: number;
	height: number;
}

export function fitViewerFrame(
	containerWidth: number,
	containerHeight: number,
	assetWidth: number,
	assetHeight: number,
): ViewerFrameRect {
	if (
		!Number.isFinite(containerWidth) ||
		!Number.isFinite(containerHeight) ||
		!Number.isFinite(assetWidth) ||
		!Number.isFinite(assetHeight) ||
		containerWidth <= 0 ||
		containerHeight <= 0 ||
		assetWidth <= 0 ||
		assetHeight <= 0
	)
		return { width: 0, height: 0 };
	const scale = Math.min(
		containerWidth / assetWidth,
		containerHeight / assetHeight,
	);
	return {
		width: assetWidth * scale,
		height: assetHeight * scale,
	};
}

export const FIT_VIEWER_TRANSFORM: Omit<ViewerTransformState, "assetRevision"> =
	{
		scale: 1,
		focal: { x: 0.5, y: 0.5 },
	};

const FIT_FOCAL = FIT_VIEWER_TRANSFORM.focal;

function clamp(value: number, minimum: number, maximum: number): number {
	return Math.min(maximum, Math.max(minimum, value));
}

function finitePositive(value: number): boolean {
	return Number.isFinite(value) && value > 0;
}

function validContext(
	context: ViewerTransformContext | null | undefined,
): context is ViewerTransformContext {
	if (!context) return false;
	return (
		finitePositive(context.viewportWidth) &&
		finitePositive(context.viewportHeight) &&
		finitePositive(context.imageWidth) &&
		finitePositive(context.imageHeight) &&
		finitePositive(context.naturalWidth) &&
		finitePositive(context.naturalHeight)
	);
}

function safeFocal(point: ViewerPoint | null | undefined): ViewerPoint {
	const x = point?.x ?? Number.NaN;
	const y = point?.y ?? Number.NaN;
	return {
		x: Number.isFinite(x) ? clamp(x, 0, 1) : FIT_FOCAL.x,
		y: Number.isFinite(y) ? clamp(y, 0, 1) : FIT_FOCAL.y,
	};
}

function safeState(
	state: ViewerTransformState | null | undefined,
): ViewerTransformState {
	return {
		assetRevision: Number.isFinite(state?.assetRevision)
			? (state?.assetRevision ?? 0)
			: 0,
		scale: Number.isFinite(state?.scale) ? Math.max(1, state?.scale ?? 1) : 1,
		focal: safeFocal(state?.focal),
	};
}

function safeRevision(state: ViewerTransformState | null | undefined): number {
	return safeState(state).assetRevision;
}

function fitForContext(context: ViewerTransformContext): ViewerFrameRect {
	return fitViewerFrame(
		context.viewportWidth,
		context.viewportHeight,
		context.imageWidth,
		context.imageHeight,
	);
}

function maxScaleFor(
	context: ViewerTransformContext,
	fit: ViewerFrameRect,
): number {
	if (!finitePositive(fit.width) || !finitePositive(fit.height)) return 1;
	return Math.max(
		1,
		Math.min(
			context.naturalWidth / fit.width,
			context.naturalHeight / fit.height,
		),
	);
}

function safeGeometry(): ViewerTransformGeometry {
	return {
		mode: "fit",
		fitWidth: 0,
		fitHeight: 0,
		scale: 1,
		maxScale: 1,
		translateX: 0,
		translateY: 0,
		focal: { ...FIT_FOCAL },
		visibleImageRect: { x: 0, y: 0, width: 0, height: 0 },
	};
}

function axisTranslation(
	viewport: number,
	dimension: number,
	focal: number,
): number {
	if (dimension <= viewport) return 0;
	const excess = dimension - viewport;
	return clamp((0.5 - focal) * dimension, -excess / 2, excess / 2);
}

function axisVisibleRect(
	viewport: number,
	dimension: number,
	translation: number,
): { start: number; size: number } {
	if (!finitePositive(dimension)) return { start: 0, size: 0 };
	const size = Math.min(1, viewport / dimension);
	const left = (viewport - dimension) / 2 + translation;
	const start = clamp(-left / dimension, 0, 1 - size);
	return { start, size };
}

export function resetViewerTransform(
	assetRevision: number,
): ViewerTransformState {
	return {
		assetRevision: Number.isFinite(assetRevision) ? assetRevision : 0,
		scale: FIT_VIEWER_TRANSFORM.scale,
		focal: { ...FIT_FOCAL },
	};
}

export function deriveViewerTransform(
	state: ViewerTransformState,
	context: ViewerTransformContext,
): ViewerTransformGeometry {
	if (!validContext(context)) return safeGeometry();
	const fit = fitForContext(context);
	if (!finitePositive(fit.width) || !finitePositive(fit.height))
		return safeGeometry();
	const maxScale = maxScaleFor(context, fit);
	const safe = safeState(state);
	const scale = clamp(safe.scale, 1, maxScale);
	const focal = safe.focal;
	const width = fit.width * scale;
	const height = fit.height * scale;
	const translateX = axisTranslation(context.viewportWidth, width, focal.x);
	const translateY = axisTranslation(context.viewportHeight, height, focal.y);
	const visibleX = axisVisibleRect(context.viewportWidth, width, translateX);
	const visibleY = axisVisibleRect(context.viewportHeight, height, translateY);
	return {
		mode: scale > 1 ? "zoomed" : "fit",
		fitWidth: fit.width,
		fitHeight: fit.height,
		scale,
		maxScale,
		translateX,
		translateY,
		focal: { ...focal },
		visibleImageRect: {
			x: visibleX.start,
			y: visibleY.start,
			width: visibleX.size,
			height: visibleY.size,
		},
	};
}

export function imagePointAtViewportPoint(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	point: ViewerPoint,
): ViewerPoint {
	if (
		!validContext(context) ||
		!Number.isFinite(point?.x) ||
		!Number.isFinite(point?.y)
	)
		return { ...FIT_FOCAL };
	const geometry = deriveViewerTransform(state, context);
	if (
		!finitePositive(geometry.fitWidth) ||
		!finitePositive(geometry.fitHeight)
	) {
		return { ...FIT_FOCAL };
	}
	return {
		x: clamp(
			(point.x -
				((context.viewportWidth - geometry.fitWidth * geometry.scale) / 2 +
					geometry.translateX)) /
				(geometry.fitWidth * geometry.scale),
			0,
			1,
		),
		y: clamp(
			(point.y -
				((context.viewportHeight - geometry.fitHeight * geometry.scale) / 2 +
					geometry.translateY)) /
				(geometry.fitHeight * geometry.scale),
			0,
			1,
		),
	};
}

function stateWithTranslation(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	geometry: ViewerTransformGeometry,
	translateX: number,
	translateY: number,
): ViewerTransformState {
	const width = geometry.fitWidth * geometry.scale;
	const height = geometry.fitHeight * geometry.scale;
	const focal = {
		x:
			width > context.viewportWidth
				? clamp(0.5 - translateX / width, 0, 1)
				: geometry.focal.x,
		y:
			height > context.viewportHeight
				? clamp(0.5 - translateY / height, 0, 1)
				: geometry.focal.y,
	};
	return {
		assetRevision: safeState(state).assetRevision,
		scale: geometry.scale,
		focal,
	};
}

export function zoomViewerAt(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	nextScale: number,
	anchor: ViewerPoint,
): ViewerTransformState {
	if (
		!validContext(context) ||
		!Number.isFinite(nextScale) ||
		!Number.isFinite(anchor?.x) ||
		!Number.isFinite(anchor?.y)
	)
		return resetViewerTransform(safeRevision(state));
	const geometry = deriveViewerTransform(state, context);
	const fit = fitForContext(context);
	const maxScale = maxScaleFor(context, fit);
	const scale = clamp(nextScale, 1, maxScale);
	const assetRevision = safeState(state).assetRevision;
	if (scale <= 1) return { ...resetViewerTransform(assetRevision) };
	const imagePoint = imagePointAtViewportPoint(state, context, anchor);
	const width = fit.width * scale;
	const height = fit.height * scale;
	const desiredX = clamp(
		anchor.x - imagePoint.x * width - (context.viewportWidth - width) / 2,
		-(width - context.viewportWidth) / 2,
		(width - context.viewportWidth) / 2,
	);
	const desiredY = clamp(
		anchor.y - imagePoint.y * height - (context.viewportHeight - height) / 2,
		-(height - context.viewportHeight) / 2,
		(height - context.viewportHeight) / 2,
	);
	return stateWithTranslation(
		{ ...state, assetRevision },
		context,
		{ ...geometry, scale },
		desiredX,
		desiredY,
	);
}

export function panViewerBy(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	delta: ViewerPoint,
): ViewerTransformState {
	if (
		!validContext(context) ||
		!Number.isFinite(delta?.x) ||
		!Number.isFinite(delta?.y)
	)
		return resetViewerTransform(safeRevision(state));
	const geometry = deriveViewerTransform(state, context);
	if (geometry.scale <= 1) return safeState(state);
	const width = geometry.fitWidth * geometry.scale;
	const height = geometry.fitHeight * geometry.scale;
	const nextX = clamp(
		geometry.translateX + delta.x,
		-(width - context.viewportWidth) / 2,
		(width - context.viewportWidth) / 2,
	);
	const nextY = clamp(
		geometry.translateY + delta.y,
		-(height - context.viewportHeight) / 2,
		(height - context.viewportHeight) / 2,
	);
	return stateWithTranslation(state, context, geometry, nextX, nextY);
}

export function viewerZoomStep(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	direction: 1 | -1,
): ViewerTransformState {
	if (direction !== 1 && direction !== -1)
		return resetViewerTransform(safeRevision(state));
	const safe = safeState(state);
	return zoomViewerAt(
		safe,
		context,
		safe.scale * (direction === 1 ? 1.25 : 1 / 1.25),
		{
			x: (context?.viewportWidth ?? Number.NaN) / 2,
			y: (context?.viewportHeight ?? Number.NaN) / 2,
		},
	);
}
