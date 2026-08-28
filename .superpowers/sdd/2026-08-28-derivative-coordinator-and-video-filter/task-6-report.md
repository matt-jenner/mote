# Task 6 report — thumbnail paint before viewer open

## RED

- Baseline HEAD: `ab6893d` (`docs: record Task 5 fix round 5`).
- The pre-implementation wall suite was green at 42 tests.
- After adding the paint-before-open browser test, the exact wall command failed as intended: `1 failed | 42 skipped (43)`. The tile exposed its open button as soon as decode completed, before the opacity transition had ended.

## GREEN

- `npm test` — 16 files, 129 tests passed.
- `npm run test:browser` — 3 files, 139 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion` — 1 file, 2 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast` — 1 file, 2 tests passed.
- `npm run typecheck` — passed.
- `npm run check` — 69 files checked, no errors.
- `npm run --workspace @photo-viewer/interface build` — production build passed.
- `git diff --check` — passed.
- Exact final wall command — 45 tests passed.

The browser suite still emits the one existing non-failing React `act(...)` warning from `ViewerStage`; no new warning was introduced.

## Browser behavior

- `PhotoTile` now keeps a stable figure, image, placeholder, colour, and warning layer through `placeholder → decoding → fading → interactive` (or `failed`).
- The open control is a separate absolute overlay and appears only for the current revision after successful load/decode plus opacity transition. Reduced motion uses one cancellable RAF.
- Cached-complete images, load/decode rejection, URL replacement, stale decode, stale transition, and unmount cleanup are covered. The image and handlers are revision-fenced, and late failures cannot reopen a tile.
- Unexpected video fixtures remain inert without a caption/count. Viewer preview planning and URL resolution also exclude video assets, so no video request is made.
- Opening a painted photo produces exactly one visible current screen-preview request. Screen-only colour tiles cannot open.
- Viewer Escape uses current refs in the required order: information drawer, zoom Fit, then viewer close. Discrete zoom labels continue to mutate the polite live region; wheel/pinch frames remain silent.
- Returning from the viewer focuses the stable tile’s interactive overlay and retains the return highlight.

## Appearance

The contrast project now renders the viewer under `data-theme="light"`/system-light appearance and verifies the deliberate dark viewer token: overlay and canvas compute to `rgb(8, 9, 11)`, while the chrome button uses the matching translucent dark surface.

## Source boundary

Changes are limited to the interface wall/viewer, its browser/contrast tests, and the AppShell return-focus adaptation required by the stable figure root. No backend, Tauri, plan, specification, or progress-ledger files were changed. No source-media or filesystem read path was added, and no video request path was introduced.

## Self-review

- `git diff --check` and the full interface gates are clean.
- No debug logging, stale timers, unbounded preview work, or new warning output remains.
- The code-review skill normally requests a reviewer subagent, but the task explicitly prohibited subagents; the final diff was reviewed locally against each brief requirement instead.

## SHA

- Implementation commit: `08371e2` (`fix: paint thumbnails before opening photos`).
