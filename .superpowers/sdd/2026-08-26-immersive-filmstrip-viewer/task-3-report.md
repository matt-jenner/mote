# Task 3 report: open, view, and exact-return checkpoint

## Status

Implemented and committed as `78ef56572410e0aa5349da019e257a3221253283` (`feat: open photos in immersive viewer`). The worktree is clean.

## TDD evidence

### RED

Added `apps/interface/src/components/PhotoViewer.browser.test.tsx` before production implementation. The focused WebKit command was:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx
```

After granting the loopback-server permission, the test failed as expected with 2 failures: `Cannot find element with locator: page.getByRole('button', { name: 'Open Coast' })`. The existing wall rendered figure tiles and had no accessible open control, establishing the feature-missing RED state. (The initial sandboxed invocation was blocked before test execution by `listen EPERM ::1`; it was rerun with the required loopback permission.)

### GREEN

The same focused command passed after implementation:

```text
Test Files  1 passed (1)
Tests       2 passed (2)
```

The complete WebKit browser suite also passed:

```text
Test Files  3 passed (3)
Tests       51 passed (51)
```

Additional verification passed:

```text
npm test       # Test Files 6 passed, Tests 78 passed
npm run typecheck  # exit 0
npm run check      # Checked 44 files; no fixes applied
```

## Files changed

- `apps/interface/src/components/AppShell.tsx`: owns viewer reducer, wall region anchor, inert workspace, exact scroll/focus restoration, and return highlight.
- `apps/interface/src/components/JustifiedWall.tsx`: passes open and highlight callbacks to tiles.
- `apps/interface/src/components/PhotoTile.tsx`: renders accessible open buttons for viewable assets and inert figures for unavailable assets without derivatives.
- `apps/interface/src/components/PhotoWallCanvas.tsx`: receives the lifted wall region ref and marks the mounted wall.
- `apps/interface/src/components/SourceCanvas.tsx`: forwards viewer props to the wall canvas.
- `apps/interface/src/components/PhotoViewerOverlay.tsx`: fixed, modal viewer shell with safe-area-aware 44px back target and Escape handling.
- `apps/interface/src/components/ViewerStage.tsx`: fitted catalog-dimension frame and screen-preview, wall-thumbnail, or representative-colour fallback ordering.
- `apps/interface/src/components/PhotoViewer.browser.test.tsx`: focused WebKit open/return and first-frame tests with controlled ready fixtures.
- `apps/interface/src/styles/photoWall.module.css`: button tile reset and non-moving return highlight.
- `apps/interface/src/styles/photoViewer.module.css`: edge-to-edge fixed viewer and stage styling.

## Self-review

- The wall remains mounted while the overlay is open; only the workspace is inert.
- The anchor is captured from the wall region at open time and restored in a layout effect on close.
- WebKit can reset a scroll container when its ancestor becomes inert. The opening layout effect immediately reapplies the captured scroll position, and tile focus uses `preventScroll` so focusing the origin does not change the restored offset.
- Viewer derivative selection catches stale/unavailable URLs and falls through from screen preview to wall thumbnail to representative colour.
- Dark-stage clicks have no close handler; only the back control and Escape close the viewer.
- Generated Vitest attachments and screenshots were removed; none are tracked.

## Native smoke

No long-running native GUI process was started in this subtask. The automated WebKit checkpoint is complete; the coordinator is responsible for the bounded macOS native launch/smoke pass after review.

## Concerns

- Native macOS smoke remains outstanding for the coordinator, per the task brief.
- Viewer navigation beyond the open/close checkpoint (filmstrip, next/previous, metadata controls) is intentionally deferred to later tasks.
