# Task 6 implementation report

Status: implementation complete, awaiting task-scoped review.

Implementation commit: `49fd533` (`feat: connect browser galleries over HTTP`), based on `33f1a43`.

## Delivered behavior

- Added the host-neutral folder listing, folder-browser recovery, capability mode, and sort-preference methods to `PhotoService`.
- Added strict versioned browser preferences at `photo-viewer.hosted.v1` and a per-tab client ID at `photo-viewer.client.v1`. The local payload contains only selection ID, structured breadcrumbs, appearance, gallery scope, and sort direction.
- Added the hosted HTTP adapter with origin-relative folder, selection, wall, derivative, interaction, event, and opaque derivative URLs.
- Added strict success-payload decoding and fixed, path-free public error mapping. A saved selection returning 404 clears only its ID and retains structured breadcrumbs for Task 7 recovery.
- Added SSE replay keyed to selection and scope, browser-compatible `afterEventId`, capped 250 ms to 5 s reconnect delays, strict wall-update decoding, and one `resyncRequired` delivery per detected gap.
- Added active interaction refresh on entry, every 10 seconds, and again when the event stream opens. Idle, stream disposal, selection replacement, stale-selection recovery, and adapter disposal cancel the refresh timer.
- Added origin-relative opaque derivative URLs such as `/api/v1/derivatives/cache%2Fkey`.
- Initialized the wall reducer from the adapter's stored direction, preserved that direction across source reset and resync, removed the first-query `oldestFirst` override, and persisted only accepted direction changes.
- Kept memory and Tauri adapters on explicit native picker behavior. Their hosted folder methods reject with `unsupportedCapability`, and their sort preference remains adapter-local.
- Selected the HTTP adapter only in Vite `hosted` mode. Memory and desktop modes keep their existing adapters.

The compile-conformance fixture in `PhotoViewer.browser.test.tsx` and the existing memory-adapter contract tests were updated even though they were not named in the Task 6 file list. Both directly implement or exercise the expanded `PhotoService` interface.

## RED evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/browserPreferences.test.ts src/services/httpPhotoService.test.ts src/wall/wallReducer.test.ts
  RED: the two new service modules were missing; reducer reset returned oldestFirst instead of newestFirst. Existing reducer coverage: 36 passed.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoWall.browser.test.tsx
  Initial sandbox run: EPERM while binding the local browser-test server.
  Approved rerun RED: 50 passed, 1 failed. The first wall request used oldestFirst instead of the restored newestFirst direction.
```

## GREEN evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/browserPreferences.test.ts src/services/httpPhotoService.test.ts src/wall/wallReducer.test.ts
  PASS: 47 passed, 0 failed

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/PhotoWall.browser.test.tsx
  PASS: 51 passed, 0 failed

npm run check
  PASS: Biome checked 73 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 142 tests

npm run test:browser
  PASS: 3 files, 148 tests

CARGO_BUILD_JOBS=2 cargo check --offline --jobs 2 --manifest-path apps/desktop/src-tauri/Cargo.toml
  PASS: desktop and dependent Rust crates checked successfully

git diff --check
  PASS: no whitespace errors
```

The full browser run emitted the same pre-existing React `act(...)` warning recorded in the branch baseline. It had no test failure. The desktop check regenerated two dependency-list lines in its nested lockfile from earlier Task 5 manifests; Task 6 adds no Rust dependency, so those two lines were restored and the lockfile is unchanged from HEAD.

## Cleanup and constraints

- Removed RED-run `.vitest-attachments` and screenshot directories before staging.
- No source-media, Rust production, dependency, or server file changed.
- No network access, dependency installation, Cargo clean, background build, merge, or push occurred.
- Cargo and browser commands ran serially in the foreground. A final read-only process check found no Cargo, rustc, Vitest, Vite, or Playwright process.

## Concerns

Task 7 still owns the hosted folder dialog and shell branch. Until that task lands, hosted mode has the HTTP service and capability contract but no contained-folder picker UI. No other implementation concern remains from Task 6.
