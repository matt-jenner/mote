# Viewer Zoom and Pan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add cache-only moderate zoom, pan, visible controls, and a contextual navigator to the immersive viewer without changing its backend contract or progressive-preview behaviour.

**Architecture:** Keep the existing decoded image layers and apply one CSS transform derived from a browser-independent geometry module. A React controller owns scale and normalized focal point, while the existing stage, input adapter, controls, and navigator consume the same derived geometry. Asset navigation resets the controller; resize, rotation, and preview refinement preserve it.

**Tech Stack:** React 19, TypeScript 7, CSS modules, Vitest unit tests, Vitest Browser with WebKit and Playwright, Tauri 2 for the macOS demonstration, existing Rust catalog/cache/application service.

**Spec:** `docs/superpowers/specs/2026-08-27-viewer-zoom-pan-design.md`

## Global Constraints

- Source media remains read-only; no zoom path may write, rename, rate, tag, copy, or delete a source file.
- The slice uses only ready wall thumbnails and 4096-pixel screen previews. It must not read originals on demand or request deep-zoom tiles.
- The maximum scale is one decoded derivative pixel per CSS pixel and never less than fit.
- Asset navigation always resets zoom to fit.
- Resize and rotation preserve the normalized image point at the centre, subject only to valid clamping.
- Thumbnail-to-preview refinement preserves scale, focal point, pan position, and fitted geometry.
- At fit, horizontal touch swipe navigates. While zoomed, every image drag pans and never navigates.
- The navigator appears only while zoomed. It is draggable on desktop and display-only on touch devices.
- All visible controls are at least 44 by 44 CSS pixels and remain safe-area aware.
- The React implementation imports no Tauri API and adds no `PhotoService` or Rust contract.
- Consumed viewer gestures must not leak into browser page zoom or document scrolling; behaviour outside the open viewer remains unchanged.
- Keep the existing `wall-demo` app profile and leave the app running for every user-facing checkpoint.
- Do not add video zoom, original-resolution delivery, locked comparison zoom, momentum, or elastic overscroll.

---

### Task 1: Build the pure transform geometry engine

**Files:**
- Create: `apps/interface/src/viewer/viewerTransform.ts`
- Create: `apps/interface/src/viewer/viewerTransform.test.ts`
- Modify: `apps/interface/src/components/ViewerStage.tsx`
- Modify: `apps/interface/src/components/ViewerStage.test.ts`

**Interfaces:**
- Consumes: existing `fitViewerFrame(containerWidth, containerHeight, assetWidth, assetHeight)`.
- Produces:

```ts
export interface ViewerPoint { x: number; y: number }

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

export const FIT_VIEWER_TRANSFORM: Omit<ViewerTransformState, "assetRevision">;
export function resetViewerTransform(assetRevision: number): ViewerTransformState;
export function deriveViewerTransform(
	state: ViewerTransformState,
	context: ViewerTransformContext,
): ViewerTransformGeometry;
export function zoomViewerAt(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	nextScale: number,
	anchor: ViewerPoint,
): ViewerTransformState;
export function panViewerBy(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	delta: ViewerPoint,
): ViewerTransformState;
export function viewerZoomStep(
	state: ViewerTransformState,
	context: ViewerTransformContext,
	direction: 1 | -1,
): ViewerTransformState;
```

- All exported functions reject non-finite input by returning a clamped fit-safe result.
- `visibleImageRect` uses normalized image coordinates and is the later navigator contract.

- [ ] **Step 1: Write failing fit and native-limit tests**

