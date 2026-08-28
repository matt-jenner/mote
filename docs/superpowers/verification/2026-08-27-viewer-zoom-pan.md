# Viewer zoom and pan verification

Date: 2026-08-27
Branch: `codex/viewer-zoom-pan`
Feature base: `14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc`
Feature head: final delivery commit; exact SHA is recorded in the companion
Task 6 report after commit finalization.

## Implementation

Task 6 completed the final browser and motion evidence, documented the cache-only
viewer controls, and added two small behavior fixes driven by failing tests:

- `apps/interface/src/components/PhotoViewer.browser.test.tsx`
  - full Axe state matrix for fit, zoomed controls, hidden chrome, navigator,
    Info drawer, and rotated phone layout;
  - native 44px target and disabled-state checks;
  - discrete polite live-region zoom announcements;
  - no navigator keyboard trap, forced offline cached zoom, source sentinel
    containment, malformed dimensions, zero-natural derivative, and interaction
    boundary coverage;
  - controlled browser fixture uses a 1536×1024 cached derivative so the
    native-resolution zoom ceiling remains testable while offline.
- `apps/interface/src/components/PhotoViewer.motion.test.tsx`
  - normal-motion preview crossfade and immediate transform-transition evidence.
- `apps/interface/src/components/PhotoViewerOverlay.tsx`
  - keeps discrete `Fit`/native-resolution zoom announcements separate from
    continuous transform frames.
- `apps/interface/src/components/ViewerStage.tsx`
  - gates preview natural dimensions on the successful decode token and restores
    the wall-thumbnail ceiling after a failed decode.
- `apps/interface/src/components/ViewerNavigator.tsx`
  - makes the navigator decorative to assistive technology and cleans up active
    drag state across capture loss, lifecycle changes, and unmount.
- `apps/interface/src/viewer/useViewerGestures.ts`
  - tombstones excluded touch IDs through their own release/cancel and protects
    active stage gestures from excluded pointer cancellation.
- `apps/interface/src/styles/photoViewer.module.css`
  - removes the zoom cluster from narrow drawer layouts while preserving its
    tab order after the drawer closes.
- `README.md`
  - documents mouse, keyboard, touch, navigator, and cache-only limitations.

No Rust, service, cache, native-path, source-media, or desktop protocol change
was made.

## RED/GREEN evidence

The live-region assertion was first run against the feature head and failed for
the intended missing behavior:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t 'announces discrete zoom labels'
1 failed | 69 skipped
Expected: Fit
Received: Coast, photo 1 of 61
```

After adding the zoom label to the existing polite status, the focused test was
green:

```text
Test Files  1 passed (1)
Tests       1 passed | 69 skipped (70)
```

The double-tap boundary assertion then failed for the intended missing browser
default cancellation:

```text
npm run test:browser --workspace @photo-viewer/interface -- PhotoViewer.browser.test.tsx -t 'prevents browser double-tap zoom'
1 failed | 70 skipped
AssertionError: expected false to be true
at secondUp.defaultPrevented
```

After adding cancellation at the recognized double-tap boundary, the focused
test was green:

```text
Test Files  1 passed (1)
Tests       1 passed | 70 skipped (71)
```

The additional final-state evidence was green:

```text
audits every open viewer accessibility state: 1 passed | 71 skipped (72)
malformed geometry and zero-natural: 1 passed | 72 skipped (73)
ready derivative URLs + current position: 2 passed | 72 skipped (74)
```

The first full run after the new offline assertion exposed a deliberately
undersized 1×1 cached fixture and the old status string. The fixture was changed
to the controlled 1536×1024 derivative and the status expectation was updated;
the final full run below is green.

## Final review RED/GREEN evidence

The final review regressions were added before their production fixes. The first
focused run failed the decode-ceiling, excluded-touch, narrow-drawer,
continuous-live-region, and navigator-lifecycle assertions. The focused unit
and browser reruns were green after the corresponding fixes, including the
screen-preview success/failure ceiling transitions, fresh pointer-ID reuse,
desktop/coarse decorative navigator states, and double-click live-region
mutations.

## Interface gates

```text
npm test
Test Files  16 passed (16)
Tests       129 passed (129)

