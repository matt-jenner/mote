import {
	createElement,
	type PointerEvent as ReactPointerEvent,
	useState,
} from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import {
	classifyViewerGesture,
	useViewerGestures,
	type ViewerGestures,
} from "./useViewerGestures";

class GestureElement {
	private captures = new Set<number>();

	closest() {
		return null;
	}

	setPointerCapture(pointerId: number) {
		this.captures.add(pointerId);
	}

	hasPointerCapture(pointerId: number) {
		return this.captures.has(pointerId);
	}

	releasePointerCapture(pointerId: number) {
		this.captures.delete(pointerId);
	}
}

const originalElement = globalThis.Element;

if (typeof document === "undefined")
	afterEach(() => {
		Object.defineProperty(globalThis, "Element", {
			configurable: true,
			value: originalElement,
		});
	});

function nodeDescribe(name: string, factory: () => void) {
	if (typeof document === "undefined") describe(name, factory);
}

export function GestureLifecycleHarness({
	stats,
}: {
	stats: { panEnds: number };
}) {
	const [assetRevision, setAssetRevision] = useState(1);
	const [viewportRevision, setViewportRevision] = useState(1);
	const [viewerOpen, setViewerOpen] = useState(true);
	const gestures = useViewerGestures({
		assetRevision,
		onNavigate: () => undefined,
		onPan: () => undefined,
		onPanEnd: () => {
			stats.panEnds += 1;
		},
		onPanStart: () => undefined,
		onTap: () => undefined,
		viewportRevision,
		viewState: "zoomed",
		viewerOpen,
	});
	return createElement(
		"div",
		{ "data-testid": "gesture-lifecycle-harness" },
		createElement("div", {
			"data-testid": "gesture-lifecycle-target",
			onLostPointerCapture: gestures.onLostPointerCapture,
			onPointerCancel: gestures.onPointerCancel,
			onPointerDown: gestures.onPointerDown,
			onPointerMove: gestures.onPointerMove,
			onPointerUp: gestures.onPointerUp,
		}),
		createElement(
			"button",
			{
				"data-testid": "gesture-asset-revision",
				onClick: () => setAssetRevision((value) => value + 1),
				type: "button",
			},
			"Asset revision",
		),
		createElement(
			"button",
			{
				"data-testid": "gesture-viewport-revision",
				onClick: () => setViewportRevision((value) => value + 1),
				type: "button",
			},
			"Viewport revision",
		),
		createElement(
			"button",
			{
				"data-testid": "gesture-close",
				onClick: () => setViewerOpen(false),
				type: "button",
			},
			"Close",
		),
		createElement(
			"output",
			{ "data-testid": "gesture-pan-ends" },
			stats.panEnds,
		),
	);
}

function mountGestures(viewState: "fit" | "zoomed") {
	Object.defineProperty(globalThis, "Element", {
		configurable: true,
		value: GestureElement,
	});
	const pans: Array<{ x: number; y: number }> = [];
	const stats = { navigations: 0, panEnds: 0, panStarts: 0 };
	let gestures: ViewerGestures | undefined;
	function Harness() {
		gestures = useViewerGestures({
			viewerOpen: true,
			viewportRevision: 1,
			viewState,
			onNavigate: () => {
				stats.navigations += 1;
			},
			onTap: () => undefined,
			onPanStart: () => {
				stats.panStarts += 1;
			},
			onPan: (delta) => {
				pans.push(delta);
			},
			onPanEnd: () => {
				stats.panEnds += 1;
			},
		});
		return null;
	}
	renderToStaticMarkup(createElement(Harness));
	if (!gestures) throw new Error("gesture hook did not mount");
	return { gestures, pans, stats };
}

function pointerEvent(
	element: GestureElement,
	type: string,
	clientX: number,
	clientY: number,
): ReactPointerEvent<HTMLElement> {
	return {
		button: 0,
		clientX,
		clientY,
		currentTarget: element,
		isPrimary: true,
		pointerId: 1,
		pointerType: "mouse",
		target: element,
		type,
	} as unknown as ReactPointerEvent<HTMLElement>;
}

nodeDescribe("classifyViewerGesture", () => {
	it("classifies horizontal fit swipes as navigation", () => {
		expect(
			classifyViewerGesture({
				dx: -72,
				dy: 12,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("next");
		expect(
			classifyViewerGesture({
				dx: 72,
				dy: 12,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("previous");
	});

	it("preserves dominant vertical drawer movement for scrolling", () => {
		expect(
			classifyViewerGesture({
				dx: 18,
				dy: 90,
				viewState: "fit",
				inDrawer: true,
			}),
		).toBe("drawerScroll");
	});

	it("reserves horizontal movement in a zoomed state for panning", () => {
		expect(
			classifyViewerGesture({
				dx: -72,
				dy: 12,
				viewState: "zoomed",
				inDrawer: false,
			}),
		).toBe("pan");
	});

	it("classifies short movement as a tap", () => {
		expect(
			classifyViewerGesture({
				dx: 4,
				dy: 3,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("tap");
	});

	it("uses a 48-pixel threshold and strict horizontal dominance", () => {
		expect(
			classifyViewerGesture({
				dx: 47,
				dy: 0,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("tap");
		expect(
			classifyViewerGesture({
				dx: 48,
				dy: 38.4,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("none");
		expect(
			classifyViewerGesture({
				dx: 48.01,
				dy: 38.4,
				viewState: "fit",
				inDrawer: false,
			}),
		).toBe("previous");
	});
});

nodeDescribe("useViewerGestures mouse panning", () => {
	it("emits incremental pan deltas without navigating when zoomed", () => {
		const { gestures, pans, stats } = mountGestures("zoomed");
		const element = new GestureElement();
		gestures.onPointerDown(pointerEvent(element, "pointerdown", 100, 200));
		gestures.onPointerMove(pointerEvent(element, "pointermove", 130, 240));
		gestures.onPointerMove(pointerEvent(element, "pointermove", 120, 250));
		gestures.onPointerUp(pointerEvent(element, "pointerup", 120, 250));

		expect(stats.panStarts).toBe(1);
		expect(pans).toEqual([
			{ x: 30, y: 40 },
			{ x: -10, y: 10 },
		]);
		expect(stats.panEnds).toBe(1);
		expect(stats.navigations).toBe(0);
	});

	it("ends a cancelled drag exactly once", () => {
		const { gestures, stats } = mountGestures("zoomed");
		const element = new GestureElement();
		gestures.onPointerDown(pointerEvent(element, "pointerdown", 100, 200));
		gestures.onPointerCancel(pointerEvent(element, "pointercancel", 130, 240));
		gestures.onLostPointerCapture(
			pointerEvent(element, "lostpointercapture", 130, 240),
		);

		expect(stats.panStarts).toBe(1);
		expect(stats.panEnds).toBe(1);
	});

	it("keeps fit-mode mouse drags inert", () => {
		const { gestures, pans, stats } = mountGestures("fit");
		const element = new GestureElement();
		gestures.onPointerDown(pointerEvent(element, "pointerdown", 100, 200));
		gestures.onPointerMove(pointerEvent(element, "pointermove", 20, 200));
		gestures.onPointerUp(pointerEvent(element, "pointerup", 20, 200));

		expect(stats.panStarts).toBe(0);
		expect(pans).toEqual([]);
		expect(stats.panEnds).toBe(0);
		expect(stats.navigations).toBe(0);
	});
});
