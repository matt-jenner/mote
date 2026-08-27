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
  - polite live-region zoom announcements;
  - no navigator keyboard trap, forced offline cached zoom, source sentinel
    containment, malformed dimensions, zero-natural derivative, and interaction
    boundary coverage;
  - controlled browser fixture uses a 1536×1024 cached derivative so the
    native-resolution zoom ceiling remains testable while offline.
- `apps/interface/src/components/PhotoViewer.motion.test.tsx`
  - normal-motion preview crossfade and immediate transform-transition evidence.
- `apps/interface/src/components/PhotoViewerOverlay.tsx`
  - includes `Fit` or the native-resolution percentage in the polite viewer
    status announcement.
- `apps/interface/src/viewer/useViewerGestures.ts`
  - cancels a browser default only when a viewer-owned double-tap is recognized.
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

## Interface gates

```text
npm test
Test Files  16 passed (16)
Tests       126 passed (126)

npm run test:browser
Test Files  3 passed (3)
Tests       123 passed (123)
```

The full browser run emitted one React development warning about an
asynchronous `ViewerStage` update not being wrapped in `act(...)`, matching the
clean baseline recorded at feature base
`14ca838d1517d0e6bb9e72c40d0a3ab0937d6fdc`. It remains test-synchronization
noise and did not produce a failed test. The App and PhotoWall browser files
were also run alone: 49 tests passed with no warning. The viewer browser file
passed all 74 tests.

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

Browser evidence covers viewer-consumed wheel, pinch, zoomed touch pan, and
double-tap prevention while open; after close, synthetic document wheel and
double-tap events are not prevented. The malformed-dimension and zero-natural
cases stay at Fit with native zoom-in disabled and retain navigation.

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
- the live status is polite and includes Fit/native percentage after discrete
  input;
- hidden chrome remains out of the tab order while focus stays in the dialog;
- navigator content is visual-only on coarse pointers, `tabIndex=-1`, and does
  not add an assistive interaction trap;
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
final delivery commit: test: verify viewer zoom and pan
```