```ts
import { describe, expect, it } from "vitest";
import {
	deriveViewerTransform,
	resetViewerTransform,
} from "./viewerTransform";

const landscape = {
	viewportWidth: 1200,
	viewportHeight: 800,
	imageWidth: 6000,
	imageHeight: 4000,
	naturalWidth: 4096,
	naturalHeight: 2731,
};

describe("viewer transform geometry", () => {
	it("fits the image and caps zoom at one derivative pixel per CSS pixel", () => {
		const geometry = deriveViewerTransform(resetViewerTransform(1), landscape);
		expect(geometry.fitWidth).toBeCloseTo(1200);
		expect(geometry.fitHeight).toBeCloseTo(800);
		expect(geometry.maxScale).toBeCloseTo(4096 / 1200);
		expect(geometry.mode).toBe("fit");
	});

	it("keeps a small derivative at fit instead of enlarging cached pixels", () => {
		const geometry = deriveViewerTransform(resetViewerTransform(1), {
			...landscape,
			naturalWidth: 600,
			naturalHeight: 400,
		});
		expect(geometry.maxScale).toBe(1);
	});
});
```

- [ ] **Step 2: Run the focused unit test and verify RED**

Run:

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTransform.test.ts
```

Expected: FAIL because `viewerTransform.ts` does not exist.

- [ ] **Step 3: Implement fit, maximum scale, and safe normalization**

Implement the exact public interfaces above. Reuse `fitViewerFrame`. Calculate:

```ts
const maxScale = Math.max(
	1,
	Math.min(naturalWidth / fit.width, naturalHeight / fit.height),
);
```

Clamp focal coordinates into `[0, 1]`. Centre dimensions that remain smaller than the viewport. For enlarged dimensions, clamp translation so the transformed image covers the viewport.

- [ ] **Step 4: Add failing pointer-anchor, pan, resize, and navigator tests**

```ts
it("keeps the image point under the pointer fixed while zooming", () => {
	const initial = resetViewerTransform(4);
	const anchor = { x: 900, y: 250 };
	const before = imagePointAtViewportPoint(initial, landscape, anchor);
	const zoomed = zoomViewerAt(initial, landscape, 2, anchor);
	const after = imagePointAtViewportPoint(zoomed, landscape, anchor);
	expect(after.x).toBeCloseTo(before.x, 6);
	expect(after.y).toBeCloseTo(before.y, 6);
});

it("clamps panning at every edge without exposing empty canvas", () => {
	const zoomed = zoomViewerAt(
		resetViewerTransform(4),
		landscape,
		2,
		{ x: 600, y: 400 },
	);
	const moved = panViewerBy(zoomed, landscape, { x: 10000, y: -10000 });
	const geometry = deriveViewerTransform(moved, landscape);
	expect(geometry.translateX).toBe((geometry.fitWidth * geometry.scale - 1200) / 2);
	expect(geometry.translateY).toBe(-(geometry.fitHeight * geometry.scale - 800) / 2);
});

it("preserves the normalized focal point after rotation", () => {
	const zoomed = zoomViewerAt(
		resetViewerTransform(4),
		landscape,
		2,
		{ x: 850, y: 300 },
	);
	const rotated = deriveViewerTransform(zoomed, {
		...landscape,
		viewportWidth: 800,
		viewportHeight: 1200,
	});
	expect(rotated.focal).toEqual(deriveViewerTransform(zoomed, landscape).focal);
});

it("derives the visible normalized rectangle for the navigator", () => {
	const zoomed = zoomViewerAt(
		resetViewerTransform(4),
		landscape,
		2,
		{ x: 600, y: 400 },
	);
	const { visibleImageRect } = deriveViewerTransform(zoomed, landscape);
	expect(visibleImageRect).toMatchObject({ x: 0.25, y: 0.25, width: 0.5, height: 0.5 });
});
```

Add `imagePointAtViewportPoint` to the public interface because desktop wheel, touch pinch, and tests all need the same anchor conversion.

- [ ] **Step 5: Implement focal-point zoom, pan, stepping, and visible rectangle**

Use the fitted image centre as the transform origin. Convert screen anchors into normalized image coordinates before changing scale. Derive the new focal point so the same image coordinate remains beneath the anchor after zoom. Use 1.25 as the step multiplier.

Do not round transform state. Round only display labels in later tasks.

- [ ] **Step 6: Run Task 1 verification**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTransform.test.ts src/components/ViewerStage.test.ts
npm run typecheck
npm run check
git diff --check
```

