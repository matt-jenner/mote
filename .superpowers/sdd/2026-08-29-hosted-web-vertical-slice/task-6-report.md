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

## Task 6 fix round 1

Status: implementation complete at `5b9ecd8` (`fix: validate hosted event replay cursors`), based on the reviewed documentation head `3bdd930`; awaiting Sol re-review.

The review found that the event adapter accepted any one-to-twenty-digit replay ID. A decreasing ID could move the reconnect cursor backward, while a decimal above Rust's `u64::MAX` could make every reconnect fail at the server. The adapter now parses candidate IDs with `BigInt`, rejects empty, nondecimal, and out-of-range values, accepts only values strictly greater than the retained cursor, and stores the accepted decimal canonically. Selection or gallery-scope changes reset both representations of the cursor, so replay remains confined to the active `(selectionId, scope)` pair.

The requested discriminating coverage now proves:

- two independent local-storage contexts restore different selection, appearance, gallery scope, and sort direction against the same injected server, while independent session stores receive distinct client IDs;
- malformed, extra-field, and unknown-enum success DTOs fail closed, while malformed, extra-field, and unknown-kind SSE payloads produce one authoritative resync instead of leaking partial data;
- duplicate, decreasing, empty, nondecimal, and overflowing replay IDs cannot replace a valid cursor; the maximum `u64` value is accepted; scope context changes do not reuse the prior pair's cursor;
- reconnect delays progress through 250, 500, 1000, 2000, 4000, and capped 5000 ms attempts, and queued reconnects do not survive watch stop or adapter disposal;
- interaction refresh timers stop on explicit idle, selection replacement, stream stop, and adapter disposal;
- selection-summary identity remains consistent across bootstrap, representative nonempty wall pages, event streams, and their route URLs.

### Fix-round RED evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts src/services/browserPreferences.test.ts src/wall/wallReducer.test.ts
  RED: 1 failed, 55 passed. After replay ID 10 followed by decreasing ID 9, reconnect used afterEventId=9 instead of retaining 10.

npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts -t "does not replace a valid replay cursor with a decimal above u64 max"
  RED: 1 failed, 15 skipped. After valid ID 10, reconnect used afterEventId=18446744073709551616 instead of retaining 10.
```

The remaining newly added matrix cases passed during RED. One test harness correction emitted the EventSource `open` event after a successful reconnect; this makes the deterministic fake model the production behavior that resets backoff after connection establishment.

### Fix-round GREEN and final gates

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts src/services/browserPreferences.test.ts src/wall/wallReducer.test.ts
  PASS: 3 files, 57 tests

npm run check
  PASS: Biome checked 73 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 152 tests

npm run test:browser
  PASS: 3 files, 148 tests

CARGO_BUILD_JOBS=2 cargo check --offline --jobs 2 --manifest-path apps/desktop/src-tauri/Cargo.toml
  PASS: desktop compatibility check exited 0

git diff --check
  PASS: no whitespace errors
```

The browser run retained the same non-fatal React `act(...)` warning recorded at the original checkpoint. Cargo again regenerated the two Task 5 dependency-list entries in the nested desktop lockfile; Task 6 adds no Rust dependency, so only that mechanical lockfile churn was removed and `Cargo.lock` remains unchanged. No attachments or screenshot baselines were generated, and the final foreground-process check found no Cargo, rustc, Vitest, Vite, or Playwright process. No network access, dependency installation, Cargo clean, background command, merge, or push occurred.

## Task 6 fix round 2

Status: implementation complete at `7ce7155` (`fix: require canonical event replay IDs`), based on `6a69c26`; awaiting Sol re-review.

The fix-round 1 report overstated two parts of its test evidence. The two-context test did not directly inspect `folderBrowserState()`, and only a scope change directly proved replay-context reset. The corrected bullets above describe what that round actually tested. This round now checks both adapters' folder-browser state, initial path, and defensive breadcrumb copies. A separate selection replacement test proves that selection A's replay ID is absent from selection B's initial stream, then proves selection B's accepted replay ID appears on its own reconnect.

The remaining production defect was the replay regex. It admitted any one-to-twenty-digit string, so `010` and `00` passed before `BigInt` normalized them. The validator now admits exactly `0` or a nonzero ASCII decimal digit followed by at most nineteen more digits. The existing `BigInt` comparison still enforces `u64::MAX` and strict monotonic increase.

### Fix-round 2 RED evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts src/services/browserPreferences.test.ts src/wall/wallReducer.test.ts
  RED: 2 failed, 57 passed.
  Cursor 9 followed by noncanonical 010 reconnected with afterEventId=10 instead of retaining 9.
  Noncanonical 00 from empty replay state reconnected with afterEventId=0 instead of omitting the parameter.
```

The restored folder-browser and selection-switch tests passed during RED. They close acceptance-proof gaps rather than reproduce a production failure.

### Fix-round 2 GREEN and final gates

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project unit src/services/httpPhotoService.test.ts src/services/browserPreferences.test.ts src/wall/wallReducer.test.ts
  PASS: 3 files, 59 tests

npm run check
  PASS: Biome checked 73 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 154 tests

npm run test:browser
  PASS: 3 files, 148 tests

CARGO_BUILD_JOBS=2 cargo check --offline --jobs 2 --manifest-path apps/desktop/src-tauri/Cargo.toml
  PASS: desktop compatibility check exited 0

git diff --check
  PASS: no whitespace errors
```

The browser suite emitted the existing non-fatal React `act(...)` warning. The desktop check regenerated the same two unrelated Task 5 dependency-list entries, which were removed so `Cargo.lock` remains unchanged. No browser attachments or screenshot baselines were generated. A process check found no Cargo, rustc, Vitest, Vite, or Playwright process after the final-source gate. All commands ran serially in the foreground, with no network access, dependency installation, Cargo clean, merge, or push.