npm run test:browser
Test Files  3 passed (3)
Tests       129 passed (129)
```

The full browser run emitted one React development warning about an
asynchronous `ViewerStage` update not being wrapped in `act(...)`, matching the
clean baseline recorded at feature base
`14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc`. It remains test-synchronization
noise and did not produce a failed test. The App and PhotoWall browser files
were also run alone: 49 tests passed with no warning. The viewer browser file
passed all 80 tests.

```text
npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
Test Files  1 passed (1)
Tests       2 passed (2)

npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast src/components/PhotoViewer.contrast.test.tsx
Test Files  1 passed (1)
Tests       1 passed (1)

npm run typecheck
exit 0

npm run check
Checked 69 files in 109ms. No fixes applied.

npm run --workspace @photo-viewer/interface build
1888 modules transformed
dist/assets/index-DYojYnL0.css   21.61 kB │ gzip:  4.95 kB
dist/assets/index-D6LRUogd.js   314.25 kB │ gzip: 96.07 kB
built in 218ms
```

Browser evidence covers viewer-consumed wheel, pinch, zoomed touch pan,
mouse-drag translation and pointer capture, excluded control touches, drawer
coverage at 390px, decode-ceiling success/failure, navigator lifecycle cleanup,
decorative navigator semantics in desktop/coarse modes, discrete button and
keyboard shortcuts, double-click live-region updates, and continuous-route
mutation suppression. After close, synthetic document wheel and double-tap
events are not prevented. The malformed-dimension and zero-natural cases stay
at Fit with native zoom-in disabled and retain navigation.

## Rust and desktop gates

```text
cargo test --workspace --all-features
191 passed; 0 failed; 0 ignored

cargo clippy --workspace --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --all -- --check
exit 0

cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
11 passed; 0 failed
0 doc-test failures

cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
exit 0
```

The benchmark command completed successfully:

```text
cargo run -p catalog-bench --release -- --assets 10000 --output target/catalog-benchmark-viewer-zoom.json
```

Recorded `target/catalog-benchmark-viewer-zoom.json`:

```json
{
  "assets": 10000,
  "sqlite_version": "3.53.2",
  "database_bytes": 6602752,
  "insert_ms": 885.591625,
  "first_page_ms": 0.37437499999999996,
  "first_page_rows": 100,
  "unavailable_count_ms": 1.286417,
  "eviction_plan_ms": 0.092916
}
```

The unsigned macOS app build completed successfully:

```text
npm run desktop:build -- --bundles app
Finished 1 bundle at:
/Users/jennerm/repos/photo_viewer/.worktrees/viewer-zoom-pan/apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app
```

## Source-media invariant

The required aggregate hash was captured immediately before and immediately
after the controlled derivative-generation test:

```text
find apps/interface/public/demo-photos -maxdepth 1 -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6  -

cargo test -p photo-cache --test image_derivative controlled_demo_fixture_generation_leaves_source_unchanged
test controlled_demo_fixture_generation_leaves_source_unchanged ... ok
test result: ok. 1 passed; 0 failed; 8 filtered out