Expected: all pass with no warnings.

- [ ] **Step 7: Commit Task 1**

```bash
git add apps/interface/src/viewer/viewerTransform.ts apps/interface/src/viewer/viewerTransform.test.ts apps/interface/src/components/ViewerStage.tsx apps/interface/src/components/ViewerStage.test.ts
git commit -m "feat: define viewer transform geometry"
```

---

### Task 2: Connect transform state to the progressive image stage

**Files:**
- Create: `apps/interface/src/viewer/useViewerTransform.ts`
- Create: `apps/interface/src/viewer/useViewerTransform.test.ts`
- Modify: `apps/interface/src/components/ViewerStage.tsx`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: Task 1 transform state and geometry, viewer `previewGeneration`, stage drawable dimensions, and decoded derivative natural dimensions.
- Produces:

```ts
export interface ViewerNaturalSize { width: number; height: number }
export interface ViewerDrawableSize { width: number; height: number }

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
}): ViewerTransformController;
```

- `ViewerStage` gains:

```ts
transform: ViewerTransformGeometry;
onDrawableSizeChange: (size: ViewerDrawableSize) => void;
onNaturalSizeChange: (size: ViewerNaturalSize) => void;
```

`zoomLabel` is `Fit` at scale 1. Otherwise it is `Math.round((scale / maxScale) * 100)%`, so 100 percent consistently means the decoded derivative's native display resolution rather than 100 percent of fit.

- [ ] **Step 1: Write failing controller lifecycle tests**

Test the pure state transition helpers exported alongside the hook:

```ts
it("resets to fit when the asset revision changes", () => {
	const zoomed = transitionViewerTransform(
		resetViewerTransform(8),
		{ type: "zoomAt", scale: 2, anchor: { x: 600, y: 400 }, context },
	);
	const reset = transitionViewerTransform(zoomed, {
		type: "assetRevision",
		assetRevision: 9,
	});
	expect(reset).toEqual(resetViewerTransform(9));
});

it("keeps scale and focal point when natural preview dimensions improve", () => {
	const zoomed = transitionViewerTransform(
		resetViewerTransform(8),
		{ type: "zoomAt", scale: 1.5, anchor: { x: 700, y: 350 }, context },
	);
	const refined = reconcileViewerTransform(zoomed, {
		...context,
		naturalWidth: 4096,
		naturalHeight: 2731,
	});
	expect(refined.scale).toBe(zoomed.scale);
	expect(refined.focal).toEqual(zoomed.focal);
});
```

- [ ] **Step 2: Run the focused unit test and verify RED**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/useViewerTransform.test.ts
```

Expected: FAIL because the controller does not exist.

- [ ] **Step 3: Implement the controller and stage measurement callbacks**

Keep the reducer and reconciliation logic browser-independent. The hook stores current drawable and natural dimensions, calls `onInteraction` for every mutation, and resets only when `assetRevision` changes.

In `ViewerStage`, report drawable dimensions only when width or height changes. On successful image decode or load, report `naturalWidth` and `naturalHeight` from the best ready layer. Prefer the screen preview once ready; otherwise use the wall thumbnail.

Apply the transform to one wrapper around all image layers:

```tsx
<div
	className={styles.viewerTransformLayer}
	data-viewer-mode={transform.mode}
	style={{
		transform: `translate3d(${transform.translateX}px, ${transform.translateY}px, 0) scale(${transform.scale})`,
	}}
>
	{/* existing base and preview layers */}
</div>
```

Keep the wrapper's untransformed width and height equal to the fitted frame. Set `transform-origin: center` and `will-change: transform` only while zoomed or directly manipulating.

- [ ] **Step 4: Add failing browser tests for refinement, navigation reset, and rotation**

```ts
it("keeps the same transform when the screen preview replaces the thumbnail", async () => {
	const { view, releasePreviewDecode } = await openZoomHarness();
	await view.getByTestId("zoom-programmatically").click();
	const layer = view.getByTestId("viewer-transform-layer").element();
	const before = layer.style.transform;
	releasePreviewDecode();
	await expect.element(view.getByTestId("screen-preview-ready")).toBeVisible();
	expect(layer.style.transform).toBe(before);
});

