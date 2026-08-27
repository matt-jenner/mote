import { useCallback, useEffect, useRef, useState } from "react";
import {
	deriveViewerTransform,
	panViewerBy,
	resetViewerTransform,
	type ViewerPoint,
	type ViewerTransformContext,
	type ViewerTransformGeometry,
	type ViewerTransformState,
	viewerZoomStep,
	zoomViewerAt,
} from "./viewerTransform";

export interface ViewerNaturalSize {
	width: number;
	height: number;
}

export interface ViewerDrawableSize {
	width: number;
	height: number;
}

export type ViewerTransformTransition =
	| { type: "assetRevision"; assetRevision: number }
	| {
			type: "zoomAt";
			scale: number;
			anchor: ViewerPoint;
			context: ViewerTransformContext;
	  }
	| {
			type: "step";
			direction: 1 | -1;
			context: ViewerTransformContext;
			anchor?: ViewerPoint;
	  }
	| {
			type: "panBy";
			delta: ViewerPoint;
			context: ViewerTransformContext;
	  }
	| { type: "recenter"; focal: ViewerPoint };

function finitePositive(value: number): boolean {
	return Number.isFinite(value) && value > 0;
}

function safeRevision(value: number): number {
	return Number.isFinite(value) ? value : 0;
}

function safeFocal(point: ViewerPoint): ViewerPoint {
	return {
		x: Number.isFinite(point?.x) ? Math.min(1, Math.max(0, point.x)) : 0.5,
		y: Number.isFinite(point?.y) ? Math.min(1, Math.max(0, point.y)) : 0.5,
	};
}

function contextFor(
	drawable: ViewerDrawableSize,
	natural: ViewerNaturalSize,
	imageWidth: number,
	imageHeight: number,
): ViewerTransformContext {
	return {
		viewportWidth: drawable.width,
		viewportHeight: drawable.height,
		imageWidth,
		imageHeight,
		naturalWidth: natural.width,
		naturalHeight: natural.height,
	};
}

export function transitionViewerTransform(
	state: ViewerTransformState,
	action: ViewerTransformTransition,
): ViewerTransformState {
	switch (action.type) {
		case "assetRevision":
			return resetViewerTransform(action.assetRevision);
		case "zoomAt":
			return zoomViewerAt(state, action.context, action.scale, action.anchor);
		case "step":
			if (action.anchor) {
				const geometry = deriveViewerTransform(state, action.context);
				return zoomViewerAt(
					state,
					action.context,
					geometry.scale * (action.direction === 1 ? 1.25 : 1 / 1.25),
					action.anchor,
				);
			}
			return viewerZoomStep(state, action.context, action.direction);
		case "panBy":
			return panViewerBy(state, action.context, action.delta);
		case "recenter":
			return { ...state, focal: safeFocal(action.focal) };
	}
}

export function reconcileViewerTransform(
	state: ViewerTransformState,
	context: ViewerTransformContext,
): ViewerTransformState {
	const geometry = deriveViewerTransform(state, context);
	if (
		!finitePositive(geometry.fitWidth) ||
		!finitePositive(geometry.fitHeight)
	) {
		return resetViewerTransform(state.assetRevision);
	}
	return {
		assetRevision: safeRevision(state.assetRevision),
		scale: geometry.scale,
		focal: { ...geometry.focal },
	};
}

export function viewerTransformStateForRevision(
	state: ViewerTransformState,
	assetRevision: number,
): ViewerTransformState {
	return state.assetRevision === assetRevision
		? state
		: resetViewerTransform(assetRevision);
}

export interface ViewerTransformController {
	state: ViewerTransformState;
	geometry: ViewerTransformGeometry;
	mode: "fit" | "zoomed";
	zoomLabel: "Fit" | `${number}%`;
	canZoomIn: boolean;
	canZoomOut: boolean;
	reset: () => void;
	zoomAt: (scale: number, anchor: ViewerPoint) => void;
	step: (direction: 1 | -1, anchor?: ViewerPoint) => void;
	panBy: (delta: ViewerPoint) => void;
	recenter: (focal: ViewerPoint) => void;
	setDrawableSize: (size: ViewerDrawableSize) => void;
	setNaturalSize: (size: ViewerNaturalSize) => void;
}

