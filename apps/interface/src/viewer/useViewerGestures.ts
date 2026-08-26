import { useCallback, useEffect, useLayoutEffect, useRef } from "react";

export type ViewerGestureViewState = "fit" | "zoomed";
export type ViewerGestureResult =
	| "next"
	| "previous"
	| "drawerScroll"
	| "pan"
	| "tap"
	| "none";

export interface ViewerGestureInput {
	dx: number;
	dy: number;
	viewState: ViewerGestureViewState;
	inDrawer: boolean;
}

const SWIPE_THRESHOLD = 48;
const HORIZONTAL_DOMINANCE = 1.25;

export function classifyViewerGesture({
	dx,
	dy,
	viewState,
	inDrawer,
}: ViewerGestureInput): ViewerGestureResult {
	const horizontal = Math.abs(dx);
	const vertical = Math.abs(dy);
	if (horizontal < SWIPE_THRESHOLD && vertical < SWIPE_THRESHOLD) return "tap";
	if (inDrawer && vertical > horizontal * HORIZONTAL_DOMINANCE) {
		return "drawerScroll";
	}
	if (horizontal <= vertical * HORIZONTAL_DOMINANCE) return "none";
	if (viewState === "zoomed") return "pan";
	return dx < 0 ? "next" : "previous";
}

interface ActiveGesture {
	pointerId: number;
	startX: number;
	startY: number;
	inDrawer: boolean;
	viewState: ViewerGestureViewState;
	startRevision: number;
	target: HTMLElement;
}

interface ViewerGesturesOptions {
	viewerOpen: boolean;
	viewportRevision: number;
	viewState?: ViewerGestureViewState;
	onNavigate: (direction: "next" | "previous") => void;
	onTap: () => void;
}

export interface ViewerGestures {
	onPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
	onLostPointerCapture: (event: React.PointerEvent<HTMLElement>) => void;
}

function isInteractiveTarget(target: EventTarget | null): boolean {
	return (
		target instanceof Element &&
		Boolean(target.closest("button, a, input, select, textarea, fieldset"))
	);
}

export function useViewerGestures({
	viewerOpen,
	viewportRevision,
	viewState = "fit",
	onNavigate,
	onTap,
}: ViewerGesturesOptions): ViewerGestures {
	const activeGesture = useRef<ActiveGesture | null>(null);
	const revisionRef = useRef(viewportRevision);
	const previousRevision = useRef(viewportRevision);
	const navigateRef = useRef(onNavigate);
	const tapRef = useRef(onTap);
	revisionRef.current = viewportRevision;
	navigateRef.current = onNavigate;
	tapRef.current = onTap;

	const cancel = useCallback(() => {
		const gesture = activeGesture.current;
		if (gesture?.target.hasPointerCapture?.(gesture.pointerId)) {
			gesture.target.releasePointerCapture(gesture.pointerId);
		}
		activeGesture.current = null;
	}, []);

	useLayoutEffect(() => {
		if (previousRevision.current !== viewportRevision) cancel();
		previousRevision.current = viewportRevision;
	}, [cancel, viewportRevision]);

	useEffect(() => {
		if (!viewerOpen) cancel();
		return cancel;
	}, [cancel, viewerOpen]);

	const onPointerDown = useCallback(
		(event: React.PointerEvent<HTMLElement>) => {
			if (!viewerOpen || event.isPrimary === false || activeGesture.current)
				return;
			if (event.pointerType === "mouse" && event.button !== 0) return;
			if (isInteractiveTarget(event.target)) return;
			const target = event.currentTarget;
			activeGesture.current = {
				pointerId: event.pointerId,
				startX: event.clientX,
				startY: event.clientY,
				inDrawer:
					event.target instanceof Element
						? Boolean(event.target.closest("aside"))
						: false,
				viewState,
				startRevision: revisionRef.current,
				target,
			};
			try {
				target.setPointerCapture(event.pointerId);
			} catch {
				// Some browser test doubles do not implement pointer capture.
			}
		},
		[viewState, viewerOpen],
	);

	const onPointerMove = useCallback(
		(event: React.PointerEvent<HTMLElement>) => {
			const gesture = activeGesture.current;
			if (!gesture || gesture.pointerId !== event.pointerId) return;
			// Keep the gesture alive while the browser decides whether a drawer scroll wins.
		},
		[],
	);

	const finish = useCallback(
		(event: React.PointerEvent<HTMLElement>, cancelled: boolean) => {
			const gesture = activeGesture.current;
			if (!gesture) {
				// Keep compatibility with synthetic touch pointer-up events used by
				// host integrations that do not emit a pointer-down first.
				if (
					!cancelled &&
					event.pointerType === "touch" &&
					!isInteractiveTarget(event.target) &&
					event.target instanceof Element &&
					!event.target.closest("aside")
				)
					tapRef.current();
				return;
			}
			if (gesture.pointerId !== event.pointerId) return;
			activeGesture.current = null;
			if (gesture.target.hasPointerCapture?.(gesture.pointerId)) {
				gesture.target.releasePointerCapture(gesture.pointerId);
			}
			if (
				cancelled ||
				gesture.startRevision !== revisionRef.current ||
				gesture.startRevision !== viewportRevision
			)
				return;
			const result = classifyViewerGesture({
				dx: event.clientX - gesture.startX,
				dy: event.clientY - gesture.startY,
				inDrawer: gesture.inDrawer,
				viewState: gesture.viewState,
			});
			if (result === "next" || result === "previous")
				navigateRef.current(result);
			else if (result === "tap" && !gesture.inDrawer) tapRef.current();
		},
		[viewportRevision],
	);

	return {
		onPointerDown,
		onPointerMove,
		onPointerUp: (event) => finish(event, false),
		onPointerCancel: (event) => finish(event, true),
		onLostPointerCapture: (event) => finish(event, true),
	};
}