it("resets to fit on navigation but preserves zoom through rotation", async () => {
	const { view, rotate } = await openZoomHarness();
	await view.getByTestId("zoom-programmatically").click();
	const beforeRotation = transformSnapshot(view);
	rotate(844, 390);
	await expect.poll(() => transformSnapshot(view).focal).toEqual(beforeRotation.focal);
	await userEvent.keyboard("{ArrowRight}");
	await expect.element(view.getByTestId("viewer-stage")).toHaveAttribute("data-viewer-mode", "fit");
});
```

The test-only harness may expose controller actions through buttons inside the test component. Do not add debug controls to production components.

- [ ] **Step 5: Run Task 2 verification**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTransform.test.ts src/viewer/useViewerTransform.test.ts src/components/ViewerStage.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t "transform|rotation|refinement"
npm run typecheck
npm run check
git diff --check
```

- [ ] **Step 6: Commit Task 2**

```bash
git add apps/interface/src/viewer/useViewerTransform.ts apps/interface/src/viewer/useViewerTransform.test.ts apps/interface/src/components/ViewerStage.tsx apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/styles/photoViewer.module.css
git commit -m "feat: preserve viewer transform through refinement"
```

---

### Task 3: Add desktop zoom controls and pointer input

**Files:**
- Create: `apps/interface/src/components/ViewerZoomControls.tsx`
- Create: `apps/interface/src/viewer/viewerZoomInput.ts`
- Create: `apps/interface/src/viewer/viewerZoomInput.test.ts`
- Modify: `apps/interface/src/viewer/useViewerGestures.ts`
- Modify: `apps/interface/src/viewer/useViewerGestures.test.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/ViewerStage.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: Task 2 `ViewerTransformController`, current control visibility, drawer state, safe areas, and `reportInteraction`.
- Produces:

```ts
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
}): ViewerWheelIntent;
```

- `ViewerZoomControls` props are `label`, `canZoomIn`, `canZoomOut`, `visible`, `onZoomIn`, `onZoomOut`, and `onReset`.
- `useViewerGestures` gains `onPanStart`, `onPan`, and `onPanEnd`. In this task those callbacks are used only for primary-button mouse input while `viewState === "zoomed"`; Task 4 extends the same interface to touch.

- [ ] **Step 1: Write failing wheel-intent tests**

```ts
it("zooms only for modified wheel input over the viewer", () => {
	expect(classifyViewerWheel({
		deltaX: 0,
		deltaY: -80,
		ctrlKey: true,
		metaKey: false,
		mode: "fit",
	})).toMatchObject({ kind: "zoom" });
});

it("pans ordinary wheel input only while zoomed", () => {
	expect(classifyViewerWheel({
		deltaX: 20,
		deltaY: 40,
		ctrlKey: false,
		metaKey: false,
		mode: "zoomed",
	})).toEqual({ kind: "pan", delta: { x: -20, y: -40 } });
	expect(classifyViewerWheel({
		deltaX: 0,
		deltaY: 40,
		ctrlKey: false,
		metaKey: false,
		mode: "fit",
	})).toEqual({ kind: "none" });
});
```

- [ ] **Step 2: Run the focused unit test and verify RED**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerZoomInput.test.ts
```

Expected: FAIL because `viewerZoomInput.ts` does not exist.

- [ ] **Step 3: Implement desktop controls, wheel, shortcuts, and double-click**

Use exponential wheel zoom:

```ts
const factor = Math.exp(-deltaY * 0.0025);
```

Anchor wheel zoom at pointer coordinates relative to the drawable stage. Call `preventDefault()` only for consumed zoom or pan. Attach a non-passive native wheel listener to the stage because React's wheel listener may be passive in a hosted browser.

Handle shortcuts in the existing viewer key path:

