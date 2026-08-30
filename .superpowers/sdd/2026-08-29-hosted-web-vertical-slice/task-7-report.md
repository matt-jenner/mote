# Task 7 implementation report

Status: implementation complete; review fix round 1 applied.

Implementation commits:

- `383ba85` (`feat: choose hosted gallery folders responsively`), based on `fa353ab`.
- `39ff25e` (`fix: keep hosted folder selection coherent`), based on `12959b5`.

## Delivered behavior

- Added a contained hosted folder browser that uses `PhotoService` only. React never imports browser preferences, HTTP code, Tauri code, or native paths.
- Added structured breadcrumb recovery. The browser tries saved `breadcrumb.path` values from deepest to shallowest, then the mounted root. Back and clickable breadcrumbs also use only paths returned by `listFolders`.
- Kept the native picker route unchanged. The shell opens the contained browser only when `capabilities.folderSelection === "hosted"`.
- Added a centred desktop modal capped at 640 px by 70 vh. Narrow viewports and coarse pointers use a full-height sheet inside the safe-area insets. Folder rows meet the 44 px touch-target minimum.
- Added dialog semantics, focus entry and containment, Escape handling, trigger restoration, `aria-busy`, loading and empty states, error alerts, Retry, and reduced-motion handling.
- Preserved the last good listing when a child request fails. Monotonic request tokens prevent stale or unmounted requests from replacing current state, and recovery stops before issuing another fallback read after unmount.
- Once folder selection is submitted, the dialog enters a non-dismissible busy state: Close and navigation are disabled, Escape is ignored, and keyboard focus remains inside the dialog. A selected result is always delivered to AppShell, which closes the hosted flow; cancellation or failure restores interaction and focus to Open.
- Applied selected hosted state through the existing bootstrap query boundary. The source title, justified wall, and existing photo viewer update without changing another browser's service-owned state.
- Kept hosted mode free of a Locate Folder action.

## RED evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/App.browser.test.tsx
  RED: 10 passed, 1 failed.
  The hosted Folders action called chooseFolder(), exposed "Hosted mode must not open the native picker", and never rendered the contained dialog.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx
  RED: the test file failed import analysis because ./HostedFolderBrowser did not exist.
```

The App failure discriminates the missing capability branch. The component failure proves the planned contained-browser boundary was absent before production work.

## Focused GREEN evidence

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx src/components/App.browser.test.tsx
  PASS: 2 files, 27 tests
```

The focused files cover desktop modal bounds, safe-area phone layout, coarse-pointer layout, focus trap and restoration, loading and retry, last-good-listing retention, deepest-first recovery, root fallback, prompt recovery stop after unmount, server-derived Back navigation, pending-selection dismissal blocking, selection cancellation, reduced motion, axe, native and hosted routing, selected source updates, breadcrumb resume, justified wall rendering, viewer open and return, and the complete phone-width folder-to-viewer flow.

The first component GREEN run found two focused issues. A coarse-pointer sheet retained the desktop backdrop padding, and the standalone focus test opened from an unfocused pointer target. The sheet now removes that padding. The keyboard test opens from a focused trigger, while AppShell records an explicit visible fallback for pointer activation. A later phone integration test proved why that fallback matters: WebKit left `document.activeElement` on `<body>`, which is not a valid return target.

## Review fix round 1 evidence

The original report overstated selection consistency: it correctly said the shell waited for a selected result before closing, but Close and Escape still allowed the browser to disappear while the HTTP adapter was committing that selection. Deferred-promise tests captured the gap before the fix:

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx
  RED: 10 passed, 3 failed.
  Recovery continued from `saved-iceland` through `saved-trips` and root after unmount.
  Close remained enabled while `selectFolder` was pending.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx -t 'blocks Escape'
  RED: 12 skipped, 1 failed.
  Dispatching Escape on the pending dialog called `onClose` once.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx
  GREEN: 1 file, 13 tests.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/App.browser.test.tsx -t 'completes hosted selection into the wall and viewer at phone width'
  GREEN: 1 passed, 13 skipped.

npm exec --workspace @photo-viewer/interface -- vitest run --project browser src/components/HostedFolderBrowser.browser.test.tsx src/components/App.browser.test.tsx
  GREEN: 2 files, 27 tests.
```

## Final interface gates

```text
npm run check
  PASS: Biome checked 75 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 154 tests

npm run test:browser
  PASS: 4 files, 165 tests

git diff --check
  PASS: no whitespace errors
```

The full browser run emitted the branch's existing non-fatal ViewerStage `act(...)` warning. It had no test failure.

## Cleanup and constraints

- Removed the RED and focused-run screenshot directory and `.vitest-attachments` before staging.
- No Rust, server, dependency, lockfile, source-media, hosting, startup, security, or packaging file changed.
- No network access, dependency installation, Cargo command, background command, merge, or push occurred.
- All npm and Vitest commands ran serially in the foreground. The final process audit found no Cargo, rustc, Vitest, Vite, or Playwright process beyond the audit command itself.

## Concerns

No Task 7 implementation concern remains. Task 8 still owns static hosting, security headers, and production server startup. Task 9 still owns OCI packaging and hosted lifecycle acceptance.
