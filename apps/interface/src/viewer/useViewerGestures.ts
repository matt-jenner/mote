import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import {
	DOUBLE_TAP_WINDOW_MS,
	isViewerDoubleTap,
	type PinchSnapshot,
	pinchScale,
	pinchSnapshot,
	type TouchPoint,
} from "./viewerTouchGesture";
import type { ViewerPoint } from "./viewerTransform";

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

interface ActiveMouseGesture {
	pointerId: number;
	startX: number;
	startY: number;
	lastX: number;
	lastY: number;
	panStarted: boolean;
	target: HTMLElement;
}

interface TouchGesture {
	startX: number;
	startY: number;
	inDrawer: boolean;
	viewState: ViewerGestureViewState;
	startRevision: number;
	startAssetRevision: number;
	lastX: number;
	lastY: number;
	panStarted: boolean;
	pinching: boolean;
	consumed: boolean;
	targets: Map<number, HTMLElement>;
	pinch: PinchSnapshot | null;
	pinchStart: PinchSnapshot | null;
}

interface PendingTap {
	at: number;
	point: ViewerPoint;
	timer: ReturnType<typeof setTimeout>;
}

interface ViewerGesturesOptions {
	viewerOpen: boolean;
	viewportRevision: number;
	assetRevision?: number;
	viewState?: ViewerGestureViewState;
	onNavigate: (direction: "next" | "previous") => void;
	onTap: () => void;
	onPanStart?: () => void;
	onPan?: (delta: { x: number; y: number }) => void;
	onPanEnd?: () => void;
	onPinch?: (scale: number, midpoint: ViewerPoint) => void;
	onPinchStart?: () => void;
	onPinchEnd?: () => void;
	onDoubleTap?: (point: ViewerPoint) => void;
}

export interface ViewerGestures {
	onPointerDown: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
	onPointerCancel: (event: React.PointerEvent<HTMLElement>) => void;
	onLostPointerCapture: (event: React.PointerEvent<HTMLElement>) => void;
}

function isElement(target: EventTarget | null): target is Element {
	return typeof Element !== "undefined" && target instanceof Element;
}

function isInteractiveTarget(target: EventTarget | null): boolean {
	return (
		isElement(target) &&
		Boolean(target.closest("button, a, input, select, textarea, fieldset"))
	);
}

function isViewerControlTarget(target: EventTarget | null): boolean {
	return (
		isInteractiveTarget(target) ||
		(isElement(target) &&
			Boolean(
				target.closest(
					"[data-viewer-controls], [data-viewer-chrome], [data-viewer-info], [data-viewer-zoom-controls], [data-viewer-navigator], aside",
				),
			))
	);
}

function touchPointMap(points: Map<number, TouchPoint>): TouchPoint[] {
	return [...points.values()];
}

function capture(target: HTMLElement, pointerId: number) {
	try {
		target.setPointerCapture(pointerId);
	} catch {
		// Some browser test doubles do not implement pointer capture.
	}
}