- `+` or `=`: zoom in around the drawable centre
- `-`: zoom out around the drawable centre
- `0`: reset to fit
- Arrow keys: navigate and let the asset revision reset the transform

Double-click toggles fit and maximum native scale around the clicked point.

For primary-button mouse input while zoomed, capture the pointer on down, emit incremental `onPan({ x: currentX - previousX, y: currentY - previousY })` deltas on move, and release it on up, cancellation, lost capture, asset revision, or viewport revision. Call `onPanStart` and `onPanEnd` exactly once per completed or cancelled drag. Mouse drag at fit remains inert and never navigates.

Render the zoom cluster within the existing chrome visibility and focus-pause policy. The middle button label is `Fit` at scale 1 and a rounded percentage otherwise. It always resets to fit.

- [ ] **Step 4: Add failing browser flows for every desktop route**

Test visible controls, keyboard, modified wheel, ordinary wheel, double-click, pan cursor, bounds, and navigation reset:

```ts
await view.getByRole("button", { name: "Zoom in" }).click();
await expect.element(view.getByRole("button", { name: /Reset zoom/ })).toHaveTextContent(/%/);
await userEvent.keyboard("0");
await expect.element(view.getByRole("button", { name: /Reset zoom/ })).toHaveTextContent("Fit");

stage.dispatchEvent(new WheelEvent("wheel", {
	bubbles: true,
	cancelable: true,
	ctrlKey: true,
	deltaY: -120,
	clientX: 720,
	clientY: 320,
}));
await expect.element(stage).toHaveAttribute("data-viewer-mode", "zoomed");
```

Also assert:

- plus and minus disable at limits
- every zoom control is at least 44 by 44 CSS pixels
- ordinary wheel at fit is not prevented
- consumed wheel while zoomed does not scroll the document
- controls remain visible while focused
- zoom shortcuts do not fire from editable drawer content
- the zoom cluster shifts clear of an open Info drawer

- [ ] **Step 5: Run Task 3 verification and demonstrate desktop zoom**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTransform.test.ts src/viewer/viewerZoomInput.test.ts src/viewer/useViewerGestures.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t "zoom|wheel|double-click"
npm test
npm run typecheck
npm run check
git diff --check
```

Leave the macOS app running. Demonstrate double-click, modified wheel, ordinary wheel pan, drag pan, `+`, `-`, `0`, buttons, and next-photo reset.

- [ ] **Step 6: Commit Task 3**

```bash
git add apps/interface/src/components/ViewerZoomControls.tsx apps/interface/src/viewer/viewerZoomInput.ts apps/interface/src/viewer/viewerZoomInput.test.ts apps/interface/src/viewer/useViewerGestures.ts apps/interface/src/viewer/useViewerGestures.test.ts apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/components/ViewerStage.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/styles/photoViewer.module.css
git commit -m "feat: add desktop viewer zoom controls"
```

---

### Task 4: Add touch pinch, double-tap, and zoomed panning

**Files:**
- Modify: `apps/interface/src/viewer/useViewerGestures.ts`
- Modify: `apps/interface/src/viewer/useViewerGestures.test.ts`
- Create: `apps/interface/src/viewer/viewerTouchGesture.ts`
- Create: `apps/interface/src/viewer/viewerTouchGesture.test.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: Task 2 controller actions and existing fit-state navigation threshold.
- Produces:

```ts
export interface TouchPoint { pointerId: number; x: number; y: number }
export interface PinchSnapshot {
	midpoint: ViewerPoint;
	distance: number;
}

export function pinchSnapshot(points: readonly TouchPoint[]): PinchSnapshot | null;
export function pinchScale(start: PinchSnapshot, current: PinchSnapshot): number;
export function isViewerDoubleTap(
	previous: { at: number; point: ViewerPoint } | null,
	current: { at: number; point: ViewerPoint },
): boolean;
```

- `useViewerGestures` retains Task 3's `onPanStart`, `onPan`, and `onPanEnd`, then adds `onPinch` and `onDoubleTap` while retaining `onNavigate` and delayed `onTap`.

