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

## Fix round 1 — RED

- Fix base: `8eadd0c` (`docs: record Task 6 verification`).
- The real `<StrictMode>` viewer test was run before the hook fix from the
  initial render through open: it observed two visible current screen-preview
  requests instead of one (`expected 1, received 2`). This reproduced the
  effect-replay cleanup bug.
- The focused wall/viewer regression additions were kept asynchronous and
  event-driven; the request assertion no longer uses a fixed sleep.

## Fix round 1 — GREEN

- `npm test` — 16 files, 129 tests passed.
- `npm run test:browser` — 3 files, 142 tests passed.
- Exact `PhotoWall.browser.test.tsx` — 47 tests passed.
- Exact `PhotoViewer.browser.test.tsx` — 85 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion` — 1 file, 2 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast` — 1 file, 2 tests passed.
- `npm run typecheck`, `npm run check`, production build, and `git diff --check` — passed.

The full browser run still emits the one existing non-failing React
`ViewerStage` `act(...)` warning. The warning count did not increase; the exact
wall run emitted none.

## Fix round 1 — Browser

- `useViewerPreview` now retains request records through StrictMode effect
  replay, while active asset/generation/key cleanup still resets stale plans;
  refs naturally disappear on a genuine unmount, and late completions/retries
  are mounted-fenced.
- The StrictMode test counts from initial render, asserts zero before the
  painted tile click, one visible current request after open, no duplicate
  after completion, and one new request after close/reopen.
- Wall coverage now asserts a screen-preview-only colour tile has no request,
  no button, and no `onOpen`; error and every stale load/decode/transition
  path likewise stays inert. Cached-complete imagery is driven through its
  current decode/transition and then opened exactly once.
- Reduced-motion URL replacement and unmount tests prove pending RAFs are
  cancelled and stale callbacks cannot expose/open a tile.
- The combined Escape test uses a `MutationObserver`: continuous wheel zoom
  leaves the prior `Fit` announcement untouched, while the second Escape
  itself resets to Fit and produces a discrete live-region mutation before the
  third Escape closes the viewer.

## Fix round 1 — Appearance

No production appearance changes were made. The existing contrast project
continued to verify the always-dark viewer surface under system-light
appearance.

## Fix round 1 — Source boundary

The fix is limited to interface preview lifecycle code and browser tests. No
backend, Tauri, plan/specification, progress-ledger, source-media, or native
path files changed; no video request path was added.

## Fix round 1 — Self-review

- Reviewed the hook cleanup against both StrictMode replay and real unmount:
  in-flight dedupe survives replay, current asset/generation/key changes are
  still fenced, retries are cancelled, and completion handlers cannot update
  an unmounted viewer.
- Confirmed generated browser attachment/screenshot directories were removed
  before commit and `git diff --check` is clean.

## Fix round 1 — SHA

- Implementation and regression tests: `b7c5748` (`fix: preserve preview dedupe across StrictMode`).

## Fix round 2 — RED

- Fix base: `a1ce0af` (`docs: record Task 6 Fix Round 1 verification`).
- The inactive-key regression failed before the fix: after three rejected
  attempts, deactivation and reactivation produced no fourth request
  (`expected 4, received 3`). This exposed `attemptCounts` and
  `failedRequestKeys` surviving after `requestRecords` was removed.
- The integrated screen-only wall fixture initially rendered provisionally;
  it was corrected to a settled page so the actual PhotoWall/AppShell tile
  path could be exercised. The stale decode assertion was changed from a
  fixed 25ms delay to an explicit decode-continuation flag.

## Fix round 2 — GREEN

- `npm test` — 16 files, 129 tests passed.
- `npm run test:browser` — 3 files, 145 tests passed.
- Exact `PhotoWall.browser.test.tsx` — 48 tests passed.
- Exact `PhotoViewer.browser.test.tsx` — 87 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion` — 1 file, 2 tests passed.
- `npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast` — 1 file, 2 tests passed.
- `npm run typecheck`, `npm run check`, production build, and `git diff --check` — passed.

The same single non-failing React `ViewerStage` `act(...)` warning remains in
the viewer/full browser runs; no new warning was introduced.

## Fix round 2 — Browser

- Inactive cleanup now removes request records, attempt counts, retry timers,
  and failed-key state together for every inactive asset/generation/key.
  React state is mirrored by a ref so cleanup can clear the plan without
  leaking state across a later activation.
- Real unmount sets the mounted fence immediately and defers full plan cleanup
  to a token-checked microtask. StrictMode replay cancels that deferred clear,
  preserving in-flight dedupe; a genuine unmount clears timers and all plan
  maps. Late unresolved completions cannot mutate state or schedule retries.
- The integrated AppShell/PhotoWall test dispatches pointer and click events
  on a screen-preview-only colour tile, verifies no open button/dialog, and
  confirms no screen-preview request or additional derivative request follows
  activation.
- The zero-natural-width case now includes an `onOpen` spy, real tile
  activation attempts, and explicit no-button/no-call assertions.

## Fix round 2 — Appearance

No production appearance changes were made. The always-dark viewer contrast
coverage continues to pass under system-light appearance.

## Fix round 2 — Source boundary

Only interface preview lifecycle code and browser tests changed. No backend,
Tauri, plan/specification, progress-ledger, source-media, or native-path files
were touched, and no video request behavior was added.

## Fix round 2 — Self-review

- Reviewed StrictMode setup/cleanup ordering: the mounted fence is immediate,
  deferred cleanup is cancelled by replay setup, and active-key cleanup still
  resets current asset/generation/key changes.
- Confirmed generated browser attachment/screenshot directories were removed
  before commit; the worktree is clean and `git diff --check` passes.

## Fix round 2 — SHA

- Implementation and regression tests: `f59dae5` (`fix: clear inactive preview retry state`).
