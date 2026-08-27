import type { ViewerPoint } from "./viewerTransform";

export interface TouchPoint {
	pointerId: number;
	x: number;
	y: number;
}

export interface PinchSnapshot {
	midpoint: ViewerPoint;
	distance: number;
}

export const DOUBLE_TAP_WINDOW_MS = 280;
export const DOUBLE_TAP_MAX_DISTANCE = 32;

// Descriptive aliases keep the contract discoverable to callers that prefix
// viewer-specific constants while preserving the short names used in tests.
export const VIEWER_DOUBLE_TAP_WINDOW_MS = DOUBLE_TAP_WINDOW_MS;
export const VIEWER_DOUBLE_TAP_MAX_DISTANCE = DOUBLE_TAP_MAX_DISTANCE;

export function pinchSnapshot(
	points: readonly TouchPoint[],
): PinchSnapshot | null {
	if (points.length < 2) return null;
	const first = points[0];
	const second = points[1];
	if (!first || !second) return null;
	const dx = second.x - first.x;
	const dy = second.y - first.y;
	return {
		midpoint: {
			x: (first.x + second.x) / 2,
			y: (first.y + second.y) / 2,
		},
		distance: Math.hypot(dx, dy),
	};
}

export function pinchScale(
	start: PinchSnapshot,
	current: PinchSnapshot,
): number {
	if (
		!Number.isFinite(start.distance) ||
		!Number.isFinite(current.distance) ||
		start.distance <= 0
	)
		return 1;
	return current.distance / start.distance;
}

export function isViewerDoubleTap(
	previous: { at: number; point: ViewerPoint } | null,
	current: { at: number; point: ViewerPoint },
): boolean {
	if (!previous) return false;
	const elapsed = current.at - previous.at;
	const dx = current.point.x - previous.point.x;
	const dy = current.point.y - previous.point.y;
	return (
		Number.isFinite(elapsed) &&
		elapsed >= 0 &&
		elapsed <= DOUBLE_TAP_WINDOW_MS &&
		Number.isFinite(dx) &&
		Number.isFinite(dy) &&
		Math.hypot(dx, dy) <= DOUBLE_TAP_MAX_DISTANCE
	);
}