export function useViewerTransform(options: {
	assetId: string;
	assetRevision: number;
	imageWidth: number;
	imageHeight: number;
	onInteraction: () => void;
}): ViewerTransformController {
	const [state, setState] = useState(() =>
		resetViewerTransform(options.assetRevision),
	);
	const dimensionsRef = useRef<{
		drawable: ViewerDrawableSize;
		natural: ViewerNaturalSize;
	}>({
		drawable: { width: 0, height: 0 },
		natural: { width: 0, height: 0 },
	});
	const [dimensions, setDimensions] = useState(dimensionsRef.current);
	const optionsRef = useRef(options);
	optionsRef.current = options;

	const currentContext = useCallback(
		(next = dimensionsRef.current) =>
			contextFor(
				next.drawable,
				next.natural,
				optionsRef.current.imageWidth,
				optionsRef.current.imageHeight,
			),
		[],
	);
	const interact = useCallback(() => {
		optionsRef.current.onInteraction();
	}, []);

	useEffect(() => {
		setState(resetViewerTransform(options.assetRevision));
		dimensionsRef.current = {
			drawable: dimensionsRef.current.drawable,
			natural: { width: 0, height: 0 },
		};
		setDimensions(dimensionsRef.current);
	}, [options.assetRevision]);

	useEffect(() => {
		setState((previous) =>
			reconcileViewerTransform(
				previous,
				contextFor(
					dimensionsRef.current.drawable,
					dimensionsRef.current.natural,
					options.imageWidth,
					options.imageHeight,
				),
			),
		);
	}, [options.imageHeight, options.imageWidth]);

	const mutate = useCallback(
		(action: ViewerTransformTransition) => {
			setState((previous) =>
				transitionViewerTransform(
					viewerTransformStateForRevision(
						previous,
						optionsRef.current.assetRevision,
					),
					action,
				),
			);
			interact();
		},
		[interact],
	);
	const reset = useCallback(() => {
		mutate({
			type: "assetRevision",
			assetRevision: optionsRef.current.assetRevision,
		});
	}, [mutate]);
	const zoomAt = useCallback(
		(scale: number, anchor: ViewerPoint) =>
			mutate({ type: "zoomAt", scale, anchor, context: currentContext() }),
		[currentContext, mutate],
	);
	const step = useCallback(
		(direction: 1 | -1, anchor?: ViewerPoint) =>
			mutate({ type: "step", direction, anchor, context: currentContext() }),
		[currentContext, mutate],
	);
	const panBy = useCallback(
		(delta: ViewerPoint) =>
			mutate({ type: "panBy", delta, context: currentContext() }),
		[currentContext, mutate],
	);
	const recenter = useCallback(
		(focal: ViewerPoint) => mutate({ type: "recenter", focal }),
		[mutate],
	);
	const updateDimensions = useCallback(
		(
			kind: "drawable" | "natural",
			value: ViewerDrawableSize | ViewerNaturalSize,
		) => {
			const nextValue = {
				width: finitePositive(value.width) ? value.width : 0,
				height: finitePositive(value.height) ? value.height : 0,
			};
			const previous = dimensionsRef.current;
			if (
				previous[kind].width === nextValue.width &&
				previous[kind].height === nextValue.height
			)
				return;
			const next = { ...previous, [kind]: nextValue } as typeof previous;
			dimensionsRef.current = next;
			setDimensions(next);
			setState((current) =>
				reconcileViewerTransform(current, currentContext(next)),
			);
		},
		[currentContext],
	);
	const setDrawableSize = useCallback(
		(size: ViewerDrawableSize) => updateDimensions("drawable", size),
		[updateDimensions],
	);
	const setNaturalSize = useCallback(
		(size: ViewerNaturalSize) => updateDimensions("natural", size),
		[updateDimensions],
	);

	const renderState = viewerTransformStateForRevision(
		state,
		options.assetRevision,
	);
	const geometry = deriveViewerTransform(
		renderState,
		contextFor(
			dimensions.drawable,
			dimensions.natural,
			options.imageWidth,
			options.imageHeight,
		),
	);
	const effectiveScale = geometry.scale;
	return {
		state: renderState,
		geometry,
		mode: geometry.mode,
		zoomLabel:
			effectiveScale === 1
				? "Fit"
				: `${Math.round((effectiveScale / geometry.maxScale) * 100)}%`,
		canZoomIn: effectiveScale < geometry.maxScale,
		canZoomOut: effectiveScale > 1,
		reset,
		zoomAt,
		step,
		panBy,
		recenter,
		setDrawableSize,
		setNaturalSize,
	};
}