- [ ] **Step 1: Write failing pinch and double-tap tests**

```ts
it("derives pinch midpoint and relative scale", () => {
	const start = pinchSnapshot([
		{ pointerId: 1, x: 100, y: 100 },
		{ pointerId: 2, x: 200, y: 100 },
	]);
	const current = pinchSnapshot([
		{ pointerId: 1, x: 50, y: 100 },
		{ pointerId: 2, x: 250, y: 100 },
	]);
	expect(start?.midpoint).toEqual({ x: 150, y: 100 });
	expect(pinchScale(start!, current!)).toBe(2);
});

it("accepts a nearby second tap within 280 milliseconds", () => {
	expect(isViewerDoubleTap(
		{ at: 1000, point: { x: 120, y: 160 } },
		{ at: 1220, point: { x: 132, y: 168 } },
	)).toBe(true);
});
```

Use 280 milliseconds and a maximum 32-pixel tap separation. Export both constants for exact tests.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTouchGesture.test.ts src/viewer/useViewerGestures.test.ts
```

Expected: FAIL because multi-pointer zoom and delayed tap arbitration do not exist.

- [ ] **Step 3: Implement the touch state machine**

Track touch pointers by ID. One pointer at fit retains existing swipe classification. One pointer while zoomed emits incremental pan deltas and never navigation. Two pointers cancel any pending single tap and emit pinch scale around the current midpoint.

Delay single-tap `onTap` for 280 milliseconds. A valid second tap cancels the pending single tap and calls `onDoubleTap`. Pointer cancellation, lost capture, asset revision, viewport revision, and viewer close clear every pointer and pending timer.

Gestures that start inside buttons, filmstrip controls, Info drawer, or zoom controls do not enter the stage gesture machine. Drawer vertical scrolling and horizontal fit-state navigation retain existing behaviour.

- [ ] **Step 4: Add browser regressions for touch arbitration**

Cover:

- pinch out and pinch in around a stable midpoint
- one-finger pan while zoomed
- fit-state horizontal swipe still navigates one asset
- the same drag while zoomed pans and does not navigate at an edge
- single tap toggles chrome after the arbitration delay
- double-tap zooms without a transient chrome toggle
- navigation resets to fit
- pointer cancellation and rotation cancel a pending gesture
- touch on drawer, filmstrip, and zoom controls is not treated as stage input
- `touch-action` prevents document scrolling only on the consumed zoomed stage

Use real `PointerEvent` sequences with distinct IDs in WebKit. Assert the visible transform or current asset, not callback spies alone.

- [ ] **Step 5: Run Task 4 verification and demonstrate mobile behaviour**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerTouchGesture.test.ts src/viewer/useViewerGestures.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t "pinch|double-tap|zoomed pan|fit swipe"
npm test
npm run typecheck
npm run check
git diff --check
```

Use phone-sized WebKit to demonstrate fit swipe navigation, double-tap zoom, pinch, one-finger pan, control toggling, and navigation reset. Leave the Mac app running for trackpad testing.

- [ ] **Step 6: Commit Task 4**

```bash
git add apps/interface/src/viewer/useViewerGestures.ts apps/interface/src/viewer/useViewerGestures.test.ts apps/interface/src/viewer/viewerTouchGesture.ts apps/interface/src/viewer/viewerTouchGesture.test.ts apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/styles/photoViewer.module.css
git commit -m "feat: support touch viewer zoom and pan"
```

---

### Task 5: Add the contextual navigator

**Files:**
- Create: `apps/interface/src/components/ViewerNavigator.tsx`
- Create: `apps/interface/src/components/ViewerNavigator.test.ts`
- Modify: `apps/interface/src/viewer/viewerTransform.ts`
- Modify: `apps/interface/src/viewer/viewerTransform.test.ts`
- Modify: `apps/interface/src/components/PhotoViewerOverlay.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/styles/photoViewer.module.css`

