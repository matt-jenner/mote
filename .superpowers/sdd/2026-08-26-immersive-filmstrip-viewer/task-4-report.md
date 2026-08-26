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