export function useViewerGestures({
	viewerOpen,
	viewportRevision,
	assetRevision = 0,
	viewState = "fit",
	onNavigate,
	onTap,
	onPanStart,
	onPan,
	onPanEnd,
	onPinch,
	onPinchStart,
	onPinchEnd,
	onDoubleTap,
}: ViewerGesturesOptions): ViewerGestures {
	const activeMouse = useRef<ActiveMouseGesture | null>(null);
	const touchPoints = useRef<Map<number, TouchPoint>>(new Map());
	const ignoredTouchPointers = useRef<Set<number>>(new Set());
	const touchGesture = useRef<TouchGesture | null>(null);
	const pendingTaps = useRef<PendingTap[]>([]);
	const lastTap = useRef<{ at: number; point: ViewerPoint } | null>(null);
	const revisionRef = useRef(viewportRevision);
	const assetRevisionRef = useRef(assetRevision);
	const previousRevision = useRef(viewportRevision);
	const previousAssetRevision = useRef(assetRevision);
	const navigateRef = useRef(onNavigate);
	const tapRef = useRef(onTap);
	const panStartRef = useRef(onPanStart);
	const panRef = useRef(onPan);
	const panEndRef = useRef(onPanEnd);
	const pinchRef = useRef(onPinch);
	const pinchStartRef = useRef(onPinchStart);
	const pinchEndRef = useRef(onPinchEnd);
	const doubleTapRef = useRef(onDoubleTap);
	revisionRef.current = viewportRevision;
	assetRevisionRef.current = assetRevision;
	navigateRef.current = onNavigate;
	tapRef.current = onTap;
	panStartRef.current = onPanStart;
	panRef.current = onPan;
	panEndRef.current = onPanEnd;
	pinchRef.current = onPinch;
	pinchStartRef.current = onPinchStart;
	pinchEndRef.current = onPinchEnd;
	doubleTapRef.current = onDoubleTap;

	const clearPendingTap = useCallback(() => {
		for (const pending of pendingTaps.current) clearTimeout(pending.timer);
		pendingTaps.current = [];
	}, []);

	const releaseMouseCapture = useCallback((gesture: ActiveMouseGesture) => {
		if (gesture.target.hasPointerCapture?.(gesture.pointerId)) {
			gesture.target.releasePointerCapture(gesture.pointerId);
		}
	}, []);

	const releaseTouchCapture = useCallback((gesture: TouchGesture) => {
		for (const [pointerId, target] of gesture.targets) {
			if (target.hasPointerCapture?.(pointerId))
				target.releasePointerCapture(pointerId);
		}
	}, []);

	const cancel = useCallback(() => {
		const mouse = activeMouse.current;
		activeMouse.current = null;
		if (mouse?.panStarted) panEndRef.current?.();
		if (mouse) releaseMouseCapture(mouse);

		const touch = touchGesture.current;
		touchGesture.current = null;
		touchPoints.current.clear();
		if (touch?.panStarted) panEndRef.current?.();
		if (touch?.pinching) pinchEndRef.current?.();
		if (touch) releaseTouchCapture(touch);
		clearPendingTap();
		lastTap.current = null;
	}, [clearPendingTap, releaseMouseCapture, releaseTouchCapture]);

	useLayoutEffect(() => {
		if (previousRevision.current !== viewportRevision) cancel();
		previousRevision.current = viewportRevision;
		if (previousAssetRevision.current !== assetRevision) cancel();
		previousAssetRevision.current = assetRevision;
	}, [assetRevision, cancel, viewportRevision]);

	useEffect(() => {
		if (!viewerOpen) cancel();
		return cancel;
	}, [cancel, viewerOpen]);

	const onPointerDown = useCallback(
		(event: React.PointerEvent<HTMLElement>) => {
			if (!viewerOpen) return;
			if (event.pointerType === "touch") {
				if (ignoredTouchPointers.current.has(event.pointerId)) return;
				if (isViewerControlTarget(event.target)) {
					ignoredTouchPointers.current.add(event.pointerId);
					return;
				}
			} else if (isViewerControlTarget(event.target)) return;
			if (event.pointerType === "mouse") {
				if (event.isPrimary === false || event.button !== 0) return;
				if (activeMouse.current || touchPoints.current.size > 0) return;
				const target = event.currentTarget;
				const mouse: ActiveMouseGesture = {
					pointerId: event.pointerId,
					startX: event.clientX,
					startY: event.clientY,
					lastX: event.clientX,
					lastY: event.clientY,
					panStarted: viewState === "zoomed",
					target,
				};
				activeMouse.current = mouse;
				if (mouse.panStarted) panStartRef.current?.();
				capture(target, event.pointerId);
				return;
			}
			if (event.pointerType !== "touch") return;
			if (touchPoints.current.has(event.pointerId)) return;
			if (touchPoints.current.size >= 2) {
				ignoredTouchPointers.current.add(event.pointerId);
				return;
			}
			touchPoints.current.set(event.pointerId, {
				pointerId: event.pointerId,
				x: event.clientX,
				y: event.clientY,
			});
			const target = event.currentTarget;
			const gesture = touchGesture.current;
			if (!gesture) {
				const next: TouchGesture = {
					startX: event.clientX,
					startY: event.clientY,
					inDrawer: false,
					viewState,
					startRevision: revisionRef.current,
					startAssetRevision: assetRevisionRef.current,
					lastX: event.clientX,
					lastY: event.clientY,
					panStarted: viewState === "zoomed",
					pinching: false,
					consumed: false,
					targets: new Map([[event.pointerId, target]]),
					pinch: null,
					pinchStart: null,
				};
				touchGesture.current = next;
				if (next.panStarted) panStartRef.current?.();
				capture(target, event.pointerId);
				return;
			}
			clearPendingTap();
			lastTap.current = null;
			gesture.targets.set(event.pointerId, target);
			gesture.pinching = true;
			gesture.consumed = true;
			if (gesture.panStarted) {
				panEndRef.current?.();
				gesture.panStarted = false;
			}
			gesture.pinch = pinchSnapshot(touchPointMap(touchPoints.current));
			gesture.pinchStart = gesture.pinch;
			pinchStartRef.current?.();
			capture(target, event.pointerId);
		},
		[clearPendingTap, viewerOpen, viewState],
	);

	const onPointerMove = useCallback(
		(event: React.PointerEvent<HTMLElement>) => {
			const mouse = activeMouse.current;
			if (
				mouse?.pointerId === event.pointerId &&
				event.pointerType === "mouse"
			) {
				if (mouse.panStarted) {
					panRef.current?.({
						x: event.clientX - mouse.lastX,
						y: event.clientY - mouse.lastY,
					});
					mouse.lastX = event.clientX;
					mouse.lastY = event.clientY;
				}
				return;
			}
			if (event.pointerType !== "touch") return;
			const point = touchPoints.current.get(event.pointerId);
			if (!point) return;
			point.x = event.clientX;
			point.y = event.clientY;
			const gesture = touchGesture.current;
			if (!gesture) return;
			if (gesture.pinching) {
				const current = pinchSnapshot(touchPointMap(touchPoints.current));
				if (!current || !gesture.pinchStart) return;
				const scale = pinchScale(gesture.pinchStart, current);
				event.preventDefault();
				if (scale !== 1) {
					pinchRef.current?.(scale, current.midpoint);
					gesture.consumed = true;
				}
				gesture.pinch = current;
				return;
			}
			if (gesture.panStarted && touchPoints.current.size === 1) {
				event.preventDefault();
				panRef.current?.({
					x: event.clientX - gesture.lastX,
					y: event.clientY - gesture.lastY,
				});
				gesture.lastX = event.clientX;
				gesture.lastY = event.clientY;
				gesture.consumed = true;
			}
		},
		[],
	);

	const validLifecycle = useCallback(
		(gesture: { startRevision: number; startAssetRevision: number }) =>
			gesture.startRevision === revisionRef.current &&
			gesture.startRevision === viewportRevision &&
			gesture.startAssetRevision === assetRevisionRef.current &&
			gesture.startAssetRevision === assetRevision,
		[assetRevision, viewportRevision],
	);

	const finishMouse = useCallback(
		(event: React.PointerEvent<HTMLElement>) => {
			const gesture = activeMouse.current;
			if (!gesture || gesture.pointerId !== event.pointerId) return;
			activeMouse.current = null;
			if (gesture.panStarted) panEndRef.current?.();
			releaseMouseCapture(gesture);
		},
		[releaseMouseCapture],
	);

	const finishTouch = useCallback(
		(event: React.PointerEvent<HTMLElement>, cancelled: boolean) => {
			if (ignoredTouchPointers.current.delete(event.pointerId)) return;
			const gesture = touchGesture.current;
			if (!gesture || !touchPoints.current.has(event.pointerId)) {
				if (gesture) return;
				if (cancelled) {
					clearPendingTap();
					lastTap.current = null;
					return;
				}
				// Preserve compatibility with host integrations that emit a synthetic
				// touch-up without a preceding down on the stage itself.
				if (
					!cancelled &&
					event.pointerType === "touch" &&
					!isViewerControlTarget(event.target)
				)
					tapRef.current();
				return;
			}
			const point = touchPoints.current.get(event.pointerId);
			if (point) {
				point.x = event.clientX;
				point.y = event.clientY;
			}
			if (cancelled) {
				cancel();
				return;
			}
			const hadMultiplePointers =
				gesture.pinching || touchPoints.current.size > 1;
			touchPoints.current.delete(event.pointerId);
			const target = gesture.targets.get(event.pointerId);
			if (target?.hasPointerCapture?.(event.pointerId))
				target.releasePointerCapture(event.pointerId);
			gesture.targets.delete(event.pointerId);
			if (touchPoints.current.size > 0) {
				// A pinch ending with one finger still down cannot become a tap or
				// navigation gesture.
				gesture.consumed = true;
				return;
			}
			touchGesture.current = null;
			if (gesture.panStarted) panEndRef.current?.();
			if (gesture.pinching) pinchEndRef.current?.();
			if (!validLifecycle(gesture) || gesture.consumed || hadMultiplePointers)
				return;
			const result = classifyViewerGesture({
				dx: event.clientX - gesture.startX,
				dy: event.clientY - gesture.startY,
				inDrawer: gesture.inDrawer,
				viewState: gesture.viewState,
			});
			if (result === "next" || result === "previous") {
				lastTap.current = null;
				navigateRef.current(result);
				return;
			}
			if (result !== "tap" || gesture.inDrawer) {
				lastTap.current = null;
				return;
			}
			const now = Date.now();
			const current = {
				at: now,
				point: { x: event.clientX, y: event.clientY },
			};
			const previous = lastTap.current;
			if (previous && isViewerDoubleTap(previous, current)) {
				if (event.cancelable) event.preventDefault();
				clearPendingTap();
				lastTap.current = null;
				doubleTapRef.current?.(current.point);
				return;
			}
			lastTap.current = current;
			const pending: PendingTap = {
				...current,
				timer: setTimeout(() => {
					const index = pendingTaps.current.indexOf(pending);
					if (index < 0) return;
					pendingTaps.current.splice(index, 1);
					tapRef.current();
				}, DOUBLE_TAP_WINDOW_MS),
			};
			pendingTaps.current.push(pending);
		},
		[cancel, clearPendingTap, validLifecycle],
	);

	const finish = useCallback(
		(event: React.PointerEvent<HTMLElement>, cancelled: boolean) => {
			if (event.pointerType === "touch") {
				finishTouch(event, cancelled);
				return;
			}
			finishMouse(event);
		},
		[finishMouse, finishTouch],
	);

	return {
		onPointerDown,
		onPointerMove,
		onPointerUp: (event) => finish(event, false),
		onPointerCancel: (event) => finish(event, true),
		onLostPointerCapture: (event) => finish(event, true),
	};
}