**Interfaces:**
- Consumes: Task 1 `visibleImageRect`, Task 2 controller `recenter`, best decoded derivative URL, control visibility, interaction state, drawer state, viewport size, and safe areas.
- Produces:

```ts
export interface ViewerNavigatorProps {
	assetName: string;
	imageUrl: string | null;
	imageWidth: number;
	imageHeight: number;
	visibleRect: { x: number; y: number; width: number; height: number };
	visible: boolean;
	interactive: boolean;
	onRecenter: (focal: ViewerPoint) => void;
	onInteraction: () => void;
}

export function navigatorPointToFocal(
	point: ViewerPoint,
	bounds: { left: number; top: number; width: number; height: number },
): ViewerPoint;
```

- [ ] **Step 1: Write failing navigator geometry tests**

```ts
it("maps navigator points to clamped normalized focal coordinates", () => {
	expect(navigatorPointToFocal(
		{ x: 180, y: 70 },
		{ left: 80, top: 20, width: 200, height: 100 },
	)).toEqual({ x: 0.5, y: 0.5 });
	expect(navigatorPointToFocal(
		{ x: 400, y: -20 },
		{ left: 80, top: 20, width: 200, height: 100 },
	)).toEqual({ x: 1, y: 0 });
});
```

Add component tests for aspect-correct image bounds and viewport rectangle percentages.

- [ ] **Step 2: Run focused unit tests and verify RED**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/components/ViewerNavigator.test.ts src/viewer/viewerTransform.test.ts
```

Expected: FAIL because the navigator does not exist.

- [ ] **Step 3: Implement the navigator component and desktop input**

Render a labelled visual container with the best current decoded image and an `aria-hidden` viewport rectangle. The container itself has `aria-hidden="true"` on mobile display-only mode. Desktop mode supports click-to-centre and pointer-captured dragging. Every movement calls the controller's `recenter` and reports interaction.

The navigator is present only when `mode === "zoomed"` and either controls are visible or manipulation is active. It fades with the existing controls and unmounts at fit. It must not be tabbable because equivalent zoom and pan operations already exist through the keyboard and zoom controls.

Position it above the zoom cluster. Recalculate its right offset from the open drawer width and `safe-area-right`. Use no native or source URL; consume only `service.derivativeUrl` output already accepted by the stage.

- [ ] **Step 4: Add browser tests for visibility, accuracy, drag, and layout**

Cover:

- absent at fit
- appears immediately after zoom
- viewport rectangle matches the transform's normalized visible rectangle
- click recentres and desktop drag pans continuously
- touch navigator remains display-only and does not capture stage gestures
- hides after inactivity and stays visible during manipulation
- resets away on photo navigation
- shifts left of an open Info drawer
- safe-area-correct in landscape rotation
- uses the wall thumbnail until the decoded preview is ready, then swaps image without moving the rectangle
- high-contrast viewport border and reduced-motion fade suppression

- [ ] **Step 5: Run Task 5 verification and demonstrate the complete viewer interaction**

```bash
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/components/ViewerNavigator.test.ts src/viewer/viewerTransform.test.ts
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t "navigator|zoom controls|safe area"
npm test
npm run typecheck
npm run check
npm run --workspace @photo-viewer/interface build
git diff --check
```

Demonstrate zoom, pan, navigator click and drag, drawer avoidance, resize preservation, and next-photo reset in the running macOS app.

- [ ] **Step 6: Commit Task 5**

```bash
git add apps/interface/src/components/ViewerNavigator.tsx apps/interface/src/components/ViewerNavigator.test.ts apps/interface/src/viewer/viewerTransform.ts apps/interface/src/viewer/viewerTransform.test.ts apps/interface/src/components/PhotoViewerOverlay.tsx apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/styles/photoViewer.module.css
git commit -m "feat: add viewer zoom navigator"
```

---

### Task 6: Complete accessibility, offline evidence, and native acceptance

**Files:**
- Modify: `apps/interface/src/components/PhotoViewer.browser.test.tsx`
- Modify: `apps/interface/src/components/PhotoViewer.motion.test.tsx`
- Modify: `README.md`
- Create: `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`

**Interfaces:**
- Consumes: Tasks 1 through 5 and the existing controlled source fixtures.
- Produces: complete automated evidence, source-read-only audit, unsigned macOS bundle, and a running app for user acceptance.

- [ ] **Step 1: Add final accessibility and offline browser assertions**

Run Axe at fit, zoomed with controls, zoomed with hidden controls, zoomed with navigator, open Info drawer, and rotated phone layout. Assert no serious violations.

Add explicit assertions that:

- all zoom controls are at least 44 by 44 CSS pixels
- disabled limits expose native disabled state
- `Fit` or percentage is announced through a polite live region after discrete input
- focus remains contained when zoom chrome hides
- navigator is not an extra keyboard or screen-reader interaction trap
- high-contrast mode keeps the viewport rectangle visible
- reduced-motion mode removes navigator fade and transform settling
- cached wall thumbnail and screen preview remain zoomable after `sourceUnavailable`
- no deliberate `file:///private/source/secret.jpg` sentinel reaches the viewer DOM