find apps/interface/public/demo-photos -maxdepth 1 -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6  -
```

The exact feature-range audit uses base
`14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc` through the final delivery head;
the exact resolved head SHA is in the companion Task 6 report:

```text
git diff --name-only 14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc..HEAD -- '*.rs'
(no output)
```

The corresponding name-status audit listed README, interface viewer
source/tests/CSS/config, and this verification document only. Production-diff
review found no source write, rename, move, copy, rating, tag, deletion, or
native-path operation. A source/native token search had only test labels, DOM
`removeProperty` cleanup, and UI wording; no filesystem operation. The
selected native path was not inspected or recorded.

## Native acceptance state

The Mac development app was already running from this worktree under the
controller-managed `wall-demo` profile. Per the task binding, this task did not
start, stop, inspect, or record that GUI process or the selected native path.
The app bundle was built successfully above. The controller should restart the
protected app from the verified bundle/dev state and perform the requested
double-click, pinch, pan, keyboard, navigator, refinement, navigation-reset,
resize, and safely reproducible cached-offline acceptance observations. No
merge or push was performed.

## Documentation, self-review, and concerns

README now documents Fit/percentage controls, keyboard and pointer gestures,
native-resolution limits, navigator behavior, and cached offline limitations.

Self-review confirms:

- zoom controls remain native buttons with 44px targets and native disabled
  state;
- the live status is polite, preserves photo position, and only changes its
  zoom label for discrete controls, shortcuts, and double-click/tap input;
- hidden chrome remains out of the tab order while focus stays in the dialog;
- navigator content is decorative (`aria-hidden="true"`) at every pointer mode,
  `tabIndex=-1`, and does not add an assistive interaction trap while retaining
  visible desktop pointer functionality;
- preview natural dimensions remain at the wall-thumbnail ceiling until the
  matching screen preview decode succeeds, and return to that ceiling on failure;
- narrow drawers hide the overlapping zoom cluster and restore its tab order on
  close;
- navigator drag cleanup ends manipulation once across capture loss, unmount,
  asset/viewport revision, and modality changes;
- cached derivative URLs remain usable after `sourceUnavailable` without the
  sentinel or a native/source URL entering viewer DOM;
- malformed geometry and zero-natural derivatives fail closed to Fit;
- double-tap cancellation is scoped to the mounted/open viewer gesture hook;
- no source media was modified.

Concerns/limitations:

- the full browser project emits one non-failing React `act(...)` console
  warning from asynchronous `ViewerStage` work, matching the clean baseline
  recorded at feature base `14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc`; it is
  test-synchronization noise;
- native acceptance observations remain for the controller because the running
  app was intentionally not interrupted;
- cache-only zoom does not recover an uncached derivative while its source is
  unavailable, and it never enlarges the original source file.

Final cleanup checks and commit evidence:

```text
git diff --check
exit 0

git status --short
(clean after commit)

git show -s --format='%h %s' HEAD
The exact post-commit SHA and subject are recorded in the companion Task 6 and
final-fix reports after commit finalization.
```

## Task 7 automated verification (2026-08-28)

Task 7 extends the catalog benchmark without changing the application
coordinator. It generates a deterministic 90 percent still and 10 percent
video catalog, assigns every asset to one shaped wall group, and keeps videos
indexed while the wall and coordinator projections filter them. The benchmark
now measures the first and cursor-continuation wall pages, the 250-ID
coordinator page, and a terminal-failure lookup using the current asset key.
Insertion, unavailable counting, and eviction planning remain in the report.

### Benchmark RED/GREEN evidence

The smoke test was extended before the implementation. The first run failed at
compile time because `BenchmarkReport` did not yet have the new page and
terminal lookup fields:

```text
cargo test -p catalog-bench --test benchmark_smoke
error[E0609]: no field `second_page_ms` on type `BenchmarkReport`
error[E0609]: no field `second_page_rows` on type `BenchmarkReport`
error[E0609]: no field `coordinator_page_ms` on type `BenchmarkReport`
error[E0609]: no field `coordinator_page_rows` on type `BenchmarkReport`
error[E0609]: no field `terminal_lookup_ms` on type `BenchmarkReport`
```

After the benchmark implementation, the focused smoke test passed:

```text
cargo test -p catalog-bench --test benchmark_smoke
test result: ok. 1 passed; 0 failed; 0 ignored
```

### Million-asset benchmark

The compile-warm release command was:

```text
cargo run --release -p catalog-bench -- --assets 1000000 --output /tmp/photo-viewer-million-report.json
```

The report at `/tmp/photo-viewer-million-report.json` has SHA-256
`85c36fbc4735b8dcbde049a77cf8232cb23837dc63fe7d34cbcb4143cf8f300d` and
contains:

```json
{
  "assets": 1000000,
  "sqlite_version": "3.53.2",
  "database_bytes": 789831680,
  "insert_ms": 421234.77991700004,
  "first_page_ms": 0.259917,
  "first_page_rows": 100,
  "second_page_ms": 0.173375,
  "second_page_rows": 100,
  "coordinator_page_ms": 0.24516700000000002,
  "coordinator_page_rows": 250,
  "terminal_lookup_ms": 0.033166,
  "unavailable_count_ms": 332.129875,
  "eviction_plan_ms": 3.2902500000000003
}
```

The previous first-page measurement was 1.08 ms, recorded for the 1,000,000
asset profile at commit `3186354`. The 20 percent ceiling is 1.296 ms. The
fresh 0.259917 ms result is 24.066389 percent of that earlier measurement.
The bounded query pages contain exactly 100, 100, and 250 rows, and every
returned row is a still. The benchmark does not materialize the collection or
take a queue snapshot.

Machine context for this run:

```text
MacBookPro18,1, Apple M1 Pro, 10 cores, 16 GB RAM
macOS 26.5.2 (Darwin 25.5.0, arm64)
rustc 1.97.1, cargo 1.97.1
Node v24.18.0, npm 11.16.0
```

### Interface gates

```text
npm test
Test Files  16 passed (16)
Tests       129 passed (129)

