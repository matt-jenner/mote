# Task 4 report: progressive viewer preview refinement

## Status

Implemented and committed as `cef2e7d` (`feat: refine viewer photos from preview cache`).

## TDD evidence

### RED

Added the pure preview-plan tests and a focused browser regression before production implementation.

The focused unit command initially failed because the requested planner module did not exist:

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/viewer/viewerPreviewPlan.test.ts
Error: Cannot find module './viewerPreviewPlan'
Test Files  1 failed (1)
Tests       no tests
```

The focused WebKit command then showed the missing request behavior. After granting loopback-server permission, it reported:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
Test Files  1 failed (1)
Tests       1 failed | 5 passed (6)
```

The failing test was `requests the visible photo as a screen preview`, which observed no `screenPreview` request.

### GREEN

The focused unit planner command passed after implementation:

```text
Test Files  1 passed (1)
Tests       2 passed (2)
```

The focused WebKit command passed:

```text
Test Files  1 passed (1)
Tests       6 passed (6)
```

Final verification passed:

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit
Test Files  7 passed (7)
Tests       80 passed (80)

npm run test:browser --workspace @photo-viewer/interface
Test Files  3 passed (3)
Tests       55 passed (55)

npm run typecheck
exit 0

npm run check
Checked 47 files in 57ms. No fixes applied.

git diff --check
exit 0
```

## Files changed

- `apps/interface/src/viewer/viewerPreviewPlan.ts`: pure current, immediate-neighbour, and bounded idle-neighbour screen-preview plan builder.
- `apps/interface/src/viewer/viewerPreviewPlan.test.ts`: plan ordering and bounds tests.
- `apps/interface/src/viewer/useViewerPreview.ts`: visible and near-viewport screen-preview requests, idle scheduling/cancellation, request de-duplication, retry handling, URL resolution, and failure state.
- `apps/interface/src/components/PhotoViewerOverlay.tsx`: derives the current wall index and supplies preview state and generation to the stage.
- `apps/interface/src/components/ViewerStage.tsx`: keeps the wall thumbnail base mounted, decodes the screen preview before revealing it, fences stale completion by asset/generation/URL, and preserves fallback rendering.
- `apps/interface/src/components/PhotoViewer.browser.test.tsx`: visible screen-preview request regression.
- `apps/interface/src/styles/photoViewer.module.css`: fitted-rectangle preview layering, opacity-only crossfade, and reduced-motion behavior.

## Self-review

- The planner remains browser-free and emits the exact derivative request shapes required by the shared service contract.
- Immediate requests are sent synchronously from the hook effect; the bounded distance-two group is scheduled through `requestIdleCallback` with a timeout fallback and cancelled on cleanup.
- Requests are keyed by asset and derivative class. Rejected requests clear only the matching attempt and retry; warning-code/retryability changes permit another attempt.
- The wall thumbnail remains visible while a screen preview is pending or fails. Decode success alone changes preview opacity; frame geometry remains tied to catalog dimensions and resize only updates fitted bounds.
- Decode completion checks an asset/generation/URL token, so an older promise cannot reveal a newer viewer asset.
- The stage exposes `data-large-preview-unavailable="true"` for failed/unavailable screen previews for the later information UI.
- Generated Vitest screenshots and attachments were removed and are not tracked.

## Concerns

- Viewer navigation and information UI are intentionally deferred by the Task 4 scope, so the stale-completion path is implemented at the stage boundary but not exercised by a navigation browser test in this task.
- Native macOS smoke remains the coordinator's responsibility.

## Fix round 1

### Findings addressed

- Retry ticks are now observable dependencies of the request effect, so a rejected request clears its matching attempt and deterministically re-runs the current plan.
- Request records retain priority and generation. A near-viewport request is promoted to a visible request on selection unless a visible request for that asset and generation is already recorded.
- ViewerStage stores the exact decoded token and only renders a ready screen layer when that token equals the current asset/generation/URL token, eliminating one-commit readiness leakage.
- Added a WebKit harness using the real `useViewerPreview` and `ViewerStage`: it controls decode promises for assets A and B, switches generation/current asset, resolves B, then resolves stale A and verifies B remains current and ready.

### RED evidence

Before the fix, the focused WebKit run showed the two production regressions:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
Test Files  1 failed (1)
Tests       3 failed | 6 passed (9)
```

The failures were `retries a rejected screen-preview request` (only the initial attempt), `promotes a near-viewport request to visible after switching assets` (no visible promotion), and the initial stale-decode harness fixture (the first data-GIF fixture was invalid and could not reach ready state; it was corrected before the final GREEN run).

### GREEN evidence

After the fixes and valid deterministic image fixture, the focused viewer WebKit tests passed:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
Test Files  1 passed (1)
Tests       9 passed (9)
```

The complete verification set passed:

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit
Test Files  7 passed (7)
Tests       80 passed (80)

npm run test:browser --workspace @photo-viewer/interface
Test Files  3 passed (3)
Tests       58 passed (58)

npm run typecheck
exit 0

npm run check
Checked 47 files in 72ms. No fixes applied.

git diff --check
exit 0
```

### Fix-round files and self-review

- `apps/interface/src/viewer/useViewerPreview.ts`: observable retry trigger and priority/generation-aware promotion.
- `apps/interface/src/components/ViewerStage.tsx`: exact decoded-token readiness.
- `apps/interface/src/components/PhotoViewer.browser.test.tsx`: deterministic rejection/retry, near-to-visible promotion, and real hook/stage stale-decode regressions.

The retry test rejects only the first A request and observes the second request. The promotion test first observes B near-viewport, then switches to B and requires a one-asset visible request. The stale test resolves B before stale A and confirms the stage remains on B with its ready layer.

### Fix-round concerns

- Navigation controls remain deferred to Task 5; the stale harness supplies only the minimal current-index/generation switch needed to exercise the product path.
- Native macOS smoke and the unavailable-marker/reduced-motion/report-tracking Minor ledger items remain coordinator scope.

## Fix round 2

### Stale-decode regression correction

Rewrote the WebKit harness sequence so asset A's decode promise remains unresolved across the switch to asset B (generation 2). The test now captures and resolves B first, verifies B's screen layer is ready and visible, then resolves A, flushes the completion, and verifies that B remains current and ready with no A URL/layer becoming visible.

No production code changed in this round.

### Verification evidence

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
Test Files  1 passed (1)
Tests       9 passed (9)

npx biome check --write apps/interface/src/components/PhotoViewer.browser.test.tsx
Checked 1 file in 14ms. No fixes applied.

git diff --check
exit 0
```

### Self-review

- A remains pending while B is selected, so the final A resolution is a genuine stale completion rather than a no-op.
- B readiness is asserted before A is released; the post-flush assertions require the same B URL and ready state.
- The harness invokes the real `useViewerPreview` and `ViewerStage` components and controls only the browser image decode boundary.