- [ ] **Step 2: Add full interaction-boundary browser coverage**

Assert that viewer-consumed wheel, pinch, touch pan, and double-tap prevent document scroll or browser zoom only while the viewer is open. Close the viewer and prove the same synthetic document-level wheel event is no longer prevented by viewer code.

Test malformed dimensions and a derivative with zero natural width. The viewer must remain at Fit with zoom-in disabled and navigation usable.

- [ ] **Step 3: Run the complete interface suite**

```bash
npm test
npm run test:browser
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
npm run typecheck
npm run check
npm run --workspace @photo-viewer/interface build
```

Expected: all unit, WebKit, normal-motion, accessibility, type, format, and production-build checks pass.

- [ ] **Step 4: Run the complete Rust and desktop gates**

No Rust change is expected, but the final feature branch must preserve the full product:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-viewer-zoom.json
npm run desktop:build -- --bundles app
```

- [ ] **Step 5: Audit the source-media invariant**

Hash `apps/interface/public/demo-photos` immediately before and after the controlled derivative-generation test:

```bash
find apps/interface/public/demo-photos -maxdepth 1 -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
cargo test -p photo-cache --test image_derivative controlled_demo_fixture_generation_leaves_source_unchanged
find apps/interface/public/demo-photos -maxdepth 1 -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

The aggregate hashes must match. Audit the full zoom feature range for source write, rename, move, copy, rating, tag, deletion, or native-path operations. Record the exact base and head commits. Do not inspect or record the user's selected native path.

- [ ] **Step 6: Run the macOS acceptance demonstration**

Use `PHOTO_VIEWER_PROFILE=wall-demo npm run desktop:dev` and demonstrate:

1. Double-click to native 100 percent
2. Trackpad pinch centred under the pointer
3. Mouse drag and ordinary trackpad pan
4. Plus, minus, Fit, `+`, `-`, and `0`
5. Contextual navigator and desktop viewport dragging
6. Thumbnail-to-preview refinement without a transform jump
7. Keyboard, button, and filmstrip navigation resetting to fit
8. Narrow and wide window resizing preserving the focal point
9. Cached zoom while the source is unavailable, only if safely reproducible

Leave the app running for user exploratory acceptance. Do not merge or push.

- [ ] **Step 7: Document and commit final evidence**

Update `README.md` with zoom controls and cache-only limitations. Record exact command results, counts, normal-motion evidence, bundle path, fixture hash equality, source audit, native observations, and limitations in `docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md`.

Run:

```bash
git diff --check
git status --short
```

Remove generated browser screenshot and attachment directories before commit. Commit:

```bash
git add README.md docs/superpowers/verification/2026-08-27-viewer-zoom-pan.md apps/interface/src/components/PhotoViewer.browser.test.tsx apps/interface/src/components/PhotoViewer.motion.test.tsx
git commit -m "test: verify viewer zoom and pan"
```

Do not merge or push. Report the feature branch and running-app state for user acceptance.