npm run test:browser
Test Files  3 passed (3)
Tests       145 passed (145)

npm exec --workspace @photo-viewer/interface -- vitest run --project browser-motion
Test Files  1 passed (1)
Tests       2 passed (2)

npm exec --workspace @photo-viewer/interface -- vitest run --project browser-contrast
Test Files  1 passed (1)
Tests       2 passed (2)

npm run typecheck
exit 0

npm run check
Checked 69 files in 220ms. No fixes applied.

npm run --workspace @photo-viewer/interface build
1888 modules transformed
dist/assets/index-DJKmgDN-.css   21.67 kB, gzip 4.97 kB
dist/assets/index-CcP2hML6.js   317.79 kB, gzip 97.14 kB
```

The full browser run emitted the one existing non-failing React `ViewerStage`
`act(...)` warning. Motion, contrast, unit, typecheck, and Biome runs emitted
no warnings. No unhandled promise rejection, accessibility violation, or
screenshot attachment was produced.

### Rust and desktop gates

```text
cargo test --workspace --all-features
191 passed; 0 failed; 0 ignored

cargo clippy --workspace --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --all -- --check
exit 0

cargo test -p catalog-bench --test benchmark_smoke
1 passed; 0 failed

cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
11 passed; 0 failed; 0 doc-test failures

cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
Finished `dev` profile; no warnings or errors

cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check
exit 0

npm run desktop:build
Finished 1 bundle at:
/Users/jennerm/repos/photo_viewer/.worktrees/viewer-zoom-pan/apps/desktop/src-tauri/target/release/bundle/macos/Photo Viewer.app

git diff --check
exit 0
```

The interface build assets are 317,796 bytes of JavaScript and 21,673 bytes
of CSS. The app bundle contains three files, occupies 23,176 KiB on disk, and
its native executable is 23,448,864 bytes.

### Source and cache safety audit

The controlled fixture tree was snapshotted immediately before and after one
complete automated safety window. The exact tree is
`apps/interface/public/demo-photos`; the six-file `mtime|size` manifest was
identical at both boundaries:

```text
apps/interface/public/demo-photos/city.jpg|1787835664|615559
apps/interface/public/demo-photos/coast.jpg|1787835664|597900
apps/interface/public/demo-photos/forest.jpg|1787835664|865825
apps/interface/public/demo-photos/interior.jpg|1787835664|446647
apps/interface/public/demo-photos/mountain.jpg|1787835664|483651
apps/interface/public/demo-photos/portrait.jpg|1787835664|319646
```

The exact command window was:

```text
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 stat -f '%N|%m|%z'
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256

cargo test -p photo-app-service --test task7_source_safety -- --nocapture
1 passed; 0 failed

npm run test:browser
3 files, 145 passed; 0 failed

