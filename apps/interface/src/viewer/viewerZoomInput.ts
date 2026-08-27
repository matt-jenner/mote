import type { ViewerPoint } from "./viewerTransform";

export interface ViewerWheelIntent {
	kind: "zoom" | "pan" | "none";
	zoomFactor?: number;
	delta?: ViewerPoint;
}

export function classifyViewerWheel(input: {
	deltaX: number;
	deltaY: number;
	ctrlKey: boolean;
	metaKey: boolean;
	mode: "fit" | "zoomed";
}): ViewerWheelIntent {
	if (input.ctrlKey || input.metaKey) {
		return {
			kind: "zoom",
			zoomFactor: Math.exp(-input.deltaY * 0.0025),
		};
	}
	if (input.mode === "zoomed" && (input.deltaX !== 0 || input.deltaY !== 0)) {
		return {
			kind: "pan",
			delta: { x: -input.deltaX, y: -input.deltaY },
		};
	}
	return { kind: "none" };
}
