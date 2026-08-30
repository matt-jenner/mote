# Task 7 implementation report

Status: implementation complete, awaiting task-scoped review.

Implementation commit: `383ba85` (`feat: choose hosted gallery folders responsively`), based on `fa353ab`.

## Delivered behavior

- Added a contained hosted folder browser that uses `PhotoService` only. React never imports browser preferences, HTTP code, Tauri code, or native paths.
- Added structured breadcrumb recovery. The browser tries saved `breadcrumb.path` values from deepest to shallowest, then the mounted root. Back and clickable breadcrumbs also use only paths returned by `listFolders`.
- Kept the native picker route unchanged. The shell opens the contained browser only when `capabilities.folderSelection === "hosted"`.
- Added a centred desktop modal capped at 640 px by 70 vh. Narrow viewports and coarse pointers use a full-height sheet inside the safe-area insets. Folder rows meet the 44 px touch-target minimum.
- Added dialog semantics, focus entry and containment, Escape handling, trigger restoration, `aria-busy`, loading and empty states, error alerts, Retry, and reduced-motion handling.
- Preserved the last good listing when a child request fails. Monotonic request tokens prevent stale or unmounted requests from replacing current state.
- Disabled folder selection during directory loads and closed the hosted flow only after `selectFolder` returned a selected result.
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
  PASS: 2 files, 23 tests
```

The focused files cover desktop modal bounds, safe-area phone layout, coarse-pointer layout, focus trap and restoration, loading and retry, last-good-listing retention, deepest-first recovery, root fallback, server-derived Back navigation, selection cancellation, reduced motion, axe, native and hosted routing, selected source updates, breadcrumb resume, justified wall rendering, viewer open and return, and the phone drawer flow.

The first component GREEN run found two focused issues. A coarse-pointer sheet retained the desktop backdrop padding, and the standalone focus test opened from an unfocused pointer target. The sheet now removes that padding. The keyboard test opens from a focused trigger, while AppShell records an explicit visible fallback for pointer activation. A later phone integration test proved why that fallback matters: WebKit left `document.activeElement` on `<body>`, which is not a valid return target.

## Final interface gates

```text
npm run check
  PASS: Biome checked 75 files, no fixes required

npm run typecheck
  PASS: TypeScript project build exited 0

npm test
  PASS: 18 files, 154 tests

npm run test:browser
  PASS: 4 files, 161 tests

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