find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 stat -f '%N|%m|%z'
find apps/interface/public/demo-photos -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256
```

The before and after aggregate SHA-256 was
`a4c522354a075e2a6b6804a31b9df42dcd56b6509cef14ae4667e700227ae3e6`.
The metadata diff had 0 lines, the per-file hash diff had 0 lines, and
`git status --short -- apps/interface/public/demo-photos` had no output. No
`PHOTO_VIEWER_PROFILE` variable was used: the app-service harness creates
`tempdir()/catalog` and `tempdir()/cache` and asserts both are contained by the
same temporary root before opening the service.

The committed harness opens the exact source tree read-only, scans it, asserts
the first cursor page and continuation contain exactly six JPEG rows with no
video rows, sorts both directions, plans a current visible request and
near-viewport neighbours, completes the thumbnail phase, then the preview
phase, and waits for derivative quiescence. It removes only a temporary
catalogue wall row to model a legacy cached screen preview, marks that
catalogue root offline, and proves the wall thumbnail is repaired from the
managed screen cache. It then restarts the service outside the runtime,
re-queries the six cached rows, and completes an offline cached preview
request. The test snapshots every source file's bytes, BLAKE3 digest, size,
and nanosecond mtime immediately before this sequence and after the restarted
service is dropped.

The browser tests use the same `/demo-photos/*.jpg` URLs for wall sorting,
viewer opening, and zoom. The full run emitted one known non-failing React
`ViewerStage` `act(...)` warning; it had no rejection, accessibility failure,
or screenshot attachment. No desktop development process was started by this
task.

The feature-range source audit used accepted base
`14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc` through the code/test head
`67393e1` (`test: guard demo source tree`). The added-line audit covers Rust,
TypeScript, and TSX and includes filesystem create/write/save/copy/move/
rename/remove/delete/unlink forms, `OpenOptions`, `File::create`,
`write_all`, `write_atomic`, and source/cache/folder/path DTO tokens. Its
exact command and all 56 file:line matches, with classifications, are in the
companion Task 7 report.

```text
git diff --name-status 14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc..67393e1 -- '*.rs' '*.ts' '*.tsx'
git diff --unified=0 --no-color 14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc..67393e1 -- '*.rs' '*.ts' '*.tsx' | awk '
/^\+\+\+ b\// { file=substr($0,7); next }
/^@@ / { p=index($0,"+"); if (p) { h=substr($0,p+1); sub(",.*","",h); line=h+0 }; next }
/^\+/ && !/^\+\+\+/ {
  text=substr($0,2)
  if (text ~ /(std::fs::(create_dir_all|write|rename|remove_file|remove_dir_all|copy)|fs::(write|rename|remove|copy)|\.save[[:space:]]*\(|File::create|OpenOptions|write_all|write_atomic|writeFile|write_file|copyFile|copy_file|moveFile|move_file|rename|removeFile|remove_file|unlink|delete_derivatives|deleteAsset|setRating|updateRating|selectedFolder(Name)?|selected_folder|source(Path|_path)|folder(Path|_path)|relative(Path|_path)|display(Path|_path)|native(Path|_path)|cache(Path|_path)|locateFolder)/) print file ":" line ": " text
  line++
}
'
```

Production source code contains no source-media write, delete, rename, move,
or copy call and no native source-path DTO field. Production cache writes are
confined to `CacheWriter::write_atomic`/`Write::write_all` under the managed
cache root; source paths are only opened for decoding. The matched test lines
are temporary image/video fixture setup, source-availability simulation, or
temporary catalogue/cache cleanup; the new safety test's only deletion is its
temporary catalogue row. In-memory `HashMap::remove`, DOM `delete`, and
`tokio::spawn(async move ...)` matches were separately reviewed as non-filesystem
operations.

The existing managed wall-demo cache root was inspected without changing it:

```text
/Users/jennerm/Library/Caches/app.photoviewer.desktop/profiles/wall-demo
regular_file_count=1566
symlink_count=0
resolved_paths_outside_root=0
cache_root_exists=yes
```

### Approved behavior and limitations

- Videos remain indexed but stay invisible on photo surfaces until
  cross-platform playback ships.
- A wall tile opens only after its current thumbnail paints. Background work
  runs thumbnails first and previews second.
- Corrupt photos do not block healthy work.
- Escape closes Info, then resets zoom, then returns to the wall.
- The viewer remains dark under system-light appearance.
- Zoom is cache-only. It does not read or enlarge the original source, and an
  uncached derivative has no offline recovery path.

The native acceptance build was created, but the native demo was intentionally
not launched here. The controller still owns the required user acceptance
observations and must leave the `wall-demo` process in the requested state.
